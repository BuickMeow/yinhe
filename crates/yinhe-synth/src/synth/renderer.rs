//! GPU 渲染器：管线创建与块渲染。

use std::sync::Arc;

use wgpu::util::DeviceExt;

use super::buffers::{BufferSpec, GpuBuffers};
use super::types::{
    CHANNEL_COUNT, ChState, EnvUpdateCmd, GpuVoiceState, MAX_CHUNKS, RENDER_SEGMENT_FRAMES,
    ReleaseCmd, RenderParams, RenderSegment, SegInfo, WORKGROUP_SIZE,
};

/// GPU-accelerated audio renderer with persistent buffers.
pub struct GpuAudioRenderer {
    pub(crate) device: Arc<wgpu::Device>,
    pub(crate) queue: Arc<wgpu::Queue>,
    pub(crate) pipeline: wgpu::ComputePipeline, // pass1: 每 voice 串行帧
    pub(crate) mix_pipeline: wgpu::ComputePipeline, // pass2: 归约 partial
    pub(crate) scatter_pipeline: wgpu::ComputePipeline, // 状态散写（替代逐区间 copy）
    #[allow(dead_code)]
    pub(crate) pipeline_layout: wgpu::PipelineLayout,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) dummy_buf: wgpu::Buffer,
    pub(crate) buffers: Option<GpuBuffers>,
    /// 采样数据（连续大块，Arc 与拼接缓存共享同一份内存；chunk 切片直接在
    /// `ensure_buffers` 里上传，不再预切成 `Vec<Vec<f32>>` 以避免额外拷贝）。
    pub(crate) sample_data: Arc<Vec<f32>>,
    /// 采样 GPU buffer（跨 GpuBuffers 重建复用：voice/帧数扩容触发的重建
    /// 不再重传采样数据）。`upload_samples` 置 None 表示数据已更新。
    pub(crate) sample_buffers: Option<Vec<wgpu::Buffer>>,
    /// 采样 GPU 上传次数（测试回归：扩容重建不应重传）。
    #[cfg(test)]
    pub(crate) sample_upload_count: usize,
    pub(crate) frame_count: u32,
    /// voice 槽位容量（实时默认 MAX_VOICE_SLOTS；导出按需扩容）。
    pub(crate) voice_capacity: u32,
    /// render_into 的 per-channel 混音临时缓冲（复用，避免每块分配）
    pub(crate) mix_scratch: Vec<f32>,
    /// 每段 release 指令的按帧前缀和（复用，避免每段分配）
    pub(crate) release_by_frame_scratch: Vec<u32>,
    /// 按槽位索引的 voice 状态镜像（write_voice_state 直接更新；
    /// 同一 vid 多次写入天然去重，flush 只处理脏列表）。
    voice_state_mirror: Vec<GpuVoiceState>,
    /// 各槽位是否在脏列表中（与 mirror 等长）
    voice_dirty: Vec<bool>,
    /// 本块被修改过的槽位（升序去重后使用；复用避免每块分配）
    dirty_vids: Vec<u32>,
    /// 批量写 staging（连续 vid 合并后紧凑暂存；复用避免每块分配）
    voice_write_scratch: Vec<GpuVoiceState>,
    /// scatter 项（每项 2 u32：staging 元素索引、目标槽位；复用避免每块分配）
    scatter_items: Vec<u32>,
    /// 下一个使用的轮转区域（0..PIPELINE_DEPTH）
    voice_staging_turn: usize,
    /// 本块 scatter 拷贝项数（submit_block 编码 dispatch 后清零）
    scatter_count: u32,
    /// 本块 scatter 项数组在 scatter_items_buf 中的 u32 起始索引
    scatter_items_base: u32,
}

mod alt;
mod blocks;

pub use blocks::PendingReadback;

