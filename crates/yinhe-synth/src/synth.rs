//! GPU-accelerated audio renderer for offline export.
//!
//! Uses wgpu compute shaders with multi-chunk sample buffers to handle
//! soundfont data larger than the GPU's max buffer binding size.

use std::sync::Arc;
use wgpu::util::DeviceExt;

const MAX_CHUNKS: usize = 5;
const CHUNK_SIZE: usize = 30_000_000; // 30M f32 = 120MB per chunk
const WORKGROUP_SIZE: u32 = 256;

/// Per-voice state that is uploaded to the GPU each block.
/// 布局必须与 WGSL 的 VoiceState 结构体严格对应。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVoiceState {
    // Sample playback
    pub sample_offset: u32,
    pub sample_length: u32,
    pub speed: f32,
    pub base_gain: f32,
    pub time: f32,
    pub start_offset: u32, // 块内起始帧偏移
    // Envelope state at start of block
    pub envelope: f32,       // 当前 envelope 值
    pub env_stage: u32,      // 0=Delay,1=Attack,2=Hold,3=Decay,4=Sustain,5=Release,6=Finished
    pub stage_progress: f32, // 当前阶段已用帧数
    // Envelope parameters
    pub env_level: f32,     // peak = gain
    pub sustain_level: f32, // 0..1
    pub env_start: f32,     // attack 起点 / release 起始值
    // Stage durations (frames)
    pub delay_frames: f32,
    pub attack_frames: f32,
    pub hold_frames: f32,
    pub decay_frames: f32,
    pub release_frames: f32,
    // 声像：音色库基础声像（通道 pan 渐变在 shader 内逐帧计算，见 ch_pan）
    pub base_pan_l: f32,
    pub base_pan_r: f32,
    // 通道渐变状态（xsynth ValueLerp：CC7/10/11 10ms 线性渐变，shader 逐帧推进）
    pub ch_vol: f32,
    pub ch_vol_step: f32,
    pub ch_vol_frames: u32,
    pub ch_expr: f32,
    pub ch_expr_step: f32,
    pub ch_expr_frames: u32,
    pub ch_pan: f32,
    pub ch_pan_step: f32,
    pub ch_pan_frames: u32,
    // Loop
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_mode: u32, // 0=NoLoop, 1=LoopContinuous, 2=LoopSustain, 3=OneShot
    // 采样布局与插值（与 xsynth 默认对齐：interp=0 Nearest）
    pub is_stereo: u32, // 0=单声道样本, 1=交错立体声
    pub interp: u32,    // 0=Nearest, 1=Linear
    // per-voice biquad（cutoff > 0 启用）
    pub cutoff: f32,      // Hz
    pub resonance: f32,   // 线性 Q（保留字段，系数已由 CPU 预计算）
    pub filter_type: u32, // 0=LowPass, 1=HighPass, 2=BandPass, 3=SinglePoleLowPass
    pub flt_b0: f32,
    pub flt_b1: f32,
    pub flt_b2: f32,
    pub flt_a1: f32,
    pub flt_a2: f32,
    // DirectForm1 状态（左声道；跨 block 由 GPU 写回）
    pub flt_x1: f32,
    pub flt_x2: f32,
    pub flt_y1: f32,
    pub flt_y2: f32,
    // DirectForm1 状态（右声道，仅立体声样本使用）
    pub flt_x1r: f32,
    pub flt_x2r: f32,
    pub flt_y1r: f32,
    pub flt_y2r: f32,
}

/// Uniform buffer for render parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RenderParams {
    pub frame_count: u32,
    pub voice_count: u32,
    pub sample_rate: u32,
    pub sample_chunk_count: u32,
    pub voice_wg_count: u32, // pass1 workgroup 数 = ceil(voice_count / 256)
}

/// RBJ cookbook biquad 系数（与 xsynth 的 biquad crate 完全一致）。
/// 返回 (b0, b1, b2, a1, a2)，用于 DirectForm1：
/// y = b0*x + b1*x1 + b2*x2 - a1*y1 - a2*y2
pub fn biquad_coeffs(
    filter_type: u32,
    cutoff: f32,
    resonance: f32,
    sample_rate: f32,
) -> (f32, f32, f32, f32, f32) {
    let omega = 2.0 * std::f32::consts::PI * cutoff / sample_rate;
    let q = if resonance > 0.0 {
        resonance
    } else {
        std::f32::consts::FRAC_1_SQRT_2
    };
    match filter_type {
        3 => {
            // SinglePoleLowPass
            let omega_t = (omega / 2.0).tan();
            let a0 = 1.0 + omega_t;
            let b0 = omega_t / a0;
            ((b0), (b0), 0.0, (omega_t - 1.0) / a0, 0.0)
        }
        1 => {
            // HighPass
            let omega_s = omega.sin();
            let omega_c = omega.cos();
            let alpha = omega_s / (2.0 * q);
            let b0 = (1.0 + omega_c) * 0.5;
            let a0 = 1.0 + alpha;
            (
                b0 / a0,
                -b0 * 2.0 / a0,
                b0 / a0,
                -2.0 * omega_c / a0,
                (1.0 - alpha) / a0,
            )
        }
        2 => {
            // BandPass
            let omega_s = omega.sin();
            let omega_c = omega.cos();
            let alpha = omega_s / (2.0 * q);
            let a0 = 1.0 + alpha;
            let div = 1.0 / a0;
            (
                omega_s / 2.0 * div,
                0.0,
                -omega_s / 2.0 * div,
                -2.0 * omega_c * div,
                (1.0 - alpha) * div,
            )
        }
        _ => {
            // LowPass
            let omega_s = omega.sin();
            let omega_c = omega.cos();
            let alpha = omega_s / (2.0 * q);
            let b0 = (1.0 - omega_c) * 0.5;
            let a0 = 1.0 + alpha;
            (
                b0 / a0,
                2.0 * b0 / a0,
                b0 / a0,
                -2.0 * omega_c / a0,
                (1.0 - alpha) / a0,
            )
        }
    }
}

