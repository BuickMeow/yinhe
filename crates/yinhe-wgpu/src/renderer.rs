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

use yinhe_types::KEY_COUNT;

use crate::cull::CullState;
use crate::layer::{AnyLayer, LayerKind};
use crate::pipeline::RenderPipelineState;
use crate::resource::TrackedBuffer;
use crate::vertex::{CurveInstance, NoteInstance, SelectionUniform, Uniforms, VelocityBarInstance};

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
    cached_track_offsets: Option<Vec<f32>>,
    cached_selection: Option<SelectionUniform>,
    layers: Vec<AnyLayer>,
    pub(crate) cull: CullState,
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

            // Cull 先建：note_pipeline 的 group 1 复用其 all_instances bind
            // group layout（顶点阶段经索引间接读 all_instances）。
            let cull = CullState::new(&device);
            let render = RenderPipelineState::new(
                &device,
                format,
                &render_shader,
                &cull.all_bind_group_layout,
            );

            Self {
                device,
                queue,
                render,
                cached_uniforms: None,
                cached_track_colors: None,
                cached_track_offsets: None,
                cached_selection: None,
                layers: Vec::new(),
                cull,
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
            self.queue
                .write_buffer(&self.render.track_colors_buffer, 0, bytes);
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
        let new_buffer = TrackedBuffer::new(
            &self.device,
            &BufferDescriptor {
                label: Some("track_colors"),
                size: new_size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        // 旧 buffer 随赋值 drop 自动 sub_gpu_resource（旧代码漏 sub，统计会漂移）。
        self.render.track_colors_buffer = new_buffer;
        self.render.track_colors_capacity = new_capacity;
        // Recreate bind group with the new buffer.
        self.rebuild_render_bind_group();
        // Invalidate cache to force re-upload with the new buffer.
        self.cached_track_colors = None;
    }

    /// Recreate the shared render bind group（storage buffer 增长后必须重建，
    /// 否则 bind group 仍指向旧 buffer）。track_colors / track_offsets 共用。
    fn rebuild_render_bind_group(&mut self) {
        self.render.bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("render_bind_group"),
            layout: &self.render.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: self.render.uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: self.render.track_colors_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: self.render.selection_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: self.render.track_offsets_buffer.as_entire_binding(),
                },
            ],
        });
    }

    /// Upload AR 每轨主行 y 偏移表（音乐坐标像素）。Skips the write when unchanged.
    pub fn upload_track_offsets(&mut self, offsets: &[f32]) {
        self.ensure_track_offsets_capacity(offsets.len());
        if self.cached_track_offsets.as_deref() != Some(offsets) {
            let bytes = bytemuck::cast_slice(offsets);
            self.queue
                .write_buffer(&self.render.track_offsets_buffer, 0, bytes);
            self.cached_track_offsets = Some(offsets.to_vec());
        }
    }

    /// Grow the track_offsets storage buffer when count exceeds current capacity.
    /// Recreates the buffer + bind group（与 track_colors 同一模式）。
    fn ensure_track_offsets_capacity(&mut self, count: usize) {
        if count <= self.render.track_offsets_capacity as usize {
            return;
        }
        let new_capacity = count.max(4) as u32;
        let new_size = new_capacity as u64 * 4;
        let new_buffer = TrackedBuffer::new(
            &self.device,
            &BufferDescriptor {
                label: Some("track_offsets"),
                size: new_size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        // 旧 buffer 随赋值 drop 自动 sub_gpu_resource（同上，修复统计泄漏）。
        self.render.track_offsets_buffer = new_buffer;
        self.render.track_offsets_capacity = new_capacity;
        // Recreate bind group with the new buffer.
        self.rebuild_render_bind_group();
        // Invalidate cache to force re-upload with the new buffer.
        self.cached_track_offsets = None;
    }

    /// Upload selection rects to the GPU.  Skips the write when the value is unchanged.
    pub fn upload_selection(&mut self, sel: &SelectionUniform) {
        if self.cached_selection.as_ref() != Some(sel) {
            self.queue
                .write_buffer(&self.render.selection_buffer, 0, bytemuck::bytes_of(sel));
            self.cached_selection = Some(*sel);
        }
    }

    /// Ensure at least `count` decor layers exist (pushing empty ones as needed).
    /// Layers created here are decor by default; call `upload_note_layer` to
    /// upgrade a layer to the note pipeline.
    pub fn ensure_layers(&mut self, count: usize) {
        while self.layers.len() < count {
            self.layers
                .push(AnyLayer::new(&self.device, LayerKind::Decor));
        }
    }

    /// Ensure layer `index` exists with the given kind.  If the layer already
    /// exists with a different kind, it is replaced (buffer is recreated).
    pub fn ensure_layer(&mut self, index: usize, kind: LayerKind) {
        while self.layers.len() <= index {
            self.layers
                .push(AnyLayer::new(&self.device, LayerKind::Decor));
        }
        if self.layers[index].kind() != kind {
            self.layers[index] = AnyLayer::new(&self.device, kind);
        }
    }

    impl_upload_layer!(upload_note_layer, Note, Note, NoteInstance);
    impl_upload_layer!(upload_curve_layer, Curve, Curve, CurveInstance);
    impl_upload_layer!(
        upload_velocity_layer,
        Velocity,
        Velocity,
        VelocityBarInstance
    );

    /// Upload the per-track visibility bitmask to the cull shader.
    /// Call whenever `track_visible` changes; the shader immediately stops
    /// emitting hidden tracks' notes even from stale (pre-rebuild) buffers.
    pub fn upload_track_mask(&mut self, track_visible: &[bool]) {
        self.cull.upload_track_mask(&self.queue, track_visible);
    }

    /// Upload ALL note instances to the persistent GPU buffer for compute cull.
    /// Call this once on MIDI load/change, NOT every frame.
    /// Also records per-key offsets and revisions for future incremental uploads.
    pub fn upload_all_notes_for_cull(
        &mut self,
        notes: &[NoteInstance],
        per_key_offsets: &[u32; KEY_COUNT + 1],
        key_revisions: &[u64; KEY_COUNT],
    ) {
        if let Err(e) = self.cull.upload_all_notes(
            &self.device,
            &self.queue,
            &self.render.uniform_buffer,
            notes,
            per_key_offsets,
            key_revisions,
        ) {
            tracing::error!("[cull] 全量上传音符失败（显存预算不足，已降级跳过）：{e}");
        }
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
        if let Err(e) = self.cull.upload_one_key(
            &self.device,
            &self.queue,
            &self.render.uniform_buffer,
            key,
            notes,
        ) {
            tracing::error!("[cull] 单 key 上传失败（显存预算不足，已降级跳过）：{e}");
            return false;
        }
        self.cull.uploaded_key_revisions[key as usize] = revision;
        true
    }

    /// Get the uploaded key revisions for comparison with model.
    pub fn uploaded_key_revisions(&self) -> &[u64; KEY_COUNT] {
        &self.cull.uploaded_key_revisions
    }

    /// Whether GPU compute cull is ready (all notes have been uploaded).
    pub fn cull_ready(&self) -> bool {
        self.cull.is_ready()
    }

    /// Drop all per-key GPU cull buffers and reset tracking so the next render
    /// treats the document as fresh (forces full upload on the next frame).
    ///
    /// Call when the active document changes (close / switch / new project) so
    /// note buffers from the previous document don't leak into the next render.
    pub fn clear_cull(&mut self) {
        self.cull.clear_cull();
    }

    /// Whether GPU cull note buffers are uploaded (draw will use compute cull).
    pub fn cull_is_ready(&self) -> bool {
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
        log::debug!(
            "[draw] enter cull_ready={} layers={}",
            self.cull.is_ready(),
            self.layers.len()
        );
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
                layer.draw(pass, 0, None);
            }
        }
        for layer in &self.layers {
            if layer.kind() == LayerKind::Velocity {
                pass.set_pipeline(&self.render.velocity_pipeline);
                layer.draw(pass, 0, Some(&self.render.index_buffer));
            }
        }
        for layer in &self.layers {
            if layer.kind() == LayerKind::Curve {
                pass.set_pipeline(&self.render.curve_pipeline);
                layer.draw(pass, 0, None);
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
            encoder,
            target,
            &self.render.pipeline,
            &self.render.bind_group,
            width,
            height,
        );

        self.draw_static_layers(&mut pass);

        // Step 4: all note layers
        for layer in &self.layers {
            if layer.kind() == LayerKind::Note {
                pass.set_pipeline(&self.render.note_direct_pipeline);
                layer.draw(&mut pass, 0, Some(&self.render.index_buffer));
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
        if u.orientation == 1 {
            // 纵向瀑布流：音高轴沿 X（key * key_height - scroll_x）。
            // 可见 key 范围按横向方位计算，padding 1 键防子像素抖动。
            let lo = (u.scroll_x / u.key_height).floor() as i32;
            let hi = ((u.scroll_x + u.width) / u.key_height).ceil() as i32;
            let lo = lo.clamp(0, 127);
            let hi = hi.saturating_sub(1).clamp(0, 127);
            let lo = lo.saturating_sub(1).clamp(0, 127);
            let hi = hi.saturating_add(1).clamp(0, 127);
            return (lo as u8, hi as u8);
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
        _encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        tracing::debug!(
            "[cull-draw] is_ready={} cached_scroll_x={:?} cached_ppu={:?} cached_mode={:?}",
            self.cull.is_ready(),
            self.cached_uniforms.as_ref().map(|u| u.scroll_x),
            self.cached_uniforms.as_ref().map(|u| u.pixels_per_tick),
            self.cached_uniforms.as_ref().map(|u| u.mode),
        );
        let (key_lo, key_hi) = self.visible_key_range();
        let uniforms = self.cached_uniforms.unwrap_or_default();
        // 桌面间接绘制（零回读）vs Adreno 回读分支
        if self.cull.use_indirect() {
            // 单 encoder：compute cull → render pass，GPU 内 barrier，无 CPU 同步
            let mut enc = self
                .device
                .create_command_encoder(&CommandEncoderDescriptor::default());
            let _ = self
                .cull
                .dispatch_cull(&mut enc, &self.queue, key_lo, key_hi, &uniforms);
            let mut pass = crate::util::begin_pianoroll_pass(
                &mut enc,
                target,
                &self.render.pipeline,
                &self.render.bind_group,
                width,
                height,
            );
            self.draw_static_layers(&mut pass);
            self.cull.draw_visible_notes_indirect(
                &mut pass,
                &self.render.note_pipeline,
                &self.render.bind_group,
                &self.render.index_buffer,
                key_lo,
                key_hi,
            );
            let ghost = self.layers.iter().rfind(|l| l.kind() == LayerKind::Note);
            if let Some(ghost) = ghost {
                pass.set_pipeline(&self.render.note_direct_pipeline);
                ghost.draw(&mut pass, 0, Some(&self.render.index_buffer));
            }
            drop(pass);
            self.queue.submit([enc.finish()]);
            return;
        }
        // Adreno 分支：compute 单独提交后同步读回，再渲染
        let mut enc = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        let executed = self
            .cull
            .dispatch_cull(&mut enc, &self.queue, key_lo, key_hi, &uniforms);
        if executed {
            self.queue.submit([enc.finish()]);
            self.cull.readback_args_to_cpu(&self.device, &self.queue);
        }

        let mut enc2 = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        let mut pass = crate::util::begin_pianoroll_pass(
            &mut enc2,
            target,
            &self.render.pipeline,
            &self.render.bind_group,
            width,
            height,
        );

        self.draw_static_layers(&mut pass);

        self.cull.draw_visible_notes(
            &mut pass,
            &self.render.note_pipeline,
            &self.render.bind_group,
            &self.render.index_buffer,
            key_lo,
            key_hi,
        );

        let ghost = self.layers.iter().rfind(|l| l.kind() == LayerKind::Note);
        if let Some(ghost) = ghost {
            pass.set_pipeline(&self.render.note_direct_pipeline);
            ghost.draw(&mut pass, 0, Some(&self.render.index_buffer));
        }
        drop(pass);
        self.queue.submit([enc2.finish()]);
    }

    /// 当前 GPU 主题（从全局读取，随用户主题切换自动更新）。
    pub fn theme(&self) -> yinhe_theme::GpuTheme {
        yinhe_theme::current_gpu_theme()
    }
}
