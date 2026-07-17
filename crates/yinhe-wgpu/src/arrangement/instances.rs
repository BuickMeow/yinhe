use std::collections::HashSet;

use rayon::prelude::*;
use yinhe_types::{key_notes_in_range, NoteSource, TimeSigEvent};

use yinhe_types::ArrangementView;
use crate::grid;
use crate::vertex::{DrawInstance, NoteInstance};

/// Stack red zone threshold. When stack usage exceeds this, `stacker` will
/// allocate a new stack segment before calling the closure.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size to allocate when the red zone is exceeded.
const STACK_SIZE: usize = 1024 * 1024; // 1MB per segment

/// Build grid line instances (layer 0).
/// Dependencies: scroll_x, pixels_per_tick, time_sig
pub fn build_grid(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &ArrangementView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    scroll_x_pixel: f32,
    theme: &yinhe_theme::GpuTheme,
) {
    grid::build_timeline_grid(
        out,
        w,
        h,
        &view.base,
        tpb,
        default_num,
        default_den,
        time_sig_events,
        theme.ar_measure_line,
        theme.ar_beat_line,
        None,
        scroll_x_pixel,
    );
}

/// Build note rectangle instances with sub-pixel merging (layer 2).
/// Dependencies: track_visible, tick range (scroll_x), track range (scroll_y)
///
/// Output is 16B `NoteInstance` (semantic data only: ticks, key, track, vel).
/// All pixel positions and colors are computed in the GPU vertex shader:
///   - x/w: ticks → pixels via ppu + scroll_x (same as before)
///   - y/h: shader computes from lane_height + scroll_y + key + track
/// This means scroll_y changes do NOT invalidate the cache (same optimization
/// as PR notes).
///
/// GPU clips off-screen notes.
///
/// Uses `stacker::maybe_grow` to dynamically allocate new stack segments
/// when the current stack is close to overflowing.
pub fn build_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &ArrangementView,
    track_visible: &[bool],
    hidden_notes: &HashSet<(u16, u32, u8)>,
) {
    let ppu = view.base.pixels_per_tick;
    let num_tracks = track_visible.len();
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);

    let note_instances: Vec<Vec<NoteInstance>> = (0u8..128)
        .into_par_iter()
        .filter_map(|key| {
            // Wrap key processing in stacker to get fresh stack segments on demand.
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let notes = key_notes_in_range(midi.key_notes(key), tick_start as u32, tick_end as u32);
                if notes.is_empty() {
                    return None;
                }

                let mut local = Vec::new();

                let flush_merge =
                    |local: &mut Vec<NoteInstance>, ti: usize, start: u32, end: u32, vel: u8| {
                        let s = (start as f64).max(tick_start) as u32;
                        let e = (end as f64).min(tick_end).max(start as f64) as u32;
                        if s >= e {
                            return;
                        }
                        if ti < trk_first || ti >= trk_last {
                            return;
                        }
                        if !track_visible.get(ti).copied().unwrap_or(true) {
                            return;
                        }
                        // 16B NoteInstance: shader computes pixel_y from
                        // lane_height + scroll_y + key + track, and fetches color
                        // from track_colors storage buffer via track index.
                        // ti is usize; saturate to u16 (MAX_TRACKS=65536).
                        local.push(NoteInstance {
                            start_tick: s,
                            end_tick: e,
                            packed: NoteInstance::pack(key, ti.min(65535) as u16, vel),
                            reserved: 0,
                        });
                    };

                let merge_gap_ticks = (1.0 / ppu).ceil() as u32;

                let mut track_buckets: Vec<Vec<(u32, u32, u8)>> = vec![Vec::new(); num_tracks];
                for note in notes {
                    if note.start_tick as f64 > tick_end {
                        break;
                    }
                    if (note.end_tick as f64) < tick_start {
                        continue;
                    }
                    let ti = note.track as usize;
                    if ti < trk_first || ti >= trk_last {
                        continue;
                    }
                    if !track_visible.get(ti).copied().unwrap_or(true) {
                        continue;
                    }
                    if hidden_notes.contains(&(note.track, note.start_tick, key)) {
                        continue;
                    }
                    track_buckets[ti].push((note.start_tick, note.end_tick, note.velocity));
                }

                for (ti, notes_in_track) in track_buckets.iter().enumerate() {
                    if notes_in_track.is_empty() {
                        continue;
                    }
                    let mut merge_start = notes_in_track[0].0;
                    let mut merge_end = notes_in_track[0].1;
                    let mut merge_vel = notes_in_track[0].2;

                    for &(s, e, v) in &notes_in_track[1..] {
                        if s <= merge_end + merge_gap_ticks {
                            merge_end = merge_end.max(e);
                            merge_vel = merge_vel.max(v);
                        } else {
                            flush_merge(&mut local, ti, merge_start, merge_end, merge_vel);
                            merge_start = s;
                            merge_end = e;
                            merge_vel = v;
                        }
                    }
                    flush_merge(&mut local, ti, merge_start, merge_end, merge_vel);
                }

                if local.is_empty() { None } else { Some(local) }
            })
        })
        .collect();

    out.extend(note_instances.into_iter().flatten());
}

/// Build ghost note instances for move-drag preview (layer 3, no cache).
///
/// Each ghost is `(start_tick, end_tick, key, track)`. Positions are in
/// absolute tick/key space — the GPU shader converts to pixels.
pub fn build_ghost_notes(
    out: &mut Vec<NoteInstance>,
    ghost_notes: &[(f64, f64, u8, u16)],
) {
    for &(start_tick, end_tick, key, track) in ghost_notes {
        let s = start_tick.max(0.0) as u32;
        let e = end_tick.max(start_tick).max(0.0) as u32;
        if s >= e {
            continue;
        }
        out.push(NoteInstance {
            start_tick: s,
            end_tick: e,
            packed: NoteInstance::pack(key, track.min(65535) as u16, 0),
            reserved: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_midi;

    #[test]
    fn test_no_duplicate_grid_lines_at_time_sig_boundary() {
        let _mock = make_midi(vec![(60, 0, 480, 0, 100)]);
        let tpb = 480;
        let mut view = ArrangementView::default();
        view.base.pixels_per_tick = 0.5;
        view.base.left_panel_width = 0.0;
        view.base.scroll_x = 0.0;
        view.base.dirty = true;

        let events = vec![
            TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            },
            TimeSigEvent {
                tick: 1920,
                numerator: 7,
                denominator: 2,
            },
        ];

        let mut grid_lines = Vec::new();
        build_grid(&mut grid_lines, 2000.0, 400.0, &view, tpb, 4, 2, &events, 0.0, &yinhe_theme::GpuTheme::default());

        let ticks: Vec<u32> = grid_lines.iter().map(|i| i.tag).collect();

        let mut sorted = ticks.clone();
        sorted.sort();
        let deduped = {
            let mut d = sorted.clone();
            d.dedup();
            d
        };
        assert_eq!(
            sorted.len(),
            deduped.len(),
            "Duplicate grid lines at same tick!\n  ticks: {:?}\n  sorted: {:?}",
            ticks,
            sorted,
        );

        let count_1920 = ticks.iter().filter(|&&t| t == 1920).count();
        assert_eq!(
            count_1920, 1,
            "Boundary tick 1920 must appear exactly once, got {}",
            count_1920
        );
    }
}
