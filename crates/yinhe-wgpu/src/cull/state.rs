//! GPU compute cull 状态机：per-key 音符 buffer + 间接 dispatch。
//! 拆分自原 cull.rs：本文件为 CullState 实现，tick 桶索引见 `bucket.rs`。

use wgpu::*;

use yinhe_types::{KEY_COUNT, MAX_KEY};

use crate::resource::{GpuBudget, GpuBudgetError, TrackedBuffer};
use crate::vertex::{NoteInstance, Uniforms};

use super::KeyBucketIndex;

pub(crate) fn culling_relevant_eq(a: &Uniforms, b: &Uniforms) -> bool {
    a.width == b.width
        && a.height == b.height
        && a.scroll_x == b.scroll_x
        && a.keyboard_width == b.keyboard_width
        && a.pixels_per_tick == b.pixels_per_tick
        && a.key_height == b.key_height
        && a.scroll_y == b.scroll_y
        && a.mode == b.mode
        && a.lane_height == b.lane_height
        && a.orientation == b.orientation
}
/// Viewport tick range the cull shader may consider visible, computed with
/// the same f32 math as `cull.wgsl` plus a margin of one pixel + 2 ticks, so
/// f32 rounding near the viewport edges never drops a bucket that the exact
/// shader test could still pass.
///
/// 横向（默认）：左边界 = keyboard_width 像素（cull.wgsl 的
/// `pixel_right >= keyboard_width`），即键盘列/轨道面板列之下的音符不参与
/// 渲染，chunk 调度范围随之左移。
/// 纵向：时间轴沿 Y，可见 tick 范围由 scroll_y / height 决定。
pub(crate) fn visible_tick_range(uniforms: &Uniforms) -> (u32, u32) {
    let ppu = uniforms.pixels_per_tick;
    let pad = (1.0 / ppu).ceil() as i64 + 2;
    if uniforms.orientation == 1 {
        let ts = (((uniforms.scroll_y) / ppu).floor() as i64 - pad)
            .max(0)
            .min(u32::MAX as i64);
        let te = (((uniforms.scroll_y + uniforms.height) / ppu).ceil() as i64 + pad)
            .max(0)
            .min(u32::MAX as i64);
        return (ts as u32, te as u32);
    }
    let x_offset = uniforms.keyboard_width - uniforms.scroll_x;
    let ts = (((uniforms.keyboard_width - x_offset) / ppu).floor() as i64 - pad)
        .max(0)
        .min(u32::MAX as i64);
    let te = (((uniforms.width - x_offset) / ppu).ceil() as i64 + pad)
        .max(0)
        .min(u32::MAX as i64);
    (ts as u32, te as u32)
}

