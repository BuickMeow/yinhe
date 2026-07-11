//! GPU-accelerated audio export — bypasses xsynth, loads SFZ samples directly,
//! dispatches MIDI events, and renders via GPU compute shader.
//!
//! Voice 状态（time/envelope/env_stage）跨块在 CPU 端维护，GPU shader 只读。

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::audio_model::{AudioModel, tick_to_sample};
use crate::export::{ExportError, WavBitDepth, write_samples};
use crate::gpu_renderer::{GpuAudioRenderer, GpuVoiceState, advance_voices};
use crate::sfz_parser;
use yinhe_core::YinModel;

/// Convert MIDI note number to pitch ratio relative to pitch_keycenter.
fn pitch_ratio(midi_note: u8, pitch_keycenter: u8) -> f32 {
    2.0f32.powf((midi_note as f32 - pitch_keycenter as f32) / 12.0)
}

/// Render audio using GPU acceleration.
pub fn export_wav_gpu(
    model: Arc<YinModel>,
    sample_rate: u32,
    sfz_path: &Path,
    skip_tracks: &[bool],
    path: &Path,
    bit_depth: WavBitDepth,
    progress: impl Fn(f32, &str),
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
) -> Result<(), ExportError> {
    let t_start = Instant::now();

    // ── 1. Parse SFZ ──
    progress(0.0, "解析 SFZ...");
    let key_map = sfz_parser::build_key_map_from_sfz(sfz_path)
        .map_err(|e| ExportError::Render(format!("SFZ parse error: {}", e)))?;
    eprintln!("[gpu] SFZ parsed: {} keys, {:.2?}", key_map.len(), t_start.elapsed());

    // ── 2. Load WAV samples ──
    progress(0.02, "加载采样...");
    let t_load = Instant::now();
    let mut sample_cache: std::collections::HashMap<std::path::PathBuf, (u32, u32)> = std::collections::HashMap::new();
    let mut sample_data: Vec<f32> = Vec::new();

    // 收集所有 key 的所有 velocity layer 的采样文件
    for key_layers in &key_map {
        for info in key_layers {
            if info.sample_path.to_string_lossy() == "missing" { continue; }
            if sample_cache.contains_key(&info.sample_path) { continue; }
            let offset = sample_data.len() as u32;
            match crate::sfz_parser::load_wav_as_f32(&info.sample_path) {
                Ok((samples, src_sr)) => {
                    let samples = if src_sr != sample_rate {
                        // sinc 重采样到目标采样率（与 xsynth 一致）
                        eprintln!("[gpu] Resampling {:?}: {} → {}Hz", info.sample_path.file_name(), src_sr, sample_rate);
                        xsynth_soundfonts::resample::resample_vec(samples, src_sr as f32, sample_rate as f32).to_vec()
                    } else {
                        samples
                    };
                    let len = samples.len() as u32;
                    sample_data.extend_from_slice(&samples);
                    sample_cache.insert(info.sample_path.clone(), (offset, len));
                }
                Err(e) => {
                    eprintln!("[gpu] Warning: failed to load {:?}: {}", info.sample_path, e);
                }
            }
        }
    }
    eprintln!("[gpu] Loaded {} unique samples ({} total frames, {:.2?})",
        sample_cache.len(), sample_data.len(), t_load.elapsed(),
    );

    let audio_model = AudioModel::from_model(&model);

    // ── 3. Upload samples to GPU ──
    progress(0.05, "上传采样到 GPU...");
    let t_gpu = Instant::now();
    let mut renderer = GpuAudioRenderer::new(device, queue)
        .map_err(|e| ExportError::Render(format!("GPU init error: {}", e)))?;
    renderer.upload_samples(&sample_data);
    eprintln!("[gpu] GPU init + sample upload: {:.2?}", t_gpu.elapsed());

    // ── 4. Build sorted event list ──
    progress(0.06, "构建事件列表...");
    let t_events = Instant::now();
    let mut events: Vec<(u64, u8, u8, u8, bool)> = Vec::new(); // (sample, key, velocity, channel, is_on)
    let segments = &model.tempo_map.tempo_segments;
    let tpb = model.tempo_map.ticks_per_beat;
    let sr = sample_rate as f64;

    for key in 0..128usize {
        for note in model.notes[key].iter() {
            if note.velocity <= 1 { continue; }
            let track = note.track as usize;
            if track < skip_tracks.len() && skip_tracks[track] { continue; }
            let ch = audio_model.track_channel(track);
            let start_sample = tick_to_sample(note.start_tick as u64, segments, tpb, sr);
            let end_sample = tick_to_sample(note.end_tick as u64, segments, tpb, sr);
            events.push((start_sample, key as u8, note.velocity, ch, true));
            events.push((end_sample, key as u8, 0, ch, false));
        }
    }
    events.sort_by_key(|e| e.0);
    eprintln!("[gpu] Built {} events in {:.2?}", events.len(), t_events.elapsed());

    // ── 5. Compute total duration ──
    let main_duration = {
        let mut max_sample = 0u64;
        for key in 0..128usize {
            if let Some(last_note) = model.notes[key].last() {
                let end = tick_to_sample(last_note.end_tick as u64, segments, tpb, sr);
                if end > max_sample { max_sample = end; }
            }
        }
        max_sample
    };
    if main_duration == 0 {
        return Err(ExportError::Render("歌曲时长为零".into()));
    }
    let audio_secs = main_duration as f64 / sample_rate as f64;
    eprintln!("[gpu] Duration: {:.1}s, {} blocks", audio_secs, main_duration / 1024);

    // ── 6. Set up WAV writer ──
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: match bit_depth {
            WavBitDepth::Bit16 => 16,
            WavBitDepth::Bit24 => 24,
            WavBitDepth::Bit32Float => 32,
        },
        sample_format: match bit_depth {
            WavBitDepth::Bit32Float => hound::SampleFormat::Float,
            _ => hound::SampleFormat::Int,
        },
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(ExportError::from)?;

    // ── 7. Render loop ──
    progress(0.08, "GPU 渲染中...");
    let t_render = Instant::now();
    let block_size: u64 = 1024;
    let mut rendered: u64 = 0;
    let mut event_cursor: usize = 0;

    // 持久 voice 状态：跨块维护 time/envelope/env_stage
    let mut voices: Vec<GpuVoiceState> = Vec::new();
    let mut voice_keys: Vec<u8> = Vec::new(); // 每个 voice 对应的 MIDI key
    let mut output = vec![0.0f32; block_size as usize * 2];

    // XSynth 风格动态限幅器（跨块维护 loudness 状态）
    let mut limiter = xsynth_core::effects::VolumeLimiter::new(2);

    while rendered < main_duration {
        let block_end = (rendered + block_size).min(main_duration);
        let frames = (block_end - rendered) as u32;

        // ── 7a. Dispatch events ──
        while event_cursor < events.len() {
            let (sample, key, vel, _ch, is_on) = events[event_cursor];
            if sample > block_end { break; }
            if sample >= rendered {
                if is_on {
                    // 根据 key + velocity 选择对应的 SFZ region（力度分层）
                    let info = match sfz_parser::select_key_info(&key_map, key, vel) {
                        Some(i) => i,
                        None => { event_cursor += 1; continue; }
                    };
                    if let Some(&(offset, length)) = sample_cache.get(&info.sample_path) {
                        if length > 0 {
                            let sr = sample_rate as f32;
                            // pitch ratio: (key - pitch_keycenter + tune/100) / 12
                            let pitch_semitones = (key as f32 - info.pitch_keycenter as f32)
                                + info.tune as f32 / 100.0;
                            let speed = 2.0f32.powf(pitch_semitones / 12.0);

                            // 力度增益: amp_veltrack 曲线
                            // vel_norm = vel/127, gain = vel_norm^(100/amp_veltrack) 当 amp_veltrack>0
                            let vel_norm = vel as f32 / 127.0;
                            let vel_gain = if info.amp_veltrack >= 100.0 {
                                vel_norm
                            } else {
                                vel_norm.powf(100.0 / info.amp_veltrack.max(1.0))
                            };
                            let gain = vel_gain * info.volume;

                            // pan → left/right gain (等功率声像)
                            let (pan_l, pan_r) = if info.pan == 0.0 {
                                (1.0, 1.0)
                            } else {
                                let angle = info.pan * std::f32::consts::FRAC_PI_4;
                                (angle.cos(), angle.sin())
                            };

                            // ADSR 帧数
                            let sr_f = sample_rate as f32;
                            let delay_frames = info.ampeg_delay * sr_f;
                            let attack_frames = info.ampeg_attack * sr_f;
                            let hold_frames = info.ampeg_hold * sr_f;
                            let decay_frames = info.ampeg_decay * sr_f;
                            let release_frames = info.ampeg_release * sr_f;

                            let start_offset = (sample - rendered) as u32;
                            voices.push(GpuVoiceState {
                                sample_offset: offset + info.offset,
                                sample_length: length - info.offset.min(length),
                                speed,
                                gain,
                                time: 0.0,
                                start_offset,
                                envelope: info.ampeg_start,
                                env_stage: 0, // Delay
                                stage_progress: 0.0,
                                env_level: gain,
                                sustain_level: info.ampeg_sustain,
                                env_start: info.ampeg_start,
                                delay_frames,
                                attack_frames,
                                hold_frames,
                                decay_frames,
                                release_frames,
                                pan_left: pan_l,
                                pan_right: pan_r,
                                loop_start: info.loop_start,
                                loop_end: info.loop_end,
                                loop_mode: info.loop_mode as u32,
                            });
                            voice_keys.push(key);
                        }
                    }
                } else {
                    // NoteOff → 触发 release（匹配最近的同 key voice）
                    for i in (0..voices.len()).rev() {
                        if voice_keys[i] == key && voices[i].env_stage < 5u32 {
                            // 保存当前 envelope 值作为 release 起始值
                            voices[i].env_start = voices[i].envelope;
                            voices[i].env_stage = 5; // Release
                            voices[i].stage_progress = 0.0;
                            break;
                        }
                    }
                }
            }
            event_cursor += 1;
        }

        // ── 7b. 移除已结束的 voice ──
        let mut i = 0;
        while i < voices.len() {
            if voices[i].env_stage >= 6 || (voices[i].time as u32) >= voices[i].sample_length {
                voices.swap_remove(i);
                voice_keys.swap_remove(i);
            } else {
                i += 1;
            }
        }

        // ── 7c. GPU render ──
        output.fill(0.0);
        if !voices.is_empty() {
            let gpu_out = renderer.render_block(&voices, frames, sample_rate);
            output[..frames as usize * 2].copy_from_slice(&gpu_out[..frames as usize * 2]);
        }

        // ── 7d. 推进 voice 状态 ──
        advance_voices(&mut voices, frames);

        // ── 7e. 动态限幅（与 XSynth VolumeLimiter 一致）──
        let buf = &mut output[..frames as usize * 2];
        limiter.limit(buf);

        // ── 7f. Write to WAV ──
        write_samples(&mut writer, buf, bit_depth)?;

        rendered = block_end;

        if rendered % (block_size * 100) < block_size {
            let pct = 0.08 + (rendered as f32 / main_duration as f32) * 0.90;
            progress(pct, &format!("GPU 渲染中 {:.0}%", pct * 100.0));
        }
    }

    progress(0.99, "写入文件...");
    writer.finalize()?;
    let total = t_start.elapsed();
    let rtf = audio_secs / total.as_secs_f64();
    eprintln!("[gpu] Render loop: {:.2?}", t_render.elapsed());
    eprintln!("[gpu] Export done: {:.2?} (rtf={:.1}x, audio={:.1}s)", total, rtf, audio_secs);
    progress(1.0, "导出完成");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_ratio_test() {
        assert!((pitch_ratio(60, 60) - 1.0).abs() < 0.001);
        assert!((pitch_ratio(72, 60) - 2.0).abs() < 0.001);
        assert!((pitch_ratio(48, 60) - 0.5).abs() < 0.001);
    }

    #[test]
    fn gpu_export_benchmark() {
        let midi_path = "/Users/jieneng/Music/MIDIs/Mesmerizer.mid";
        let sfz_path = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";

        if !std::path::Path::new(midi_path).exists() || !std::path::Path::new(sfz_path).exists() {
            eprintln!("Files not found, skipping");
            return;
        }

        let model = Arc::new(yinhe_mid2::parse_path(midi_path).unwrap());
        let skip: Vec<bool> = vec![false; model.tracks.len()];
        let sample_rate = 48000u32;

        eprintln!("=== GPU Export Benchmark ===");
        let tmp = tempfile::NamedTempFile::new().unwrap();

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
        })).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("gpu_export_test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits {
                    max_storage_buffer_binding_size: 512 * 1024 * 1024,
                    max_buffer_size: 512 * 1024 * 1024,
                    ..wgpu::Limits::default()
                },
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            },
        )).unwrap();
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let t_start = std::time::Instant::now();
        let result = export_wav_gpu(
            Arc::clone(&model),
            sample_rate,
            std::path::Path::new(sfz_path),
            &skip,
            tmp.path(),
            crate::export::WavBitDepth::Bit24,
            |_pct, _msg| {},
            device,
            queue,
        );
        let elapsed = t_start.elapsed();

        match result {
            Ok(()) => {
                let file_size = std::fs::metadata(tmp.path()).unwrap().len();
                let audio_secs = model.tick_length as f64 / model.tempo_map.ticks_per_beat as f64 / 120.0 * 60.0;
                let rtf = audio_secs / elapsed.as_secs_f64();
                eprintln!(
                    "=== GPU DONE === time={:.2?} file={}MB rtf={:.1}x audio={:.1}s",
                    elapsed, file_size / 1024 / 1024, rtf, audio_secs,
                );
            }
            Err(e) => {
                eprintln!("GPU export failed: {e}");
            }
        }
    }
}
