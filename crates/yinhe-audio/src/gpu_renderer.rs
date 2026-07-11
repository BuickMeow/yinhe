//! GPU-accelerated audio renderer for offline export.
//!
//! Phase 1.5+: Single-pass merged shader, persistent buffers.
//! The voice render and merge passes are combined into one shader
//! that uses shared memory tree reduction, eliminating the 120MB
//! intermediate buffer entirely.

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

/// Persistent GPU state — all buffers allocated once, reused every block.
struct GpuBuffers {
    sample_buf: wgpu::Buffer,
    voice_state_buf: wgpu::Buffer,
    max_voices: u32,
    final_output_buf: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    staging: [wgpu::Buffer; 2],
    staging_idx: usize,
    bind_groups: [wgpu::BindGroup; 2],
}

/// GPU-accelerated audio renderer with persistent buffers.
pub struct GpuAudioRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    buffers: Option<GpuBuffers>,
    pending_samples: Option<wgpu::Buffer>,
    frame_count: u32,
}

impl GpuAudioRenderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        let shader_source = include_str!("shaders/voice_render.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voice_render"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("audio_render_bgl"),
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

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            buffers: None,
            pending_samples: None,
            frame_count: 0,
        })
    }

    /// Upload soundfont sample data. Call once before rendering begins.
    pub fn upload_samples(&mut self, sample_data: &[f32]) {
        let sample_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gpu_samples"),
                contents: bytemuck::cast_slice(sample_data),
                usage: wgpu::BufferUsages::STORAGE,
            });
        self.buffers = None;
        self.pending_samples = Some(sample_buf);
    }

    fn ensure_buffers(&mut self, voice_count: u32, frame_count: u32) {
        let needs_recreate = match (&self.buffers, &self.pending_samples) {
            (Some(b), _) => b.max_voices < voice_count || self.frame_count < frame_count,
            (None, Some(_)) => true,
            (None, None) => return,
        };

        if !needs_recreate {
            return;
        }

        let device = &self.device;
        let sample_buf = if let Some(buf) = self.pending_samples.take() {
            buf
        } else {
            self.buffers.as_ref().unwrap().sample_buf.clone()
        };

        let voice_state_size =
            (voice_count.max(1) as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
        let final_output_size =
            (frame_count.max(1) as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let params_size = std::mem::size_of::<RenderParams>() as u64;

        let voice_state_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_voice_states"),
            size: voice_state_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
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
            label: Some("gpu_staging_0"),
            size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_staging_1"),
            size: final_output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let make_bg = |sample: &wgpu::Buffer, voice: &wgpu::Buffer, output: &wgpu::Buffer, params: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("audio_bg"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: sample.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: voice.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: params.as_entire_binding(),
                    },
                ],
            })
        };

        self.buffers = Some(GpuBuffers {
            sample_buf: sample_buf.clone(),
            voice_state_buf: voice_state_buf.clone(),
            max_voices: voice_count.max(1),
            final_output_buf: final_output_buf.clone(),
            params_buf: params_buf.clone(),
            staging: [staging0, staging1],
            staging_idx: 0,
            bind_groups: [
                make_bg(&sample_buf, &voice_state_buf, &final_output_buf, &params_buf),
                make_bg(&sample_buf, &voice_state_buf, &final_output_buf, &params_buf),
            ],
        });
        self.frame_count = frame_count;
    }

    /// Render a block of audio using the GPU.
    pub fn render_block(
        &mut self,
        voices: &[GpuVoiceState],
        frame_count: u32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let voice_count = voices.len() as u32;
        if voice_count == 0 || frame_count == 0 {
            return vec![0.0; frame_count as usize * 2];
        }

        self.ensure_buffers(voice_count, frame_count);
        let buf = self.buffers.as_mut().unwrap();

        // Upload voice states and params (zero-allocation writes)
        self.queue
            .write_buffer(&buf.voice_state_buf, 0, bytemuck::cast_slice(voices));

        let params = RenderParams {
            frame_count,
            voice_count,
            sample_rate,
            _pad: 0,
        };
        self.queue
            .write_buffer(&buf.params_buf, 0, bytemuck::bytes_of(&params));

        // Encode single compute pass
        let idx = buf.staging_idx;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("audio_render"),
            });

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voice_merge_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.pipeline);
            cpass.set_bind_group(0, &buf.bind_groups[idx], &[]);
            // One workgroup per output frame, 256 threads per workgroup.
            cpass.dispatch_workgroups(frame_count, 1, 1);
        }

        let final_output_size =
            (frame_count as usize * 2 * std::mem::size_of::<f32>()) as u64;
        encoder.copy_buffer_to_buffer(
            &buf.final_output_buf,
            0,
            &buf.staging[idx],
            0,
            final_output_size,
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Readback
        let buffer_slice = buf.staging[idx].slice(..);
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
        buf.staging[idx].unmap();

        buf.staging_idx = 1 - buf.staging_idx;

        result
    }
}

