use wgpu::*;

use crate::layer::LayerSlot;
use crate::pipeline::RenderPipelineState;
use crate::vertex::{CurveInstance, DrawInstance, NoteInstance, Uniforms, SelectionUniform, VelocityBarInstance};

/// Maximum visible note instances the cull output buffer can hold.
/// 8M instances × 16B = 128MB — the wgpu `max_storage_buffer_binding_size` limit.
/// Beyond this, `create_bind_group` will panic. If more than 8M notes are
/// visible simultaneously (extreme black-score at minimum zoom), the excess
/// is silently dropped by the cull shader.
const MAX_VISIBLE_NOTES: u64 = 8_000_000;

/// Per-frame timing breakdown returned by `prepare`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrepareTimings {
    /// Time spent in the user-supplied `build` closure.
    pub build_static: std::time::Duration,
    /// Total instances uploaded.
    pub instance_count: usize,
}

/// Layer kind: decor (32B `DrawInstance`), note (16B `NoteInstance`),
/// velocity (16B `VelocityBarInstance`), or curve (32B `CurveInstance` — automation SDF lines/curves).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    Decor,
    Note,
    Velocity,
    Curve,
}

/// Type-erased layer slot that can hold `DrawInstance`, `NoteInstance`, `VelocityBarInstance`, or `CurveInstance`.
pub enum AnyLayer {
    Decor(LayerSlot<DrawInstance>),
    Note(LayerSlot<NoteInstance>),
    Velocity(LayerSlot<VelocityBarInstance>),
    Curve(LayerSlot<CurveInstance>),
}

impl AnyLayer {
    fn new(device: &Device, kind: LayerKind) -> Self {
        match kind {
            LayerKind::Decor => AnyLayer::Decor(LayerSlot::new(device)),
            LayerKind::Note => AnyLayer::Note(LayerSlot::new(device)),
            LayerKind::Velocity => AnyLayer::Velocity(LayerSlot::new(device)),
            LayerKind::Curve => AnyLayer::Curve(LayerSlot::new(device)),
        }
    }

    fn kind(&self) -> LayerKind {
        match self {
            AnyLayer::Decor(_) => LayerKind::Decor,
            AnyLayer::Note(_) => LayerKind::Note,
            AnyLayer::Velocity(_) => LayerKind::Velocity,
            AnyLayer::Curve(_) => LayerKind::Curve,
        }
    }

    fn draw<'a>(&self, pass: &mut RenderPass<'a>, vertex_slot: u32) {
        match self {
            AnyLayer::Decor(l) => l.draw(pass, vertex_slot),
            AnyLayer::Note(l) => l.draw(pass, vertex_slot),
            AnyLayer::Velocity(l) => l.draw(pass, vertex_slot),
            AnyLayer::Curve(l) => l.draw(pass, vertex_slot),
        }
    }
}

/// GPU compute cull state: pipeline, per-key buffers, and shared output buffers.
///
/// Architecture: each MIDI key owns its own `all_notes` storage buffer + bind
/// group. The cull dispatch loops over keys, binding one buffer at a time.
/// This keeps every binding well under wgpu's `max_storage_buffer_binding_size`
/// (128MB) regardless of total note count - a single global buffer would
/// exceed the limit at ~8M notes and panic in `create_bind_group`.
struct CullState {
    pipeline: ComputePipeline,
    bind_group_layout: BindGroupLayout,
    /// Per-key bind groups (128 slots). `None` until the key is first uploaded.
    per_key_bind_groups: Vec<Option<BindGroup>>,
    /// Per-key all-notes storage buffers, grown on demand.
    per_key_buffers: Vec<Option<Buffer>>,
    /// Shared compacted visible-notes buffer (all keys append into this).
    visible_notes_buffer: Buffer,
    /// Shared DrawIndirectArgs buffer (`instance_count` accumulates across keys).
    indirect_args_buffer: Buffer,

    /// Per-key note count at last upload (in NoteInstance units).
    per_key_counts: [u32; 128],

    /// Per-key revision at last upload (full or incremental).
    /// Compared with model.note_revisions to detect incremental re-upload needs.
    uploaded_key_revisions: [u64; 128],
}

