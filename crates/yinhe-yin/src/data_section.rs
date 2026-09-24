//! data 段（.yin 第 3 段）编解码：meta 流 + 5 个轨段列式音符流。
//!
//! ```text
//! 0: postcard(conductor + tracks payload + segments)  ← 非音符部分（单帧 zstd）
//! 1: delta 列（varint u32：轨段首为绝对 start，其余 = start - prev）
//! 2: key   列（u8 × N）
//! 3: vel   列（u8 × N）
//! 4: gate  列（varint u32）
//! 5: id delta 列（zigzag varint，段内独立：段首为绝对 id）
//! ```
//!
//! 音符列是**分帧流**：`[len u32 LE][zstd 帧]` 重复，帧边界落在音符边界。
//! 保存侧归并时按 track 暂存列缓冲，归并结束后按 track 升序拼接、攒到
//! `FRAME_BYTES` 即压缩一帧——不需要全量 SoA/全局列缓冲，保存峰值从
//! 1.64 亿音符 ~6.7GB 降到 ~3.9GB（模型本身 2.6GB）。
//! 实测 16MB 分帧的压缩率损失 <2%（1MB 帧则损失 29-68%）。
//! 加载侧逐帧解压、边解析边 `NoteLoader::feed`，不物化全量列 Vec。

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use yinhe_core::{ConductorData, NoteLoader, PcEvent, YinModel};
use yinhe_types::{AutomationLane, KEY_COUNT};

use crate::codec::{deserialize_postcard, push_varint, serialize_postcard, unzigzag, zigzag};
use crate::error::{YinError, invalid_data};
use crate::progress::{YinProgress, YinProgressStage, progress};

/// 分帧压缩的单帧上限（压缩前字节）。实测 16MB 帧压缩率损失 <2%。
const FRAME_BYTES: usize = 16 << 20;

/// 非音符部分（conductor + tracks payload + 轨段表），整体 postcard + zstd。
#[derive(Serialize, Deserialize)]
pub(crate) struct MetaPayload {
    pub(crate) conductor: ConductorData,
    pub(crate) tracks: Vec<TrackPayload>,
    /// 音符流按 (track, start, key) 分段的段表（track 升序，跳过空轨）。
    pub(crate) segments: Vec<TrackSegment>,
}

/// 轨段头：track-major 布局下每段对应一个轨道，`count` 为该段音符数。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TrackSegment {
    pub(crate) track: u16,
    pub(crate) count: u32,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TrackPayload {
    pub(crate) uuid: String,
    pub(crate) automation_lanes: Vec<AutomationLane>,
    pub(crate) program_change: Vec<PcEvent>,
    #[serde(default)]
    pub(crate) lyrics: Vec<yinhe_types::LyricsEvent>,
    #[serde(default)]
    pub(crate) chord: Vec<yinhe_types::ChordEvent>,
}

/// 归并堆元素：堆顶 = 当前 (start, track, key) 最小的桶游标。
/// `key` 即桶号（0-255），同 (start, track) 的不同桶 key 必不同，全序无歧义。
/// `note` 携带游标指向的音符本体（元素已被 `next()` 消费，避免二次取）。
/// 比较只按 (start, track, key)（`Note` 无 Eq/Ord，不参与排序）。
#[derive(Clone, Copy)]
struct HeapEntry<'a> {
    start: u32,
    track: u16,
    key: u8,
    note: &'a yinhe_types::Note,
}

impl PartialEq for HeapEntry<'_> {
    fn eq(&self, other: &Self) -> bool {
        (self.start, self.track, self.key) == (other.start, other.track, other.key)
    }
}
impl Eq for HeapEntry<'_> {}
impl PartialOrd for HeapEntry<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapEntry<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.start, self.track, self.key).cmp(&(other.start, other.track, other.key))
    }
}

/// 每 1M 音符汇报一次进度的掩码。
const PROGRESS_MASK: usize = 0xF_FFFF;

