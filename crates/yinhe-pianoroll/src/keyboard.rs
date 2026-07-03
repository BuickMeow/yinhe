use yinhe_theme::GpuTheme;
use yinhe_types::is_black_key;

use crate::vertex::{DrawInstance, pack_props, pack_rgba};

/// Keyboard appearance constants (geometry only; colors come from GpuTheme).
const WHITE_KEY_CORNER_RADIUS: f32 = 2.0;
const BLACK_KEY_CORNER_RADIUS: f32 = 1.5;
const KEY_BORDER_WIDTH: f32 = 0.5;

/// Append keyboard instances to the output buffer.
pub fn append_keyboard_instances(
    out: &mut Vec<DrawInstance>,
    keyboard_width: f32,
    key_height: f32,
    scroll_y: f32,
    canvas_height: f32,
    theme: &GpuTheme,
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

        let (r, g, b) = theme.pr_white_key;

        out.push(DrawInstance {
            x: 0.0,
            y,
            w: keyboard_width,
            h: key_height,
            rgba_packed: pack_rgba(r, g, b, 1.0),
            props_packed: pack_props(WHITE_KEY_CORNER_RADIUS, KEY_BORDER_WIDTH),
            velocity: 0,
            tag: 0,
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

        let (r, g, b) = theme.pr_black_key;

        out.push(DrawInstance {
            x: 0.0,
            y,
            w: keyboard_width,
            h: key_height,
            rgba_packed: pack_rgba(r, g, b, 1.0),
            props_packed: pack_props(BLACK_KEY_CORNER_RADIUS, KEY_BORDER_WIDTH),
            velocity: 0,
            tag: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_theme() -> GpuTheme {
        GpuTheme::default()
    }

    #[test]
    fn test_append_keyboard_instances_basic() {
        let mut out = Vec::new();
        let theme = make_theme();
        append_keyboard_instances(&mut out, 60.0, 12.0, 0.0, 500.0, &theme);
        assert!(!out.is_empty(), "should produce instances");
        let first = &out[0];
        assert_eq!(first.x, 0.0);
        assert_eq!(first.w, 60.0);
        assert_eq!(first.h, 12.0);
    }

    #[test]
    fn test_keyboard_white_keys_first() {
        let mut out = Vec::new();
        let theme = make_theme();
        append_keyboard_instances(&mut out, 60.0, 12.0, 0.0, 1536.0, &theme);
        let mut white_count = 0;
        let mut black_count = 0;
        for inst in &out {
            let (r, _g, _b, _) = crate::vertex::unpack_rgba(inst.rgba_packed);
            if (r - 0.70).abs() < 0.01 {
                white_count += 1;
            } else if (r - 0.16).abs() < 0.01 {
                black_count += 1;
            }
        }
        assert_eq!(white_count, 75);
        assert_eq!(black_count, 53);
    }

    #[test]
    fn test_keyboard_scrolled() {
        let mut out = Vec::new();
        let theme = make_theme();
        append_keyboard_instances(&mut out, 60.0, 12.0, 500.0, 500.0, &theme);
        assert!(!out.is_empty());
        for inst in &out {
            assert!(inst.y + inst.h > 0.0, "key should be on screen");
            assert!(inst.y < 500.0, "key should be on screen");
        }
    }

    #[test]
    fn test_keyboard_zero_height() {
        let mut out = Vec::new();
        let theme = make_theme();
        append_keyboard_instances(&mut out, 60.0, 12.0, 0.0, 0.0, &theme);
        // With canvas_height=0, only key 127 (topmost) has y=0 which is not culled
        assert_eq!(out.len(), 1, "expected 1 instance (key 127), got {}", out.len());
        assert_eq!(out[0].y, 0.0);
    }

    #[test]
    fn test_keyboard_props() {
        let mut out = Vec::new();
        let theme = make_theme();
        append_keyboard_instances(&mut out, 60.0, 12.0, 0.0, 1536.0, &theme);
        let white = &out[0];
        let (cr, bw) = crate::vertex::unpack_props(white.props_packed);
        assert!((cr - 2.0).abs() < 0.01, "white key corner radius");
        assert!((bw - 0.5).abs() < 0.01, "key border width");
        let black = out.iter().find(|i| {
            let (r, _g, _b, _) = crate::vertex::unpack_rgba(i.rgba_packed);
            (r - 0.16).abs() < 0.01
        }).unwrap();
        let (cr_b, bw_b) = crate::vertex::unpack_props(black.props_packed);
        assert!((cr_b - 1.5).abs() < 0.01, "black key corner radius");
        assert!((bw_b - 0.5).abs() < 0.01, "black key border width");
    }
}
