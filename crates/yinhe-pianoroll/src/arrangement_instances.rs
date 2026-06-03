use rayon::prelude::*;
use yinhe_types::{NoteSource, TimeSigEvent, seek_first_note};

use crate::arrangement_view::ArrangementView;
use crate::grid::{self, push_grid_line};
use crate::vertex::{NoteInstance, pack_props, pack_rgba};

const NOTE_ROUNDING: f32 = 0.2;

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
    let ppu = view.pixels_per_tick;
    if ppu <= 0.01 {
        return;
    }

    let (tick_start, tick_end) = view.visible_tick_range(w);
    let sub_beat_div = 4u32;
    let ticks_per_sub = (tpb / sub_beat_div).max(1);
    let lb_w = view.label_width;
    let x_origin = lb_w - view.scroll_x;

    let segments = grid::build_time_sig_segments(time_sig_events, default_num, default_den);

    for i in 0..segments.len() {
        let (seg_start, num, den) = segments[i];
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
        let seg_start_f = seg_start as f64;
        if seg_start_f > tick_end {
            break;
        }

        let ticks_per_measure = grid::measure_ticks(tpb, num, den);
        let ticks_per_beat = ticks_per_measure / num as u32;

        let sub_f = ticks_per_sub as f64;
        let first_tick = seg_start_f.max(tick_start);
        let first = ((first_tick / sub_f).floor() as u32)
            .saturating_mul(ticks_per_sub)
            .max(seg_start);

        let mut tick = first;
        while (tick as f64) <= tick_end && tick < seg_end {
            let local = tick - seg_start;

            let x = x_origin + tick as f32 * ppu;
            if x >= lb_w && x <= w {
                let is_measure = local % ticks_per_measure == 0;
                let is_beat = if !is_measure {
                    let beat_local = local % ticks_per_measure;
                    beat_local % ticks_per_beat == 0 && beat_local > 0
                } else {
                    false
                };
                if is_measure {
                    push_grid_line(out, x, h, 2.0, grid::AR_MEASURE_LINE_COLOR, tick);
                } else if is_beat {
                    push_grid_line(out, x, h, 1.0, grid::AR_BEAT_LINE_COLOR, tick);
                }
            }
            tick += ticks_per_sub;
        }
    }
}

