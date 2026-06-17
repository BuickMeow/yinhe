//! Bridge: convert `yinhe_core::YinModel` to a legacy `yinhe_midi::MidiFile`.
//!
//! Used during the migration window so that consumers still calling
//! `doc.midi()` keep working without modification. Lazily computed via
//! `Document.midi_compat: OnceLock<Arc<MidiFile>>` and invalidated on
//! every model rebuild.
//!
//! This module is deleted on switchover Phase 4 along with the old crates.

use std::collections::HashMap;

use yinhe_core::YinModel;
use yinhe_midi::{MidiControlEvent, MidiFile};
use yinhe_types::{Note, TimeSigEvent as OldTsEvent};

/// Build a legacy `MidiFile` view of the new `YinModel`.
pub(crate) fn core_to_midi_file(model: &YinModel) -> MidiFile {
    let ppq = model.meta.ppq;
    let num_tracks = model.tracks.len();

    // ── Build per-key note lists with track tagging ───────────────
    let mut key_notes: [Vec<Note>; 128] = std::array::from_fn(|_| Vec::new());
    for (track_idx, track) in model.tracks.iter().enumerate() {
        for n in &track.notes {
            key_notes[n.key as usize].push(Note {
                start_tick: n.start_tick,
                end_tick: n.end_tick,
                velocity: n.velocity,
                track: track_idx as u16,
            });
        }
    }
    for v in key_notes.iter_mut() {
        v.sort_by_key(|n| n.start_tick);
    }

    let note_count: u64 = key_notes.iter().map(|v| v.len() as u64).sum();
    let tick_length = key_notes
        .iter()
        .flat_map(|v| v.iter().map(|n| n.end_tick as u64))
        .max()
        .unwrap_or(0);

    // ── Per-track ports / channels / names ───────────────────────
    let mut track_ports: Vec<u8> = Vec::with_capacity(num_tracks);
    let mut track_channel_prefixes: Vec<Option<u8>> = Vec::with_capacity(num_tracks);
    let mut track_channels: Vec<u8> = Vec::with_capacity(num_tracks);
    let mut track_names: Vec<String> = Vec::with_capacity(num_tracks);
    let mut raw_track_names: Vec<Vec<u8>> = Vec::with_capacity(num_tracks);
    for t in &model.tracks {
        track_ports.push(t.port);
        track_channel_prefixes.push(t.channel_prefix);
        // Old code uses `track_channels[i]` as the "global" channel
        // (port * 16 + channel), matching the audio engine's expectation.
        track_channels.push((t.port & 0x0F) << 4 | (t.channel & 0x0F));
        track_names.push(t.name.clone());
        raw_track_names.push(t.name.as_bytes().to_vec());
    }

    // ── Tempo segments (rebuild from conductor.tempo) ────────────
    const DEFAULT_MPQ: u64 = 500_000; // 120 BPM
    let mut tempo_segments: Vec<yinhe_midi::TempoSegment> = Vec::new();
    if model.conductor.tempo.is_empty() || model.conductor.tempo[0].tick != 0 {
        tempo_segments.push(yinhe_midi::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: DEFAULT_MPQ,
        });
    }
    for ev in &model.conductor.tempo {
        let mpq = if ev.bpm > 0.0 {
            (60_000_000.0 / ev.bpm).round() as u64
        } else {
            DEFAULT_MPQ
        };
        tempo_segments.push(yinhe_midi::TempoSegment {
            start_tick: ev.tick,
            start_time: 0.0,
            micros_per_quarter: mpq,
        });
    }
    tempo_segments.sort_by_key(|s| s.start_tick);
    tempo_segments.dedup_by_key(|s| s.start_tick);
    yinhe_midi::recompute_tempo_start_times(&mut tempo_segments, ppq);

    // ── Time signature events ────────────────────────────────────
    let mut time_sig_events: Vec<OldTsEvent> = model
        .conductor
        .time_sig
        .iter()
        .map(|ts| OldTsEvent {
            tick: ts.tick,
            numerator: ts.numerator,
            denominator: ts.denominator,
        })
        .collect();
    time_sig_events.sort_by_key(|e| e.tick);
    let (time_sig_numerator, time_sig_denominator) = time_sig_events
        .first()
        .map(|e| (e.numerator, e.denominator))
        .unwrap_or((4, 2));

    // ── Flatten control events from each track ───────────────────
    let mut control_events: Vec<MidiControlEvent> = Vec::new();
    for (track_idx, track) in model.tracks.iter().enumerate() {
        let track_u16 = track_idx as u16;
        for (&controller, evs) in &track.cc {
            for e in evs {
                control_events.push(MidiControlEvent::ControlChange {
                    tick: e.tick,
                    controller,
                    value: e.value,
                    track: track_u16,
                });
            }
        }
        for e in &track.pitch_bend {
            control_events.push(MidiControlEvent::PitchBend {
                tick: e.tick,
                value: e.value,
                track: track_u16,
            });
        }
        for e in &track.program_change {
            control_events.push(MidiControlEvent::ProgramChange {
                tick: e.tick,
                program: e.program,
                track: track_u16,
            });
        }
        // RPN expanded back to CC101 + CC100 + CC6 (+ CC38 if LSB != 0)
        for (&rpn_key, evs) in &track.rpn {
            let msb = ((rpn_key >> 8) & 0x7F) as u8;
            let lsb = (rpn_key & 0x7F) as u8;
            for e in evs {
                let data_msb = ((e.value >> 7) & 0x7F) as u8;
                let data_lsb = (e.value & 0x7F) as u8;
                control_events.push(MidiControlEvent::ControlChange {
                    tick: e.tick,
                    controller: 101,
                    value: msb,
                    track: track_u16,
                });
                control_events.push(MidiControlEvent::ControlChange {
                    tick: e.tick,
                    controller: 100,
                    value: lsb,
                    track: track_u16,
                });
                control_events.push(MidiControlEvent::ControlChange {
                    tick: e.tick,
                    controller: 6,
                    value: data_msb,
                    track: track_u16,
                });
                if data_lsb != 0 {
                    control_events.push(MidiControlEvent::ControlChange {
                        tick: e.tick,
                        controller: 38,
                        value: data_lsb,
                        track: track_u16,
                    });
                }
            }
        }
    }
    control_events.sort_by_key(|e| match e {
        MidiControlEvent::ControlChange { tick, .. }
        | MidiControlEvent::ProgramChange { tick, .. }
        | MidiControlEvent::PitchBend { tick, .. } => *tick,
    });

    let _ = HashMap::<(), ()>::new(); // silence unused import

    let scan_index = yinhe_types::NoteScanIndex::build(&key_notes, tick_length);
    const BUCKET_SIZE: u32 = 65536;
    let tick_buckets = yinhe_types::TickBuckets::build(&key_notes, tick_length, BUCKET_SIZE);
    let automation_lanes =
        yinhe_midi::build_automation_lanes(&control_events, &key_notes, &track_channels);

    MidiFile {
        key_notes,
        duration: 0.0,
        ticks_per_beat: ppq,
        tempo_segments,
        note_count,
        tick_length,
        time_sig_numerator,
        time_sig_denominator,
        track_ports,
        track_channel_prefixes,
        track_channels,
        track_names,
        raw_track_names,
        time_sig_events,
        control_events,
        scan_index: Some(scan_index),
        tick_buckets: Some(tick_buckets),
        automation_lanes,
    }
}

