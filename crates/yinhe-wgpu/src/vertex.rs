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
}

/// Packed instance: 32 bytes.
/// Layout: xywh (vec4 f32) + packed (vec4 u32) = 2 vertex attributes.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NoteInstance {
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
        assert_eq!(std::mem::size_of::<NoteInstance>(), 32);
    }

    #[test]
    fn test_uniforms_trait_bounds() {
        fn assert_pod<T: bytemuck::Pod + bytemuck::Zeroable>() {}
        assert_pod::<Uniforms>();
        assert_pod::<NoteInstance>();
    }
}
