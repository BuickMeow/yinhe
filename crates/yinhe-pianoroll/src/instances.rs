use rayon::prelude::*;
use yinhe_types::{NoteSource, TRACK_PALETTE, TimeSigEvent, is_black_key};

use crate::grid;
use crate::keyboard;
use crate::vertex::{NoteInstance, pack_props, pack_rgba};
use crate::view::PianoRollView;

const BLACK_KEY_ROW_COLOR: (f32, f32, f32) = (0.10, 0.10, 0.12);
const NOTE_ROUNDING: f32 = 0.15;

/// Build background + black-key row instances (layer 0).
/// Dependencies: scroll_y, key_height, h
pub fn build_decor(out: &mut Vec<NoteInstance>, w: f32, h: f32, kb_w: f32, kh: f32, scroll_y: f32) {
    let bottom = 128.0 * kh - scroll_y;

    out.push(NoteInstance {
        x: kb_w,
        y: 0.0,
        w: w - kb_w,
        h,
        rgba_packed: pack_rgba(
            grid::PR_BG_COLOR.0,
            grid::PR_BG_COLOR.1,
            grid::PR_BG_COLOR.2,
            1.0,
        ),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        tag: 0,
    });
    for key in 0u8..128 {
        if !is_black_key(key) {
            continue;
        }
        let y = bottom - (key as f32 + 1.0) * kh;
        if y + kh < 0.0 || y > h {
            continue;
        }
        out.push(NoteInstance {
            x: kb_w,
            y,
            w: w - kb_w,
            h: kh,
            rgba_packed: pack_rgba(
                BLACK_KEY_ROW_COLOR.0,
                BLACK_KEY_ROW_COLOR.1,
                BLACK_KEY_ROW_COLOR.2,
                1.0,
            ),
            props_packed: pack_props(0.0, 0.0),
            velocity: 0,
            tag: 0,
        });
    }
}

/// Build grid line instances (layer 1).
/// Dependencies: scroll_x, pixels_per_tick, time_sig
pub fn build_grid(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    view: &PianoRollView,
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
        grid::PR_MEASURE_LINE_COLOR,
        grid::PR_BEAT_LINE_COLOR,
        Some(grid::PR_SUB_BEAT_LINE_COLOR),
    );
}

/// Build note instances (layer 2).
/// Dependencies: scroll_y, key_height, selection, track_visible
/// x/w store ticks (shader converts to pixels), y/h store pixel positions.
///
/// `tick_pad`: extra ticks to include on each side of the visible range.
/// Used when scroll_x is quantized in the cache key — ensures cached notes
/// cover the full bucket range.
pub fn build_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &PianoRollView,
    selected: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    tick_pad: f64,
) {
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let bottom = 128.0 * kh - view.base.scroll_y;
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let (key_lo, key_hi) = view.visible_key_range(h);
    let has_selection = !selected.is_empty();
    let pad_start = (tick_start - tick_pad).max(0.0);
    let pad_end = tick_end + tick_pad;

    let results: Vec<Vec<NoteInstance>> = (key_lo..=key_hi)
        .into_par_iter()
        .filter_map(|key| {
            let notes = midi.key_notes_in_range(key, pad_start as u32, pad_end as u32);
            if notes.is_empty() {
                return None;
            }

            let key_y = bottom - (key as f32 + 1.0) * kh;

            let mut local = Vec::new();

            for note in notes {
                if note.start_tick as f64 > pad_end {
                    break;
                }
                if (note.end_tick as f64) < pad_start {
                    continue;
                }
                if !track_visible
                    .get(note.track as usize)
                    .copied()
                    .unwrap_or(true)
                {
                    continue;
                }

                let trk_idx = note.track as usize;
                let color = track_colors
                    .get(trk_idx)
                    .copied()
                    .unwrap_or_else(|| TRACK_PALETTE[trk_idx % TRACK_PALETTE.len()]);

                let is_selected =
                    has_selection && selected.contains(&(note.track, note.start_tick, key));

                local.push(NoteInstance {
                    x: note.start_tick as f32, // tick (shader converts to pixel)
                    y: key_y,                  // pixel
                    w: note.end_tick as f32,   // tick (shader converts to pixel)
                    h: kh,                     // pixel
                    rgba_packed: pack_rgba(color[0], color[1], color[2], 1.0),
                    props_packed: 0, // shader computes rounding/border
                    velocity: note.velocity as u32,
                    tag: if is_selected { 1 } else { 0 },
                });
            }

            if local.is_empty() { None } else { Some(local) }
        })
        .collect();

    for mut local in results {
        out.append(&mut local);
    }
}

