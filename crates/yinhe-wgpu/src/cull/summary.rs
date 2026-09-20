//! 摘要层（LOD）GPU 资源：每个档位持有与原始层同构的 per-key buffers。
//!
//! 摘要层复用 `cull.wgsl`（同样的 visible 判定 / 前缀和 / 稀疏槽输出），
//! 区别只是数据量小、没有 tick 桶索引（全量 dispatch：摘要只在 ppu 很小、
//! 视口覆盖大部分时间轴时启用）。
//!
//! 与原始层共享的 GPU 资源（uniform、bind group layout、track mask、
//! dispatch args）通过 [`CullShared`] 传入。

use wgpu::*;
use yinhe_types::KEY_COUNT;

use crate::resource::{GpuBudget, GpuBudgetError, TrackedBuffer};
use crate::vertex::NoteInstance;

use super::KeyBucketIndex;

/// CullState 与摘要层共享的 GPU 资源。
pub(crate) struct CullShared<'a> {
    pub uniform_buffer: &'a Buffer,
    pub cull_layout: &'a BindGroupLayout,
    pub all_layout: &'a BindGroupLayout,
    pub track_mask: &'a TrackedBuffer,
    pub dispatch_args: &'a TrackedBuffer,
}

/// 摘要层的 workgroup/chunk 大小，与 cull.wgsl 的 @workgroup_size 一致。
pub(crate) const SUMMARY_CHUNK: u64 = 256;

/// 一个摘要档位的全部 per-key GPU 资源。
pub(crate) struct SummaryLevel {
    pub(crate) per_key_buffers: Vec<Option<TrackedBuffer>>,
    pub(crate) per_key_visible_buffers: Vec<Option<TrackedBuffer>>,
    pub(crate) per_key_draw_args_buffers: Vec<Option<TrackedBuffer>>,
    pub(crate) per_key_bind_groups: Vec<Option<BindGroup>>,
    pub(crate) per_key_all_bind_groups: Vec<Option<BindGroup>>,
    pub(crate) per_key_counts: [u32; KEY_COUNT],
    /// 每 key 的总 chunk 数（buffer 容量单位）。
    pub(crate) per_key_chunks: [u32; KEY_COUNT],
    /// 本帧实际 dispatch 的 chunk 数（桶索引裁剪后），draw 的 multi_draw
    /// count 用它；args 每帧从 0 开始写。
    pub(crate) frame_chunks: [u32; KEY_COUNT],
    /// 每 key 的 tick 桶索引：只 dispatch 与视口相交的 chunk（与原始层同款）。
    pub(crate) bucket_indexes: Vec<Option<KeyBucketIndex>>,
}

impl SummaryLevel {
    pub(crate) fn new() -> Self {
        Self {
            per_key_buffers: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_visible_buffers: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_draw_args_buffers: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_bind_groups: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_all_bind_groups: (0..KEY_COUNT).map(|_| None).collect(),
            per_key_counts: [0; KEY_COUNT],
            per_key_chunks: [0; KEY_COUNT],
            frame_chunks: [0; KEY_COUNT],
            bucket_indexes: (0..KEY_COUNT).map(|_| None).collect(),
        }
    }

    /// 是否至少有一个 key 已上传（选择该层的前提）。
    pub(crate) fn is_ready(&self) -> bool {
        self.per_key_bind_groups.iter().any(|bg| bg.is_some())
    }

    pub(crate) fn clear(&mut self) {
        for buf in self
            .per_key_buffers
            .iter_mut()
            .chain(self.per_key_visible_buffers.iter_mut())
            .chain(self.per_key_draw_args_buffers.iter_mut())
        {
            buf.take();
        }
        self.per_key_bind_groups.fill(None);
        self.per_key_all_bind_groups.fill(None);
        self.per_key_counts.fill(0);
        self.per_key_chunks.fill(0);
        self.frame_chunks.fill(0);
        self.bucket_indexes.fill(None);
    }

