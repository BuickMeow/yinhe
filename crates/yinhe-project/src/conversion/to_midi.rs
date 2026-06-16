use std::collections::HashMap;

use crate::archive::ProjectArchive;
use crate::events::*;
use crate::header::*;
use crate::paths::*;
use crate::schema::*;

use super::{extract_uuid, TrackEventGroup};

/// Convert a ProjectArchive back into a MidiFile.
pub fn archive_to_midi(archive: &ProjectArchive) -> yinhe_midi::MidiFile {
    let mut midi = yinhe_midi::MidiFile::default();

    if let Some(proj) = archive.get_json::<ProjectJson>("project.json") {
        midi.ticks_per_beat = proj.ppq;
    }

    read_conductor_events(archive, &mut midi);

    let mapping: Option<MappingJson> = archive.get_json("mapping.json");
    let uuid_to_track = build_uuid_to_track_map(&mapping);

    let (mut track_data, num_tracks) =
        read_track_data_from_entries(archive, &uuid_to_track, &mapping);
    apply_mapping_to_midi(&mut midi, &mapping, num_tracks);

    rebuild_midi_arrays(archive, &mut midi, &mut track_data, &uuid_to_track, num_tracks);
    finalize_midi(&mut midi);

    midi
}

fn read_conductor_events(archive: &ProjectArchive, midi: &mut yinhe_midi::MidiFile) {
    let tempos = archive
        .get_delta_events::<TempoEvent>(&conductor_path("tempo.zst"))
        .unwrap_or_default();
    let mut segments: Vec<yinhe_midi::TempoSegment> = tempos
        .iter()
        .map(|t| yinhe_midi::TempoSegment {
            start_tick: t.tick,
            start_time: 0.0,
            micros_per_quarter: yinhe_midi::mpq_from_bpm(t.bpm),
        })
        .collect();
    if segments.first().map(|s| s.start_tick).unwrap_or(u32::MAX) > 0 {
        segments.insert(
            0,
            yinhe_midi::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: yinhe_midi::mpq_from_bpm(120.0),
            },
        );
    }
    yinhe_midi::recompute_tempo_start_times(&mut segments, midi.ticks_per_beat);
    midi.tempo_segments = segments;

    if let Some(time_sigs) =
        archive.get_delta_events::<TimeSigEvent>(&conductor_path("time_sig.zst"))
    {
        midi.time_sig_events = time_sigs
            .iter()
            .map(|e| yinhe_types::TimeSigEvent {
                tick: e.tick,
                numerator: e.numerator,
                denominator: e.denominator_power,
            })
            .collect();
    }
}

fn build_uuid_to_track_map(mapping: &Option<MappingJson>) -> HashMap<&str, usize> {
    let mut uuid_to_track: HashMap<&str, usize> = HashMap::new();
    if let Some(mapping) = mapping {
        for port_mapping in &mapping.ports {
            for ch_mapping in &port_mapping.channels {
                for track_mapping in &ch_mapping.tracks {
                    uuid_to_track.insert(
                        track_mapping.uuid.as_str(),
                        track_mapping.track_index as usize,
                    );
                }
            }
        }
    }
    uuid_to_track
}

