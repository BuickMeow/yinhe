use std::collections::HashMap;

use midly::num::{u4, u7, u15};
use midly::{Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind};
use std::io::Write;
use uuid::Uuid;
use yinhe_project::*;

/// Convert a MidiFile into a ProjectArchive.
pub fn midi_to_archive(midi: &yinhe_midi::MidiFile) -> ProjectArchive {
    let names: Vec<String> = (0..midi.track_ports.len())
        .map(|i| {
            midi.track_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("Track {}", i + 1))
        })
        .collect();
    midi_to_archive_with_names(midi, &names)
}

/// Same as `midi_to_archive` but uses caller-provided track names (the
/// authoritative editable copy from `Document.track_names`).
pub fn midi_to_archive_with_names(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
) -> ProjectArchive {
    let mut archive = ProjectArchive::new();

    // ── Conductor events ──
    let tempos: Vec<TempoEvent> = midi
        .tempo_segments
        .iter()
        .map(|s| TempoEvent {
            tick: s.start_tick,
            bpm: yinhe_midi::bpm_from_mpq(s.micros_per_quarter) as f32,
        })
        .collect();
    if !tempos.is_empty() {
        archive.set_events(
            conductor_path("tempo.zst"),
            FileHeader::new(*b"YHTM", 0, 0, 0),
            &tempos,
        );
    }

    let time_sigs: Vec<TimeSigEvent> = midi
        .time_sig_events
        .iter()
        .map(|e| TimeSigEvent {
            tick: e.tick,
            numerator: e.numerator,
            denominator_power: e.denominator,
        })
        .collect();
    if !time_sigs.is_empty() {
        archive.set_events(
            conductor_path("time_sig.zst"),
            FileHeader::new(*b"YHTS", 0, 0, 0),
            &time_sigs,
        );
    }

    // ── Group notes by track index ──
    let num_tracks = midi.track_ports.len();
    let mut track_notes: Vec<Vec<yinhe_project::Note>> = (0..num_tracks).map(|_| Vec::new()).collect();
    let mut track_channels: Vec<u8> = vec![0; num_tracks];

    for (key_idx, key_notes) in midi.key_notes.iter().enumerate() {
        for note in key_notes {
            let idx = note.track as usize;
            if idx < num_tracks {
                track_notes[idx].push(yinhe_project::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    key: key_idx as u8,
                    velocity: note.velocity,
                });
                if track_channels[idx] == 0 {
                    track_channels[idx] = note.channel;
                }
            }
        }
    }

    for notes in &mut track_notes {
        notes.sort_by_key(|n| n.start_tick);
    }

    // ── Generate UUIDs for tracks and build mapping ──
    let mut port_map: HashMap<u8, Vec<(u8, Vec<TrackMapping>)>> = HashMap::new();

    for track_idx in 0..num_tracks {
        let port = midi.track_ports.get(track_idx).copied().unwrap_or(0);
        let channel = track_channels[track_idx];
        let uuid = Uuid::new_v4().to_string();
        let name = midi
            .track_names
            .get(track_idx)
            .cloned()
            .unwrap_or_else(|| format!("Track {}", track_idx + 1));
        let name = track_names
            .get(track_idx)
            .cloned()
            .unwrap_or(name);

        if !track_notes[track_idx].is_empty() {
            archive.set_notes(
                track_notes_path(port, channel, &uuid),
                FileHeader::new(*b"YHTK", port, channel, track_idx as u8),
                &track_notes[track_idx],
            );
        }

        // Collect CC events per controller number
        let mut cc_events: HashMap<u8, Vec<CcEvent>> = HashMap::new();
        let mut pitch_events: Vec<PitchBendEvent> = Vec::new();
        let mut pc_events: Vec<PcEvent> = Vec::new();
        for ev in &midi.control_events {
            let ev_track = match ev {
                yinhe_midi::MidiControlEvent::ControlChange { track, .. }
                | yinhe_midi::MidiControlEvent::ProgramChange { track, .. }
                | yinhe_midi::MidiControlEvent::PitchBend { track, .. } => *track,
            };
            if ev_track as usize != track_idx {
                continue;
            }
            match ev {
                yinhe_midi::MidiControlEvent::ControlChange {
                    tick,
                    controller,
                    value,
                    ..
                } => {
                    cc_events
                        .entry(*controller)
                        .or_default()
                        .push(CcEvent {
                            tick: *tick,
                            value: *value,
                        });
                }
                yinhe_midi::MidiControlEvent::PitchBend { tick, value, .. } => {
                    pitch_events.push(PitchBendEvent {
                        tick: *tick,
                        value: *value,
                    });
                }
                yinhe_midi::MidiControlEvent::ProgramChange { tick, program, .. } => {
                    pc_events.push(PcEvent {
                        tick: *tick,
                        program: *program,
                    });
                }
            }
        }

        for (cc_num, events) in &cc_events {
            archive.set_events(
                cc_path(port, channel, *cc_num),
                FileHeader::new(*b"YHCC", port, channel, *cc_num),
                events,
            );
        }

        if !pitch_events.is_empty() {
            archive.set_events(
                pitch_path(port, channel),
                FileHeader::new(*b"YHPB", port, channel, 0),
                &pitch_events,
            );
        }

        if !pc_events.is_empty() {
            archive.set_events(
                pc_path(port, channel),
                FileHeader::new(*b"YHPC", port, channel, 0),
                &pc_events,
            );
        }

        // Add to mapping
        port_map
            .entry(port)
            .or_default()
            .push((channel, vec![]));
        let channels = port_map.get_mut(&port).unwrap();
        let ch_entry = channels
            .iter_mut()
            .find(|(ch, _)| *ch == channel)
            .unwrap();
        ch_entry.1.push(TrackMapping {
            uuid,
            name,
            color: [0.5, 0.5, 0.5],
        });
    }

    // ── Write mapping.json ──
    let mapping = MappingJson {
        ports: port_map
            .into_iter()
            .map(|(port, channels)| PortMapping {
                port,
                channels: channels
                    .into_iter()
                    .map(|(channel, tracks)| ChannelMapping { channel, tracks })
                    .collect(),
            })
            .collect(),
    };
    archive.set_events("mapping.json", FileHeader::new(*b"YHMP", 0, 0, 0), &[mapping]);

    // ── Write project.json ──
    let proj = ProjectJson {
        version: 1,
        name: String::new(),
        artist: String::new(),
        ppq: midi.ticks_per_beat,
        zstd_level: 0,
        description: String::new(),
    };
    archive.set_events("project.json", FileHeader::new(*b"YHPR", 0, 0, 0), &[proj]);

    archive
}