impl CullState {
    fn new(device: &Device) -> Self {
        let cull_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("cull_shader"),
            source: ShaderSource::Wgsl(include_str!("cull.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("cull_bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("cull_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &cull_shader,
            entry_point: Some("main"),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None,
        });

        let visible_size = MAX_VISIBLE_NOTES * std::mem::size_of::<NoteInstance>() as u64;
        let visible_notes_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("visible_notes"),
            size: visible_size,
            usage: BufferUsages::STORAGE | BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(visible_size);

        // DrawIndirectArgs: vertex_count(6) + instance_count + first_vertex(0) + first_instance(0) = 16 bytes
        // Plus 4 bytes padding for the _padding field in the shader struct = 20 bytes total.
        // But wgpu draw_indirect only reads 16 bytes (the standard DrawIndirectArgs).
        // We make it 20 to match the shader struct, but draw_indirect only reads first 16.
        let indirect_args_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("indirect_args"),
            size: 20,
            usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(20);

        Self {
            pipeline,
            bind_group_layout,
            per_key_bind_groups: (0..128).map(|_| None).collect(),
            per_key_buffers: (0..128).map(|_| None).collect(),
            visible_notes_buffer,
            indirect_args_buffer,
            per_key_counts: [0; 128],
            uploaded_key_revisions: [0; 128],
        }
    }

    /// Upload notes for all 128 keys. `notes` is a flat buffer; `per_key_offsets`
    /// slices it into per-key segments. Each key gets its own storage buffer
    /// (grown on demand) and bind group, keeping every binding under the
    /// `max_storage_buffer_binding_size` limit regardless of total note count.
    fn upload_all_notes(
        &mut self,
        device: &Device,
        queue: &Queue,
        uniform_buffer: &Buffer,
        notes: &[NoteInstance],
        per_key_offsets: &[u32; 129],
        key_revisions: &[u64; 128],
    ) {
        for key in 0u8..128 {
            let start = per_key_offsets[key as usize] as usize;
            let end = per_key_offsets[key as usize + 1] as usize;
            let key_notes = &notes[start..end];
            self.upload_one_key(device, queue, uniform_buffer, key, key_notes);
            self.uploaded_key_revisions[key as usize] = key_revisions[key as usize];
        }
    }

    /// Grow (if needed) + write + bind-group-recreate (if buffer grew) for one key.
    fn upload_one_key(
        &mut self,
        device: &Device,
        queue: &Queue,
        uniform_buffer: &Buffer,
        key: u8,
        notes: &[NoteInstance],
    ) {
        let needed = notes.len() as u64 * std::mem::size_of::<NoteInstance>() as u64;

        let need_recreate = match &self.per_key_buffers[key as usize] {
            None => true,
            Some(buf) => buf.size() < needed,
        };
        if need_recreate {
            if let Some(ref buf) = self.per_key_buffers[key as usize] {
                yinhe_memtrace::sub_gpu_resource(buf.size());
            }
            let size = needed.max(4096);
            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some("all_notes_key"),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            yinhe_memtrace::add_gpu_resource(size);
            self.per_key_buffers[key as usize] = Some(buffer);
            self.recreate_cull_bind_group(device, uniform_buffer, key);
        }

        if !notes.is_empty() {
            if let Some(ref buf) = self.per_key_buffers[key as usize] {
                queue.write_buffer(buf, 0, bytemuck::cast_slice(notes));
            }
        }
        self.per_key_counts[key as usize] = notes.len() as u32;
    }

    /// Recreate the bind group for a single key (after its buffer grew).
    fn recreate_cull_bind_group(&mut self, device: &Device, uniform_buffer: &Buffer, key: u8) {
        let all_buf = match &self.per_key_buffers[key as usize] {
            Some(b) => b.clone(),
            None => return,
        };
        self.per_key_bind_groups[key as usize] = Some(device.create_bind_group(&BindGroupDescriptor {
            label: Some("cull_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                BindGroupEntry { binding: 1, resource: all_buf.as_entire_binding() },
                BindGroupEntry { binding: 2, resource: self.visible_notes_buffer.as_entire_binding() },
                BindGroupEntry { binding: 3, resource: self.indirect_args_buffer.as_entire_binding() },
            ],
        }));
    }

    fn is_ready(&self) -> bool {
        self.per_key_bind_groups.iter().any(|bg| bg.is_some())
    }

    /// Reset indirect args once, then dispatch the cull pass per key.
    /// `instance_count` accumulates across all keys via atomicAdd in the shader.
    fn dispatch_cull(&self, queue: &Queue, encoder: &mut CommandEncoder) {
        // Reset indirect args: [vertex_count=6, instance_count=0, first_vertex=0, first_instance=0, pad=0]
        let reset_data: [u32; 5] = [6, 0, 0, 0, 0];
        queue.write_buffer(&self.indirect_args_buffer, 0, bytemuck::bytes_of(&reset_data));

        let mut cull_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("note_cull"),
            timestamp_writes: None,
        });
        cull_pass.set_pipeline(&self.pipeline);
        for key in 0u8..128 {
            let Some(bg) = &self.per_key_bind_groups[key as usize] else { continue };
            let count = self.per_key_counts[key as usize];
            if count == 0 { continue; }
            cull_pass.set_bind_group(0, bg, &[]);
            // 2D dispatch to support >65535 workgroups per key (extreme black-score
            // cases where one key holds >16.7M notes). Shader already indexes via
            // global_id.x + global_id.y * (65535*256).
            let wg = (count as u64).div_ceil(256);
            let wg_x = wg.min(65535) as u32;
            let wg_y = wg.div_ceil(65535) as u32;
            cull_pass.dispatch_workgroups(wg_x, wg_y, 1);
        }
        drop(cull_pass);
    }

    fn draw_visible_notes(&self, pass: &mut RenderPass<'_>, note_pipeline: &RenderPipeline, bind_group: &BindGroup) {
        pass.set_pipeline(note_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_vertex_buffer(0, self.visible_notes_buffer.slice(..));
        pass.draw_indirect(&self.indirect_args_buffer, 0);
    }
}

