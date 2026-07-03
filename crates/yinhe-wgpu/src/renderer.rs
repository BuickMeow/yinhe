use wgpu::*;

use crate::layer::LayerSlot;
use crate::pipeline::RenderPipelineState;
use crate::vertex::{DrawInstance, Uniforms, TrackColorsUniform, SelectionUniform};

/// Per-frame timing breakdown returned by `prepare`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrepareTimings {
    /// Time spent in the user-supplied `build` closure.
    pub build_static: std::time::Duration,
    /// Total instances uploaded.
    pub instance_count: usize,
}

/// Generic wgpu renderer for instanced rectangle drawing.
///
/// Manages GPU buffers and provides a layered cache API:
/// `upload_uniforms` + `upload_track_colors` + `upload_selection` + `ensure_layers` + `upload_layer` / `upload_layer_force` + `draw`
///
/// View-specific convenience methods (like pianoroll's `prepare()`) belong in
/// the calling crate.
pub struct InstanceRenderer {
    device: Device,
    queue: Queue,
    render: RenderPipelineState,
    cached_uniforms: Option<Uniforms>,
    cached_track_colors: Option<TrackColorsUniform>,
    cached_selection: Option<SelectionUniform>,
    layers: Vec<LayerSlot>,
}

impl InstanceRenderer {
    pub fn new(device: Device, queue: Queue, format: TextureFormat) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
            let render_shader = device.create_shader_module(ShaderModuleDescriptor {
                label: Some("pianoroll_shader"),
                source: ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
            });

            let render = RenderPipelineState::new(&device, format, &render_shader);

            Self {
                device,
                queue,
                render,
                cached_uniforms: None,
                cached_track_colors: None,
                cached_selection: None,
                layers: Vec::new(),
            }
        })
    }

    /// Upload uniforms to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_uniforms(&mut self, uniforms: Uniforms) {
        crate::util::write_uniforms_if_changed(
            &self.queue,
            &self.render.uniform_buffer,
            &mut self.cached_uniforms,
            uniforms,
        );
    }

    /// Upload track colors to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_track_colors(&mut self, colors: &TrackColorsUniform) {
        if self.cached_track_colors.as_ref() != Some(colors) {
            self.queue.write_buffer(&self.render.track_colors_buffer, 0, bytemuck::bytes_of(colors));
            self.cached_track_colors = Some(*colors);
        }
    }

    /// Upload selection rects to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_selection(&mut self, sel: &SelectionUniform) {
        if self.cached_selection.as_ref() != Some(sel) {
            self.queue.write_buffer(&self.render.selection_buffer, 0, bytemuck::bytes_of(sel));
            self.cached_selection = Some(*sel);
        }
    }

    /// Ensure at least `count` layers exist (pushing empty ones as needed).
    pub fn ensure_layers(&mut self, count: usize) {
        while self.layers.len() < count {
            self.layers.push(LayerSlot::new(&self.device));
        }
    }

    /// Upload a layer with cache: skips rebuild when `cache_key` matches.
    ///
    /// `index` is the layer index (0 = bottom).  Layers are drawn in order.
    /// Panics if `index >= layers.len()` — call `ensure_layers` first.
    pub fn upload_layer(
        &mut self,
        index: usize,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<DrawInstance>),
    ) -> bool {
        self.layers[index].upload(&self.device, &self.queue, cache_key, build)
    }

    /// Upload a layer without cache (always rebuilds).
    pub fn upload_layer_force(
        &mut self,
        index: usize,
        build: impl FnOnce(&mut Vec<DrawInstance>),
    ) {
        self.layers[index].upload_force(&self.device, &self.queue, build);
    }

    /// Draw all layers into the given render target.
    pub fn draw(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        let mut pass = crate::util::begin_pianoroll_pass(
            encoder,
            target,
            &self.render.pipeline,
            &self.render.bind_group,
            width,
            height,
        );

        for layer in &self.layers {
            layer.draw(&mut pass, 0);
        }
    }

    /// Total instances across all layers.
    pub fn total_layer_instances(&self) -> usize {
        self.layers.iter().map(|l| l.instance_count()).sum()
    }
}
