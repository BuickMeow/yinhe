use yinhe_types::AutomationPanelView;
use rayon::prelude::*;
use yinhe_types::{key_notes_in_range, NoteSource};

use crate::vertex::VelocityBarInstance;

/// Stack red zone threshold for stacker.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size for stacker.
const STACK_SIZE: usize = 1024 * 1024;

/// Build velocity bar instances from NoteSource (automation panel, velocity mode).
///
/// Outputs `VelocityBarInstance` (16B) — semantic data only (tick, length, track,
/// velocity). All pixel positions and colors are computed on the GPU in
/// `vs_main_velocity` from uniforms + track_colors storage buffer.
///
/// Unified border-based mode (fill + border), same as note rendering.
/// No occlusion sorting: border ensures visibility for overlapping bars.
/// A simple (tick, track) sort is applied for deterministic frame-to-frame output
/// (rayon parallel collection order is otherwise non-deterministic).
///
/// Uses `stacker::maybe_grow` to prevent stack overflow when processing
/// many notes at very low zoom levels.
pub fn build_velocity_bars(
    out: &mut Vec<VelocityBarInstance>,
    w: f32,
    midi: &dyn NoteSource,
    view: &AutomationPanelView,
    track_visible: &[bool],
) {
    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;

    let mut bars: Vec<VelocityBarInstance> = (0u8..128)
        .into_par_iter()
        .flat_map_iter(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let mut local: Vec<VelocityBarInstance> = Vec::new();
                let notes = key_notes_in_range(midi.key_notes(key), pad_start, pad_end);
                for note in notes {
                    if note.start_tick as f64 > pad_end as f64 {
                        break;
                    }
                    if (note.end_tick as f64) < pad_start as f64 {
                        continue;
                    }
                    let trk_idx = note.track as usize;
                    if !track_visible.get(trk_idx).copied().unwrap_or(true) {
                        continue;
                    }
                    local.push(VelocityBarInstance {
                        tick: note.start_tick,
                        length: note.end_tick - note.start_tick,
                        packed: VelocityBarInstance::pack(note.track, note.velocity),
                        reserved: 0,
                    });
                }
                local
            })
        })
        .collect();

    // Deterministic order: sort by (tick, packed) so output is stable across frames.
    // rayon parallel collection order is non-deterministic; this sort fixes it.
    // Within same (tick, track) the order is unspecified — overlapping bars at
    // the same tick+track are rare and border mode ensures visibility regardless.
    bars.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.packed.cmp(&b.packed)));

    out.extend(bars);
}
