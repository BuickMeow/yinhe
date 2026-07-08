use wgpu::*;
use yinhe_types::TimeSigEvent;

use crate::vertex::Uniforms;

/// Hash time signature events using position-sensitive folding.
/// Order matters: reordering events changes the hash.
pub fn hash_time_sigs(events: &[TimeSigEvent]) -> u64 {
    let mut h: u64 = 0;
    for ev in events {
        h = h.wrapping_mul(31).wrapping_add(ev.tick as u64);
        h = h.wrapping_mul(31).wrapping_add(ev.numerator as u64);
        h = h.wrapping_mul(31).wrapping_add(ev.denominator as u64);
    }
    h
}

/// Compute `(scroll_x_pos, scroll_frac)` from raw scroll_x and scroll_mode.
///
/// - `scroll_mode == 0`: raw scroll, returns `(scroll_x, 0.0)`
/// - `scroll_mode != 0`: integer-aligned scroll, returns
///   `(scroll_x.floor(), scroll_x - scroll_x.floor())`
pub fn compute_scroll_frac(scroll_x: f32, scroll_mode: u32) -> (f32, f32) {
    match scroll_mode {
        0 => (scroll_x, 0.0),
        _ => {
            let f = scroll_x.floor();
            (f, scroll_x - f)
        }
    }
}

/// Hash a sequence of f64 values using position-sensitive folding.
/// Order matters: `hash_f64s(&[1.0, 2.0]) != hash_f64s(&[2.0, 1.0])`.
pub fn hash_f64s(values: &[f64]) -> u64 {
    let mut h: u64 = 0;
    for &v in values {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v.to_bits() as u64);
    }
    h
}

/// Hash a sequence of f32 values using position-sensitive folding.
pub fn hash_f32s(values: &[f32]) -> u64 {
    let mut h: u64 = 0;
    for &v in values {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v.to_bits() as u64);
    }
    h
}

/// Hash a sequence of bool values using position-sensitive folding.
pub fn hash_bools(values: &[bool]) -> u64 {
    let mut h: u64 = 0;
    for &v in values {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
    }
    h
}

/// Create a GPU vertex buffer with memtrace tracking.
pub(super) fn create_vertex_buffer(
    device: &Device,
    label: &str,
    size_bytes: u64,
) -> Buffer {
    let buffer = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: size_bytes,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    });
    yinhe_memtrace::add_gpu_resource(size_bytes);
    buffer
}

/// Round up `required` to the next power-of-two ≥ `min`.
pub(super) fn next_capacity(required: usize, min: usize) -> usize {
    let mut cap = min;
    while cap < required {
        cap *= 2;
    }
    cap
}

/// Write uniforms to GPU only if they differ from the cached value.
/// Returns `true` if the write was performed.
pub(super) fn write_uniforms_if_changed(
    queue: &Queue,
    buffer: &Buffer,
    cached: &mut Option<Uniforms>,
    uniforms: Uniforms,
) -> bool {
    if cached.as_ref().is_some_and(|c| *c == uniforms) {
        return false;
    }
    queue.write_buffer(buffer, 0, bytemuck::bytes_of(&uniforms));
    *cached = Some(uniforms);
    true
}

/// Begin a render pass with the standard pianoroll clear color and set up
/// viewport, pipeline, and bind group.
pub(super) fn begin_pianoroll_pass<'a>(
    encoder: &'a mut CommandEncoder,
    target: &'a TextureView,
    pipeline: &'a RenderPipeline,
    bind_group: &'a BindGroup,
    width: u32,
    height: u32,
) -> RenderPass<'a> {
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("pianoroll_pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: Operations {
                load: LoadOp::Clear(Color {
                    r: 0.12,
                    g: 0.12,
                    b: 0.14,
                    a: 1.0,
                }),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    });
    pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass
}
