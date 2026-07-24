//! Generic wgpu renderer for instanced rectangle drawing.
//!
//! Manages three pipelines sharing one uniform buffer:
//!   - **decor pipeline** (32B `DrawInstance`, `vs_main`): decor, grid, keyboard, cursor
//!   - **curve pipeline** (32B `CurveInstance`, `vs_main_curve`): automation SDF lines/curves
//!   - **note pipeline** (16B `NoteInstance`, `vs_main_note`): PR notes, AR notes, ghost notes
//!
//! With GPU compute cull enabled, notes are uploaded once to a persistent
//! buffer and culled on the GPU each frame instead of rebuilt on the CPU.
//!
//! Layers are stored in z-order; `draw` switches pipelines as needed when
//! traversing layers.

use wgpu::*;

use crate::cull::CullState;
use crate::layer::{AnyLayer, LayerKind};
use crate::pipeline::RenderPipelineState;
use crate::vertex::{CurveInstance, NoteInstance, Uniforms, SelectionUniform, VelocityBarInstance};

/// Per-frame timing breakdown returned by `prepare`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrepareTimings {
    /// Time spent in the user-supplied `build` closure.
    pub build_static: std::time::Duration,
    /// Total instances uploaded.
    pub instance_count: usize,
}

pub struct InstanceRenderer {
    device: Device,
    queue: Queue,
    render: RenderPipelineState,
    cached_uniforms: Option<Uniforms>,
    cached_track_colors: Option<Vec<[f32; 4]>>,
    cached_selection: Option<SelectionUniform>,
    layers: Vec<AnyLayer>,
    cull: CullState,
    pub theme: yinhe_theme::GpuTheme,
}

/// Generates a typed `upload_*_layer` method for one layer variant.
/// Eliminates the 4× near-identical boilerplate that previously existed.
macro_rules! impl_upload_layer {
    ($method:ident, $kind:ident, $variant:ident, $T:ty) => {
        /// Upload a layer. Skips rebuild when `cache_key` matches the previous value.
        /// Pass `cache_key: 0` to force upload (always rebuilds).
        pub fn $method(
            &mut self,
            index: usize,
            cache_key: u64,
            build: impl FnOnce(&mut Vec<$T>),
        ) -> bool {
            self.ensure_layer(index, LayerKind::$kind);
            if let AnyLayer::$variant(slot) = &mut self.layers[index] {
                if cache_key == 0 {
                    slot.upload_force(&self.device, &self.queue, build);
                    true
                } else {
                    slot.upload(&self.device, &self.queue, cache_key, build)
                }
            } else {
                unreachable!()
            }
        }
    };
}

impl InstanceRenderer {
    pub fn new(device: Device, queue: Queue, format: TextureFormat) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
            let render_shader = device.create_shader_module(ShaderModuleDescriptor {
                label: Some("pianoroll_shader"),
                source: ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
            });

            let render = RenderPipelineState::new(&device, format, &render_shader);
            let cull = CullState::new(&device);

