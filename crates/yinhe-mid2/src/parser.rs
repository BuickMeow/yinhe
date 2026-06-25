//! Parse Standard MIDI File bytes directly into a `yinhe_core::YinModel`.
//!
//! Single-pass per track: NoteOn/NoteOff pairing, port/channel prefix
//! tracking, CC/PB/PC collection, and RPN state-machine decoding all
//! happen in one walk. Conductor events (tempo, time signature) are
//! collected across all tracks first.

use std::collections::BTreeMap;
use std::path::Path;

use rayon::prelude::*;

use yinhe_core::{
    CcEvent, ConductorData, NoteEvent, PcEvent, PitchBendEvent, ProjectMeta, RpnEvent, TempoEvent,
    TimeSigEvent, TrackData, YinModel,
};

use crate::encoding::MidiImportEncoding;
use crate::error::MidiError;

/// Progress reporter type for long-running parses.
#[derive(Clone, Copy, Debug)]
pub struct LoadProgress {
    pub current_track: usize,
    pub total_tracks: usize,
}

/// Fallback ticks-per-beat for SMPTE-timecode MIDI files (which we don't
/// fully support; treat as if metrical with 480 ppq).
const TIMECODE_FALLBACK_TPB: u32 = 480;

/// Parse a .mid file from disk.
pub fn parse_path(path: impl AsRef<Path>) -> Result<YinModel, MidiError> {
    yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
        let data = std::fs::read(path.as_ref())?;
        parse_bytes(&data)
    })
}

/// Parse .mid bytes (UTF-8 track names) without progress callback.
pub fn parse_bytes(data: &[u8]) -> Result<YinModel, MidiError> {
    parse_bytes_with_encoding(data, MidiImportEncoding::Utf8, |_| {})
}

/// Parse .mid bytes with a chosen track-name encoding and progress callback.
pub fn parse_bytes_with_encoding(
    data: &[u8],
    encoding: MidiImportEncoding,
    mut progress: impl FnMut(LoadProgress),
) -> Result<YinModel, MidiError> {
    yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
        // 惰性解析：只切出 header + track 块迭代器，不构建全量事件树喵～
        let (header, track_iter) = midly::parse(data)?;

        let ticks_per_beat = match header.timing {
            midly::Timing::Metrical(t) => t.as_int() as u32,
            midly::Timing::Timecode(_, _) => TIMECODE_FALLBACK_TPB,
        };

        // Pass 1: collect conductor events (tempo + time-sig) across ALL tracks.
        // 克隆一个惰性迭代器逐事件扫描，扫完即丢，常驻 O(1)。
        let conductor = collect_conductor(track_iter.clone())?;

        // Pass 2: per-track parse → TrackData, run in parallel across tracks.
        // Each track parses independently (all state in parse_track is local),
        // so we collect the per-track EventIters first, then fan them out with
        // rayon. Results are gathered in original track order.
        // Skip "conductor-only" tracks: those with no MIDI messages at all
        // (only meta events). These are typical of SMF format-1 files where
        // track 0 is a conductor track.
        let track_events: Vec<midly::EventIter> =
            track_iter.clone().collect::<Result<Vec<_>, _>>()?;
        let total_tracks = track_events.len();
        progress(LoadProgress {
            current_track: total_tracks,
            total_tracks,
        });

        let parsed: Vec<Option<TrackData>> = track_events
            .into_par_iter()
            .enumerate()
            .map(|(track_idx, events)| parse_track(events, track_idx, encoding))
            .collect::<Result<Vec<_>, _>>()?;

        // Drop skipped tracks and assign fallback names by final position.
        let mut tracks: Vec<TrackData> = Vec::with_capacity(total_tracks);
        let mut per_track_notes: Vec<Vec<NoteEvent>> = Vec::with_capacity(total_tracks);
        for mut td in parsed.into_iter().flatten() {
            if td.name.is_empty() {
                td.name = format!("Track {}", tracks.len() + 1);
            }
            per_track_notes.push(td.notes);
            td.notes = Vec::new(); // notes moved out, clear to avoid confusion
            tracks.push(td);
        }

        let meta = ProjectMeta {
            ppq: ticks_per_beat,
            ..ProjectMeta::default()
        };

        let mut model = YinModel {
            conductor: std::sync::Arc::new(conductor),
            tracks: tracks.into_iter().map(std::sync::Arc::new).collect(),
            meta,
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();

        Ok(model)
    })
}

