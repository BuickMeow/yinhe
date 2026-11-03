//! Tick-domain transforms shared by PPQ rescale and MIDI import.
//!
//! A single implementation of "every tick field in the model" keeps
//! `rescale_ppq` and MIDI import from drifting apart: the old rescale path
//! missed key signatures / markers / lyrics / chords, so those events stayed
//! at their old tick after a PPQ change.

use std::sync::Arc;

use yinhe_types::{AutomationLane, Note};

use crate::model::YinModel;

fn rescale_tick(tick: u32, new_ppq: u32, old_ppq: u32, offset: u32) -> u32 {
    let num = new_ppq.max(1) as u128;
    let den = old_ppq.max(1) as u128;
    let scaled = (tick as u128 * num + den / 2) / den;
    (scaled + offset as u128).min(u32::MAX as u128) as u32
}

fn rescale_lane(lane: &mut AutomationLane, new_ppq: u32, old_ppq: u32, offset: u32) {
    for ev in &mut lane.events {
        ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
    }
    lane.events.sort_by_key(|e| e.tick);
}

impl YinModel {
    /// Rescale every tick in the model from `self.meta.ppq` to `new_ppq` and
    /// add `offset` afterwards. Does **not** rebuild derived stats; callers do
    /// that once, after the whole transform (import merges before rebuilding).
    ///
    /// `offset` lets MIDI import append a source model at the end of the
    /// destination project without a separate event-walking pass.
    pub fn rescale_and_offset_ticks(&mut self, new_ppq: u32, offset: u32) {
        let old_ppq = self.meta.ppq.max(1);
        let new_ppq = new_ppq.max(1);
        if old_ppq == new_ppq && offset == 0 {
            return;
        }

        // Notes: scaling is monotonic and `offset` is constant, so bucket order
        // is preserved. `rebuild()` (or the per-key import insertion) re-sorts
        // only when needed.
        use rayon::prelude::*;
        self.notes.par_iter_mut().for_each(|bucket| {
            let bucket = Arc::make_mut(bucket);
            for note in bucket.iter_mut() {
                note.start_tick = rescale_tick(note.start_tick, new_ppq, old_ppq, offset);
                note.end_tick = rescale_tick(note.end_tick, new_ppq, old_ppq, offset);
                if note.end_tick <= note.start_tick {
                    note.end_tick = note.start_tick.saturating_add(1);
                }
            }
        });

        // Conductor: tempo / time sig / key sig / markers / lyrics / chords.
        {
            let conductor = Arc::make_mut(&mut self.conductor);
            rescale_lane(&mut conductor.tempo, new_ppq, old_ppq, offset);
            for ev in &mut conductor.time_sig {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            conductor.time_sig.sort_by_key(|e| e.tick);
            for ev in &mut conductor.key_sig {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            conductor.key_sig.sort_by_key(|e| e.tick);
            for ev in &mut conductor.markers {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            conductor.markers.sort_by_key(|e| e.tick);
            for ev in &mut conductor.lyrics {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            conductor.lyrics.sort_by_key(|e| e.tick);
            for ev in &mut conductor.chord {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            conductor.chord.sort_by_key(|e| e.tick);
        }

        // Per-track lanes and event lists.
        for track in &mut self.tracks {
            let track = Arc::make_mut(track);
            for lane in &mut track.automation_lanes {
                rescale_lane(lane, new_ppq, old_ppq, offset);
            }
            for ev in &mut track.program_change {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            track.program_change.sort_by_key(|e| e.tick);
            for ev in &mut track.lyrics {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            track.lyrics.sort_by_key(|e| e.tick);
            for ev in &mut track.chord {
                ev.tick = rescale_tick(ev.tick, new_ppq, old_ppq, offset);
            }
            track.chord.sort_by_key(|e| e.tick);
            // Parsed-but-not-yet-loaded notes (not used at runtime, but keep the
            // transform total so a caller can't leave half the model behind).
            for note in &mut track.notes {
                note.start_tick = rescale_tick(note.start_tick, new_ppq, old_ppq, offset);
                note.end_tick = rescale_tick(note.end_tick, new_ppq, old_ppq, offset);
                if note.end_tick <= note.start_tick {
                    note.end_tick = note.start_tick.saturating_add(1);
                }
            }
        }

        self.meta.ppq = new_ppq;
    }

    /// Content end: last note end or last event tick on any conductor/track
    /// list. Used by MIDI import to append after everything the user can see.
    pub fn content_end_tick(&self) -> u32 {
        let mut end = self.tick_length.min(u32::MAX as u64) as u32;
        let mut bump = |t: u32| end = end.max(t);
        for ev in &self.conductor.tempo.events {
            bump(ev.tick);
        }
        for ev in &self.conductor.time_sig {
            bump(ev.tick);
        }
        for ev in &self.conductor.key_sig {
            bump(ev.tick);
        }
        for ev in &self.conductor.markers {
            bump(ev.tick);
        }
        for ev in &self.conductor.lyrics {
            bump(ev.tick);
        }
        for ev in &self.conductor.chord {
            bump(ev.tick);
        }
        for track in &self.tracks {
            for lane in &track.automation_lanes {
                if let Some(ev) = lane.events.last() {
                    bump(ev.tick);
                }
            }
            if let Some(ev) = track.program_change.last() {
                bump(ev.tick);
            }
            if let Some(ev) = track.lyrics.last() {
                bump(ev.tick);
            }
            if let Some(ev) = track.chord.last() {
                bump(ev.tick);
            }
            for note in &track.notes {
                bump(note.end_tick);
            }
        }
        end
    }
}

/// Remove every note whose id falls in `[start, end)`.
///
/// Used by the import undo path: imported ids are allocated contiguously, so
/// no per-note copy is needed in the undo stack.
pub fn remove_notes_by_id_range(model: &mut YinModel, start: u32, end: u32, keys: &[u8]) {
    if start >= end {
        return;
    }
    for &key in keys {
        let bucket = Arc::make_mut(&mut model.notes[key as usize]);
        let before = bucket.len();
        bucket.retain(|n: &Note| n.id < start || n.id >= end);
        if bucket.len() != before {
            model.mark_dirty(key);
        }
    }
}
