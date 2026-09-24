//! Top-level save / load API.
//!
//! `save_yin(model, path)` and `load_yin(path)` are the public entry points.
//! `save_yin_bytes(model)` / `load_yin_bytes(bytes)` operate on memory
//! buffers (used by tests and for streaming).

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use yinhe_core::{BucketNote, ConductorData, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_types::{AutomationLane, KEY_COUNT};

use yinhe_mixer::MixerParams;

use crate::container::{Sections, pack, unpack};
use crate::error::YinError;
use crate::mapping::MappingFile;
use crate::project_meta::{ProjectFile, SfChannelOverride};

/// 混音段格式版本（段内前 4 字节）。MixerParams 字段演进时递增，
/// 加载侧版本不符则忽略混音段（工程本体照常打开）。
///
/// v2：新增 instruments（当时按乐器通道索引）。加载 v1（无该字段）时按旧结构
/// 解码并补空；v5 起 instruments 改为按 MIDI 通道索引（旧语义作废，迁移时清空）。
///
/// v3：新增 buses / bus_inserts / sends（总线与发送）。加载 v1/v2 时补空。
///
/// v4：新增乐器通道 strip/inserts/sends 与音频通道 strip/inserts/sends
///（乐器通道命名空间已废弃，v4 不再支持）。
///
/// v5：乐器挂载统一到 MIDI 通道：删除 instrument_strips/inserts/sends，
/// instruments 固定 CHANNEL_COUNT 长度（None = 内置 XSynth）。
const MIXER_SECTION_VERSION: u32 = 5;

/// v1 混音段结构（无 instruments 字段），供旧版工程迁移解码。
/// 字段必须与编码顺序完整对齐（postcard 非自描述），未用字段允许 dead_code。
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MixerParamsV1 {
    pub channels: Vec<yinhe_mixer::StripParams>,
    pub master: yinhe_mixer::MasterParams,
    pub channel_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub master_inserts: Vec<yinhe_mixer::InsertRef>,
}

/// v2 混音段结构（无 buses/bus_inserts/sends），供旧版工程迁移解码。
/// `instruments` 是旧乐器通道语义，迁移时丢弃。
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MixerParamsV2 {
    pub channels: Vec<yinhe_mixer::StripParams>,
    pub master: yinhe_mixer::MasterParams,
    pub channel_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub master_inserts: Vec<yinhe_mixer::InsertRef>,
    pub instruments: Vec<Option<yinhe_mixer::InsertRef>>,
}

/// v3 混音段结构（无乐器/音频通道 strip 表），供旧版工程迁移解码。
/// `instruments` 是旧乐器通道语义，迁移时丢弃。
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MixerParamsV3 {
    pub channels: Vec<yinhe_mixer::StripParams>,
    pub master: yinhe_mixer::MasterParams,
    pub channel_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub master_inserts: Vec<yinhe_mixer::InsertRef>,
    pub instruments: Vec<Option<yinhe_mixer::InsertRef>>,
    pub buses: Vec<yinhe_mixer::StripParams>,
    pub bus_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub sends: Vec<Vec<yinhe_mixer::SendParams>>,
}

/// 编码混音段：version u32 LE + zstd(postcard MixerParams)。
fn encode_mixer_section(mixer: &MixerParams, level: i32) -> Result<Vec<u8>, YinError> {
    let payload = serialize_postcard(mixer)?;
    let comp = zstd::encode_all(Cursor::new(&payload), level.clamp(0, 22))?;
    let mut out = Vec::with_capacity(4 + comp.len());
    out.extend_from_slice(&MIXER_SECTION_VERSION.to_le_bytes());
    out.extend_from_slice(&comp);
    Ok(out)
}

/// 解码混音段。版本不符或损坏时记日志并返回 None（不阻断工程加载）。
fn decode_mixer_section(section: &[u8]) -> Option<MixerParams> {
    if section.len() < 4 {
        return None;
    }
    let version = u32::from_le_bytes(section[..4].try_into().ok()?);
    let payload = zstd::decode_all(Cursor::new(&section[4..])).ok()?;
    let mut params: MixerParams = match version {
        // 当前版本：完整结构（含乐器/音频通道 strip 表）。
        MIXER_SECTION_VERSION => deserialize_postcard(&payload)
            .map_err(|e| tracing::warn!("混音段解析失败，忽略混音设置: {e}"))
            .ok()?,
        // 旧版 v3：按旧结构解码；乐器表是旧通道语义、随命名空间废弃清空
        //（其它设置保留）。postcard 必须整段对齐，不能跳过未知字段。
        3 => {
            let v3: MixerParamsV3 = deserialize_postcard(&payload)
                .map_err(|e| tracing::warn!("旧版混音段解析失败，忽略混音设置: {e}"))
                .ok()?;
            MixerParams {
                channels: v3.channels,
                master: v3.master,
                channel_inserts: v3.channel_inserts,
                master_inserts: v3.master_inserts,
                instruments: Vec::new(),
                audio_channels: Vec::new(),
                audio_inserts: Vec::new(),
                audio_sends: Vec::new(),
                buses: v3.buses,
                bus_inserts: v3.bus_inserts,
                sends: v3.sends,
            }
        }
        // 旧版 v2：总线/发送留空（不丢其它混音设置）。
        2 => {
            let v2: MixerParamsV2 = deserialize_postcard(&payload)
                .map_err(|e| tracing::warn!("旧版混音段解析失败，忽略混音设置: {e}"))
                .ok()?;
            MixerParams {
                channels: v2.channels,
                master: v2.master,
                channel_inserts: v2.channel_inserts,
                master_inserts: v2.master_inserts,
                instruments: Vec::new(),
                audio_channels: Vec::new(),
                audio_inserts: Vec::new(),
                audio_sends: Vec::new(),
                buses: Vec::new(),
                bus_inserts: Vec::new(),
                sends: Vec::new(),
            }
        }
        // 旧版 v1：乐器表与总线留空（不丢其它混音设置）。
        1 => {
            let v1: MixerParamsV1 = deserialize_postcard(&payload)
                .map_err(|e| tracing::warn!("旧版混音段解析失败，忽略混音设置: {e}"))
                .ok()?;
            MixerParams {
                channels: v1.channels,
                master: v1.master,
                channel_inserts: v1.channel_inserts,
                master_inserts: v1.master_inserts,
                instruments: Vec::new(),
                audio_channels: Vec::new(),
                audio_inserts: Vec::new(),
                audio_sends: Vec::new(),
                buses: Vec::new(),
                bus_inserts: Vec::new(),
                sends: Vec::new(),
            }
        }
        other => {
            tracing::warn!("混音段版本 {other} 不受支持，忽略混音设置");
            return None;
        }
    };
    params.ensure_len();
    Some(params)
}

/// 保存/加载的进度阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YinProgressStage {
    /// 保存：检查桶序（乱序桶兜底排序）
    Collect,
    /// 保存：KEY_COUNT 路归并 + 列编码（可连续汇报）
    Sort,
    /// 保存：zstd 压缩（6 个流）
    Compress,
    /// 加载：zstd 解压（6 个流）
    Decompress,
    /// 加载：按 key 分桶还原音符
    Rebuild,
    /// 加载：桶内按 start 排序
    Resort,
}

