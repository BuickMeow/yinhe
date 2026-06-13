use crate::MidiError;
use crate::event_collector::TempoEvent;
use crate::midi::{LoadProgress, MidiControlEvent, MidiFile, Note};
use crate::time::{DEFAULT_MPQ, TIMECODE_FALLBACK_TPB, ticks_to_seconds};
use std::path::Path;
use yinhe_types::NoteScanIndex;

pub struct MidiParser;

impl MidiParser {
    pub fn load(path: impl AsRef<Path>) -> Result<MidiFile, MidiError> {
        Self::load_with_progress(path, |_| {})
    }

    pub fn load_with_progress(
        path: impl AsRef<Path>,
        progress: impl FnMut(LoadProgress),
    ) -> Result<MidiFile, MidiError> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
            let data = std::fs::read(path.as_ref())?;
            Self::parse_bytes_with_progress_owned(data, progress)
        })
    }

    pub fn parse_bytes_with_progress(
        data: &[u8],
        progress: impl FnMut(LoadProgress),
    ) -> Result<MidiFile, MidiError> {
        Self::parse_bytes_with_progress_owned(data.to_vec(), progress)
    }

    pub fn parse_bytes_with_progress_owned(
        data: Vec<u8>,
        mut progress: impl FnMut(LoadProgress),
    ) -> Result<MidiFile, MidiError> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
        let mut smf = midly::Smf::parse(&data)?;

        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(t) => t.as_int() as u32,
            midly::Timing::Timecode(_, _) => TIMECODE_FALLBACK_TPB,
        };

        let tempo_events = crate::event_collector::collect_tempo_events(&smf.tracks);
        let tempo_segments = Self::build_tempo_segments(tempo_events, ticks_per_beat);

        let time_sig_events = crate::event_collector::collect_time_sig_events(&smf.tracks);
        let (time_sig_numerator, time_sig_denominator) = time_sig_events
            .first()
            .map(|e| (e.numerator, e.denominator))
            .unwrap_or((4, 4));

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_end_tick: u64 = 0;
        let mut track_ports: Vec<u8> = Vec::with_capacity(smf.tracks.len());
        let mut track_channel_prefixes: Vec<Option<u8>> = Vec::with_capacity(smf.tracks.len());
        let mut track_names: Vec<String> = Vec::with_capacity(smf.tracks.len());
        let mut control_events: Vec<MidiControlEvent> = Vec::new();

        // Take ownership of the parsed tracks so we can consume them one by one
        // and free each track's event vector immediately after parsing. The raw
        // file bytes (`data`) are kept alive because the events may borrow them,
        // but only one track's events live in memory at a time, which prevents
        // the entire midly representation plus our MidiFile from coexisting.
        let mut tracks = std::mem::take(&mut smf.tracks);
        drop(smf);

        let total_tracks = tracks.len();
        for (track_idx, track) in tracks.iter_mut().enumerate() {
            progress(LoadProgress {
                current_track: track_idx + 1,
                total_tracks,
            });
            // Extract track name from MetaMessage::TrackName before clearing.
            let track_name = track.iter().find_map(|ev| {
                if let midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(name)) = ev.kind {
                    Some(String::from_utf8_lossy(name).into_owned())
                } else {
                    None
                }
            }).unwrap_or_else(|| format!("Track {}", track_idx + 1));
            track_names.push(track_name);

            let (port, channel_prefix) = crate::track_parser::parse_track(
                track,
                &tempo_segments,
                ticks_per_beat,
                track_idx as u16,
                &mut key_notes,
                &mut global_end_tick,
                &mut control_events,
            );
            track_ports.push(port);
            track_channel_prefixes.push(channel_prefix);

            // Free this track's event vector now that we've extracted everything
            // we need from it.
            track.clear();
            track.shrink_to_fit();
        }
        drop(tracks);
        drop(data);

        // Sort each key's notes by start tick.
        for notes in &mut key_notes {
            notes.sort_by_key(|n| n.start_tick);
        }

        let note_count = key_notes.iter().map(|v| v.len() as u64).sum();
        let tick_length = key_notes
            .iter()
            .flat_map(|v| v.iter().map(|n| n.end_tick as u64))
            .max()
            .unwrap_or(global_end_tick);

        // Build scan index for fast note seeking at render time.
        let scan_index = NoteScanIndex::build(&key_notes, tick_length);

        // Build coarse tick buckets so renderers can cull off-screen notes
        // without scanning entire keys.  65536 ticks is large enough that a
        // typical screen spans only a handful of buckets, while still being
        // fine-grained enough to avoid huge per-bucket scans.
        const BUCKET_SIZE: u32 = 65536;
        let tick_buckets = yinhe_types::TickBuckets::build(&key_notes, tick_length, BUCKET_SIZE);

        // Build automation lanes from control events and note velocity.
        let automation_lanes =
            crate::midi::build_automation_lanes(&control_events, &key_notes);

        Ok(MidiFile {
            key_notes,
            duration: 0.0, // computed on demand from tick_length
            ticks_per_beat,
            tempo_segments,
            note_count,
            tick_length,
            time_sig_numerator,
            time_sig_denominator,
            time_sig_events,
            track_names,
            track_ports,
            track_channel_prefixes,
            control_events,
            scan_index: Some(scan_index),
            tick_buckets: Some(tick_buckets),
            automation_lanes,
        })
        })
    }

    fn build_tempo_segments(
        events: Vec<TempoEvent>,
        ticks_per_beat: u32,
    ) -> Vec<crate::TempoSegment> {
        let mut segments = Vec::new();
        let mut last_tick: u32 = 0;
        let mut last_time: f64 = 0.0;
        let mut last_mpq: u64 = DEFAULT_MPQ;

        if events.is_empty() || events[0].tick > 0 {
            segments.push(crate::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: DEFAULT_MPQ,
            });
        }

        for ev in events {
            let dtick = ev.tick - last_tick;
            if dtick > 0 {
                last_time += ticks_to_seconds(dtick as u64, ticks_per_beat, last_mpq);
            }
            segments.push(crate::TempoSegment {
                start_tick: ev.tick,
                start_time: last_time,
                micros_per_quarter: ev.micros_per_quarter,
            });
            last_tick = ev.tick;
            last_mpq = ev.micros_per_quarter;
        }
        segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TempoSegment;

    #[test]
    fn test_build_tempo_segments_empty() {
        let segments = MidiParser::build_tempo_segments(vec![], 480);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_tick, 0);
        assert_eq!(segments[0].micros_per_quarter, DEFAULT_MPQ);
    }

    #[test]
    fn test_build_tempo_segments_single_event_at_zero() {
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_quarter: 250_000,
        }];
        let segments = MidiParser::build_tempo_segments(events, 480);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].micros_per_quarter, 250_000);
    }

    #[test]
    fn test_build_tempo_segments_two_events() {
        let events = vec![
            TempoEvent {
                tick: 0,
                micros_per_quarter: DEFAULT_MPQ,
            },
            TempoEvent {
                tick: 480,
                micros_per_quarter: 250_000,
            },
        ];
        let segments = MidiParser::build_tempo_segments(events, 480);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1].start_tick, 480);
        assert!((segments[1].start_time - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_midi_file_tick_at_time_no_tempo() {
        let midi = MidiFile {
            key_notes: std::array::from_fn(|_| Vec::new()),
            duration: 10.0,
            ticks_per_beat: 480,
            tempo_segments: vec![],
            note_count: 0,
            tick_length: 0,
            time_sig_numerator: 4,
            time_sig_denominator: 2,
            track_names: Vec::new(),
            time_sig_events: Vec::new(),
            track_ports: Vec::new(),
            track_channel_prefixes: Vec::new(),
            control_events: Vec::new(),
            scan_index: None,
            tick_buckets: None,
            automation_lanes: Vec::new(),
        };
        assert!((midi.tick_at_time(1.0) - 960.0).abs() < 1e-6);
        assert!((midi.tick_at_time(0.5) - 480.0).abs() < 1e-6);
    }

    #[test]
    fn test_midi_file_tick_at_time_with_tempo() {
        let midi = MidiFile {
            key_notes: std::array::from_fn(|_| Vec::new()),
            duration: 10.0,
            ticks_per_beat: 480,
            tempo_segments: vec![
                TempoSegment {
                    start_tick: 0,
                    start_time: 0.0,
                    micros_per_quarter: DEFAULT_MPQ,
                },
                TempoSegment {
                    start_tick: 480,
                    start_time: 0.5,
                    micros_per_quarter: 250_000,
                },
            ],
            note_count: 0,
            tick_length: 1440,
            time_sig_numerator: 4,
            time_sig_denominator: 2,
            track_names: Vec::new(),
            time_sig_events: Vec::new(),
            track_ports: Vec::new(),
            track_channel_prefixes: Vec::new(),
            control_events: Vec::new(),
            scan_index: None,
            tick_buckets: None,
            automation_lanes: Vec::new(),
        };
        assert!((midi.tick_at_time(0.5) - 480.0).abs() < 1e-6);
        assert!((midi.tick_at_time(1.0) - 1440.0).abs() < 1e-6);
    }

    #[test]
    #[ignore] // Depends on a local MIDI file not present in CI
    fn test_cyber_night_track_channel_mapping() {
        let midi = MidiFile::load("/Users/jieneng/Music/MIDIs/cyber-night.mid")
            .expect("failed to load cyber-night.mid");
        let info = midi.track_info();

        let expected: &[(usize, u8, &str)] = &[
            (2, 1, "NOTE 1"),
            (3, 1, "CC 1-1"),
            (4, 1, "CC 1-2"),
            (5, 2, "NOTE 2"),
            (6, 2, "CC 2-1"),
            (7, 2, "CC 2-2"),
            (8, 3, "NOTE 3"),
            (9, 3, "CC 3"),
            (10, 4, "NOTE 4"),
            (11, 4, "CC 4"),
            (12, 5, "NOTE 5"),
            (13, 5, "CC 5"),
            (14, 6, "NOTE 6"),
            (15, 6, "CC 6"),
            (16, 7, "NOTE 7"),
            (17, 7, "CC 7-1"),
            (18, 7, "CC 7-2"),
            (19, 8, "NOTE 8"),
            (20, 8, "CC 8"),
            (21, 9, "NOTE 9-1"),
            (22, 9, "NOTE 9-2"),
            (23, 9, "CC 9-1"),
            (24, 9, "CC 9-2"),
            (25, 10, "NOTE 10-1"),
            (26, 10, "NOTE 10-2"),
            (27, 10, "CC 10"),
            (28, 11, "NOTE 11"),
            (29, 11, "CC 11"),
            (30, 12, "NOTE 12"),
            (31, 12, "CC 12-1"),
            (32, 12, "CC 12-2"),
            (33, 13, "NOTE 13-1"),
            (34, 13, "NOTE 13-2"),
            (35, 13, "NOTE 13-3"),
            (36, 13, "NOTE 13-4"),
            (37, 13, "NOTE 13-5"),
            (38, 13, "NOTE 13-6"),
            (39, 13, "CC 13"),
            (40, 14, "NOTE 14"),
            (41, 14, "CC 14"),
            (42, 15, "NOTE 15"),
            (43, 15, "CC 15"),
            (44, 16, "NOTE 16-1"),
            (45, 16, "NOTE 16-2"),
            (46, 16, "CC 16"),
        ];
        for &(idx, ch, _name) in expected {
            assert_eq!(
                info[idx].channel, ch,
                "Track #{} ({}) expected channel {}",
                idx, info[idx].name, ch,
            );
        }
        assert!(info.iter().all(|ti| ti.port == 0));
        assert_eq!(info[0].channel, 1);
        assert_eq!(info[1].channel, 1);
    }

    #[test]
    fn test_load_from_bytes_minimal_midi() {
        let data = minimal_midi_bytes();
        let midi = MidiFile::load_from_bytes(&data).expect("failed to parse minimal MIDI");

        assert_eq!(midi.ticks_per_beat, 480);
        assert_eq!(midi.note_count, 1);
        assert_eq!(midi.key_notes[60].len(), 1);

        let note = &midi.key_notes[60][0];
        assert_eq!(note.velocity, 100);
        assert_eq!(note.start_tick, 0);
        assert_eq!(note.end_tick, 320);
    }

    fn minimal_midi_bytes() -> Vec<u8> {
        let mut data = Vec::new();
        // MThd: format 0, 1 track, 480 tpb
        data.extend_from_slice(b"MThd");
        data.extend_from_slice(&6u32.to_be_bytes());
        data.extend_from_slice(&[0, 0, 0, 1, 1, 0xE0]);
        // MTrk: tempo + NoteOn C4 + NoteOff C4
        data.extend_from_slice(b"MTrk");
        let track: &[u8] = &[
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // 120 BPM
            0x00, 0x90, 60, 100, // NoteOn
            0x82, 0x40, 0x80, 60, 0, // NoteOff (delta=320 ticks: 0x82=2<<7, 0x40=64, total=320)
            0x00, 0xFF, 0x2F, 0x00, // End
        ];
        data.extend_from_slice(&(track.len() as u32).to_be_bytes());
        data.extend_from_slice(track);
        data
    }
}
