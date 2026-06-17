use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use yinhe_project::{
    ArchiveEntry, CcEvent as ProjectCcEvent, FileHeader, InnerHeader, Note as ProjectNote,
    PcEvent as ProjectPcEvent, PitchBendEvent as ProjectPitchBendEvent,
    ProjectArchive, ProjectJson, RpnEvent as ProjectRpnEvent, TempoEvent as ProjectTempoEvent,
    TimeSigEvent as ProjectTimeSigEvent, channel_label, cc_path, conductor_path, magic,
    pc_path, pitch_path, rpn_path, track_notes_path, track_prefix,
};

use crate::events::*;
use crate::model::*;

/// Convert a `ProjectArchive` into a `YinModel`.
pub fn archive_to_yinmodel(archive: &ProjectArchive) -> YinModel {
    // ── Project metadata ──
    let (meta, soundfont_project_mode) = if let Some(proj) =
        archive.get_json::<ProjectJson>("project.json")
    {
        (
            ProjectMeta {
                name: proj.name,
                artist: proj.artist,
                description: proj.description,
                ppq: proj.ppq,
                compression_level: archive.compression_level,
            },
            proj.soundfont_project_mode,
        )
    } else {
        (ProjectMeta::default(), false)
    };

    // ── Conductor events ──
    let conductor = ConductorData {
        tempo: archive
            .get_delta_events::<ProjectTempoEvent>(&conductor_path("tempo.zst"))
            .unwrap_or_default()
            .iter()
            .map(|t| TempoEvent {
                tick: t.tick,
                bpm: t.bpm as f64,
            })
            .collect(),
        time_sig: archive
            .get_delta_events::<ProjectTimeSigEvent>(&conductor_path("time_sig.zst"))
            .unwrap_or_default()
            .iter()
            .map(|e| TimeSigEvent {
                tick: e.tick,
                numerator: e.numerator,
                denominator: e.denominator_power,
            })
            .collect(),
    };

    // ── Parse mapping.json ──
    let mapping = archive.get_json::<yinhe_project::MappingJson>("mapping.json");
    let mut uuid_to_track: HashMap<&str, usize> = HashMap::new();
    let mut track_meta: Vec<(String, u8, u8)> = Vec::new(); // (name, port, channel)
    if let Some(ref mapping) = {
        // Need to handle the borrow
        None::<()>
    } {
        unreachable!()
    }
    // Re-parse mapping
    let mapping = archive.get_json::<yinhe_project::MappingJson>("mapping.json");
    if let Some(ref m) = mapping {
        for port_mapping in &m.ports {
            for ch_mapping in &port_mapping.channels {
                for track_mapping in &ch_mapping.tracks {
                    let idx = track_mapping.track_index as usize;
                    uuid_to_track.insert(track_mapping.uuid.as_str(), idx);
                    while track_meta.len() <= idx {
                        track_meta.push((format!("Track {}", track_meta.len() + 1), 0, 0));
                    }
                    track_meta[idx] = (
                        track_mapping.name.clone(),
                        port_mapping.port,
                        ch_mapping.channel,
                    );
                }
            }
        }
    }

    // ── Build tracks from entries ──
    let mut tracks: Vec<TrackData> = Vec::new();

    for (path, entry) in &archive.entries {
        let path_str = path.as_str();
        if path_str.starts_with("conductor/")
            || path_str == "mapping.json"
            || path_str == "project.json"
        {
            continue;
        }

        let uuid_str = match extract_uuid_from_path(path_str) {
            Some(u) => u,
            None => continue,
        };
        let track_idx = match uuid_to_track.get(uuid_str) {
            Some(&idx) => idx,
            None => continue,
        };

        while tracks.len() <= track_idx {
            let (name, port, channel) = track_meta
                .get(tracks.len())
                .cloned()
                .unwrap_or_else(|| (format!("Track {}", tracks.len() + 1), 0, 0));
            tracks.push(TrackData {
                uuid: String::new(),
                name,
                port,
                channel,
                notes: Vec::new(),
                cc: BTreeMap::new(),
                pitch_bend: Vec::new(),
                program_change: Vec::new(),
                rpn: BTreeMap::new(),
            });
        }

        let h = entry.header;
        match h.magic {
            magic::TRACK_NOTES => {
                if let Some((inner, notes)) = archive.get_notes(path_str) {
                    tracks[track_idx].uuid = uuid_str.to_string();
                    for n in &notes {
                        tracks[track_idx].notes.push(NoteEvent {
                            tick: n.start_tick,
                            duration: n.end_tick.saturating_sub(n.start_tick),
                            key: n.key,
                            velocity: n.velocity,
                        });
                    }
                }
            }
            magic::CC => {
                if let Some((inner, events)) =
                    archive.get_delta_events_with_inner::<ProjectCcEvent>(path_str)
                {
                    let cc_num = h.extra;
                    let cc_events = tracks[track_idx].cc.entry(cc_num).or_default();
                    for ev in &events {
                        cc_events.push(CcEvent {
                            tick: ev.tick,
                            value: ev.value,
                        });
                    }
                }
            }
            magic::PITCH_BEND => {
                if let Some((inner, events)) =
                    archive.get_delta_events_with_inner::<ProjectPitchBendEvent>(path_str)
                {
                    for ev in &events {
                        tracks[track_idx].pitch_bend.push(PitchBendEvent {
                            tick: ev.tick,
                            value: ev.value,
                        });
                    }
                }
            }
            magic::PC => {
                if let Some((inner, events)) =
                    archive.get_delta_events_with_inner::<ProjectPcEvent>(path_str)
                {
                    for ev in &events {
                        tracks[track_idx].program_change.push(PcEvent {
                            tick: ev.tick,
                            program: ev.program,
                            bank_msb: ev.bank_msb,
                            bank_lsb: ev.bank_lsb,
                        });
                    }
                }
            }
            magic::RPN => {
                if let Some((inner, events)) =
                    archive.get_delta_events_with_inner::<ProjectRpnEvent>(path_str)
                {
                    let rpn_num = h.extra;
                    let rpn_events = tracks[track_idx].rpn.entry(rpn_num).or_default();
                    for ev in &events {
                        rpn_events.push(RpnEvent {
                            tick: ev.tick,
                            value: ev.value,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // Ensure at least one track
    if tracks.is_empty() {
        tracks.push(TrackData {
            uuid: Uuid::new_v4().to_string(),
            name: "Track 1".to_string(),
            port: 0,
            channel: 0,
            notes: Vec::new(),
            cc: BTreeMap::new(),
            pitch_bend: Vec::new(),
            program_change: Vec::new(),
            rpn: BTreeMap::new(),
        });
    }

    // Sort events by tick within each track
    for track in &mut tracks {
        track.notes.sort_by_key(|n| n.tick);
        for cc_events in track.cc.values_mut() {
            cc_events.sort_by_key(|e| e.tick);
        }
        track.pitch_bend.sort_by_key(|e| e.tick);
        track.program_change.sort_by_key(|e| e.tick);
        for rpn_events in track.rpn.values_mut() {
            rpn_events.sort_by_key(|e| e.tick);
        }
    }

    let mut model = YinModel {
        conductor,
        tracks,
        meta,
        key_index: KeyIndex::default(),
        key_notes_cache: (0..128).map(|_| Vec::new()).collect(),
        note_count: 0,
        tick_length: 0,
    };
    model.rebuild();
    model
}

fn extract_uuid_from_path(path: &str) -> Option<&str> {
    // Path format: "channels/A01/{uuid}/notes.zst"
    let rest = path.strip_prefix("channels/")?;
    let slash1 = rest.find('/')?;
    let after_label = &rest[slash1 + 1..];
    let slash2 = after_label.find('/')?;
    Some(&after_label[..slash2])
}