/// CPU 端推进 voice 状态：用解析公式直接计算，不逐帧迭代。
/// 7 阶段: 0=Delay, 1=Attack(线性), 2=Hold, 3=Decay(指数), 4=Sustain, 5=Release(指数), 6=Finished
pub fn advance_voices(voices: &mut [GpuVoiceState], frame_count: u32) {
    for voice in voices.iter_mut() {
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.start_offset = 0;
        if voice.env_stage >= 6 || active_frames == 0 {
            continue;
        }
        voice.time += voice.speed * active_frames as f32;

        // 循环回绕
        let has_loop = voice.loop_mode > 0 && voice.loop_end > voice.loop_start;
        if has_loop && voice.time >= voice.loop_end as f32 {
            let loop_len = (voice.loop_end - voice.loop_start) as f32;
            if loop_len > 0.0 {
                voice.time =
                    voice.loop_start as f32 + ((voice.time - voice.loop_start as f32) % loop_len);
            }
        }

        let peak = voice.env_level;
        let sus = voice.sustain_level * peak;
        let mut remaining = active_frames as f32;

        while remaining > 0.0 && voice.env_stage < 6 {
            match voice.env_stage {
                0 => {
                    // Delay
                    let dur = voice.delay_frames - voice.stage_progress;
                    if remaining < dur {
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        remaining -= dur;
                        voice.env_stage = 1;
                        voice.stage_progress = 0.0;
                    }
                }
                1 => {
                    // Attack: 线性
                    let dur = voice.attack_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.attack_frames;
                        voice.envelope = voice.env_start + (peak - voice.env_start) * t;
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = peak;
                        remaining -= dur;
                        voice.env_stage = 2;
                        voice.stage_progress = 0.0;
                    }
                }
                2 => {
                    // Hold
                    let dur = voice.hold_frames - voice.stage_progress;
                    if remaining < dur {
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        remaining -= dur;
                        voice.env_stage = 3;
                        voice.stage_progress = 0.0;
                    }
                }
                3 => {
                    // Decay: 指数 (1-t)^8
                    let dur = voice.decay_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.decay_frames;
                        voice.envelope = sus + (peak - sus) * (1.0 - t).powi(8);
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = sus;
                        remaining -= dur;
                        voice.env_stage = 4;
                        voice.stage_progress = 0.0;
                    }
                }
                4 => {
                    // Sustain: envelope 恒为 sus（与 GPU 逐帧推进一致）
                    voice.envelope = sus;
                    remaining = 0.0;
                } // Sustain: 无限
                5 => {
                    // Release: 指数 (1-t)^8
                    let dur = voice.release_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.release_frames;
                        voice.envelope = voice.env_start * (1.0 - t).powi(8);
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = 0.0;
                        remaining -= dur;
                        voice.env_stage = 6;
                        voice.stage_progress = 0.0;
                    }
                }
                _ => break,
            }
        }
    }
}

/// Persistent GPU state — all buffers allocated once, reused every block.
struct GpuBuffers {
    #[allow(dead_code)]
    sample_chunks: Vec<wgpu::Buffer>,
    #[allow(dead_code)]
    chunk_offsets_buf: wgpu::Buffer,
    chunk_count: u32,
    voice_state_buf: wgpu::Buffer,
    max_voices: u32,
    final_output_buf: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    /// pass1 归约中间结果（voice_workgroups × frames × 2 f32）
    #[allow(dead_code)] // 经 bind_groups 使用
    partial_buf: wgpu::Buffer,
    staging: [wgpu::Buffer; 2],
    /// 读回 voice 状态（滤波器 IIR 状态跨 block 持久）
    staging_voice: [wgpu::Buffer; 2],
    staging_idx: usize,
    bind_groups: [wgpu::BindGroup; 2],
}

/// GPU-accelerated audio renderer with persistent buffers.
pub struct GpuAudioRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::ComputePipeline,     // pass1: 每 voice 串行帧
    mix_pipeline: wgpu::ComputePipeline, // pass2: 归约 partial
    #[allow(dead_code)]
    pipeline_layout: wgpu::PipelineLayout,
    bind_group_layout: wgpu::BindGroupLayout,
    dummy_buf: wgpu::Buffer,
    buffers: Option<GpuBuffers>,
    /// Persistent copy of sample data chunks (never consumed, reused for buffer rebuilds).
    sample_chunks: Vec<Vec<f32>>,
    frame_count: u32,
}

