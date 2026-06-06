use rayon::prelude::*;
use yinhe_types::{NoteSource, TimeSigEvent, seek_first_note};

use crate::view::ArrangementView;
use yinhe_wgpu::grid;
use yinhe_wgpu::vertex::{NoteInstance, pack_props, pack_rgba};

/// Build grid line instances into `out`, respecting time signature changes.
pub fn build_arrangement_grid(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    view: &ArrangementView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
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
    );
}

/// Build static arrangement instances (background, track lanes, grid, note
/// rectangles).  Does NOT include the playhead cursor — call
/// `build_arrangement_cursor` separately so the cursor can be updated every
/// frame without rebuilding expensive note geometry.
pub fn build_arrangement_static(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &ArrangementView,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) {
    let w = width as f32;
    let h = height as f32;
    let lh = view.lane_height;
    let lb_w = view.base.left_panel_width;
    let ppu = view.base.pixels_per_tick;
    let num_tracks = track_visible.len();

    // 1. Background quad
    instances.push(NoteInstance {
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

    // 2. Track lane backgrounds (alternating colors)
    if num_tracks > 0 {
        let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);
        for idx in trk_first..trk_last {
            if !track_visible.get(idx).copied().unwrap_or(true) {
                continue;
            }
            let y = view.lane_y(idx);
            let col = if idx % 2 == 0 {
                grid::AR_LANE_EVEN_COLOR
            } else {
                grid::AR_LANE_ODD_COLOR
            };
            instances.push(NoteInstance {
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

    // 3. Grid lines + 4. Note rectangles
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (tick_start, tick_end) = view.visible_tick_range(w);

        // Grid lines
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        build_arrangement_grid(instances, w, h, view, tpb, def_num, def_den, sig_events);

        // Note rectangles — merge consecutive same-track same-key notes into longer rects.
        let tick_pad = (w - lb_w) / ppu;
        let pad_start = (tick_start - tick_pad as f64).max(0.0);
        let pad_end = tick_end + tick_pad as f64;
        let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);

        let x_offset = lb_w - view.base.scroll_x;
        let y_offset = -view.base.scroll_y;
        let lh_per_key = lh / 128.0;
        let note_h = lh_per_key.max(1.0);

        let note_instances: Vec<Vec<NoteInstance>> = (0u8..128)
            .into_par_iter()
            .filter_map(|key| {
                let notes = midi.key_notes(key);
                if notes.is_empty() {
                    return None;
                }
                let start_idx = seek_first_note(key, midi, pad_start as u32);
                if start_idx >= notes.len() {
                    return None;
                }
                if notes.first().is_none_or(|n| n.start_tick as f64 > pad_end) {
                    return None;
                }

                let key_y_base = y_offset + lh - (key as f32 + 1.0) * lh_per_key;

                let mut local = Vec::new();

                let flush_merge =
                    |local: &mut Vec<NoteInstance>, ti: usize, start: u32, end: u32, vel: u8| {
                        let s = (start as f64).max(pad_start) as u32;
                        let e = (end as f64).min(pad_end).max(start as f64) as u32;
                        if s >= e {
                            return;
                        }
                        if ti < trk_first || ti >= trk_last {
                            return;
                        }
                        if !track_visible.get(ti).copied().unwrap_or(true) {
                            return;
                        }
                        let nx = x_offset + s as f32 * ppu;
                        let nw = ((e - s) as f32 * ppu).max(2.0);
                        let note_y = key_y_base + ti as f32 * lh;
                        let color = track_colors.get(ti).copied().unwrap_or([0.5, 0.5, 0.5]);
                        local.push(NoteInstance {
                            x: nx,
                            y: note_y,
                            w: nw,
                            h: note_h,
                            rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                            props_packed: pack_props(0.0, 0.0),
                            velocity: vel as u32,
                            tag: 0,
                        });
                    };

                // Sub-pixel merge: gaps < 1 pixel are invisible.
                let merge_gap_ticks = (1.0 / ppu).ceil() as u32;

                // Bucket notes by track using Vec (avoids per-frame HashMap allocation).
                let mut track_buckets: Vec<Vec<(u32, u32, u8)>> = vec![Vec::new(); num_tracks];
                for note in &notes[start_idx..] {
                    if note.start_tick as f64 > pad_end {
                        break;
                    }
                    if (note.end_tick as f64) < pad_start {
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
            instances.append(&mut local);
        }
    }
}

/// Build the playhead cursor instance (a single vertical line).
/// Call this every frame — it is cheap (O(1)) and independent of the static
/// note geometry built by `build_arrangement_static`.
pub fn build_arrangement_cursor(
    instances: &mut Vec<NoteInstance>,
    cursor_tick: Option<f64>,
    view: &ArrangementView,
    width: u32,
    height: u32,
) {
    let w = width as f32;
    let h = height as f32;
    let lb_w = view.base.left_panel_width;

    // 5. Playhead
    if let Some(ct) = cursor_tick {
        let cx = view.tick_to_x(ct);
        if cx >= lb_w && cx <= w {
            grid::push_grid_line(instances, cx, h, 2.0, grid::AR_PLAYHEAD_COLOR, 0);
        }
    }
}

/// Build all instances for the arrangement view frame (convenience wrapper
/// around `build_arrangement_static` + `build_arrangement_cursor`).
///
/// Prefer calling the two functions separately with `prepare_with_static_cache`
/// so the static geometry is cached across frames during playback.
pub fn build_arrangement_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &ArrangementView,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: Option<f64>,
) {
    build_arrangement_static(
        instances,
        width,
        height,
        midi,
        view,
        track_visible,
        track_colors,
    );
    build_arrangement_cursor(instances, cursor_tick, view, width, height);
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::Note;

    /// Mock MIDI data for testing.
    struct MockMidi {
        notes: [Vec<Note>; 128],
        tpb: u32,
        tick_len: u64,
    }

    impl NoteSource for MockMidi {
        fn key_notes(&self, key: u8) -> &[Note] {
            &self.notes[key as usize]
        }
        fn duration(&self) -> f64 {
            10.0
        }
        fn ticks_per_beat(&self) -> Option<u32> {
            Some(self.tpb)
        }
        fn tick_length(&self) -> Option<u64> {
            Some(self.tick_len)
        }
    }

    fn make_midi(notes: Vec<(u8, u32, u32, u16, u8)>) -> MockMidi {
        let mut key_notes: [Vec<Note>; 128] = core::array::from_fn(|_| Vec::new());
        let mut max_tick: u64 = 0;
        for (key, start_tick, end_tick, track, vel) in notes {
            let n = Note {
                start_tick,
                end_tick,
                key,
                velocity: vel,
                channel: 0,
                track,
            };
            if (end_tick as u64) > max_tick {
                max_tick = end_tick as u64;
            }
            key_notes[key as usize].push(n);
        }
        MockMidi {
            notes: key_notes,
            tpb: 480,
            tick_len: max_tick,
        }
    }

    #[test]
    fn test_basic_note_instances() {
        let mock = make_midi(vec![
            (60, 0, 480, 0, 100),
            (60, 480, 960, 0, 100),
            (64, 240, 720, 1, 80),
        ]);

        let view = ArrangementView::default();
        let track_visible = vec![true; 2];
        let track_colors = [[0.3, 0.6, 0.9], [0.9, 0.3, 0.3]];
        let mut instances = Vec::new();

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            1200,
            400,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "build took too long: {:?}",
            elapsed
        );
        assert!(!instances.is_empty(), "should have generated instances");

        let note_count = instances.iter().filter(|i| i.velocity > 0).count();
        assert_eq!(note_count, 2, "should have 2 note instances after merge");

        for inst in &instances {
            if inst.velocity > 0 {
                assert!(
                    inst.x >= view.base.left_panel_width,
                    "note x should be >= label_width"
                );
                assert!(inst.w > 0.0, "note width should be positive");
            }
        }
    }

    #[test]
    fn test_all_keys_performance() {
        let mut notes = Vec::with_capacity(128);
        for key in 0..128u8 {
            notes.push((key, key as u32 * 10, key as u32 * 10 + 120, 0, 90));
        }
        let mock = make_midi(notes);

        let view = ArrangementView::default();
        let track_visible = vec![true; 1];
        let track_colors = [[0.3, 0.6, 0.9]];
        let mut instances = Vec::new();

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            1200,
            400,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "128-key build took: {:?}",
            elapsed
        );
        assert!(instances.len() > 128, "should have many instances");
    }

    #[test]
    fn test_no_instance_cap() {
        let mut notes = Vec::new();
        let num_tick_positions = 1200u32;
        for tick_offset in 0..num_tick_positions {
            for key in 0..128u8 {
                for track in 0..16u16 {
                    let tick = 100 + tick_offset * 3;
                    notes.push((key, tick, tick + 1, track, 100));
                }
            }
        }
        let mock = make_midi(notes);

        let view = ArrangementView {
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: 0.08,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 0.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            lane_height: 40.0,
        };
        let track_visible = vec![true; 16];
        let track_colors = [[0.5f32; 3]; 16];
        let mut instances = Vec::new();

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            2000,
            800,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 3000,
            "large build took: {:?}",
            elapsed
        );

        let note_count = instances.iter().filter(|i| i.velocity > 0).count();
        assert!(note_count > 0, "should have generated note instances");
        assert!(
            note_count > 2000,
            "should have many merged instances: got {}",
            note_count
        );
    }

    #[test]
    fn test_grid_lines_dont_hang() {
        let mock = make_midi(vec![(60, 0, 480, 0, 100)]);
        let mut view = ArrangementView::default();
        view.base.pixels_per_tick = 10.0;
        let track_visible = vec![true; 1];
        let track_colors = [[0.3, 0.6, 0.9]];
        let mut instances = Vec::new();

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            2000,
            400,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 200,
            "extreme zoom build took: {:?}",
            elapsed
        );
    }

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
        build_arrangement_grid(&mut grid_lines, 2000.0, 400.0, &view, tpb, 4, 2, &events);

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
