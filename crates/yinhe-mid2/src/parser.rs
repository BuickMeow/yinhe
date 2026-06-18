//! Parse Standard MIDI File bytes directly into a `yinhe_core::YinModel`.
//!
//! Single-pass per track: NoteOn/NoteOff pairing, port/channel prefix
//! tracking, CC/PB/PC collection, and RPN state-machine decoding all
//! happen in one walk. Conductor events (tempo, time signature) are
//! collected across all tracks first.

use std::collections::BTreeMap;
use std::path::Path;

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
        let smf = midly::Smf::parse(data)?;

        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(t) => t.as_int() as u32,
            midly::Timing::Timecode(_, _) => TIMECODE_FALLBACK_TPB,
        };

        // Pass 1: collect conductor events (tempo + time-sig) across ALL tracks.
        let conductor = collect_conductor(&smf.tracks);

        // Pass 2: per-track parse → TrackData.
        // Skip "conductor-only" tracks: those with no MIDI messages at all
        // (only meta events). These are typical of SMF format-1 files where
        // track 0 is a conductor track.
        let total_tracks = smf.tracks.len();
        let mut tracks: Vec<TrackData> = Vec::with_capacity(total_tracks);
        for (track_idx, raw_track) in smf.tracks.iter().enumerate() {
            progress(LoadProgress {
                current_track: track_idx + 1,
                total_tracks,
            });
            if track_is_conductor_only(raw_track) {
                continue;
            }
            let mut td = parse_track(raw_track, track_idx, encoding);
            // Set fallback name if MetaMessage::TrackName was missing.
            if td.name.is_empty() {
                td.name = format!("Track {}", tracks.len() + 1);
            }
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
        model.rebuild();

        Ok(model)
    })
}

// =========================================================
//  Conductor pass (across all tracks)
// =========================================================

fn collect_conductor(tracks: &[midly::Track]) -> ConductorData {
    let mut tempo: Vec<TempoEvent> = Vec::new();
    let mut time_sig: Vec<TimeSigEvent> = Vec::new();

    for track in tracks {
        let mut tick: u32 = 0;
        for ev in track {
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

    ConductorData { tempo, time_sig }
}

/// True if a track has no MIDI messages (only meta events).
///
/// Such tracks are conductor tracks in SMF format-1 files; their tempo and
/// time-signature events are already collected by `collect_conductor` and
/// they shouldn't surface as a `TrackData`.
fn track_is_conductor_only(track: &midly::Track) -> bool {
    !track
        .iter()
        .any(|ev| matches!(ev.kind, midly::TrackEventKind::Midi { .. }))
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

fn parse_track(track: &midly::Track, track_idx: usize, encoding: MidiImportEncoding) -> TrackData {
    let mut td = TrackData::new(0, 0);
    td.uuid = uuid::Uuid::new_v4().to_string();

    let mut current_tick: u32 = 0;
    let mut current_port: u8 = 0;
    let mut active_notes: Vec<ActiveNote> = Vec::new();
    let mut first_global_channel: Option<u8> = None;

    // RPN state per channel (channel 0..16).
    let mut rpn_state: [RpnState; 16] = [RpnState::default(); 16];

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

    for ev in track {
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
                        td.program_change.push(PcEvent {
                            tick: current_tick,
                            program: program.as_int(),
                            bank_msb: 0xFF,
                            bank_lsb: 0xFF,
                        });
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

    // Pin port/channel from first MIDI event seen, or default.
    let _ = track_idx; // (kept for future use)
    td.port = current_port;
    td.channel = first_global_channel
        .map(|gc| gc & 0x0F)
        .unwrap_or(0);

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

    td
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