impl GpuAudioRenderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        let shader_source = include_str!("shaders/voice_render.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voice_render"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // 10-binding layout:
        // 0: params (uniform)
        // 1: voice_states (storage read_write，滤波器状态跨 block 写回)
        // 2: final_output (storage read_write)
        // 3-7: 5 sample chunks (storage read)
        // 8: chunk_offsets (uniform, separate)
        // 9: partial（pass1 归约中间结果，read_write）
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
            sample_chunks: Vec::new(),
            frame_count: 0,
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
        }))
        .map_err(|_| "No GPU adapter found")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gpu_audio"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: 512 * 1024 * 1024,
                max_buffer_size: 512 * 1024 * 1024,
                ..wgpu::Limits::default()
            },
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("Failed to create device: {}", e))?;
        Self::new(Arc::new(device), Arc::new(queue))
    }

    /// Upload soundfont sample data. Splits into chunks for GPU buffer limits.
    pub fn upload_samples(&mut self, sample_data: &[f32]) {
        self.sample_chunks = sample_data.chunks(CHUNK_SIZE).map(|c| c.to_vec()).collect();
        self.buffers = None;
    }

    fn ensure_buffers(&mut self, voice_count: u32, frame_count: u32) {
        // 幂增长策略：向上取整到 2 的幂次，避免每个 block 都重建缓冲区
        let rounded_voices = voice_count.max(64).next_power_of_two();
        let needs_recreate = if self.sample_chunks.is_empty() {
            return;
        } else {
            match &self.buffers {
                Some(b) => b.max_voices < rounded_voices || self.frame_count < frame_count,
                None => true,
            }
        };
        if !needs_recreate {
            return;
        }

        let device = &self.device;
        let chunk_count = self.sample_chunks.len().min(MAX_CHUNKS) as u32;

        // Create sample chunk buffers
        let sample_chunks: Vec<wgpu::Buffer> = self
            .sample_chunks
            .iter()
            .map(|data| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("sample_chunk"),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                })
            })
            .collect();

        // Create chunk_offsets buffer (uniform, padded to 32 bytes = 8 u32 for 16-byte alignment)
        let mut offsets: Vec<u32> = Vec::with_capacity(8);
        let mut acc = 0u32;
        for chunk in &self.sample_chunks {
            offsets.push(acc);
            acc += chunk.len() as u32;
        }
        offsets.push(acc); // total = sentinel
        // Pad to exactly 8 entries for WGSL struct alignment
        while offsets.len() < 8 {
            offsets.push(0);
        }
        let chunk_offsets_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_offsets"),
            contents: bytemuck::cast_slice(&offsets),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Other persistent buffers（用 rounded_voices 分配，和 max_voices 一致）
        let voice_state_size =
            (rounded_voices as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
        let final_output_size =
            (frame_count.max(1) as usize * 2 * std::mem::size_of::<f32>()) as u64;
        // pass1 workgroup 数（按分配的最大 voice 数向上取整）
        let alloc_wg_count = rounded_voices.div_ceil(WORKGROUP_SIZE);
        let partial_size = (alloc_wg_count as usize
            * frame_count.max(1) as usize
            * 2
            * std::mem::size_of::<f32>()) as u64;
        let params_size = std::mem::size_of::<RenderParams>() as u64;

        let voice_state_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_voice_states"),
            size: voice_state_size,
            // read_write：pass1 块末写回滤波器 IIR 状态
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let partial_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_partial"),
            size: partial_size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let final_output_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_final_output"),
            size: final_output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_params"),
            size: params_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_0"),
            size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_1"),
            size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // voice 状态读回（滤波器 IIR 状态）
        let staging_voice0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_voice_0"),
            size: voice_state_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging_voice1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_voice_1"),
            size: voice_state_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Build bind group entries
        let make_bg = |p: &wgpu::Buffer,
                       v: &wgpu::Buffer,
                       f: &wgpu::Buffer,
                       co: &wgpu::Buffer,
                       sc: &[wgpu::Buffer],
                       db: &wgpu::Buffer,
                       pt: &wgpu::Buffer| {
            let mut bg_entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: p.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: v.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: f.as_entire_binding(),
                },
            ];
            // 必须固定迭代 MAX_CHUNKS 次：sc 可能不足 MAX_CHUNKS，
            // 其余 binding slot 用 dummy buffer 占位（layout 要求全部填充）。
            #[allow(clippy::needless_range_loop)]
            for i in 0..MAX_CHUNKS {
                let resource = if (i as u32) < chunk_count {
                    sc[i].as_entire_binding()
                } else {
                    db.as_entire_binding()
                };
                bg_entries.push(wgpu::BindGroupEntry {
                    binding: (3 + i) as u32,
                    resource,
                });
            }
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 8,
                resource: co.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 9,
                resource: pt.as_entire_binding(),
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("audio_bg"),
                layout: &self.bind_group_layout,
                entries: &bg_entries,
            })
        };

        self.buffers = Some(GpuBuffers {
            bind_groups: [
                make_bg(
                    &params_buf,
                    &voice_state_buf,
                    &final_output_buf,
                    &chunk_offsets_buf,
                    &sample_chunks,
                    &self.dummy_buf,
                    &partial_buf,
                ),
                make_bg(
                    &params_buf,
                    &voice_state_buf,
                    &final_output_buf,
                    &chunk_offsets_buf,
                    &sample_chunks,
                    &self.dummy_buf,
                    &partial_buf,
                ),
            ],
            sample_chunks,
            chunk_offsets_buf,
            chunk_count,
            voice_state_buf,
            max_voices: rounded_voices,
            final_output_buf,
            params_buf,
            partial_buf,
            staging: [staging0, staging1],
            staging_voice: [staging_voice0, staging_voice1],
            staging_idx: 0,
        });
        self.frame_count = frame_count;
    }

    /// Render a block of audio using the GPU.
    /// 渲染一块音频。输出写入 `output`（长度 = frame_count * 2，立体声交错）。
    /// `voices` 会被更新：读回 GPU 端推进的滤波器 IIR 状态（跨 block 持久）。
    /// 返回实际 voice 数量（0 表示静音）。
    pub fn render_into(
        &mut self,
        voices: &mut [GpuVoiceState],
        output: &mut [f32],
        sample_rate: u32,
    ) -> u32 {
        let frame_count = (output.len() / 2) as u32;
        let voice_count = voices.len() as u32;
        if voice_count == 0 || frame_count == 0 {
            output.fill(0.0);
            return 0;
        }

        self.ensure_buffers(voice_count, frame_count);
        // 未 upload 采样时（音色库为空）直接输出静音，绝不 panic
        let buf = match self.buffers.as_mut() {
            Some(b) => b,
            None => {
                output.fill(0.0);
                return 0;
            }
        };

        let voice_wg_count = voice_count.div_ceil(WORKGROUP_SIZE);
        self.queue
            .write_buffer(&buf.voice_state_buf, 0, bytemuck::cast_slice(voices));
        let params = RenderParams {
            frame_count,
            voice_count,
            sample_rate,
            sample_chunk_count: buf.chunk_count,
            voice_wg_count,
        };
        self.queue
            .write_buffer(&buf.params_buf, 0, bytemuck::bytes_of(&params));

        let idx = buf.staging_idx;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("audio_render"),
            });

        // pass1：每 voice 串行渲染 block 内所有帧（含逐帧包络推进与 per-voice 滤波）
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voice_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.pipeline);
            cpass.set_bind_group(0, &buf.bind_groups[idx], &[]);
            cpass.dispatch_workgroups(voice_wg_count, 1, 1);
        }
        // pass2：归约 partial 到最终输出
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mix_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.mix_pipeline);
            cpass.set_bind_group(0, &buf.bind_groups[idx], &[]);
            cpass.dispatch_workgroups(frame_count, 1, 1);
        }

        let final_output_size = (frame_count as usize * 2 * std::mem::size_of::<f32>()) as u64;
        encoder.copy_buffer_to_buffer(
            &buf.final_output_buf,
            0,
            &buf.staging[idx],
            0,
            final_output_size,
        );
        // 读回 voice 状态（滤波器 IIR 状态，供下一 block 上传）
        let voice_state_size = buf.voice_state_buf.size();
        encoder.copy_buffer_to_buffer(
            &buf.voice_state_buf,
            0,
            &buf.staging_voice[idx],
            0,
            voice_state_size,
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // 只 map 本次实际渲染的帧数（staging 可能比本次 block 大）
        let buffer_slice = buf.staging[idx].slice(..final_output_size);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let voice_slice = buf.staging_voice[idx].slice(..);
        let (vsender, vreceiver) = std::sync::mpsc::channel();
        voice_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = vsender.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        // map 失败（如设备丢失）：输出静音，不 unwrap 保命
        if !matches!(receiver.recv(), Ok(Ok(()))) || !matches!(vreceiver.recv(), Ok(Ok(()))) {
            output.fill(0.0);
            return 0;
        }

        let data = buffer_slice.get_mapped_range();
        let gpu_output: &[f32] = bytemuck::cast_slice(&data);
        output[..gpu_output.len()].copy_from_slice(gpu_output);
        drop(data);
        buf.staging[idx].unmap();

        // 读回滤波器状态（其余字段由 CPU advance_voices 推进）
        let vdata = voice_slice.get_mapped_range();
        let gpu_voices: &[GpuVoiceState] = bytemuck::cast_slice(&vdata);
        for (i, v) in voices.iter_mut().enumerate() {
            v.flt_x1 = gpu_voices[i].flt_x1;
            v.flt_x2 = gpu_voices[i].flt_x2;
            v.flt_y1 = gpu_voices[i].flt_y1;
            v.flt_y2 = gpu_voices[i].flt_y2;
            v.flt_x1r = gpu_voices[i].flt_x1r;
            v.flt_x2r = gpu_voices[i].flt_x2r;
            v.flt_y1r = gpu_voices[i].flt_y1r;
            v.flt_y2r = gpu_voices[i].flt_y2r;
        }
        drop(vdata);
        buf.staging_voice[idx].unmap();
        buf.staging_idx = 1 - buf.staging_idx;

        voice_count
    }

    /// 渲染一块音频（返回新分配的 Vec，兼容旧接口）。
    pub fn render_block(
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

/// CPU reference implementation (与 GPU shader pass1 逐帧逻辑完全对应).
/// 7 阶段: 0=Delay, 1=Attack, 2=Hold, 3=Decay, 4=Sustain, 5=Release, 6=Finished
/// 立体声采样 + 插值 + per-voice biquad 滤波均与 shader 一致，用于对比测试。
pub fn cpu_render_voices(
    sample_data: &[f32],
    voices: &mut [GpuVoiceState],
    frame_count: u32,
) -> Vec<f32> {
    let mut output = vec![0.0f32; frame_count as usize * 2];
    for voice in voices.iter_mut() {
        for fi in 0..frame_count as usize {
            if voice.env_stage >= 6 {
                break;
            }
            if fi < voice.start_offset as usize {
                continue;
            }
            let frame_in_voice = fi - voice.start_offset as usize;

            // 通道渐变逐帧推进（与 shader/xsynth ValueLerp 一致）
            if voice.ch_vol_frames > 0 {
                voice.ch_vol += voice.ch_vol_step;
                voice.ch_vol_frames -= 1;
            }
            if voice.ch_expr_frames > 0 {
                voice.ch_expr += voice.ch_expr_step;
                voice.ch_expr_frames -= 1;
            }
            if voice.ch_pan_frames > 0 {
                voice.ch_pan += voice.ch_pan_step;
                voice.ch_pan_frames -= 1;
            }
            let ch_vol = voice.ch_vol * voice.ch_expr;
            let ch_gain = voice.base_gain * ch_vol * ch_vol;
            let ch_ang = voice.ch_pan * std::f32::consts::FRAC_PI_2;
            let ch_pan_l = voice.base_pan_l * ch_ang.cos();
            let ch_pan_r = voice.base_pan_r * ch_ang.sin();

            let t = voice.time + frame_in_voice as f32 * voice.speed;
            let mut idx = t as u32;
            let frac = t - idx as f32;
            let max_idx = voice.sample_length.saturating_sub(1);

            // 循环回绕
            let has_loop = voice.loop_mode > 0 && voice.loop_end > voice.loop_start;
            if has_loop && idx >= voice.loop_end {
                let loop_len = voice.loop_end - voice.loop_start;
                if loop_len > 0 {
                    idx = voice.loop_start + ((idx - voice.loop_start) % loop_len);
                }
            }

            if idx < voice.sample_length {
                let scale = 1 + voice.is_stereo as usize;
                let i = voice.sample_offset as usize + idx as usize * scale;
                let (mut l0, mut r0) = if voice.is_stereo == 1 {
                    (sample_data[i], sample_data[i + 1])
                } else {
                    let s = sample_data[i];
                    (s, s)
                };
                if voice.interp == 1 && idx < max_idx {
                    let j = i + scale;
                    let (l1, r1) = if voice.is_stereo == 1 {
                        (sample_data[j], sample_data[j + 1])
                    } else {
                        let s = sample_data[j];
                        (s, s)
                    };
                    l0 += (l1 - l0) * frac;
                    r0 += (r1 - r0) * frac;
                }

                let mut s_l = l0 * ch_gain * voice.envelope;
                let mut s_r = r0 * ch_gain * voice.envelope;
                if voice.cutoff > 0.0 {
                    // 单声道样本只用一组滤波器，右声道复用左声道输出（与 shader/xsynth 一致）
                    let (x1, x2, y1, y2) = (voice.flt_x1, voice.flt_x2, voice.flt_y1, voice.flt_y2);
                    let out_l = voice.flt_b0 * s_l + voice.flt_b1 * x1 + voice.flt_b2 * x2
                        - voice.flt_a1 * y1
                        - voice.flt_a2 * y2;
                    voice.flt_x1 = s_l;
                    voice.flt_x2 = x1;
                    voice.flt_y1 = out_l;
                    voice.flt_y2 = y1;
                    s_l = out_l;
                    if voice.is_stereo == 1 {
                        let (x1r, x2r, y1r, y2r) =
                            (voice.flt_x1r, voice.flt_x2r, voice.flt_y1r, voice.flt_y2r);
                        let out_r = voice.flt_b0 * s_r + voice.flt_b1 * x1r + voice.flt_b2 * x2r
                            - voice.flt_a1 * y1r
                            - voice.flt_a2 * y2r;
                        voice.flt_x1r = s_r;
                        voice.flt_x2r = x1r;
                        voice.flt_y1r = out_r;
                        voice.flt_y2r = y1r;
                        s_r = out_r;
                    } else {
                        s_r = s_l;
                    }
                }
                output[fi * 2] += s_l * ch_pan_l;
                output[fi * 2 + 1] += s_r * ch_pan_r;
            }
            advance_env_cpu(voice);
        }
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.time += voice.speed * active_frames as f32;
        voice.start_offset = 0;
    }
    output
}

/// 逐帧推进 envelope（与 shader `advance_env` 完全对应）。
fn advance_env_cpu(v: &mut GpuVoiceState) {
    if v.env_stage >= 6 {
        return;
    }
    let peak = v.env_level;
    let sus = v.sustain_level * peak;
    match v.env_stage {
        0 => {
            // Delay
            if v.stage_progress + 1.0 >= v.delay_frames {
                v.env_stage = 1;
                v.stage_progress = 0.0;
            } else {
                v.stage_progress += 1.0;
            }
        }
        1 => {
            // Attack: 线性
            let n = v.stage_progress + 1.0;
            if n >= v.attack_frames {
                v.envelope = peak;
                v.env_stage = 2;
                v.stage_progress = 0.0;
            } else {
                v.envelope = v.env_start + (peak - v.env_start) * (n / v.attack_frames);
                v.stage_progress = n;
            }
        }
        2 => {
            // Hold
            if v.stage_progress + 1.0 >= v.hold_frames {
                v.env_stage = 3;
                v.stage_progress = 0.0;
            } else {
                v.stage_progress += 1.0;
            }
        }
        3 => {
            // Decay: 指数 (1-t)^8
            let n = v.stage_progress + 1.0;
            if n >= v.decay_frames {
                v.envelope = sus;
                v.env_stage = 4;
                v.stage_progress = 0.0;
            } else {
                let t = n / v.decay_frames;
                v.envelope = sus + (peak - sus) * (1.0 - t).powi(8);
                v.stage_progress = n;
            }
        }
        4 => {
            // Sustain
            v.envelope = sus;
        }
        5 => {
            // Release: 指数 (1-t)^8
            let n = v.stage_progress + 1.0;
            if n >= v.release_frames {
                v.envelope = 0.0;
                v.env_stage = 6;
                v.stage_progress = 0.0;
            } else {
                let t = n / v.release_frames;
                v.envelope = v.env_start * (1.0 - t).powi(8);
                v.stage_progress = n;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sine_samples(len: usize, freq: f32, sr: f32) -> Vec<f32> {
        (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect()
    }

    fn make_voices(sample_len: u32, count: u32, speed: f32) -> Vec<GpuVoiceState> {
        (0..count)
            .map(|i| GpuVoiceState {
                sample_offset: (i % 4) * sample_len,
                sample_length: sample_len,
                speed,
                base_gain: 0.5,
                env_stage: 4,
                env_level: 1.0,
                sustain_level: 1.0,
                base_pan_l: 1.0,
                base_pan_r: 1.0,
                ch_vol: 1.0,
                ch_expr: 1.0,
                ch_pan: 0.5,
                ..Default::default()
            })
            .collect()
    }

    fn setup_gpu() -> Option<(GpuAudioRenderer, Vec<f32>)> {
        let mut renderer = GpuAudioRenderer::new_default().ok()?;
        let sample_len = 4096u32;
        let samples: Vec<f32> = (0..4)
            .flat_map(|inst| {
                make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
            })
            .collect();
        renderer.upload_samples(&samples);
        let limits = renderer.device.limits();
        eprintln!(
            "GPU limits: min_storage_buf_align={} max_buf_binding={}",
            limits.min_storage_buffer_offset_alignment, limits.max_storage_buffer_binding_size
        );
        Some((renderer, samples))
    }

    fn bench_samples(sample_len: u32) -> Vec<f32> {
        (0..4)
            .flat_map(|inst| {
                make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
            })
            .collect()
    }

    #[test]
    fn phase15_single_pass_smoke() {
        let (mut renderer, _samples) = match setup_gpu() {
            Some(r) => r,
            None => {
                eprintln!("No GPU");
                return;
            }
        };
        let mut voices = make_voices(4096, 16, 1.0);
        let result = renderer.render_block(&mut voices, 1024, 44100);
        assert_eq!(result.len(), 1024 * 2);
        assert!(result.iter().fold(0.0f32, |m, &s| m.max(s.abs())) > 0.0);
    }

    #[test]
    fn phase15_benchmark() {
        let (mut renderer, _samples) = match setup_gpu() {
            Some(r) => r,
            None => {
                eprintln!("No GPU");
                return;
            }
        };
        let sample_len = 4096u32;
        let samples = bench_samples(sample_len);
        let frame_count = 1024u32;

        for &vc in &[4, 16, 64, 256, 1024, 4096, 15000] {
            let mut voices = make_voices(sample_len, vc, 1.0);
            for _ in 0..3 {
                let _ = renderer.render_block(&mut voices, frame_count, 44100);
            }
            let n = 10;
            let gpu_start = std::time::Instant::now();
            for _ in 0..n {
                let _ = renderer.render_block(&mut voices, frame_count, 44100);
            }
            let gpu_per_block = gpu_start.elapsed() / n;
            let cpu_start = std::time::Instant::now();
            for _ in 0..n {
                let mut v = make_voices(sample_len, vc, 1.0);
                let _ = cpu_render_voices(&samples, &mut v, frame_count);
            }
            let cpu_per_block = cpu_start.elapsed() / n;
            let speedup = cpu_per_block.as_secs_f64() / gpu_per_block.as_secs_f64();
            eprintln!(
                "Voices={vc:>6}: CPU={cpu_per_block:>8.2?} GPU={gpu_per_block:>8.2?} speedup={speedup:.2}x"
            );
        }
    }

    /// 立体声交错样本（LRLR）
    fn make_stereo_samples(len: usize, freq: f32, sr: f32) -> Vec<f32> {
        let l: Vec<f32> = (0..len)
            .map(|i| 0.8 * (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect();
        let r: Vec<f32> = (0..len)
            .map(|i| 0.6 * (2.0 * std::f32::consts::PI * freq * 1.5 * i as f32 / sr).sin())
            .collect();
        l.into_iter().zip(r).flat_map(|(l, r)| [l, r]).collect()
    }

    /// GPU 与 CPU 参考实现逐 block 对比（含立体声、滤波器、跨 block IIR 状态、全 7 阶段包络）
    #[test]
    fn gpu_vs_cpu_correctness() {
        let (mut renderer, _) = match setup_gpu() {
            Some(r) => r,
            None => {
                eprintln!("No GPU");
                return;
            }
        };

        let sample_len = 4096u32; // 帧数
        let samples = make_stereo_samples(sample_len as usize, 220.0, 44100.0);
        renderer.upload_samples(&samples);
        renderer.buffers = None;

        let make_test_voices = |stage: u32| {
            vec![
                // 立体声 + LowPass 滤波器 + Nearest
                GpuVoiceState {
                    sample_offset: 0,
                    sample_length: sample_len,
                    speed: 1.0,
                    base_gain: 0.4,
                    env_stage: stage,
                    env_level: 1.0,
                    sustain_level: 0.3,
                    delay_frames: 40.0,
                    attack_frames: 300.0,
                    hold_frames: 120.0,
                    decay_frames: 400.0,
                    release_frames: 500.0,
                    base_pan_l: 0.8,
                    base_pan_r: 0.6,
                    ch_vol: 1.0,
                    ch_expr: 1.0,
                    ch_pan: 0.5,
                    is_stereo: 1,
                    interp: 0,
                    cutoff: 1800.0,
                    resonance: 2.0,
                    filter_type: 0,
                    flt_b0: biquad_coeffs(0, 1800.0, 2.0, 44100.0).0,
                    flt_b1: biquad_coeffs(0, 1800.0, 2.0, 44100.0).1,
                    flt_b2: biquad_coeffs(0, 1800.0, 2.0, 44100.0).2,
                    flt_a1: biquad_coeffs(0, 1800.0, 2.0, 44100.0).3,
                    flt_a2: biquad_coeffs(0, 1800.0, 2.0, 44100.0).4,
                    ..Default::default()
                },
                // 单声道 + 无滤波器 + Linear 插值 + 循环
                GpuVoiceState {
                    sample_offset: 0,
                    sample_length: sample_len,
                    speed: 0.7,
                    base_gain: 0.3,
                    env_stage: stage,
                    env_level: 1.0,
                    sustain_level: 0.6,
                    delay_frames: 40.0,
                    attack_frames: 300.0,
                    hold_frames: 120.0,
                    decay_frames: 400.0,
                    release_frames: 500.0,
                    base_pan_l: 0.5,
                    base_pan_r: 1.0,
                    ch_vol: 1.0,
                    ch_expr: 1.0,
                    ch_pan: 0.5,
                    loop_mode: 1,
                    loop_start: 100,
                    loop_end: 2048,
                    is_stereo: 0,
                    interp: 1,
                    ..Default::default()
                },
                // 立体声 + HighPass + 偏移起始
                GpuVoiceState {
                    sample_offset: 0,
                    sample_length: sample_len,
                    speed: 1.0,
                    base_gain: 0.2,
                    start_offset: 13,
                    env_stage: stage,
                    env_level: 1.0,
                    sustain_level: 0.9,
                    delay_frames: 40.0,
                    attack_frames: 300.0,
                    hold_frames: 120.0,
                    decay_frames: 400.0,
                    release_frames: 500.0,
                    base_pan_l: 1.0,
                    base_pan_r: 0.3,
                    ch_vol: 1.0,
                    ch_expr: 1.0,
                    ch_pan: 0.5,
                    is_stereo: 1,
                    cutoff: 4000.0,
                    resonance: 3.0,
                    filter_type: 1,
                    flt_b0: biquad_coeffs(1, 4000.0, 3.0, 44100.0).0,
                    flt_b1: biquad_coeffs(1, 4000.0, 3.0, 44100.0).1,
                    flt_b2: biquad_coeffs(1, 4000.0, 3.0, 44100.0).2,
                    flt_a1: biquad_coeffs(1, 4000.0, 3.0, 44100.0).3,
                    flt_a2: biquad_coeffs(1, 4000.0, 3.0, 44100.0).4,
                    ..Default::default()
                },
            ]
        };

        let frame_count = 512u32;
        // 从 Delay 起步连续渲染 6 个 block：覆盖 attack/hold/decay 阶段切换 + 跨 block 滤波器状态
        let mut gpu_voices = make_test_voices(0);
        let mut cpu_voices = gpu_voices.clone();
        for block in 0..6 {
            let mut out_gpu = vec![0.0f32; frame_count as usize * 2];
            renderer.render_into(&mut gpu_voices, &mut out_gpu, 44100);
            // 真实路径在渲染后推进 time/env（与 GpuSynth::render 一致）
            advance_voices(&mut gpu_voices, frame_count);
            let out_cpu = cpu_render_voices(&samples, &mut cpu_voices, frame_count);

            let mut max_diff = 0.0f32;
            for (a, b) in out_gpu.iter().zip(&out_cpu) {
                max_diff = max_diff.max((a - b).abs());
            }
            assert!(
                max_diff < 1e-3,
                "block {block}: max diff {max_diff} (gpu[0]={} cpu[0]={})",
                out_gpu[0],
                out_cpu[0]
            );
            // 滤波器 IIR 状态跨 block 一致
            for (g, c) in gpu_voices.iter().zip(&cpu_voices) {
                assert!(
                    (g.flt_y1 - c.flt_y1).abs() < 1e-3
                        && (g.flt_y2 - c.flt_y2).abs() < 1e-3
                        && (g.flt_x1 - c.flt_x1).abs() < 1e-3
                        && (g.flt_x2 - c.flt_x2).abs() < 1e-3,
                    "block {block}: filter state mismatch"
                );
            }
        }
    }

    /// biquad 系数在 CPU 与 GPU 同源（测试防线）
    #[test]
    fn biquad_coeffs_reference() {
        // LowPass 1kHz Q=1（RBJ cookbook 已知值）
        let (b0, b1, _b2, a1, a2) = biquad_coeffs(0, 1000.0, 1.0, 44100.0);
        let omega = 2.0 * std::f32::consts::PI * 1000.0 / 44100.0;
        let alpha = omega.sin() / (2.0 * 1.0);
        let a0 = 1.0 + alpha;
        assert!((b0 - ((1.0 - omega.cos()) * 0.5) / a0).abs() < 1e-6);
        assert!((b1 - (1.0 - omega.cos()) / a0).abs() < 1e-6);
        assert!((a1 - (-2.0 * omega.cos()) / a0).abs() < 1e-6);
        assert!((a2 - (1.0 - alpha) / a0).abs() < 1e-6);
    }

    /// GPU 滤波器 vs biquad crate（权威参照）：系数直接用 biquad crate 生成，
    /// 排除系数计算差异，验证 DF1 状态方程、包络推进与跨 block 状态传递。
    #[test]
    fn gpu_filter_matches_biquad_crate() {
        use biquad::Biquad as _;
        use biquad::frequency::ToHertz;

        let (mut renderer, _) = match setup_gpu() {
            Some(r) => r,
            None => {
                eprintln!("No GPU");
                return;
            }
        };
        let sr = 44100.0f32;
        // 扫频样本（覆盖滤波频段，频谱丰富）
        let sample_len = 8192u32;
        let samples: Vec<f32> = (0..sample_len as usize)
            .map(|i| {
                let t = i as f32 / sr;
                (2.0 * std::f32::consts::PI * (200.0 + 8000.0 * t / 0.2) * t).sin()
            })
            .collect();
        renderer.upload_samples(&samples);
        renderer.buffers = None;

        // 用 biquad crate 生成系数（权威来源）
        let cutoff = 1195.0f32;
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let coeffs = biquad::Coefficients::<f32>::from_params(
            biquad::Type::LowPass,
            sr.hz(),
            cutoff.hz(),
            q,
        )
        .unwrap();

        let make_voice = |with_filter: bool| GpuVoiceState {
            sample_length: sample_len,
            speed: 1.0,
            base_gain: 0.5,
            env_stage: 4,
            env_level: 1.0,
            sustain_level: 1.0,
            base_pan_l: 1.0,
            base_pan_r: 1.0,
            // ch_pan=0 → cos(0)=1，左声道无衰减（与下方 expected 一致）
            ch_vol: 1.0,
            ch_expr: 1.0,
            ch_pan: 0.0,
            cutoff: if with_filter { cutoff } else { 0.0 },
            resonance: q,
            filter_type: 0,
            flt_b0: coeffs.b0,
            flt_b1: coeffs.b1,
            flt_b2: coeffs.b2,
            flt_a1: coeffs.a1,
            flt_a2: coeffs.a2,
            ..Default::default()
        };

        // 对照组：无滤波（cutoff=0）——验证采样读取与 envelope 通路
        let frame_count = 512u32;
        {
            let mut voices = vec![make_voice(false)];
            let mut out = vec![0.0f32; frame_count as usize * 2];
            renderer.render_into(&mut voices, &mut out, sr as u32);
            advance_voices(&mut voices, frame_count);
            for fi in 0..8 {
                let expected = samples[fi] * 0.5; // sustain env=1.0, base_gain=0.5, mono, ch_pan=0 → cos=1
                assert!(
                    (out[fi * 2] - expected).abs() < 1e-4,
                    "nofilter frame {fi}: gpu={} expected={}",
                    out[fi * 2],
                    expected
                );
            }
        }

        // 滤波路径：4 个连续 block，验证跨 block IIR 状态传递
        let mut gpu_voices = vec![make_voice(true)];
        let mut cpu_filters = [
            biquad::DirectForm1::<f32>::new(coeffs),
            biquad::DirectForm1::<f32>::new(coeffs),
        ];

        for block in 0..4 {
            let mut out_gpu = vec![0.0f32; frame_count as usize * 2];
            renderer.render_into(&mut gpu_voices, &mut out_gpu, sr as u32);
            advance_voices(&mut gpu_voices, frame_count);

            // 参照：逐帧 biquad crate 滤波（单声道，左右同值）
            let start = block as usize * frame_count as usize;
            for fi in 0..frame_count as usize {
                let input = samples[start + fi] * 0.5;
                let out = cpu_filters[0].run(input);
                assert!(
                    (out_gpu[fi * 2] - out).abs() < 1e-3,
                    "block {block} frame {fi}: gpu={} biquad={}",
                    out_gpu[fi * 2],
                    out
                );
            }
        }
    }
}