/// Convert a ProjectArchive back into a MidiFile.
pub fn archive_to_midi(archive: &ProjectArchive) -> yinhe_midi::MidiFile {
    let mut midi = yinhe_midi::MidiFile::default();

    // ── Read project.json for ppq ──
    if let Some(proj) = archive.get_events::<ProjectJson>("project.json") {
        if let Some(p) = proj.first() {
            midi.ticks_per_beat = p.ppq;
        }
    }

    // ── Read conductor events ──
    if let Some(tempos) = archive.get_events::<TempoEvent>(&conductor_path("tempo.zst")) {
        midi.tempo_segments = tempos
            .iter()
            .map(|t| yinhe_midi::TempoSegment {
                start_tick: t.tick,
                start_time: 0.0,
                micros_per_quarter: yinhe_midi::mpq_from_bpm(t.bpm),
            })
            .collect();
        yinhe_midi::recompute_tempo_start_times(&mut midi.tempo_segments, midi.ticks_per_beat);
    }

    if let Some(time_sigs) = archive.get_events::<TimeSigEvent>(&conductor_path("time_sig.zst")) {
        midi.time_sig_events = time_sigs
            .iter()
            .map(|e| yinhe_types::TimeSigEvent {
                tick: e.tick,
                numerator: e.numerator,
                denominator: e.denominator_power,
            })
            .collect();
    }

    // ── Read mapping ──
    let mapping: Vec<MappingJson> = archive.get_events("mapping.json").unwrap_or_default();
    let mapping = mapping.into_iter().next();

    // ── Collect all track entries ──
    let mut track_entries: Vec<(u8, u8, u8, Vec<yinhe_project::Note>)> = Vec::new();

    for (_path, entry) in &archive.entries {
        if entry.header.magic == *b"YHTK" {
            let notes = if entry.header.version >= yinhe_project::NOTES_VERSION_DELTA_GATE {
                yinhe_project::decode_notes_delta_gate(&entry.data)
            } else if let Ok(v) = bincode::deserialize::<Vec<yinhe_project::Note>>(&entry.data) {
                v
            } else {
                continue;
            };
            track_entries.push((entry.header.port, entry.header.channel, entry.header.extra, notes));
        }
    }

    track_entries.sort_by_key(|e| e.2);

    let num_tracks = track_entries
        .iter()
        .map(|e| e.2 as usize + 1)
        .max()
        .unwrap_or(1);

    midi.track_ports = vec![0; num_tracks];
    midi.track_names = (0..num_tracks)
        .map(|i| format!("Track {}", i + 1))
        .collect();

    if let Some(mapping) = &mapping {
        for port_mapping in &mapping.ports {
            for ch_mapping in &port_mapping.channels {
                for (track_idx, track_mapping) in ch_mapping.tracks.iter().enumerate() {
                    let idx = track_idx.min(num_tracks.saturating_sub(1));
                    midi.track_names[idx] = track_mapping.name.clone();
                    if idx < midi.track_ports.len() {
                        midi.track_ports[idx] = port_mapping.port;
                    }
                }
            }
        }
    }

    // ── Rebuild key_notes ──
    for (port, channel, track_idx, notes) in &track_entries {
        let idx = *track_idx as usize;
        if idx >= num_tracks {
            continue;
        }
        midi.track_ports[idx] = *port;
        for note in notes {
            let key = note.key as usize;
            if key < 128 {
                midi.key_notes[key].push(yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    channel: *channel,
                    track: *track_idx as u16,
                });
            }
        }
    }

    for notes in &mut midi.key_notes {
        notes.sort_by_key(|n| n.start_tick);
    }

    // ── Rebuild control events ──
    for (_path, entry) in &archive.entries {
        let h = entry.header;
        if h.magic == *b"YHCC" {
            if let Ok(events) = bincode::deserialize::<Vec<CcEvent>>(&entry.data) {
                for ev in events {
                    midi.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
                        tick: ev.tick,
                        channel: h.channel,
                        controller: h.extra,
                        value: ev.value,
                        track: h.extra as u16,
                    });
                }
            }
        } else if h.magic == *b"YHPB" {
            if let Ok(events) = bincode::deserialize::<Vec<PitchBendEvent>>(&entry.data) {
                for ev in events {
                    midi.control_events.push(yinhe_midi::MidiControlEvent::PitchBend {
                        tick: ev.tick,
                        channel: h.channel,
                        value: ev.value,
                        track: h.extra as u16,
                    });
                }
            }
        } else if h.magic == *b"YHPC" {
            if let Ok(events) = bincode::deserialize::<Vec<PcEvent>>(&entry.data) {
                for ev in events {
                    midi.control_events.push(yinhe_midi::MidiControlEvent::ProgramChange {
                        tick: ev.tick,
                        channel: h.channel,
                        program: ev.program,
                        track: h.extra as u16,
                    });
                }
            }
        }
    }

    midi.control_events.sort_by_key(|e| match e {
        yinhe_midi::MidiControlEvent::ControlChange { tick, .. } => *tick,
        yinhe_midi::MidiControlEvent::ProgramChange { tick, .. } => *tick,
        yinhe_midi::MidiControlEvent::PitchBend { tick, .. } => *tick,
    });

    // ── Recompute derived fields ──
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

    midi.automation_lanes = yinhe_midi::build_automation_lanes(&midi.control_events, &midi.key_notes);

    midi
}

