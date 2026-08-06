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
use yinhe_types::AutomationLane;

use crate::container::{Sections, pack, unpack};
use crate::error::YinError;
use crate::mapping::MappingFile;
use crate::project_meta::{ProjectFile, SfPortOverride};

/// 保存/加载的进度阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YinProgressStage {
    /// 保存：检查桶序（乱序桶兜底排序）
    Collect,
    /// 保存：128 路归并 + 列编码（可连续汇报）
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

/// bincode varint 编码（所有整数 LEB128）。
fn serialize_with_varint<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, YinError> {
    use bincode::Options;
    let config = bincode::DefaultOptions::new()
        .with_varint_encoding()
        .with_little_endian();
    Ok(config.serialize(v)?)
}

fn deserialize_with_varint<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, YinError> {
    use bincode::Options;
    let config = bincode::DefaultOptions::new()
        .with_varint_encoding()
        .with_little_endian();
    Ok(config.deserialize(bytes)?)
}

/// SoundFont state attached to a project (mode + per-port overrides).
///
/// This is what `save_yin_with_sf` consumes and `load_yin_with_sf` returns.
/// `mode = true` means the project was saved in per-port mode; `false`
/// means global mode (or the file predates SF persistence).
#[derive(Debug, Clone, Default)]
pub struct ProjectSoundFonts {
    pub mode: bool,
    pub overrides: Vec<SfPortOverride>,
}

// =========================================================
//  v5 列式音符格式
// =========================================================

/// 非音符部分（conductor + tracks payload），整体 bincode varint + zstd。
#[derive(Serialize, Deserialize)]
struct MetaPayload {
    conductor: ConductorData,
    tracks: Vec<TrackPayload>,
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

/// 5 个列式音符流（全局按 (start, track, key) 排序后各字段独立成流）。
///
/// 黑乐谱的重复单元是"同一 tick 内所有轨齐发的图案"，按 (start, track, key)
/// 排序后图案整块重复；再按字段拆列，zstd 对每列各自达到最佳匹配
/// （交错流会被其他字段稀释）。实测 start.mid 1.64 亿音符：
/// v4 key 桶 75MB → 列式 40.8MB（zstd3）/ 13.5MB（zstd19）。
#[derive(Default)]
struct NoteStreams {
    /// 第一音符绝对 start，其余 = start - prev（同 tick 相邻为 0）
    delta: Vec<u32>,
    key: Vec<u8>,
    track: Vec<u16>,
    vel: Vec<u8>,
    gate: Vec<u32>,
}

/// 归并堆元素：堆顶 = 当前 (start, track, key) 最小的桶游标。
/// `key` 即桶号（0-127），同 (start, track) 的不同桶 key 必不同，全序无歧义。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct HeapEntry {
    start: u32,
    track: u16,
    key: u8,
}

/// 每 1M 音符汇报一次进度的掩码。
const PROGRESS_MASK: usize = 0xF_FFFF;

fn progress(on_progress: &mut dyn FnMut(YinProgress), stage: YinProgressStage, fraction: f32) {
    on_progress(YinProgress { stage, fraction });
}

