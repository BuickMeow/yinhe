//! GPU compute cull state: per-key note buffers + indirect dispatch.
//!
//! Architecture: each MIDI key (0..127) owns its own `all_notes` (input),
//! `visible_notes` (output), and a per-key draw-args buffer. The cull
//! dispatch loops over keys; each key's visible capacity equals its all-notes
//! capacity, so there is no global visible-note cap.
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

    /// Chunk range [0, chunk_count) that can intersect [tick_start, tick_end].
    /// Conservative: may include buckets that the shader's exact AABB test
    /// then culls. Returns None when nothing can intersect.
    ///
    /// Both arrays are monotonic, so the visible buckets form a prefix:
    ///   - `bucket_suffix_max_end` is non-increasing: buckets with
    ///     suffix_max_end >= ts can intersect (conservative), the first bucket
    ///     with suffix_max_end < ts and everything after it cannot (all notes
    ///     end before ts). partition_point uses `m >= ts` (a true-prefix
    ///     predicate; `m < ts` would be a false-prefix on a decreasing array
    ///     and its binary search result would be undefined).
    ///   - `bucket_start` is non-decreasing: buckets with start <= te can
    ///     intersect.
    ///
    /// Visible buckets = [0, min(b_lo, b_hi_end)), so c_lo is always 0.
    fn visible_chunk_range(&self, tick_start: u32, tick_end: u32) -> Option<(u32, u32)> {
        if self.chunk_total == 0 || tick_start > tick_end {
            return None;
        }
        let b_lo = self
            .bucket_suffix_max_end
            .partition_point(|&m| m >= tick_start);
        let b_hi_end = self.bucket_start.partition_point(|&s| s <= tick_end);
        let b_end = b_lo.min(b_hi_end);
        if b_end == 0 {
            return None;
        }
        let chunks_per_bucket = NOTES_PER_BUCKET / 256;
        let c_hi_end = (b_end * chunks_per_bucket).min(self.chunk_total as usize);
        Some((0, c_hi_end as u32))
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
    /// Per-key bind groups (128 slots). `None` until the key is first uploaded.
    per_key_bind_groups: Vec<Option<BindGroup>>,
    /// Per-key all-notes storage buffers (cull input), grown on demand.
    per_key_buffers: Vec<Option<Buffer>>,
    /// Per-key visible-notes storage buffers (cull output + draw vertex source).
    /// 256-aligned so every chunk's sparse slots [chunk*256, chunk*256+256)
    /// fit; visible slots beyond the written count hold stale data and are
    /// not drawn (draw args bound instance_count to the culled count).
    per_key_visible_buffers: Vec<Option<Buffer>>,
    /// Per-key draw args buffer (one DrawIndirectArgs per chunk, 16B each).
    /// Written by the cull shader's thread 0 per chunk; read by
    /// `multi_draw_indirect` in draw order (chunk order = input order).
    per_key_draw_args_buffers: Vec<Option<Buffer>>,
    /// Chunk count dispatched for each key in the current frame (0 = none).
    /// Filled by `dispatch_cull`, read by `draw_visible_notes`.
    frame_chunk_counts: [u32; 128],

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

        Self {
            pipeline,
            bind_group_layout,
            per_key_bind_groups: (0..128).map(|_| None).collect(),
            per_key_buffers: (0..128).map(|_| None).collect(),
            per_key_visible_buffers: (0..128).map(|_| None).collect(),
            per_key_draw_args_buffers: (0..128).map(|_| None).collect(),
            frame_chunk_counts: [0; 128],
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

    /// Grow (if needed) + write + bind-group-recreate (if any buffer grew) for
    /// one key. Visible buffer is 256-aligned (chunk*256 sparse slots); draw
    /// args buffer holds one DrawIndirectArgs per chunk. The three buffers
    /// grow together so the bind group always sees consistent sizes.
    pub(crate) fn upload_one_key(
        &mut self,
        device: &Device,
        queue: &Queue,
        uniform_buffer: &Buffer,
        key: u8,
        notes: &[NoteInstance],
    ) {
        let needed = notes.len() as u64 * std::mem::size_of::<NoteInstance>() as u64;
        let chunk_total = (notes.len() as u64).div_ceil(256).max(1);
        // Visible buffer is 256-aligned so every chunk's sparse slots
        // [chunk*256, chunk*256+256) fit; draw args: one per chunk.
        let vis_size = chunk_total * 256 * std::mem::size_of::<NoteInstance>() as u64;
        let args_size = chunk_total * std::mem::size_of::<u32>() as u64 * 4;

        let need_recreate = match &self.per_key_buffers[key as usize] {
            None => true,
            Some(buf) => buf.size() < needed,
        } || match &self.per_key_visible_buffers[key as usize] {
            None => true,
            Some(buf) => buf.size() < vis_size,
        } || match &self.per_key_draw_args_buffers[key as usize] {
            None => true,
            Some(buf) => buf.size() < args_size,
        };
        if need_recreate {
            // 释放旧的三个 buffer（如有）并创建新的三个：
            //   all_notes: 大小 needed.max(4096)，usage STORAGE | COPY_DST
            //   visible:   大小 vis_size，usage STORAGE | VERTEX
            //   draw_args: 大小 args_size，usage STORAGE | INDIRECT
            // 全部走 yinhe_memtrace::sub_gpu_resource / add_gpu_resource
            // （可先统一释放旧的三个，再统一创建新的三个）。
            // 只释放当前 key 的三个 buffer——不能遍历全部 keys，那会销毁
            // 其他 key 的 buffer，全量上传后只剩最后一个 key 存活。
            for buf in [
                &mut self.per_key_buffers[key as usize],
                &mut self.per_key_visible_buffers[key as usize],
                &mut self.per_key_draw_args_buffers[key as usize],
            ] {
                if let Some(b) = buf.take() {
                    yinhe_memtrace::sub_gpu_resource(b.size());
                }
            }

            let all_size = needed.max(4096);
            let all_buf = device.create_buffer(&BufferDescriptor {
                label: Some("all_notes_key"),
                size: all_size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            yinhe_memtrace::add_gpu_resource(all_size);
            self.per_key_buffers[key as usize] = Some(all_buf);

            let vis_buf = device.create_buffer(&BufferDescriptor {
                label: Some("visible_notes_key"),
                size: vis_size,
                // COPY_SRC so tests can read back the culled output.
                usage: BufferUsages::STORAGE | BufferUsages::VERTEX | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            yinhe_memtrace::add_gpu_resource(vis_size);
            self.per_key_visible_buffers[key as usize] = Some(vis_buf);

            let args_buf = device.create_buffer(&BufferDescriptor {
                label: Some("draw_args_key"),
                size: args_size,
                usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            yinhe_memtrace::add_gpu_resource(args_size);
            self.per_key_draw_args_buffers[key as usize] = Some(args_buf);

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
    /// Binds: uniform, all_notes[k], visible_notes[k], per-key draw_args[k],
    /// and the dispatch-args slot k (256-byte slice at offset k*256).
    fn recreate_cull_bind_group(&mut self, device: &Device, uniform_buffer: &Buffer, key: u8) {
        let all_buf = match &self.per_key_buffers[key as usize] {
            Some(b) => b.clone(),
            None => return,
        };
        let vis_buf = match &self.per_key_visible_buffers[key as usize] {
            Some(b) => b.clone(),
            None => return,
        };
        let args_buf = match &self.per_key_draw_args_buffers[key as usize] {
            Some(b) => b.clone(),
            None => return,
        };
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
                        resource: args_buf.as_entire_binding(),
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
    /// The shared `dispatch_args_buffer` is reused (it'll be overwritten on the
    /// next `dispatch_cull`).
    pub(crate) fn clear_cull(&mut self) {
        for buf in self
            .per_key_buffers
            .iter_mut()
            .chain(self.per_key_visible_buffers.iter_mut())
            .chain(self.per_key_draw_args_buffers.iter_mut())
        {
            if let Some(b) = buf.take() {
                yinhe_memtrace::sub_gpu_resource(b.size());
            }
        }
        self.per_key_bind_groups.fill(None);
        self.per_key_counts.fill(0);
        self.frame_chunk_counts.fill(0);
        self.bucket_indexes.fill(None);
        self.uploaded_key_revisions.fill(0);
        self.last_cull_uniforms = None;
        self.notes_dirty = false;
    }

    /// Dispatch the cull pass per key. Each key writes into its own visible
    /// buffer's fixed sparse slots + its per-key draw-args buffer.
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
    /// frame's `visible_notes` + per-key draw args are still valid and the
    /// entire dispatch is skipped. This makes idle frames (no scroll, no edit)
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
            self.frame_chunk_counts[key] = chunk_count;
        }
        let mut dispatched_keys = 0u32;
        let mut total_chunks = 0u32;
        for key in 0..128 {
            if info[key * 64] > 0 {
                dispatched_keys += 1;
                total_chunks += info[key * 64];
            }
        }
        tracing::debug!(
            "[cull-dispatch] scroll_x={} ts={} te={} dispatched_keys={dispatched_keys} total_chunks={total_chunks}",
            uniforms.scroll_x,
            tick_start,
            tick_end,
        );
        queue.write_buffer(&self.dispatch_args_buffer, 0, bytemuck::cast_slice(&info));

        let mut cull_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("note_cull"),
            timestamp_writes: None,
        });

        // Per-key cull dispatches. Only keys with visible chunks are dispatched
        // (info[slot] == 0 means chunk_count == 0 → skip).
        cull_pass.set_pipeline(&self.pipeline);
        for key in key_lo..=key_hi {
            let Some(bg) = &self.per_key_bind_groups[key as usize] else {
                continue;
            };
            if info[key as usize * 64] == 0 {
                continue;
            }
            cull_pass.set_bind_group(0, bg, &[]);
            // Dispatch args (wg_x, wg_y, 1, count, c_lo) were written above as
            // one 32KB blob; the GPU reads them via dispatch_workgroups_indirect.
            cull_pass.dispatch_workgroups_indirect(&self.dispatch_args_buffer, key as u64 * 256);
        }
        drop(cull_pass);

        self.last_cull_uniforms = Some(*uniforms);
        self.notes_dirty = false;
    }

    /// Draw the culled notes via `multi_draw_indirect`. Each key's chunks are
    /// drawn in buffer order, and chunks are written in dispatch order
    /// (= input/tick order), so the z-order is fully deterministic across
    /// frames — no dependence on GPU workgroup scheduling.
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
        for key in key_lo..=key_hi {
            let Some(vis_buf) = &self.per_key_visible_buffers[key as usize] else {
                continue;
            };
            let Some(args_buf) = &self.per_key_draw_args_buffers[key as usize] else {
                continue;
            };
            let chunk_count = self.frame_chunk_counts[key as usize];
            if chunk_count == 0 {
                continue;
            }
            pass.set_vertex_buffer(0, vis_buf.slice(..));
            // multi_draw_indirect draws chunks in buffer order (= input order),
            // giving deterministic z-order. Split into ≤1M-draw segments in
            // case a single key ever exceeds maxDrawIndirectCount (1,000,000).
            let mut remaining = chunk_count;
            let mut offset = 0u32;
            while remaining > 0 {
                let n = remaining.min(1_000_000);
                pass.multi_draw_indirect(args_buf, offset as u64 * 16, n);
                offset += n;
                remaining -= n;
            }
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

        // Read back the per-key draw args (instance_count at byte offset 4).
        let args_readback = device.create_buffer(&BufferDescriptor {
            label: Some("args_readback"),
            size: 16,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            cull.per_key_draw_args_buffers[0]
                .as_ref()
                .expect("uploaded"),
            0,
            &args_readback,
            0,
            16,
        );
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
        // DrawIndirectArgs: [vertex_count=6, instance_count, first_vertex=0,
        // first_instance=0] — chunk 0 starts at sparse slot 0.
        assert_eq!(args[0], 6, "vertex_count must be 6 (two triangles)");
        assert_eq!(args[2], 0, "first_vertex must be 0");
        assert_eq!(args[3], 0, "first_instance must be 0 (chunk 0)");
        let instance_count = args[1];
        drop(view);
        args_readback.unmap();

        assert_eq!(
            instance_count, 50,
            "stale notes beyond the uploaded count must not be drawn"
        );
    }

    /// Z-order must be deterministic across frames: with 1000 notes at the
    /// same tick (spanning 4 chunks), the culled output order must follow the
    /// input order every frame, independent of GPU workgroup scheduling.
    #[test]
    fn cull_output_order_is_deterministic() {
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
        // 1000 notes at tick 0 (4 chunks); track index distinguishes order.
        let notes: Vec<NoteInstance> = (0..1000)
            .map(|i| NoteInstance {
                start_tick: 0,
                end_tick: 100,
                packed: NoteInstance::pack(60, i as u16, 100),
            })
            .collect();
        cull.upload_one_key(&device, &queue, &uniform_buffer, 0, &notes);

        let run = |cull: &mut CullState, scroll_x: f32| -> Vec<u32> {
            let mut u = visible_uniforms();
            u.scroll_x = scroll_x; // 1px shift still keeps all notes visible
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 0, &u);
            let readback = device.create_buffer(&BufferDescriptor {
                label: Some("vis_readback"),
                size: 4 * 256 * 12, // 4 chunks × 256 slots × 12B
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(
                cull.per_key_visible_buffers[0].as_ref().expect("uploaded"),
                0,
                &readback,
                0,
                4 * 256 * 12,
            );
            queue.submit([encoder.finish()]);

            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(Ordering::SeqCst));
            let view = readback.slice(..).get_mapped_range();
            let insts: &[NoteInstance] = bytemuck::cast_slice(&view);
            let packed: Vec<u32> = insts[..1000].iter().map(|n| n.packed).collect();
            drop(view);
            readback.unmap();
            packed
        };

        let a = run(&mut cull, 0.0);
        let b = run(&mut cull, 1.0);
        assert_eq!(a, b, "z-order must be stable across frames");
        // Output order == input order: packed = track<<8 | vel<<24 | key, with
        // track = i, so packed increases by 256 per note.
        let expected: Vec<u32> = (0..1000)
            .map(|i| NoteInstance::pack(60, i as u16, 100))
            .collect();
        assert_eq!(a, expected, "culled output must follow input (tick) order");
    }

    fn build_index(start_ends: &[(u32, u32)]) -> KeyBucketIndex {
        let notes: Vec<NoteInstance> = start_ends
            .iter()
            .map(|&(s, e)| NoteInstance {
                start_tick: s,
                end_tick: e,
                packed: NoteInstance::pack(60, 0, 100),
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
    fn bucket_index_prefix_with_long_note() {
        // 4 buckets: bucket 0 has a long note (max_end = 1_000_000), buckets
        // 1..3 are short notes ending well before the viewport.
        // suffix_max = [1_000_000, 163_835, 163_835, 163_835] (non-increasing,
        // with a mid-array boundary). Viewport [200_000, 210_000]: only bucket
        // 0 can intersect (its long note crosses the viewport). Regression:
        // the old code started dispatch at the first suffix < ts bucket,
        // skipping exactly the bucket that contains the visible long note.
        let mut notes: Vec<(u32, u32)> = Vec::new();
        // Bucket 0: 4096 notes, first one is a long note.
        for i in 0..4096 {
            notes.push((i * 10, i * 10 + 5));
        }
        notes[0] = (0, 1_000_000);
        // Buckets 1..3: short notes.
        for b in 1..4 {
            let base = b * 4096 * 10;
            for i in 0..4096 {
                notes.push((base + i * 10, base + i * 10 + 5));
            }
        }
        let idx = build_index(&notes);
        assert_eq!(idx.chunk_total, 64);
        // Only bucket 0 (chunks [0, 16)) is dispatched.
        assert_eq!(idx.visible_chunk_range(200_000, 210_000), Some((0, 16)));
        // Viewport at the very start (bucket 0 starts at tick 0): b_lo = len
        // (suffix all >= ts), b_hi_end = 1 → bucket 0 only.
        assert_eq!(idx.visible_chunk_range(0, 10), Some((0, 16)));
        // Viewport after every note end → nothing.
        assert!(idx.visible_chunk_range(2_000_000, 3_000_000).is_none());
    }

    #[test]
    fn bucket_index_all_suffix_visible() {
        // Every bucket's max_end >= ts (e.g. black-score long notes): b_lo ==
        // len, and the key must still be dispatched (NOT return None).
        // 3 buckets, all with a long note at the start.
        let mut notes: Vec<(u32, u32)> = Vec::new();
        for b in 0..3 {
            let base = b * 4096 * 10;
            notes.push((base, 10_000_000)); // long note in every bucket
            for i in 1..4096 {
                notes.push((base + i * 10, base + i * 10 + 5));
            }
        }
        let idx = build_index(&notes);
        assert_eq!(idx.chunk_total, 48);
        // Viewport in the middle: all buckets' suffix_max_end >= ts, so the
        // whole start-side prefix up to b_hi_end is dispatched.
        assert_eq!(idx.visible_chunk_range(500_000, 600_000), Some((0, 48)));
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

    /// 端到端：黑乐谱风格构造数据（128 keys × 8192 音符 + 每 key 长音符），
    /// 模拟 PR 默认视口，验证每个可见 key 都有输出。
    #[test]
    fn cull_end_to_end_multi_key() {
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

        let mut all_notes = Vec::new();
        let mut offsets = [0u32; 129];
        for key in 0..128u8 {
            let mut notes = Vec::new();
            // 长音符（覆盖到 tick 10M，start=0）
            notes.push(NoteInstance {
                start_tick: 0,
                end_tick: 10_000_000,
                packed: NoteInstance::pack(key, 0, 100),
            });
            // 密集短音符
            for i in 0..8192 {
                notes.push(NoteInstance {
                    start_tick: i * 10 + 1,
                    end_tick: i * 10 + 6,
                    packed: NoteInstance::pack(key, 0, 100),
                });
            }
            offsets[key as usize] = all_notes.len() as u32;
            all_notes.extend(notes);
        }
        offsets[128] = all_notes.len() as u32;
        cull.upload_all_notes(
            &device,
            &queue,
            &uniform_buffer,
            &all_notes,
            &offsets,
            &[0; 128],
        );

        // PR 默认视口：scroll=0, ppu=0.1, kh=12, height=600 → 可见 key 77..127
        // （key 76 的行在 y∈[612, 624)，完全在视口外）
        let u = Uniforms {
            width: 800.0,
            height: 600.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            pixels_per_tick: 0.1,
            key_height: 12.0,
            keyboard_width: 60.0,
            mode: 1,
            ..Default::default()
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.dispatch_cull(&mut encoder, &queue, 76, 127, &u);
        // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
        queue.submit([encoder.finish()]);

        // 读回每个 key 的 draw_args[0]，断言 instance_count >= 1（长音符可见）
        // 可见范围是 77..=127：key 76 的行在 y∈[612, 624)，完全在视口外。
        let mut bad: Vec<u32> = Vec::new();
        for key in 77..=127 {
            let readback = device.create_buffer(&BufferDescriptor {
                label: Some("args_readback"),
                size: 16,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let args_buf = match &cull.per_key_draw_args_buffers[key as usize] {
                Some(b) => b,
                None => panic!("key {key} 没有 args buffer (upload 后应存在)"),
            };
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 16);
            queue.submit([enc.finish()]);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(Ordering::SeqCst));
            let view = readback.slice(..).get_mapped_range();
            let args: &[u32] = bytemuck::cast_slice(&view);
            let count = args[1];
            drop(view);
            readback.unmap();
            if count == 0 {
                bad.push(key as u32);
            }
        }
        assert!(bad.is_empty(), "这些 key 没有可见音符: {bad:?}");
    }

    /// 端到端：真实 MIDI 文件。CPU 路径（build_notes）与 GPU cull 的输出对比。
    /// 文件不存在时跳过（CI 兼容）。
    #[test]
    fn cull_real_midi_vs_cpu() {
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

        // 先测一个小文件（几万音符级），再测大文件
        let paths = [
            "/Users/jieneng/Music/MIDIs/99 Luftballons.mid",
            "/Users/jieneng/Music/MIDIs/APT.mid",
            "/Users/jieneng/Music/MIDIs/1.mid",
        ];
        let mut tested_any = false;
        let mut bad_ratios: Vec<(&str, u32, u64, f64)> = Vec::new();
        for path in paths {
            // `parser` 是 yinhe-mid2 的私有模块，解析入口在 crate 根：
            let Ok(model) = yinhe_mid2::parse_path(path) else {
                continue; // 文件不存在或解析失败 → 跳过
            };
            tested_any = true;

            // ── 构造统一的 PR 视口 ──
            let ppu = 0.1f32;
            let kh = 12.0f32;
            let width = 800.0f32;
            let height = 600.0f32;
            let kb_w = 60.0f32;
            // TimelineViewBase 没有 derive Default，用 PianoRollView::default() 再覆写。
            // TimelineViewBase 没有 derive Default，字段全部显式给出。
            let view = yinhe_types::PianoRollView {
                key_height: kh,
                viewport_h: height,
                base: yinhe_types::TimelineViewBase {
                    pixels_per_tick: ppu,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    left_panel_width: kb_w,
                    dirty: true,
                    track_panel_row_height: 40.0,
                    track_panel_scroll_y: 0.0,
                },
            };
            let hidden = std::collections::HashSet::new();
            let track_visible: Vec<bool> = vec![true; model.tracks.len()];

            // ── CPU 期望值 ──
            let mut cpu_out: Vec<NoteInstance> = Vec::new();
            crate::pianoroll::build_notes(
                &mut cpu_out,
                width,
                height,
                &model,
                &view,
                &hidden,
                &track_visible,
            );
            // CPU 输出按 key 统计
            let mut cpu_by_key = [0u32; 128];
            for n in &cpu_out {
                cpu_by_key[(n.packed & 0xFF) as usize] += 1;
            }
            let cpu_total: u32 = cpu_by_key.iter().sum();

            // ── GPU cull ──
            let (all_notes, offsets) =
                crate::pianoroll::build_all_notes(&model, &hidden, &track_visible);
            cull.upload_all_notes(
                &device,
                &queue,
                &uniform_buffer,
                &all_notes,
                &offsets,
                &[0; 128],
            );

            let u = Uniforms {
                width,
                height,
                scroll_x: 0.0,
                scroll_y: 0.0,
                pixels_per_tick: ppu,
                key_height: kh,
                keyboard_width: kb_w,
                mode: 1,
                ..Default::default()
            };
            // 写 uniform buffer：dispatch_cull 只读 Rust 侧 Uniforms 算 CPU 端
            // 桶索引，不会把 uniform 写进 GPU buffer。不写的话 shader 读到
            // 全零 uniform（mode=0、width/height=0、ppu=0），所有音符都通过
            // 裁剪，GPU 输出等于全量音符。
            queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
            // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
            queue.submit([encoder.finish()]);

            // 读回每个 key 的 draw_args。只读本帧实际派发的 chunk
            // （frame_chunk_counts）：未派发的 key 的 draw_args 从未被 shader
            // 写入（内容未定义，读了是垃圾），按 0 计。
            let mut gpu_total: u64 = 0;
            let mut gpu_by_key = [0u64; 128];
            for (key, gpu_key_total) in gpu_by_key.iter_mut().enumerate() {
                let chunk_count = cull.frame_chunk_counts[key];
                if chunk_count == 0 {
                    continue;
                }
                let Some(args_buf) = &cull.per_key_draw_args_buffers[key] else {
                    continue; // buffer 被销毁（upload 释放 bug）→ 无输出，按 0 计
                };
                let read_size = chunk_count as u64 * 16;
                let readback = device.create_buffer(&BufferDescriptor {
                    label: Some("args_readback"),
                    size: read_size,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut enc = device.create_command_encoder(&Default::default());
                enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, read_size);
                queue.submit([enc.finish()]);
                let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let done2 = done.clone();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                    done2.store(true, Ordering::SeqCst);
                });
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("poll failed");
                assert!(done.load(Ordering::SeqCst));
                let view = readback.slice(..).get_mapped_range();
                let args: &[u32] = bytemuck::cast_slice(&view);
                let mut key_total: u64 = 0;
                for c in 0..chunk_count as usize {
                    key_total += args[c * 4 + 1] as u64; // instance_count
                }
                drop(view);
                readback.unmap();
                *gpu_key_total = key_total;
                gpu_total += key_total;
            }

            // ── 报告（println 输出，测试结束后我分析）──
            let cpu_keys: Vec<u32> = (0..128u32)
                .filter(|&k| cpu_by_key[k as usize] > 0)
                .collect();
            let gpu_keys: Vec<u32> = (0..128u32)
                .filter(|&k| gpu_by_key[k as usize] > 0)
                .collect();
            println!(
                "FILE {path}: CPU total={cpu_total} keys={cpu_keys:?}; GPU total={gpu_total} keys={gpu_keys:?}"
            );
            println!(
                "  per-key GPU counts: {:?}",
                (0..128u32)
                    .filter(|&k| gpu_by_key[k as usize] > 0 || cpu_by_key[k as usize] > 0)
                    .map(|k| (k, cpu_by_key[k as usize], gpu_by_key[k as usize]))
                    .collect::<Vec<_>>()
            );

            // 断言：GPU 输出与 CPU 同数量级（GPU 是 CPU 的 50%..150%）。
            // 不立即 panic，而是收集所有文件的违规，全部跑完后统一断言，
            // 这样所有文件的对比数字都能打印出来供分析。
            if cpu_total > 0 {
                let ratio = gpu_total as f64 / cpu_total as f64;
                if !(ratio > 0.5 && ratio < 1.5) {
                    bad_ratios.push((path, cpu_total, gpu_total, ratio));
                }
            }
        }
        assert!(
            bad_ratios.is_empty(),
            "GPU/CPU 输出比例异常: {bad_ratios:?}"
        );
        if !tested_any {
            eprintln!("没有可用的 MIDI 文件，测试跳过");
        }
    }

    /// 滚动序列：upload 后两次不同 scroll_x 的 dispatch，输出必须不同。
    /// 如果相同 → dispatch 层没更新（cull 层 bug）。
    #[test]
    fn cull_scroll_sequence_updates() {
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
        let mut all_notes = Vec::new();
        let mut offsets = [0u32; 129];
        for key in 0..128u8 {
            let mut notes = Vec::new();
            for i in 0..20_000 {
                // 均匀 10-tick 网格会让 c1/c2 两个视口恰好容纳相同数量音符
                // （c1==c2，滚动是否更新无从分辨）。在 [50000, 80000) 挖一个
                // 空洞，让三个视口的音符数各不相同。
                let start = (i as u32) * 10 + if i >= 5000 { 30_000 } else { 0 };
                notes.push(NoteInstance {
                    start_tick: start,
                    end_tick: start + 5,
                    packed: NoteInstance::pack(key, 0, 100),
                });
            }
            offsets[key as usize] = all_notes.len() as u32;
            all_notes.extend(notes);
        }
        offsets[128] = all_notes.len() as u32;
        cull.upload_all_notes(
            &device,
            &queue,
            &uniform_buffer,
            &all_notes,
            &offsets,
            &[0; 128],
        );

        let run = |cull: &mut CullState, scroll_x: f32| -> u64 {
            let u = Uniforms {
                width: 800.0,
                height: 600.0,
                scroll_x,
                scroll_y: 0.0,
                pixels_per_tick: 0.1,
                key_height: 12.0,
                keyboard_width: 60.0,
                mode: 1,
                ..Default::default()
            };
            queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
            // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
            queue.submit([encoder.finish()]);
            // 读回所有 key 的 draw_args（只读 frame_chunk_counts 个 chunk）
            let mut total: u64 = 0;
            for key in 0..128 {
                let Some(args_buf) = &cull.per_key_draw_args_buffers[key] else {
                    continue;
                };
                let chunk_count = cull.frame_chunk_counts[key] as usize;
                if chunk_count == 0 {
                    continue;
                }
                let readback = device.create_buffer(&BufferDescriptor {
                    label: Some("args_readback"),
                    size: 16 * chunk_count as u64,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut enc = device.create_command_encoder(&Default::default());
                enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 16 * chunk_count as u64);
                queue.submit([enc.finish()]);
                let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let done2 = done.clone();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                    done2.store(true, Ordering::SeqCst);
                });
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("poll failed");
                assert!(done.load(Ordering::SeqCst));
                let view = readback.slice(..).get_mapped_range();
                let args: &[u32] = bytemuck::cast_slice(&view);
                for c in 0..chunk_count {
                    total += args[c * 4 + 1] as u64;
                }
                drop(view);
                readback.unmap();
            }
            total
        };

        let c0 = run(&mut cull, 0.0); // 视口 tick ~[0, 7412]
        let c1 = run(&mut cull, 4000.0); // 视口 tick ~[39388, 47412]
        let c2 = run(&mut cull, 8000.0); // 视口 tick ~[79388, 87412]（音符到 199990，仍有）
        println!("SCROLL: c0={c0} c1={c1} c2={c2}");
        assert!(c1 != c0, "滚动后输出必须变化: c0={c0} c1={c1}");
        assert!(c2 != c1, "滚动后输出必须变化: c1={c1} c2={c2}");
        assert!(c0 > 0, "首个视口应有输出");
    }

    /// 真实 MIDI：模拟 egui 层完整序列（upload 判断 → dispatch → 滚动 → 切轨）。
    #[test]
    fn cull_real_midi_sequence() {
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
        let paths = [
            "/Users/jieneng/Music/MIDIs/99 Luftballons.mid",
            "/Users/jieneng/Music/MIDIs/1.mid",
            "/Users/jieneng/Music/MIDIs/123.mid",
        ];
        let mut tested_any = false;
        for path in paths {
            let Ok(model) = yinhe_mid2::parse_path(path) else {
                continue; // 文件不存在或解析失败 → 跳过
            };
            tested_any = true;
            println!(
                "=== FILE {path}: tracks={} note_count={}",
                model.tracks.len(),
                model.note_count
            );

            let hidden = std::collections::HashSet::new();
            let all_visible: Vec<bool> = vec![true; model.tracks.len()];

            // ── 模拟 gpu_upload::upload 的 note_key 判断 + 上传 ──
            let note_key =
                |revision: u64, tv: &[bool], h: &std::collections::HashSet<(u16, u32, u8)>| {
                    crate::NoteBufferKey::new(revision, tv, h).value()
                };
            let mut last_key = 0u64;
            let mut last_rev = 0u64;
            let mut last_hidden = 0u64;
            let upload_once =
                |cull: &mut CullState,
                 model: &yinhe_core::YinModel,
                 tv: &[bool],
                 note_revisions: &[u64; 128],
                 last_key: &mut u64,
                 last_rev: &mut u64,
                 last_hidden: &mut u64,
                 revision: u64,
                 hidden: &std::collections::HashSet<(u16, u32, u8)>| {
                    let cull_was_ready = cull.per_key_bind_groups.iter().any(|bg| bg.is_some());
                    if !cull_was_ready {
                        *last_key = 0;
                    }
                    let nk = note_key(revision, tv, hidden);
                    if nk == *last_key {
                        return;
                    }
                    // 全量上传（简化：不做增量路径，测试重点是全量+track_visible）
                    let (all_notes, offsets) = crate::pianoroll::build_all_notes(model, hidden, tv);
                    cull.upload_all_notes(
                        &device,
                        &queue,
                        &uniform_buffer,
                        &all_notes,
                        &offsets,
                        note_revisions,
                    );
                    *last_key = nk;
                    *last_rev = revision;
                    *last_hidden = crate::hash_hidden(hidden);
                };

            let revision: u64 = 1;
            let note_revisions = [revision; 128];

            // 步骤 1：首次全量上传（全轨道可见）
            upload_once(
                &mut cull,
                &model,
                &all_visible,
                &note_revisions,
                &mut last_key,
                &mut last_rev,
                &mut last_hidden,
                revision,
                &hidden,
            );
            // 步骤 2：dispatch 视口 1（scroll_x=0）
            let run = |cull: &mut CullState, scroll_x: f32| -> u64 {
                let u = Uniforms {
                    width: 800.0,
                    height: 600.0,
                    scroll_x,
                    scroll_y: 0.0,
                    pixels_per_tick: 0.1,
                    key_height: 12.0,
                    keyboard_width: 60.0,
                    mode: 1,
                    ..Default::default()
                };
                queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
                let mut encoder = device.create_command_encoder(&Default::default());
                cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
                // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
                queue.submit([encoder.finish()]);
                let mut total: u64 = 0;
                for key in 0..128 {
                    let Some(args_buf) = &cull.per_key_draw_args_buffers[key] else {
                        continue;
                    };
                    let chunk_count = cull.frame_chunk_counts[key] as usize;
                    if chunk_count == 0 {
                        continue;
                    }
                    let readback = device.create_buffer(&BufferDescriptor {
                        label: Some("args_readback"),
                        size: 16 * chunk_count as u64,
                        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    let mut enc = device.create_command_encoder(&Default::default());
                    enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 16 * chunk_count as u64);
                    queue.submit([enc.finish()]);
                    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let done2 = done.clone();
                    readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                        done2.store(true, Ordering::SeqCst);
                    });
                    device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .expect("poll failed");
                    assert!(done.load(Ordering::SeqCst));
                    let view = readback.slice(..).get_mapped_range();
                    let args: &[u32] = bytemuck::cast_slice(&view);
                    for c in 0..chunk_count {
                        total += args[c * 4 + 1] as u64;
                    }
                    drop(view);
                    readback.unmap();
                }
                total
            };
            let s0 = run(&mut cull, 0.0);
            // 步骤 3：模拟滚动（upload 判断：note_key 不变 → 跳过）→ dispatch 视口 2
            upload_once(
                &mut cull,
                &model,
                &all_visible,
                &note_revisions,
                &mut last_key,
                &mut last_rev,
                &mut last_hidden,
                revision,
                &hidden,
            );
            let s1 = run(&mut cull, 1000.0);
            // 步骤 4：模拟切轨（track_visible 变化 → 必须重新上传）→ dispatch
            let mut half_visible = all_visible.clone();
            for (i, v) in half_visible.iter_mut().enumerate() {
                if i % 2 == 1 {
                    *v = false;
                }
            }
            upload_once(
                &mut cull,
                &model,
                &half_visible,
                &note_revisions,
                &mut last_key,
                &mut last_rev,
                &mut last_hidden,
                revision,
                &hidden,
            );
            let s2 = run(&mut cull, 1000.0);
            println!("SEQ {path}: s0={s0} s1={s1} s2={s2}");
            assert!(s1 != s0, "滚动后输出必须变化: s0={s0} s1={s1}");
            // 切轨后输出应减少（一半轨道隐藏；若模型轨道数<=1 或全部音符在同一轨道则可能不减，允许相等）
            assert!(s2 <= s1, "切轨后输出不应增加: s1={s1} s2={s2}");
        }
        if !tested_any {
            eprintln!("无可用 MIDI 文件，跳过");
        }
    }

    /// start.mid（1.64 亿音符）端到端 GPU cull 测试：模拟真实视口
    /// （width=1376, height=419, ppu=0.026372144, kh=3.2734375, scroll_y=0,
    /// keyboard_width=60, mode=1），在 5 个滚动位置验证：
    /// 1. 滚动后 GPU 输出的 instance_count 必须变化（无固定数量限制）；
    /// 2. GPU 每 key 计数与 CPU 逐音符（精确镜像 shader 的 f32 条件）完全一致。
    #[test]
    fn cull_start_mid_sequence() {
        let path = "/Users/jieneng/Music/MIDIs/start.mid";
        if !std::path::Path::new(path).exists() {
            eprintln!("start.mid 不存在，跳过");
            return;
        }
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let t0 = std::time::Instant::now();
        let model = yinhe_mid2::parse_path(path).expect("parse start.mid 失败");
        println!(
            "PARSE {:?} note_count={} tick_length={} tracks={}",
            t0.elapsed(),
            model.note_count,
            model.tick_length,
            model.tracks.len()
        );

        let mut cull = CullState::new(&device);
        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("test_uniform"),
            size: 256,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let t1 = std::time::Instant::now();
        let track_visible: Vec<bool> = vec![true; model.tracks.len()];
        let (all_notes, offsets) = crate::pianoroll::build_all_notes(
            &model,
            &std::collections::HashSet::new(),
            &track_visible,
        );
        println!(
            "build_all_notes {:?} len={} ({:.2} GB)",
            t1.elapsed(),
            all_notes.len(),
            all_notes.len() as f64 * 12.0 / 1e9
        );

        let t2 = std::time::Instant::now();
        cull.upload_all_notes(
            &device,
            &queue,
            &uniform_buffer,
            &all_notes,
            &offsets,
            &[0; 128],
        );
        println!(
            "upload_all_notes {:?} GPU per_key_counts={:?}",
            t2.elapsed(),
            &cull.per_key_counts[..]
        );
        drop(all_notes); // 释放 CPU 副本，降低峰值内存

        // 真实视口（用户日志数值）
        const W: f32 = 1376.0;
        const H: f32 = 419.0;
        const PPU: f32 = 0.026372144;
        const KH: f32 = 3.2734375;
        const KB_W: f32 = 60.0;
        // te = (scroll_x + 1316)/ppu → scroll_x = te*ppu - 1316
        // tick 100 万 → scroll_x ≈ 25056；tick 300 万 → scroll_x ≈ 77800
        let scrolls = [0.0f32, 1000.0, 6490.0, 25056.0, 77800.0];

        let mut prev_total: Option<u64> = None;
        for &scroll_x in &scrolls {
            let u = Uniforms {
                width: W,
                height: H,
                scroll_x,
                scroll_y: 0.0,
                pixels_per_tick: PPU,
                key_height: KH,
                keyboard_width: KB_W,
                mode: 1,
                ..Default::default()
            };
            let (ts, te) = visible_tick_range(&u);
            queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
            queue.submit([encoder.finish()]);

            // 读回每个 key 的 draw_args（只读本帧实际派发的 chunk）
            let mut gpu_per_key = [0u64; 128];
            let mut gpu_total: u64 = 0;
            for (key, &chunk_count) in cull.frame_chunk_counts.iter().enumerate() {
                if chunk_count == 0 {
                    continue;
                }
                let Some(args_buf) = &cull.per_key_draw_args_buffers[key] else {
                    continue;
                };
                let read_size = chunk_count as u64 * 16;
                let readback = device.create_buffer(&BufferDescriptor {
                    label: Some("args_readback"),
                    size: read_size,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut enc = device.create_command_encoder(&Default::default());
                enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, read_size);
                queue.submit([enc.finish()]);
                let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let done2 = done.clone();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                    done2.store(true, Ordering::SeqCst);
                });
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("poll failed");
                assert!(done.load(Ordering::SeqCst));
                let view = readback.slice(..).get_mapped_range();
                let args: &[u32] = bytemuck::cast_slice(&view);
                let mut key_total: u64 = 0;
                for c in 0..chunk_count as usize {
                    key_total += args[c * 4 + 1] as u64; // instance_count
                }
                drop(view);
                readback.unmap();
                gpu_per_key[key] = key_total;
                gpu_total += key_total;
            }
            let gpu_keys = gpu_per_key.iter().filter(|&&c| c > 0).count();

            // CPU 参考：精确镜像 shader 的 X 可见条件（同样的 f32 运算，
            // 不用 visible_tick_range——它含 pad 边距而 shader 没有）。
            let t3 = std::time::Instant::now();
            let x_offset = KB_W - scroll_x;
            let mut cpu_total: u64 = 0;
            let mut cpu_keys = 0u32;
            let mut cpu_per_key = [0u64; 128];
            for (key, slot) in cpu_per_key.iter_mut().enumerate() {
                let mut c = 0u64;
                for n in model.notes[key].iter() {
                    if n.end_tick > n.start_tick {
                        let x = x_offset + n.start_tick as f32 * PPU;
                        let right = x_offset + n.end_tick as f32 * PPU;
                        if right >= 0.0 && x <= W {
                            c += 1;
                        }
                    }
                }
                *slot = c;
                cpu_total += c;
                if c > 0 {
                    cpu_keys += 1;
                }
            }
            println!("CPU 参考计数耗时 {:?}", t3.elapsed());

            let dispatched_chunks: u32 = cull.frame_chunk_counts.iter().sum();
            println!(
                "SCROLL_X={scroll_x} ts={ts} te={te} chunks={dispatched_chunks} | GPU total={gpu_total} keys>0={gpu_keys} | CPU total={cpu_total} keys>0={cpu_keys}"
            );
            // GPU vs CPU 每 key 对比
            let mut diffs: Vec<(u32, i64, u64)> = Vec::new();
            for (key, &g) in gpu_per_key.iter().enumerate() {
                let c = cpu_per_key[key];
                if g != c {
                    diffs.push((key as u32, g as i64 - c as i64, c));
                }
            }
            println!(
                "  GPU≠CPU 的 key 数={}，示例(前10): {:?}",
                diffs.len(),
                &diffs[..diffs.len().min(10)]
            );
            // f32 镜像后 GPU 与 CPU 应逐 key 完全一致（留 2 容差防平台舍入差）。
            assert!(
                diffs.iter().all(|&(_, d, _)| d.abs() <= 2),
                "GPU 与 CPU 逐 key 计数不一致: {diffs:?}"
            );

            if let Some(prev) = prev_total {
                assert_ne!(
                    gpu_total, prev,
                    "滚动后 GPU 输出必须变化: scroll 前={prev} scroll 后={gpu_total}"
                );
            }
            prev_total = Some(gpu_total);
        }
    }

    /// 精确逐 key 复现测试：每 key 50000 音符（13 个 bucket），视口滚动到
    /// 歌曲各位置（开头 / 1/4 / 中间 / 3/4），GPU cull 输出与 CPU f32 镜像
    /// 逐 key 精确对比（容差 2）。
    ///
    /// 现有测试只断言「比例 50%~150%」或「输出 > 0」，覆盖不到「每个 key
    /// 只显示前几个 bucket、后面全部丢失」这类部分丢失 bug。
    #[test]
    fn cull_mid_song_exact_per_key() {
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

        // 每 key 50000 音符：start = i*20, end = start+10 → 覆盖 [0, 1M) ticks，
        // 13 个 bucket / 196 chunks。
        let mut all_notes = Vec::new();
        let mut offsets = [0u32; 129];
        for key in 0..128u8 {
            let notes: Vec<NoteInstance> = (0..50_000u32)
                .map(|i| NoteInstance {
                    start_tick: i * 20,
                    end_tick: i * 20 + 10,
                    packed: NoteInstance::pack(key, 0, 100),
                })
                .collect();
            offsets[key as usize] = all_notes.len() as u32;
            all_notes.extend(notes);
        }
        offsets[128] = all_notes.len() as u32;
        cull.upload_all_notes(
            &device,
            &queue,
            &uniform_buffer,
            &all_notes,
            &offsets,
            &[0; 128],
        );

        let (w, h, kh, kb_w) = (800.0f32, 600.0f32, 12.0f32, 60.0f32);
        let mut any_bad = false;
        for ppu in [0.1f32, 0.026372144] {
            // 视口中心 tick：开头 / 1/4 / 中间 / 3/4（scroll_x = tick * ppu）
            for &center_tick in &[0u32, 250_000, 500_000, 750_000] {
                let scroll_x = center_tick as f32 * ppu;
                let u = Uniforms {
                    width: w,
                    height: h,
                    scroll_x,
                    scroll_y: 0.0,
                    pixels_per_tick: ppu,
                    key_height: kh,
                    keyboard_width: kb_w,
                    mode: 1,
                    ..Default::default()
                };
                queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
                let mut encoder = device.create_command_encoder(&Default::default());
                cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
                queue.submit([encoder.finish()]);

                let (ts, te) = visible_tick_range(&u);
                let x_offset = kb_w - scroll_x;
                let bottom_y = 128.0 * kh - u.scroll_y;
                let mut mismatches: Vec<(u32, u64, u64, i64)> = Vec::new();
                for key in 0..128u8 {
                    // CPU 期望：f32 镜像 shader 的 X + Y 条件
                    let mut expected = 0u64;
                    for n in &all_notes
                        [offsets[key as usize] as usize..offsets[key as usize + 1] as usize]
                    {
                        if n.end_tick > n.start_tick {
                            let px = x_offset + n.start_tick as f32 * ppu;
                            let pr = x_offset + n.end_tick as f32 * ppu;
                            if pr >= 0.0 && px <= w {
                                let k = (n.packed & 0xFF) as f32;
                                let pb = bottom_y - k * kh;
                                let py = bottom_y - (k + 1.0) * kh;
                                if pb >= 0.0 && py <= h {
                                    expected += 1;
                                }
                            }
                        }
                    }

                    // GPU：读回该 key 的 draw_args（本帧实际派发的 chunk）
                    let chunk_count = cull.frame_chunk_counts[key as usize] as usize;
                    let mut gpu = 0u64;
                    if chunk_count > 0 {
                        let args_buf = cull.per_key_draw_args_buffers[key as usize]
                            .as_ref()
                            .expect("有 chunk 派发却没有 args buffer");
                        let readback = device.create_buffer(&BufferDescriptor {
                            label: Some("args_readback"),
                            size: 16 * chunk_count as u64,
                            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        });
                        let mut enc = device.create_command_encoder(&Default::default());
                        enc.copy_buffer_to_buffer(
                            args_buf,
                            0,
                            &readback,
                            0,
                            16 * chunk_count as u64,
                        );
                        queue.submit([enc.finish()]);
                        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let done2 = done.clone();
                        readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                            done2.store(true, Ordering::SeqCst);
                        });
                        device
                            .poll(wgpu::PollType::wait_indefinitely())
                            .expect("poll failed");
                        assert!(done.load(Ordering::SeqCst));
                        let view = readback.slice(..).get_mapped_range();
                        let args: &[u32] = bytemuck::cast_slice(&view);
                        for c in 0..chunk_count {
                            gpu += args[c * 4 + 1] as u64;
                        }
                        drop(view);
                        readback.unmap();
                    }

                    let diff = gpu as i64 - expected as i64;
                    if diff.abs() > 2 {
                        mismatches.push((key as u32, expected, gpu, diff));
                    }
                }
                if !mismatches.is_empty() {
                    any_bad = true;
                    println!(
                        "✗ ppu={ppu} scroll_x={scroll_x} (ts={ts} te={te}) 不匹配 key 数={}, 示例: {:?}",
                        mismatches.len(),
                        &mismatches[..mismatches.len().min(8)]
                    );
                    // 打印一个坏 key 的 bucket 诊断
                    let key = mismatches[0].0 as usize;
                    if let Some(idx) = &cull.bucket_indexes[key] {
                        println!(
                            "  key={key}: buckets={} chunk_total={} b_lo={} b_hi_end={} dispatched_chunks={}",
                            idx.bucket_start.len(),
                            idx.chunk_total,
                            idx.bucket_suffix_max_end.partition_point(|&m| m >= ts),
                            idx.bucket_start.partition_point(|&s| s <= te),
                            cull.frame_chunk_counts[key],
                        );
                    }
                }
            }
        }
        assert!(!any_bad, "存在 GPU 与 CPU 逐 key 计数不匹配（见上方打印）");
    }

    /// 真实大 MIDI + 相对视口：视口中心 = 歌曲总 tick 的 25% / 50% / 75%，
    /// GPU cull 输出与 CPU f32 镜像逐 key 精确对比（容差 2）。
    ///
    /// 黑乐谱大文件音符密度极高（start.mid：1.64 亿音符 / 309 万 ticks），
    /// 每个 key 的音符数可达数百万（几十上百个 bucket），与合成均匀分布
    /// 的场景不同，需要真实文件验证。
    #[test]
    fn cull_real_large_midi_relative_viewport() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let paths = [
            "/Users/jieneng/Music/MIDIs/test.mid",
            "/Users/jieneng/Music/MIDIs/Night Voyager.mid",
            "/Users/jieneng/Music/MIDIs/Ouranos - HDSQ & The Romanticist [v1.6.6].mid",
            "/Users/jieneng/Music/MIDIs/start.mid", // 1.64 亿音符，parse 约 3.6 分钟
        ];
        let (w, h, kh, kb_w) = (1376.0f32, 419.0f32, 3.2734375, 60.0f32);
        let ppu = 0.026372144f32;
        let mut tested_any = false;
        for path in paths {
            let t0 = std::time::Instant::now();
            let Ok(model) = yinhe_mid2::parse_path(path) else {
                println!("{path}: 不存在或解析失败，跳过");
                continue;
            };
            tested_any = true;
            println!(
                "=== {path}: parse={:?} note_count={} tick_length={} tracks={}",
                t0.elapsed(),
                model.note_count,
                model.tick_length,
                model.tracks.len()
            );

            let hidden = std::collections::HashSet::new();
            let track_visible: Vec<bool> = vec![true; model.tracks.len()];
            let t1 = std::time::Instant::now();
            let (all_notes, offsets) =
                crate::pianoroll::build_all_notes(&model, &hidden, &track_visible);
            println!("build_all_notes {:?} len={}", t1.elapsed(), all_notes.len());

            let mut cull = CullState::new(&device);
            let uniform_buffer = device.create_buffer(&BufferDescriptor {
                label: Some("test_uniform"),
                size: 256,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let t2 = std::time::Instant::now();
            cull.upload_all_notes(
                &device,
                &queue,
                &uniform_buffer,
                &all_notes,
                &offsets,
                &[0; 128],
            );
            println!("upload_all_notes {:?}", t2.elapsed());

            let total_ticks = model.tick_length;
            let x_offset = |scroll_x: f32| kb_w - scroll_x;
            let bottom_y = 128.0 * kh;
            let mut any_bad = false;
            // 视口中心 = 歌曲总长的比例处（相对位置）
            for frac in [0.0f32, 0.25, 0.5, 0.75] {
                let center_tick = total_ticks as f32 * frac;
                // scroll_x 使 tick=center_tick 落在视口中心
                let scroll_x = (kb_w + center_tick * ppu - w / 2.0).max(0.0);
                let u = Uniforms {
                    width: w,
                    height: h,
                    scroll_x,
                    scroll_y: 0.0,
                    pixels_per_tick: ppu,
                    key_height: kh,
                    keyboard_width: kb_w,
                    mode: 1,
                    ..Default::default()
                };
                queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
                let mut encoder = device.create_command_encoder(&Default::default());
                cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
                queue.submit([encoder.finish()]);

                let (ts, te) = visible_tick_range(&u);
                let xo = x_offset(scroll_x);
                let mut mismatches: Vec<(u32, u64, u64, i64)> = Vec::new();
                let mut gpu_total = 0u64;
                let mut cpu_total = 0u64;
                for key in 0..128u8 {
                    // CPU 期望：f32 镜像 shader 的 X + Y 条件
                    let mut expected = 0u64;
                    for n in &all_notes
                        [offsets[key as usize] as usize..offsets[key as usize + 1] as usize]
                    {
                        if n.end_tick > n.start_tick {
                            let px = xo + n.start_tick as f32 * ppu;
                            let pr = xo + n.end_tick as f32 * ppu;
                            if pr >= 0.0 && px <= w {
                                let k = (n.packed & 0xFF) as f32;
                                let pb = bottom_y - k * kh;
                                let py = bottom_y - (k + 1.0) * kh;
                                if pb >= 0.0 && py <= h {
                                    expected += 1;
                                }
                            }
                        }
                    }
                    cpu_total += expected;

                    let chunk_count = cull.frame_chunk_counts[key as usize] as usize;
                    let mut gpu = 0u64;
                    if chunk_count > 0 {
                        let args_buf = cull.per_key_draw_args_buffers[key as usize]
                            .as_ref()
                            .expect("有 chunk 派发却没有 args buffer");
                        let readback = device.create_buffer(&BufferDescriptor {
                            label: Some("args_readback"),
                            size: 16 * chunk_count as u64,
                            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        });
                        let mut enc = device.create_command_encoder(&Default::default());
                        enc.copy_buffer_to_buffer(
                            args_buf,
                            0,
                            &readback,
                            0,
                            16 * chunk_count as u64,
                        );
                        queue.submit([enc.finish()]);
                        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let done2 = done.clone();
                        readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                            done2.store(true, Ordering::SeqCst);
                        });
                        device
                            .poll(wgpu::PollType::wait_indefinitely())
                            .expect("poll failed");
                        assert!(done.load(Ordering::SeqCst));
                        let view = readback.slice(..).get_mapped_range();
                        let args: &[u32] = bytemuck::cast_slice(&view);
                        for c in 0..chunk_count {
                            gpu += args[c * 4 + 1] as u64;
                        }
                        drop(view);
                        readback.unmap();
                    }
                    gpu_total += gpu;

                    let diff = gpu as i64 - expected as i64;
                    if diff.abs() > 2 {
                        mismatches.push((key as u32, expected, gpu, diff));
                    }
                }
                println!(
                    "  frac={frac} (ts={ts} te={te}) GPU_total={gpu_total} CPU_total={cpu_total} 不匹配 key 数={}",
                    mismatches.len()
                );
                if !mismatches.is_empty() {
                    any_bad = true;
                    println!("    示例: {:?}", &mismatches[..mismatches.len().min(10)]);
                    let key = mismatches[0].0 as usize;
                    if let Some(idx) = &cull.bucket_indexes[key] {
                        println!(
                            "    key={key}: buckets={} chunk_total={} b_lo={} b_hi_end={} dispatched_chunks={}",
                            idx.bucket_start.len(),
                            idx.chunk_total,
                            idx.bucket_suffix_max_end.partition_point(|&m| m >= ts),
                            idx.bucket_start.partition_point(|&s| s <= te),
                            cull.frame_chunk_counts[key],
                        );
                    }
                }
            }
            assert!(
                !any_bad,
                "{path}: 存在 GPU 与 CPU 逐 key 计数不匹配（见上方打印）"
            );
        }
        assert!(tested_any, "没有任何可用 MIDI 文件");
    }

    /// 多帧交互序列：模拟真实使用中的状态机（滚动 → skip 优化 → 编辑增量
    /// 上传 → 切轨 → hidden 变化），每帧 GPU cull 输出与 CPU f32 镜像逐 key
    /// 精确对比。覆盖单帧 dispatch 测试测不到的上传/派发状态交互。
    #[test]
    fn cull_multi_frame_interaction_sequence() {
        let path = "/Users/jieneng/Music/MIDIs/test.mid";
        if !std::path::Path::new(path).exists() {
            eprintln!("test.mid 不存在，跳过");
            return;
        }
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let t0 = std::time::Instant::now();
        let mut model = yinhe_mid2::parse_path(path).expect("parse test.mid 失败");
        println!(
            "parse {:?} note_count={} tick_length={}",
            t0.elapsed(),
            model.note_count,
            model.tick_length
        );

        let mut cull = CullState::new(&device);
        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("test_uniform"),
            size: 256,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (w, h, kh, kb_w) = (1376.0f32, 419.0f32, 3.2734375, 60.0f32);
        let ppu = 0.026372144f32;
        let bottom_y = 128.0 * kh;

        // ── 模拟 gpu_upload::upload 的状态机 ──
        let mut last_key = 0u64; // note_key
        let mut last_rev = 0u64; // revision
        let mut last_hidden = 0u64; // hidden hash
        let mut revision: u64 = 1;
        let mut tv: Vec<bool> = vec![true; model.tracks.len()];
        let mut hidden: std::collections::HashSet<(u16, u32, u8)> =
            std::collections::HashSet::new();
        let note_key_of =
            |revision: u64, tv: &[bool], h: &std::collections::HashSet<(u16, u32, u8)>| {
                crate::NoteBufferKey::new(revision, tv, h).value()
            };

        // 帧处理：执行上传状态机 + dispatch + 读回 + 对比
        let mut frame_no = 0;
        let mut run_frame = |frame_no: &mut usize,
                             cull: &mut CullState,
                             model: &yinhe_core::YinModel,
                             revision: u64,
                             tv: &[bool],
                             hidden: &std::collections::HashSet<(u16, u32, u8)>,
                             scroll_x: f32|
         -> bool {
            *frame_no += 1;
            // ── 上传状态机（镜像 gpu_upload::upload）──
            let cull_was_ready = cull.per_key_bind_groups.iter().any(|bg| bg.is_some());
            if !cull_was_ready {
                last_key = 0;
            }
            let nk = note_key_of(revision, tv, hidden);
            let mut uploaded_kind = "skip";
            if nk != last_key {
                if !cull_was_ready {
                    let (all_notes, offsets) = crate::pianoroll::build_all_notes(model, hidden, tv);
                    cull.upload_all_notes(
                        &device,
                        &queue,
                        &uniform_buffer,
                        &all_notes,
                        &offsets,
                        &model.note_revisions,
                    );
                    uploaded_kind = "full";
                } else {
                    let revision_changed = revision != last_rev;
                    let hidden_changed = crate::hash_hidden(hidden) != last_hidden;
                    if hidden_changed && !revision_changed {
                        let (all_notes, offsets) =
                            crate::pianoroll::build_all_notes(model, hidden, tv);
                        cull.upload_all_notes(
                            &device,
                            &queue,
                            &uniform_buffer,
                            &all_notes,
                            &offsets,
                            &model.note_revisions,
                        );
                        uploaded_kind = "full(hidden)";
                    } else if revision_changed {
                        let dirty: Vec<u8> = (0u8..128)
                            .filter(|&k| {
                                model.note_revisions[k as usize]
                                    != cull.uploaded_key_revisions[k as usize]
                            })
                            .collect();
                        if dirty.is_empty() {
                            uploaded_kind = "none(rev-only)";
                        } else {
                            let mut all_ok = true;
                            for &k in &dirty {
                                let key_notes =
                                    crate::pianoroll::build_key_notes(model, k, hidden, tv);
                                if cull.per_key_buffers[k as usize].is_none() {
                                    all_ok = false;
                                    break;
                                }
                                cull.upload_one_key(
                                    &device,
                                    &queue,
                                    &uniform_buffer,
                                    k,
                                    &key_notes,
                                );
                                cull.uploaded_key_revisions[k as usize] =
                                    model.note_revisions[k as usize];
                            }
                            if all_ok {
                                uploaded_kind = "incremental";
                            } else {
                                let (all_notes, offsets) =
                                    crate::pianoroll::build_all_notes(model, hidden, tv);
                                cull.upload_all_notes(
                                    &device,
                                    &queue,
                                    &uniform_buffer,
                                    &all_notes,
                                    &offsets,
                                    &model.note_revisions,
                                );
                                uploaded_kind = "full(fallback)";
                            }
                        }
                    } else {
                        let (all_notes, offsets) =
                            crate::pianoroll::build_all_notes(model, hidden, tv);
                        cull.upload_all_notes(
                            &device,
                            &queue,
                            &uniform_buffer,
                            &all_notes,
                            &offsets,
                            &model.note_revisions,
                        );
                        uploaded_kind = "full(tv)";
                    }
                }
                last_key = nk;
                last_rev = revision;
                last_hidden = crate::hash_hidden(hidden);
            }

            // ── dispatch ──
            let u = Uniforms {
                width: w,
                height: h,
                scroll_x,
                scroll_y: 0.0,
                pixels_per_tick: ppu,
                key_height: kh,
                keyboard_width: kb_w,
                mode: 1,
                ..Default::default()
            };
            queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
            queue.submit([encoder.finish()]);

            // ── CPU 参考：镜像 shader X+Y 条件（用当前 model）──
            let xo = kb_w - scroll_x;
            let mut mismatches: Vec<(u32, u64, u64, i64)> = Vec::new();
            let mut gpu_total = 0u64;
            let mut cpu_total = 0u64;
            for key in 0..128u8 {
                let mut expected = 0u64;
                for n in model.notes[key as usize].iter() {
                    if n.end_tick > n.start_tick
                        && tv.get(n.track as usize).copied().unwrap_or(true)
                        && !hidden.contains(&(n.track, n.start_tick, key))
                    {
                        let px = xo + n.start_tick as f32 * ppu;
                        let pr = xo + n.end_tick as f32 * ppu;
                        if pr >= 0.0 && px <= w {
                            let k = key as f32;
                            let pb = bottom_y - k * kh;
                            let py = bottom_y - (k + 1.0) * kh;
                            if pb >= 0.0 && py <= h {
                                expected += 1;
                            }
                        }
                    }
                }
                cpu_total += expected;

                let chunk_count = cull.frame_chunk_counts[key as usize] as usize;
                let mut gpu = 0u64;
                if chunk_count > 0 {
                    let args_buf = cull.per_key_draw_args_buffers[key as usize]
                        .as_ref()
                        .expect("有 chunk 派发却没有 args buffer");
                    let readback = device.create_buffer(&BufferDescriptor {
                        label: Some("args_readback"),
                        size: 16 * chunk_count as u64,
                        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    let mut enc = device.create_command_encoder(&Default::default());
                    enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 16 * chunk_count as u64);
                    queue.submit([enc.finish()]);
                    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let done2 = done.clone();
                    readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                        done2.store(true, Ordering::SeqCst);
                    });
                    device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .expect("poll failed");
                    assert!(done.load(Ordering::SeqCst));
                    let view = readback.slice(..).get_mapped_range();
                    let args: &[u32] = bytemuck::cast_slice(&view);
                    for c in 0..chunk_count {
                        gpu += args[c * 4 + 1] as u64;
                    }
                    drop(view);
                    readback.unmap();
                }
                gpu_total += gpu;

                let diff = gpu as i64 - expected as i64;
                if diff.abs() > 2 {
                    mismatches.push((key as u32, expected, gpu, diff));
                }
            }
            println!(
                "帧{frame_no}: upload={uploaded_kind} scroll_x={scroll_x} GPU={gpu_total} CPU={cpu_total} 不匹配={}",
                mismatches.len()
            );
            if !mismatches.is_empty() {
                println!("  示例: {:?}", &mismatches[..mismatches.len().min(5)]);
                return false;
            }
            true
        };

        let mut ok = true;
        let total_ticks = model.tick_length as f32;
        // 帧 1：首次全量上传 + 开头视口
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            0.0,
        );
        // 帧 2：滚动到 25%
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            (total_ticks * 0.25 * ppu).max(0.0),
        );
        // 帧 3：滚动到 50%（跳过，note_key 不变 → upload=skip）
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            (total_ticks * 0.5 * ppu).max(0.0),
        );
        // 帧 4：相同视口（dispatch 的 skip 优化）
        let mid_scroll = (total_ticks * 0.5 * ppu).max(0.0);
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            mid_scroll,
        );
        // 帧 5：编辑 key 60（加一个音符）→ 增量上传
        {
            let k = 60u8;
            let start = (model.tick_length / 2) as u32;
            let id = model.alloc_note_id();
            std::sync::Arc::make_mut(&mut model.notes[k as usize]).push(yinhe_types::Note {
                id,
                start_tick: start,
                end_tick: start + 240,
                velocity: 100,
                track: 0,
            });
            model.mark_dirty(k);
            model.rebuild_dirty();
            revision = revision.wrapping_add(1);
        }
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            mid_scroll,
        );
        // 帧 6：切轨（隐藏 track 1-7）→ track_visible 全量
        for v in tv.iter_mut().take(8).skip(1) {
            *v = false;
        }
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            mid_scroll,
        );
        // 帧 7：hidden_notes 变化（全量）
        hidden.insert((0, 0, 60));
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            mid_scroll,
        );
        // 帧 8：滚动到 75%
        ok &= run_frame(
            &mut frame_no,
            &mut cull,
            &model,
            revision,
            &tv,
            &hidden,
            (total_ticks * 0.75 * ppu).max(0.0),
        );
        assert!(ok, "多帧交互序列存在 GPU/CPU 不匹配（见上方打印）");
    }
}
