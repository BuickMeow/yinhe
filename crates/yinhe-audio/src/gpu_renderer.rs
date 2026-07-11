//! GPU-accelerated audio renderer for offline export.
//!
//! Uses wgpu compute shaders to render all voices in parallel,
//! providing significant speedup for complex arrangements with
//! many active voices.

use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Per-voice state that is uploaded to the GPU each block.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVoiceState {
    pub sample_offset: u32,
    pub sample_length: u32,
    pub speed: f32,
    pub gain: f32,
    pub time: f32,
    pub envelope: f32,
    pub env_stage: u32,
    pub env_level: f32,
    pub _pad: u32,
}

/// Uniform buffer for render parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RenderParams {
    pub frame_count: u32,
    pub voice_count: u32,
    pub sample_rate: u32,
    pub _pad: u32,
}

/// GPU-accelerated audio renderer.
pub struct GpuAudioRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    voice_pipeline: wgpu::ComputePipeline,
    merge_pipeline: wgpu::ComputePipeline,
    voice_layout: wgpu::BindGroupLayout,
    merge_layout: wgpu::BindGroupLayout,
}

impl GpuAudioRenderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        let shader_source = include_str!("shaders/voice_render.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voice_render"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // Voice render pipeline
        let voice_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("voice_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let voice_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("voice_pipeline_layout"),
                bind_group_layouts: &[Some(&voice_layout)],
                immediate_size: 0,
            });

        let voice_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voice_pipeline"),
            layout: Some(&voice_pipeline_layout),
            module: &shader_module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Merge pipeline
        let merge_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("merge_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let merge_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("merge_pipeline_layout"),
                bind_group_layouts: &[None, Some(&merge_layout)],
                immediate_size: 0,
            });

        let merge_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("merge_pipeline"),
            layout: Some(&merge_pipeline_layout),
            module: &shader_module,
            entry_point: Some("merge_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            voice_pipeline,
            merge_pipeline,
            voice_layout,
            merge_layout,
        })
    }

    /// Render a block of audio using the GPU.
    ///
    /// `sample_data`: all soundfont samples concatenated into one f32 buffer.
    /// `voices`: per-voice state (sample offset, speed, gain, envelope, etc.)
    /// `frame_count`: number of stereo frames to render.
    /// `sample_rate`: output sample rate.
    ///
    /// Returns a stereo f32 buffer of length `frame_count * 2`.
    pub fn render_block(
        &self,
        sample_data: &[f32],
        voices: &[GpuVoiceState],
        frame_count: u32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let voice_count = voices.len() as u32;
        if voice_count == 0 || frame_count == 0 {
            return vec![0.0; frame_count as usize * 2];
        }

        let device = &self.device;

        let sample_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("samples"),
            contents: bytemuck::cast_slice(sample_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let voice_state_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voice_states"),
            contents: bytemuck::cast_slice(voices),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let voice_output_size =
            (voice_count as usize * frame_count as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let voice_output_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voice_outputs"),
            size: voice_output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let final_output_size = (frame_count as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let final_output_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("final_output"),
            size: final_output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let params = RenderParams {
            frame_count,
            voice_count,
            sample_rate,
            _pad: 0,
        };
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let voice_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voice_bind_group"),
            layout: &self.voice_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sample_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: voice_state_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: voice_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let merge_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("merge_bind_group"),
            layout: &self.merge_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: voice_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: final_output_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("audio_render"),
        });

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voice_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.voice_pipeline);
            cpass.set_bind_group(0, &voice_bind_group, &[]);
            let workgroups_x = (voice_count + 255) / 256;
            cpass.dispatch_workgroups(workgroups_x, 1, 1);
        }

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("merge_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.merge_pipeline);
            cpass.set_bind_group(1, &merge_bind_group, &[]);
            let workgroups_x = (frame_count + 255) / 256;
            cpass.dispatch_workgroups(workgroups_x, 1, 1);
        }

        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(&final_output_buf, 0, &staging, 0, final_output_size);
        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();

        result
    }
}

