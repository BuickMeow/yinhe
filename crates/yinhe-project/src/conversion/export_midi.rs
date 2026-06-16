use midly::num::{u4, u7, u15};
use midly::{
    Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind,
};

/// Export a MidiFile as a standard MIDI file.
pub fn export_midi(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    path: &str,
) -> Result<(), String> {
    let num_tracks = midi.track_ports.len();
    let ppq = midi.ticks_per_beat;

    let mut tracks: Vec<Vec<TrackEvent>> = Vec::new();

    for track_idx in 0..num_tracks {
        let track_ch = midi
            .track_channels
            .get(track_idx)
            .copied()
            .unwrap_or(0);

        let events = collect_track_events(midi, track_idx, track_ch);
        let track_events = build_track_event_list(events, track_names.get(track_idx));
        tracks.push(track_events);
    }

    let conductor_track = build_conductor_track(midi);
    tracks.insert(0, conductor_track);

    let smf = Smf {
        header: Header {
            format: Format::SingleTrack,
            timing: Timing::Metrical(u15::new(ppq as u16)),
        },
        tracks: tracks
            .into_iter()
            .map(|t| t.into_iter().collect())
            .collect(),
    };

    let mut buf = Vec::new();
    smf.write(&mut buf).map_err(|e| format!("{e}"))?;
    std::fs::write(path, &buf).map_err(|e| format!("{e}"))?;
    Ok(())
}

fn collect_track_events(
    midi: &yinhe_midi::MidiFile,
    track_idx: usize,
    track_ch: u8,
) -> Vec<(u32, TrackEventKind<'_>)> {
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
                    channel: u4::new(track_ch & 0x0F),
                    message: MidiMessage::NoteOn {
                        key: key_u7,
                        vel: u7::new(note.velocity),
                    },
                },
            ));
            events.push((
                note.end_tick,
                TrackEventKind::Midi {
                    channel: u4::new(track_ch & 0x0F),
                    message: MidiMessage::NoteOff {
                        key: key_u7,
                        vel: u7::new(0),
                    },
                },
            ));
        }
    }

    for ev in &midi.control_events {
        let (tick, track, kind) = match ev {
            yinhe_midi::MidiControlEvent::ControlChange {
                tick,
                controller,
                value,
                track,
            } => (
                *tick,
                *track,
                TrackEventKind::Midi {
                    channel: u4::new(track_ch & 0x0F),
                    message: MidiMessage::Controller {
                        controller: u7::new(*controller),
                        value: u7::new(*value),
                    },
                },
            ),
            yinhe_midi::MidiControlEvent::ProgramChange {
                tick,
                program,
                track,
            } => (
                *tick,
                *track,
                TrackEventKind::Midi {
                    channel: u4::new(track_ch & 0x0F),
                    message: MidiMessage::ProgramChange {
                        program: u7::new(*program),
                    },
                },
            ),
            yinhe_midi::MidiControlEvent::PitchBend {
                tick,
                value,
                track,
            } => (
                *tick,
                *track,
                TrackEventKind::Midi {
                    channel: u4::new(track_ch & 0x0F),
                    message: MidiMessage::PitchBend {
                        bend: PitchBend::from_int(*value),
                    },
                },
            ),
        };
        if track as usize == track_idx {
            events.push((tick, kind));
        }
    }

    events.sort_by_key(|e| e.0);
    events
}

fn build_track_event_list<'a>(
    events: Vec<(u32, TrackEventKind<'a>)>,
    track_name: Option<&'a String>,
) -> Vec<TrackEvent<'a>> {
    let mut abs_tick = 0u32;
    let mut track_events = Vec::new();

    if let Some(name) = track_name {
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

    track_events
}

fn build_conductor_track(midi: &yinhe_midi::MidiFile) -> Vec<TrackEvent<'_>> {
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

    conductor_track
}
