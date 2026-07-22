//! GPU compute cull state: per-key note buffers + indirect dispatch.
//!
//! Architecture: each MIDI key (0..127) owns its own `all_notes` (input),
//! `visible_notes` (output), and a slot in the shared `indirect_args` buffer.
//! The cull dispatch loops over keys; each key's visible capacity equals its
//! all-notes capacity, so there is no global visible-note cap.
//!
//! Memory: all_notes + visible_notes ≈ 2 × total notes × 16B (worst case:
//! minimum zoom, every note visible). H2O.mid (13.8M) ≈ 374MB; 100M ≈ 3.2GB.

use wgpu::*;

use crate::vertex::{NoteInstance, Uniforms};

/// Compare only the uniform fields that affect GPU culling (read by `cull.wgsl`).
/// Non-culling fields (scroll_frac, scroll_mode, track_count, sel_rect_count,
/// note_outline, value_zoom, value_scroll, min_border_width) are excluded so
/// that irrelevant changes don't trigger a re-cull.
fn culling_relevant_eq(a: &Uniforms, b: &Uniforms) -> bool {
    a.width == b.width
        && a.height == b.height
        && a.scroll_x == b.scroll_x
        && a.keyboard_width == b.keyboard_width
        && a.pixels_per_tick == b.pixels_per_tick
        && a.key_height == b.key_height
        && a.scroll_y == b.scroll_y
        && a.mode == b.mode
        && a.lane_height == b.lane_height
}

pub(crate) struct CullState {
    pipeline: ComputePipeline,
    bind_group_layout: BindGroupLayout,
    /// Reset pipeline: zeros all 128 indirect_args slots in one dispatch.
    /// Eliminates the CPU-side 32KB write_buffer per frame.
    reset_pipeline: ComputePipeline,
    reset_bind_group: BindGroup,
    /// Per-key bind groups (128 slots). `None` until the key is first uploaded.
    per_key_bind_groups: Vec<Option<BindGroup>>,
    /// Per-key all-notes storage buffers (cull input), grown on demand.
    per_key_buffers: Vec<Option<Buffer>>,
    /// Per-key visible-notes storage buffers (cull output + draw vertex source).
    /// Same size as the corresponding `per_key_buffers` slot (visible ≤ all).
    per_key_visible_buffers: Vec<Option<Buffer>>,
    /// Shared indirect-args buffer: 128 slots × 256 bytes (DrawIndirectArgs + pad).
    /// Slot k is at byte offset k * 256. 256-byte stride satisfies
    /// `min_storage_buffer_offset_alignment` (typically 256). Reset to
    /// [6,0,0,0,0] before each dispatch.
    indirect_args_buffer: Buffer,

    /// Per-key note count at last upload (in NoteInstance units).
    per_key_counts: [u32; 128],

    /// Per-key dispatch args buffer: 128 × DispatchIndirectArgs (12 bytes each).
    /// Pre-computed at upload time so `dispatch_cull` can use
    /// `dispatch_workgroups_indirect` instead of computing wg_x/wg_y per frame.
    /// Slot k is at byte offset k * 12.
    dispatch_args_buffer: Buffer,

    /// Per-key revision at last upload (full or incremental).
    /// Compared with model.note_revisions to detect incremental re-upload needs.
    pub(crate) uploaded_key_revisions: [u64; 128],

    /// Uniforms snapshot from the last cull dispatch. When the culling-relevant
    /// fields match and `notes_dirty` is false, the previous frame's
    /// `visible_notes` + `indirect_args` are still valid and the dispatch can
    /// be skipped entirely.
    last_cull_uniforms: Option<Uniforms>,

    /// True when note data has been uploaded (full or incremental) since the
    /// last cull dispatch. Set by `upload_all_notes` / `upload_one_key`;
    /// cleared by `dispatch_cull`.
    notes_dirty: bool,
}

