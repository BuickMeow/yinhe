use wgpu::*;

use crate::pipeline::RenderPipelineState;
use crate::vertex::{NoteInstance, Uniforms};

/// Maximum instances per draw call.
const MAX_INSTANCE_COUNT: usize = 6_000_000;
/// Minimum instance buffer capacity (in number of instances).
const MIN_INSTANCE_BUFFER_CAPACITY: usize = 4096;

/// Per-frame timing breakdown returned by `prepare_with_static_cache`.
/// Durations are zero when the corresponding phase did not run.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrepareTimings {
    /// Whether any GPU-visible state was updated (uniforms or instances).
    pub dirty: bool,
    /// Whether the static instance cache was rebuilt this frame.
    pub static_rebuilt: bool,
    /// Time spent in the user-supplied `build_static` closure (zero if not run).
    pub build_static: std::time::Duration,
    /// Time spent in the user-supplied `build_cursor` closure.
    /// Kept for API stability; always zero now that the cursor is drawn by egui.
    pub build_cursor: std::time::Duration,
    /// Time spent uploading the instance buffer to the GPU (zero if not run).
    pub upload: std::time::Duration,
    /// Total instances uploaded.
    pub instance_count: usize,
}

/// Instance buffer with tracking of capacity and GPU bytes.
struct InstanceBufferSlot {
    buffer: Buffer,
    capacity_instances: usize,
    size_bytes: u64,
}

impl Drop for InstanceBufferSlot {
    fn drop(&mut self) {
        yinhe_memtrace::sub_gpu_resource(self.size_bytes);
    }
}

fn create_instance_buffer_slot(
    device: &Device,
    instance_size: u64,
    capacity: usize,
) -> InstanceBufferSlot {
    let size_bytes = instance_size * capacity as u64;
    let buffer = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
        device.create_buffer(&BufferDescriptor {
            label: Some("instance_buffer"),
            size: size_bytes,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    });
    yinhe_memtrace::add_gpu_resource(size_bytes);
    InstanceBufferSlot {
        buffer,
        capacity_instances: capacity,
        size_bytes,
    }
}

fn next_instance_capacity(required: usize) -> usize {
    let mut cap = MIN_INSTANCE_BUFFER_CAPACITY;
    while cap < required {
        cap *= 2;
    }
    cap
}

/// Generic wgpu renderer for instanced rectangle drawing.
///
/// Manages GPU buffers and provides `prepare_from_parts()` + `draw()` for
/// rendering NoteInstance data.  View-specific convenience methods (like
/// pianoroll's `prepare()`) belong in the calling crate.
pub struct PianorollRenderer {
    device: Device,
    queue: Queue,
    render: RenderPipelineState,
    instance_buffers: Vec<InstanceBufferSlot>,
    instance_scratch: Vec<NoteInstance>,
    current_batch_counts: Vec<usize>,
    /// Cached last uniforms — used to skip GPU write when nothing changed.
    cached_uniforms: Option<Uniforms>,
    /// CPU-side cache of static instances (background, grid, notes, keyboard).
    static_instance_cache: Vec<NoteInstance>,
    /// Viewport hash when the static cache was last built.
    cached_viewport_hash: Option<u64>,
}

