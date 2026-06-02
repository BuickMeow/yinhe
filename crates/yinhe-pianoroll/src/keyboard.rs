use yinhe_types::is_black_key;

use crate::vertex::{NoteInstance, pack_props, pack_rgba};

/// Keyboard appearance constants.
const WHITE_KEY_COLOR: (f32, f32, f32) = (0.94, 0.94, 0.94);
const BLACK_KEY_COLOR: (f32, f32, f32) = (0.16, 0.16, 0.17);
const WHITE_KEY_CORNER_RADIUS: f32 = 2.0;
const BLACK_KEY_CORNER_RADIUS: f32 = 1.5;
const KEY_BORDER_WIDTH: f32 = 0.5;

/// Append keyboard instances to the output buffer.
pub fn append_keyboard_instances(
    out: &mut Vec<NoteInstance>,
    keyboard_width: f32,
    key_height: f32,
    scroll_y: f32,
    canvas_height: f32,
    active_keys: &[bool; 128],
    active_colors: &[[f32; 3]; 128],
) {
    let bottom = 128.0 * key_height - scroll_y;

    // White keys first
    for key in 0u8..128 {
        if is_black_key(key) {
            continue;
        }
        let y = bottom - (key as f32 + 1.0) * key_height;
        // Skip if off screen
        if y + key_height < 0.0 || y > canvas_height {
            continue;
        }

        let (r, g, b) = if active_keys[key as usize] {
            let c = active_colors[key as usize];
            (c[0], c[1], c[2])
        } else {
            WHITE_KEY_COLOR
        };

        out.push(NoteInstance {
            x: 0.0,
            y,
            w: keyboard_width,
            h: key_height,
            rgba_packed: pack_rgba(r, g, b, 1.0),
            props_packed: pack_props(WHITE_KEY_CORNER_RADIUS, KEY_BORDER_WIDTH),
            velocity: 0,
            flags: 0,
        });
    }

    // Black keys on top
    for key in 0u8..128 {
        if !is_black_key(key) {
            continue;
        }
        let y = bottom - (key as f32 + 1.0) * key_height;
        if y + key_height < 0.0 || y > canvas_height {
            continue;
        }

        let (r, g, b) = if active_keys[key as usize] {
            let c = active_colors[key as usize];
            (c[0], c[1], c[2])
        } else {
            BLACK_KEY_COLOR
        };

        out.push(NoteInstance {
            x: 0.0,
            y,
            w: keyboard_width,
            h: key_height,
            rgba_packed: pack_rgba(r, g, b, 1.0),
            props_packed: pack_props(BLACK_KEY_CORNER_RADIUS, KEY_BORDER_WIDTH),
            velocity: 0,
            flags: 0,
        });
    }
}
