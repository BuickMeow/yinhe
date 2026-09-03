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
use yinhe_types::{AutomationTarget, Note, SegmentShape};

use crate::encoding::MidiImportEncoding;
use crate::error::MidiError;

/// MIDI导出选项（对应 设置→MIDI导出 栏）。
#[derive(Debug, Clone)]
pub struct MidiExportOptions {
    pub encoding: MidiImportEncoding,
    pub rpn_full: bool,
    pub curve_interpolate: bool,
    pub curve_density: u32,
    pub strip_empty_tracks: bool,
    pub dedup_overlaps: bool,
}

impl Default for MidiExportOptions {
    fn default() -> Self {
        Self {
            encoding: MidiImportEncoding::Utf8,
            rpn_full: true,
            curve_interpolate: false,
            curve_density: 1,
            strip_empty_tracks: true,
            dedup_overlaps: false,
        }
    }
}

fn encode_text(encoding: MidiImportEncoding, s: &str) -> Vec<u8> {
    if encoding == MidiImportEncoding::Utf8 {
        s.as_bytes().to_vec()
    } else {
        encoding.encode(s)
    }
}

fn leak_bytes(v: Vec<u8>) -> &'static [u8] {
    Box::leak(v.into_boxed_slice())
}

/// Serialize a `YinModel` to SMF bytes (Standard MIDI File, format 1).
pub fn write_to_bytes(model: &YinModel) -> Result<Vec<u8>, MidiError> {
    write_with_options(model, &MidiExportOptions::default())
}

/// 带选项的导出（供设置栏驱动）。
pub fn write_with_options(
    model: &YinModel,
    opts: &MidiExportOptions,
) -> Result<Vec<u8>, MidiError> {
    let ppq = model.meta.ppq;

    let mut tracks: Vec<Vec<TrackEvent<'_>>> = Vec::with_capacity(model.tracks.len() + 1);
    tracks.push(build_conductor_track_with_encoding(model, opts.encoding));

    let num_tracks = model.tracks.len();
    let mut per_track_notes: Vec<Vec<(Note, u8)>> = vec![Vec::new(); num_tracks];
    for (key, bucket) in model.notes.iter().enumerate() {
        for n in bucket.iter() {
            let t = n.track as usize;
            if t < num_tracks {
                per_track_notes[t].push((*n, key as u8));
            }
        }
    }

    if opts.dedup_overlaps {
        for notes in per_track_notes.iter_mut() {
            if notes.len() <= 1 {
                continue;
            }
            notes.sort_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| a.0.start_tick.cmp(&b.0.start_tick))
                    .then_with(|| a.0.end_tick.cmp(&b.0.end_tick))
            });
            let mut deduped: Vec<(Note, u8)> = Vec::with_capacity(notes.len());
            let mut last_end: std::collections::HashMap<u8, u32> = std::collections::HashMap::new();
            for (n, k) in notes.drain(..) {
                if let Some(&le) = last_end.get(&k) {
                    if n.start_tick < le {
                        continue;
                    }
                }
                last_end.insert(k, n.end_tick);
                deduped.push((n, k));
            }
            *notes = deduped;
        }
    }

    let color_payloads: Vec<Option<Vec<u8>>> = model
        .tracks
        .iter()
        .map(|t| {
            if t.color != yinhe_core::DEFAULT_TRACK_COLOR {
                Some(vec![
                    0x00,
                    0x0F,
                    0x7F,
                    0x00,
                    (t.color[0] * 255.0).round() as u8,
                    (t.color[1] * 255.0).round() as u8,
                    (t.color[2] * 255.0).round() as u8,
                    (t.color[3] * 255.0).round() as u8,
                ])
            } else {
                None
            }
        })
        .collect();

    for (i, t) in model.tracks.iter().enumerate() {
        let notes = &per_track_notes[i];
        let is_empty = notes.is_empty()
            && t.automation_lanes.is_empty()
            && t.program_change.is_empty()
            && t.lyrics.is_empty();
        if opts.strip_empty_tracks && is_empty {
            continue;
        }
        tracks.push(build_track_with_options(
            t,
            notes,
            color_payloads[i].as_deref(),
            opts,
        ));
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
        .map_err(|e| MidiError::Io(std::io::Error::other(e.to_string())))?;
    Ok(buf)
}