/// Generic wgpu renderer for instanced rectangle drawing.
///
/// Manages three pipelines sharing one uniform buffer:
///   - **decor pipeline** (32B `DrawInstance`, `vs_main`): decor, grid, keyboard, cursor
///   - **curve pipeline** (32B `CurveInstance`, `vs_main_curve`): automation SDF lines/curves
///   - **note pipeline** (16B `NoteInstance`, `vs_main_note`): PR notes, AR notes, ghost notes
///
/// With GPU compute cull enabled, notes are uploaded once to a persistent
/// buffer and culled on the GPU each frame instead of rebuilt on the CPU.
///
/// Layers are stored in z-order; `draw` switches pipelines as needed when
/// traversing layers.
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

    /// Upload a decor layer. Skips rebuild when `cache_key` matches the previous value.
    /// Pass `cache_key: 0` to force upload (always rebuilds).
    pub fn upload_layer(
        &mut self,
        index: usize,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<DrawInstance>),
    ) -> bool {
        self.ensure_layer(index, LayerKind::Decor);
        if let AnyLayer::Decor(slot) = &mut self.layers[index] {
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

    /// Upload a note layer. Skips rebuild when `cache_key` matches the previous value.
    /// Pass `cache_key: 0` to force upload (always rebuilds).
    pub fn upload_note_layer(
        &mut self,
        index: usize,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<NoteInstance>),
    ) -> bool {
        self.ensure_layer(index, LayerKind::Note);
        if let AnyLayer::Note(slot) = &mut self.layers[index] {
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

    /// Upload a curve layer (automation SDF lines/curves).
    /// Skips rebuild when `cache_key` matches the previous value.
    /// Pass `cache_key: 0` to force upload (always rebuilds, used for ghost layer).
    pub fn upload_curve_layer(
        &mut self,
        index: usize,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<CurveInstance>),
    ) -> bool {
        self.ensure_layer(index, LayerKind::Curve);
        if let AnyLayer::Curve(slot) = &mut self.layers[index] {
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

    /// Upload a velocity bar layer (automation panel velocity bars).
    /// Skips rebuild when `cache_key` matches the previous value.
    /// Pass `cache_key: 0` to force upload (always rebuilds).
    pub fn upload_velocity_layer(
        &mut self,
        index: usize,
        cache_key: u64,
        build: impl FnOnce(&mut Vec<VelocityBarInstance>),
    ) -> bool {
        self.ensure_layer(index, LayerKind::Velocity);
        if let AnyLayer::Velocity(slot) = &mut self.layers[index] {
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
        if self.cull.per_key_buffers[key as usize].is_none() {
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
        &self,
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

        // Step 1: all decor layers (background + grid)
        for layer in &self.layers {
            if layer.kind() == LayerKind::Decor {
                pass.set_pipeline(&self.render.pipeline);
                layer.draw(&mut pass, 0);
            }
        }

        // Step 2: all velocity bar layers
        for layer in &self.layers {
            if layer.kind() == LayerKind::Velocity {
                pass.set_pipeline(&self.render.velocity_pipeline);
                layer.draw(&mut pass, 0);
            }
        }

        // Step 3: all curve layers (automation SDF lines/curves)
        for layer in &self.layers {
            if layer.kind() == LayerKind::Curve {
                pass.set_pipeline(&self.render.curve_pipeline);
                layer.draw(&mut pass, 0);
            }
        }

        // Step 4: all note layers
        for layer in &self.layers {
            if layer.kind() == LayerKind::Note {
                pass.set_pipeline(&self.render.note_pipeline);
                layer.draw(&mut pass, 0);
            }
        }
    }

    /// GPU compute cull draw: dispatch cull pass, then draw layers.
    ///
    /// Z-order: decor (bg + grid) → velocity bars → curve (automation) → culled notes → ghost notes.
    fn draw_with_cull(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        // Phase 1: Compute cull
        self.cull.dispatch_cull(&self.queue, encoder);

        // Phase 2: Single render pass
        let mut pass = crate::util::begin_pianoroll_pass(
            encoder, target, &self.render.pipeline, &self.render.bind_group, width, height,
        );

        // Step 1: all decor layers (background + grid)
        for layer in &self.layers {
            if layer.kind() == LayerKind::Decor {
                pass.set_pipeline(&self.render.pipeline);
                layer.draw(&mut pass, 0);
            }
        }

        // Step 2: all velocity bar layers
        for layer in &self.layers {
            if layer.kind() == LayerKind::Velocity {
                pass.set_pipeline(&self.render.velocity_pipeline);
                layer.draw(&mut pass, 0);
            }
        }

        // Step 3: all curve layers (automation SDF lines/curves)
        for layer in &self.layers {
            if layer.kind() == LayerKind::Curve {
                pass.set_pipeline(&self.render.curve_pipeline);
                layer.draw(&mut pass, 0);
            }
        }

        // Step 4: culled notes (from GPU compute cull buffer)
        self.cull.draw_visible_notes(&mut pass, &self.render.note_pipeline, &self.render.bind_group);

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
