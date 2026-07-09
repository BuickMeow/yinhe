//! Parse Standard MIDI File bytes directly into a `yinhe_core::YinModel`.
//!
//! Single-pass per track: NoteOn/NoteOff pairing, port/channel prefix
//! tracking, CC/PB/PC collection, and RPN/NRPN state-machine decoding all
//! happen in one walk. Conductor events (tempo, time signature) are
//! collected across all tracks first.
//!
//! Control events are unified into `AutomationLane` — one lane per
//! parameter per track. RPN and NRPN are decoded from their CC sequences
//! and stored as `AutomationTarget::Rpn` / `AutomationTarget::Nrpn`.

use std::path::Path;

use rayon::prelude::*;

use yinhe_core::{
    ConductorData, NoteEvent, PcEvent, ProjectMeta, TempoEvent, TrackData, YinModel,
};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, TimeSigEvent};

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

        // Drop skipped tracks and assign fallback names / final track indices by position.
        let mut tracks: Vec<TrackData> = Vec::with_capacity(total_tracks);
        let mut per_track_notes: Vec<Vec<NoteEvent>> = Vec::with_capacity(total_tracks);
        for mut td in parsed.into_iter().flatten() {
            if td.name.is_empty() {
                td.name = format!("Track {}", tracks.len() + 1);
            }
            // 用最终 model 位置修正 lane.track，避免与后续插入 conductor 后的编号不一致
            for lane in td.automation_lanes.iter_mut() {
                lane.track = tracks.len() as u16;
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

        // Purge mimalloc free pages: after load_track_notes drops the
        // per-track temporary Vecs, many pages are idle in mimalloc's
        // free list.  This hint tells it to munmap them back to the OS,
        // reducing RSS without affecting future allocations.
        yinhe_memtrace::purge_free_pages();

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
/// CC6 (Data Entry MSB) and CC38 (Data Entry LSB). NRPNs use CC99 (MSB) +
/// CC98 (LSB) instead.
///
/// When CC6/38 arrives with both msb/lsb selected, emit an RPN or NRPN
/// AutomationEvent. The selector CCs (101/100/99/98) are NOT stored as
/// plain CC — they are consumed by the RPN/NRPN state machine.
#[derive(Default, Clone, Copy)]
struct RpnState {
    msb: Option<u8>,
    lsb: Option<u8>,
}

/// Per-channel pending Bank Select state.
///
/// CC 0 (Bank MSB) and CC 32 (Bank LSB) are buffered here and folded into
/// the next ProgramChange on the same tick. If no PC follows, they are
/// flushed to automation_lanes at the end of the track.
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
    // NRPN state per channel (channel 0..16).
    let mut nrpn_state: [RpnState; 16] = [RpnState::default(); 16];

    // Pending Bank Select per channel (CC 0 / CC 32).
    let mut pending_bank: [PendingBank; 16] = [PendingBank::default(); 16];

    // Accumulate automation events per target during parsing.
    // Key = (target_variant, controller_or_parameter).
    // We use a Vec<(AutomationTarget, AutomationEvent)> and sort at the end.
    let mut auto_events: Vec<(AutomationTarget, AutomationEvent)> = Vec::new();

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
                                // RPN MSB selector
                                rpn_state[ch_idx].msb = Some(val);
                            }
                            100 => {
                                // RPN LSB selector
                                rpn_state[ch_idx].lsb = Some(val);
                            }
                            99 => {
                                // NRPN MSB selector
                                nrpn_state[ch_idx].msb = Some(val);
                            }
                            98 => {
                                // NRPN LSB selector
                                nrpn_state[ch_idx].lsb = Some(val);
                            }
                            0 => {
                                // Bank MSB: buffer for potential PC folding
                                pending_bank[ch_idx].msb = Some((val, current_tick));
                            }
                            32 => {
                                // Bank LSB: buffer for potential PC folding
                                pending_bank[ch_idx].lsb = Some((val, current_tick));
                            }
                            6 => handle_cc6(val, ch_idx, current_tick, &rpn_state, &nrpn_state, &mut auto_events),
                            38 => handle_cc38(val, ch_idx, current_tick, &rpn_state, &nrpn_state, &mut auto_events),
                            _ => {
                                // All other CC → AutomationTarget::CC
                                auto_events.push((
                                    AutomationTarget::CC { controller: cc },
                                    AutomationEvent {
                                        tick: current_tick,
                                        value: val as u16,
                                        ..Default::default()
                                    },
                                ));
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
                        auto_events.push((
                            AutomationTarget::PitchBend,
                            AutomationEvent {
                                tick: current_tick,
                                value: bend.0.as_int(), // raw 0–16383
                                ..Default::default()
                            },
                        ));
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
    for (_ch_idx, bank) in pending_bank.iter().enumerate() {
        if let Some((val, tick)) = bank.msb {
            auto_events.push((
                AutomationTarget::CC { controller: 0 },
                AutomationEvent { tick, value: val as u16, ..Default::default() },
            ));
        }
        if let Some((val, tick)) = bank.lsb {
            auto_events.push((
                AutomationTarget::CC { controller: 32 },
                AutomationEvent { tick, value: val as u16, ..Default::default() },
            ));
        }
    }

    // Assign dup_index for any (key, start_tick) collisions.
    assign_dup_indices(&mut td.notes);

    td.program_change.sort_by_key(|e| e.tick);

    // Build automation_lanes from accumulated events.
    // Sort by (target, tick) then group into lanes.
    auto_events.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| a.1.tick.cmp(&b.1.tick))
    });

    td.automation_lanes = group_automation_events(auto_events, td.port, td.channel, track_idx);

    Ok(Some(td))
}