fn build_conductor_track<'a>(model: &'a YinModel) -> Vec<TrackEvent<'a>> {
    build_conductor_track_with_encoding(model, MidiImportEncoding::Utf8)
}

fn build_conductor_track_with_encoding<'a>(
    model: &'a YinModel,
    encoding: MidiImportEncoding,
) -> Vec<TrackEvent<'a>> {
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
    for ev in &model.conductor.key_sig {
        let (sf, mi) = ev.scale.to_midi_sf_mi(ev.root);
        events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::KeySignature(sf, mi != 0)),
        ));
    }
    for ev in &model.conductor.markers {
        let bytes: &'a [u8] = if encoding == MidiImportEncoding::Utf8 {
            ev.text.as_bytes()
        } else {
            leak_bytes(encode_text(encoding, &ev.text))
        };
        events.push((ev.tick, TrackEventKind::Meta(MetaMessage::Marker(bytes))));
    }
    for ev in &model.conductor.lyrics {
        let bytes: &'a [u8] = if encoding == MidiImportEncoding::Utf8 {
            ev.text.as_bytes()
        } else {
            leak_bytes(encode_text(encoding, &ev.text))
        };
        events.push((ev.tick, TrackEventKind::Meta(MetaMessage::Lyric(bytes))));
    }

    events.sort_by_key(|e| e.0);

    if model.meta.name.is_empty() {
        flatten_to_track(events, None)
    } else {
        let encoded: &'a [u8] = if encoding == MidiImportEncoding::Utf8 {
            model.meta.name.as_bytes()
        } else {
            leak_bytes(encode_text(encoding, &model.meta.name))
        };
        flatten_to_track_with_bytes(events, Some(encoded))
    }
}

fn build_track<'a>(
    track: &'a TrackData,
    notes: &[(Note, u8)],
    color_payload: Option<&'a [u8]>,
) -> Vec<TrackEvent<'a>> {
    build_track_with_options(track, notes, color_payload, &MidiExportOptions::default())
}

fn build_track_with_options<'a>(
    track: &'a TrackData,
    notes: &[(Note, u8)],
    color_payload: Option<&'a [u8]>,
    opts: &MidiExportOptions,
) -> Vec<TrackEvent<'a>> {
    let ch = u4::new(track.channel & 0x0F);
    let mut events: Vec<(u32, TrackEventKind<'a>)> = Vec::new();

    for (n, key) in notes {
        events.push((
            n.start_tick,
            TrackEventKind::Midi {
                channel: ch,
                message: MidiMessage::NoteOn {
                    key: u7::new((*key) & 0x7F),
                    vel: u7::new(n.velocity & 0x7F),
                },
            },
        ));
        events.push((
            n.end_tick,
            TrackEventKind::Midi {
                channel: ch,
                message: MidiMessage::NoteOff {
                    key: u7::new((*key) & 0x7F),
                    vel: u7::new(0),
                },
            },
        ));
    }

    for lane in &track.automation_lanes {
        let n = lane.events.len();
        for (idx, ev) in lane.events.iter().enumerate() {
            let v = ev.value.round() as u16;
            push_lane_event(&mut events, lane, ev.tick, v, ch, opts.rpn_full);
            if opts.curve_interpolate && idx + 1 < n && !matches!(ev.shape, SegmentShape::Step) {
                let next = &lane.events[idx + 1];
                let tick1 = ev.tick;
                let tick2 = next.tick;
                if tick2 > tick1 {
                    let v1 = ev.value;
                    let v2 = next.value;
                    let span = (tick2 - tick1) as f32;
                    let density = opts.curve_density.max(1);
                    let mut t = tick1.saturating_add(density);
                    while t < tick2 {
                        let frac = (t - tick1) as f32 / span;
                        let f = ev.shape.interpolate(frac);
                        let v = v1 + (v2 - v1) * f;
                        let vi = v.round() as u16;
                        push_lane_event(&mut events, lane, t, vi, ch, opts.rpn_full);
                        t = t.saturating_add(density);
                    }
                }
            }
        }
    }

    for ev in &track.program_change {
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

    if track.port != 0 {
        events.push((
            0,
            TrackEventKind::Meta(MetaMessage::MidiPort(midly::num::u7::new(
                track.port & 0x7F,
            ))),
        ));
    }
    if let Some(payload) = color_payload {
        events.push((0, TrackEventKind::Meta(MetaMessage::Unknown(0x0A, payload))));
    }
    if let Some(ch) = track.channel_prefix {
        events.push((
            0,
            TrackEventKind::Meta(MetaMessage::MidiChannel(midly::num::u4::new(ch & 0x0F))),
        ));
    }

    for ev in &track.lyrics {
        let bytes: &'a [u8] = if opts.encoding == MidiImportEncoding::Utf8 {
            ev.text.as_bytes()
        } else {
            leak_bytes(encode_text(opts.encoding, &ev.text))
        };
        events.push((ev.tick, TrackEventKind::Meta(MetaMessage::Lyric(bytes))));
    }

    events.sort_by_key(|e| e.0);

    if track.name.is_empty() {
        flatten_to_track(events, None)
    } else {
        let encoded: &'a [u8] = if opts.encoding == MidiImportEncoding::Utf8 {
            track.name.as_bytes()
        } else {
            leak_bytes(encode_text(opts.encoding, &track.name))
        };
        flatten_to_track_with_bytes(events, Some(encoded))
    }
}