fn read_track_data_from_entries(
    archive: &ProjectArchive,
    uuid_to_track: &HashMap<&str, usize>,
    mapping: &Option<MappingJson>,
) -> (Vec<Option<TrackEventGroup>>, usize) {
    let mut track_data: Vec<Option<TrackEventGroup>> = Vec::new();

    for (path, entry) in &archive.entries {
        let path = path.as_str();
        if path.starts_with("conductor/") || path == "mapping.json" || path == "project.json" {
            continue;
        }
        let Some(uuid_str) = extract_uuid(path) else {
            continue;
        };
        let Some(&track_idx) = uuid_to_track.get(uuid_str) else {
            continue;
        };

        while track_data.len() <= track_idx {
            track_data.push(None);
        }
        let data = track_data[track_idx].get_or_insert_with(TrackEventGroup::new);

        let h = entry.header;
        match h.magic {
            magic::TRACK_NOTES => {
                if let Some((_inner, notes)) = archive.get_notes(path) {
                    data.notes.extend(notes);
                }
            }
            magic::CC => {
                if let Some((_inner, events)) =
                    archive.get_delta_events_with_inner::<CcEvent>(path)
                {
                    for ev in &events {
                        data.cc_events.push((h.extra, *ev));
                    }
                }
            }
            magic::PITCH_BEND => {
                if let Some((_inner, events)) =
                    archive.get_delta_events_with_inner::<PitchBendEvent>(path)
                {
                    data.pitch_events.extend(events);
                }
            }
            magic::PC => {
                if let Some((_inner, events)) =
                    archive.get_delta_events_with_inner::<PcEvent>(path)
                {
                    data.pc_events.extend(events);
                }
            }
            magic::RPN => {
                if let Some((_inner, events)) =
                    archive.get_delta_events_with_inner::<RpnEvent>(path)
                {
                    for ev in &events {
                        data.rpn_events.push((h.extra, *ev));
                    }
                }
            }
            _ => {}
        }
    }

    let num_tracks_from_entries = track_data.len();
    let num_tracks_from_mapping = mapping
        .as_ref()
        .map(|m| {
            m.ports
                .iter()
                .flat_map(|p| p.channels.iter())
                .flat_map(|c| c.tracks.iter())
                .map(|t| t.track_index as usize + 1)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let num_tracks = num_tracks_from_entries.max(num_tracks_from_mapping).max(1);
    while track_data.len() < num_tracks {
        track_data.push(None);
    }

    (track_data, num_tracks)
}

fn apply_mapping_to_midi(
    midi: &mut yinhe_midi::MidiFile,
    mapping: &Option<MappingJson>,
    num_tracks: usize,
) {
    midi.track_ports = vec![0; num_tracks];
    midi.track_channel_prefixes = vec![None; num_tracks];
    midi.track_channels = vec![0; num_tracks];
    midi.track_names = (0..num_tracks)
        .map(|i| format!("Track {}", i + 1))
        .collect();

    if let Some(mapping) = mapping {
        for port_mapping in &mapping.ports {
            for ch_mapping in &port_mapping.channels {
                for track_mapping in &ch_mapping.tracks {
                    let idx = track_mapping.track_index as usize;
                    if idx < num_tracks {
                        midi.track_names[idx] = track_mapping.name.clone();
                        midi.track_ports[idx] = port_mapping.port;
                        midi.track_channel_prefixes[idx] = track_mapping.channel_prefix;
                    }
                }
            }
        }
    }
}

fn rebuild_midi_arrays(
    archive: &ProjectArchive,
    midi: &mut yinhe_midi::MidiFile,
    track_data: &mut [Option<TrackEventGroup>],
    uuid_to_track: &HashMap<&str, usize>,
    num_tracks: usize,
) {
    for track_idx in 0..num_tracks {
        let Some(data) = &track_data[track_idx] else {
            continue;
        };

        let global_ch = resolve_track_channel(archive, uuid_to_track, track_idx, midi);
        let port = global_ch >> 4;
        midi.track_ports[track_idx] = port;
        midi.track_channels[track_idx] = global_ch;

        for note in &data.notes {
            let key = note.key as usize;
            if key < 128 {
                midi.key_notes[key].push(yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    track: track_idx as u16,
                });
            }
        }

        for (controller, ev) in &data.cc_events {
            midi.control_events
                .push(yinhe_midi::MidiControlEvent::ControlChange {
                    tick: ev.tick,
                    controller: *controller,
                    value: ev.value,
                    track: track_idx as u16,
                });
        }

        for ev in &data.pitch_events {
            midi.control_events
                .push(yinhe_midi::MidiControlEvent::PitchBend {
                    tick: ev.tick,
                    value: ev.value,
                    track: track_idx as u16,
                });
        }

        for ev in &data.pc_events {
            midi.control_events
                .push(yinhe_midi::MidiControlEvent::ProgramChange {
                    tick: ev.tick,
                    program: ev.program,
                    track: track_idx as u16,
                });
            if ev.bank_msb != 0xFF {
                midi.control_events
                    .push(yinhe_midi::MidiControlEvent::ControlChange {
                        tick: ev.tick,
                        controller: 0,
                        value: ev.bank_msb,
                        track: track_idx as u16,
                    });
            }
            if ev.bank_lsb != 0xFF {
                midi.control_events
                    .push(yinhe_midi::MidiControlEvent::ControlChange {
                        tick: ev.tick,
                        controller: 32,
                        value: ev.bank_lsb,
                        track: track_idx as u16,
                    });
            }
        }

        for (rpn_num, ev) in &data.rpn_events {
            midi.control_events
                .push(yinhe_midi::MidiControlEvent::ControlChange {
                    tick: ev.tick,
                    controller: 101,
                    value: *rpn_num,
                    track: track_idx as u16,
                });
            midi.control_events
                .push(yinhe_midi::MidiControlEvent::ControlChange {
                    tick: ev.tick,
                    controller: 100,
                    value: 0,
                    track: track_idx as u16,
                });
            midi.control_events
                .push(yinhe_midi::MidiControlEvent::ControlChange {
                    tick: ev.tick,
                    controller: 6,
                    value: ev.value.min(127) as u8,
                    track: track_idx as u16,
                });
        }
    }
}

fn resolve_track_channel(
    archive: &ProjectArchive,
    uuid_to_track: &HashMap<&str, usize>,
    track_idx: usize,
    midi: &yinhe_midi::MidiFile,
) -> u8 {
    for (path, entry) in &archive.entries {
        let path = path.as_str();
        if path.starts_with("conductor/") || path == "mapping.json" || path == "project.json" {
            continue;
        }
        let Some(uuid_str) = extract_uuid(path) else {
            continue;
        };
        let Some(&tid) = uuid_to_track.get(uuid_str) else {
            continue;
        };
        if tid != track_idx {
            continue;
        }
        let inner = match entry.header.magic {
            magic::TRACK_NOTES => archive.get_notes(path).map(|(i, _)| i),
            magic::CC => archive
                .get_delta_events_with_inner::<CcEvent>(path)
                .map(|(i, _)| i),
            magic::PITCH_BEND => archive
                .get_delta_events_with_inner::<PitchBendEvent>(path)
                .map(|(i, _)| i),
            magic::PC => archive
                .get_delta_events_with_inner::<PcEvent>(path)
                .map(|(i, _)| i),
            magic::RPN => archive
                .get_delta_events_with_inner::<RpnEvent>(path)
                .map(|(i, _)| i),
            _ => None,
        };
        if let Some(inner) = inner {
            return inner.channel;
        }
    }
    let port = midi.track_ports.get(track_idx).copied().unwrap_or(0);
    port * 16
}

fn finalize_midi(midi: &mut yinhe_midi::MidiFile) {
    for notes in &mut midi.key_notes {
        notes.sort_by_key(|n| n.start_tick);
    }

    midi.control_events.sort_by_key(|e| match e {
        yinhe_midi::MidiControlEvent::ControlChange { tick, .. } => *tick,
        yinhe_midi::MidiControlEvent::ProgramChange { tick, .. } => *tick,
        yinhe_midi::MidiControlEvent::PitchBend { tick, .. } => *tick,
    });

    midi.note_count = midi.key_notes.iter().map(|n| n.len() as u64).sum();
    midi.tick_length = midi
        .key_notes
        .iter()
        .flat_map(|notes| notes.iter())
        .map(|n| n.end_tick as u64)
        .max()
        .unwrap_or(0);

    midi.scan_index = Some(yinhe_types::NoteScanIndex::build(
        &midi.key_notes,
        midi.tick_length,
    ));

    midi.automation_lanes = yinhe_midi::build_automation_lanes(
        &midi.control_events,
        &midi.key_notes,
        &midi.track_channels,
    );
}