/// Group sorted (target, event) pairs into AutomationLane vecs.
fn group_automation_events(
    events: Vec<(AutomationTarget, AutomationEvent)>,
    _port: u8,
    _channel: u8,
    track_idx: usize,
) -> Vec<AutomationLane> {
    if events.is_empty() {
        return Vec::new();
    }
    let mut lanes: Vec<AutomationLane> = Vec::new();
    let mut i = 0;
    while i < events.len() {
        let target = events[i].0.clone();
        let start = i;
        while i < events.len() && events[i].0 == target {
            i += 1;
        }
        let lane_events: Vec<AutomationEvent> = events[start..i]
            .iter()
            .map(|(_, e)| e.clone())
            .collect();
        lanes.push(AutomationLane {
            target,
            track: track_idx as u16,
            events: lane_events,
        });
    }
    lanes
}

/// Handle CC 6 (Data Entry MSB) with RPN/NRPN state machine.
fn handle_cc6(
    val: u8,
    ch_idx: usize,
    current_tick: u32,
    rpn_state: &[RpnState; 16],
    nrpn_state: &[RpnState; 16],
    auto_events: &mut Vec<(AutomationTarget, AutomationEvent)>,
) {
    let rpn = rpn_state[ch_idx];
    let nrpn = nrpn_state[ch_idx];
    if let (Some(msb), Some(lsb)) = (rpn.msb, rpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        let target = AutomationTarget::Rpn { parameter };
        let value = if target.is_14bit() {
            (val as u16) << 7
        } else {
            val as u16
        };
        auto_events.push((
            target,
            AutomationEvent { tick: current_tick, value, ..Default::default() },
        ));
    } else if let (Some(msb), Some(lsb)) = (nrpn.msb, nrpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        auto_events.push((
            AutomationTarget::Nrpn { parameter },
            AutomationEvent {
                tick: current_tick,
                value: (val as u16) << 7,
                ..Default::default()
            },
        ));
    } else {
        auto_events.push((
            AutomationTarget::CC { controller: 6 },
            AutomationEvent {
                tick: current_tick,
                value: val as u16,
                ..Default::default()
            },
        ));
    }
}

/// Handle CC 38 (Data Entry LSB) with RPN/NRPN state machine.
fn handle_cc38(
    val: u8,
    ch_idx: usize,
    current_tick: u32,
    rpn_state: &[RpnState; 16],
    nrpn_state: &[RpnState; 16],
    auto_events: &mut Vec<(AutomationTarget, AutomationEvent)>,
) {
    let rpn = rpn_state[ch_idx];
    let nrpn = nrpn_state[ch_idx];
    if let (Some(msb), Some(lsb)) = (rpn.msb, rpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        let target = AutomationTarget::Rpn { parameter };
        if target.is_14bit() {
            if let Some((_, last)) = auto_events
                .iter_mut()
                .rfind(|(t, e)| *t == target && e.tick == current_tick)
            {
                last.value = (last.value & 0xFF80) | (val as u16);
            } else {
                auto_events.push((
                    target,
                    AutomationEvent { tick: current_tick, value: val as u16, ..Default::default() },
                ));
            }
        } else {
            auto_events.push((
                AutomationTarget::CC { controller: 38 },
                AutomationEvent { tick: current_tick, value: val as u16, ..Default::default() },
            ));
        }
    } else if let (Some(msb), Some(lsb)) = (nrpn.msb, nrpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        let target = AutomationTarget::Nrpn { parameter };
        if let Some((_, last)) = auto_events
            .iter_mut()
            .rfind(|(t, e)| *t == target && e.tick == current_tick)
        {
            last.value = (last.value & 0xFF80) | (val as u16);
        } else {
            auto_events.push((
                target,
                AutomationEvent {
                    tick: current_tick,
                    value: val as u16,
                    ..Default::default()
                },
            ));
        }
    } else {
        auto_events.push((
            AutomationTarget::CC { controller: 38 },
            AutomationEvent {
                tick: current_tick,
                value: val as u16,
                ..Default::default()
            },
        ));
    }
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
/// order. We use a std::collections::BTreeMap<(key, start_tick), u8> counter.
fn assign_dup_indices(notes: &mut [NoteEvent]) {
    use std::collections::BTreeMap;
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