/// 单列的分帧编码器：攒够 `FRAME_BYTES` 就压一帧到输出。
#[derive(Default)]
struct ColumnEncoder {
    buf: Vec<u8>,
    out: Vec<u8>,
}

impl ColumnEncoder {
    fn flush(&mut self, level: i32) -> Result<(), YinError> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let comp = zstd::encode_all(Cursor::new(&self.buf), level)?;
        self.out
            .extend_from_slice(&(comp.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&comp);
        self.buf.clear();
        Ok(())
    }

    fn finish(mut self, level: i32) -> Result<Vec<u8>, YinError> {
        self.flush(level)?;
        Ok(self.out)
    }
}

/// 编码完成的 5 个分帧列流（每流 = 若干 `[len u32][zstd 帧]`）。
#[derive(Default)]
pub(crate) struct EncodedNoteStreams {
    pub(crate) delta: Vec<u8>,
    pub(crate) key: Vec<u8>,
    pub(crate) vel: Vec<u8>,
    pub(crate) gate: Vec<u8>,
    pub(crate) id_delta: Vec<u8>,
}

/// 保存侧：KEY_COUNT 路归并（桶内已按 start 有序）输出 (track, start, key)
/// 轨段分帧列流。
///
/// 归并序仍是全局 (start, track, key)（O(N log KEY_COUNT)）；同一 track 的
/// 音符按 start 递增出现，直接追加到该 track 的列缓冲，归并结束后按 track
/// 升序拼接压缩。乱序桶兜底本地排序（模型不变量，正常不触发）。
pub(crate) fn encode_note_streams(
    model: &YinModel,
    level: i32,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<(EncodedNoteStreams, Vec<TrackSegment>), YinError> {
    let level = level.clamp(0, 22);
    let total: usize = model.notes.iter().map(|b| b.len()).sum();

    // 兜底：乱序桶先本地排（不写回模型，只影响本次归并源）。
    let mut sorted_copies: Vec<Option<Vec<yinhe_types::Note>>> = Vec::with_capacity(KEY_COUNT);
    for (key, bucket) in model.notes.iter().enumerate() {
        if bucket.is_sorted() {
            sorted_copies.push(None);
        } else {
            let mut b: Vec<yinhe_types::Note> = bucket.iter().copied().collect();
            b.sort_unstable_by_key(|n| n.start_tick);
            sorted_copies.push(Some(b));
        }
        progress(
            on_progress,
            YinProgressStage::Collect,
            (key as f32 + 1.0) / KEY_COUNT as f32,
        );
    }
    let mut sources: Vec<Box<dyn Iterator<Item = &yinhe_types::Note> + '_>> =
        Vec::with_capacity(KEY_COUNT);
    for (key, bucket) in model.notes.iter().enumerate() {
        match &sorted_copies[key] {
            Some(c) => sources.push(Box::new(c.iter())),
            None => sources.push(Box::new(bucket.iter())),
        }
    }

    // 预扫描：最大 track 号 + 每轨音符数（per-track 列缓冲精确预分配）。
    let mut max_track: usize = 0;
    for bucket in model.notes.iter() {
        for n in bucket.iter() {
            max_track = max_track.max(n.track as usize);
        }
    }
    let track_slots = max_track + 1;
    let mut track_counts = vec![0u32; track_slots];
    for bucket in model.notes.iter() {
        for n in bucket.iter() {
            track_counts[n.track as usize] += 1;
        }
    }

    // per-track 列缓冲：同一 track 的音符按 start 递增出现，直接追加即段内有序。
    // key/vel 恒定 1B/音符精确分配；delta/gate/id 是 varint（多数 1B，少数更长），
    // 预留 12.5% 余量避免 Vec 翻倍增长（翻倍会白占一倍内存）。
    let alloc_exact = |c: u32| Vec::with_capacity(c as usize);
    let alloc_slack = |c: u32| Vec::with_capacity(c as usize + c as usize / 8 + 16);
    let mut delta_bufs: Vec<Vec<u8>> = track_counts.iter().map(|&c| alloc_slack(c)).collect();
    let mut key_bufs: Vec<Vec<u8>> = track_counts.iter().map(|&c| alloc_exact(c)).collect();
    let mut vel_bufs: Vec<Vec<u8>> = track_counts.iter().map(|&c| alloc_exact(c)).collect();
    let mut gate_bufs: Vec<Vec<u8>> = track_counts.iter().map(|&c| alloc_slack(c)).collect();
    let mut id_bufs: Vec<Vec<u8>> = track_counts.iter().map(|&c| alloc_slack(c)).collect();
    let mut prev_start = vec![0u32; track_slots];
    let mut prev_id = vec![0i64; track_slots];

    // KEY_COUNT 路归并：每桶一个游标在堆里，pop 最小 (start, track, key) 后
    // 推进该桶下一个。桶内按 start 有序（兜底已排），输出即全局序。
    let mut heap: std::collections::BinaryHeap<std::cmp::Reverse<HeapEntry>> =
        std::collections::BinaryHeap::with_capacity(KEY_COUNT);
    for (key, src) in sources.iter_mut().enumerate() {
        if let Some(n) = src.next() {
            heap.push(std::cmp::Reverse(HeapEntry {
                start: n.start_tick,
                track: n.track,
                key: key as u8,
                note: n,
            }));
        }
    }

    for i in 0..total {
        let Some(std::cmp::Reverse(e)) = heap.pop() else {
            // 归并堆在 total 次 pop 内应保持非空（每 pop 一个立即补该桶下一个）。
            return Err(invalid_data("note merge heap underflow"));
        };
        let key = e.key as usize;
        let n = e.note;
        if let Some(next) = sources[key].next() {
            heap.push(std::cmp::Reverse(HeapEntry {
                start: next.start_tick,
                track: next.track,
                key: e.key,
                note: next,
            }));
        }
        let tr = n.track as usize;
        push_varint(&mut delta_bufs[tr], (n.start_tick - prev_start[tr]) as u64);
        key_bufs[tr].push(e.key);
        vel_bufs[tr].push(n.velocity);
        push_varint(
            &mut gate_bufs[tr],
            n.end_tick.saturating_sub(n.start_tick) as u64,
        );
        push_varint(&mut id_bufs[tr], zigzag(n.id as i64 - prev_id[tr]));
        prev_start[tr] = n.start_tick;
        prev_id[tr] = n.id as i64;
        if i & PROGRESS_MASK == 0 {
            progress(
                on_progress,
                YinProgressStage::Sort,
                (i as f32 + 1.0) / total as f32,
            );
        }
    }
    progress(on_progress, YinProgressStage::Sort, 1.0);

    // 按 track 升序拼接 + 分帧压缩：每轨处理完立即释放其列缓冲，
    // 峰值 ≈ 未压缩轨缓冲总和 + 5 帧组缓冲，而不是全量列 + 全量 SoA。
    let mut delta_enc = ColumnEncoder::default();
    let mut key_enc = ColumnEncoder::default();
    let mut vel_enc = ColumnEncoder::default();
    let mut gate_enc = ColumnEncoder::default();
    let mut id_enc = ColumnEncoder::default();
    let mut segments = Vec::new();
    for tr in 0..track_slots {
        if track_counts[tr] == 0 {
            continue;
        }
        segments.push(TrackSegment {
            track: tr as u16,
            count: track_counts[tr],
        });
        delta_enc.buf.extend_from_slice(&delta_bufs[tr]);
        key_enc.buf.extend_from_slice(&key_bufs[tr]);
        vel_enc.buf.extend_from_slice(&vel_bufs[tr]);
        gate_enc.buf.extend_from_slice(&gate_bufs[tr]);
        id_enc.buf.extend_from_slice(&id_bufs[tr]);
        delta_bufs[tr] = Vec::new();
        key_bufs[tr] = Vec::new();
        vel_bufs[tr] = Vec::new();
        gate_bufs[tr] = Vec::new();
        id_bufs[tr] = Vec::new();
        if delta_enc.buf.len() >= FRAME_BYTES {
            delta_enc.flush(level)?;
            key_enc.flush(level)?;
            vel_enc.flush(level)?;
            gate_enc.flush(level)?;
            id_enc.flush(level)?;
        }
        progress(
            on_progress,
            YinProgressStage::Compress,
            (tr as f32 + 1.0) / track_slots as f32,
        );
    }
    progress(on_progress, YinProgressStage::Compress, 1.0);
    let notes = EncodedNoteStreams {
        delta: delta_enc.finish(level)?,
        key: key_enc.finish(level)?,
        vel: vel_enc.finish(level)?,
        gate: gate_enc.finish(level)?,
        id_delta: id_enc.finish(level)?,
    };
    Ok((notes, segments))
}

/// 保存侧：meta 流（单帧 zstd）+ 5 个已分帧压缩的音符列，各自 length-prefix。
pub(crate) fn compress_data(
    meta: &MetaPayload,
    notes: EncodedNoteStreams,
    level: i32,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<u8>, YinError> {
    let meta_raw = serialize_postcard(meta)?;
    let meta_comp = zstd::encode_all(Cursor::new(&meta_raw), level.clamp(0, 22))?;
    let streams: [Vec<u8>; 6] = [
        meta_comp,
        notes.delta,
        notes.key,
        notes.vel,
        notes.gate,
        notes.id_delta,
    ];
    let mut out = Vec::new();
    for (i, stream) in streams.into_iter().enumerate() {
        out.extend_from_slice(&(stream.len() as u32).to_le_bytes());
        out.extend_from_slice(&stream);
        progress(
            on_progress,
            YinProgressStage::Compress,
            (i as f32 + 1.0) / 6.0,
        );
    }
    Ok(out)
}

// =========================================================
//  加载侧
// =========================================================

/// 单列分帧流读取器：逐帧解压 `[len u32][zstd 帧]` 直到流尾。
struct FrameReader<'a> {
    data: &'a [u8],
    pos: usize,
    frame: Vec<u8>,
    frame_pos: usize,
}

impl<'a> FrameReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            frame: Vec::new(),
            frame_pos: 0,
        }
    }

    /// 解压下一帧到缓冲；流已耗尽返回 false。
    fn next_frame(&mut self) -> Result<bool, YinError> {
        if self.pos >= self.data.len() {
            return Ok(false);
        }
        if self.pos + 4 > self.data.len() {
            return Err(YinError::Truncated {
                needed: 4,
                available: self.data.len() - self.pos,
            });
        }
        let len = u32::from_le_bytes(
            self.data[self.pos..self.pos + 4]
                .try_into()
                .map_err(|_| invalid_data("frame length prefix"))?,
        ) as usize;
        self.pos += 4;
        if self.pos + len > self.data.len() {
            return Err(YinError::Truncated {
                needed: len,
                available: self.data.len() - self.pos,
            });
        }
        self.frame = zstd::decode_all(Cursor::new(&self.data[self.pos..self.pos + len]))?;
        self.pos += len;
        self.frame_pos = 0;
        Ok(true)
    }

    fn read_u8(&mut self) -> Result<u8, YinError> {
        if self.frame_pos >= self.frame.len() && !self.next_frame()? {
            return Err(invalid_data("note stream ended early"));
        }
        let b = self.frame[self.frame_pos];
        self.frame_pos += 1;
        Ok(b)
    }

    fn read_varint(&mut self) -> Result<u64, YinError> {
        let mut v: u64 = 0;
        let mut shift = 0u32;
        loop {
            let b = self.read_u8()?;
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
            if shift >= 64 {
                return Err(invalid_data("varint too long"));
            }
        }
    }
}

