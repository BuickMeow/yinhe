use rayon::prelude::*;
use yinhe_types::{NoteSource, TimeSigEvent, TRACK_PALETTE, is_black_key, seek_first_note};

use crate::grid;
use crate::keyboard;
use crate::vertex::{NoteInstance, pack_props, pack_rgba};
use crate::view::PianoRollView;

const BLACK_KEY_ROW_COLOR: (f32, f32, f32) = (0.10, 0.10, 0.12);
const NOTE_ROUNDING: f32 = 0.15;
const NOTE_BORDER_WIDTH: f32 = 0.5;
const SELECTED_BORDER_WIDTH: f32 = 1.5;

/// Build piano roll grid line instances into `out`, respecting time signature changes.
pub fn build_pianoroll_grid(
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
        out, w, h, &view.base, tpb, default_num, default_den, time_sig_events,
        grid::PR_MEASURE_LINE_COLOR,
        grid::PR_BEAT_LINE_COLOR,
        Some(grid::PR_SUB_BEAT_LINE_COLOR),
    );
}

/// Build all instances for the piano roll frame (backward-compatible wrapper).
pub fn build_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &std::collections::HashSet<(u16, u32)>,
    track_visible: &[bool],
    cursor_tick: Option<f64>,
) -> ([bool; 128], [[f32; 3]; 128]) {
    let result = build_static_instances(instances, width, height, midi, view, selected, track_visible, cursor_tick);
    build_cursor_instance(instances, cursor_tick, view, width, height);
    result
}

/// Build static instances: background, black-key rows, grid lines, notes, keyboard.
///
/// Uses `cursor_tick` only for active-key detection (keyboard highlighting),
/// NOT for drawing the cursor line — that is handled by `build_cursor_instance`.
pub fn build_static_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &std::collections::HashSet<(u16, u32)>,
    track_visible: &[bool],
    cursor_tick: Option<f64>,
) -> ([bool; 128], [[f32; 3]; 128]) {
    let mut active_keys = [false; 128];
    let mut active_colors = [[0.0f32; 3]; 128];

    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let bottom = 128.0 * kh - view.base.scroll_y;
    let ppu = view.base.pixels_per_tick;

    // 1. Background: single full-area quad, then overlay black-key rows only.
    instances.push(NoteInstance {
        x: kb_w,
        y: 0.0,
        w: w - kb_w,
        h,
        rgba_packed: pack_rgba(grid::PR_BG_COLOR.0, grid::PR_BG_COLOR.1, grid::PR_BG_COLOR.2, 1.0),
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
        instances.push(NoteInstance {
            x: kb_w,
            y,
            w: w - kb_w,
            h: kh,
            rgba_packed: pack_rgba(BLACK_KEY_ROW_COLOR.0, BLACK_KEY_ROW_COLOR.1, BLACK_KEY_ROW_COLOR.2, 1.0),
            props_packed: pack_props(0.0, 0.0),
            velocity: 0,
            tag: 0,
        });
    }

    if let Some(midi) = midi {
        if let Some(tpb) = midi.ticks_per_beat() {
            let (tick_start, tick_end) = view.visible_tick_range(w);

            // 2. Grid lines
            let (def_num, def_den) = midi.time_sig_default();
            let sig_events = midi.time_sig_events();
            build_pianoroll_grid(instances, w, h, view, tpb, def_num, def_den, sig_events);

            // 3. Notes — padded tick range, with seek and parallel key processing
            let tick_pad = (w - kb_w) / ppu;
            let pad_start = (tick_start - tick_pad as f64).max(0.0);
            let pad_end = tick_end + tick_pad as f64;
            let (key_lo, key_hi) = view.visible_key_range(h);
            let has_selection = !selected.is_empty();

            let x_offset = kb_w - view.base.scroll_x;

            let results: Vec<(Vec<NoteInstance>, u8, bool, [f32; 3])> = (0u8..128)
                .into_par_iter()
                .filter_map(|key| {
                    if key < key_lo || key > key_hi {
                        return None;
                    }
                    let notes = midi.key_notes(key);
                    if notes.is_empty() {
                        return None;
                    }
                    let start_idx = seek_first_note(key, midi, pad_start as u32);

                    let key_y = bottom - (key as f32 + 1.0) * kh;

                    let mut local = Vec::new();
                    let mut key_active = false;
                    let mut key_color = [0.0f32; 3];

                    for note in &notes[start_idx..] {
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

                        let nx = x_offset + note.start_tick as f32 * ppu;
                        let nw =
                            ((note.end_tick - note.start_tick) as f32 * ppu).max(2.0);
                        let ny = key_y;
                        let nh = kh;

                        let trk = note.track as usize % TRACK_PALETTE.len();
                        let color = TRACK_PALETTE[trk];

                        let is_selected = has_selection
                            && selected.contains(&(note.track, note.start_tick));
                        let border_w = if is_selected {
                            SELECTED_BORDER_WIDTH
                        } else {
                            NOTE_BORDER_WIDTH
                        };
                        let rounding = NOTE_ROUNDING * nw.min(nh);

                        local.push(NoteInstance {
                            x: nx,
                            y: ny,
                            w: nw,
                            h: nh,
                            rgba_packed: pack_rgba(
                                color[0], color[1], color[2], 1.0,
                            ),
                            props_packed: pack_props(rounding, border_w),
                            velocity: note.velocity as u32,
                            tag: if is_selected { 1 } else { 0 },
                        });

                        if let Some(ct) = cursor_tick {
                            if note.start_tick as f64 <= ct
                                && ct < note.end_tick as f64
                            {
                                key_active = true;
                                key_color = color;
                            }
                        }
                    }

                    if local.is_empty() {
                        None
                    } else {
                        Some((local, key, key_active, key_color))
                    }
                })
                .collect();

            for (mut local, key, active, color) in results {
                instances.append(&mut local);
                if active {
                    active_keys[key as usize] = true;
                    active_colors[key as usize] = color;
                }
            }
        }
    }

    // 4. Keyboard
    keyboard::append_keyboard_instances(
        instances,
        kb_w,
        kh,
        view.base.scroll_y,
        h,
        &active_keys,
        &active_colors,
    );

    (active_keys, active_colors)
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
