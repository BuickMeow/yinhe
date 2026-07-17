/// Uniforms passed to the vertex / fragment shader.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub width: f32,
    pub height: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub pixels_per_tick: f32,
    pub key_height: f32,
    pub keyboard_width: f32,
    pub mode: u32, // 0=pixel, 1=PR notes(tick→pixel+rounding), 2=AR notes(tick→pixel)
    pub scroll_frac: f32, // fractional part of scroll_x for sub-pixel NDC offset
    pub scroll_mode: u32, // 0=原始, 1=整数对齐, 2=子像素偏移
    pub min_border_width: f32,
    pub track_count: u32, // number of valid tracks in track_colors
    pub sel_rect_count: u32, // number of valid selection rects
    pub note_outline: u32, // 0=no outline (saves GPU fill rate), 1=outline on
    pub lane_height: f32, // AR: per-track lane height (PR unused, set to 0)
    pub note_alpha: f32, // note alpha override (PR=1.0, AR=0.85)
}

/// Maximum number of tracks supported in track_colors storage buffer.
/// 65536 × 16B = 1MB — exceeds typical uniform buffer limits, so we bind it
/// as a read-only storage buffer (see pipeline.rs).
pub const MAX_TRACKS: usize = 65536;

/// Maximum number of selection rects supported in uniform buffer.
pub const MAX_SEL_RECTS: usize = 32;

/// Track colors buffer: array of vec4<f32> (RGBA).
/// Each entry is (r, g, b, a) in 0.0-1.0 range.
/// Bound as a read-only storage buffer (1MB at MAX_TRACKS=65536).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TrackColorsUniform {
    pub colors: [[f32; 4]; MAX_TRACKS],
}

/// Selection rects uniform buffer: array of vec4<u32>.
/// Each rect uses 2 vec4 entries:
///   entry 0: (tick_start, tick_end, key_lo, key_hi)
///   entry 1: (track_lo, track_hi, 0, 0)
#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SelectionUniform {
    pub rects: [[u32; 4]; MAX_SEL_RECTS * 2],
}

/// Packed instance: 32 bytes.
/// Layout: xywh (vec4 f32) + packed (vec4 u32) = 2 vertex attributes.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawInstance {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// RGBA packed as 4x UNORM8: R|G<<8|B<<16|A<<24
    pub rgba_packed: u32,
    /// corner_radius (f16 high) | border_width (f16 low)
    pub props_packed: u32,
    /// Reserved
    pub velocity: u32,
    /// Semantic tag: grid lines store tick, notes store selection state
    pub tag: u32,
}

/// Note instance: 16 bytes = vec4<u32>.
/// Used by the note pipeline (PR notes, AR notes, ghost notes).
///
/// All pixel positions are computed in the GPU vertex shader from uniforms;
/// the CPU only stores semantic data (ticks, key, track).
///
/// Layout:
///   d0 = start_tick (u32)
///   d1 = end_tick   (u32)
///   d2 = packed: key(u8) | track(u16) | vel(u8)
///   d3 = reserved (u32)
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NoteInstance {
    pub start_tick: u32,
    pub end_tick: u32,
    /// key(u8, bits 0..8) | track(u16, bits 8..24) | vel(u8, bits 24..32)
    pub packed: u32,
    pub reserved: u32,
}

impl NoteInstance {
    pub fn pack(key: u8, track: u16, vel: u8) -> u32 {
        key as u32 | ((track as u32) << 8) | ((vel as u32) << 24)
    }
}

/// Curve/line/anchor instance: 32 bytes.
///
/// One instance renders one of:
/// - **Line/curve segment** (`shape == 0`): parameterized curve whose x is
///   linear in t and y follows `f(t) = k·t² + (1-k)·t` (k = tension_norm).
///   - `tension == 0.0` → straight line (shader fast path via `sd_line`)
///   - `tension != 0.0` → quadratic curve (shader numerically solves nearest t)
///   For `SegmentShape::Step`, CPU pushes **two** instances: a horizontal
///   segment (y1→y1) plus a vertical segment (x2,x2 with y1→y2).
/// - **Filled circle** (`shape == 1`): anchor point at (x1, y1) with
///   `thickness` = radius. `x2/y2/tension` ignored.
///
/// Layout (matches `vs_main_curve` in shader.wgsl):
///   - `@location(0)`: Float32x4 = (x1, y1, x2, y2)
///   - `@location(1)`: Float32x4 = (thickness, tension_norm, shape, _)
///   - `@location(2)`: Uint32    = rgba UNORM8 (R|G<<8|B<<16|A<<24)
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CurveInstance {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub thickness: f32,
    /// 归一化张力 ∈ [-1, 1]，= `i8_tension / 127.0`。circle 时忽略。
    pub tension: f32,
    pub rgba_packed: u32,
    /// 0 = line/curve segment, 1 = filled circle (anchor).
    pub shape: u32,
}

impl CurveInstance {
    /// Construct a straight-line segment instance.
    pub fn line(x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, color: [f32; 4]) -> Self {
        CurveInstance {
            x1, y1, x2, y2,
            thickness,
            tension: 0.0,
            rgba_packed: pack_rgba(color[0], color[1], color[2], color[3]),
            shape: 0,
        }
    }