/// 进度回调载荷：阶段 + 阶段内进度 0.0~1.0。
#[derive(Clone, Copy, Debug)]
pub struct YinProgress {
    pub stage: YinProgressStage,
    pub fraction: f32,
}

/// postcard 编码（varint，紧凑二进制）。
fn serialize_postcard<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, YinError> {
    Ok(postcard::to_stdvec(v)?)
}

fn deserialize_postcard<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, YinError> {
    Ok(postcard::from_bytes(bytes)?)
}

/// SoundFont state attached to a project（每源通道覆盖）。
///
/// This is what `save_yin_with_sf` consumes and `load_yin_with_sf` returns.
/// 未列出的通道使用全局音色库。
#[derive(Debug, Clone, Default)]
pub struct ProjectSoundFonts {
    pub overrides: Vec<SfChannelOverride>,
}

// =========================================================
//  v8 轨段列式音符格式
// =========================================================

/// 非音符部分（conductor + tracks payload + 轨段表），整体 postcard + zstd。
#[derive(Serialize, Deserialize)]
struct MetaPayload {
    conductor: ConductorData,
    tracks: Vec<TrackPayload>,
    /// v8：音符流按 (track, start, key) 分段的段表（track 升序，跳过空轨）。
    segments: Vec<TrackSegment>,
}

