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
    CcEvent, ConductorData, NoteEvent, PcEvent, PitchBendEvent, RpnEvent, TrackData, YinModel,
};

use crate::container::{Sections, pack, unpack};
use crate::error::YinError;
use crate::mapping::MappingFile;
use crate::project_meta::ProjectFile;

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

/// Serialize a `YinModel` to `.yin` bytes.
pub fn save_yin_bytes(model: &YinModel) -> Result<Vec<u8>, YinError> {
    // 1. project.json
    let project = ProjectFile::from_meta(&model.meta);
    let project_json = serde_json::to_vec_pretty(&project)?;

    // 2. mapping.json
    let mapping = MappingFile::from_tracks(&model.tracks);
    let mapping_json = serde_json::to_vec_pretty(&mapping)?;

    // 3. data.bin: bincode(ModelData) → zstd
    //
    // Track payloads are emitted in mapping.flat_tracks() order so that
    // load can match payloads to TrackMap entries by index.
    let mut tracks_payload = Vec::with_capacity(model.tracks.len());
    {
        // Build (port, channel, uuid) → &TrackData lookup so we can iterate
        // mapping order while still finding the right track.
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

/// Save a `YinModel` to a file at `path`.
pub fn save_yin(model: &YinModel, path: impl AsRef<Path>) -> Result<(), YinError> {
    let bytes = save_yin_bytes(model)?;
    std::fs::write(path.as_ref(), &bytes)?;
    Ok(())
}

// =========================================================
//  Load
// =========================================================

/// Parse `.yin` bytes into a `YinModel`.
pub fn load_yin_bytes(bytes: &[u8]) -> Result<YinModel, YinError> {
    let sections = unpack(bytes)?;

    let project: ProjectFile = serde_json::from_slice(&sections.project_json)?;
    let mapping: MappingFile = serde_json::from_slice(&sections.mapping_json)?;

    // zstd decompress + bincode deserialize the data section.
    let plain = zstd::decode_all(&sections.data[..])?;
    let model_data: ModelData = bincode::deserialize(&plain)?;

    // Re-assemble TrackData by zipping mapping entries with payloads in order.
    let flat: Vec<(u8, u8, &crate::mapping::TrackMap)> = mapping.flat_tracks().collect();
    if flat.len() != model_data.tracks.len() {
        // Length mismatch is recoverable: trust payload count for events, but
        // metadata will be partial. We choose to error here so callers know.
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
        // Mismatched UUID = corrupted file.
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
        meta: project.into_meta(),
        ..Default::default()
    };
    model.rebuild();
    Ok(model)
}

/// Load a `.yin` file from `path`.
pub fn load_yin(path: impl AsRef<Path>) -> Result<YinModel, YinError> {
    let bytes = std::fs::read(path.as_ref())?;
    load_yin_bytes(&bytes)
}
