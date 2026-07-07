use wgpu::*;

use crate::layer::LayerSlot;
use crate::pipeline::RenderPipelineState;
use crate::vertex::{DrawInstance, NoteInstance, Uniforms, TrackColorsUniform, SelectionUniform};

/// Per-frame timing breakdown returned by `prepare`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrepareTimings {
    /// Time spent in the user-supplied `build` closure.
    pub build_static: std::time::Duration,
    /// Total instances uploaded.
    pub instance_count: usize,
}

/// Layer kind: decor (32B `DrawInstance`) or note (16B `NoteInstance`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    Decor,
    Note,
}

/// Type-erased layer slot that can hold either `DrawInstance` or `NoteInstance`.
pub enum AnyLayer {
    Decor(LayerSlot<DrawInstance>),
    Note(LayerSlot<NoteInstance>),
}

impl AnyLayer {
    fn new(device: &Device, kind: LayerKind) -> Self {
        match kind {
            LayerKind::Decor => AnyLayer::Decor(LayerSlot::new(device)),
            LayerKind::Note => AnyLayer::Note(LayerSlot::new(device)),
        }
    }

    fn kind(&self) -> LayerKind {
        match self {
            AnyLayer::Decor(_) => LayerKind::Decor,
            AnyLayer::Note(_) => LayerKind::Note,
        }
    }

    pub fn instance_count(&self) -> usize {
        match self {
            AnyLayer::Decor(l) => l.instance_count(),
            AnyLayer::Note(l) => l.instance_count(),
        }
    }

    fn draw<'a>(&self, pass: &mut RenderPass<'a>, vertex_slot: u32) {
        match self {
            AnyLayer::Decor(l) => l.draw(pass, vertex_slot),
            AnyLayer::Note(l) => l.draw(pass, vertex_slot),
        }
    }
}

/// Generic wgpu renderer for instanced rectangle drawing.
///
/// Manages two pipelines sharing one uniform buffer:
///   - **decor pipeline** (32B `DrawInstance`, `vs_main`): decor, grid, keyboard, cursor, automation
///   - **note pipeline** (16B `NoteInstance`, `vs_main_note`): PR notes, AR notes, ghost notes
///
/// Layers are stored in z-order; `draw` switches pipelines as needed when
/// traversing layers.
pub struct InstanceRenderer {
    device: Device,
    queue: Queue,
    render: RenderPipelineState,
    cached_uniforms: Option<Uniforms>,
    cached_track_colors: Option<Vec<u8>>,
    cached_selection: Option<SelectionUniform>,
    layers: Vec<AnyLayer>,
    pub theme: yinhe_theme::GpuTheme,
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
                theme: yinhe_theme::GpuTheme::default(),
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
        let new_bytes = bytemuck::bytes_of(colors);
        if self.cached_track_colors.as_deref() != Some(new_bytes) {
            self.queue.write_buffer(&self.render.track_colors_buffer, 0, new_bytes);
            self.cached_track_colors = Some(new_bytes.to_vec());
        }
    }

    /// Upload selection rects to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_selection(&mut self, sel: &SelectionUniform) {
        if self.cached_selection.as_ref() != Some(sel) {
            self.queue.write_buffer(&self.render.selection_buffer, 0, bytemuck::bytes_of(sel));
            self.cached_selection = Some(*sel);
        }
    }

    /// Ensure at least `count` decor layers exist (pushing empty ones as needed).
    /// Layers created here are decor by default; call `upload_note_layer` to
    /// upgrade a layer to the note pipeline.
    pub fn ensure_layers(&mut self, count: usize) {
        while self.layers.len() < count {
            self.layers.push(AnyLayer::new(&self.device, LayerKind::Decor));
        }
    }

    /// Ensure layer `index` exists with the given kind.  If the layer already
    /// exists with a different kind, it is replaced (buffer is recreated).
    pub fn ensure_layer(&mut self, index: usize, kind: LayerKind) {
        while self.layers.len() <= index {
            self.layers.push(AnyLayer::new(&self.device, LayerKind::Decor));
        }
        if self.layers[index].kind() != kind {
            self.layers[index] = AnyLayer::new(&self.device, kind);
        }
    }

    /// Upload a decor layer with cache: skips rebuild when `cache_key` matches.
    pub fn upload_layer(
        &mut self,
        index: usize,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<DrawInstance>),
    ) -> bool {
        self.ensure_layer(index, LayerKind::Decor);
        if let AnyLayer::Decor(slot) = &mut self.layers[index] {
            slot.upload(&self.device, &self.queue, cache_key, build)
        } else {
            unreachable!()
        }
    }

    /// Upload a decor layer without cache (always rebuilds).
    pub fn upload_layer_force(
        &mut self,
        index: usize,
        build: impl FnOnce(&mut Vec<DrawInstance>),
    ) {
        self.ensure_layer(index, LayerKind::Decor);
        if let AnyLayer::Decor(slot) = &mut self.layers[index] {
            slot.upload_force(&self.device, &self.queue, build);
        } else {
            unreachable!()
        }
    }

    /// Upload a note layer with cache: skips rebuild when `cache_key` matches.
    pub fn upload_note_layer(
        &mut self,
        index: usize,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<NoteInstance>),
    ) -> bool {
        self.ensure_layer(index, LayerKind::Note);
        if let AnyLayer::Note(slot) = &mut self.layers[index] {
            slot.upload(&self.device, &self.queue, cache_key, build)
        } else {
            unreachable!()
        }
    }

    /// Upload a note layer without cache (always rebuilds).
    pub fn upload_note_layer_force(
        &mut self,
        index: usize,
        build: impl FnOnce(&mut Vec<NoteInstance>),
    ) {
        self.ensure_layer(index, LayerKind::Note);
        if let AnyLayer::Note(slot) = &mut self.layers[index] {
            slot.upload_force(&self.device, &self.queue, build);
        } else {
            unreachable!()
        }
    }

    /// Draw all layers into the given render target.
    /// Switches between decor and note pipelines as needed.
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

        let mut current_kind: Option<LayerKind> = None;
        for layer in &self.layers {
            let kind = layer.kind();
            if current_kind != Some(kind) {
                match kind {
                    LayerKind::Decor => pass.set_pipeline(&self.render.pipeline),
                    LayerKind::Note => pass.set_pipeline(&self.render.note_pipeline),
                }
                current_kind = Some(kind);
            }
            layer.draw(&mut pass, 0);
        }
    }

    /// Total instances across all layers.
    pub fn total_layer_instances(&self) -> usize {
        self.layers.iter().map(|l| l.instance_count()).sum()
    }

    pub fn theme(&self) -> &yinhe_theme::GpuTheme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: yinhe_theme::GpuTheme) {
        self.theme = theme;
    }
}