/// Build keyboard instances (layer 3).
/// Dependencies: scroll_y, key_height, h
pub fn build_keyboard(out: &mut Vec<NoteInstance>, kb_w: f32, kh: f32, scroll_y: f32, h: f32) {
    keyboard::append_keyboard_instances(out, kb_w, kh, scroll_y, h);
}

/// Build all instances for the piano roll frame (backward-compatible wrapper).
pub fn build_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) {
    build_static_instances(
        instances,
        width,
        height,
        midi,
        view,
        selected,
        track_visible,
        track_colors,
    );
}

/// Build static instances (backward-compatible — rebuilds everything).
pub fn build_static_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) {
    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;

    build_decor(instances, w, h, kb_w, kh, scroll_y);

    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        build_grid(instances, w, h, view, tpb, def_num, def_den, sig_events);
        build_notes(
            instances,
            w,
            h,
            midi,
            view,
            selected,
            track_visible,
            track_colors,
            0.0,
        );
    }

    build_keyboard(instances, kb_w, kh, scroll_y, h);
}

/// Build only the cursor line instance (O(1) work).
pub fn build_cursor_instance(
    instances: &mut Vec<NoteInstance>,
    cursor_tick: Option<f64>,
    view: &PianoRollView,
    width: u32,
    height: u32,
) {
    if let Some(ct) = cursor_tick {
        let kb_w = view.keyboard_width();
        let w = width as f32;
        let h = height as f32;
        let cx = view.tick_to_x(ct);
        if cx >= kb_w && cx <= w {
            instances.push(NoteInstance {
                x: cx,
                y: 0.0,
                w: 2.0,
                h,
                rgba_packed: pack_rgba(1.0, 1.0, 1.0, 0.8),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{Note, TimeSigEvent, TimelineViewBase};

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

    fn make_view() -> PianoRollView {
        PianoRollView {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            key_height: 12.0,
        }
    }

    #[test]
    fn test_build_decor_background() {
        let mut out = Vec::new();
        build_decor(&mut out, 800.0, 500.0, 60.0, 12.0, 0.0);
        assert!(!out.is_empty());
        let bg = &out[0];
        assert_eq!(bg.x, 60.0);
        assert_eq!(bg.y, 0.0);
        assert_eq!(bg.w, 740.0);
        assert_eq!(bg.h, 500.0);
    }

    #[test]
    fn test_build_decor_black_key_rows() {
        let mut out = Vec::new();
        build_decor(&mut out, 800.0, 500.0, 60.0, 12.0, 0.0);
        let black_rows = &out[1..];
        assert!(!black_rows.is_empty());
        for row in black_rows {
            assert_eq!(row.x, 60.0);
            assert_eq!(row.w, 740.0);
            assert_eq!(row.h, 12.0);
        }
    }

    #[test]
    fn test_build_decor_scrolled() {
        let mut out = Vec::new();
        build_decor(&mut out, 800.0, 500.0, 60.0, 12.0, 1000.0);
        assert!(!out.is_empty());
        for inst in &out[1..] {
            assert!(inst.y + inst.h > 0.0, "row should be on screen");
            assert!(inst.y < 500.0, "row should be on screen");
        }
    }

    #[test]
    fn test_build_grid_basic() {
        let mut out = Vec::new();
        let view = make_view();
        build_grid(&mut out, 800.0, 500.0, &view, 480, 4, 2, &[]);
        assert!(!out.is_empty(), "grid should produce lines");
        for inst in &out {
            // Grid lines are centered: x = tick_x - line_width/2
            // First line at tick 0 has tick_x = 60.0, line_width = 2.0 → x = 59.0
            assert!(inst.x >= 58.0, "grid line should be near keyboard boundary");
            assert!(inst.x <= 800.0, "grid line should be within viewport");
            assert_eq!(inst.h, 500.0);
        }
    }

    #[test]
    fn test_build_grid_with_time_sig_change() {
        let mut out = Vec::new();
        let view = make_view();
        let sigs = vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
        ];
        build_grid(&mut out, 800.0, 500.0, &view, 480, 4, 2, &sigs);
        assert!(!out.is_empty());
    }

    #[test]
    fn test_build_notes_basic() {
        let mut out = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let selected = std::collections::HashSet::new();
        let track_visible = vec![true];
        let track_colors = [[0.5, 0.5, 0.5]];

        build_notes(&mut out, 800.0, 500.0, &midi, &view, &selected, &track_visible, &track_colors, 0.0);
        assert!(!out.is_empty(), "should produce note instances");
        let note = &out[0];
        assert_eq!(note.x, 0.0);
        assert_eq!(note.w, 480.0);
        assert_eq!(note.h, 12.0);
        assert_eq!(note.velocity, 100);
    }

    #[test]
    fn test_build_notes_hidden_track() {
        let mut out = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let selected = std::collections::HashSet::new();
        let track_visible = vec![false];

        build_notes(&mut out, 800.0, 500.0, &midi, &view, &selected, &track_visible, &[], 0.0);
        assert!(out.is_empty(), "notes on hidden track should be skipped");
    }

    #[test]
    fn test_build_notes_selected_tag() {
        let mut out = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let mut selected = std::collections::HashSet::new();
        selected.insert((0, 0, 100));
        let track_visible = vec![true];
        let track_colors = [[0.5, 0.5, 0.5]];

        build_notes(&mut out, 800.0, 500.0, &midi, &view, &selected, &track_visible, &track_colors, 0.0);
        assert_eq!(out[0].tag, 1, "selected note should have tag=1");
    }

    #[test]
    fn test_build_notes_unselected_tag() {
        let mut out = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let selected = std::collections::HashSet::new();
        let track_visible = vec![true];
        let track_colors = [[0.5, 0.5, 0.5]];

        build_notes(&mut out, 800.0, 500.0, &midi, &view, &selected, &track_visible, &track_colors, 0.0);
        assert_eq!(out[0].tag, 0, "unselected note should have tag=0");
    }

    #[test]
    fn test_build_notes_multiple_keys() {
        let mut out = Vec::new();
        let midi = make_midi(vec![
            (100, 0, 480, 0, 100),
            (104, 0, 480, 0, 80),
            (107, 0, 480, 0, 90),
        ]);
        let view = make_view();
        let selected = std::collections::HashSet::new();
        let track_visible = vec![true];
        let track_colors = [[0.5, 0.5, 0.5]];

        build_notes(&mut out, 800.0, 500.0, &midi, &view, &selected, &track_visible, &track_colors, 0.0);
        assert_eq!(out.len(), 3, "should produce 3 note instances");
    }

    #[test]
    fn test_build_notes_tick_pad() {
        let mut out = Vec::new();
        let midi = make_midi(vec![(100, 1000, 1480, 0, 100)]);
        let mut view = make_view();
        view.base.scroll_x = 0.0;
        let selected = std::collections::HashSet::new();
        let track_visible = vec![true];
        let track_colors = [[0.5, 0.5, 0.5]];

        build_notes(&mut out, 800.0, 500.0, &midi, &view, &selected, &track_visible, &track_colors, 500.0);
        assert!(!out.is_empty(), "note should be included with tick_pad");
    }

    #[test]
    fn test_build_cursor_instance_visible() {
        let mut out = Vec::new();
        let view = make_view();
        build_cursor_instance(&mut out, Some(480.0), &view, 800, 500);
        assert_eq!(out.len(), 1);
        let cursor = &out[0];
        assert!((cursor.x - 132.0).abs() < 0.01);
        assert_eq!(cursor.w, 2.0);
        assert_eq!(cursor.h, 500.0);
    }

    #[test]
    fn test_build_cursor_instance_none() {
        let mut out = Vec::new();
        let view = make_view();
        build_cursor_instance(&mut out, None, &view, 800, 500);
        assert!(out.is_empty());
    }

    #[test]
    fn test_build_cursor_instance_off_screen_left() {
        let mut out = Vec::new();
        let view = make_view();
        build_cursor_instance(&mut out, Some(0.0), &view, 800, 500);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn test_build_cursor_instance_off_screen_right() {
        let mut out = Vec::new();
        let view = make_view();
        build_cursor_instance(&mut out, Some(99999.0), &view, 800, 500);
        assert!(out.is_empty(), "cursor off-screen right should be skipped");
    }

    #[test]
    fn test_build_static_instances_with_midi() {
        let mut out = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let selected = std::collections::HashSet::new();
        let track_visible = vec![true];
        let track_colors = [[0.5, 0.5, 0.5]];

        build_static_instances(&mut out, 800, 500, Some(&midi), &view, &selected, &track_visible, &track_colors);
        assert!(out.len() > 3, "should have multiple layers");
    }

    #[test]
    fn test_build_static_instances_without_midi() {
        let mut out = Vec::new();
        let view = make_view();
        let selected = std::collections::HashSet::new();

        build_static_instances(&mut out, 800, 500, None, &view, &selected, &[], &[]);
        assert!(!out.is_empty(), "should still produce decor and keyboard");
    }
}