impl CullState {
    pub(crate) fn new(device: &Device) -> Self {
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

        // DrawIndirectArgs: 128 slots × 256 bytes each.
        // Each slot: [vertex_count=6, instance_count=0, first_vertex=0, first_instance=0] (16 bytes)
        // + 240 bytes padding to satisfy min_storage_buffer_offset_alignment (typically 256).
        // Slot k is at byte offset k * 256. draw_indirect reads 16 bytes from a slot.
        let indirect_args_stride = 256;
        let indirect_args_size = 128 * indirect_args_stride;
        let indirect_args_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("indirect_args"),
            size: indirect_args_size,
            usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(indirect_args_size);

        // DispatchIndirectArgs: 128 × 12 bytes (x, y, z as u32).
        // Written at upload time; read by dispatch_workgroups_indirect.
        let dispatch_args_size = 128 * 12;
        let dispatch_args_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("cull_dispatch_args"),
            size: dispatch_args_size,
            usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(dispatch_args_size);

        // Reset pipeline: zeros all 128 indirect_args slots in one dispatch.
        // Uses a separate bind group layout that binds the full indirect_args
        // buffer as a flat array<u32> (the cull bind group only binds a 256-byte
        // slice per key).
        let reset_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("cull_reset_bind_group_layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let reset_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("cull_reset_pipeline_layout"),
            bind_group_layouts: &[Some(&reset_bind_group_layout)],
            immediate_size: 0,
        });
        let reset_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("cull_reset_pipeline"),
            layout: Some(&reset_pipeline_layout),
            module: &cull_shader,
            entry_point: Some("reset_indirect_args"),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None,
        });
        let reset_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("cull_reset_bind_group"),
            layout: &reset_bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: indirect_args_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            bind_group_layout,
            reset_pipeline,
            reset_bind_group,
            per_key_bind_groups: (0..128).map(|_| None).collect(),
            per_key_buffers: (0..128).map(|_| None).collect(),
            per_key_visible_buffers: (0..128).map(|_| None).collect(),
            indirect_args_buffer,
            per_key_counts: [0; 128],
            dispatch_args_buffer,
            uploaded_key_revisions: [0; 128],
            last_cull_uniforms: None,
            notes_dirty: false,
        }
    }

    /// Upload notes for all 128 keys. `notes` is a flat buffer; `per_key_offsets`
    /// slices it into per-key segments. Each key gets its own storage buffer
    /// (grown on demand) and bind group, keeping every binding under the
    /// `max_storage_buffer_binding_size` limit regardless of total note count.
    pub(crate) fn upload_all_notes(
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
        self.notes_dirty = true;
    }

    /// Grow (if needed) + write + bind-group-recreate (if buffer grew) for one key.
    /// Also grows the per-key visible buffer to match (visible ≤ all).
    pub(crate) fn upload_one_key(
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

            // Visible buffer matches all-notes size (worst case: all visible).
            if let Some(ref buf) = self.per_key_visible_buffers[key as usize] {
                yinhe_memtrace::sub_gpu_resource(buf.size());
            }
            let vis_buffer = device.create_buffer(&BufferDescriptor {
                label: Some("visible_notes_key"),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::VERTEX,
                mapped_at_creation: false,
            });
            yinhe_memtrace::add_gpu_resource(size);
            self.per_key_visible_buffers[key as usize] = Some(vis_buffer);

            self.recreate_cull_bind_group(device, uniform_buffer, key);
        }

        if !notes.is_empty() {
            if let Some(ref buf) = self.per_key_buffers[key as usize] {
                queue.write_buffer(buf, 0, bytemuck::cast_slice(notes));
            }
        }
        self.per_key_counts[key as usize] = notes.len() as u32;

        // Pre-compute dispatch args for this key (used by dispatch_workgroups_indirect).
        let wg = (notes.len() as u64).div_ceil(256);
        let args = [wg.min(65535) as u32, wg.div_ceil(65535) as u32, 1u32];
        queue.write_buffer(&self.dispatch_args_buffer, key as u64 * 12, bytemuck::cast_slice(&args));

        self.notes_dirty = true;
    }

    /// Whether a per-key buffer exists for `key` (incremental upload precondition).
    pub(crate) fn has_key_buffer(&self, key: u8) -> bool {
        self.per_key_buffers[key as usize].is_some()
    }

    /// Recreate the bind group for a single key (after its buffer grew).
    /// Binds: uniform, all_notes[k], visible_notes[k], indirect_args slot k
    /// (256-byte slice at offset k*256 — no dynamic offset needed since each
    /// key has its own bind group).
    fn recreate_cull_bind_group(&mut self, device: &Device, uniform_buffer: &Buffer, key: u8) {
        let all_buf = match &self.per_key_buffers[key as usize] {
            Some(b) => b.clone(),
            None => return,
        };
        let vis_buf = match &self.per_key_visible_buffers[key as usize] {
            Some(b) => b.clone(),
            None => return,
        };
        let slot_offset = key as u64 * 256;
        let slot_size = 256u64;
        self.per_key_bind_groups[key as usize] = Some(device.create_bind_group(&BindGroupDescriptor {
            label: Some("cull_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                BindGroupEntry { binding: 1, resource: all_buf.as_entire_binding() },
                BindGroupEntry { binding: 2, resource: vis_buf.as_entire_binding() },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &self.indirect_args_buffer,
                        offset: slot_offset,
                        size: Some(std::num::NonZeroU64::new(slot_size).unwrap()),
                    }),
                },
            ],
        }));
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.per_key_bind_groups.iter().any(|bg| bg.is_some())
    }

    /// Reset all 128 indirect-args slots, then dispatch the cull pass per key.
    /// Each key writes into its own visible buffer + indirect-args slot.
    ///
    /// Only keys in `key_lo..=key_hi` are dispatched — off-screen keys would
    /// produce zero visible instances anyway, so skipping them saves both CPU
    /// dispatch overhead and GPU compute work.
    ///
    /// **Skip optimization**: if no notes were re-uploaded (`!notes_dirty`) and
    /// the culling-relevant uniform fields match the last dispatch, the previous
    /// frame's `visible_notes` + `indirect_args` are still valid and the entire
    /// reset + dispatch is skipped. This makes idle frames (no scroll, no edit)
    /// cost zero GPU compute work.
    pub(crate) fn dispatch_cull(
        &mut self,
        encoder: &mut CommandEncoder,
        key_lo: u8,
        key_hi: u8,
        uniforms: &Uniforms,
    ) {
        // Skip if nothing changed since last cull.
        if !self.notes_dirty
            && self.last_cull_uniforms.as_ref().is_some_and(|last| culling_relevant_eq(last, uniforms))
        {
            return;
        }

        let mut cull_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("note_cull"),
            timestamp_writes: None,
        });

        // Phase 0: GPU-side reset of all 128 indirect_args slots.
        // One dispatch of 128 threads; thread k writes [6,0,0,0] to slot k.
        // Replaces the old CPU-side 32KB write_buffer.
        cull_pass.set_pipeline(&self.reset_pipeline);
        cull_pass.set_bind_group(0, &self.reset_bind_group, &[]);
        cull_pass.dispatch_workgroups(1, 1, 1);

        // Phase 1: per-key cull dispatches.
        cull_pass.set_pipeline(&self.pipeline);
        for key in key_lo..=key_hi {
            let Some(bg) = &self.per_key_bind_groups[key as usize] else { continue };
            let count = self.per_key_counts[key as usize];
            if count == 0 { continue; }
            // Each key's bind group already binds its own indirect_args slot
            // (256-byte slice at offset k*256), so no dynamic offset is needed.
            cull_pass.set_bind_group(0, bg, &[]);
            // Dispatch args (wg_x, wg_y, 1) were pre-computed at upload time
            // and stored in dispatch_args_buffer. This avoids per-frame div_ceil.
            cull_pass.dispatch_workgroups_indirect(&self.dispatch_args_buffer, key as u64 * 12);
        }
        drop(cull_pass);

        self.last_cull_uniforms = Some(*uniforms);
        self.notes_dirty = false;
    }

    pub(crate) fn draw_visible_notes(&self, pass: &mut RenderPass<'_>, note_pipeline: &RenderPipeline, bind_group: &BindGroup, key_lo: u8, key_hi: u8) {
        pass.set_pipeline(note_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        // Draw each key's visible notes via its own indirect-args slot.
        for key in key_lo..=key_hi {
            let Some(vis_buf) = &self.per_key_visible_buffers[key as usize] else { continue };
            let count = self.per_key_counts[key as usize];
            if count == 0 { continue; }
            pass.set_vertex_buffer(0, vis_buf.slice(..));
            pass.draw_indirect(&self.indirect_args_buffer, key as u64 * 256);
        }
    }
}
