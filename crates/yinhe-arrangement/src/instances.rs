use rayon::prelude::*;
use yinhe_types::{NoteSource, TimeSigEvent};

use crate::view::ArrangementView;
use yinhe_wgpu::grid;
use yinhe_wgpu::vertex::{DrawInstance, pack_props, pack_rgba};

/// Build background + track lane instances (layer 0).
/// Dependencies: scroll_y, lane_height, track_visible
pub fn build_decor(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    lb_w: f32,
    lh: f32,
    scroll_y: f32,
    track_visible: &[bool],
) {
    let num_tracks = track_visible.len();

    out.push(DrawInstance {
        x: lb_w,
        y: 0.0,
        w: w - lb_w,
        h,
        rgba_packed: pack_rgba(
            grid::AR_BG_COLOR.0,
            grid::AR_BG_COLOR.1,
            grid::AR_BG_COLOR.2,
            1.0,
        ),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        tag: 0,
    });

    if num_tracks > 0 {
        let (trk_first, trk_last) = ArrangementView::visible_track_range_static(scroll_y, h, lh, num_tracks);
        for idx in trk_first..trk_last {
            if !track_visible.get(idx).copied().unwrap_or(true) {
                continue;
            }
            let y = ArrangementView::lane_y_static(idx, scroll_y, lh);
            let col = if idx % 2 == 0 {
                grid::AR_LANE_EVEN_COLOR
            } else {
                grid::AR_LANE_ODD_COLOR
            };
            out.push(DrawInstance {
                x: lb_w,
                y,
                w: w - lb_w,
                h: lh,
                rgba_packed: pack_rgba(col.0, col.1, col.2, 1.0),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }
    }
}

/// Build grid line instances (layer 1).
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
        grid::AR_MEASURE_LINE_COLOR,
        grid::AR_BEAT_LINE_COLOR,
        None,
        scroll_x_pixel,
    );
}

/// Build note rectangle instances with sub-pixel merging (layer 2).
/// Dependencies: scroll_y, lane_height, track_visible, track_colors
/// x/w store ticks (shader converts to pixels), y/h store pixel positions.
///
/// GPU clips off-screen notes.
pub fn build_notes(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &ArrangementView,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) {
    let lh = view.lane_height;
    let ppu = view.base.pixels_per_tick;
    let num_tracks = track_visible.len();
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);
    let y_offset = -view.base.scroll_y;
    let lh_per_key = lh / 128.0;
    let note_h = lh_per_key.max(1.0);

    let note_instances: Vec<Vec<DrawInstance>> = (0u8..128)
        .into_par_iter()
        .filter_map(|key| {
            let notes = midi.key_notes_in_range(key, tick_start as u32, tick_end as u32);
            if notes.is_empty() {
                return None;
            }

            let key_y_base = y_offset + lh - (key as f32 + 1.0) * lh_per_key;

            let mut local = Vec::new();

            let flush_merge =
                |local: &mut Vec<DrawInstance>, ti: usize, start: u32, end: u32, vel: u8| {
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
                    let note_y = key_y_base + ti as f32 * lh;
                    let color = track_colors.get(ti).copied().unwrap_or([0.5, 0.5, 0.5]);
                    local.push(DrawInstance {
                        x: s as f32,        // tick (shader converts to pixel)
                        y: note_y,           // pixel
                        w: e as f32,         // tick (shader converts to pixel)
                        h: note_h,           // pixel
                        rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                        props_packed: 0,     // no rounding/border for AR notes
                        velocity: vel as u32,
                        tag: 0,
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
        .collect();

    for mut local in note_instances {
        out.append(&mut local);
    }
}

/// Build the playhead cursor instance (a single vertical line).
pub fn build_arrangement_cursor(
    instances: &mut Vec<DrawInstance>,
    cursor_tick: Option<f64>,
    view: &ArrangementView,
    width: u32,
    height: u32,
) {
    let w = width as f32;
    let h = height as f32;
    let lb_w = view.base.left_panel_width;

    if let Some(ct) = cursor_tick {
        let cx = view.tick_to_x(ct);
        if cx >= lb_w && cx <= w {
            grid::push_grid_line(instances, cx, h, 2.0, grid::AR_PLAYHEAD_COLOR, 0);
        }
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
        build_grid(&mut grid_lines, 2000.0, 400.0, &view, tpb, 4, 2, &events, 0.0);

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
