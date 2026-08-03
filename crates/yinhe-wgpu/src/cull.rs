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

/// Per-key tick-bucket index over the key's notes (sorted by start_tick).
///
/// Buckets are fixed-size (NOTES_PER_BUCKET notes each). Chunk c of a key
/// covers notes [c*256, min((c+1)*256, count)) — contiguous, so the shader
/// computes its input range directly from (c_lo, chunk id) without any GPU
/// lookup table.
///
/// A bucket can intersect the viewport tick range [ts, te] iff
/// bucket_start[b] <= te AND max_end[b] >= ts. max_end is not monotonic, so
/// we store its suffix max (monotonic non-increasing), making both bounds
/// binary-searchable:
///   - b_lo = first bucket with suffix_max_end >= ts
///   - b_hi = last bucket with bucket_start <= te
#[derive(Clone)]
struct KeyBucketIndex {
    /// start_tick of each bucket's first note (monotonic non-decreasing).
    bucket_start: Vec<u32>,
    /// max over buckets [b..] of per-bucket max end_tick (monotonic
    /// non-increasing). Used to find the first bucket that can intersect.
    bucket_suffix_max_end: Vec<u32>,
    /// Total chunk count = ceil(note_count / 256).
    chunk_total: u32,
}

/// Notes per bucket; each bucket spans NOTES_PER_BUCKET / 256 = 16 chunks.
const NOTES_PER_BUCKET: usize = 4096;

impl KeyBucketIndex {
    fn build(notes: &[NoteInstance]) -> Self {
        let mut bucket_start = Vec::new();
        let mut bucket_max_end = Vec::new();
        let mut i = 0;
        while i < notes.len() {
            let end = (i + NOTES_PER_BUCKET).min(notes.len());
            bucket_start.push(notes[i].start_tick);
            let mut max_end = notes[i].end_tick;
            for n in &notes[i..end] {
                max_end = max_end.max(n.end_tick);
            }
            bucket_max_end.push(max_end);
            i = end;
        }
        let mut suffix_max = Vec::with_capacity(bucket_max_end.len());
        let mut cur = 0;
        for &m in bucket_max_end.iter().rev() {
            cur = cur.max(m);
            suffix_max.push(cur);
        }
        suffix_max.reverse();
        KeyBucketIndex {
            bucket_start,
            bucket_suffix_max_end: suffix_max,
            chunk_total: notes.len().div_ceil(256) as u32,
        }
    }

    /// Chunk range [c_lo, c_lo + chunk_count) that can intersect
    /// [tick_start, tick_end]. Conservative: may include buckets that the
    /// shader's exact AABB test then culls. Returns None when nothing can.
    fn visible_chunk_range(&self, tick_start: u32, tick_end: u32) -> Option<(u32, u32)> {
        if self.chunk_total == 0 || tick_start > tick_end {
            return None;
        }
        let b_lo = self
            .bucket_suffix_max_end
            .partition_point(|&m| m < tick_start);
        if b_lo >= self.bucket_start.len() {
            return None;
        }
        // b_hi + 1 = first bucket whose start_tick > tick_end.
        let b_hi_end = self.bucket_start.partition_point(|&s| s <= tick_end);
        if b_hi_end == 0 || b_hi_end <= b_lo {
            return None;
        }
        let b_hi = b_hi_end - 1;
        let chunks_per_bucket = NOTES_PER_BUCKET / 256;
        let c_lo = b_lo * chunks_per_bucket;
        let c_hi_end = ((b_hi + 1) * chunks_per_bucket).min(self.chunk_total as usize);
        Some((c_lo as u32, (c_hi_end - c_lo) as u32))
    }
}