/// CPU reference implementation.
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
            let t = voice.time;
            let idx = t as u32;
            let frac = t - idx as f32;
            let max_idx = voice.sample_length.saturating_sub(1);
            let a = sample_data
                [voice.sample_offset as usize + (idx as usize).min(max_idx as usize)];
            let b = sample_data[voice.sample_offset as usize
                + ((idx + 1) as usize).min(max_idx as usize)];
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
                gain: 0.5,
                time: 0.0,
                envelope: 0.0,
                env_stage: 0,
                env_level: 1.0,
                _pad: 0,
            })
            .collect()
    }

    fn setup_gpu() -> Option<GpuAudioRenderer> {
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
            Err(_) => return None,
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
        let mut renderer = GpuAudioRenderer::new(device, queue).unwrap();

        let sample_len = 4096u32;
        let samples: Vec<f32> = (0..4)
            .flat_map(|inst| {
                make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
            })
            .collect();
        renderer.upload_samples(&samples);
        Some(renderer)
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
        let mut renderer = match setup_gpu() {
            Some(r) => r,
            None => {
                eprintln!("No GPU adapter, skipping");
                return;
            }
        };
        let voices = make_voices(4096, 16, 1.0);
        let result = renderer.render_block(&voices, 1024, 44100);
        assert_eq!(result.len(), 1024 * 2);
        let max_abs = result.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(max_abs > 0.0);
    }

    #[test]
    fn phase15_benchmark() {
        let mut renderer = match setup_gpu() {
            Some(r) => r,
            None => {
                eprintln!("No GPU adapter, skipping benchmark");
                return;
            }
        };
        let sample_len = 4096u32;
        let samples = bench_samples(sample_len);
        let frame_count = 1024u32;

        for &voice_count in &[4, 16, 64, 256, 1024, 4096, 15000] {
            let voices = make_voices(sample_len, voice_count, 1.0);

            // Warm up
            for _ in 0..3 {
                let _ = renderer.render_block(&voices, frame_count, 44100);
            }

            // GPU benchmark
            let n = 10;
            let gpu_start = std::time::Instant::now();
            for _ in 0..n {
                let _ = renderer.render_block(&voices, frame_count, 44100);
            }
            let gpu_elapsed = gpu_start.elapsed();

            // CPU benchmark
            let cpu_start = std::time::Instant::now();
            for _ in 0..n {
                let mut v = make_voices(sample_len, voice_count, 1.0);
                let _ = cpu_render_voices(&samples, &mut v, frame_count);
            }
            let cpu_elapsed = cpu_start.elapsed();

            let speedup = cpu_elapsed.as_secs_f64() / gpu_elapsed.as_secs_f64();
            eprintln!(
                "Voices={voice_count:>6}: CPU={:>9.2?} GPU={:>9.2?} speedup={speedup:.2}x",
                cpu_elapsed / n,
                gpu_elapsed / n,
            );
        }
    }

    #[test]
    fn real_midi_export_benchmark() {
        let midi_path = "/Users/jieneng/Music/MIDIs/Mesmerizer.mid";
        let sfz_path = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";

        if !std::path::Path::new(midi_path).exists() {
            eprintln!("MIDI file not found, skipping");
            return;
        }
        if !std::path::Path::new(sfz_path).exists() {
            eprintln!("SFZ file not found, skipping");
            return;
        }

        // Parse MIDI
        eprintln!("Parsing MIDI...");
        let t0 = std::time::Instant::now();
        let model = match yinhe_mid2::parse_path(midi_path) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                eprintln!("Failed to parse MIDI: {e}");
                return;
            }
        };
        eprintln!("  MIDI parsed in {:.2?}", t0.elapsed());
        eprintln!(
            "  tick_length={}, tracks={}, notes_est={}",
            model.tick_length,
            model.tracks.len(),
            model.notes.iter().map(|n| n.len()).sum::<usize>()
        );

        // CPU export benchmark
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let sfz_str = sfz_path.to_string();
        let port_sfs: [(u8, Vec<String>); 1] = [(0, vec![sfz_str.clone()])];
        let skip: Vec<bool> = vec![false; model.tracks.len()];
        let sample_rate = 48000u32;

        // Time the engine setup + SFZ loading separately
        eprintln!("Setting up engine + loading SFZ...");
        let t_engine = std::time::Instant::now();
        let (_num_ch, active_mask) = crate::spawn::channels_for_model(&model);
        let mut engine = crate::engine::AudioEngine::new(sample_rate, 0, active_mask);
        engine.handle_command(crate::spawn::AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        eprintln!("  LoadModel: {:.2?}", t_engine.elapsed());

        let t_sf = std::time::Instant::now();
        engine.handle_command(crate::spawn::AudioCommand::LoadSoundFont {
            port: 0,
            paths: vec![sfz_str.clone()],
        });
        eprintln!("  LoadSoundFont: {:.2?}", t_sf.elapsed());
        eprintln!("  Total engine setup: {:.2?}", t_engine.elapsed());

        let cpu_start = std::time::Instant::now();
        let result = crate::export::export_wav(
            Arc::clone(&model),
            sample_rate,
            &port_sfs,
            &skip,
            tmp.path(),
            crate::export::WavBitDepth::Bit24,
            None, // unlimited layers
            |pct, msg| {
                // suppress progress output
            },
            None,
            None,
        );
        let cpu_elapsed = cpu_start.elapsed();

        match result {
            Ok(()) => {
                let file_size = std::fs::metadata(tmp.path()).unwrap().len();
                eprintln!(
                    "CPU export: {:.2?} (file size: {} MB)",
                    cpu_elapsed,
                    file_size / 1024 / 1024
                );
            }
            Err(e) => {
                eprintln!("CPU export failed: {e}");
            }
        }

        // Also time the GPU synthetic benchmark at the same voice count
        // to estimate potential speedup.
        if let Some(mut renderer) = setup_gpu() {
            let sample_len = 4096u32;
            // We don't know the exact voice count from the MIDI, but we can
            // estimate from the export. For now, benchmark at various counts.
            for &vc in &[256, 1024, 4096, 15000] {
                let voices = make_voices(sample_len, vc, 1.0);
                for _ in 0..3 {
                    let _ = renderer.render_block(&voices, 1024, sample_rate);
                }
                let n = 10;
                let gpu_start = std::time::Instant::now();
                for _ in 0..n {
                    let _ = renderer.render_block(&voices, 1024, sample_rate);
                }
                let gpu_per_block = gpu_start.elapsed() / n;
                eprintln!(
                    "  GPU @ {vc} voices: {gpu_per_block:.2?}/block (1024 frames)"
                );
            }
        }
    }
}