    /// Construct a quadratic-curve segment instance.
    /// `tension_norm` ∈ [-1, 1] (= `i8_tension / 127.0`).
    pub fn curve(
        x1: f32, y1: f32, x2: f32, y2: f32,
        thickness: f32, tension_norm: f32, color: [f32; 4],
    ) -> Self {
        CurveInstance {
            x1, y1, x2, y2,
            thickness,
            tension: tension_norm,
            rgba_packed: pack_rgba(color[0], color[1], color[2], color[3]),
            shape: 0,
        }
    }

    /// Construct a filled-circle anchor instance at `(cx, cy)` with `radius`.
    pub fn circle(cx: f32, cy: f32, radius: f32, color: [f32; 4]) -> Self {
        CurveInstance {
            x1: cx, y1: cy,
            x2: cx, y2: cy,  // AABB collapses to a single point; pad handles the rest
            thickness: radius,
            tension: 0.0,
            rgba_packed: pack_rgba(color[0], color[1], color[2], color[3]),
            shape: 1,
        }
    }
}

impl DrawInstance {
    /// Construct a solid-filled rectangle with no rounded corners, no border.
    pub fn solid_rect(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        DrawInstance {
            x, y, w, h,
            rgba_packed: pack_rgba(color[0], color[1], color[2], color[3]),
            props_packed: pack_props(0.0, 0.0),
            velocity: 0,
            tag: 0,
        }
    }

    /// Construct a rectangle with corner radius and border width.
    pub fn with_props(x: f32, y: f32, w: f32, h: f32, color: [f32; 4], corner_radius: f32, border_width: f32) -> Self {
        DrawInstance {
            x, y, w, h,
            rgba_packed: pack_rgba(color[0], color[1], color[2], color[3]),
            props_packed: pack_props(corner_radius, border_width),
            velocity: 0,
            tag: 0,
        }
    }
}

/// Pack RGBA floats (0.0-1.0) into a single u32 (UNORM8 x 4).
pub fn pack_rgba(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let r8 = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let g8 = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let b8 = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let a8 = (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    r8 | (g8 << 8) | (b8 << 16) | (a8 << 24)
}

/// Pack corner_radius and border_width (both f32) into a single u32 (2x f16).
pub fn pack_props(corner_radius: f32, border_width: f32) -> u32 {
    let cr = half::f16::from_f32(corner_radius);
    let bw = half::f16::from_f32(border_width);
    (cr.to_bits() as u32) | ((bw.to_bits() as u32) << 16)
}

/// Unpack a u32 into RGBA floats (0.0-1.0).  Inverse of `pack_rgba`.
pub fn unpack_rgba(packed: u32) -> (f32, f32, f32, f32) {
    let r = (packed & 0xff) as f32 / 255.0;
    let g = ((packed >> 8) & 0xff) as f32 / 255.0;
    let b = ((packed >> 16) & 0xff) as f32 / 255.0;
    let a = ((packed >> 24) & 0xff) as f32 / 255.0;
    (r, g, b, a)
}

/// Unpack a u32 into corner_radius and border_width f32 values.  Inverse of `pack_props`.
pub fn unpack_props(packed: u32) -> (f32, f32) {
    let cr = half::f16::from_bits(packed as u16);
    let bw = half::f16::from_bits((packed >> 16) as u16);
    (cr.to_f32(), bw.to_f32())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_rgba_roundtrip() {
        let colors = [
            (0.0, 0.0, 0.0, 0.0),
            (1.0, 1.0, 1.0, 1.0),
            (0.5, 0.5, 0.5, 1.0),
            (0.2, 0.4, 0.6, 0.8),
        ];
        for &(r, g, b, a) in &colors {
            let packed = pack_rgba(r, g, b, a);
            let (r2, g2, b2, a2) = unpack_rgba(packed);
            assert!((r - r2).abs() < 0.005, "r mismatch: {} vs {}", r, r2);
            assert!((g - g2).abs() < 0.005, "g mismatch: {} vs {}", g, g2);
            assert!((b - b2).abs() < 0.005, "b mismatch: {} vs {}", b, b2);
            assert!((a - a2).abs() < 0.005, "a mismatch: {} vs {}", a, a2);
        }
    }

    #[test]
    fn test_pack_rgba_clamps() {
        let packed = pack_rgba(-0.5, 2.0, 0.5, 1.5);
        let (r, g, b, a) = unpack_rgba(packed);
        assert!((r - 0.0).abs() < 0.01);
        assert!((g - 1.0).abs() < 0.01);
        assert!((b - 0.5).abs() < 0.01);
        assert!((a - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_pack_props_roundtrip() {
        let cases = [(0.0, 0.0), (2.0, 0.5), (1.5, 1.0), (60.0, 3.0)];
        for &(cr, bw) in &cases {
            let packed = pack_props(cr, bw);
            let (cr2, bw2) = unpack_props(packed);
            assert!((cr - cr2).abs() < 0.01, "cr mismatch: {} vs {}", cr, cr2);
            assert!((bw - bw2).abs() < 0.01, "bw mismatch: {} vs {}", bw, bw2);
        }
    }

    #[test]
    fn test_note_instance_size() {
        assert_eq!(std::mem::size_of::<DrawInstance>(), 32);
        assert_eq!(std::mem::size_of::<NoteInstance>(), 16);
        assert_eq!(std::mem::size_of::<CurveInstance>(), 32);
    }

    #[test]
    fn test_uniforms_trait_bounds() {
        fn assert_pod<T: bytemuck::Pod + bytemuck::Zeroable>() {}
        assert_pod::<Uniforms>();
        assert_pod::<DrawInstance>();
        assert_pod::<NoteInstance>();
        assert_pod::<CurveInstance>();
    }
}