/// Save a document as a .yin file.
pub fn save_project(doc: &crate::document::Document, path: &str) -> std::io::Result<()> {
    let archive = build_archive(doc);
    archive.write_to(path)
}

/// Build a ProjectArchive from a Document (without writing to disk).
pub fn build_archive(doc: &crate::document::Document) -> ProjectArchive {
    build_archive_from(
        &doc.midi,
        &doc.track_names,
        &doc.project_name,
        &doc.project_artist,
        doc.project_ppq,
        doc.archive.as_ref().map(|a| a.compression_level).unwrap_or(0),
        &doc.project_description,
    )
}

/// Build a ProjectArchive from raw fields (usable from a background thread).
pub fn build_archive_from(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    project_name: &str,
    project_artist: &str,
    project_ppq: u32,
    compression_level: i32,
    project_description: &str,
) -> ProjectArchive {
    let mut archive = midi_to_archive_with_names(midi, track_names);

    let proj = ProjectJson {
        version: 1,
        name: project_name.to_string(),
        artist: project_artist.to_string(),
        ppq: project_ppq,
        zstd_level: compression_level,
        description: project_description.to_string(),
    };
    archive.set_events("project.json", FileHeader::new(*b"YHPR", 0, 0, 0), &[proj]);

    archive.compression_level = compression_level;
    archive
}

/// Load a .yin file and return a MidiFile + file stem name + the archive.
pub fn load_project_full(path: &str) -> std::io::Result<(yinhe_midi::MidiFile, String, ProjectArchive)> {
    let archive = ProjectArchive::read_from(path)?;
    let midi = archive_to_midi(&archive);

    let file_name = std::path::Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_default();

    Ok((midi, file_name, archive))
}