    /// 上传单 key 的摘要段（空则释放该 key 的资源）。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub(crate) fn upload_key(
        &mut self,
        device: &Device,
        queue: &Queue,
        shared: &CullShared<'_>,
        budget: &GpuBudget,
        key: u8,
        notes: &[NoteInstance],
    ) -> Result<(), GpuBudgetError> {
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
            self.per_key_chunks[key as usize] = 0;
            self.frame_chunks[key as usize] = 0;
            self.bucket_indexes[key as usize] = None;
            return Ok(());
        }

        let needed = notes.len() as u64 * std::mem::size_of::<NoteInstance>() as u64;
        let chunk_total = (notes.len() as u64).div_ceil(SUMMARY_CHUNK).max(1);
        let vis_size = chunk_total * SUMMARY_CHUNK * std::mem::size_of::<u32>() as u64;
        let args_size = chunk_total * 20;

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
            for buf in [
                &mut self.per_key_buffers[key as usize],
                &mut self.per_key_visible_buffers[key as usize],
                &mut self.per_key_draw_args_buffers[key as usize],
            ] {
                buf.take();
            }
            let all_size = needed.max(4096);
            budget.reserve(all_size + vis_size + args_size)?;
            let all_buf = TrackedBuffer::new(
                device,
                &BufferDescriptor {
                    label: Some("summary_notes_key"),
                    size: all_size,
                    usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            let vis_buf = TrackedBuffer::new(
                device,
                &BufferDescriptor {
                    label: Some("summary_visible_key"),
                    size: vis_size,
                    usage: BufferUsages::STORAGE
                        | BufferUsages::VERTEX
                        | BufferUsages::COPY_SRC
                        | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            let args_buf = TrackedBuffer::new(
                device,
                &BufferDescriptor {
                    label: Some("summary_draw_args_key"),
                    size: args_size,
                    usage: BufferUsages::STORAGE
                        | BufferUsages::INDIRECT
                        | BufferUsages::COPY_SRC
                        | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            self.per_key_buffers[key as usize] = Some(all_buf);
            self.per_key_visible_buffers[key as usize] = Some(vis_buf);
            self.per_key_draw_args_buffers[key as usize] = Some(args_buf);
            self.recreate_bind_groups(device, shared, key);
        }

        if let Some(buf) = &self.per_key_buffers[key as usize] {
            queue.write_buffer(buf, 0, bytemuck::cast_slice(notes));
        }
        self.per_key_counts[key as usize] = notes.len() as u32;
        self.per_key_chunks[key as usize] = chunk_total as u32;
        self.bucket_indexes[key as usize] = Some(KeyBucketIndex::build(notes));
        Ok(())
    }

    fn recreate_bind_groups(&mut self, device: &Device, shared: &CullShared<'_>, key: u8) {
        let Some(all_buf) = &self.per_key_buffers[key as usize] else {
            return;
        };
        let Some(vis_buf) = &self.per_key_visible_buffers[key as usize] else {
            return;
        };
        let Some(args_buf) = &self.per_key_draw_args_buffers[key as usize] else {
            return;
        };
        self.per_key_bind_groups[key as usize] =
            Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("summary_cull_bind_group"),
                layout: shared.cull_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: shared.uniform_buffer.as_entire_binding(),
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
                            buffer: shared.dispatch_args,
                            offset: key as u64 * 256,
                            size: std::num::NonZeroU64::new(256),
                        }),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: shared.track_mask.as_entire_binding(),
                    },
                ],
            }));
        self.per_key_all_bind_groups[key as usize] =
            Some(device.create_bind_group(&BindGroupDescriptor {
                label: Some("summary_all_bind_group"),
                layout: shared.all_layout,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: all_buf.as_entire_binding(),
                }],
            }));
    }

    /// 把本帧各 key 的 dispatch 参数写入共享 dispatch args buffer。
    ///
    /// 用 tick 桶索引把 dispatch 限制在视口相交的 chunk（与原始层同款）：
    /// 细档（2/4/8）全量段数可达千万，逐帧全量 dispatch 的 compute 成本
    /// 会让放大到局部时反而比原始层慢。`tick_start/tick_end` 由调用方用
    /// `visible_tick_range(uniforms)` 计算。
    pub(crate) fn write_dispatch_info(
        &mut self,
        queue: &Queue,
        dispatch_args: &TrackedBuffer,
        tick_start: u32,
        tick_end: u32,
    ) {
        let mut info = [0u32; KEY_COUNT * 64];
        for key in 0..KEY_COUNT {
            let slot = key * 64;
            let (c_lo, c_hi) = self.bucket_indexes[key]
                .as_ref()
                .and_then(|idx| idx.visible_chunk_range(tick_start, tick_end))
                .unwrap_or((0, 0));
            let chunks = c_hi - c_lo;
            self.frame_chunks[key] = chunks;
            info[slot] = chunks.min(65535);
            info[slot + 1] = chunks.div_ceil(65535);
            info[slot + 2] = 1;
            info[slot + 3] = self.per_key_counts[key];
            info[slot + 4] = c_lo;
        }
        queue.write_buffer(dispatch_args, 0, bytemuck::cast_slice(&info));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_resets_all_state() {
        let mut level = SummaryLevel::new();
        level.per_key_counts[3] = 10;
        level.per_key_chunks[3] = 1;
        level.clear();
        assert!(!level.is_ready());
        assert_eq!(level.per_key_counts[3], 0);
        assert_eq!(level.per_key_chunks[3], 0);
    }
}
