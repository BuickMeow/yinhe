mod export_midi;
mod to_archive;
mod to_midi;

use std::collections::HashMap;

use crate::archive::ProjectArchive;
use crate::events::*;
use crate::header::*;
use crate::schema::*;

pub use export_midi::export_midi;
pub use to_archive::{midi_to_archive, midi_to_archive_with_names};
pub use to_midi::archive_to_midi;

// ── Shared utilities ──

pub(super) fn rpn_number(msb: u8, lsb: u8) -> Option<u8> {
    match (msb, lsb) {
        (0, 0) => Some(0),
        (1, 0) => Some(1),
        (2, 0) => Some(2),
        _ => None,
    }
}

pub(super) fn extract_uuid(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("channels/")?;
    let slash1 = rest.find('/')?;
    let after_label = &rest[slash1 + 1..];
    let slash2 = after_label.find('/')?;
    Some(&after_label[..slash2])
}

pub(super) struct TrackEventGroup {
    pub notes: Vec<Note>,
    pub cc_events: Vec<(u8, CcEvent)>,
    pub rpn_events: Vec<(u8, RpnEvent)>,
    pub pitch_events: Vec<PitchBendEvent>,
    pub pc_events: Vec<PcEvent>,
}

impl TrackEventGroup {
    pub fn new() -> Self {
        Self {
            notes: Vec::new(),
            cc_events: Vec::new(),
            rpn_events: Vec::new(),
            pitch_events: Vec::new(),
            pc_events: Vec::new(),
        }
    }

    pub fn has_any_events(&self) -> bool {
        !self.notes.is_empty()
            || !self.cc_events.is_empty()
            || !self.rpn_events.is_empty()
            || !self.pitch_events.is_empty()
            || !self.pc_events.is_empty()
    }
}

pub(super) fn add_to_port_map(
    port_map: &mut HashMap<u8, Vec<(u8, Vec<TrackMapping>)>>,
    port: u8,
    raw_channel: u8,
    uuid: &str,
    name: String,
    new_idx: usize,
    channel_prefix: Option<u8>,
) {
    let channels_entry = port_map.entry(port).or_default();
    let ch_entry = if let Some(existing) = channels_entry
        .iter_mut()
        .find(|(c, _)| *c == raw_channel)
    {
        existing
    } else {
        channels_entry.push((raw_channel, Vec::new()));
        channels_entry.last_mut().unwrap()
    };
    ch_entry.1.push(TrackMapping {
        uuid: uuid.to_string(),
        name,
        color: [0.5, 0.5, 0.5],
        track_index: new_idx as u16,
        channel_prefix,
    });
}

// ── Public API wrappers ──

/// Build a ProjectArchive from raw fields (usable from a background thread).
pub fn build_archive_from(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    project_name: &str,
    project_artist: &str,
    project_ppq: u32,
    compression_level: i32,
    project_description: &str,
    project_sf_overrides: &[(u8, Vec<SfEntryJson>)],
    global_enabled: bool,
) -> ProjectArchive {
    let mut archive = midi_to_archive_with_names(midi, track_names, None);

    let soundfont_overrides: Vec<SfPortOverride> = project_sf_overrides
        .iter()
        .map(|(port, entries)| SfPortOverride {
            port: *port,
            entries: entries.clone(),
        })
        .collect();

    let proj = ProjectJson {
        version: 1,
        name: project_name.to_string(),
        artist: project_artist.to_string(),
        ppq: project_ppq,
        zstd_level: compression_level,
        description: project_description.to_string(),
        soundfont_project_mode: !global_enabled,
        soundfont_overrides,
    };
    archive.set_json("project.json", FileHeader::new(*b"YHPR", 0, 0, 0), &proj);

    archive.compression_level = compression_level;
    archive
}