impl PianorollRenderer {
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
                instance_buffers: Vec::new(),
                instance_scratch: Vec::new(),
                current_batch_counts: Vec::new(),
                cached_uniforms: None,
                static_instance_cache: Vec::new(),
                cached_viewport_hash: None,
            }
        })
    }

    /// Generic prepare with scratch buffer reuse and dirty-check.
    ///
    /// Skips rebuild when `dirty` is false and `uniforms` are unchanged.
    /// The `build` closure receives a mutable scratch buffer to populate.
    pub fn prepare_with_builder(
        &mut self,
        uniforms: Uniforms,
        dirty: bool,
        build: impl FnOnce(&mut Vec<NoteInstance>),
    ) {
        if !dirty
            && let Some(ref cached) = self.cached_uniforms
                && *cached == uniforms {
                    return;
                }

        let mut scratch = std::mem::take(&mut self.instance_scratch);
        scratch.clear();
        build(&mut scratch);
        self.prepare_from_parts(uniforms, &scratch);
        scratch.clear();
        self.instance_scratch = scratch;
    }

    /// Two-phase prepare: static instances (cached) + dynamic cursor (every frame).
    ///
    /// `viewport_hash` captures all view properties that affect static instances
    /// (scroll, zoom, window size). Static layer is rebuilt only when this hash
    /// changes — NOT on every frame during playback.
    ///
    /// `build_static` populates background, grid, notes, and keyboard instances.
    /// `build_cursor` appends the cursor line (O(1) work, called every frame).
    ///
    /// Returns `true` if any GPU data was actually updated (new uniforms or
    /// instances), `false` if everything was already up-to-date.
    /// Returns timing breakdown for perf instrumentation. Callers that don't
    /// care can ignore it (it's cheap to produce — a handful of Instant calls).
    /// Prepare static instances + uniforms with a viewport-keyed cache.
    ///
    /// `build_static` is only called when `viewport_hash` differs from the
    /// previously cached hash. Uniforms are written whenever they change.
    /// Instance upload to GPU is skipped entirely on a cache hit, so
    /// playing-but-stationary frames have zero GPU upload cost.
    ///
    /// The playback cursor is no longer part of the wgpu pipeline — it is
    /// drawn by egui on top of the rendered texture.
    pub fn prepare_with_static_cache(
        &mut self,
        uniforms: Uniforms,
        viewport_hash: u64,
        build_static: impl FnOnce(&mut Vec<NoteInstance>),
    ) -> PrepareTimings {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
        let need_static = self.cached_viewport_hash != Some(viewport_hash);
        let uniforms_changed = self
            .cached_uniforms
            .as_ref()
            .is_none_or(|c| *c != uniforms);

        let mut build_static_dur = std::time::Duration::ZERO;
        let mut upload_dur = std::time::Duration::ZERO;
        if need_static {
            let mut scratch = std::mem::take(&mut self.instance_scratch);
            scratch.clear();
            let t = std::time::Instant::now();
            build_static(&mut scratch);
            build_static_dur = t.elapsed();
            let t = std::time::Instant::now();
            self.upload_instances(&scratch);
            upload_dur = t.elapsed();
            self.static_instance_cache = std::mem::take(&mut scratch);
            self.instance_scratch = scratch;
            self.cached_viewport_hash = Some(viewport_hash);
        }

        if uniforms_changed {
            self.queue.write_buffer(
                &self.render.uniform_buffer,
                0,
                bytemuck::bytes_of(&uniforms),
            );
            self.cached_uniforms = Some(uniforms);
        }

        let instance_count = self.static_instance_cache.len();

        PrepareTimings {
            dirty: need_static || uniforms_changed,
            static_rebuilt: need_static,
            build_static: build_static_dur,
            build_cursor: std::time::Duration::ZERO,
            upload: upload_dur,
            instance_count,
        }
        })
    }

    /// Upload uniforms + instances to GPU.
    pub fn prepare_from_parts(&mut self, uniforms: Uniforms, instances: &[NoteInstance]) {
        self.cached_uniforms = Some(uniforms);
        self.queue.write_buffer(
            &self.render.uniform_buffer,
            0,
            bytemuck::bytes_of(&uniforms),
        );
        self.upload_instances(instances);
    }

    /// Upload instance data to GPU buffers (no uniform write).
    fn upload_instances(&mut self, instances: &[NoteInstance]) {
        let instance_size = std::mem::size_of::<NoteInstance>() as u64;
        let batches: Vec<&[NoteInstance]> = if instances.is_empty() {
            Vec::new()
        } else {
            instances.chunks(MAX_INSTANCE_COUNT).collect()
        };

        self.current_batch_counts.clear();
        for batch in &batches {
            self.current_batch_counts.push(batch.len());
        }

        while self.instance_buffers.len() > batches.len() {
            self.instance_buffers.pop();
        }
        while self.instance_buffers.len() < batches.len() {
            self.instance_buffers.push(create_instance_buffer_slot(
                &self.device,
                instance_size,
                MIN_INSTANCE_BUFFER_CAPACITY,
            ));
        }
        for (i, batch) in batches.iter().enumerate() {
            let required_instances = batch.len().max(1);
            if self.instance_buffers[i].capacity_instances < required_instances {
                self.instance_buffers[i] = create_instance_buffer_slot(
                    &self.device,
                    instance_size,
                    next_instance_capacity(required_instances),
                );
            }
            self.queue.write_buffer(
                &self.instance_buffers[i].buffer,
                0,
                bytemuck::cast_slice(batch),
            );
        }
    }

    /// Draw the prepared instances into the given render target.
    pub fn draw(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        width: u32,
        height: u32,
    ) {
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

        if !self.instance_buffers.is_empty() && !self.current_batch_counts.is_empty() {
            pass.set_pipeline(&self.render.pipeline);
            pass.set_bind_group(0, &self.render.bind_group, &[]);
            for (i, &count) in self.current_batch_counts.iter().enumerate() {
                pass.set_vertex_buffer(0, self.instance_buffers[i].buffer.slice(..));
                pass.draw(0..6, 0..count as u32);
            }
        }
    }

    /// Total number of note instances prepared for the current frame.
    pub fn total_instances(&self) -> usize {
        self.current_batch_counts.iter().sum()
    }

    /// Check whether given uniforms differ from the last prepared ones.
    pub fn uniforms_changed(&self, uniforms: &Uniforms) -> bool {
        self.cached_uniforms
            .as_ref()
            .is_none_or(|c| *c != *uniforms)
    }

    /// Get a reference to the cached uniforms (if any).
    pub fn cached_uniforms(&self) -> Option<&Uniforms> {
        self.cached_uniforms.as_ref()
    }
}
