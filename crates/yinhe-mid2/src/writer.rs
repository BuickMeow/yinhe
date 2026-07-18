//! Write a `yinhe_core::YinModel` as Standard MIDI File bytes.
//!
//! Produces a Type-1 SMF: track 0 is the conductor (tempo + time-sig),
//! tracks 1..N+1 are the YinModel tracks. Each YinModel track flattens
//! `notes / automation_lanes / program_change` into one event stream
//! sorted by tick. RPN/NRPN lanes expand to their CC selector sequences.

use midly::num::{u4, u7, u15, u24};
use midly::{
    Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind,
};

use yinhe_core::{TrackData, YinModel};
use yinhe_types::AutomationTarget;

use crate::error::MidiError;

/// Serialize a `YinModel` to SMF bytes (Standard MIDI File, format 1).
pub fn write_to_bytes(model: &YinModel) -> Result<Vec<u8>, MidiError> {
    let ppq = model.meta.ppq;

    let mut tracks: Vec<Vec<TrackEvent<'_>>> = Vec::with_capacity(model.tracks.len() + 1);

    // Track 0: conductor (tempo + time signature)
    tracks.push(build_conductor_track(model));

    // Tracks 1..N+1: per-track event streams
    for (i, t) in model.tracks.iter().enumerate() {
        tracks.push(build_track(t, i as u16, model));
    }

    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(u15::new(ppq as u16)),
        },
        tracks,
    };

    let mut buf = Vec::new();
    smf.write(&mut buf)
        .map_err(|e| MidiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;
    Ok(buf)
}

fn build_conductor_track<'a>(model: &'a YinModel) -> Vec<TrackEvent<'a>> {
    let mut events: Vec<(u32, TrackEventKind<'a>)> = Vec::new();

    for ev in &model.conductor.tempo.events {
        let bpm = ev.value as f64;
        let mpq = if bpm > 0.0 {
            (60_000_000.0 / bpm).round() as u32
        } else {
            500_000
        };
        events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(mpq))),
        ));
    }
    for ev in &model.conductor.time_sig {
        events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                ev.numerator,
                ev.denominator,
                24,
                8,
            )),
        ));
    }

    events.sort_by_key(|e| e.0);
    flatten_to_track(events, None)
}