/// Load a .yin file and return a MidiFile + file stem name + the archive.
pub fn load_project_full(
    path: &str,
) -> std::io::Result<(yinhe_midi::MidiFile, String, ProjectArchive)> {
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

// ── Roundtrip tests ──

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use yinhe_midi::MidiControlEvent;
    use yinhe_midi::MidiFile;
    use yinhe_types::{Note, TimeSigEvent as TypesTimeSigEvent};

    fn make_test_midi() -> MidiFile {
        let mut m = MidiFile::default();
        m.ticks_per_beat = 480;
        m.track_ports = vec![0, 0, 1];
        m.track_channel_prefixes = vec![None, None, None];
        m.track_channels = vec![0, 1, 16];
        m.track_names = vec!["Lead".into(), "Bass".into(), "Drums".into()];

        m.key_notes[60].push(Note {
            start_tick: 0,
            end_tick: 480,
            velocity: 100,
            track: 0,
        });
        m.key_notes[60].push(Note {
            start_tick: 480,
            end_tick: 960,
            velocity: 100,
            track: 0,
        });
        m.key_notes[48].push(Note {
            start_tick: 0,
            end_tick: 1920,
            velocity: 90,
            track: 1,
        });
        m.key_notes[36].push(Note {
            start_tick: 0,
            end_tick: 240,
            velocity: 120,
            track: 2,
        });

        m.control_events
            .push(MidiControlEvent::ControlChange {
                tick: 0,
                controller: 7,
                value: 100,
                track: 0,
            });
        m.control_events
            .push(MidiControlEvent::ControlChange {
                tick: 240,
                controller: 7,
                value: 80,
                track: 0,
            });
        m.control_events
            .push(MidiControlEvent::PitchBend {
                tick: 100,
                value: 1024,
                track: 1,
            });
        m.control_events
            .push(MidiControlEvent::ProgramChange {
                tick: 0,
                program: 7,
                track: 2,
            });

        m.tempo_segments = vec![
            yinhe_midi::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: yinhe_midi::mpq_from_bpm(120.0),
            },
            yinhe_midi::TempoSegment {
                start_tick: 1920,
                start_time: 0.0,
                micros_per_quarter: yinhe_midi::mpq_from_bpm(140.0),
            },
        ];
        yinhe_midi::recompute_tempo_start_times(&mut m.tempo_segments, m.ticks_per_beat);

        m.time_sig_events = vec![
            TypesTimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            },
            TypesTimeSigEvent {
                tick: 1920,
                numerator: 3,
                denominator: 2,
            },
        ];

        m.note_count = m.key_notes.iter().map(|n| n.len() as u64).sum();
        m.tick_length = 1920;
        m
    }

    #[test]
    fn roundtrip_preserves_notes_and_channels() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        assert_eq!(restored.ticks_per_beat, 480);
        assert_eq!(restored.track_ports.len(), 3);
        assert_eq!(restored.track_ports, vec![0, 0, 1]);

        assert_eq!(restored.key_notes[60].len(), 2, "track 0 notes at key 60");
        assert!(restored.key_notes[60].iter().all(|n| n.track == 0));
        assert_eq!(restored.track_channels[0], 0);
        assert_eq!(restored.key_notes[48].len(), 1);
        assert_eq!(restored.key_notes[48][0].track, 1);
        assert_eq!(restored.track_channels[1], 1);
        assert_eq!(restored.key_notes[36].len(), 1);
        assert_eq!(restored.key_notes[36][0].track, 2);
        assert_eq!(restored.track_channels[2], 16);
    }

    #[test]
    fn roundtrip_preserves_control_events() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        let cc_count = restored
            .control_events
            .iter()
            .filter(|e| matches!(e, MidiControlEvent::ControlChange { .. }))
            .count();
        assert_eq!(cc_count, 2);
        let pb_count = restored
            .control_events
            .iter()
            .filter(|e| matches!(e, MidiControlEvent::PitchBend { .. }))
            .count();
        assert_eq!(pb_count, 1);
        let pc_count = restored
            .control_events
            .iter()
            .filter(|e| matches!(e, MidiControlEvent::ProgramChange { .. }))
            .count();
        assert_eq!(pc_count, 1);

        let pb = restored
            .control_events
            .iter()
            .find_map(|e| match e {
                MidiControlEvent::PitchBend {
                    tick,
                    value,
                    track,
                } => Some((*tick, *value, *track)),
                _ => None,
            })
            .unwrap();
        assert_eq!(pb, (100, 1024, 1));
        assert_eq!(restored.track_channels[1], 1);
    }

    #[test]
    fn roundtrip_preserves_tempo_and_time_sig() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        assert!(!restored.tempo_segments.is_empty());
        let bpm0 = yinhe_midi::bpm_from_mpq(restored.tempo_segments[0].micros_per_quarter);
        assert!(
            (bpm0 - 120.0).abs() < 0.5,
            "expected ~120 BPM at tick 0, got {bpm0}"
        );
        let has_140 = restored.tempo_segments.iter().any(|s| {
            s.start_tick == 1920
                && (yinhe_midi::bpm_from_mpq(s.micros_per_quarter) - 140.0).abs() < 0.5
        });
        assert!(
            has_140,
            "expected 140 BPM segment at tick 1920, got {:?}",
            restored.tempo_segments
        );

        assert_eq!(restored.time_sig_events.len(), 2);
        assert_eq!(restored.time_sig_events[0].numerator, 4);
        assert_eq!(restored.time_sig_events[1].numerator, 3);
        assert_eq!(restored.time_sig_events[1].tick, 1920);
    }

    #[test]
    fn roundtrip_no_tempo_yields_default_segment_at_zero() {
        let mut m = MidiFile::default();
        m.ticks_per_beat = 480;
        m.track_ports = vec![0];
        m.track_names = vec!["t".into()];
        m.tempo_segments.clear();
        m.time_sig_events.clear();

        let archive = midi_to_archive(&m);
        let restored = archive_to_midi(&archive);

        assert_eq!(restored.tempo_segments.len(), 1);
        assert_eq!(restored.tempo_segments[0].start_tick, 0);
        let bpm = yinhe_midi::bpm_from_mpq(restored.tempo_segments[0].micros_per_quarter);
        assert!((bpm - 120.0).abs() < 0.5);
    }

    #[test]
    fn roundtrip_preserves_track_names() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        assert_eq!(restored.track_names, vec!["Lead", "Bass", "Drums"]);
    }

    #[test]
    fn roundtrip_rpn_events() {
        let mut m = make_test_midi();
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 100,
                controller: 6,
                value: 2,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 100,
                controller: 101,
                value: 0,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 100,
                controller: 100,
                value: 0,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 200,
                controller: 101,
                value: 1,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 200,
                controller: 100,
                value: 0,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 200,
                controller: 6,
                value: 50,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 300,
                controller: 101,
                value: 2,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 300,
                controller: 100,
                value: 0,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 300,
                controller: 6,
                value: 24,
                track: 0,
            });
        m.control_events
            .push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: 300,
                controller: 38,
                value: 127,
                track: 0,
            });

        let archive = midi_to_archive(&m);
        let restored = archive_to_midi(&archive);

        let rpn_ccs: Vec<_> = restored
            .control_events
            .iter()
            .filter_map(|ev| match ev {
                yinhe_midi::MidiControlEvent::ControlChange {
                    controller: 101 | 100 | 6,
                    ..
                } => Some(ev),
                _ => None,
            })
            .collect();
        assert_eq!(rpn_ccs.len(), 9, "expected 9 RPN-related CCs");
    }
}