/// 轨段头：track-major 布局下每段对应一个轨道，`count` 为该段音符数。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct TrackSegment {
    track: u16,
    count: u32,
}

/// 写 varint（LEB128）。
fn push_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

/// 读 varint（LEB128），返回 (值, 新游标)。
fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, YinError> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    loop {
        let Some(&b) = bytes.get(*pos) else {
            return Err(YinError::Truncated {
                needed: 1,
                available: 0,
            });
        };
        *pos += 1;
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

/// zigzag：有符号 → 无符号（小绝对值映射到小值）。
fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// zigzag 逆变换。
fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

#[derive(Serialize, Deserialize)]
struct TrackPayload {
    uuid: String,
    automation_lanes: Vec<AutomationLane>,
    program_change: Vec<PcEvent>,
    #[serde(default)]
    lyrics: Vec<yinhe_types::LyricsEvent>,
    #[serde(default)]
    chord: Vec<yinhe_types::ChordEvent>,
}

// =========================================================
//  音频素材段
// =========================================================

/// 音频段格式版本（段内前 4 字节）。素材结构演进时递增。
const AUDIO_SECTION_VERSION: u32 = 1;

/// 单个内嵌音频素材的持久化形态。
#[derive(Serialize, Deserialize)]
struct AudioSourcePayload {
    uuid: String,
    name: String,
    /// 原始文件字节（wav/mp3/flac/ogg/m4a）。
    data: Vec<u8>,
    duration_seconds: f64,
}

#[derive(Serialize, Deserialize)]
struct AudioPayload {
    sources: Vec<AudioSourcePayload>,
}

/// 编码音频段：version u32 LE + zstd(postcard 素材表)。
/// 无音频素材时返回 None（不写段，保持旧文件布局）。
fn encode_audio_section(model: &YinModel, level: i32) -> Result<Option<Vec<u8>>, YinError> {
    if model.audio_sources.is_empty() {
        return Ok(None);
    }
    let payload = AudioPayload {
        sources: model
            .audio_sources
            .iter()
            .map(|s| AudioSourcePayload {
                uuid: s.uuid.clone(),
                name: s.name.clone(),
                data: (*s.data).clone(),
                duration_seconds: s.duration_seconds,
            })
            .collect(),
    };
    let raw = serialize_postcard(&payload)?;
    let comp = zstd::encode_all(Cursor::new(&raw), level.clamp(0, 22))?;
    let mut out = Vec::with_capacity(4 + comp.len());
    out.extend_from_slice(&AUDIO_SECTION_VERSION.to_le_bytes());
    out.extend_from_slice(&comp);
    Ok(Some(out))
}

/// 解码音频段。版本不符或损坏时记日志并返回空表（不阻断工程加载，
/// 但片段引用会找不到素材 —— 与文件损坏同级的降级）。
fn decode_audio_section(section: &[u8]) -> Vec<Arc<yinhe_core::AudioSource>> {
    if section.len() < 4 {
        return Vec::new();
    }
    let version = u32::from_le_bytes(match section[..4].try_into() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    });
    if version != AUDIO_SECTION_VERSION {
        tracing::warn!("音频段版本 {version} 不受支持，忽略内嵌音频");
        return Vec::new();
    }
    let payload = match zstd::decode_all(Cursor::new(&section[4..])) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("音频段解压失败，忽略内嵌音频: {e}");
            return Vec::new();
        }
    };
    let payload: AudioPayload = match deserialize_postcard(&payload) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("音频段解析失败，忽略内嵌音频: {e}");
            return Vec::new();
        }
    };
    payload
        .sources
        .into_iter()
        .map(|s| {
            Arc::new(yinhe_core::AudioSource {
                uuid: s.uuid,
                name: s.name,
                data: Arc::new(s.data),
                duration_seconds: s.duration_seconds,
            })
        })
        .collect()
}

