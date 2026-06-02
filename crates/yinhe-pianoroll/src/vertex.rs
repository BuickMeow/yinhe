/// Uniforms passed to the vertex / fragment shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub width: f32,
    pub height: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub pixels_per_tick: f32,
    pub key_height: f32,
    pub keyboard_width: f32,
    pub _pad: f32,
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
    /// Bit flags (reserved)
    pub flags: u32,
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