/// Viewport tick range the cull shader may consider visible, computed with
/// the same f32 math as `cull.wgsl` (x_offset + tick * ppu) plus a margin of
/// one pixel + 2 ticks, so f32 rounding near the viewport edges never drops
/// a bucket that the exact shader test could still pass.
fn visible_tick_range(uniforms: &Uniforms) -> (u32, u32) {
    let ppu = uniforms.pixels_per_tick;
    let x_offset = uniforms.keyboard_width - uniforms.scroll_x;
    let pad = (1.0 / ppu).ceil() as i64 + 2;
    let ts = (((-x_offset) / ppu).floor() as i64 - pad)
        .max(0)
        .min(u32::MAX as i64);
    let te = (((uniforms.width - x_offset) / ppu).ceil() as i64 + pad)
        .max(0)
        .min(u32::MAX as i64);
    (ts as u32, te as u32)
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

    /// Per-key tick-bucket index (CPU side), built at upload time. Used each
    /// frame to dispatch only the chunks whose tick range can intersect the
    /// viewport — GPU traffic per frame drops from O(all notes) to O(visible).
    bucket_indexes: Vec<Option<KeyBucketIndex>>,

    /// Per-key dispatch args buffer: 128 slots × 256 bytes each.
    /// Slot k is at byte offset k * 256 (satisfies
    /// `min_storage_buffer_offset_alignment`, typically 256):
    ///   - [0..12)  `DispatchIndirectArgs` (wg_x, wg_y, wg_z=1), host-written
    ///     every frame (wg_x = chunk_count, wg_y = ceil(chunk_count/65535)).
    ///   - [12..16) note count at last upload (u32), read by `cull.wgsl` as the
    ///     cull bound instead of `arrayLength` — the buffer capacity can exceed
    ///     the written count (grown buffers, shrunk keys), and the tail holds
    ///     stale/uninitialized data that would be culled as ghost notes.
    ///   - [16..20) c_lo (first dispatched chunk of the frame, host-written
    ///     every frame).
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
                BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
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
        // COPY_SRC so tests can read back the slot contents after a dispatch.
        let indirect_args_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("indirect_args"),
            size: indirect_args_size,
            usage: BufferUsages::STORAGE
                | BufferUsages::INDIRECT
                | BufferUsages::COPY_DST
                | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        yinhe_memtrace::add_gpu_resource(indirect_args_size);

        // Dispatch args + per-key note count + c_lo, 128 slots × 256 bytes each.
        // Slot layout: [wg_x, wg_y, wg_z, count, c_lo] (20 bytes) + padding.
        // The 256-byte stride satisfies min_storage_buffer_offset_alignment
        // (typically 256). wg_x/wg_y/c_lo are host-written every frame by
        // `dispatch_cull`; `count` bounds the cull scan in cull.wgsl (index < count).
        let dispatch_args_size = 128 * 256;
        let dispatch_args_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("cull_dispatch_args"),
            size: dispatch_args_size,
            usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
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
            bucket_indexes: (0..128).map(|_| None).collect(),
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

        if !notes.is_empty()
            && let Some(ref buf) = self.per_key_buffers[key as usize]
        {
            queue.write_buffer(buf, 0, bytemuck::cast_slice(notes));
        }
        self.per_key_counts[key as usize] = notes.len() as u32;

        // Rebuild the tick-bucket index for this key (notes are sorted by
        // start_tick). Rebuilt on every upload, including shrunk keys.
        self.bucket_indexes[key as usize] = Some(KeyBucketIndex::build(notes));

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
        self.per_key_bind_groups[key as usize] =
            Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("cull_bind_group"),
                layout: &self.bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: all_buf.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: vis_buf.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &self.indirect_args_buffer,
                            offset: slot_offset,
                            size: std::num::NonZeroU64::new(slot_size),
                        }),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &self.dispatch_args_buffer,
                            offset: key as u64 * 256,
                            size: std::num::NonZeroU64::new(256),
                        }),
                    },
                ],
            }));
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.per_key_bind_groups.iter().any(|bg| bg.is_some())
    }

    /// Drop all per-key note buffers / bind groups and reset tracking state so
    /// the next upload is forced down the full-upload path (cull_ready → false).
    ///
    /// Called when the active document changes (close / switch / new) to avoid
    /// stale note data leaking from the previous document into the next render.
    /// The shared `indirect_args_buffer` / `dispatch_args_buffer` are reused
    /// (they'll be overwritten on the next `dispatch_cull`).
    pub(crate) fn clear_cull(&mut self) {
        for buf in self
            .per_key_buffers
            .iter_mut()
            .chain(self.per_key_visible_buffers.iter_mut())
        {
            if let Some(b) = buf.take() {
                yinhe_memtrace::sub_gpu_resource(b.size());
            }
        }
        self.per_key_bind_groups.fill(None);
        self.per_key_counts.fill(0);
        self.bucket_indexes.fill(None);
        self.uploaded_key_revisions.fill(0);
        self.last_cull_uniforms = None;
        self.notes_dirty = false;
    }

    /// Reset all 128 indirect-args slots, then dispatch the cull pass per key.
    /// Each key writes into its own visible buffer + indirect-args slot.
    ///
    /// Only keys in `key_lo..=key_hi` are dispatched — off-screen keys would
    /// produce zero visible instances anyway, so skipping them saves both CPU
    /// dispatch overhead and GPU compute work.
    ///
    /// **Tick-bucket culling**: per key, the viewport tick range (with a
    /// conservative margin) is binary-searched against the key's bucket index
    /// to find the chunk range [c_lo, c_lo + chunk_count) that can possibly be
    /// visible. Only those chunks are dispatched; wg_x/wg_y/c_lo are written
    /// into the dispatch-args slot every frame (one 32KB blob for all 128
    /// keys). `count` stays cached on the CPU.
    ///
    /// **Skip optimization**: if no notes were re-uploaded (`!notes_dirty`) and
    /// the culling-relevant uniform fields match the last dispatch, the previous
    /// frame's `visible_notes` + `indirect_args` are still valid and the entire
    /// reset + dispatch is skipped. This makes idle frames (no scroll, no edit)
    /// cost zero GPU compute work.
    pub(crate) fn dispatch_cull(
        &mut self,
        encoder: &mut CommandEncoder,
        queue: &Queue,
        key_lo: u8,
        key_hi: u8,
        uniforms: &Uniforms,
    ) {
        // Skip if nothing changed since last cull.
        if !self.notes_dirty
            && self
                .last_cull_uniforms
                .as_ref()
                .is_some_and(|last| culling_relevant_eq(last, uniforms))
        {
            return;
        }

        // Viewport tick range, computed with the same f32 math as the shader
        // plus a small margin so the bucket query stays conservative (extra
        // buckets are harmless — the shader AABB test is exact).
        let (tick_start, tick_end) = visible_tick_range(uniforms);

        // Per-key dispatch info: (wg_x, wg_y, wg_z=1, count, c_lo). wg_x/wg_y
        // and c_lo change every frame; count is cached on the CPU. Written as
        // one 32KB blob per frame instead of 128 small writes.
        let mut info = [0u32; 128 * 64];
        for key in 0..128 {
            let slot = key * 64;
            let (c_lo, chunk_count) = self.bucket_indexes[key]
                .as_ref()
                .and_then(|idx| idx.visible_chunk_range(tick_start, tick_end))
                .unwrap_or((0, 0));
            info[slot] = chunk_count.min(65535);
            info[slot + 1] = chunk_count.div_ceil(65535);
            info[slot + 2] = 1;
            info[slot + 3] = self.per_key_counts[key];
            info[slot + 4] = c_lo;
        }
        queue.write_buffer(&self.dispatch_args_buffer, 0, bytemuck::cast_slice(&info));

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

        // Phase 1: per-key cull dispatches. Only keys with visible chunks are
        // dispatched (info[slot] == 0 means chunk_count == 0 → skip).
        cull_pass.set_pipeline(&self.pipeline);
        for key in key_lo..=key_hi {
            let Some(bg) = &self.per_key_bind_groups[key as usize] else {
                continue;
            };
            if info[key as usize * 64] == 0 {
                continue;
            }
            // Each key's bind group already binds its own indirect_args slot
            // (256-byte slice at offset k*256), so no dynamic offset is needed.
            cull_pass.set_bind_group(0, bg, &[]);
            // Dispatch args (wg_x, wg_y, 1, count, c_lo) were written above as
            // one 32KB blob; the GPU reads them via dispatch_workgroups_indirect.
            cull_pass.dispatch_workgroups_indirect(&self.dispatch_args_buffer, key as u64 * 256);
        }
        drop(cull_pass);

        self.last_cull_uniforms = Some(*uniforms);
        self.notes_dirty = false;
    }

    pub(crate) fn draw_visible_notes(
        &self,
        pass: &mut RenderPass<'_>,
        note_pipeline: &RenderPipeline,
        bind_group: &BindGroup,
        key_lo: u8,
        key_hi: u8,
    ) {
        pass.set_pipeline(note_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        // Draw each key's visible notes via its own indirect-args slot.
        for key in key_lo..=key_hi {
            let Some(vis_buf) = &self.per_key_visible_buffers[key as usize] else {
                continue;
            };
            let count = self.per_key_counts[key as usize];
            if count == 0 {
                continue;
            }
            pass.set_vertex_buffer(0, vis_buf.slice(..));
            pass.draw_indirect(&self.indirect_args_buffer, key as u64 * 256);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertex::NoteInstance;
    use std::sync::atomic::Ordering;

    /// Headless GPU device for cull integration tests.
    /// Returns None when no adapter is available (e.g. CI without a GPU),
    /// which skips the test.
    fn headless_device() -> Option<(Device, Queue)> {
        let instance = Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&DeviceDescriptor::default())).ok()?;
        Some((device, queue))
    }

    fn visible_uniforms() -> Uniforms {
        Uniforms {
            width: 800.0,
            height: 600.0,
            scroll_x: 0.0,
            scroll_y: 1000.0, // key 60 rows land inside the viewport (y ∈ [340, 360))
            pixels_per_tick: 0.1,
            key_height: 20.0,
            keyboard_width: 60.0,
            mode: 1,
            ..Default::default()
        }
    }

    fn test_notes(n: usize) -> Vec<NoteInstance> {
        (0..n)
            .map(|i| NoteInstance {
                start_tick: i as u32 * 10,
                end_tick: i as u32 * 10 + 5,
                packed: NoteInstance::pack(60, 0, 100),
                reserved: 0,
            })
            .collect()
    }

    /// Regression test for the ghost-note bug: a key whose buffer capacity
    /// exceeds its written note count must NOT cull stale data beyond `count`.
    ///
    /// Scenario: upload 100 notes (buffer capacity rounds up to 256 elements),
    /// then upload 50 notes (buffer is not recreated, so elements 50..255 still
    /// hold the first upload's notes). If the shader used `arrayLength`
    /// (capacity) as the cull bound, the stale notes at 50..99 would pass the
    /// AABB test and be drawn as ghosts (instance_count would be ≥ 100).
    #[test]
    fn cull_ignores_stale_notes_beyond_count() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut cull = CullState::new(&device);
        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("test_uniform"),
            size: 256,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        cull.upload_one_key(&device, &queue, &uniform_buffer, 0, &test_notes(100));
        // Shrunk upload: same key, fewer notes, buffer NOT recreated.
        cull.upload_one_key(&device, &queue, &uniform_buffer, 0, &test_notes(50));

        let mut encoder = device.create_command_encoder(&Default::default());
        cull.dispatch_cull(&mut encoder, &queue, 0, 0, &visible_uniforms());

        // Read back indirect_args slot 0 (instance_count at byte offset 4).
        let args_readback = device.create_buffer(&BufferDescriptor {
            label: Some("args_readback"),
            size: 256,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&cull.indirect_args_buffer, 0, &args_readback, 0, 256);
        queue.submit([encoder.finish()]);

        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        args_readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst), "map_async callback not fired");
        let view = args_readback.slice(..).get_mapped_range();
        let args: &[u32] = bytemuck::cast_slice(&view);
        let instance_count = args[1]; // DrawIndirectArgs.instance_count
        drop(view);
        args_readback.unmap();

        assert_eq!(
            instance_count, 50,
            "stale notes beyond the uploaded count must not be culled"
        );
    }

    fn build_index(start_ends: &[(u32, u32)]) -> KeyBucketIndex {
        let notes: Vec<NoteInstance> = start_ends
            .iter()
            .map(|&(s, e)| NoteInstance {
                start_tick: s,
                end_tick: e,
                packed: NoteInstance::pack(60, 0, 100),
                reserved: 0,
            })
            .collect();
        KeyBucketIndex::build(&notes)
    }

    #[test]
    fn bucket_index_empty_and_small() {
        let idx = build_index(&[]);
        assert_eq!(idx.chunk_total, 0);
        assert!(idx.visible_chunk_range(0, 1000).is_none());

        // 100 notes → 1 bucket, 1 chunk.
        let notes: Vec<(u32, u32)> = (0..100).map(|i| (i * 10, i * 10 + 5)).collect();
        let idx = build_index(&notes);
        assert_eq!(idx.chunk_total, 1);
        assert_eq!(idx.visible_chunk_range(0, 1000), Some((0, 1)));
        // Viewport after all notes → nothing.
        assert!(idx.visible_chunk_range(2000, 3000).is_none());
        // Single bucket fully left of the viewport → suffix max < ts → None.
        let left_notes: Vec<(u32, u32)> = (0..100).map(|i| (i, i + 5)).collect();
        let idx = build_index(&left_notes);
        assert!(idx.visible_chunk_range(500, 600).is_none());
    }

    #[test]
    fn bucket_index_multi_bucket_boundaries() {
        // 5000 notes → 2 buckets (4096 + 904), 20 chunks (16 + 4).
        let notes: Vec<(u32, u32)> = (0..5000).map(|i| (i * 10, i * 10 + 5)).collect();
        let idx = build_index(&notes);
        assert_eq!(idx.chunk_total, 20);
        // Viewport inside bucket 0 → chunks [0, 16).
        assert_eq!(idx.visible_chunk_range(0, 100), Some((0, 16)));
        // Viewport inside bucket 1's tick range: bucket 1's max_end (49995) is
        // part of bucket 0's suffix max (49995 ≥ ts), so bucket 0 is
        // conservatively included → [0, 20). The shader's exact AABB test then
        // culls bucket 0's notes. (suffix max is monotonic non-increasing, so
        // b_lo is always 0 or len — it can only reject a viewport entirely
        // past the last note end.)
        assert_eq!(idx.visible_chunk_range(40_000, 50_000), Some((0, 20)));
        // Viewport spanning both buckets → [0, 20).
        assert_eq!(idx.visible_chunk_range(0, 50_000), Some((0, 20)));
        // Gap between the buckets' start ticks (bucket 1 starts at tick 40960):
        // bucket 0's max_end (40955) is below the viewport, but the suffix max
        // is conservative (bucket 1's max_end = 49995 ≥ ts), so b_lo stays 0
        // and bucket 0 is dispatched too — the shader's exact AABB test then
        // culls bucket 0's notes. Conservative inclusion is by design.
        assert_eq!(idx.visible_chunk_range(41_000, 42_000), Some((0, 20)));
    }

    #[test]
    fn bucket_index_long_note_crossing_left_edge() {
        // A long note starting far left extends far right: bucket 0 max_end
        // covers everything, so any viewport must include bucket 0's chunks.
        let mut notes: Vec<(u32, u32)> = (0..100).map(|i| (i * 10, i * 10 + 5)).collect();
        notes[0] = (0, 10_000_000);
        let idx = build_index(&notes);
        assert_eq!(
            idx.visible_chunk_range(5_000_000, 5_001_000),
            Some((0, 1)),
            "long note crossing from off-screen-left must keep its bucket dispatched"
        );
    }

    #[test]
    fn visible_tick_range_margin() {
        let u = Uniforms {
            width: 800.0,
            height: 600.0,
            scroll_x: 100.0,
            scroll_y: 0.0,
            pixels_per_tick: 0.1,
            key_height: 20.0,
            keyboard_width: 60.0,
            mode: 1,
            ..Default::default()
        };
        // x_offset = 60 - 100 = -40; visible ticks ≈ [(0+40)/0.1, (800+40)/0.1]
        // = [400, 8400], with margin → starts before 400, ends after 8400.
        let (ts, te) = visible_tick_range(&u);
        assert!(ts <= 390 && te >= 8410, "ts={ts} te={te}");
    }
}