/// 5 个列式音符流（按 (track, start, key) 排序后各字段独立成流）。
///
/// 黑乐谱的重复单元是「单轨内乐句复现」，轨内串行后重复在轨内近距离匹配；
/// 再按字段拆列，zstd 对每列各自达到最佳匹配（交错流会被其他字段稀释）。
/// 实测 start.mid 1.64 亿音符：v7 全局排序列式 38.9MB → 轨段列式
/// 5.0MB（zstd3，-87%）；Broken World 4444 万音符 10.1MB → 2.2MB。
#[derive(Default)]
struct NoteStreams {
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

fn progress(on_progress: &mut dyn FnMut(YinProgress), stage: YinProgressStage, fraction: f32) {
    on_progress(YinProgress { stage, fraction });
}

/// 保存侧：KEY_COUNT 路归并（桶内已按 start 有序）输出 (track, start, key)
/// 轨段列式流。
///
/// 归并序仍是全局 (start, track, key)（O(N log KEY_COUNT)，比全量排序快
/// 3-4 倍）；归并时按 track 直接定位写入预分配的 SoA（先扫一遍数出每轨
/// 音符数），段内顺序自然为 (start, key)——省掉一次按 track 的全量排序与
/// order 数组。乱序桶兜底本地排序（模型不变量，正常不触发）。
fn encode_note_streams(
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
        let std::cmp::Reverse(e) = heap.pop().expect("heap must stay full until total");
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
fn compress_data(
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
fn decompress_data(
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
        let len = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
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
        return Err(invalid_data(&format!(
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
fn bucket_from_streams(
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
            return Err(invalid_data(&format!(
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
        return Err(invalid_data(&format!(
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

/// 构造 InvalidData 错误。
fn invalid_data(msg: &str) -> YinError {
    YinError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        msg.to_string(),
    ))
}

// =========================================================
//  Save
// =========================================================

/// Internal: serialize a model with optional SF state attached.
fn save_yin_bytes_inner(
    model: &YinModel,
    sf: Option<&ProjectSoundFonts>,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<u8>, YinError> {
    // 1. project.json (with or without SF state)
    let project = match sf {
        Some(sf) => ProjectFile::from_meta_with_sf(&model.meta, sf.overrides.clone()),
        None => ProjectFile::from_meta(&model.meta),
    };
    let mapping = MappingFile::from_tracks(&model.tracks);
    save_yin_bytes_with_files_inner(model, &project, &mapping, None, on_progress)
}

/// Internal: serialize with pre-built ProjectFile and MappingFile.
fn save_yin_bytes_with_files_inner(
    model: &YinModel,
    project: &ProjectFile,
    mapping: &MappingFile,
    mixer: Option<&MixerParams>,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<u8>, YinError> {
    let project_json = serde_json::to_vec_pretty(project)?;
    let mapping_json = serde_json::to_vec_pretty(mapping)?;

    // data 段：meta（conductor + tracks payload + 轨段表）+ 5 列音符流，
    // 每流独立 zstd。音符 id 以 zigzag varint delta 落盘（与轨段布局同序，
    // 压缩后近零开销），加载时保留。
    let (notes, segments) = encode_note_streams(model, on_progress)?;
    let meta = MetaPayload {
        conductor: (*model.conductor).clone(),
        // payload 按 model.tracks 顺序写，与音符流的 track 索引（model 索引）同空间；
        // 加载侧用 uuid 与 mapping 关联，不依赖 mapping 的存储顺序。
        // 曾经按 mapping.flat_tracks() 顺序写，而音符流仍是 model 索引，
        // 两个索引空间不一致，音轨顺序与 model 不同时保存→加载即错位。
        tracks: model
            .tracks
            .iter()
            .map(|t| TrackPayload {
                uuid: t.uuid.clone(),
                automation_lanes: t.automation_lanes.clone(),
                program_change: t.program_change.clone(),
                lyrics: t.lyrics.clone(),
                chord: t.chord.clone(),
            })
            .collect(),
        segments,
    };
    let data = compress_data(&meta, notes, model.meta.compression_level, on_progress)?;

    let mixer = mixer
        .map(|m| encode_mixer_section(m, model.meta.compression_level))
        .transpose()?;
    let audio = encode_audio_section(model, model.meta.compression_level)?;
    let bytes = pack(Sections {
        project_json,
        mapping_json,
        data,
        mixer,
        audio,
    });
    Ok(bytes)
}

/// Serialize a `YinModel` to `.yin` bytes (no SoundFont state).
pub fn save_yin_bytes(model: &YinModel) -> Result<Vec<u8>, YinError> {
    save_yin_bytes_inner(model, None, &mut |_| {})
}

/// Serialize a `YinModel` plus its SoundFont state to `.yin` bytes.
pub fn save_yin_bytes_with_sf(
    model: &YinModel,
    sf: &ProjectSoundFonts,
) -> Result<Vec<u8>, YinError> {
    save_yin_bytes_inner(model, Some(sf), &mut |_| {})
}

/// Save a `YinModel` to a file at `path` (no SoundFont state).
pub fn save_yin(model: &YinModel, path: impl AsRef<Path>) -> Result<(), YinError> {
    let bytes = save_yin_bytes(model)?;
    std::fs::write(path.as_ref(), &bytes)?;
    Ok(())
}

/// Save a `YinModel` plus its SoundFont state to a file at `path`.
pub fn save_yin_with_sf(
    model: &YinModel,
    path: impl AsRef<Path>,
    sf: &ProjectSoundFonts,
) -> Result<(), YinError> {
    let bytes = save_yin_bytes_with_sf(model, sf)?;
    std::fs::write(path.as_ref(), &bytes)?;
    Ok(())
}

/// Save using pre-built `ProjectFile` and `MappingFile` (faithful round-trip).
pub fn save_yin_with_files(
    model: &YinModel,
    path: impl AsRef<Path>,
    project: &ProjectFile,
    mapping: &MappingFile,
    mixer: Option<&MixerParams>,
) -> Result<(), YinError> {
    save_yin_with_files_progress(model, path, project, mapping, mixer, |_| {})
}

/// `save_yin_with_files` + 进度回调（后台线程保存时用于驱动 UI 进度条）。
pub fn save_yin_with_files_progress(
    model: &YinModel,
    path: impl AsRef<Path>,
    project: &ProjectFile,
    mapping: &MappingFile,
    mixer: Option<&MixerParams>,
    mut on_progress: impl FnMut(YinProgress) + Send,
) -> Result<(), YinError> {
    let bytes = save_yin_bytes_with_files_inner(model, project, mapping, mixer, &mut on_progress)?;
    std::fs::write(path.as_ref(), &bytes)?;
    Ok(())
}

// =========================================================
//  Load
// =========================================================

/// Internal: parse `.yin` bytes, returning model and the raw `ProjectFile`
/// (so callers can extract SF state if they want it).
fn load_yin_bytes_inner(
    bytes: &[u8],
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<(YinModel, ProjectFile, MappingFile, Option<MixerParams>), YinError> {
    let sections = unpack(bytes)?;
    let mixer = sections.mixer.as_deref().and_then(decode_mixer_section);
    let audio_sources = sections
        .audio
        .as_deref()
        .map(decode_audio_section)
        .unwrap_or_default();

    let project: ProjectFile = serde_json::from_slice(&sections.project_json)?;
    let mapping: MappingFile = serde_json::from_slice(&sections.mapping_json)?;

    let (model_data, note_streams) = decompress_data(&sections.data, on_progress)?;

    // 按 payload 顺序（保存时的 model.tracks 顺序）重建 TrackData；
    // 音轨的 port/channel/元数据取自 mapping（uuid 关联），
    // 不再依赖 mapping 的存储顺序——mapping 的 ports/channels 嵌套分组
    // 无法表达 model 的全局音轨顺序（同 port 音轨必须连续存放）。
    let flat: Vec<(u8, u8, &crate::mapping::TrackMap)> = mapping.flat_tracks().collect();
    if flat.len() != model_data.tracks.len() {
        return Err(invalid_data(&format!(
            "mapping has {} tracks but data has {}",
            flat.len(),
            model_data.tracks.len()
        )));
    }
    let mut by_uuid: std::collections::HashMap<&str, (u8, u8, &crate::mapping::TrackMap)> =
        std::collections::HashMap::with_capacity(flat.len());
    for (port, channel, tm) in flat {
        by_uuid.insert(tm.uuid.as_str(), (port, channel, tm));
    }

    let mut tracks: Vec<Arc<TrackData>> = Vec::with_capacity(model_data.tracks.len());
    for payload in model_data.tracks {
        let Some(&(port, channel, tm)) = by_uuid.get(payload.uuid.as_str()) else {
            return Err(invalid_data(&format!(
                "track UUID mismatch: mapping has no track with payload uuid {}",
                payload.uuid
            )));
        };
        let td = TrackData {
            uuid: tm.uuid.clone(),
            name: tm.name.clone(),
            color: tm.color,
            port,
            channel,
            channel_prefix: tm.channel_prefix,
            muted: tm.muted,
            soloed: tm.soloed,
            kind: tm.kind,
            audio_channel: tm.audio_channel,
            audio_clips: tm.audio_clips.clone(),
            notes: Vec::new(), // notes loaded via load_bucket_notes
            automation_lanes: payload.automation_lanes,
            program_change: payload.program_change,
            lyrics: payload.lyrics,
            chord: payload.chord,
        };
        tracks.push(Arc::new(td));
    }

    // 轨段流 → KEY_COUNT 桶（桶内按 start 排序），再由 load_bucket_notes 入模型。
    // id 已随流落盘（非 0 保留，跨会话稳定）。
    let bucket_notes = bucket_from_streams(&note_streams, &model_data.segments, on_progress)?;

    let mut model = YinModel {
        conductor: Arc::new(model_data.conductor),
        tracks,
        meta: ProjectMeta {
            name: project.name.clone(),
            artist: project.artist.clone(),
            description: project.description.clone(),
            ppq: project.ppq,
            compression_level: project.compression_level,
        },
        audio_sources,
        ..Default::default()
    };
    model.load_bucket_notes(bucket_notes);
    // 自动化事件 id 不落盘：加载后统一发号（会话内身份，选择集/undo 用）。
    model.renumber_automation_ids();
    model.rebuild();
    // 片段 id 发号器推进到已用最大值 +1（避免新建片段撞 id）。
    model.next_audio_clip_id = model
        .tracks
        .iter()
        .flat_map(|t| t.audio_clips.iter().map(|c| c.id))
        .max()
        .map(|m| m.saturating_add(1))
        .unwrap_or(1);
    Ok((model, project, mapping, mixer))
}

/// Parse `.yin` bytes into a `YinModel` (SoundFont state, if any, is dropped).
pub fn load_yin_bytes(bytes: &[u8]) -> Result<YinModel, YinError> {
    let (model, _project, _mapping, _mixer) = load_yin_bytes_inner(bytes, &mut |_| {})?;
    Ok(model)
}

/// Parse `.yin` bytes into a `YinModel` and its SoundFont state.
///
/// For files written before SF persistence, `ProjectSoundFonts` will be
/// `default()` (mode = false, overrides empty).
pub fn load_yin_bytes_with_sf(
    bytes: &[u8],
) -> Result<(YinModel, ProjectSoundFonts, MappingFile), YinError> {
    let (model, project, mapping, _mixer) = load_yin_bytes_inner(bytes, &mut |_| {})?;
    let sf = ProjectSoundFonts {
        overrides: project.sf_channel_overrides,
    };
    Ok((model, sf, mapping))
}

/// Load a `.yin` file from `path` (SoundFont state, if any, is dropped).
pub fn load_yin(path: impl AsRef<Path>) -> Result<YinModel, YinError> {
    let bytes = std::fs::read(path.as_ref())?;
    load_yin_bytes(&bytes)
}

/// Load a `.yin` file from `path`, returning the model and its SoundFont state.
pub fn load_yin_with_sf(
    path: impl AsRef<Path>,
) -> Result<
    (
        YinModel,
        ProjectSoundFonts,
        MappingFile,
        Option<MixerParams>,
    ),
    YinError,
> {
    load_yin_with_sf_progress(path, |_| {})
}

/// `load_yin_with_sf` + 进度回调（后台线程加载时用于驱动 UI 进度条）。
pub fn load_yin_with_sf_progress(
    path: impl AsRef<Path>,
    mut on_progress: impl FnMut(YinProgress) + Send,
) -> Result<
    (
        YinModel,
        ProjectSoundFonts,
        MappingFile,
        Option<MixerParams>,
    ),
    YinError,
> {
    let bytes = std::fs::read(path.as_ref())?;
    let (model, project, mapping, mixer) = load_yin_bytes_inner(&bytes, &mut on_progress)?;
    let sf = ProjectSoundFonts {
        overrides: project.sf_channel_overrides,
    };
    Ok((model, sf, mapping, mixer))
}
