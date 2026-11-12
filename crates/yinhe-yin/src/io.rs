//! Top-level save / load API.
//!
//! `save_yin(model, path)` and `load_yin(path)` are the public entry points.
//! `save_yin_bytes(model)` / `load_yin_bytes(bytes)` operate on memory
//! buffers (used by tests and for streaming).
//!
//! 段编解码分散在同目录子模块：`data_section`（meta + 音符流）、
//! `mixer_section`（混音台参数）、`audio_section`（内嵌音频素材）。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yinhe_core::{NoteLoader, ProjectMeta, TrackData, YinModel};
use yinhe_mixer::MixerParams;

use crate::audio_section::{decode_audio_section, encode_audio_section};
use crate::container::{Sections, pack, unpack};
use crate::data_section::{
    MetaPayload, TrackPayload, compress_data, encode_note_streams, open_note_streams,
};
use crate::error::{YinError, invalid_data};
use crate::mapping::MappingFile;
use crate::mixer_section::{decode_mixer_section, encode_mixer_section};
use crate::progress::YinProgress;
use crate::project_meta::{ProjectFile, SfChannelOverride};

/// `.yin` 完整加载结果（模型 / 音色库 / 映射 / 混音 / 封面）。
pub type LoadedYin = (
    YinModel,
    ProjectSoundFonts,
    MappingFile,
    Option<MixerParams>,
    Option<yinhe_core::CoverArt>,
);

/// 只读 `.yin` 头部的 `project.json`（未压缩、位于文件最前）。
///
/// 供音乐库/最近项目列表按专辑分组用：不解压音符段、不读 mapping/audio，
/// 内存只有几 KB。文件不在/不是 `.yin` 时返回错误，调用方跳过即可。
pub fn read_project_file(path: impl AsRef<Path>) -> Result<ProjectFile, YinError> {
    let mut file = std::fs::File::open(path.as_ref())?;
    let mut head = [0u8; 10];
    file.read_exact(&mut head)?;
    if &head[0..4] != crate::MAGIC {
        return Err(YinError::BadMagic);
    }
    let version = u16::from_le_bytes([head[4], head[5]]);
    if version != crate::VERSION {
        return Err(YinError::BadVersion(version));
    }
    let len = u32::from_le_bytes([head[6], head[7], head[8], head[9]]) as usize;
    // project.json 正常只有几 KB；限制上限防止损坏文件把内存吃满。
    if len > 64 * 1024 * 1024 {
        return Err(invalid_data("project.json 过大"));
    }
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
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
//  Save
// =========================================================

/// 原子写文件：同目录临时文件 + `sync_all` + rename 覆盖。
///
/// `std::fs::write` 会先截断目标文件再写，保存中途崩溃/断电/磁盘满会把
/// 原工程变成半截文件（旧内容也没了）。这里先写同目录临时文件（同一文件
/// 系统，rename 才是原子的），落盘后再 rename；目标路径要么保持旧内容，
/// 要么是完整的新内容。rename 失败（如 Windows 上目标被占用）时保留原文件。
pub fn write_file_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static NEXT_TMP: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "保存路径缺少文件名")
    })?;
    let tmp = parent.join(format!(
        ".{}.{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ));

    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        // 先把内容落盘，再让 rename 的元数据变更落盘；否则掉电后可能
        // 出现“文件已改名但内容为空”的典型 ext4/APFS 场景。
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    };

    let result = write();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    // 目录项也要 fsync，rename 才能扛住掉电（Unix 有效；Windows 无对应语义）。
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

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
    let mut mapping = MappingFile::from_tracks(&model.tracks);
    mapping.channel_instruments = model.channel_instruments;
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

    // data 段：meta（conductor + tracks payload + 轨段表）+ 5 列音符流
    //（分帧 zstd）。音符 id 以 zigzag varint delta 落盘（段内独立，与轨段
    // 布局同序，压缩后近零开销），加载时保留。
    let (notes, segments) = encode_note_streams(model, model.meta.compression_level, on_progress)?;
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
    let cover = project
        .cover
        .as_ref()
        .map(crate::cover_section::encode_cover_section);
    let bytes = pack(Sections {
        project_json,
        mapping_json,
        data,
        mixer,
        audio,
        cover,
    });
    // 编码/压缩的临时缓冲已释放，把空闲页归还 OS（1.64 亿音符可降 RSS ~1GB）。
    yinhe_memtrace::purge_free_pages();
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
    write_file_atomic(path.as_ref(), &bytes)?;
    Ok(())
}

