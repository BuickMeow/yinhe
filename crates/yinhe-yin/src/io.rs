//! Top-level save / load API.
//!
//! `save_yin(model, path)` and `load_yin(path)` are the public entry points.
//! `save_yin_bytes(model)` / `load_yin_bytes(bytes)` operate on memory
//! buffers (used by tests and for streaming).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use yinhe_core::{
    CcEvent, ConductorData, NoteEvent, PcEvent, PitchBendEvent, ProjectMeta, RpnEvent, TrackData,
    YinModel,
};

use crate::container::{Sections, pack, unpack};
use crate::error::YinError;
use crate::mapping::MappingFile;
use crate::project_meta::{ProjectFile, SfPortOverride};

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

/// What goes into the zstd-compressed bincode blob.
#[derive(Serialize, Deserialize)]
struct ModelData {
    conductor: ConductorData,
    tracks: Vec<TrackPayload>,
}

#[derive(Serialize, Deserialize)]
struct TrackPayload {
    uuid: String,
    notes: Vec<NoteEvent>,
    cc: BTreeMap<u8, Vec<CcEvent>>,
    pitch_bend: Vec<PitchBendEvent>,
    program_change: Vec<PcEvent>,
    rpn: BTreeMap<u16, Vec<RpnEvent>>,
}

impl TrackPayload {
    fn from_track(t: &TrackData) -> Self {
        Self {
            uuid: t.uuid.clone(),
            notes: t.notes.clone(),
            cc: t.cc.clone(),
            pitch_bend: t.pitch_bend.clone(),
            program_change: t.program_change.clone(),
            rpn: t.rpn.clone(),
        }
    }
}

// =========================================================
//  Save
// =========================================================

/// Internal: serialize a model with optional SF state attached.
fn save_yin_bytes_inner(
    model: &YinModel,
    sf: Option<&ProjectSoundFonts>,
) -> Result<Vec<u8>, YinError> {
    // 1. project.json (with or without SF state)
    let project = match sf {
        Some(sf) => ProjectFile::from_meta_with_sf(&model.meta, sf.mode, sf.overrides.clone()),
        None => ProjectFile::from_meta(&model.meta),
    };
    let mapping = MappingFile::from_tracks(&model.tracks);
    save_yin_bytes_with_files_inner(model, &project, &mapping)
}

/// Internal: serialize with pre-built ProjectFile and MappingFile.
fn save_yin_bytes_with_files_inner(
    model: &YinModel,
    project: &ProjectFile,
    mapping: &MappingFile,
) -> Result<Vec<u8>, YinError> {
    let project_json = serde_json::to_vec_pretty(project)?;
    let mapping_json = serde_json::to_vec_pretty(mapping)?;

    // data.bin: bincode(ModelData) → zstd
    let mut tracks_payload = Vec::with_capacity(model.tracks.len());
    {
        let mut by_uuid: std::collections::HashMap<&str, &Arc<TrackData>> =
            std::collections::HashMap::with_capacity(model.tracks.len());
        for t in &model.tracks {
            by_uuid.insert(&t.uuid, t);
        }
        for (_port, _ch, tm) in mapping.flat_tracks() {
            if let Some(t) = by_uuid.get(tm.uuid.as_str()) {
                tracks_payload.push(TrackPayload::from_track(t));
            }
        }
    }

    let model_data = ModelData {
        conductor: (*model.conductor).clone(),
        tracks: tracks_payload,
    };

    let plain = bincode::serialize(&model_data)?;
    let level = model.meta.compression_level.clamp(0, 22);
    let data = zstd::encode_all(&plain[..], level)?;

    let bytes = pack(Sections {
        project_json,
        mapping_json,
        data,
    });
    Ok(bytes)
}

/// Serialize a `YinModel` to `.yin` bytes (no SoundFont state).
pub fn save_yin_bytes(model: &YinModel) -> Result<Vec<u8>, YinError> {
    save_yin_bytes_inner(model, None)
}

/// Serialize a `YinModel` plus its SoundFont state to `.yin` bytes.
pub fn save_yin_bytes_with_sf(
    model: &YinModel,
    sf: &ProjectSoundFonts,
) -> Result<Vec<u8>, YinError> {
    save_yin_bytes_inner(model, Some(sf))
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
    let bytes = save_yin_bytes_with_files_inner(model, project, mapping)?;
    std::fs::write(path.as_ref(), &bytes)?;
    Ok(())
}

// =========================================================
//  Load
// =========================================================

/// Internal: parse `.yin` bytes, returning model and the raw `ProjectFile`
/// (so callers can extract SF state if they want it).
fn load_yin_bytes_inner(bytes: &[u8]) -> Result<(YinModel, ProjectFile, MappingFile), YinError> {
    let sections = unpack(bytes)?;

    let project: ProjectFile = serde_json::from_slice(&sections.project_json)?;
    let mapping: MappingFile = serde_json::from_slice(&sections.mapping_json)?;

    // zstd decompress + bincode deserialize the data section.
    let plain = zstd::decode_all(&sections.data[..])?;
    let model_data: ModelData = bincode::deserialize(&plain)?;

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
    for ((port, channel, tm), payload) in flat.into_iter().zip(model_data.tracks.into_iter()) {
        let td = TrackData {
            uuid: tm.uuid.clone(),
            name: tm.name.clone(),
            color: tm.color,
            port,
            channel,
            channel_prefix: tm.channel_prefix,
            muted: tm.muted,
            soloed: tm.soloed,
            notes: payload.notes,
            cc: payload.cc,
            pitch_bend: payload.pitch_bend,
            program_change: payload.program_change,
            rpn: payload.rpn,
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
    model.rebuild();
    Ok((model, project, mapping))
}

/// Parse `.yin` bytes into a `YinModel` (SoundFont state, if any, is dropped).
pub fn load_yin_bytes(bytes: &[u8]) -> Result<YinModel, YinError> {
    let (model, _project, _mapping) = load_yin_bytes_inner(bytes)?;
    Ok(model)
}

/// Parse `.yin` bytes into a `YinModel` and its SoundFont state.
///
/// For files written before SF persistence, `ProjectSoundFonts` will be
/// `default()` (mode = false, overrides empty).
pub fn load_yin_bytes_with_sf(bytes: &[u8]) -> Result<(YinModel, ProjectSoundFonts, MappingFile), YinError> {
    let (model, project, mapping) = load_yin_bytes_inner(bytes)?;
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
    let bytes = std::fs::read(path.as_ref())?;
    load_yin_bytes_with_sf(&bytes)
}
