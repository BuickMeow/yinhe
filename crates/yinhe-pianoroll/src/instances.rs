use yinhe_types::{NoteSource, is_black_key, seek_first_note};

use crate::keyboard;
use crate::vertex::{NoteInstance, pack_props, pack_rgba};
use crate::view::PianoRollView;

/// Predefined palette for up to 128 tracks.
const TRACK_PALETTE: [[f32; 3]; 16] = [
    [0.29, 0.56, 0.89], // blue
    [0.89, 0.35, 0.35], // red
    [0.30, 0.78, 0.30], // green
    [0.95, 0.65, 0.20], // orange
    [0.65, 0.40, 0.85], // purple
    [0.20, 0.80, 0.80], // cyan
    [0.95, 0.75, 0.20], // yellow
    [0.90, 0.45, 0.70], // pink
    [0.40, 0.65, 0.35], // olive
    [0.70, 0.50, 0.30], // brown
    [0.35, 0.55, 0.75], // steel
    [0.85, 0.55, 0.35], // copper
    [0.45, 0.80, 0.55], // mint
    [0.75, 0.35, 0.55], // wine
    [0.55, 0.55, 0.80], // lavender
    [0.60, 0.75, 0.30], // lime
];

/// Colors
const BG_COLOR: (f32, f32, f32) = (0.12, 0.12, 0.14);
const BLACK_KEY_ROW_COLOR: (f32, f32, f32) = (0.10, 0.10, 0.12);
const MEASURE_LINE_COLOR: (f32, f32, f32, f32) = (0.35, 0.35, 0.40, 1.0);
const BEAT_LINE_COLOR: (f32, f32, f32, f32) = (0.22, 0.22, 0.25, 1.0);
const SUB_BEAT_LINE_COLOR: (f32, f32, f32, f32) = (0.16, 0.16, 0.18, 1.0);

const NOTE_ROUNDING: f32 = 0.15;
const NOTE_BORDER_WIDTH: f32 = 0.5;
const SELECTED_BORDER_WIDTH: f32 = 1.5;

/// Build all instances for the piano roll frame.
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
    let mut active_keys = [false; 128];
    let mut active_colors = [[0.0f32; 3]; 128];

    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width;
    let kh = view.key_height;
    let bottom = 128.0 * kh - view.scroll_y;
    let ppu = view.pixels_per_tick;

    // 1. Background: single full-area quad, then overlay black-key rows only.
    //    This reduces 128 instances → 1 + (visible black keys).
    instances.push(NoteInstance {
        x: kb_w,
        y: 0.0,
        w: w - kb_w,
        h,
        rgba_packed: pack_rgba(BG_COLOR.0, BG_COLOR.1, BG_COLOR.2, 1.0),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        flags: 0,
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
            flags: 0,
        });
    }

    if let Some(midi) = midi {
        if let Some(tpb) = midi.ticks_per_beat() {
            let (tick_start, tick_end) = view.visible_tick_range(w);

            // 2. Grid lines — single pass at sub-beat granularity, with
            //    measure/beat/sub-beat color+width determined by tick alignment.
            if ppu > 0.01 {
                let ticks_per_measure = tpb * 4;
                let sub_beat_div = 4u32;
                let ticks_per_sub = tpb / sub_beat_div;

                let start = ((tick_start / ticks_per_sub as f64).floor() as u32)
                    .saturating_mul(ticks_per_sub);
                let mut tick = start;
                while (tick as f64) <= tick_end {
                    let x = view.tick_to_x(tick as f64);
                    if x >= kb_w && x <= w {
                        let is_measure = tick % ticks_per_measure == 0;
                        let is_beat = tick % tpb == 0;
                        let (lw, col) = if is_measure {
                            (2.0, MEASURE_LINE_COLOR)
                        } else if is_beat {
                            (1.0, BEAT_LINE_COLOR)
                        } else {
                            (1.0, SUB_BEAT_LINE_COLOR)
                        };
                        instances.push(NoteInstance {
                            x,
                            y: 0.0,
                            w: lw,
                            h,
                            rgba_packed: pack_rgba(col.0, col.1, col.2, col.3),
                            props_packed: pack_props(0.0, 0.0),
                            velocity: 0,
                            flags: 0,
                        });
                    }
                    tick += ticks_per_sub;
                }
            }

            // 3. Notes — padded tick range, linear scan, track_visible filter
            let tick_pad = (w - kb_w) / ppu;
            let pad_start = (tick_start - tick_pad as f64).max(0.0);
            let pad_end = tick_end + tick_pad as f64;
            let (key_lo, key_hi) = view.visible_key_range(h);
            let has_selection = !selected.is_empty();

            for key in key_lo..=key_hi {
                let notes = midi.key_notes(key);
                if notes.is_empty() {
                    continue;
                }

                let start_idx = seek_first_note(key, midi, pad_start as u32);
                for note in &notes[start_idx..] {
                    if note.start_tick as f64 > pad_end {
                        break;
                    }
                    if (note.end_tick as f64) < pad_start {
                        continue;
                    }
                    // Track visibility filter
                    if !track_visible.get(note.track as usize).copied().unwrap_or(true) {
                        continue;
                    }

                    let nx = view.tick_to_x(note.start_tick as f64);
                    let nw = ((note.end_tick - note.start_tick) as f32 * ppu).max(2.0);
                    let ny = view.key_to_y(note.key);
                    let nh = kh;

                    let trk = note.track as usize % TRACK_PALETTE.len();
                    let color = TRACK_PALETTE[trk];

                    let is_selected = has_selection && selected.contains(&(note.track, note.start_tick));
                    let border_w = if is_selected {
                        SELECTED_BORDER_WIDTH
                    } else {
                        NOTE_BORDER_WIDTH
                    };
                    let rounding = NOTE_ROUNDING * nw.min(nh);

                    instances.push(NoteInstance {
                        x: nx,
                        y: ny,
                        w: nw,
                        h: nh,
                        rgba_packed: pack_rgba(color[0], color[1], color[2], 1.0),
                        props_packed: pack_props(rounding, border_w),
                        velocity: note.velocity as u32,
                        flags: if is_selected { 1 } else { 0 },
                    });

                    // Mark active key if cursor is within this note's time range
                    if let Some(ct) = cursor_tick {
                        if note.start_tick as f64 <= ct && ct < note.end_tick as f64 {
                            active_keys[key as usize] = true;
                            active_colors[key as usize] = color;
                        }
                    }
                }
            }

            // 3b. Cursor line
            if let Some(ct) = cursor_tick {
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
                        flags: 0,
                    });
                }
            }
        }
    }

    // 4. Keyboard
    keyboard::append_keyboard_instances(
        instances,
        kb_w,
        kh,
        view.scroll_y,
        h,
        &active_keys,
        &active_colors,
    );

    (active_keys, active_colors)
}
