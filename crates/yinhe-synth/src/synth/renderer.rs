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
}

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

        // Dummy 1-element buffer for unused sample chunks
        let dummy_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dummy"),
            contents: bytemuck::bytes_of(&0.0f32),
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
        let seg = RenderSegment {
            frame_start: 0,
            frame_length: RENDER_SEGMENT_FRAMES.min(frames),
            segs: &[],
            ch_updates: &[],
            releases: &[],
            env_cmds: &[],
        };
        let t = std::time::Instant::now();
        let _ = self.render_block(1, None, &mut mix, &mut stage, &[seg], sample_rate);
        eprintln!("[gpu] GPU 管线预热（哑渲染）={:?}", t.elapsed());
    }

    /// 渲染一块音频的 per-channel 混音（32 通道 × frames × 2 f32，立体声交错）。
    ///
    /// **分段渲染**：外层块在内部按 [`RenderSegment`] 切段，每段独立跑 pass1
    /// （voice 状态推进 + partial）与 pass2（归约到该段的 channel_mix 区间）；
    /// partial 只按段长上界 [`RENDER_SEGMENT_FRAMES`] 分配（满容量 8192 voice
    /// 也只有 ~32MB，加载阶段预热后不再重建）。每段的 uniform/指令写入后立即
    /// submit（缓冲只有一份，避免后段覆盖前段），最后一段统一读回，
    /// CPU↔GPU 往返仍为一次。
    ///
    /// **voice 状态常驻 GPU**：新 voice 的状态由调用方在 dispatch 前通过
    /// [`write_voice_state`] 写入槽位；本方法只读回紧凑的 `voice_stage_out`。
    ///
    /// 返回实际 voice 数量（0 表示静音）。
    #[allow(clippy::too_many_arguments)] // 渲染上下文透传，见 AGENTS 约定
    pub fn render_block(
        &mut self,
        voice_count: u32,
        readback_states: Option<&mut [GpuVoiceState]>,
        channel_mix: &mut [f32],
        voice_stage_out: &mut [u32],
        segments: &[RenderSegment<'_>],
        sample_rate: u32,
    ) -> u32 {
        let frame_count = (channel_mix.len() / 2 / CHANNEL_COUNT) as u32;
        let voice_count = voice_count.min(super::buffers::MAX_VOICE_SLOTS);
        if voice_count == 0 || frame_count == 0 || segments.is_empty() {
            channel_mix.fill(0.0);
            return 0;
        }

        // 容量按所有段的最大需求（避免逐段扩容重建）
        let max_segs = segments.iter().map(|s| s.segs.len()).max().unwrap_or(0);
        let max_ch = segments
            .iter()
            .map(|s| s.ch_updates.len())
            .max()
            .unwrap_or(0);
        let max_rel = segments.iter().map(|s| s.releases.len()).max().unwrap_or(0);
        let max_env = segments.iter().map(|s| s.env_cmds.len()).max().unwrap_or(0);
        // partial 容量 = 所有段的最长段长（同一块内固定 stride，段间不串位）
        let partial_frames = segments.iter().map(|s| s.frame_length).max().unwrap_or(0);
        self.ensure_buffers(&BufferSpec {
            voice_count,
            frame_count,
            partial_frames,
            segs_len: max_segs,
            ch_updates_len: max_ch,
            releases_len: max_rel,
            env_cmds_len: max_env,
        });
        // 未 upload 采样时（音色库为空）直接输出静音，绝不 panic
        if self.buffers.is_none() {
            channel_mix.fill(0.0);
            return 0;
        }
        // 首块/重建后：把待写槽位 flush（buffer 就绪前调用的 write 不丢）。
        self.flush_pending_voice_writes();

        let voice_wg_count = voice_count.div_ceil(WORKGROUP_SIZE);
        let mix_size = std::mem::size_of_val(channel_mix) as u64;
        let stage_size = (voice_count as usize * std::mem::size_of::<u32>()) as u64;
        let want_full = readback_states.is_some();
        let last = segments.len() - 1;

        for (seg_i, seg) in segments.iter().enumerate() {
            if seg.frame_length == 0 {
                continue;
            }
            let Some(buf) = self.buffers.as_ref() else {
                break;
            };
            let idx = buf.staging_idx;
            // release 按帧前缀和（段内帧；scratch 复用，clear 后全量填零）
            self.release_by_frame_scratch.clear();
            self.release_by_frame_scratch
                .resize(seg.frame_length as usize + 2, 0);
            for r in seg.releases {
                self.release_by_frame_scratch[r.frame as usize + 1] += 1;
            }
            for i in 1..self.release_by_frame_scratch.len() {
                self.release_by_frame_scratch[i] += self.release_by_frame_scratch[i - 1];
            }
            let params = RenderParams {
                frame_count: seg.frame_length,
                voice_count,
                sample_rate,
                sample_chunk_count: buf.chunk_count,
                voice_wg_count,
                seg_count: seg.segs.len() as u32,
                release_count: seg.releases.len() as u32,
                env_update_count: seg.env_cmds.len() as u32,
                partial_stride: partial_frames,
                channel_mix_frames: frame_count,
                mix_offset: seg.frame_start,
            };
            self.queue
                .write_buffer(&buf.segs_buf, 0, bytemuck::cast_slice(seg.segs));
            self.queue
                .write_buffer(&buf.ch_updates_buf, 0, bytemuck::cast_slice(seg.ch_updates));
            self.queue.write_buffer(
                &buf.release_by_frame_buf,
                0,
                bytemuck::cast_slice(&self.release_by_frame_scratch),
            );
            self.queue
                .write_buffer(&buf.release_cmds_buf, 0, bytemuck::cast_slice(seg.releases));
            self.queue
                .write_buffer(&buf.env_cmds_buf, 0, bytemuck::cast_slice(seg.env_cmds));
            self.queue
                .write_buffer(&buf.params_buf, 0, bytemuck::bytes_of(&params));

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("audio_render"),
                });
            // pass1：每线程一个 voice，串行推进本段所有帧
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("voice_pass"),
                    ..Default::default()
                });
                cpass.set_pipeline(&self.pipeline);
                cpass.set_bind_group(0, &buf.bind_groups[idx], &[]);
                cpass.dispatch_workgroups(voice_wg_count, 1, 1);
            }
            // pass2：每帧一个 workgroup，把 partial 归约到 channel_mix 本段区间
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("mix_pass"),
                    ..Default::default()
                });
                cpass.set_pipeline(&self.mix_pipeline);
                cpass.set_bind_group(0, &buf.bind_groups[idx], &[]);
                cpass.dispatch_workgroups(seg.frame_length, 1, 1);
            }
            // 最后一段：读回 channel_mix（整块）+ 紧凑 stage [+ 全字段]，
            // 由紧随其后的 submit 一并执行（此后不再有段需要该缓冲）。
            if seg_i == last {
                encoder.copy_buffer_to_buffer(
                    &buf.channel_mix_buf,
                    0,
                    &buf.staging[idx],
                    0,
                    mix_size,
                );
                encoder.copy_buffer_to_buffer(
                    &buf.voice_stage_buf,
                    0,
                    &buf.staging[idx],
                    buf.staging_stage_offset,
                    stage_size,
                );
                if want_full {
                    let full_size =
                        (voice_count as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
                    encoder.copy_buffer_to_buffer(
                        &buf.voice_state_buf,
                        0,
                        &buf.staging[idx],
                        buf.staging_full_offset,
                        full_size,
                    );
                }
            }
            self.queue.submit(std::iter::once(encoder.finish()));
        }

        // 一次 map/poll（channel_mix + voice_stage [+ full states]）
        let Some(buf) = self.buffers.as_ref() else {
            channel_mix.fill(0.0);
            return 0;
        };
        let idx = buf.staging_idx;
        let buffer_slice = buf.staging[idx].slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        // map 失败（如设备丢失）：输出静音，不 unwrap 保命
        if !matches!(receiver.recv(), Ok(Ok(()))) {
            channel_mix.fill(0.0);
            return 0;
        }

        let data = match buffer_slice.get_mapped_range() {
            Ok(d) => d,
            Err(_) => {
                channel_mix.fill(0.0);
                return 0;
            }
        };
        // channel_mix
        let mix_bytes = mix_size as usize;
        let gpu_mix: &[f32] = bytemuck::cast_slice(&data[..mix_bytes]);
        channel_mix[..gpu_mix.len()].copy_from_slice(gpu_mix);
        // 紧凑 stage
        let stage_start = buf.staging_stage_offset as usize;
        let stage: &[u32] =
            bytemuck::cast_slice(&data[stage_start..stage_start + stage_size as usize]);
        let n_stage = stage.len().min(voice_stage_out.len());
        voice_stage_out[..n_stage].copy_from_slice(&stage[..n_stage]);
        // 全字段读回
        if let Some(out) = readback_states {
            let full_start = buf.staging_full_offset as usize;
            let full_bytes = voice_count as usize * std::mem::size_of::<GpuVoiceState>();
            let states: &[GpuVoiceState] =
                bytemuck::cast_slice(&data[full_start..full_start + full_bytes]);
            let n = states.len().min(out.len());
            out[..n].copy_from_slice(&states[..n]);
        }
        drop(data);
        buf.staging[idx].unmap();
        if let Some(b) = self.buffers.as_mut() {
            b.staging_idx = 1 - idx;
        }

        voice_count
    }

    /// 单段渲染（整块作一段；测试/CPU 参考路径）。块长可超过
    /// [`RENDER_SEGMENT_FRAMES`]（partial 按实际块长分配）。
    #[allow(clippy::too_many_arguments)] // 透传上下文，见 AGENTS 约定
    pub fn render_block_single(
        &mut self,
        voice_count: u32,
        readback_states: Option<&mut [GpuVoiceState]>,
        channel_mix: &mut [f32],
        voice_stage_out: &mut [u32],
        segs: &[SegInfo],
        ch_updates: &[ChState],
        releases: &[ReleaseCmd],
        env_cmds: &[EnvUpdateCmd],
        sample_rate: u32,
    ) -> u32 {
        let frame_count = (channel_mix.len() / 2 / CHANNEL_COUNT) as u32;
        let seg = RenderSegment {
            frame_start: 0,
            frame_length: frame_count,
            segs,
            ch_updates,
            releases,
            env_cmds,
        };
        self.render_block(
            voice_count,
            readback_states,
            channel_mix,
            voice_stage_out,
            &[seg],
            sample_rate,
        )
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
        let Some(buf) = &self.buffers else {
            return;
        };
        let size = std::mem::size_of::<GpuVoiceState>() as u64;
        for (vid, st) in self.pending_voice_writes.drain(..) {
            if vid >= buf.voice_slots {
                continue;
            }
            self.queue.write_buffer(
                &buf.voice_state_buf,
                vid as u64 * size,
                bytemuck::bytes_of(&st),
            );
        }
    }

    /// Render a block of audio using the GPU（测试/便利路径）。
    /// 渲染一块音频（frames × 2 立体声交错）：per-channel 混音求和，无通道滤波。
    /// `voices` 会被更新：读回 GPU 端推进的**全字段**状态（时间/包络/滤波）。
    /// 调用方**不应再**用 CPU 推进 voice 状态（GPU 已推进）。
    /// 返回实际 voice 数量（0 表示静音）。
    ///
    /// 注意：生产路径（GpuSynth::render_to_mixer）用 `render_block` 的紧凑读回；
    /// 本方法保留全字段读回供 CPU/GPU 一致性测试使用。
    pub fn render_into(
        &mut self,
        voices: &mut [GpuVoiceState],
        output: &mut [f32],
        sample_rate: u32,
    ) -> u32 {
        let frames = output.len() / 2;
        let mut scratch = std::mem::take(&mut self.mix_scratch);
        scratch.resize(CHANNEL_COUNT * frames * 2, 0.0);
        // 测试路径：先全量上传（状态自包含），再渲染 + 全字段读回。
        self.upload_voice_states(voices);
        let mut stage = vec![0u32; voices.len()];
        // 测试路径单段：整块一次 pass1+pass2
        let seg = RenderSegment {
            frame_start: 0,
            frame_length: frames as u32,
            segs: &[],
            ch_updates: &[],
            releases: &[],
            env_cmds: &[],
        };
        let n = self.render_block(
            voices.len() as u32,
            Some(voices),
            &mut scratch,
            &mut stage,
            &[seg],
            sample_rate,
        );
        self.mix_scratch = scratch;
        output.fill(0.0);
        for ch in 0..CHANNEL_COUNT {
            let base = ch * frames * 2;
            for (i, o) in output.iter_mut().enumerate() {
                *o += self.mix_scratch[base + i];
            }
        }
        n
    }

    /// 渲染一块音频（返回新分配的 Vec，辅助测试用）。
    pub fn render_block_alloc(
        &mut self,
        voices: &mut [GpuVoiceState],
        frame_count: u32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let mut output = vec![0.0; frame_count as usize * 2];
        self.render_into(voices, &mut output, sample_rate);
        output
    }
}