/// Build all instances for the arrangement view frame.
///
/// `instances` is a reusable scratch buffer — caller should retain it across frames.
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
    let w = width as f32;
    let h = height as f32;
    let lh = view.lane_height;
    let lb_w = view.label_width;
    let ppu = view.pixels_per_tick;
    let num_tracks = track_visible.len();

    // 1. Background quad
    instances.push(NoteInstance {
        x: lb_w,
        y: 0.0,
        w: w - lb_w,
        h,
        rgba_packed: pack_rgba(grid::AR_BG_COLOR.0, grid::AR_BG_COLOR.1, grid::AR_BG_COLOR.2, 1.0),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        flags: 0,
    });

    // 2. Track lane backgrounds (alternating colors)
    if num_tracks > 0 {
        let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);
        for idx in trk_first..trk_last {
            if !track_visible.get(idx).copied().unwrap_or(true) {
                continue;
            }
            let y = view.lane_y(idx);
            let col = if idx % 2 == 0 { grid::AR_LANE_EVEN_COLOR } else { grid::AR_LANE_ODD_COLOR };
            instances.push(NoteInstance {
                x: lb_w,
                y,
                w: w - lb_w,
                h: lh,
                rgba_packed: pack_rgba(col.0, col.1, col.2, 1.0),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                flags: 0,
            });
        }
    }

    // 3. Grid lines + 4. Note rectangles
    if let Some(midi) = midi {
        if let Some(tpb) = midi.ticks_per_beat() {
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

            let x_offset = lb_w - view.scroll_x;
            let y_offset = -view.scroll_y;
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
                    if notes.first().map_or(true, |n| n.start_tick as f64 > pad_end) {
                        return None;
                    }

                    let key_y_base = y_offset + lh - (key as f32 + 1.0) * lh_per_key;

                    let mut local = Vec::new();
                    let mut merge_track: Option<usize> = None;
                    let mut merge_start: u32 = 0;
                    let mut merge_end: u32 = 0;
                    let mut merge_vel: u8 = 0;

                    let flush_merge = |local: &mut Vec<NoteInstance>,
                                       ti: usize,
                                       start: u32,
                                       end: u32,
                                       vel: u8| {
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
                        let color =
                            track_colors.get(ti).copied().unwrap_or([0.5, 0.5, 0.5]);
                        let rounding = NOTE_ROUNDING * nw.min(note_h);
                        local.push(NoteInstance {
                            x: nx,
                            y: note_y,
                            w: nw,
                            h: note_h,
                            rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                            props_packed: pack_props(rounding, 0.0),
                            velocity: vel as u32,
                            flags: 0,
                        });
                    };

                    // Fast path: all visible notes on same track fully consecutive
                    if let Some(first) = notes[start_idx..].first() {
                        let t0 = first.track as usize;
                        if t0 >= trk_first
                            && t0 < trk_last
                            && track_visible.get(t0).copied().unwrap_or(true)
                        {
                            let mut fast_cont = true;
                            let mut fast_prev = first.end_tick;
                            let mut fast_max_vel = first.velocity;
                            for n in &notes[start_idx..] {
                                if (n.start_tick as f64) > pad_end {
                                    break;
                                }
                                if n.track as usize != t0 || n.start_tick > fast_prev {
                                    fast_cont = false;
                                    break;
                                }
                                fast_prev = fast_prev.max(n.end_tick);
                                if n.velocity > fast_max_vel {
                                    fast_max_vel = n.velocity;
                                }
                            }
                            if fast_cont {
                                let count = notes[start_idx..]
                                    .partition_point(|n| (n.start_tick as f64) <= pad_end);
                                if count > 0 {
                                    let last = &notes[start_idx + count - 1];
                                    flush_merge(
                                        &mut local,
                                        t0,
                                        first.start_tick,
                                        last.end_tick.max(fast_prev),
                                        fast_max_vel,
                                    );
                                    return if local.is_empty() {
                                        None
                                    } else {
                                        Some(local)
                                    };
                                }
                            }
                        }
                    }

                    for note in &notes[start_idx..] {
                        if note.start_tick as f64 > pad_end {
                            break;
                        }
                        if (note.end_tick as f64) < pad_start {
                            continue;
                        }
                        let ti = note.track as usize;
                        if merge_track == Some(ti) && merge_end >= note.start_tick {
                            merge_end = merge_end.max(note.end_tick);
                            merge_vel = merge_vel.max(note.velocity);
                        } else {
                            if let Some(prev_track) = merge_track {
                                flush_merge(
                                    &mut local, prev_track, merge_start, merge_end,
                                    merge_vel,
                                );
                            }
                            merge_track = Some(ti);
                            merge_start = note.start_tick;
                            merge_end = note.end_tick;
                            merge_vel = note.velocity;
                        }
                    }
                    if let Some(prev_track) = merge_track {
                        flush_merge(
                            &mut local, prev_track, merge_start, merge_end,
                            merge_vel,
                        );
                    }

                    if local.is_empty() { None } else { Some(local) }
                })
                .collect();

            for mut local in note_instances {
                instances.append(&mut local);
            }
        }
    }

    // 5. Playhead
    if let Some(ct) = cursor_tick {
        let cx = view.tick_to_x(ct);
        if cx >= lb_w && cx <= w {
            push_grid_line(instances, cx, h, 2.0, grid::AR_PLAYHEAD_COLOR, 0);
        }
    }
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
                key,
                start: start_tick as f64 / 480.0,
                end: end_tick as f64 / 480.0,
                start_tick,
                end_tick,
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
        assert!(elapsed.as_millis() < 100, "build took too long: {:?}", elapsed);
        assert!(!instances.is_empty(), "should have generated instances");

        let note_count = instances.iter().filter(|i| i.velocity > 0).count();
        assert_eq!(note_count, 2, "should have 2 note instances after merge");

        for inst in &instances {
            if inst.velocity > 0 {
                assert!(inst.x >= view.label_width, "note x should be >= label_width");
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
        assert!(elapsed.as_millis() < 100, "128-key build took: {:?}", elapsed);
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
            pixels_per_tick: 0.08,
            lane_height: 40.0,
            label_width: 120.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            dirty: true,
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
        assert!(elapsed.as_millis() < 3000, "large build took: {:?}", elapsed);

        let note_count = instances.iter().filter(|i| i.velocity > 0).count();
        assert!(note_count > 0, "should have generated note instances");
        assert!(note_count > 2_000_000, "should exceed old 2M cap: got {}", note_count);
    }

    #[test]
    fn test_grid_lines_dont_hang() {
        let mock = make_midi(vec![(60, 0, 480, 0, 100)]);
        let view = ArrangementView {
            pixels_per_tick: 10.0,
            ..Default::default()
        };
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
        assert!(elapsed.as_millis() < 200, "extreme zoom build took: {:?}", elapsed);
    }

    #[test]
    fn test_no_duplicate_grid_lines_at_time_sig_boundary() {
        let _mock = make_midi(vec![(60, 0, 480, 0, 100)]);
        let tpb = 480;
        let view = ArrangementView {
            pixels_per_tick: 0.5,
            label_width: 0.0,
            scroll_x: 0.0,
            dirty: true,
            ..Default::default()
        };

        let events = vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TimeSigEvent { tick: 1920, numerator: 7, denominator: 2 },
        ];

        let mut grid_lines = Vec::new();
        build_arrangement_grid(
            &mut grid_lines,
            2000.0,
            400.0,
            &view,
            tpb,
            4,
            2,
            &events,
        );

        let ticks: Vec<u32> = grid_lines.iter().map(|i| i.flags).collect();

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
        assert_eq!(count_1920, 1, "Boundary tick 1920 must appear exactly once, got {}", count_1920);
    }
}
