use crate::MidiError;
use crate::midi::{LoadProgress, MidiControlEvent, MidiFile, Note};
use crate::time::{DEFAULT_MPQ, TIMECODE_FALLBACK_TPB, ticks_to_seconds};
use std::path::Path;
use yinhe_types::NoteScanIndex;

/// Global tempo event, sorted by tick.
#[derive(Clone, Debug)]
struct TempoEvent {
    tick: u32,
    micros_per_quarter: u64,
}

/// A note currently being played (waiting for NoteOff).
#[derive(Clone, Copy, Debug)]
struct ActiveNote {
    key: u8,
    start_time: f64,
    velocity: u8,
    channel: u8,
    start_tick: u32,
    track: u16,
}

pub struct MidiParser;

impl MidiParser {
    pub fn load(path: impl AsRef<Path>) -> Result<MidiFile, MidiError> {
        Self::load_with_progress(path, |_| {})
    }

    pub fn load_with_progress(
        path: impl AsRef<Path>,
        progress: impl FnMut(LoadProgress),
    ) -> Result<MidiFile, MidiError> {
        let data = std::fs::read(path.as_ref())?;
        Self::parse_bytes_with_progress(&data, progress)
    }

    pub fn parse_bytes_with_progress(
        data: &[u8],
        mut progress: impl FnMut(LoadProgress),
    ) -> Result<MidiFile, MidiError> {
        let smf = midly::Smf::parse(data)?;

        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(t) => t.as_int() as u32,
            midly::Timing::Timecode(_, _) => TIMECODE_FALLBACK_TPB,
        };

        let tempo_events = Self::collect_tempo_events(&smf.tracks);
        let tempo_segments = Self::build_tempo_segments(tempo_events, ticks_per_beat);

        let time_sig_events = Self::collect_time_sig_events(&smf.tracks);
        let (time_sig_numerator, time_sig_denominator) = time_sig_events
            .first()
            .map(|e| (e.numerator, e.denominator))
            .unwrap_or((4, 4));

        let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
        let mut global_duration = 0.0f64;
        let mut track_ports: Vec<u8> = Vec::with_capacity(smf.tracks.len());
        let mut track_names: Vec<String> = Vec::with_capacity(smf.tracks.len());
        let mut control_events: Vec<MidiControlEvent> = Vec::new();
        let total_tracks = smf.tracks.len();

        for (track_idx, track) in smf.tracks.iter().enumerate() {
            progress(LoadProgress {
                current_track: track_idx + 1,
                total_tracks,
            });
            // Extract track name from MetaMessage::TrackName
            let track_name = track.iter().find_map(|ev| {
                if let midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(name)) = ev.kind {
                    Some(String::from_utf8_lossy(name).into_owned())
                } else {
                    None
                }
            }).unwrap_or_else(|| format!("Track {}", track_idx + 1));
            track_names.push(track_name);