fn build_track<'a>(track: &'a TrackData, track_idx: u16, model: &'a YinModel) -> Vec<TrackEvent<'a>> {
    let ch = u4::new(track.channel & 0x0F);
    let mut events: Vec<(u32, TrackEventKind<'a>)> = Vec::new();

    // Notes → NoteOn + NoteOff pairs
    for (key, bucket) in model.notes.iter().enumerate() {
        for n in bucket.iter().filter(|n| n.track == track_idx) {
            events.push((
                n.start_tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::NoteOn {
                        key: u7::new((key as u8) & 0x7F),
                        vel: u7::new(n.velocity & 0x7F),
                    },
                },
            ));
            events.push((
                n.end_tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::NoteOff {
                        key: u7::new((key as u8) & 0x7F),
                        vel: u7::new(0),
                    },
                },
            ));
        }
    }

    // Automation lanes → MIDI events
    for lane in &track.automation_lanes {
        for ev in &lane.events {
            // f32 → u16 一次，所有位运算都用这个整数
            let v = ev.value.round() as u16;
            match &lane.target {
                AutomationTarget::CC { controller } => {
                    events.push((
                        ev.tick,
                        TrackEventKind::Midi {
                            channel: ch,
                            message: MidiMessage::Controller {
                                controller: u7::new(*controller & 0x7F),
                                value: u7::new((v & 0x7F) as u8),
                            },
                        },
                    ));
                }
                AutomationTarget::PitchBend => {
                    events.push((
                        ev.tick,
                        TrackEventKind::Midi {
                            channel: ch,
                            message: MidiMessage::PitchBend {
                                bend: PitchBend(midly::num::u14::new(v)),
                            },
                        },
                    ));
                }
                AutomationTarget::Rpn { parameter } => {
                    let msb = ((parameter >> 8) & 0x7F) as u8;
                    let lsb = (parameter & 0x7F) as u8;
                    let (data_msb, data_lsb) = if lane.target.is_14bit() {
                        (((v >> 7) & 0x7F) as u8, (v & 0x7F) as u8)
                    } else {
                        (v as u8, 0u8)
                    };
                    // CC101 (RPN MSB)
                    events.push((ev.tick, TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(101),
                            value: u7::new(msb),
                        },
                    }));
                    // CC100 (RPN LSB)
                    events.push((ev.tick, TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(100),
                            value: u7::new(lsb),
                        },
                    }));
                    // CC6 (Data Entry MSB)
                    events.push((ev.tick, TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(6),
                            value: u7::new(data_msb),
                        },
                    }));
                    // CC38 (Data Entry LSB) — only for 14-bit targets with non-zero LSB
                    if data_lsb != 0 && lane.target.is_14bit() {
                        events.push((ev.tick, TrackEventKind::Midi {
                            channel: ch,
                            message: MidiMessage::Controller {
                                controller: u7::new(38),
                                value: u7::new(data_lsb),
                            },
                        }));
                    }
                }
                AutomationTarget::Nrpn { parameter } => {
                    let msb = ((parameter >> 8) & 0x7F) as u8;
                    let lsb = (parameter & 0x7F) as u8;
                    let data_msb = ((v >> 7) & 0x7F) as u8;
                    let data_lsb = (v & 0x7F) as u8;
                    // CC99 (NRPN MSB)
                    events.push((ev.tick, TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(99),
                            value: u7::new(msb),
                        },
                    }));
                    // CC98 (NRPN LSB)
                    events.push((ev.tick, TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(98),
                            value: u7::new(lsb),
                        },
                    }));
                    // CC6 (Data Entry MSB)
                    events.push((ev.tick, TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(6),
                            value: u7::new(data_msb),
                        },
                    }));
                    // CC38 (Data Entry LSB) only if non-zero
                    if data_lsb != 0 {
                        events.push((ev.tick, TrackEventKind::Midi {
                            channel: ch,
                            message: MidiMessage::Controller {
                                controller: u7::new(38),
                                value: u7::new(data_lsb),
                            },
                        }));
                    }
                }
                // Tempo 走 `conductor.tempo`（已在 build_conductor_track 写出），
                // 不应出现在 track.automation_lanes 里。
                AutomationTarget::Tempo => {}
            }
        }
    }

    // Program change + Bank Select
    for ev in &track.program_change {
        // Bank Select MSB (CC 0) — only if set (0xFF = unset)
        if ev.bank_msb != 0xFF {
            events.push((
                ev.tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(0),
                        value: u7::new(ev.bank_msb & 0x7F),
                    },
                },
            ));
        }
        // Bank Select LSB (CC 32) — only if set
        if ev.bank_lsb != 0xFF {
            events.push((
                ev.tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(32),
                        value: u7::new(ev.bank_lsb & 0x7F),
                    },
                },
            ));
        }
        events.push((
            ev.tick,
            TrackEventKind::Midi {
                channel: ch,
                message: MidiMessage::ProgramChange {
                    program: u7::new(ev.program & 0x7F),
                },
            },
        ));
    }

    // MidiPort meta (FF 21) — preserves port info on roundtrip
    if track.port != 0 {
        events.push((
            0,
            TrackEventKind::Meta(MetaMessage::MidiPort(midly::num::u7::new(
                track.port & 0x7F,
            ))),
        ));
    }
    // MidiChannel meta (FF 20) — preserves channel prefix on roundtrip
    if let Some(ch) = track.channel_prefix {
        events.push((
            0,
            TrackEventKind::Meta(MetaMessage::MidiChannel(midly::num::u4::new(
                ch & 0x0F,
            ))),
        ));
    }

    // Stable sort by tick.
    events.sort_by_key(|e| e.0);

    let track_name = if track.name.is_empty() {
        None
    } else {
        Some(track.name.as_str())
    };
    flatten_to_track(events, track_name)
}

fn flatten_to_track<'a>(
    events: Vec<(u32, TrackEventKind<'a>)>,
    track_name: Option<&'a str>,
) -> Vec<TrackEvent<'a>> {
    let mut out = Vec::with_capacity(events.len() + 2);
    if let Some(name) = track_name {
        out.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
        });
    }
    let mut last_tick: u32 = 0;
    for (tick, kind) in events {
        let delta = tick.saturating_sub(last_tick);
        out.push(TrackEvent {
            delta: delta.into(),
            kind,
        });
        last_tick = tick;
    }
    out.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    out
}