/// 5 个音符列的读取器（借用 data 段的原始字节）。
pub(crate) struct NoteStreamReaders<'a> {
    delta: FrameReader<'a>,
    key: FrameReader<'a>,
    vel: FrameReader<'a>,
    gate: FrameReader<'a>,
    id_delta: FrameReader<'a>,
}

/// 解析 data 段外层：拆出 6 个 length-prefixed 流，解压 meta，构造列读取器。
pub(crate) fn open_note_streams<'a>(
    data: &'a [u8],
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<(MetaPayload, NoteStreamReaders<'a>), YinError> {
    let mut blocks: Vec<&[u8]> = Vec::with_capacity(6);
    let mut off = 0usize;
    for i in 0..6 {
        if off + 4 > data.len() {
            return Err(YinError::Truncated {
                needed: 4,
                available: data.len() - off,
            });
        }
        let len = u32::from_le_bytes(
            data[off..off + 4]
                .try_into()
                .map_err(|_| invalid_data("stream length prefix"))?,
        ) as usize;
        off += 4;
        if off + len > data.len() {
            return Err(YinError::Truncated {
                needed: len,
                available: data.len() - off,
            });
        }
        blocks.push(&data[off..off + len]);
        off += len;
        progress(
            on_progress,
            YinProgressStage::Decompress,
            (i as f32 + 1.0) / 6.0,
        );
    }
    let meta: MetaPayload = deserialize_postcard(&zstd::decode_all(Cursor::new(blocks[0]))?)?;
    let readers = NoteStreamReaders {
        delta: FrameReader::new(blocks[1]),
        key: FrameReader::new(blocks[2]),
        vel: FrameReader::new(blocks[3]),
        gate: FrameReader::new(blocks[4]),
        id_delta: FrameReader::new(blocks[5]),
    };
    Ok((meta, readers))
}

