//! data 段（.yin 第 3 段）编解码：meta 流 + 5 个轨段列式音符流。
//!
//! ```text
//! 0: postcard(conductor + tracks payload + segments)  ← 非音符部分
//! 1: delta 列（varint u32：轨段首为绝对 start，其余 = start - prev）
//! 2: key   列（u8 × N）
//! 3: vel   列（u8 × N）
//! 4: gate  列（varint u32）
//! 5: id delta 列（zigzag varint 字节流，跨段连续累加）
//! ```

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use yinhe_core::{BucketNote, ConductorData, PcEvent, YinModel};
use yinhe_types::{AutomationLane, KEY_COUNT};

use crate::codec::{
    deserialize_postcard, push_varint, read_varint, serialize_postcard, unzigzag, zigzag,
};
use crate::error::{YinError, invalid_data};
use crate::progress::{YinProgress, YinProgressStage, progress};

/// 非音符部分（conductor + tracks payload + 轨段表），整体 postcard + zstd。
#[derive(Serialize, Deserialize)]
pub(crate) struct MetaPayload {
    pub(crate) conductor: ConductorData,
    pub(crate) tracks: Vec<TrackPayload>,
    /// v8：音符流按 (track, start, key) 分段的段表（track 升序，跳过空轨）。
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

/// 5 个列式音符流（按 (track, start, key) 排序后各字段独立成流）。
///
/// 黑乐谱的重复单元是「单轨内乐句复现」，轨内串行后重复在轨内近距离匹配；
/// 再按字段拆列，zstd 对每列各自达到最佳匹配（交错流会被其他字段稀释）。
/// 实测 start.mid 1.64 亿音符：v7 全局排序列式 38.9MB → 轨段列式
/// 5.0MB（zstd3，-87%）；Broken World 4444 万音符 10.1MB → 2.2MB。
#[derive(Default)]
pub(crate) struct NoteStreams {
    /// 轨段首音符为绝对 start，其余 = start - prev（段内单调）
    delta: Vec<u32>,
    key: Vec<u8>,
    vel: Vec<u8>,
    gate: Vec<u32>,
    /// id 的 zigzag varint delta 字节流（跨段连续累加）。
    /// 导入时 id 按 track 顺序分配，与轨段布局同序 → delta 恒为 1。
    id_delta: Vec<u8>,
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

/// 保存侧：KEY_COUNT 路归并（桶内已按 start 有序）输出 (track, start, key)
/// 轨段列式流。
///
/// 归并序仍是全局 (start, track, key)（O(N log KEY_COUNT)，比全量排序快
/// 3-4 倍）；归并时按 track 直接定位写入预分配的 SoA（先扫一遍数出每轨
/// 音符数），段内顺序自然为 (start, key)——省掉一次按 track 的全量排序与
/// order 数组。乱序桶兜底本地排序（模型不变量，正常不触发）。
pub(crate) fn encode_note_streams(
    model: &YinModel,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<(NoteStreams, Vec<TrackSegment>), YinError> {
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

    // 预扫描：每轨音符数 → 段表 + 目标偏移（归并时按 track 定位写入）。
    let mut track_counts: Vec<u32> = Vec::new();
    for bucket in model.notes.iter() {
        for n in bucket.iter() {
            let t = n.track as usize;
            if t >= track_counts.len() {
                track_counts.resize(t + 1, 0);
            }
            track_counts[t] += 1;
        }
    }
    let mut offsets: Vec<u32> = vec![0; track_counts.len()];
    let mut segments: Vec<TrackSegment> = Vec::new();
    let mut acc: u32 = 0;
    for (t, &c) in track_counts.iter().enumerate() {
        offsets[t] = acc;
        if c > 0 {
            segments.push(TrackSegment {
                track: t as u16,
                count: c,
            });
        }
        acc += c;
    }
    debug_assert_eq!(acc as usize, total, "track counts must sum to total notes");

    // 目标 SoA：按段区间顺序写入（同一 track 的写入递增，段内即 (start, key) 序）。
    let mut starts = vec![0u32; total];
    let mut tracks = vec![0u16; total];
    let mut keys = vec![0u8; total];
    let mut vels = vec![0u8; total];
    let mut gates = vec![0u32; total];
    let mut ids = vec![0u32; total];
    let mut cursor = offsets;

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
        let pos = cursor[e.track as usize] as usize;
        cursor[e.track as usize] += 1;
        starts[pos] = n.start_tick;
        tracks[pos] = n.track;
        keys[pos] = e.key;
        vels[pos] = n.velocity;
        gates[pos] = n.end_tick.saturating_sub(n.start_tick);
        ids[pos] = n.id;
        if i & PROGRESS_MASK == 0 {
            progress(
                on_progress,
                YinProgressStage::Sort,
                (i as f32 + 1.0) / total as f32,
            );
        }
    }

    // 按段顺序输出：delta 原地覆盖 starts（段首绝对、其余差值），
    // id 转 zigzag varint 字节流（跨段连续累加）。
    let mut id_delta = Vec::with_capacity(total);
    let mut prev_start = 0u32;
    let mut prev_track = u16::MAX;
    let mut prev_id: i64 = 0;
    for i in 0..total {
        if tracks[i] != prev_track {
            prev_track = tracks[i];
            prev_start = 0;
        }
        let start = starts[i];
        starts[i] = start - prev_start;
        prev_start = start;
        let id = ids[i] as i64;
        push_varint(&mut id_delta, zigzag(id - prev_id));
        prev_id = id;
    }
    progress(on_progress, YinProgressStage::Sort, 1.0);
    Ok((
        NoteStreams {
            delta: starts,
            key: keys,
            vel: vels,
            gate: gates,
            id_delta,
        },
        segments,
    ))
}

/// 保存侧：meta 流 + 5 个音符流，各自 zstd，打包成 data 段。
pub(crate) fn compress_data(
    meta: &MetaPayload,
    notes: NoteStreams,
    level: i32,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<u8>, YinError> {
    let level = level.clamp(0, 22);
    let plains: [Vec<u8>; 6] = [
        serialize_postcard(meta)?,
        serialize_postcard(&notes.delta)?,
        serialize_postcard(&notes.key)?,
        serialize_postcard(&notes.vel)?,
        serialize_postcard(&notes.gate)?,
        notes.id_delta, // 已是 zigzag varint 字节流，无需 postcard
    ];
    let mut out = Vec::new();
    for (i, plain) in plains.into_iter().enumerate() {
        let comp = zstd::encode_all(Cursor::new(&plain), level)?;
        out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        out.extend_from_slice(&comp);
        progress(
            on_progress,
            YinProgressStage::Compress,
            (i as f32 + 1.0) / 6.0,
        );
    }
    Ok(out)
}

/// 加载侧：data 段 → meta 流 + 5 个音符流（delta/key/vel/gate/id_delta）。
pub(crate) fn decompress_data(
    data: &[u8],
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<(MetaPayload, NoteStreams), YinError> {
    let mut plains: Vec<Vec<u8>> = Vec::with_capacity(6);
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
        plains.push(zstd::decode_all(Cursor::new(&data[off..off + len]))?);
        off += len;
        progress(
            on_progress,
            YinProgressStage::Decompress,
            (i as f32 + 1.0) / 6.0,
        );
    }
    let [meta_p, delta_p, key_p, vel_p, gate_p, id_p]: [Vec<u8>; 6] = plains
        .try_into()
        .map_err(|_| invalid_data("data section must contain exactly 6 streams"))?;
    let meta: MetaPayload = deserialize_postcard(&meta_p)?;
    let delta: Vec<u32> = deserialize_postcard(&delta_p)?;
    let key: Vec<u8> = deserialize_postcard(&key_p)?;
    let vel: Vec<u8> = deserialize_postcard(&vel_p)?;
    let gate: Vec<u32> = deserialize_postcard(&gate_p)?;

    let n = key.len();
    if delta.len() != n || vel.len() != n || gate.len() != n {
        return Err(invalid_data(format!(
            "note stream length mismatch: delta={} key={} vel={} gate={}",
            delta.len(),
            n,
            vel.len(),
            gate.len()
        )));
    }
    let s = NoteStreams {
        delta,
        key,
        vel,
        gate,
        id_delta: id_p,
    };
    Ok((meta, s))
}

/// 加载侧：轨段表 + 5 个音符流 → KEY_COUNT 个 key 桶（桶内按 start 排序）。
pub(crate) fn bucket_from_streams(
    s: &NoteStreams,
    segments: &[TrackSegment],
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<Vec<BucketNote>>, YinError> {
    let n = s.key.len();
    let mut buckets: Vec<Vec<BucketNote>> = Vec::with_capacity(KEY_COUNT);
    for _ in 0..KEY_COUNT {
        buckets.push(Vec::new());
    }

    let mut i = 0usize;
    let mut id_pos = 0usize;
    let mut prev_id: i64 = 0;
    for seg in segments {
        let end = i
            .checked_add(seg.count as usize)
            .ok_or_else(|| invalid_data("segment count overflow"))?;
        if end > n {
            return Err(invalid_data(format!(
                "segment count sum {} exceeds note count {n}",
                end
            )));
        }
        let mut prev_start: u32 = 0;
        while i < end {
            let start = prev_start
                .checked_add(s.delta[i])
                .ok_or_else(|| invalid_data("delta start overflow"))?;
            let id = prev_id + unzigzag(read_varint(&s.id_delta, &mut id_pos)?);
            if !(0..=u32::MAX as i64).contains(&id) {
                return Err(invalid_data("note id out of u32 range"));
            }
            buckets[s.key[i] as usize].push(BucketNote {
                id: id as u32,
                track: seg.track,
                start_tick: start,
                end_tick: start.saturating_add(s.gate[i]),
                velocity: s.vel[i],
            });
            prev_start = start;
            prev_id = id;
            i += 1;
            if i & PROGRESS_MASK == 0 {
                progress(on_progress, YinProgressStage::Rebuild, i as f32 / n as f32);
            }
        }
    }
    if i != n {
        return Err(invalid_data(format!(
            "segment count sum {i} does not match note count {n}"
        )));
    }
    progress(on_progress, YinProgressStage::Rebuild, 1.0);
    for (i, bucket) in buckets.iter_mut().enumerate() {
        bucket.sort_unstable_by_key(|x| x.start_tick);
        progress(
            on_progress,
            YinProgressStage::Resort,
            (i as f32 + 1.0) / KEY_COUNT as f32,
        );
    }
    Ok(buckets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_seg_streams() -> NoteStreams {
        NoteStreams {
            delta: vec![100, 10, 200, 5], // 段0: 100, 110；段1: 200, 205
            key: vec![60, 61, 60, 62],
            vel: vec![100, 90, 80, 70],
            gate: vec![10, 20, 30, 40],
            id_delta: vec![2, 2, 4, 2], // ids: 1, 2, 4, 5
        }
    }

    fn two_segments() -> Vec<TrackSegment> {
        vec![
            TrackSegment { track: 3, count: 2 },
            TrackSegment { track: 7, count: 2 },
        ]
    }

    #[test]
    fn bucket_from_streams_decodes_segments_and_ids() {
        let buckets =
            bucket_from_streams(&two_seg_streams(), &two_segments(), &mut |_| {}).unwrap();
        assert_eq!(buckets[60].len(), 2);
        assert_eq!(buckets[61].len(), 1);
        assert_eq!(buckets[62].len(), 1);
        // 桶内按 start 排序，track 取自段表，id 跨段连续累加。
        assert_eq!(buckets[60][0].start_tick, 100);
        assert_eq!(buckets[60][0].track, 3);
        assert_eq!(buckets[60][0].id, 1);
        assert_eq!(buckets[60][0].end_tick, 110);
        assert_eq!(buckets[60][1].start_tick, 200);
        assert_eq!(buckets[60][1].track, 7);
        assert_eq!(buckets[60][1].id, 4);
        assert_eq!(buckets[61][0].id, 2);
        assert_eq!(buckets[62][0].id, 5);
        assert_eq!(buckets[62][0].velocity, 70);
    }

    #[test]
    fn bucket_from_streams_rejects_count_mismatch() {
        let s = two_seg_streams();
        // count 总和 1 < 音符数 2
        let short = vec![TrackSegment { track: 0, count: 1 }];
        assert!(bucket_from_streams(&s, &short, &mut |_| {}).is_err());
        // count 总和 3 > 音符数 2
        let long = vec![TrackSegment { track: 0, count: 3 }];
        assert!(bucket_from_streams(&s, &long, &mut |_| {}).is_err());
    }

    #[test]
    fn bucket_from_streams_rejects_short_id_stream() {
        let mut s = two_seg_streams();
        s.id_delta.pop(); // 少一个 id delta
        assert!(bucket_from_streams(&s, &two_segments(), &mut |_| {}).is_err());
    }
}