// =========================================================
//  Conductor pass (across all tracks)
// =========================================================

fn collect_conductor(track_iter: midly::TrackIter) -> Result<ConductorData, MidiError> {
    let mut tempo: Vec<TempoEvent> = Vec::new();
    let mut time_sig: Vec<TimeSigEvent> = Vec::new();

    for track_result in track_iter {
        let events = track_result?;
        let mut tick: u32 = 0;
        for ev in events {
            let ev = ev?;
            tick += ev.delta.as_int();
            match ev.kind {
                midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(us)) => {
                    let mpq = us.as_int() as u64;
                    let bpm = if mpq == 0 {
                        120.0
                    } else {
                        60_000_000.0 / mpq as f64
                    };
                    tempo.push(TempoEvent { tick, bpm });
                }
                midly::TrackEventKind::Meta(midly::MetaMessage::TimeSignature(num, den, _, _)) => {
                    time_sig.push(TimeSigEvent {
                        tick,
                        numerator: num,
                        denominator: den,
                    });
                }
                _ => {}
            }
        }
    }

    tempo.sort_by_key(|e| e.tick);
    tempo.dedup_by_key(|e| e.tick);
    time_sig.sort_by_key(|e| e.tick);
    time_sig.dedup_by_key(|e| e.tick);

    Ok(ConductorData { tempo, time_sig })
}

// =========================================================
//  Per-track pass
// =========================================================

#[derive(Clone, Copy, Debug)]
struct ActiveNote {
    key: u8,
    velocity: u8,
    start_tick: u32,
    /// Composite (port << 4) | channel of the NoteOn — must match for NoteOff
    /// to pair. Different channels in the same track produce independent
    /// active-note stacks.
    global_channel: u8,
}

/// Per-channel RPN state machine.
///
/// MIDI RPNs are selected with CC101 (MSB) + CC100 (LSB), then written by
/// CC6 (Data Entry MSB) and CC38 (Data Entry LSB). We track the most recent
/// (msb, lsb) per channel; when CC6/38 arrives with both selected, emit an
/// RpnEvent (and DON'T store CC101/100/6/38 as plain CC).
#[derive(Default, Clone, Copy)]
struct RpnState {
    msb: Option<u8>,
    lsb: Option<u8>,
}

/// Per-channel pending Bank Select state.
///
/// CC 0 (Bank MSB) and CC 32 (Bank LSB) are buffered here and folded into
/// the next ProgramChange on the same tick. If no PC follows, they are
/// flushed to `td.cc` at the end of the track.
#[derive(Default, Clone, Copy)]
struct PendingBank {
    msb: Option<(u8, u32)>, // (value, tick)
    lsb: Option<(u8, u32)>, // (value, tick)
}