fn push_lane_event<'a>(
    events: &mut Vec<(u32, TrackEventKind<'a>)>,
    lane: &yinhe_types::AutomationLane,
    tick: u32,
    v: u16,
    ch: u4,
    rpn_full: bool,
) {
    match &lane.target {
        AutomationTarget::CC { controller } => {
            events.push((
                tick,
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
                tick,
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
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(101),
                        value: u7::new(msb),
                    },
                },
            ));
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(100),
                        value: u7::new(lsb),
                    },
                },
            ));
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(6),
                        value: u7::new(data_msb),
                    },
                },
            ));
            let should_emit_lsb = if rpn_full {
                lane.target.is_14bit()
            } else {
                data_lsb != 0 && lane.target.is_14bit()
            };
            if should_emit_lsb {
                events.push((
                    tick,
                    TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(38),
                            value: u7::new(data_lsb),
                        },
                    },
                ));
            }
        }
        AutomationTarget::Nrpn { parameter } => {
            let msb = ((parameter >> 8) & 0x7F) as u8;
            let lsb = (parameter & 0x7F) as u8;
            let data_msb = ((v >> 7) & 0x7F) as u8;
            let data_lsb = (v & 0x7F) as u8;
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(99),
                        value: u7::new(msb),
                    },
                },
            ));
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(98),
                        value: u7::new(lsb),
                    },
                },
            ));
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(6),
                        value: u7::new(data_msb),
                    },
                },
            ));
            let should_emit_lsb = if rpn_full { true } else { data_lsb != 0 };
            if should_emit_lsb {
                events.push((
                    tick,
                    TrackEventKind::Midi {
                        channel: ch,
                        message: MidiMessage::Controller {
                            controller: u7::new(38),
                            value: u7::new(data_lsb),
                        },
                    },
                ));
            }
        }
        AutomationTarget::Tempo => {}
    }
}

fn flatten_to_track<'a>(
    events: Vec<(u32, TrackEventKind<'a>)>,
    track_name: Option<&'a str>,
) -> Vec<TrackEvent<'a>> {
    let bytes = track_name.map(|s| s.as_bytes());
    flatten_to_track_with_bytes(events, bytes)
}

fn flatten_to_track_with_bytes<'a>(
    events: Vec<(u32, TrackEventKind<'a>)>,
    track_name_bytes: Option<&'a [u8]>,
) -> Vec<TrackEvent<'a>> {
    let mut out = Vec::with_capacity(events.len() + 2);
    if let Some(name) = track_name_bytes {
        out.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(name)),
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