pub(crate) struct CullState {
    pipeline: ComputePipeline,
    /// 供 note_pipeline 复用为 render pipeline 的 group 1（顶点阶段索引间接读）。
    pub(crate) bind_group_layout: BindGroupLayout,
    /// 顶点阶段专用的「all_instances 只读」bind group layout（单 binding）。
    /// 与 cull bind group 分离：render pass 引用它不会把 visible_indices
    /// （STORAGE_READ_WRITE，exclusive usage）带进 render scope 与 vertex
    /// buffer 冲突。
    pub(crate) all_bind_group_layout: BindGroupLayout,
    /// Per-key bind groups (KEY_COUNT slots). `None` until the key is first uploaded.
    pub(crate) per_key_bind_groups: Vec<Option<BindGroup>>,
    /// Per-key vertex-stage bind groups（只含 all_instances）。
    per_key_all_bind_groups: Vec<Option<BindGroup>>,
    /// Per-key all-notes storage buffers (cull input), grown on demand.
    pub(crate) per_key_buffers: Vec<Option<TrackedBuffer>>,
    /// Per-key visible-index buffers (cull output: 4B u32 indices into the
    /// key's `all_instances`; draw vertex source). 256-aligned so every
    /// chunk's sparse slots [chunk*256, chunk*256+256) fit; visible slots
    /// beyond the written count hold stale data and are not drawn (draw args
    /// bound instance_count to the culled count).
    /// 索引化（12B → 4B/槽）使全曲稀疏槽位显存降到 1/3；顶点阶段经
    /// shader.wgsl 的 @group(1) 从 all_instances 间接读回完整数据。
    pub(crate) per_key_visible_buffers: Vec<Option<TrackedBuffer>>,
    /// Per-key draw args buffer（每 chunk 一个 DrawIndexedIndirectArgs，20B）。
    /// 由 cull shader 每 chunk 的线程 0 写入（chunk 顺序 = 输入顺序）；
    /// 每帧读回 CPU 缓存（`per_key_draw_args_cpu`）供直接 draw。
    ///
    /// **注意**：Adreno 730 驱动的 draw_indexed_indirect 整体失效
    /// （CPU 手写 args 依然 0 像素，真机实测），因此
    /// 不走 indirect draw，args 读回 CPU 后循环直接 `draw_indexed`。
    pub(crate) per_key_draw_args_buffers: Vec<Option<TrackedBuffer>>,
    /// Per-key 本帧 chunk args 的 CPU 缓存：每 chunk
    /// `(instance_count, first_instance)`，由 `readback_args_to_cpu` 填充，
    /// `draw_visible_notes` 直接 draw 用。skip 帧保持上次值（args 未变）。
    per_key_draw_args_cpu: Vec<Vec<(u32, u32)>>,
    /// Per-track visibility bitmask (1 bit per track, MAX_TRACKS bits).
    /// Written by `upload_track_mask`; read by cull.wgsl (binding 5).
    /// Fixed-size (MAX_TRACKS/8 bytes) so per-key bind groups never need
    /// recreating on track-count growth.
    track_mask_buffer: TrackedBuffer,
    /// Chunk count dispatched for each key in the current frame (0 = none).
    /// Filled by `dispatch_cull`, read by `draw_visible_notes`.
    pub(crate) frame_chunk_counts: [u32; KEY_COUNT],

    /// Per-key note count at last upload (in NoteInstance units).
    per_key_counts: [u32; KEY_COUNT],

    /// Per-key tick-bucket index (CPU side), built at upload time. Used each
    /// frame to dispatch only the chunks whose tick range can intersect the
    /// viewport — GPU traffic per frame drops from O(all notes) to O(visible).
    pub(crate) bucket_indexes: Vec<Option<KeyBucketIndex>>,

    /// Per-key dispatch args buffer: KEY_COUNT slots × 256 bytes each.
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
    dispatch_args_buffer: TrackedBuffer,
    /// 显存预算守卫：per-key buffer 增长前检查，超限返回错误而非 wgpu panic。
    budget: GpuBudget,

    /// Per-key revision at last upload (full or incremental).
    /// Compared with model.note_revisions to detect incremental re-upload needs.
    pub(crate) uploaded_key_revisions: [u64; KEY_COUNT],

    /// Uniforms snapshot from the last cull dispatch. When the culling-relevant
    /// fields match and `notes_dirty` is false, the previous frame's
    /// `visible_notes` + `indirect_args` are still valid and the dispatch can
    /// be skipped entirely.
    last_cull_uniforms: Option<Uniforms>,

    /// True when note data has been uploaded (full or incremental) since the
    /// last cull dispatch. Set by `upload_all_notes` / `upload_one_key`;
    /// cleared by `dispatch_cull`.
    notes_dirty: bool,

    /// 桌面走 GPU 间接绘制（零回读），Android Adreno 驱动间接失效走回读
    use_indirect: bool,
}