fn parse_track(
    events: midly::EventIter,
    track_idx: usize,
    encoding: MidiImportEncoding,
) -> Result<Option<TrackData>, MidiError> {
    let mut td = TrackData::new(0, 0);
    td.uuid = uuid::Uuid::new_v4().to_string();

    let mut current_tick: u32 = 0;
    let mut current_port: u8 = 0;
    let mut active_notes: Vec<ActiveNote> = Vec::new();
    let mut first_global_channel: Option<u8> = None;
    // Track whether this track carries any MIDI message. Conductor-only tracks
    // (meta events only, typical of SMF format-1 track 0) are skipped — their
    // tempo/time-sig were already collected by `collect_conductor`.
    let mut has_midi_message = false;

    // RPN state per channel (channel 0..16).
    let mut rpn_state: [RpnState; 16] = [RpnState::default(); 16];

    // Pending Bank Select per channel (CC 0 / CC 32).
    let mut pending_bank: [PendingBank; 16] = [PendingBank::default(); 16];

    // Collect raw per-controller CC streams; we'll filter RPN selectors out
    // when we're sure they belong to an RPN sequence. Selectors that DON'T
    // resolve into an RpnEvent fall back to plain CC.
    // For simplicity we emit RpnEvent the moment CC6/38 arrives with both
    // msb/lsb known, and we do NOT store the corresponding CC101/100/6/38
    // in `cc`. Lone CC101/100 with no CC6 follow stays as plain CC.
    //
    // We can't know in advance if a CC101/100 will be followed by CC6,
    // so we buffer them per channel and only commit them as plain CC if
    // the channel sees a non-RPN-related event before any CC6/38 closes
    // the sequence.
    //
    // Simpler rule (matches yinhe-model::convert::from_midi semantics):
    // - CC101 / CC100 are stored as plain CC ONLY if no matching CC6 closes
    //   the sequence within the SAME tick on the same channel.
    // We'll use a simpler approximation: whenever CC6/38 fires with both
    // msb/lsb selected → emit RpnEvent and DROP the most recent
    // CC101/100/6/38 we just stored on this channel. This is what yinhe-model
    // approximates by grouping by (track, tick).

    for ev in events {
        let ev = ev?;
        current_tick += ev.delta.as_int();
        match ev.kind {
            midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(name_bytes)) => {
                if td.name.is_empty() {
                    td.name = encoding.decode(name_bytes);
                }
            }
            midly::TrackEventKind::Meta(midly::MetaMessage::MidiPort(port)) => {
                current_port = port.as_int();
            }
            midly::TrackEventKind::Meta(midly::MetaMessage::MidiChannel(ch)) => {
                td.channel_prefix = Some(ch.as_int());
            }
            midly::TrackEventKind::Midi { channel, message } => {
                has_midi_message = true;
                let ch_raw = channel.as_int();
                let global_ch = (current_port & 0x0F) << 4 | (ch_raw & 0x0F);
                if first_global_channel.is_none() {
                    first_global_channel = Some(global_ch);
                }

                match message {
                    midly::MidiMessage::NoteOn { key, vel } => {
                        let k = key.as_int();
                        let v = vel.as_int();
                        if v > 0 {
                            active_notes.push(ActiveNote {
                                key: k,
                                velocity: v,
                                start_tick: current_tick,
                                global_channel: global_ch,
                            });
                        } else {
                            // NoteOn with vel=0 == NoteOff
                            resolve_note_off(k, global_ch, current_tick, &mut active_notes, &mut td.notes);
                        }
                    }
                    midly::MidiMessage::NoteOff { key, .. } => {
                        let k = key.as_int();
                        resolve_note_off(k, global_ch, current_tick, &mut active_notes, &mut td.notes);
                    }
                    midly::MidiMessage::Controller { controller, value } => {
                        let cc = controller.as_int();
                        let val = value.as_int();
                        let ch_idx = ch_raw as usize;
                        match cc {
                            101 => {
                                rpn_state[ch_idx].msb = Some(val);
                                // Don't store CC101 — it's an RPN selector
                            }
                            100 => {
                                rpn_state[ch_idx].lsb = Some(val);
                                // Don't store CC100 — it's an RPN selector
                            }
                            0 => {
                                // Bank MSB: buffer for potential PC folding
                                pending_bank[ch_idx].msb = Some((val, current_tick));
                            }
                            32 => {
                                // Bank LSB: buffer for potential PC folding
                                pending_bank[ch_idx].lsb = Some((val, current_tick));
                            }
                            6 => {
                                // Data Entry MSB: emit RpnEvent if RPN is selected
                                let st = rpn_state[ch_idx];
                                if let (Some(msb), Some(lsb)) = (st.msb, st.lsb) {
                                    let key = ((msb as u16) << 8) | lsb as u16;
                                    td.rpn.entry(key).or_default().push(RpnEvent {
                                        tick: current_tick,
                                        value: (val as u16) << 7,
                                    });
                                } else {
                                    // No RPN selected — store as plain CC6
                                    td.cc.entry(6).or_default().push(CcEvent {
                                        tick: current_tick,
                                        value: val,
                                    });
                                }
                            }
                            38 => {
                                // Data Entry LSB: append low 7 bits to most recent RPN value
                                let st = rpn_state[ch_idx];
                                if let (Some(msb), Some(lsb)) = (st.msb, st.lsb) {
                                    let key = ((msb as u16) << 8) | lsb as u16;
                                    if let Some(events) = td.rpn.get_mut(&key) {
                                        if let Some(last) = events.last_mut() {
                                            // Combine: existing MSB in high bits, new LSB low
                                            last.value = (last.value & 0xFF80) | (val as u16);
                                            continue;
                                        }
                                    }
                                    // No prior MSB on this RPN — store as standalone with LSB only
                                    td.rpn
                                        .entry(key)
                                        .or_default()
                                        .push(RpnEvent {
                                            tick: current_tick,
                                            value: val as u16,
                                        });
                                } else {
                                    td.cc.entry(38).or_default().push(CcEvent {
                                        tick: current_tick,
                                        value: val,
                                    });
                                }
                            }
                            _ => {
                                td.cc.entry(cc).or_default().push(CcEvent {
                                    tick: current_tick,
                                    value: val,
                                });
                            }
                        }
                    }
                    midly::MidiMessage::ProgramChange { program } => {
                        let ch_idx = ch_raw as usize;
                        let bank_msb_val = pending_bank[ch_idx].msb;
                        let bank_lsb_val = pending_bank[ch_idx].lsb;
                        let bank_msb = bank_msb_val
                            .filter(|&(_, t)| t == current_tick)
                            .map(|(v, _)| v)
                            .unwrap_or(0xFF);
                        let bank_lsb = bank_lsb_val
                            .filter(|&(_, t)| t == current_tick)
                            .map(|(v, _)| v)
                            .unwrap_or(0xFF);
                        td.program_change.push(PcEvent {
                            tick: current_tick,
                            program: program.as_int(),
                            bank_msb,
                            bank_lsb,
                        });
                        // Clear pending bank values that were consumed (same tick)
                        if bank_msb_val.is_some_and(|(_, t)| t == current_tick) {
                            pending_bank[ch_idx].msb = None;
                        }
                        if bank_lsb_val.is_some_and(|(_, t)| t == current_tick) {
                            pending_bank[ch_idx].lsb = None;
                        }
                    }
                    midly::MidiMessage::PitchBend { bend } => {
                        td.pitch_bend.push(PitchBendEvent {
                            tick: current_tick,
                            value: bend.as_int(),
                        });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Skip conductor-only tracks (no MIDI messages at all).
    if !has_midi_message {
        return Ok(None);
    }

    // Pin port/channel from first MIDI event seen, or default.
    let _ = track_idx; // (kept for future use)
    td.port = current_port;
    td.channel = first_global_channel
        .map(|gc| gc & 0x0F)
        .unwrap_or(0);

    // Flush pending bank values that were NOT consumed by a ProgramChange.
    // These become plain CC events so nothing is lost.
    for bank in &pending_bank {
        if let Some((val, tick)) = bank.msb {
            td.cc.entry(0).or_default().push(CcEvent { tick, value: val });
        }
        if let Some((val, tick)) = bank.lsb {
            td.cc.entry(32).or_default().push(CcEvent { tick, value: val });
        }
    }

    // Assign dup_index for any (key, start_tick) collisions.
    assign_dup_indices(&mut td.notes);

    // Sort all event streams by tick (rebuild() also sorts, but per-track
    // streams are expected sorted for fast partition_point queries).
    td.notes.sort_by_key(|n| (n.start_tick, n.key, n.dup_index));
    for v in td.cc.values_mut() {
        v.sort_by_key(|e| e.tick);
    }
    td.pitch_bend.sort_by_key(|e| e.tick);
    td.program_change.sort_by_key(|e| e.tick);
    for v in td.rpn.values_mut() {
        v.sort_by_key(|e| e.tick);
    }

    Ok(Some(td))
}

/// Match a NoteOff (or NoteOn vel=0) to the most recent matching NoteOn.
fn resolve_note_off(
    key: u8,
    global_ch: u8,
    end_tick: u32,
    active: &mut Vec<ActiveNote>,
    notes: &mut Vec<NoteEvent>,
) {
    if let Some(idx) = active
        .iter()
        .rposition(|n| n.key == key && n.global_channel == global_ch)
    {
        let n = active.swap_remove(idx);
        notes.push(NoteEvent {
            start_tick: n.start_tick,
            end_tick,
            key: n.key,
            velocity: n.velocity,
            dup_index: 0, // assigned later
        });
    }
}

/// Walk notes and assign `dup_index` for any colliding `(key, start_tick)`.
///
/// Notes are inserted in NoteOff order, but dup_index should be insertion
/// order. We use a BTreeMap<(key, start_tick), u8> counter.
fn assign_dup_indices(notes: &mut [NoteEvent]) {
    let mut counter: BTreeMap<(u8, u32), u8> = BTreeMap::new();
    // Sort first by start_tick so we assign dup_index in start order.
    notes.sort_by_key(|n| (n.start_tick, n.key));
    for n in notes.iter_mut() {
        let k = (n.key, n.start_tick);
        let entry = counter.entry(k).or_insert(0);
        n.dup_index = *entry;
        *entry = entry.saturating_add(1);
    }
}