/// CPU reference implementation of the same voice rendering logic.
/// Used for correctness validation and performance comparison.
pub fn cpu_render_voices(
    sample_data: &[f32],
    voices: &mut [GpuVoiceState],
    frame_count: u32,
) -> Vec<f32> {
    let mut output = vec![0.0f32; frame_count as usize * 2];

    for voice in voices.iter_mut() {
        for i in 0..frame_count as usize {
            if voice.env_stage >= 4 {
                continue;
            }
            // Envelope
            match voice.env_stage {
                0 => {
                    voice.envelope += 0.01;
                    if voice.envelope >= voice.env_level {
                        voice.envelope = voice.env_level;
                        voice.env_stage = 1;
                    }
                }
                1 => {
                    voice.envelope -= 0.005;
                    if voice.envelope <= 0.7 {
                        voice.envelope = 0.7;
                        voice.env_stage = 2;
                    }
                }
                2 => {}
                3 => {
                    voice.envelope -= 0.02;
                    if voice.envelope <= 0.0 {
                        voice.envelope = 0.0;
                        voice.env_stage = 4;
                    }
                }
                _ => {}
            }
            // Sample lookup
            let t = voice.time;
            let idx = t as u32;
            let frac = t - idx as f32;
            let a = sample_data
                [voice.sample_offset as usize + (idx as usize).min(voice.sample_length as usize - 1)];
            let b = sample_data[voice.sample_offset as usize
                + ((idx + 1) as usize).min(voice.sample_length as usize - 1)];
            let sample = a + (b - a) * frac;
            let out = sample * voice.gain * voice.envelope;
            output[i * 2] += out;
            output[i * 2 + 1] += out;
            voice.time += voice.speed;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a sine wave sample buffer for testing.
    fn make_sine_samples(len: usize, freq: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin())
            .collect()
    }

    /// Create synthetic voices for benchmarking.
    fn make_voices(
        _sample_data: &[f32],
        sample_len: u32,
        count: u32,
        speed: f32,
    ) -> Vec<GpuVoiceState> {
        (0..count)
            .map(|i| GpuVoiceState {
                sample_offset: (i % 4) * sample_len,
                sample_length: sample_len,
                speed,
                gain: 0.5,
                time: 0.0,
                envelope: 0.0,
                env_stage: 0,
                env_level: 1.0,
                _pad: 0,
            })
            .collect()
    }

    #[test]
    fn gpu_renderer_smoke_test() {
        // Create device synchronously for testing
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) {
            Ok(a) => a,
            Err(_) => {
                eprintln!("No GPU adapter found, skipping smoke test");
                return;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            },
        ))
        .unwrap();

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let renderer = GpuAudioRenderer::new(device, queue).unwrap();

        // 4 instruments × 1024 samples each
        let sample_len = 1024u32;
        let samples = make_sine_samples(sample_len as usize * 4, 440.0, 44100.0);

        let voices = make_voices(&samples, sample_len, 16, 1.0);
        let result = renderer.render_block(&samples, &voices, 1024, 44100);

        assert_eq!(result.len(), 1024 * 2);
        // Should have non-zero output (voices are playing)
        let max_abs = result.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(max_abs > 0.0, "GPU output should be non-zero");
    }

    #[test]
    fn cpu_vs_gpu_correctness() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) {
            Ok(a) => a,
            Err(_) => {
                eprintln!("No GPU adapter found, skipping correctness test");
                return;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            },
        ))
        .unwrap();

        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let renderer = GpuAudioRenderer::new(device, queue).unwrap();

        let sample_len = 256u32;
        // 4 instruments × sample_len
        let samples: Vec<f32> = (0..4)
            .flat_map(|inst| make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0))
            .collect();
        let voices = make_voices(&samples, sample_len, 4, 1.0);
        let gpu_result = renderer.render_block(&samples, &voices, 256, 44100);

        let mut voices_cpu = make_voices(&samples, sample_len, 4, 1.0);
        let cpu_result = cpu_render_voices(&samples, &mut voices_cpu, 256);

        assert_eq!(gpu_result.len(), cpu_result.len());
        // Allow small floating-point differences between GPU and CPU
        for (i, (g, c)) in gpu_result.iter().zip(cpu_result.iter()).enumerate() {
            let diff = (g - c).abs();
            assert!(
                diff < 0.01,
                "Sample {i}: GPU={g}, CPU={c}, diff={diff}"
            );
        }
    }

    #[test]
    fn benchmark_cpu_vs_gpu() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) {
            Ok(a) => a,
            Err(_) => {
                eprintln!("No GPU adapter found, skipping benchmark");
                return;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("bench"),
                ..Default::default()
            },
        ))
        .unwrap();

        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let renderer = GpuAudioRenderer::new(device, queue).unwrap();

        let sample_len = 4096u32;
        // 4 instruments × sample_len each
        let samples: Vec<f32> = (0..4)
            .flat_map(|inst| make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0))
            .collect();
        let frame_count = 4096u32;

        for &voice_count in &[4, 16, 64, 256] {
            // GPU benchmark
            let voices = make_voices(&samples, sample_len, voice_count, 1.0);
            let gpu_start = std::time::Instant::now();
            for _ in 0..10 {
                let _ = renderer.render_block(&samples, &voices, frame_count, 44100);
            }
            let gpu_elapsed = gpu_start.elapsed();

            // CPU benchmark
            let cpu_start = std::time::Instant::now();
            for _ in 0..10 {
                let mut v = make_voices(&samples, sample_len, voice_count, 1.0);
                let _ = cpu_render_voices(&samples, &mut v, frame_count);
            }
            let cpu_elapsed = cpu_start.elapsed();

            let speedup = cpu_elapsed.as_secs_f64() / gpu_elapsed.as_secs_f64();
            eprintln!(
                "Voices={voice_count}: CPU={:.2?} GPU={:.2?} speedup={speedup:.2}x",
                cpu_elapsed / 10,
                gpu_elapsed / 10,
            );
        }
    }
}