impl GpuAudioRenderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        let shader_source = include_str!("../shaders/voice_render.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voice_render"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // 10-binding layout:
        // 0: params (uniform)
        // 1: voice_states (storage read_write，滤波器状态跨 block 写回)
        // 2: channel_mix (storage read_write，32 通道 × frames × 2)
        // 3-7: 5 sample chunks (storage read)
        // 8: chunk_offsets (uniform, separate)
        // 9: partial（pass1 每 voice 输出，read_write）
        let mut entries = Vec::with_capacity(10);
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        for i in 0..MAX_CHUNKS {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: (3 + i) as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        // chunk_offsets uniform (binding 8)
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 8,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        // partial buffer (binding 9)
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 9,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        // 块内段结构（binding 10-14，全部 storage read）
        for binding in 10..15u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        // 活跃 voice 列表 + 每通道区间（binding 17，storage read）
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 17,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        // scatter 拷贝项（binding 18）与状态上传 staging（binding 19），storage read
        for binding in [18u32, 19u32] {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("audio_render_bgl"),
            entries: &entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("audio_render_pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("audio_render_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mix_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("audio_mix_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("mix_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let scatter_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("audio_scatter_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("scatter_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Dummy buffer for unused sample chunks（shader 用 vec2 视图读采样，
        // 最小绑定 8 字节：1 个 vec2<f32>）
        let dummy_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dummy"),
            contents: bytemuck::bytes_of(&[0.0f32; 2]),
            usage: wgpu::BufferUsages::STORAGE,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            mix_pipeline,
            scatter_pipeline,
            pipeline_layout,
            bind_group_layout,
            dummy_buf,
            buffers: None,
            sample_data: Arc::new(Vec::new()),
            sample_buffers: None,
            #[cfg(test)]
            sample_upload_count: 0,
            frame_count: 0,
            voice_capacity: super::buffers::MAX_VOICE_SLOTS,
            mix_scratch: Vec::new(),
            release_by_frame_scratch: Vec::new(),
            voice_state_mirror: Vec::new(),
            voice_dirty: Vec::new(),
            dirty_vids: Vec::new(),
            voice_write_scratch: Vec::new(),
            scatter_items: Vec::new(),
            voice_staging_turn: 0,
            scatter_count: 0,
            scatter_items_base: 0,
        })
    }

    /// Create a renderer with its own wgpu device/queue (for standalone use).
    pub fn new_default() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|_| "No GPU adapter found")?;
        // 采样库可达 GB 级（大型 GM 音色库）：storage binding 与单 buffer 上限
        // 取 adapter 支持的最大值，而不是固定 512MB——固定上限配合旧的分片
        // 逻辑会把超出部分静默丢弃（表现为整库无声）。
        let adapter_limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gpu_audio"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
                max_buffer_size: adapter_limits.max_buffer_size,
                // GPU 合成器需要 16 个 storage buffer（采样块 + 段结构 + 指令）
                max_storage_buffers_per_shader_stage:
                    adapter_limits.max_storage_buffers_per_shader_stage,
                ..wgpu::Limits::default()
            },
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("Failed to create device: {}", e))?;
        Self::new(Arc::new(device), Arc::new(queue))
    }

    /// 上传音色库采样数据（接管所有权；GPU 上传发生在下一次 `render_block` 的
    /// `ensure_buffers`，为避免额外拷贝这里不做预切分）。
    pub fn upload_samples(&mut self, sample_data: Arc<Vec<f32>>) {
        self.sample_data = sample_data;
        self.sample_buffers = None;
        self.buffers = None;
    }

    /// 设备可容纳的 voice 槽位上限：partial 缓冲是每槽位 `段长 × 4` 字节的
    /// 单 binding，必须同时满足设备单 buffer 大小与 storage binding 视图
    /// 两个上限；voice 状态/staging 等其他缓冲按同容量分配且每槽位字节数
    /// 远小（~百字节级），不构成瓶颈。设备 limits 已在 request_device 时取
    /// adapter 最大值，无需额外常量。
    pub fn max_voice_capacity(&self) -> u32 {
        let per_slot = RENDER_SEGMENT_FRAMES as u64 * std::mem::size_of::<u32>() as u64;
        let limits = self.device.limits();
        let base = limits
            .max_buffer_size
            .min(limits.max_storage_buffer_binding_size);
        (base / per_slot).min(u32::MAX as u64) as u32
    }

    /// 设置 voice 槽位容量（导出按需扩容 / 结束后恢复）。变化时置空缓冲，
    /// 下次渲染重建（重建会清 GPU 侧 voice 状态，调用方须在无活跃 voice 时用）。
    pub fn set_voice_capacity(&mut self, capacity: u32) {
        if self.voice_capacity != capacity {
            self.voice_capacity = capacity;
            self.buffers = None;
        }
    }

    /// 预热 GPU 缓冲与渲染管线（音色库加载完成后调用）：
    /// 1. 按最大 voice 容量与段长分配全部缓冲（播放中不再因 voice 增长重建）；
    /// 2. 跑一次**哑渲染**（1 个零状态 voice + 整块帧数），触发 shader/管线/
    ///    派发路径的 GPU 首次执行——否则这份开销会落在播放后的首块上，
    ///    表现为"playhead 走到第一个音符前卡一下"。
    ///
    /// 哑 voice 全零：`sample_length == 0` 立即进入 Finished，输出静音。
    pub fn prewarm(&mut self, frames: u32, sample_rate: u32) {
        let frames = frames.max(1);
        self.ensure_buffers(&BufferSpec {
            voice_count: self.voice_capacity,
            frame_count: frames,
            partial_frames: RENDER_SEGMENT_FRAMES,
            segs_len: 0,
            ch_updates_len: 0,
            releases_len: 0,
            env_cmds_len: 0,
        });
        if self.buffers.is_none() {
            return;
        }
        let mut mix = vec![0.0f32; CHANNEL_COUNT * frames as usize * 2];
        let mut stage = vec![0u32; 1];
        // 哑渲染只跑一个段长：目的是触发 shader/管线/派发路径的首次执行，
        // 不是跑满整块（少占 GPU，减轻与 UI 渲染的竞争）。
        let dummy_active: [u32; 1] = [0];
        let mut dummy_ranges = vec![0u32; CHANNEL_COUNT * 2];
        dummy_ranges[0] = 0;
        dummy_ranges[1] = 1;
        let seg = RenderSegment {
            frame_start: 0,
            frame_length: RENDER_SEGMENT_FRAMES.min(frames),
            segs: &[],
            ch_updates: &[],
            releases: &[],
            env_cmds: &[],
            active_count: 1,
            active_data: &dummy_active,
            active_ranges: &dummy_ranges,
        };
        let t = std::time::Instant::now();
        let _ = self.render_block(1, None, &mut mix, &mut stage, &[seg], sample_rate);
        eprintln!("[gpu] GPU 管线预热（哑渲染）={:?}", t.elapsed());
    }

    /// 写入单个 voice 的完整状态到 GPU 槽位（新 voice / chase 恢复用）。
    /// 状态常驻 GPU 后，CPU 只在创建或修改 voice 时写，不再整块重传。
    /// 调用点可能在 buffer 尚未创建时（首块）：入队，`render_block` 在
    /// ensure_buffers 之后统一 flush。
    pub fn write_voice_state(&mut self, vid: u32, state: &GpuVoiceState) {
        // 写镜像 + 脏标记：buffer 可能还没创建（首块），flush 统一发生在
        // ensure_buffers 之后；同一 vid 多次写入只保留最后一份。
        let vid = vid as usize;
        if vid >= self.voice_state_mirror.len() {
            self.voice_state_mirror
                .resize(vid + 1, GpuVoiceState::default());
            self.voice_dirty.resize(vid + 1, false);
        }
        self.voice_state_mirror[vid] = *state;
        if !self.voice_dirty[vid] {
            self.voice_dirty[vid] = true;
            self.dirty_vids.push(vid as u32);
        }
    }

    /// 全量上传 voice 状态（测试路径；生产用 `write_voice_state` 增量写）。
    pub fn upload_voice_states(&mut self, states: &[GpuVoiceState]) {
        for (i, st) in states.iter().enumerate() {
            self.write_voice_state(i as u32, st);
        }
    }

    /// 把待写 voice 槽位 flush 到 GPU（render_block 在 ensure_buffers 之后调用）。
    ///
    /// 状态按 vid 排序合并**连续区间**后紧凑写入 staging 的当前轮转区域；
    /// 真正落位由 `scatter_main` 一次 dispatch 完成（替代原先每区间一条
    /// `copy_buffer_to_buffer`：高潮段每块数千条命令的编码是 CPU 侧大头）。
    /// 轮转区域保证复用前该块已 harvest（GPU 已消费完），无覆盖竞态。
    fn flush_pending_voice_writes(&mut self) {
        if self.dirty_vids.is_empty() {
            return;
        }
        let Some(buf) = self.buffers.as_ref() else {
            return;
        };
        let buf_slots = buf.voice_slots;
        let size = std::mem::size_of::<GpuVoiceState>();
        let staging_slots = self.voice_capacity as usize;
        let t_flush = std::time::Instant::now();
        let n_slots = self.dirty_vids.len() as u64;
        // 脏 vid 天然去重（每槽位只入列一次），升序后合并连续区间
        self.dirty_vids.sort_unstable();
        self.voice_staging_turn =
            (self.voice_staging_turn + 1) % crate::synth::buffers::PIPELINE_DEPTH;
        let base = self.voice_staging_turn * staging_slots;
        let items_base = self.voice_staging_turn * staging_slots * 2;
        self.voice_write_scratch.clear();
        self.scatter_items.clear();
        let mut i = 0usize;
        while i < self.dirty_vids.len() {
            let start = self.dirty_vids[i];
            if start >= buf_slots {
                // 超出槽位（防御）：清除剩余脏标记，避免下次写入被漏掉
                for &vid in &self.dirty_vids[i..] {
                    self.voice_dirty[vid as usize] = false;
                }
                break;
            }
            let mut j = i + 1;
            while j < self.dirty_vids.len() && self.dirty_vids[j] == self.dirty_vids[j - 1] + 1 {
                j += 1;
            }
            let src_elem = self.voice_write_scratch.len() as u32;
            for &vid in &self.dirty_vids[i..j] {
                let vid_usize = vid as usize;
                self.voice_write_scratch
                    .push(self.voice_state_mirror[vid_usize]);
                self.voice_dirty[vid_usize] = false;
            }
            let count = (j - i) as u32;
            // 展开为逐槽位 scatter 项 (staging 元素索引, 目标槽位)
            for k in 0..count {
                self.scatter_items.push(base as u32 + src_elem + k);
                self.scatter_items.push(start + k);
            }
            crate::gpu_synth::FLUSH_WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            i = j;
        }
        self.dirty_vids.clear();
        if !self.voice_write_scratch.is_empty() {
            self.queue.write_buffer(
                &buf.voice_staging_buf,
                (base * size) as u64,
                bytemuck::cast_slice(&self.voice_write_scratch),
            );
        }
        if !self.scatter_items.is_empty() {
            self.queue.write_buffer(
                &buf.scatter_items_buf,
                (items_base * std::mem::size_of::<u32>()) as u64,
                bytemuck::cast_slice(&self.scatter_items),
            );
        }
        self.scatter_count = (self.scatter_items.len() / 2) as u32;
        self.scatter_items_base = items_base as u32;
        let sl = &crate::gpu_synth::FLUSH_SLOTS;
        sl.fetch_add(n_slots, std::sync::atomic::Ordering::Relaxed);
        let us = &crate::gpu_synth::FLUSH_US;
        us.fetch_add(
            t_flush.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}