/// 保存侧：128 路归并（桶内已按 start 有序）输出列式流。
///
/// 全局 (start, track, key) 序 = 128 个有序桶的归并：O(N log 128)，
/// 比全量排序 O(N log N) 快 3-4 倍，且不需要 12B/音符的临时数组
/// （1.64 亿音符省 ~2.6GB 内存）；归并边输出边汇报，UI 进度条连续动，
/// 不再卡在 0%。乱序桶兜底本地排序（模型不变量，正常不触发）。
fn encode_note_streams(
    model: &YinModel,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<NoteStreams, YinError> {
    let total: usize = model.notes.iter().map(|b| b.len()).sum();

    // 兜底：乱序桶先本地排（不写回模型，只影响本次归并源）。
    let mut sorted_copies: Vec<Option<Vec<yinhe_types::Note>>> = Vec::with_capacity(128);
    for (key, bucket) in model.notes.iter().enumerate() {
        if bucket.is_sorted_by_key(|n| n.start_tick) {
            sorted_copies.push(None);
        } else {
            let mut b: Vec<yinhe_types::Note> = bucket.iter().copied().collect();
            b.sort_unstable_by_key(|n| n.start_tick);
            sorted_copies.push(Some(b));
        }
        progress(
            on_progress,
            YinProgressStage::Collect,
            (key as f32 + 1.0) / 128.0,
        );
    }
    let mut sources: Vec<&[yinhe_types::Note]> = Vec::with_capacity(128);
    for (key, bucket) in model.notes.iter().enumerate() {
        match &sorted_copies[key] {
            Some(c) => sources.push(c.as_slice()),
            None => sources.push(bucket.as_slice()),
        }
    }

    // 128 路归并：每桶一个游标在堆里，pop 最小 (start, track, key) 后
    // 推进该桶下一个。桶内按 start 有序（兜底已排），输出即全局序。
    let mut cur = [0usize; 128];
    let mut heap: std::collections::BinaryHeap<std::cmp::Reverse<HeapEntry>> =
        std::collections::BinaryHeap::with_capacity(128);
    for (key, src) in sources.iter().enumerate() {
        if let Some(first) = src.first() {
            heap.push(std::cmp::Reverse(HeapEntry {
                start: first.start_tick,
                track: first.track,
                key: key as u8,
            }));
        }
    }

    let mut s = NoteStreams::default();
    let (mut delta, mut key_v, mut track_v, mut vel_v, mut gate_v) = (
        Vec::with_capacity(total),
        Vec::with_capacity(total),
        Vec::with_capacity(total),
        Vec::with_capacity(total),
        Vec::with_capacity(total),
    );
    let mut prev_start: u32 = 0;
    for i in 0..total {
        let std::cmp::Reverse(e) = heap.pop().expect("heap must stay full until total");
        let key = e.key as usize;
        let n = sources[key][cur[key]];
        cur[key] += 1;
        if let Some(next) = sources[key].get(cur[key]) {
            heap.push(std::cmp::Reverse(HeapEntry {
                start: next.start_tick,
                track: next.track,
                key: e.key,
            }));
        }
        delta.push(if i == 0 {
            n.start_tick
        } else {
            n.start_tick - prev_start
        });
        key_v.push(e.key);
        track_v.push(n.track);
        vel_v.push(n.velocity);
        gate_v.push(n.end_tick.saturating_sub(n.start_tick));
        prev_start = n.start_tick;
        if i & PROGRESS_MASK == 0 {
            progress(
                on_progress,
                YinProgressStage::Sort,
                (i as f32 + 1.0) / total as f32,
            );
        }
    }
    progress(on_progress, YinProgressStage::Sort, 1.0);
    s.delta = delta;
    s.key = key_v;
    s.track = track_v;
    s.vel = vel_v;
    s.gate = gate_v;
    Ok(s)
}

/// 保存侧：meta 流 + 5 个音符流，各自 zstd，打包成 data 段。
fn compress_data(
    meta: &MetaPayload,
    notes: &NoteStreams,
    level: i32,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<u8>, YinError> {
    let level = level.clamp(0, 22);
    let plains: [Vec<u8>; 6] = [
        serialize_with_varint(meta)?,
        serialize_with_varint(&notes.delta)?,
        serialize_with_varint(&notes.key)?,
        serialize_with_varint(&notes.track)?,
        serialize_with_varint(&notes.vel)?,
        serialize_with_varint(&notes.gate)?,
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

/// 加载侧：data 段 → meta 流 + 5 个音符流。
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
    let meta: MetaPayload = deserialize_with_varint(&plains[0])?;
    let delta: Vec<u32> = deserialize_with_varint(&plains[1])?;
    let key: Vec<u8> = deserialize_with_varint(&plains[2])?;
    let track: Vec<u16> = deserialize_with_varint(&plains[3])?;
    let vel: Vec<u8> = deserialize_with_varint(&plains[4])?;
    let gate: Vec<u32> = deserialize_with_varint(&plains[5])?;

    let n = key.len();
    if delta.len() != n || track.len() != n || vel.len() != n || gate.len() != n {
        return Err(YinError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "note stream length mismatch: delta={} key={} track={} vel={} gate={}",
                delta.len(),
                n,
                track.len(),
                vel.len(),
                gate.len()
            ),
        )));
    }
    let s = NoteStreams {
        delta,
        key,
        track,
        vel,
        gate,
    };
    Ok((meta, s))
}