            Self {
                device,
                queue,
                render,
                cached_uniforms: None,
                cached_track_colors: None,
                cached_selection: None,
                layers: Vec::new(),
                cull,
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
    pub fn upload_track_colors(&mut self, colors: &[[f32; 4]]) {
        self.ensure_track_colors_capacity(colors.len());
        if self.cached_track_colors.as_deref() != Some(colors) {
            let bytes = bytemuck::cast_slice(colors);
            self.queue.write_buffer(&self.render.track_colors_buffer, 0, bytes);
            self.cached_track_colors = Some(colors.to_vec());
        }
    }

    /// Grow the track_colors storage buffer when `count` exceeds current capacity.
    /// Recreates the buffer + bind group (cheap, happens only when track count grows).
    fn ensure_track_colors_capacity(&mut self, count: usize) {
        if count <= self.render.track_colors_capacity as usize {
            return;
        }
        let new_capacity = count.max(1) as u32;
        let new_size = new_capacity as u64 * 16;
        let new_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("track_colors"),
            size: new_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(new_size);
        self.render.track_colors_buffer = new_buffer;
        self.render.track_colors_capacity = new_capacity;
        // Recreate bind group with the new buffer.
        self.render.bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("render_bind_group"),
            layout: &self.render.bind_group_layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: self.render.uniform_buffer.as_entire_binding() },
                BindGroupEntry { binding: 1, resource: self.render.track_colors_buffer.as_entire_binding() },
                BindGroupEntry { binding: 2, resource: self.render.selection_buffer.as_entire_binding() },
            ],
        });
        // Invalidate cache to force re-upload with the new buffer.
        self.cached_track_colors = None;
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

    impl_upload_layer!(upload_note_layer, Note, Note, NoteInstance);
    impl_upload_layer!(upload_curve_layer, Curve, Curve, CurveInstance);
    impl_upload_layer!(upload_velocity_layer, Velocity, Velocity, VelocityBarInstance);

    /// Upload ALL note instances to the persistent GPU buffer for compute cull.
    /// Call this once on MIDI load/change, NOT every frame.
    /// Also records per-key offsets and revisions for future incremental uploads.
    pub fn upload_all_notes_for_cull(
        &mut self,
        notes: &[NoteInstance],
        per_key_offsets: &[u32; 129],
        key_revisions: &[u64; 128],
    ) {
        self.cull.upload_all_notes(
            &self.device, &self.queue, &self.render.uniform_buffer,
            notes, per_key_offsets, key_revisions,
        );
    }

    /// Incrementally upload a single key's notes. Grows the key's buffer and
    /// recreates its bind group on demand, so this handles count changes too.
    /// Returns false only if the key was never uploaded before (caller should
    /// fall back to `upload_all_notes_for_cull`).
    pub fn try_incremental_key_upload(
        &mut self,
        key: u8,
        notes: &[NoteInstance],
        revision: u64,
    ) -> bool {
        if !self.cull.has_key_buffer(key) {
            return false;
        }
        self.cull.upload_one_key(
            &self.device, &self.queue, &self.render.uniform_buffer, key, notes,
        );
        self.cull.uploaded_key_revisions[key as usize] = revision;
        true
    }

    /// Get the uploaded key revisions for comparison with model.
    pub fn uploaded_key_revisions(&self) -> &[u64; 128] {
        &self.cull.uploaded_key_revisions
    }

    /// Whether GPU compute cull is ready (all notes have been uploaded).
    pub fn cull_ready(&self) -> bool {
        self.cull.is_ready()
    }

    /// Draw all layers into the given render target.
    /// Uses GPU compute cull for note layers if available, otherwise falls back
    /// to CPU-built layer data.
    pub fn draw(
        &mut self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        if self.cull.is_ready() {
            self.draw_with_cull(encoder, target, width, height);
        } else {
            self.draw_legacy(encoder, target, width, height);
        }
    }

    /// Draw decor → velocity → curve layers (shared by both draw paths).
    /// Notes are handled separately by each path (legacy: CPU-built note layers;
    /// cull: GPU compute culled notes + ghost layer).
    fn draw_static_layers(&self, pass: &mut RenderPass<'_>) {
        for layer in &self.layers {
            if layer.kind() == LayerKind::Decor {
                pass.set_pipeline(&self.render.pipeline);
                layer.draw(pass, 0);
            }
        }
        for layer in &self.layers {
            if layer.kind() == LayerKind::Velocity {
                pass.set_pipeline(&self.render.velocity_pipeline);
                layer.draw(pass, 0);
            }
        }
        for layer in &self.layers {
            if layer.kind() == LayerKind::Curve {
                pass.set_pipeline(&self.render.curve_pipeline);
                layer.draw(pass, 0);
            }
        }
    }

    /// Legacy draw (no GPU cull): draw all decor layers then all note layers.
    ///
    /// Z-order: decor (bg + grid) → velocity bars → curve (automation) → notes
    fn draw_legacy(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        let mut pass = crate::util::begin_pianoroll_pass(
            encoder, target, &self.render.pipeline, &self.render.bind_group, width, height,
        );

        self.draw_static_layers(&mut pass);

        // Step 4: all note layers
        for layer in &self.layers {
            if layer.kind() == LayerKind::Note {
                pass.set_pipeline(&self.render.note_pipeline);
                layer.draw(&mut pass, 0);
            }
        }
    }

    /// Compute the visible key range from cached uniforms (PR mode only).
    /// Returns `(lo, hi)` inclusive. For non-PR modes, returns `(0, 127)` since
    /// the Y position depends on both key and track (can't skip by key alone).
    ///
    /// Adds 1 key of padding on each side to handle notes whose top/bottom edge
    /// peeks into the viewport due to sub-pixel rounding.
    fn visible_key_range(&self) -> (u8, u8) {
        let u = match &self.cached_uniforms {
            Some(u) => u,
            None => return (0, 127),
        };
        if u.mode != 1 || u.key_height <= 0.0 {
            return (0, 127);
        }
        // PR: bottom = 128 * key_height - scroll_y
        // y_to_key(y) = ceil((bottom - y) / key_height) - 1, clamped to 0..127
        let bottom = 128.0 * u.key_height - u.scroll_y;
        let top_key = ((bottom / u.key_height).ceil() as i32 - 1).clamp(0, 127);
        let bottom_key = (((bottom - u.height) / u.key_height).ceil() as i32 - 1).clamp(0, 127);
        let lo = bottom_key.min(top_key);
        let hi = bottom_key.max(top_key);
        // 1-key padding for sub-pixel edge cases.
        let lo = lo.saturating_sub(1).clamp(0, 127);
        let hi = hi.saturating_add(1).clamp(0, 127);
        (lo as u8, hi as u8)
    }

    /// GPU compute cull draw: dispatch cull pass, then draw layers.
    ///
    /// Z-order: decor (bg + grid) → velocity bars → curve (automation) → culled notes → ghost notes.
    fn draw_with_cull(
        &mut self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        let (key_lo, key_hi) = self.visible_key_range();
        // Phase 1: Compute cull (skipped if uniforms + notes unchanged since last frame)
        let uniforms = self.cached_uniforms.unwrap_or_default();
        self.cull.dispatch_cull(encoder, key_lo, key_hi, &uniforms);

        // Phase 2: Single render pass
        let mut pass = crate::util::begin_pianoroll_pass(
            encoder, target, &self.render.pipeline, &self.render.bind_group, width, height,
        );

        // Step 1-3: decor → velocity → curve
        self.draw_static_layers(&mut pass);

        // Step 4: culled notes (from GPU compute cull buffer)
        self.cull.draw_visible_notes(&mut pass, &self.render.note_pipeline, &self.render.bind_group, key_lo, key_hi);

        // Step 5: ghost notes (last note layer, if any) — on top of everything
        let ghost = self.layers.iter().filter(|l| l.kind() == LayerKind::Note).last();
        if let Some(ghost) = ghost {
            pass.set_pipeline(&self.render.note_pipeline);
            ghost.draw(&mut pass, 0);
        }
    }

    pub fn theme(&self) -> &yinhe_theme::GpuTheme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: yinhe_theme::GpuTheme) {
        self.theme = theme;
    }
}