impl NoteStreamReaders<'_> {
    /// 第一遍：只解压 key 列统计每桶音符数，供 `NoteLoader` 精确预分配。
    pub(crate) fn key_counts(&self) -> Result<[u32; KEY_COUNT], YinError> {
        let mut counts = [0u32; KEY_COUNT];
        let mut r = FrameReader::new(self.key.data);
        while r.next_frame()? {
            for &b in &r.frame {
                counts[b as usize] += 1;
            }
        }
        Ok(counts)
    }

    /// 第二遍：5 列同步解析，逐音符喂给 `NoteLoader`（不物化全量列 Vec）。
    ///
    /// id delta 段内独立：每段首音符为绝对 id，段内累加。
    pub(crate) fn feed(
        &mut self,
        segments: &[TrackSegment],
        loader: &mut NoteLoader,
        on_progress: &mut dyn FnMut(YinProgress),
    ) -> Result<(), YinError> {
        let total: usize = segments.iter().map(|s| s.count as usize).sum();
        let mut done = 0usize;
        for seg in segments {
            let mut prev_start: u32 = 0;
            let mut prev_id: i64 = 0;
            for _ in 0..seg.count {
                let delta = u32::try_from(self.delta.read_varint()?)
                    .map_err(|_| invalid_data("delta out of u32 range"))?;
                let start = prev_start
                    .checked_add(delta)
                    .ok_or_else(|| invalid_data("delta start overflow"))?;
                let key = self.key.read_u8()?;
                let vel = self.vel.read_u8()?;
                let gate = u32::try_from(self.gate.read_varint()?)
                    .map_err(|_| invalid_data("gate out of u32 range"))?;
                let id = prev_id + unzigzag(self.id_delta.read_varint()?);
                if !(0..=u32::MAX as i64).contains(&id) {
                    return Err(invalid_data("note id out of u32 range"));
                }
                loader.feed(
                    key,
                    seg.track,
                    start,
                    start.saturating_add(gate),
                    vel,
                    id as u32,
                );
                prev_start = start;
                prev_id = id;
                done += 1;
                if done & PROGRESS_MASK == 0 {
                    progress(
                        on_progress,
                        YinProgressStage::Rebuild,
                        done as f32 / total as f32,
                    );
                }
            }
        }
        progress(on_progress, YinProgressStage::Rebuild, 1.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yinhe_core::{NoteEvent, TrackData};

    use super::*;

    fn encode_varints(values: &[u64]) -> Vec<u8> {
        let mut enc = ColumnEncoder::default();
        for &v in values {
            push_varint(&mut enc.buf, v);
        }
        enc.finish(3).unwrap()
    }

    #[test]
    fn frame_reader_reads_varints() {
        let values = [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX];
        let stream = encode_varints(&values);
        let mut r = FrameReader::new(&stream);
        for v in values {
            assert_eq!(r.read_varint().unwrap(), v);
        }
        assert!(r.read_varint().is_err(), "流耗尽应报错");
    }

    #[test]
    fn frame_reader_handles_multiple_frames() {
        // 两个帧拼接：读取应跨帧连续。
        let mut out = Vec::new();
        for chunk in [[10u64, 20], [30, 40]] {
            let mut enc = ColumnEncoder::default();
            for v in chunk {
                push_varint(&mut enc.buf, v);
            }
            out.extend_from_slice(&enc.finish(3).unwrap());
        }
        let mut r = FrameReader::new(&out);
        for v in [10u64, 20, 30, 40] {
            assert_eq!(r.read_varint().unwrap(), v);
        }
    }

    #[test]
    fn frame_reader_rejects_truncated_frame() {
        let stream = encode_varints(&[1, 2, 3]);
        let truncated = &stream[..stream.len() - 1];
        let mut r = FrameReader::new(truncated);
        assert!(r.read_varint().is_err());
    }

    /// 全链路：模型 → 编码 → 分帧压缩 → 解析 → NoteLoader，内容与 id 一致。
    #[test]
    fn encode_open_feed_roundtrip() {
        let mut t0 = TrackData::new(0, 0);
        t0.name = "A".into();
        let mut t1 = TrackData::new(0, 1);
        t1.name = "B".into();
        let n0 = vec![
            NoteEvent {
                id: 1000,
                start_tick: 0,
                end_tick: 10,
                key: 60,
                velocity: 100,
            },
            NoteEvent {
                id: 5,
                start_tick: 10,
                end_tick: 20,
                key: 61,
                velocity: 90,
            },
        ];
        let n1 = vec![NoteEvent {
            id: 777,
            start_tick: 5,
            end_tick: 15,
            key: 40,
            velocity: 80,
        }];
        let mut m1 = YinModel {
            tracks: vec![Arc::new(t0), Arc::new(t1)],
            ..Default::default()
        };
        m1.load_track_notes(vec![n0, n1]);
        m1.rebuild();

        let (notes, segments) = encode_note_streams(&m1, 3, &mut |_| {}).unwrap();
        let meta = MetaPayload {
            conductor: ConductorData::default(),
            tracks: Vec::new(),
            segments,
        };
        let data = compress_data(&meta, notes, 3, &mut |_| {}).unwrap();

        let (meta2, mut readers) = open_note_streams(&data, &mut |_| {}).unwrap();
        assert_eq!(meta2.segments.len(), 2, "两个非空轨各一段");
        let counts = readers.key_counts().unwrap();
        assert_eq!(counts.iter().sum::<u32>(), 3);

        let mut loader = NoteLoader::new(2, 1, counts);
        readers
            .feed(&meta2.segments, &mut loader, &mut |_| {})
            .unwrap();
        let mut m2 = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        loader.finish(&mut m2);

        let snap = |m: &YinModel| {
            let mut v: Vec<(u16, u32, u32, u8, u32)> = m
                .notes
                .iter()
                .enumerate()
                .flat_map(|(k, b)| {
                    b.iter()
                        .map(move |n| (n.track, n.start_tick, n.id, k as u8, n.end_tick))
                })
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(snap(&m1), snap(&m2), "音符与 id 应逐条一致");
        assert_eq!(m2.next_note_id, 1001);
    }
}