/// 加载侧：5 个音符流 → 128 个 key 桶（桶内按 start 排序）。
fn bucket_from_streams(
    s: &NoteStreams,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<Vec<BucketNote>>, YinError> {
    let n = s.key.len();
    let mut buckets: Vec<Vec<BucketNote>> = Vec::with_capacity(128);
    for _ in 0..128 {
        buckets.push(Vec::new());
    }
    let mut prev_start: u32 = 0;
    for i in 0..n {
        let start = if i == 0 {
            s.delta[0]
        } else {
            prev_start + s.delta[i]
        };
        buckets[s.key[i] as usize].push(BucketNote {
            track: s.track[i],
            start_tick: start,
            end_tick: start + s.gate[i],
            velocity: s.vel[i],
        });
        prev_start = start;
        if n > 0 && i & PROGRESS_MASK == 0 {
            progress(on_progress, YinProgressStage::Rebuild, i as f32 / n as f32);
        }
    }
    progress(on_progress, YinProgressStage::Rebuild, 1.0);
    for (i, bucket) in buckets.iter_mut().enumerate() {
        bucket.sort_unstable_by_key(|x| x.start_tick);
        progress(
            on_progress,
            YinProgressStage::Resort,
            (i as f32 + 1.0) / 128.0,
        );
    }
    Ok(buckets)
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
        Some(sf) => ProjectFile::from_meta_with_sf(&model.meta, sf.mode, sf.overrides.clone()),
        None => ProjectFile::from_meta(&model.meta),
    };
    let mapping = MappingFile::from_tracks(&model.tracks);
    save_yin_bytes_with_files_inner(model, &project, &mapping, on_progress)
}

