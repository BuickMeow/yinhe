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
    /// render_into 的 per-channel 混音临时缓冲（复用，避免每块分配）
    pub(crate) mix_scratch: Vec<f32>,
    /// 每段 release 指令的按帧前缀和（复用，避免每段分配）
    pub(crate) release_by_frame_scratch: Vec<u32>,
    /// 待写入的 voice 槽位更新（buffer 未就绪时也不丢；render_block 在
    /// ensure_buffers 之后统一 flush）。
    pending_voice_writes: Vec<(u32, GpuVoiceState)>,
    /// 批量写 staging（连续 vid 合并后紧凑暂存；复用避免每块分配）
    voice_write_scratch: Vec<GpuVoiceState>,
    /// voice 状态暂存缓冲：flush 紧凑写一次，GPU 端按连续区间拷贝到槽位。
    /// **内含 PIPELINE_DEPTH 个轮转区域**——`render_to_mixer` 保证最多
    /// PIPELINE_DEPTH 个提交在途（收割等待最老块完成），因此区域复用时
    /// 使用它的块必然已 harvest（GPU 已消费完），无覆盖竞态。
    voice_staging_buf: Option<wgpu::Buffer>,
    /// 每个轮转区域的槽位容量（元素数）
    voice_staging_slots: usize,
    /// 下一个使用的轮转区域（0..PIPELINE_DEPTH）
    voice_staging_turn: usize,
    /// 本块待执行的 GPU 拷贝 (staging 元素起, 目标槽位起, 槽位数)。
    pending_state_copies: Vec<(u32, u32, u32)>,
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
        // 紧凑 voice_stage（binding 15，pass1 写、CPU 读回）
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 15,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });

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
            pipeline_layout,
            bind_group_layout,
            dummy_buf,
            buffers: None,
            sample_data: Arc::new(Vec::new()),
            sample_buffers: None,
            #[cfg(test)]
            sample_upload_count: 0,
            frame_count: 0,
            mix_scratch: Vec::new(),
            release_by_frame_scratch: Vec::new(),
            pending_voice_writes: Vec::new(),
            voice_write_scratch: Vec::new(),
            voice_staging_buf: None,
            voice_staging_slots: 0,
            voice_staging_turn: 0,
            pending_state_copies: Vec::new(),
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gpu_audio"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: 512 * 1024 * 1024,
                max_buffer_size: 512 * 1024 * 1024,
                // GPU 合成器需要 13 个 storage buffer（采样块 + 段结构 + 指令）
                max_storage_buffers_per_shader_stage: 16,
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
            voice_count: super::buffers::MAX_VOICE_SLOTS,
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
        // 一律入队：buffer 可能还没创建，或将在本块 render_block 里因扩容重建，
        // flush 统一发生在 ensure_buffers 之后，写入不会丢。
        self.pending_voice_writes.push((vid, *state));
    }

    /// 全量上传 voice 状态（测试路径；生产用 `write_voice_state` 增量写）。
    pub fn upload_voice_states(&mut self, states: &[GpuVoiceState]) {
        for (i, st) in states.iter().enumerate() {
            self.write_voice_state(i as u32, st);
        }
    }

    /// 把待写 voice 槽位 flush 到 GPU（render_block 在 ensure_buffers 之后调用）。
    fn flush_pending_voice_writes(&mut self) {
        if self.pending_voice_writes.is_empty() {
            return;
        }
        let Some(buf_slots) = self.buffers.as_ref().map(|b| b.voice_slots) else {
            return;
        };
        let size = std::mem::size_of::<GpuVoiceState>();
        let t_flush = std::time::Instant::now();
        let n_slots = self.pending_voice_writes.len() as u64;
        // 按 vid 排序后合并**连续区间**，状态紧凑写入 staging 的当前轮转区域
        // （一次 write_buffer），真正落位由 GPU 端 `copy_buffer_to_buffer`
        // 完成（copy 记录 ~1µs vs write_buffer 每次 ~2.5µs~10µs 固定开销；
        // 高潮段每块上千区间是 CPU 侧最大头）。
        self.pending_voice_writes
            .sort_unstable_by_key(|(vid, _)| *vid);
        let cap = self.pending_voice_writes.len().next_power_of_two().max(64);
        if self.voice_staging_slots < cap {
            // 区域重建：旧 buffer 可能仍被在途块引用——wgpu 保证其存活，
            // 新 buffer 从 0 轮转，两代互不干扰。
            self.voice_staging_buf = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("voice_staging"),
                size: (cap * crate::synth::buffers::PIPELINE_DEPTH * size) as u64,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.voice_staging_slots = cap;
            self.voice_staging_turn = 0;
        }
        self.voice_staging_turn =
            (self.voice_staging_turn + 1) % crate::synth::buffers::PIPELINE_DEPTH;
        let base = self.voice_staging_turn * self.voice_staging_slots;
        self.voice_write_scratch.clear();
        self.pending_state_copies.clear();
        let mut i = 0usize;
        while i < self.pending_voice_writes.len() {
            let start = self.pending_voice_writes[i].0;
            if start >= buf_slots {
                break;
            }
            let mut j = i + 1;
            while j < self.pending_voice_writes.len()
                && self.pending_voice_writes[j].0 == self.pending_voice_writes[i].0 + (j - i) as u32
            {
                j += 1;
            }
            let src_elem = self.voice_write_scratch.len() as u32;
            self.voice_write_scratch
                .extend(self.pending_voice_writes[i..j].iter().map(|(_, st)| *st));
            let count = ((j - i) as u32).min(buf_slots - start);
            self.pending_state_copies
                .push((base as u32 + src_elem, start, count));
            crate::gpu_synth::FLUSH_WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            i = j;
        }
        if let Some(staging) = &self.voice_staging_buf {
            self.queue.write_buffer(
                staging,
                (base * size) as u64,
                bytemuck::cast_slice(&self.voice_write_scratch),
            );
        }
        let sl = &crate::gpu_synth::FLUSH_SLOTS;
        sl.fetch_add(n_slots, std::sync::atomic::Ordering::Relaxed);
        let us = &crate::gpu_synth::FLUSH_US;
        us.fetch_add(
            t_flush.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.pending_voice_writes.clear();
    }
}