/// Load a .yin file and return a MidiFile + file stem name.
pub fn load_project(path: &str) -> std::io::Result<(yinhe_midi::MidiFile, String)> {
    let (midi, file_name, _) = load_project_full(path)?;
    Ok((midi, file_name))
}

/// Export the current document as a standard MIDI file.
pub fn export_midi(doc: &crate::document::Document, path: &str) -> Result<(), String> {
    let midi = &doc.midi;
    let num_tracks = midi.track_ports.len();
    let ppq = midi.ticks_per_beat;

    let mut tracks: Vec<Vec<TrackEvent>> = Vec::new();

    for track_idx in 0..num_tracks {
        let mut events: Vec<(u32, TrackEventKind)> = Vec::new();

        for (key_idx, key_notes) in midi.key_notes.iter().enumerate() {
            let key_u7 = u7::new(key_idx as u8);
            for note in key_notes {
                if note.track as usize != track_idx {
                    continue;
                }
                events.push((
                    note.start_tick,
                    TrackEventKind::Midi {
                        channel: u4::new(note.channel & 0x0F),
                        message: MidiMessage::NoteOn {
                            key: key_u7,
                            vel: u7::new(note.velocity),
                        },
                    },
                ));
                events.push((
                    note.end_tick,
                    TrackEventKind::Midi {
                        channel: u4::new(note.channel & 0x0F),
                        message: MidiMessage::NoteOff {
                            key: key_u7,
                            vel: u7::new(0),
                        },
                    },
                ));
            }
        }

        for ev in &midi.control_events {
            let (tick, track, channel, kind) = match ev {
                yinhe_midi::MidiControlEvent::ControlChange {
                    tick, channel, controller, value, track
                } => (*tick, *track, *channel, TrackEventKind::Midi {
                    channel: u4::new(*channel & 0x0F),
                    message: MidiMessage::Controller {
                        controller: u7::new(*controller),
                        value: u7::new(*value),
                    },
                }),
                yinhe_midi::MidiControlEvent::ProgramChange {
                    tick, channel, program, track
                } => (*tick, *track, *channel, TrackEventKind::Midi {
                    channel: u4::new(*channel & 0x0F),
                    message: MidiMessage::ProgramChange {
                        program: u7::new(*program),
                    },
                }),
                yinhe_midi::MidiControlEvent::PitchBend {
                    tick, channel, value, track
                } => (*tick, *track, *channel, TrackEventKind::Midi {
                    channel: u4::new(*channel & 0x0F),
                    message: MidiMessage::PitchBend {
                        bend: PitchBend::from_int(*value),
                    },
                }),
            };
            if track as usize == track_idx {
                events.push((tick, kind));
            }
        }

        events.sort_by_key(|e| e.0);

        let mut abs_tick = 0u32;
        let mut track_events = Vec::new();

        if let Some(name) = doc.track_names.get(track_idx) {
            track_events.push(TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
            });
        }

        for (tick, kind) in events {
            let delta = tick.saturating_sub(abs_tick);
            track_events.push(TrackEvent {
                delta: delta.into(),
                kind,
            });
            abs_tick = tick;
        }

        track_events.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });

        tracks.push(track_events);
    }

    // Build conductor track
    let mut conductor_events: Vec<(u32, TrackEventKind)> = Vec::new();

    for seg in &midi.tempo_segments {
        conductor_events.push((
            seg.start_tick,
            TrackEventKind::Meta(MetaMessage::Tempo(midly::num::u24::new(
                seg.micros_per_quarter as u32,
            ))),
        ));
    }

    for ev in &midi.time_sig_events {
        conductor_events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                ev.numerator,
                ev.denominator,
                24,
                8,
            )),
        ));
    }

    conductor_events.sort_by_key(|e| e.0);

    let mut abs_tick = 0u32;
    let mut conductor_track = Vec::new();
    for (tick, kind) in conductor_events {
        let delta = tick.saturating_sub(abs_tick);
        conductor_track.push(TrackEvent {
            delta: delta.into(),
            kind,
        });
        abs_tick = tick;
    }
    conductor_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    tracks.insert(0, conductor_track);

    let smf = Smf {
        header: Header {
            format: Format::SingleTrack,
            timing: Timing::Metrical(u15::new(ppq as u16)),
        },
        tracks: tracks.into_iter().map(|t| t.into_iter().collect()).collect(),
    };

    let mut buf = Vec::new();
    smf.write(&mut buf).map_err(|e| format!("{e}"))?;
    std::fs::write(path, &buf).map_err(|e| format!("{e}"))?;
    Ok(())
}
