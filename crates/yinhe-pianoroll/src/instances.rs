use yinhe_types::{NoteSource, is_black_key};

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
) -> ([bool; 128], [[f32; 3]; 128]) {
    let mut active_keys = [false; 128];
    let mut active_colors = [[0.0f32; 3]; 128];

    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width;
    let kh = view.key_height;
    let bottom = 128.0 * kh - view.scroll_y;

    // 1. Background rows (alternating for black/white keys)
    for key in 0u8..128 {
        let y = bottom - (key as f32 + 1.0) * kh;
        if y + kh < 0.0 || y > h {
            continue;
        }
        let (r, g, b) = if is_black_key(key) {
            BLACK_KEY_ROW_COLOR
        } else {
            BG_COLOR
        };
        instances.push(NoteInstance {
            x: kb_w,
            y,
            w: w - kb_w,
            h: kh,
            rgba_packed: pack_rgba(r, g, b, 1.0),
            props_packed: pack_props(0.0, 0.0),
            velocity: 0,
            flags: 0,
        });
    }

    // 2. Grid lines (vertical: measures, beats, sub-beats)
    if let Some(midi) = midi {
        if let Some(tpb) = midi.ticks_per_beat() {
            let (tick_start, tick_end) = view.visible_tick_range(w);
            let ppu = view.pixels_per_tick;

            // Assume 4/4 time: 4 beats per measure, each beat = tpb ticks
            let ticks_per_measure = tpb * 4;
            let sub_beat_div = 4; // 16th notes
            let ticks_per_sub = tpb / sub_beat_div;

            // Sub-beat lines
            if ppu > 0.02 {
                let start = ((tick_start / ticks_per_sub as f64).floor() as u32)
                    .saturating_mul(ticks_per_sub);
                let mut tick = start;
                while (tick as f64) <= tick_end {
                    let x = view.tick_to_x(tick as f64);
                    if x >= kb_w && x <= w {
                        instances.push(NoteInstance {
                            x,
                            y: 0.0,
                            w: 1.0,
                            h,
                            rgba_packed: pack_rgba(
                                SUB_BEAT_LINE_COLOR.0,
                                SUB_BEAT_LINE_COLOR.1,
                                SUB_BEAT_LINE_COLOR.2,
                                SUB_BEAT_LINE_COLOR.3,
                            ),
                            props_packed: pack_props(0.0, 0.0),
                            velocity: 0,
                            flags: 0,
                        });
                    }
                    tick += ticks_per_sub;
                }
            }

            // Beat lines
            let start = ((tick_start / tpb as f64).floor() as u32)
                .saturating_mul(tpb);
            let mut tick = start;
            while (tick as f64) <= tick_end {
                let x = view.tick_to_x(tick as f64);
                if x >= kb_w && x <= w {
                    instances.push(NoteInstance {
                        x,
                        y: 0.0,
                        w: 1.0,
                        h,
                        rgba_packed: pack_rgba(
                            BEAT_LINE_COLOR.0,
                            BEAT_LINE_COLOR.1,
                            BEAT_LINE_COLOR.2,
                            BEAT_LINE_COLOR.3,
                        ),
                        props_packed: pack_props(0.0, 0.0),
                        velocity: 0,
                        flags: 0,
                    });
                }
                tick += tpb;
            }

            // Measure lines
            let start = ((tick_start / ticks_per_measure as f64).floor() as u32)
                .saturating_mul(ticks_per_measure);
            let mut tick = start;
            while (tick as f64) <= tick_end {
                let x = view.tick_to_x(tick as f64);
                if x >= kb_w && x <= w {
                    instances.push(NoteInstance {
                        x,
                        y: 0.0,
                        w: 2.0,
                        h,
                        rgba_packed: pack_rgba(
                            MEASURE_LINE_COLOR.0,
                            MEASURE_LINE_COLOR.1,
                            MEASURE_LINE_COLOR.2,
                            MEASURE_LINE_COLOR.3,
                        ),
                        props_packed: pack_props(0.0, 0.0),
                        velocity: 0,
                        flags: 0,
                    });
                }
                tick += ticks_per_measure;
            }

            // 3. Notes
            let (tick_start, tick_end) = view.visible_tick_range(w);
            let (key_lo, key_hi) = view.visible_key_range(h);

            for key in key_lo..=key_hi {
                let notes = midi.key_notes(key);
                if notes.is_empty() {
                    continue;
                }

                // Binary search for first visible note
                let start_idx = notes.partition_point(|n| (n.end_tick as f64) < tick_start);

                for note in &notes[start_idx..] {
                    if note.start_tick as f64 > tick_end {
                        break;
                    }

                    let nx = view.tick_to_x(note.start_tick as f64);
                    let nw = ((note.end_tick - note.start_tick) as f32 * ppu).max(2.0);
                    let ny = view.key_to_y(note.key);
                    let nh = kh;

                    // Track color
                    let trk = note.track as usize % TRACK_PALETTE.len();
                    let color = TRACK_PALETTE[trk];

                    // Check if selected
                    let is_selected = selected.contains(&(note.track, note.start_tick));
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

                    // Mark active key if note is currently "playing" (for visual feedback)
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
        view.scroll_y,
        h,
        &active_keys,
        &active_colors,
    );

    (active_keys, active_colors)
}