            let port = Self::parse_track(
                track,
                &tempo_segments,
                ticks_per_beat,
                track_idx as u16,
                &mut key_notes,
                &mut global_duration,
                &mut control_events,
            );
            track_ports.push(port);
        }

        // Sort each key's notes by start time.
        for notes in &mut key_notes {
            notes.sort_by(|a, b| {
                a.start
                    .partial_cmp(&b.start)
                    .expect("note start times should never be NaN")
            });
        }

        let note_count = key_notes.iter().map(|v| v.len() as u64).sum();
        let tick_length = key_notes
            .iter()
            .flat_map(|v| v.iter().map(|n| n.end_tick as u64))
            .max()
            .unwrap_or(0);

        // Build scan index for fast note seeking at render time.
        let scan_index = NoteScanIndex::build(&key_notes, tick_length);

        Ok(MidiFile {
            key_notes,
            duration: global_duration,
            ticks_per_beat,
            tempo_segments,
            note_count,
            tick_length,
            time_sig_numerator,
            time_sig_denominator,
            time_sig_events,
            track_names,
            track_ports,
            control_events,
            scan_index: Some(scan_index),
        })
    }

    fn collect_tempo_events(tracks: &[midly::Track]) -> Vec<TempoEvent> {
        let mut events = Vec::new();
        for track in tracks {
            let mut tick: u32 = 0;
            for event in track {
                tick += event.delta.as_int();
                if let midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(us)) = event.kind {
                    events.push(TempoEvent {
                        tick,
                        micros_per_quarter: us.as_int() as u64,
                    });
                }
            }
        }
        events.sort_by_key(|e| e.tick);
        events.dedup_by_key(|e| e.tick);
        events
    }

    fn collect_time_sig_events(tracks: &[midly::Track]) -> Vec<crate::midi::TimeSigEvent> {
        let mut events = Vec::new();
        for track in tracks {
            let mut tick: u32 = 0;
            for event in track {
                tick += event.delta.as_int();
                if let midly::TrackEventKind::Meta(midly::MetaMessage::TimeSignature(
                    numerator,
                    denominator,
                    _,
                    _,
                )) = event.kind
                {
                    events.push(crate::midi::TimeSigEvent {
                        tick,
                        numerator,
                        denominator,
                    });
                }
            }
        }
        events.sort_by_key(|e| e.tick);
        events.dedup_by_key(|e| e.tick);
        events
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
                last_time += ticks_to_seconds(dtick, ticks_per_beat, last_mpq);
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

    fn parse_track(
        track: &midly::Track,
        segments: &[crate::TempoSegment],
        ticks_per_beat: u32,
        track_idx: u16,
        key_notes: &mut [Vec<Note>; 128],
        global_duration: &mut f64,
        control_events: &mut Vec<MidiControlEvent>,
    ) -> u8 {
        let mut active_notes: Vec<ActiveNote> = Vec::new();
        let mut current_tick: u32 = 0;
        let mut current_seconds: f64 = 0.0;
        let mut seg_idx: usize = 0;
        let mut current_port: u8 = 0;

        for event in track {
            let new_tick = current_tick + event.delta.as_int();
            let delta = new_tick - current_tick;

            if delta > 0 {
                let (new_seconds, new_seg_idx) = Self::advance_time(
                    current_tick,
                    current_seconds,
                    new_tick,
                    seg_idx,
                    segments,
                    ticks_per_beat,
                );
                current_tick = new_tick;
                current_seconds = new_seconds;
                seg_idx = new_seg_idx;
            } else {
                current_tick = new_tick;
            }

            match event.kind {
                midly::TrackEventKind::Meta(midly::MetaMessage::MidiPort(port)) => {
                    current_port = port.as_int();
                }
                midly::TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    let global_ch = current_port * 16 + ch;
                    match message {
                        midly::MidiMessage::NoteOn { key, vel } => {
                            let k = key.as_int();
                            if vel.as_int() > 0 {
                                active_notes.push(ActiveNote {
                                    key: k,
                                    start_time: current_seconds,
                                    velocity: vel.as_int(),
                                    channel: global_ch,
                                    start_tick: current_tick,
                                    track: track_idx,
                                });
                            } else {
                                Self::resolve_note_off(
                                    k,
                                    global_ch,
                                    current_seconds,
                                    current_tick,
                                    &mut active_notes,
                                    key_notes,
                                    global_duration,
                                );
                            }
                        }
                        midly::MidiMessage::NoteOff { key, .. } => {
                            let k = key.as_int();
                            Self::resolve_note_off(
                                k,
                                global_ch,
                                current_seconds,
                                current_tick,
                                &mut active_notes,
                                key_notes,
                                global_duration,
                            );
                        }
                        midly::MidiMessage::Controller { controller, value } => {
                            control_events.push(MidiControlEvent::ControlChange {
                                tick: current_tick,
                                channel: global_ch,
                                controller: controller.as_int(),
                                value: value.as_int(),
                                track: track_idx,
                            });
                        }
                        midly::MidiMessage::ProgramChange { program } => {
                            control_events.push(MidiControlEvent::ProgramChange {
                                tick: current_tick,
                                channel: global_ch,
                                program: program.as_int(),
                                track: track_idx,
                            });
                        }
                        midly::MidiMessage::PitchBend { bend } => {
                            control_events.push(MidiControlEvent::PitchBend {
                                tick: current_tick,
                                channel: global_ch,
                                value: bend.as_int(),
                                track: track_idx,
                            });
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        current_port
    }

    fn advance_time(
        current_tick: u32,
        current_seconds: f64,
        target_tick: u32,
        mut seg_idx: usize,
        segments: &[crate::TempoSegment],
        ticks_per_beat: u32,
    ) -> (f64, usize) {
        let mut tick_cursor = current_tick;
        let mut sec_cursor = current_seconds;

        while seg_idx + 1 < segments.len() && segments[seg_idx + 1].start_tick <= target_tick {
            let boundary = segments[seg_idx + 1].start_tick;
            let d = boundary - tick_cursor;
            sec_cursor += ticks_to_seconds(d, ticks_per_beat, segments[seg_idx].micros_per_quarter);
            tick_cursor = boundary;
            seg_idx += 1;
        }

        let d = target_tick - tick_cursor;
        sec_cursor += ticks_to_seconds(d, ticks_per_beat, segments[seg_idx].micros_per_quarter);

        (sec_cursor, seg_idx)
    }

    fn resolve_note_off(
        key: u8,
        channel: u8,
        end_time: f64,
        end_tick: u32,
        active_notes: &mut Vec<ActiveNote>,
        key_notes: &mut [Vec<Note>; 128],
        global_duration: &mut f64,
    ) {
        if let Some(idx) = active_notes
            .iter()
            .rposition(|n| n.key == key && n.channel == channel)
        {
            let n = active_notes.swap_remove(idx);
            *global_duration = global_duration.max(end_time);
            key_notes[n.key as usize].push(Note {
                key: n.key,
                start: n.start_time,
                end: end_time,
                start_tick: n.start_tick,
                end_tick,
                velocity: n.velocity,
                channel: n.channel,
                track: n.track,
            });
        }
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
            control_events: Vec::new(),
            scan_index: None,
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
            control_events: Vec::new(),
            scan_index: None,
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

        // Verify each NOTE and CC track's channel matches its name number
        // e.g. NOTE 1 → A01, NOTE 12 → A12, CC 2-1 → A02, etc.
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
        // Verify all tracks on port A for this file
        assert!(info.iter().all(|ti| ti.port == 0));
        // Track #0 (Cyber Night) and #1 (Eye) have no events at all
        assert_eq!(info[0].channel, 1);
        assert_eq!(info[1].channel, 1);
    }
}