// ─────────────────────────────────────────────────────────────────
//  Reverse bridge: yinhe_core::YinModel → legacy yinhe_model::YinModel.
//  Used by callers that still go through yinhe_model::convert::to_archive
//  for saving .yin files. Phase 1b will switch to yinhe_yin::save_yin
//  directly. Deleted in Phase 4.
// ─────────────────────────────────────────────────────────────────

pub fn core_to_old_model(model: &yinhe_core::YinModel) -> yinhe_model::YinModel {
    let conductor = yinhe_model::ConductorData {
        tempo: model
            .conductor
            .tempo
            .iter()
            .map(|t| yinhe_model::TempoEvent { tick: t.tick, bpm: t.bpm })
            .collect(),
        time_sig: model
            .conductor
            .time_sig
            .iter()
            .map(|t| yinhe_model::TimeSigEvent {
                tick: t.tick,
                numerator: t.numerator,
                denominator: t.denominator,
            })
            .collect(),
    };
    let tracks: Vec<yinhe_model::TrackData> = model
        .tracks
        .iter()
        .map(|t| {
            let notes: Vec<yinhe_model::NoteEvent> = t
                .notes
                .iter()
                .map(|n| yinhe_model::NoteEvent {
                    tick: n.start_tick,
                    duration: n.end_tick.saturating_sub(n.start_tick),
                    key: n.key,
                    velocity: n.velocity,
                })
                .collect();
            let mut cc: std::collections::BTreeMap<u8, Vec<yinhe_model::CcEvent>> =
                std::collections::BTreeMap::new();
            for (&controller, evs) in &t.cc {
                cc.insert(
                    controller,
                    evs.iter()
                        .map(|e| yinhe_model::CcEvent { tick: e.tick, value: e.value })
                        .collect(),
                );
            }
            let pitch_bend: Vec<yinhe_model::PitchBendEvent> = t
                .pitch_bend
                .iter()
                .map(|e| yinhe_model::PitchBendEvent { tick: e.tick, value: e.value })
                .collect();
            let program_change: Vec<yinhe_model::PcEvent> = t
                .program_change
                .iter()
                .map(|e| yinhe_model::PcEvent {
                    tick: e.tick,
                    program: e.program,
                    bank_msb: e.bank_msb,
                    bank_lsb: e.bank_lsb,
                })
                .collect();
            let mut rpn: std::collections::BTreeMap<u8, Vec<yinhe_model::RpnEvent>> =
                std::collections::BTreeMap::new();
            for (&rpn_key, evs) in &t.rpn {
                let msb = ((rpn_key >> 8) & 0xFF) as u8;
                rpn.insert(
                    msb,
                    evs.iter()
                        .map(|e| yinhe_model::RpnEvent { tick: e.tick, value: e.value })
                        .collect(),
                );
            }
            yinhe_model::TrackData {
                uuid: t.uuid.clone(),
                name: t.name.clone(),
                port: t.port,
                channel: t.channel,
                notes,
                cc,
                pitch_bend,
                program_change,
                rpn,
            }
        })
        .collect();

    let meta = yinhe_model::ProjectMeta {
        name: model.meta.name.clone(),
        artist: model.meta.artist.clone(),
        description: model.meta.description.clone(),
        ppq: model.meta.ppq,
        compression_level: model.meta.compression_level,
    };

    let mut old = yinhe_model::YinModel {
        conductor,
        tracks,
        meta,
        key_index: yinhe_model::KeyIndex::default(),
        key_notes_cache: (0..128).map(|_| Vec::new()).collect(),
        note_count: 0,
        tick_length: 0,
    };
    old.rebuild();
    old
}
