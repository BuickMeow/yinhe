use wgpu::*;

use crate::layer::LayerSlot;
use crate::pipeline::RenderPipelineState;
use crate::vertex::{DrawInstance, NoteInstance, Uniforms, TrackColorsUniform, SelectionUniform};

/// Maximum visible note instances the cull output buffer can hold.
/// 1M instances × 16B = 16MB — enough for any screen at any zoom.
const MAX_VISIBLE_NOTES: u64 = 1_000_000;

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

/// GPU compute cull state: pipeline, buffers, and bind group for note culling.
struct CullState {
    pipeline: ComputePipeline,
    bind_group_layout: BindGroupLayout,
    bind_group: Option<BindGroup>,
    all_notes_buffer: Option<Buffer>,
    all_notes_count: u32,
    visible_notes_buffer: Buffer,
    indirect_args_buffer: Buffer,
    cull_info_buffer: Buffer,
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
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
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
                BindGroupLayoutEntry {
                    binding: 4,
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

        let cull_info_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("cull_info"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(16);

        Self {
            pipeline,
            bind_group_layout,
            bind_group: None,
            all_notes_buffer: None,
            all_notes_count: 0,
            visible_notes_buffer,
            indirect_args_buffer,
            cull_info_buffer,
        }
    }

    fn upload_all_notes(&mut self, device: &Device, queue: &Queue, notes: &[NoteInstance]) {
        let needed = notes.len() as u64 * std::mem::size_of::<NoteInstance>() as u64;

        let recreate = match &self.all_notes_buffer {
            None => true,
            Some(buf) => buf.size() < needed,
        };

        if recreate {
            if let Some(ref buf) = self.all_notes_buffer {
                yinhe_memtrace::sub_gpu_resource(buf.size());
            }
            let size = needed.max(4096);
            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some("all_notes"),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            yinhe_memtrace::add_gpu_resource(size);
            self.all_notes_buffer = Some(buffer);
        }

        if !notes.is_empty() {
            if let Some(ref buf) = self.all_notes_buffer {
                queue.write_buffer(buf, 0, bytemuck::cast_slice(notes));
            }
        }

        self.all_notes_count = notes.len() as u32;
        queue.write_buffer(
            &self.cull_info_buffer, 0,
            bytemuck::bytes_of(&[self.all_notes_count, 0u32, 0u32, 0u32]),
        );

        // Recreate bind group with correct uniform buffer from render pipeline
        // (passed via a separate method since we don't own it here)
        self.recreate_bind_group(device);
    }

    /// Recreate bind group. Must be called after upload_all_notes and
    /// after the render pipeline's uniform buffer is available.
    fn recreate_bind_group(&mut self, device: &Device) {
        let all_buf = match &self.all_notes_buffer {
            Some(b) => b,
            None => return,
        };

        self.bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
            label: Some("cull_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                // binding 0: Uniforms — will be overridden per-frame via a separate bind group
                BindGroupEntry { binding: 0, resource: all_buf.as_entire_binding() },
                BindGroupEntry { binding: 1, resource: self.cull_info_buffer.as_entire_binding() },
                BindGroupEntry { binding: 2, resource: all_buf.as_entire_binding() },
                BindGroupEntry { binding: 3, resource: self.visible_notes_buffer.as_entire_binding() },
                BindGroupEntry { binding: 4, resource: self.indirect_args_buffer.as_entire_binding() },
            ],
        }));
    }

    fn is_ready(&self) -> bool {
        self.bind_group.is_some() && self.all_notes_count > 0
    }

    /// Reset indirect args and dispatch the compute cull pass.
    fn dispatch_cull(&self, queue: &Queue, encoder: &mut CommandEncoder, _uniform_buffer: &Buffer) {
        // Reset indirect args: [vertex_count=6, instance_count=0, first_vertex=0, first_instance=0, pad=0]
        let reset_data: [u32; 5] = [6, 0, 0, 0, 0];
        queue.write_buffer(&self.indirect_args_buffer, 0, bytemuck::bytes_of(&reset_data));

        // Create a per-frame bind group that uses the current uniform buffer
        // (binding 0 must point to the render pipeline's uniform buffer)
        // We can't create this here since we don't have the device.
        // Instead, we rely on the bind group created in recreate_bind_group_with_uniforms.
        let bg = self.bind_group.as_ref().unwrap();

        let mut cull_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("note_cull"),
            timestamp_writes: None,
        });
        cull_pass.set_pipeline(&self.pipeline);
        cull_pass.set_bind_group(0, bg, &[]);
        let wg = (self.all_notes_count + 255) / 256;
        cull_pass.dispatch_workgroups(wg, 1, 1);
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
/// Manages two pipelines sharing one uniform buffer:
///   - **decor pipeline** (32B `DrawInstance`, `vs_main`): decor, grid, keyboard, cursor, automation
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
    cached_track_colors: Option<Vec<u8>>,
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

    /// Upload ALL note instances to the persistent GPU buffer for compute cull.
    /// Call this once on MIDI load/change, NOT every frame.
    pub fn upload_all_notes_for_cull(&mut self, notes: &[NoteInstance]) {
        self.cull.upload_all_notes(&self.device, &self.queue, notes);
        // Recreate bind group with the render pipeline's uniform buffer
        if self.cull.all_notes_buffer.is_some() {
            self.recreate_cull_bind_group();
        }
    }

    /// Recreate the cull bind group so that binding 0 points to the
    /// render pipeline's uniform buffer (which gets updated every frame).
    fn recreate_cull_bind_group(&mut self) {
        let all_buf = match &self.cull.all_notes_buffer {
            Some(b) => b.clone(),
            None => return,
        };
        self.cull.bind_group = Some(self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("cull_bind_group"),
            layout: &self.cull.bind_group_layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: self.render.uniform_buffer.as_entire_binding() },
                BindGroupEntry { binding: 1, resource: self.cull.cull_info_buffer.as_entire_binding() },
                BindGroupEntry { binding: 2, resource: all_buf.as_entire_binding() },
                BindGroupEntry { binding: 3, resource: self.cull.visible_notes_buffer.as_entire_binding() },
                BindGroupEntry { binding: 4, resource: self.cull.indirect_args_buffer.as_entire_binding() },
            ],
        }));
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

    /// Legacy draw: iterate all layers, switch pipelines per layer kind.
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

    /// GPU compute cull draw: dispatch cull pass, then draw layers in correct Z-order:
    /// background + grid → culled notes → keyboard → ghost notes.
    fn draw_with_cull(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
        // Phase 1: Compute cull
        self.cull.dispatch_cull(&self.queue, encoder, &self.render.uniform_buffer);

        // Phase 2: Single render pass, multiple layers with pipeline switches
        let mut pass = crate::util::begin_pianoroll_pass(
            encoder, target, &self.render.pipeline, &self.render.bind_group, width, height,
        );

        // Layout: 0=decor, 1=grid, 2=notes(skip), 3=keyboard, 4=ghost
        // Z-order: decor + grid → culled notes → keyboard → ghost notes
        let decor_layers: Vec<_> = self.layers.iter().filter(|l| l.kind() == LayerKind::Decor).collect();
        let note_layers: Vec<_> = self.layers.iter().filter(|l| l.kind() == LayerKind::Note).collect();

        // Step 1: background + grid (first 2 decor layers)
        for (i, layer) in decor_layers.iter().enumerate() {
            if i < 2 {
                pass.set_pipeline(&self.render.pipeline);
                layer.draw(&mut pass, 0);
            }
        }

        // Step 2: culled notes
        self.cull.draw_visible_notes(&mut pass, &self.render.note_pipeline, &self.render.bind_group);

        // Step 3: keyboard (3rd decor layer, if any) — on top of notes
        if decor_layers.len() >= 3 {
            pass.set_pipeline(&self.render.pipeline);
            decor_layers[2].draw(&mut pass, 0);
        }

        // Step 4: ghost notes (note layer, if any)
        for layer in &note_layers {
            pass.set_pipeline(&self.render.note_pipeline);
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