impl CullState {
    pub(crate) fn new(device: &Device) -> Self {
        let cull_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("cull_shader"),
            source: ShaderSource::Wgsl(include_str!("../cull.wgsl").into()),
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
                BindGroupLayoutEntry {
                    binding: 5,
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

        // 顶点阶段专用：单 binding 的 all_instances 只读 layout。
        // 与 cull bind group 分离，render pass 引用时不会把 exclusive 的
        // STORAGE_READ_WRITE（visible_indices）带进同一 usage scope。
        let all_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("all_instances_bind_group_layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
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

        // Dispatch args + per-key note count + c_lo, KEY_COUNT slots × 256 bytes each.
        // Slot layout: [wg_x, wg_y, wg_z, count, c_lo] (20 bytes) + padding.
        // The 256-byte stride satisfies min_storage_buffer_offset_alignment
        // (typically 256). wg_x/wg_y/c_lo are host-written every frame by
        // `dispatch_cull`; `count` bounds the cull scan in cull.wgsl (index < count).
        let dispatch_args_size = (KEY_COUNT * 256) as u64;
        let dispatch_args_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("cull_dispatch_args"),
                size: dispatch_args_size,
                usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        // Track visibility bitmask: fixed size = MAX_TRACKS bits (8 KB), so
        // per-key bind groups bind a stable buffer and never need recreating
        // when the track count grows. Initialized to all-visible.
        let track_mask_size = crate::vertex::MAX_TRACKS as u64 / 8;
        let track_mask_buffer = TrackedBuffer::new(
            device,
            &BufferDescriptor {
                label: Some("cull_track_mask"),
                size: track_mask_size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: true,
            },
        );
        {
            let words = vec![u32::MAX; crate::vertex::MAX_TRACKS / 32];
            let Ok(mut mapped) = track_mask_buffer.slice(..).get_mapped_range_mut() else {
                // mapped_at_creation buffer 映射失败 = 设备不可用，无法继续渲染
                panic!("track_mask buffer not mappable at creation");
            };
            mapped.copy_from_slice(bytemuck::cast_slice(&words));
        }
        track_mask_buffer.unmap();

        // Android Adreno 间接绘制失效，强制回读；桌面走间接零回读（需 INDIRECT_FIRST_INSTANCE）
        let use_indirect = !cfg!(target_os = "android")
            && device
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        Self {
            pipeline,
            bind_group_layout,
            all_bind_group_layout,
            per_key_bind_groups: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_all_bind_groups: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_buffers: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_visible_buffers: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_draw_args_buffers: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_draw_args_cpu: (0..KEY_COUNT).map(|_| Vec::new()).collect(),
            track_mask_buffer,
            frame_chunk_counts: [0; KEY_COUNT],
            per_key_counts: [0; KEY_COUNT],
            bucket_indexes: (0..KEY_COUNT).map(|_| None).collect(),
            dispatch_args_buffer,
            budget: GpuBudget::new(device),
            uploaded_key_revisions: [0; KEY_COUNT],
            last_cull_uniforms: None,
            notes_dirty: false,
            use_indirect,
        }
    }

    /// Update the per-track visibility bitmask. Called whenever track_visible
    /// changes (before/while the background full rebuild runs), so the cull
    /// shader immediately stops emitting hidden tracks' notes from the current
    /// (possibly stale) buffers.
    ///
    /// Marks `notes_dirty` so the next `dispatch_cull` re-runs even when the
    /// culling-relevant uniforms are unchanged (mask change must invalidate
    /// the skip optimization).
    pub(crate) fn upload_track_mask(&mut self, queue: &Queue, track_visible: &[bool]) {
        // 一次性打包：默认全 1，逐位清隐藏轨道。8KB 固定 buffer（MAX_TRACKS/8）。
        let mut words = vec![u32::MAX; crate::vertex::MAX_TRACKS / 32];
        for (i, &v) in track_visible
            .iter()
            .take(crate::vertex::MAX_TRACKS)
            .enumerate()
        {
            if !v {
                words[i / 32] &= !(1u32 << (i % 32));
            }
        }
        queue.write_buffer(&self.track_mask_buffer, 0, bytemuck::cast_slice(&words));
        self.notes_dirty = true;
    }

    /// Upload notes for all KEY_COUNT keys. `notes` is a flat buffer; `per_key_offsets`
    /// slices it into per-key segments. Each key gets its own storage buffer
    /// (grown on demand) and bind group, keeping every binding under the
    /// `max_storage_buffer_binding_size` limit regardless of total note count.
    pub(crate) fn upload_all_notes(
        &mut self,
        device: &Device,
        queue: &Queue,
        uniform_buffer: &Buffer,
        notes: &[NoteInstance],
        per_key_offsets: &[u32; KEY_COUNT + 1],
        key_revisions: &[u64; KEY_COUNT],
    ) -> Result<(), GpuBudgetError> {
        for key in 0u8..=MAX_KEY {
            let start = per_key_offsets[key as usize] as usize;
            let end = per_key_offsets[key as usize + 1] as usize;
            let key_notes = &notes[start..end];
            self.upload_one_key(device, queue, uniform_buffer, key, key_notes)?;
            self.uploaded_key_revisions[key as usize] = key_revisions[key as usize];
        }
        self.notes_dirty = true;
        Ok(())
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
    ) -> Result<(), GpuBudgetError> {
        // 空 key 不建缓冲：省 3 次 create_buffer + bind_group，首帧 128→非空数
        if notes.is_empty() {
            for buf in [
                &mut self.per_key_buffers[key as usize],
                &mut self.per_key_visible_buffers[key as usize],
                &mut self.per_key_draw_args_buffers[key as usize],
            ] {
                buf.take();
            }
            self.per_key_bind_groups[key as usize] = None;
            self.per_key_all_bind_groups[key as usize] = None;
            self.per_key_counts[key as usize] = 0;
            self.per_key_draw_args_cpu[key as usize].clear();
            self.frame_chunk_counts[key as usize] = 0;
            self.bucket_indexes[key as usize] = None;
            self.notes_dirty = true;
            return Ok(());
        }
        let needed = notes.len() as u64 * std::mem::size_of::<NoteInstance>() as u64;
        let chunk_total = (notes.len() as u64).div_ceil(256).max(1);
        // Visible buffer is 256-aligned so every chunk's sparse slots
        // [chunk*256, chunk*256+256) fit; slots are 4B u32 indices now.
        let vis_size = chunk_total * 256 * std::mem::size_of::<u32>() as u64;
        // DrawIndexedIndirectArgs = 5 × u32 = 20B per chunk.
        let args_size = chunk_total * std::mem::size_of::<u32>() as u64 * 5;

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
            //   visible:   大小 vis_size（4B 索引），usage STORAGE | VERTEX
            //   draw_args: 大小 args_size，usage STORAGE | COPY_SRC | COPY_DST
            // 只释放当前 key 的三个 buffer——不能遍历全部 keys，那会销毁
            // 其他 key 的 buffer，全量上传后只剩最后一个 key 存活。
            // 旧 buffer 随 take 立即 drop，TrackedBuffer 自动 sub_gpu_resource。
            for buf in [
                &mut self.per_key_buffers[key as usize],
                &mut self.per_key_visible_buffers[key as usize],
                &mut self.per_key_draw_args_buffers[key as usize],
            ] {
                buf.take();
            }

            // 显存预算检查（在旧 buffer 释放后检查：used 已减掉旧 buffer 占用量）。
            // 超限时返回错误，上层记录日志并降级（不 panic、不创建超限 buffer）。
            let all_size = needed.max(4096);
            self.budget.reserve(all_size + vis_size + args_size)?;
            let all_buf = TrackedBuffer::new(
                device,
                &BufferDescriptor {
                    label: Some("all_notes_key"),
                    size: all_size,
                    usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            self.per_key_buffers[key as usize] = Some(all_buf);

            let vis_buf = TrackedBuffer::new(
                device,
                &BufferDescriptor {
                    label: Some("visible_indices_key"),
                    size: vis_size,
                    // COPY_SRC so tests can read back the culled output;
                    // COPY_DST so tests can overwrite slots directly.
                    usage: BufferUsages::STORAGE
                        | BufferUsages::VERTEX
                        | BufferUsages::COPY_SRC
                        | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            self.per_key_visible_buffers[key as usize] = Some(vis_buf);

            let args_buf = TrackedBuffer::new(
                device,
                &BufferDescriptor {
                    label: Some("draw_args_key"),
                    size: args_size,
                    // COPY_SRC 供 Adreno 回读 + 诊断；INDIRECT 供桌面直接 draw_indexed_indirect
                    usage: BufferUsages::STORAGE
                        | BufferUsages::INDIRECT
                        | BufferUsages::COPY_SRC
                        | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            self.per_key_draw_args_buffers[key as usize] = Some(args_buf);

            self.recreate_cull_bind_group(device, uniform_buffer, key);
        }

        // GPU 写（memcpy 到 staging）与 tick 索引重建并行执行：编辑场景两者
        // 都是 O(key 音符数)，重叠后每帧只付 max(write, build) 而非两者之和。
        let index = rayon::join(
            || {
                if !notes.is_empty()
                    && let Some(ref buf) = self.per_key_buffers[key as usize]
                {
                    queue.write_buffer(buf, 0, bytemuck::cast_slice(notes));
                }
            },
            || KeyBucketIndex::build(notes),
        )
        .1;
        self.per_key_counts[key as usize] = notes.len() as u32;

        // Rebuild the tick-bucket index for this key (notes are sorted by
        // start_tick). Rebuilt on every upload, including shrunk keys.
        self.bucket_indexes[key as usize] = Some(index);

        self.notes_dirty = true;
        Ok(())
    }

    /// Whether a per-key buffer exists for `key` (incremental upload precondition).
    pub(crate) fn has_key_buffer(&self, key: u8) -> bool {
        self.per_key_buffers[key as usize].is_some()
    }

    pub(crate) fn use_indirect(&self) -> bool {
        self.use_indirect
    }

    /// The per-key vertex-stage bind group (only `all_instances`).
    /// Render pass 用它做索引间接读，不会把 exclusive 的 visible_indices
    /// usage 带进 render scope。
    pub(crate) fn per_key_all_bind_group(&self, key: u8) -> Option<&BindGroup> {
        self.per_key_all_bind_groups[key as usize].as_ref()
    }

    /// Recreate the bind group for a single key (after its buffer grew).
    /// Binds: uniform, all_notes[k], visible_notes[k], per-key draw_args[k],
    /// and the dispatch-args slot k (256-byte slice at offset k*256).
    fn recreate_cull_bind_group(&mut self, device: &Device, uniform_buffer: &Buffer, key: u8) {
        // TrackedBuffer 有意不实现 Clone，bind group 只需借用引用。
        let all_buf = match &self.per_key_buffers[key as usize] {
            Some(b) => b,
            None => return,
        };
        let vis_buf = match &self.per_key_visible_buffers[key as usize] {
            Some(b) => b,
            None => return,
        };
        let args_buf = match &self.per_key_draw_args_buffers[key as usize] {
            Some(b) => b,
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
                    BindGroupEntry {
                        binding: 5,
                        resource: self.track_mask_buffer.as_entire_binding(),
                    },
                ],
            }));
        // 顶点阶段专用 bind group（只含 all_instances）：grow 后必须重建。
        self.per_key_all_bind_groups[key as usize] =
            Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("all_instances_bind_group"),
                layout: &self.all_bind_group_layout,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: all_buf.as_entire_binding(),
                }],
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
            // take 即 drop，TrackedBuffer 自动 sub_gpu_resource。
            buf.take();
        }
        self.per_key_bind_groups.fill(None);
        self.per_key_all_bind_groups.fill(None);
        self.per_key_counts.fill(0);
        self.frame_chunk_counts.fill(0);
        self.bucket_indexes.fill(None);
        self.per_key_draw_args_cpu.iter_mut().for_each(Vec::clear);
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
    /// into the dispatch-args slot every frame (one blob for all KEY_COUNT
    /// keys). `count` stays cached on the CPU.
    ///
    /// **Skip optimization**: if no notes were re-uploaded (`!notes_dirty`) and
    /// the culling-relevant uniform fields match the last dispatch, the previous
    /// frame's `visible_notes` + per-key draw args are still valid and the
    /// entire dispatch is skipped. This makes idle frames (no scroll, no edit)
    /// cost zero GPU compute work.
    /// 是否跳过本帧 dispatch（uniform 与 notes 都没变，上帧输出仍有效）。
    pub(crate) fn dispatch_cull(
        &mut self,
        encoder: &mut CommandEncoder,
        queue: &Queue,
        key_lo: u8,
        key_hi: u8,
        uniforms: &Uniforms,
    ) -> bool {
        // Skip if nothing changed since last cull.
        if !self.notes_dirty
            && self
                .last_cull_uniforms
                .as_ref()
                .is_some_and(|last| culling_relevant_eq(last, uniforms))
        {
            return false;
        }

        // Viewport tick range, computed with the same f32 math as the shader
        // plus a small margin so the bucket query stays conservative (extra
        // buckets are harmless — the shader AABB test is exact).
        let (tick_start, tick_end) = visible_tick_range(uniforms);

        // Per-key dispatch info: (wg_x, wg_y, wg_z=1, count, c_lo). wg_x/wg_y
        // and c_lo change every frame; count is cached on the CPU. Written as
        // one blob per frame instead of KEY_COUNT small writes.
        let mut info = [0u32; KEY_COUNT * 64];
        for key in 0..KEY_COUNT {
            let slot = key * 64;
            // visible_chunk_range 返回 (c_lo, c_hi) 区间，chunk 数 = c_hi - c_lo。
            let (c_lo, c_hi) = self.bucket_indexes[key]
                .as_ref()
                .and_then(|idx| idx.visible_chunk_range(tick_start, tick_end))
                .unwrap_or((0, 0));
            let chunk_count = c_hi - c_lo;
            info[slot] = chunk_count.min(65535);
            info[slot + 1] = chunk_count.div_ceil(65535);
            info[slot + 2] = 1;
            info[slot + 3] = self.per_key_counts[key];
            info[slot + 4] = c_lo;
            self.frame_chunk_counts[key] = chunk_count;
        }
        let mut dispatched_keys = 0u32;
        let mut total_chunks = 0u32;
        for key in 0..KEY_COUNT {
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
        true
    }

    /// 把本帧 GPU cull 写出的 args 同步读回 CPU 缓存（`per_key_draw_args_cpu`），
    /// 供 `draw_visible_notes` 直接 draw 用。
    ///
    /// **为什么读回而不是 indirect draw？** Adreno 730 驱动的
    /// `draw_indexed_indirect` 整体失效（真机实测：CPU 手写
    /// args [6,1,0,0,0]、纯 INDIRECT usage 依然 0 像素），而直接 `draw_indexed`
    /// 完全正常。跨 submit 读回则一直稳定（STORAGE→COPY_SRC barrier 正常）。
    ///
    /// 每帧派发的 chunk args 总量很小（可见 chunk × 20B，通常 < 2KB），
    /// 同步读回开销可忽略；skip 帧不调用（args 未变，缓存仍有效）。
    pub(crate) fn readback_args_to_cpu(&mut self, device: &Device, queue: &Queue) {
        // 每 key 一个 readback buffer + 一个 copy 命令，合并为一次提交。
        let mut readbacks: Vec<Option<TrackedBuffer>> = Vec::with_capacity(KEY_COUNT);
        let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
        for key in 0u8..=MAX_KEY {
            let chunk_count = self.frame_chunk_counts[key as usize] as usize;
            if chunk_count == 0 {
                // 本帧不可见的 key：清空 CPU 缓存，防止 draw 画旧 args。
                self.per_key_draw_args_cpu[key as usize].clear();
                readbacks.push(None);
                continue;
            }
            let Some(src) = &self.per_key_draw_args_buffers[key as usize] else {
                self.per_key_draw_args_cpu[key as usize].clear();
                readbacks.push(None);
                continue;
            };
            let bytes = 20 * chunk_count as u64;
            let readback = TrackedBuffer::new(
                device,
                &BufferDescriptor {
                    label: Some("args_sync_readback"),
                    size: bytes,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                },
            );
            enc.copy_buffer_to_buffer(src, 0, &readback, 0, bytes);
            readbacks.push(Some(readback));
        }
        if readbacks.is_empty() || readbacks.iter().all(Option::is_none) {
            return;
        }
        queue.submit([enc.finish()]);

        // 先全部 map_async，再 poll 一次等全部完成：逐个 poll 会让每个 key
        // 都等一次 GPU 全队列，真机上每帧几十次同步等待 → 帧率骤降。
        let mut pending: Vec<(u8, std::sync::Arc<std::sync::atomic::AtomicBool>)> = Vec::new();
        for key in 0u8..=MAX_KEY {
            let Some(readback) = &readbacks[key as usize] else {
                continue;
            };
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            pending.push((key, done));
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        for (key, done) in pending {
            if !done.load(std::sync::atomic::Ordering::SeqCst) {
                continue;
            }
            let Some(readback) = &readbacks[key as usize] else {
                continue;
            };
            let Ok(view) = readback.slice(..).get_mapped_range() else {
                continue;
            };
            // 每 chunk 20B：(index_count=6, instance_count, first_index=0,
            // base_vertex=0, first_instance) → 只留 draw 需要的两个字段。
            let args: Vec<(u32, u32)> = view
                .chunks_exact(20)
                .map(|c| {
                    (
                        u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                        u32::from_le_bytes([c[16], c[17], c[18], c[19]]),
                    )
                })
                .collect();
            drop(view);
            readback.unmap();
            self.per_key_draw_args_cpu[key as usize] = args;
        }
    }

    /// Draw the culled notes：循环 CPU 缓存的 chunk args 直接 `draw_indexed`。
    /// chunks 按输入（=tick）顺序写入，draw 顺序相同，z-order 帧间确定。
    ///
    /// Per-key bind group (@group(1)) is set before each key's draw so the
    /// vertex shader can indirect-read that key's `all_instances` from the
    /// 4-byte visible index. 4 顶点 + 共享 index buffer（顶点 -33%）。
    pub(crate) fn draw_visible_notes(
        &self,
        pass: &mut RenderPass<'_>,
        note_pipeline: &RenderPipeline,
        bind_group: &BindGroup,
        index_buffer: &Buffer,
        key_lo: u8,
        key_hi: u8,
    ) {
        pass.set_pipeline(note_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint32);
        for key in key_lo..=key_hi {
            let Some(vis_buf) = &self.per_key_visible_buffers[key as usize] else {
                continue;
            };
            let chunks = &self.per_key_draw_args_cpu[key as usize];
            if chunks.is_empty() {
                continue;
            }
            let Some(bg) = self.per_key_all_bind_group(key) else {
                continue;
            };
            pass.set_bind_group(1, bg, &[]);
            pass.set_vertex_buffer(0, vis_buf.slice(..));
            // Adreno 驱动的 draw_indexed_indirect 整体失效（真机实测），
            // 用 CPU 读回的 args 直接 draw。instances range 的 start 即
            // first_instance（顶点流索引 = start + instance_index），与
            // indirect args 的语义一致，且属 wgpu 核心行为，无需任何 feature。
            for &(instance_count, first_instance) in chunks {
                if instance_count > 0 {
                    pass.draw_indexed(0..6, 0, first_instance..first_instance + instance_count);
                }
            }
        }
    }

    /// 桌面间接绘制：零回读，直接 `draw_indexed_indirect`（需 INDIRECT_FIRST_INSTANCE）
    pub(crate) fn draw_visible_notes_indirect(
        &self,
        pass: &mut RenderPass<'_>,
        note_pipeline: &RenderPipeline,
        bind_group: &BindGroup,
        index_buffer: &Buffer,
        key_lo: u8,
        key_hi: u8,
    ) {
        pass.set_pipeline(note_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint32);
        for key in key_lo..=key_hi {
            let Some(vis_buf) = &self.per_key_visible_buffers[key as usize] else {
                continue;
            };
            let Some(args_buf) = &self.per_key_draw_args_buffers[key as usize] else {
                continue;
            };
            let chunk_count = self.frame_chunk_counts[key as usize] as usize;
            if chunk_count == 0 {
                continue;
            }
            let Some(bg) = self.per_key_all_bind_group(key) else {
                continue;
            };
            pass.set_bind_group(1, bg, &[]);
            pass.set_vertex_buffer(0, vis_buf.slice(..));
            for chunk in 0..chunk_count {
                pass.draw_indexed_indirect(args_buf, chunk as u64 * 20);
            }
        }
    }
}