/// Save a `YinModel` plus its SoundFont state to a file at `path`.
pub fn save_yin_with_sf(
    model: &YinModel,
    path: impl AsRef<Path>,
    sf: &ProjectSoundFonts,
) -> Result<(), YinError> {
    let bytes = save_yin_bytes_with_sf(model, sf)?;
    write_file_atomic(path.as_ref(), &bytes)?;
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
    write_file_atomic(path.as_ref(), &bytes)?;
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

    let mut project: ProjectFile = serde_json::from_slice(&sections.project_json)?;
    project.cover = sections
        .cover
        .as_deref()
        .and_then(crate::cover_section::decode_cover_section);
    let mapping: MappingFile = serde_json::from_slice(&sections.mapping_json)?;

    let (model_data, mut note_streams) = open_note_streams(&sections.data, on_progress)?;

    // 按 payload 顺序（保存时的 model.tracks 顺序）重建 TrackData；
    // 音轨的 port/channel/元数据取自 mapping（uuid 关联），
    // 不再依赖 mapping 的存储顺序——mapping 的 ports/channels 嵌套分组
    // 无法表达 model 的全局音轨顺序（同 port 音轨必须连续存放）。
    let flat: Vec<(u8, u8, &crate::mapping::TrackMap)> = mapping.flat_tracks().collect();
    if flat.len() != model_data.tracks.len() {
        return Err(invalid_data(format!(
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
            return Err(invalid_data(format!(
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
            clips: tm.clips.clone(),
            notes: Vec::new(), // notes loaded via NoteLoader
            automation_lanes: payload.automation_lanes,
            program_change: payload.program_change,
            lyrics: payload.lyrics,
            chord: payload.chord,
        };
        tracks.push(Arc::new(td));
    }

    // A10 默认鼓通道：旧存档缺省（None）时补上 Standard Kit。
    // 历史工程用 bank_msb == 128 表示鼓，这里统一迁移成显式 is_drum。
    let mut channel_instruments = mapping.channel_instruments;
    for inst in channel_instruments.iter_mut().flatten() {
        inst.normalize_legacy_bank();
    }
    if channel_instruments[9].is_none() {
        channel_instruments[9] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
            is_drum: true,
        });
    }
    let mut model = YinModel {
        conductor: Arc::new(model_data.conductor),
        channel_instruments,
        tracks,
        meta: ProjectMeta {
            name: project.name.clone(),
            artist: project.artist.clone(),
            description: project.description.clone(),
            ppq: project.ppq,
            compression_level: project.compression_level,
            default_bpm: project.default_bpm,
            default_time_sig: project.default_time_sig,
            album: project.album.clone(),
            album_artist: project.album_artist.clone(),
            genre: project.genre.clone(),
            year: project.year.clone(),
            track_no: project.track_no,
            disc_no: project.disc_no,
            copyright: project.copyright.clone(),
            artists: project.artists.clone(),
            work_time_secs: project.work_time_secs,
        },
        audio_sources,
        ..Default::default()
    };
    // 流式加载：第一遍只解压 key 列统计每桶容量，第二遍 5 列同步解析直接
    // 喂 NoteLoader（id 已随流落盘，非 0 保留），finish 逐桶排序分块。
    let key_counts = note_streams.key_counts()?;
    let mut loader = NoteLoader::new(model.tracks.len(), model.next_note_id, key_counts);
    note_streams.feed(&model_data.segments, &mut loader, on_progress)?;
    loader.finish(&mut model);
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
    // 流式加载的帧缓冲/装载临时内存已释放，把空闲页归还 OS。
    yinhe_memtrace::purge_free_pages();
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
    let (model, sf, mapping, mixer, _cover) = load_yin_with_sf_progress(path, |_| {})?;
    Ok((model, sf, mapping, mixer))
}

/// `load_yin_with_sf` + 进度回调（后台线程加载时用于驱动 UI 进度条）。
pub fn load_yin_with_sf_progress(
    path: impl AsRef<Path>,
    mut on_progress: impl FnMut(YinProgress) + Send,
) -> Result<LoadedYin, YinError> {
    let bytes = std::fs::read(path.as_ref())?;
    let (model, project, mapping, mixer) = load_yin_bytes_inner(&bytes, &mut on_progress)?;
    let sf = ProjectSoundFonts {
        overrides: project.sf_channel_overrides,
    };
    Ok((model, sf, mapping, mixer, project.cover))
}