/// Internal: serialize with pre-built ProjectFile and MappingFile.
fn save_yin_bytes_with_files_inner(
    model: &YinModel,
    project: &ProjectFile,
    mapping: &MappingFile,
    on_progress: &mut dyn FnMut(YinProgress),
) -> Result<Vec<u8>, YinError> {
    let project_json = serde_json::to_vec_pretty(project)?;
    let mapping_json = serde_json::to_vec_pretty(mapping)?;

    // data 段：meta（conductor + tracks payload）+ 5 列音符流，每流独立 zstd。
    // 音符 id 不落盘（会话内身份，全局递增 id 在 zstd 下几乎压不动），
    // 加载时由 load_bucket_notes 重新分配。
    let notes = encode_note_streams(model, on_progress)?;
    let meta = MetaPayload {
        conductor: (*model.conductor).clone(),
        tracks: {
            let mut by_uuid: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::with_capacity(model.tracks.len());
            for (i, t) in model.tracks.iter().enumerate() {
                by_uuid.insert(&t.uuid, i);
            }
            let mut tracks_payload = Vec::with_capacity(model.tracks.len());
            for (_port, _ch, tm) in mapping.flat_tracks() {
                if let Some(&track_idx) = by_uuid.get(tm.uuid.as_str()) {
                    let t = &model.tracks[track_idx];
                    tracks_payload.push(TrackPayload {
                        uuid: t.uuid.clone(),
                        automation_lanes: t.automation_lanes.clone(),
                        program_change: t.program_change.clone(),
                        lyrics: t.lyrics.clone(),
                        chord: t.chord.clone(),
                    });
                }
            }
            tracks_payload
        },
    };
    let data = compress_data(&meta, &notes, model.meta.compression_level, on_progress)?;

    let bytes = pack(Sections {
        project_json,
        mapping_json,
        data,
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
) -> Result<(), YinError> {
    save_yin_with_files_progress(model, path, project, mapping, |_| {})
}

/// `save_yin_with_files` + 进度回调（后台线程保存时用于驱动 UI 进度条）。
pub fn save_yin_with_files_progress(
    model: &YinModel,
    path: impl AsRef<Path>,
    project: &ProjectFile,
    mapping: &MappingFile,
    mut on_progress: impl FnMut(YinProgress) + Send,
) -> Result<(), YinError> {
    let bytes = save_yin_bytes_with_files_inner(model, project, mapping, &mut on_progress)?;
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
) -> Result<(YinModel, ProjectFile, MappingFile), YinError> {
    let sections = unpack(bytes)?;

    let project: ProjectFile = serde_json::from_slice(&sections.project_json)?;
    let mapping: MappingFile = serde_json::from_slice(&sections.mapping_json)?;

    let (model_data, note_streams) = decompress_data(&sections.data, on_progress)?;

    // Re-assemble TrackData by zipping mapping entries with payloads in order.
    let flat: Vec<(u8, u8, &crate::mapping::TrackMap)> = mapping.flat_tracks().collect();
    if flat.len() != model_data.tracks.len() {
        return Err(YinError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "mapping has {} tracks but data has {}",
                flat.len(),
                model_data.tracks.len()
            ),
        )));
    }

    let mut tracks: Vec<Arc<TrackData>> = Vec::with_capacity(flat.len());
    for ((port, channel, tm), payload) in flat.into_iter().zip(model_data.tracks) {
        let td = TrackData {
            uuid: tm.uuid.clone(),
            name: tm.name.clone(),
            color: tm.color,
            port,
            channel,
            channel_prefix: tm.channel_prefix,
            muted: tm.muted,
            soloed: tm.soloed,
            notes: Vec::new(), // notes loaded via load_bucket_notes
            automation_lanes: payload.automation_lanes,
            program_change: payload.program_change,
            lyrics: payload.lyrics,
            chord: payload.chord,
        };
        if td.uuid != payload.uuid {
            return Err(YinError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "track UUID mismatch: mapping={} payload={}",
                    td.uuid, payload.uuid
                ),
            )));
        }
        tracks.push(Arc::new(td));
    }

    // 列式流 → 128 桶（桶内按 start 排序），再由 load_bucket_notes 入模型并分配 id。
    let bucket_notes = bucket_from_streams(&note_streams, on_progress)?;

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
        ..Default::default()
    };
    model.load_bucket_notes(bucket_notes);
    model.rebuild();
    Ok((model, project, mapping))
}

/// Parse `.yin` bytes into a `YinModel` (SoundFont state, if any, is dropped).
pub fn load_yin_bytes(bytes: &[u8]) -> Result<YinModel, YinError> {
    let (model, _project, _mapping) = load_yin_bytes_inner(bytes, &mut |_| {})?;
    Ok(model)
}

/// Parse `.yin` bytes into a `YinModel` and its SoundFont state.
///
/// For files written before SF persistence, `ProjectSoundFonts` will be
/// `default()` (mode = false, overrides empty).
pub fn load_yin_bytes_with_sf(
    bytes: &[u8],
) -> Result<(YinModel, ProjectSoundFonts, MappingFile), YinError> {
    let (model, project, mapping) = load_yin_bytes_inner(bytes, &mut |_| {})?;
    let sf = ProjectSoundFonts {
        mode: project.soundfont_project_mode,
        overrides: project.soundfont_overrides,
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
) -> Result<(YinModel, ProjectSoundFonts, MappingFile), YinError> {
    load_yin_with_sf_progress(path, |_| {})
}

/// `load_yin_with_sf` + 进度回调（后台线程加载时用于驱动 UI 进度条）。
pub fn load_yin_with_sf_progress(
    path: impl AsRef<Path>,
    mut on_progress: impl FnMut(YinProgress) + Send,
) -> Result<(YinModel, ProjectSoundFonts, MappingFile), YinError> {
    let bytes = std::fs::read(path.as_ref())?;
    let (model, project, mapping) = load_yin_bytes_inner(&bytes, &mut on_progress)?;
    let sf = ProjectSoundFonts {
        mode: project.soundfont_project_mode,
        overrides: project.soundfont_overrides,
    };
    Ok((model, sf, mapping))
}
