//! GPU-accelerated audio export — replaces CPU xsynth rendering with
//! direct SFZ loading + MIDI event dispatch + GPU compute shader rendering.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::audio_model::{AudioModel, tick_to_sample};
use crate::export::{ExportError, WavBitDepth, write_samples};
use crate::gpu_renderer::{GpuAudioRenderer, GpuVoiceState};
use crate::sfz_parser::SfzSoundfont;
use yinhe_core::YinModel;

/// Active voice state tracked on the CPU side.
struct ActiveVoice {
    key: u8,
    velocity: u8,
    channel: u8,
    sample_offset: u32,
    sample_length: u32,
    pitch_keycenter: u8,
    release_time: f32,
    start_sample: u64,
    end_sample: u64,
    /// Current time in the sample (for GPU state sync).
    time: f32,
    speed: f32,
}

/// Convert MIDI note number to pitch ratio relative to pitch_keycenter.
fn pitch_ratio(midi_note: u8, pitch_keycenter: u8) -> f32 {
    2.0f32.powf((midi_note as f32 - pitch_keycenter as f32) / 12.0)
}

/// Render audio using GPU acceleration.
///
/// This bypasses xsynth entirely: loads SFZ samples directly, dispatches
/// MIDI events, builds GPU voice states, and renders via compute shader.
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
    let sfz = SfzSoundfont::parse(sfz_path)
        .map_err(|e| ExportError::Render(format!("SFZ parse error: {}", e)))?;
    let key_map = sfz.build_key_map();
    eprintln!("[gpu] SFZ parsed: {} regions, {:.2?}", sfz.regions.len(), t_start.elapsed());

    // ── 2. Load WAV samples ──
    progress(0.02, "加载采样...");
    let t_load = Instant::now();
    let mut sample_data: Vec<f32> = Vec::new();
    let mut sample_offsets: Vec<u32> = vec![0; 128]; // offset per key
    let mut sample_lengths: Vec<u32> = vec![0; 128];

    for key in 0..128u8 {
        let (ref sample_path, _pkc, _release) = key_map[key as usize];
        if sample_path.to_string_lossy() == "missing" {
            continue;
        }
        let offset = sample_data.len() as u32;
        match crate::sfz_parser::load_wav_as_f32(sample_path) {
            Ok(samples) => {
                let len = samples.len() as u32;
                sample_data.extend_from_slice(&samples);
                sample_offsets[key as usize] = offset;
                sample_lengths[key as usize] = len;
            }
            Err(e) => {
                eprintln!("[gpu] Warning: failed to load {:?}: {}", sample_path, e);
            }
        }
    }
    eprintln!("[gpu] Loaded {} samples ({} frames, {:.2?})",
        key_map.iter().filter(|(p, _, _)| !p.to_string_lossy().contains("missing")).count(),
        sample_data.len(),
        t_load.elapsed(),
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

    // Collect all note events (on + off) sorted by sample position
    let mut events: Vec<(u64, u8, u8, u8, bool)> = Vec::new(); // (sample, key, velocity, channel, is_on)
    let segments = &model.tempo_map.tempo_segments;
    let tpb = model.tempo_map.ticks_per_beat;
    let sr = sample_rate as f64;

    for key in 0..128usize {
        for note in model.notes[key].iter() {
            if note.velocity <= 1 {
                continue;
            }
            let track = note.track as usize;
            if track < skip_tracks.len() && skip_tracks[track] {
                continue;
            }
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
                let end = tick_to_sample(
                    last_note.end_tick as u64, segments, tpb, sr,
                );
                if end > max_sample {
                    max_sample = end;
                }
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
    let block_size: u64 = 1024;
    let mut rendered: u64 = 0;
    let mut event_cursor: usize = 0;
    let mut active_voices: Vec<ActiveVoice> = Vec::new();
    let mut output = vec![0.0f32; block_size as usize * 2];
    let mut gpu_voices: Vec<GpuVoiceState> = Vec::new();

    while rendered < main_duration {
        let block_end = (rendered + block_size).min(main_duration);
        let frames = (block_end - rendered) as usize;

        // ── 7a. Dispatch events in this block ──
        // NoteOn events
        while event_cursor < events.len() {
            let (sample, key, vel, ch, is_on) = events[event_cursor];
            if sample > block_end {
                break;
            }
            if sample >= rendered {
                if is_on {
                    // Find sample data for this key
                    let offset = sample_offsets[key as usize];
                    let length = sample_lengths[key as usize];
                    if length > 0 {
                        let (_, pkc, release) = key_map[key as usize];
                        let speed = pitch_ratio(key, pkc);
                        active_voices.push(ActiveVoice {
                            key,
                            velocity: vel,
                            channel: ch,
                            sample_offset: offset,
                            sample_length: length,
                            pitch_keycenter: pkc,
                            release_time: release,
                            start_sample: sample,
                            end_sample: 0, // will be set by NoteOff
                            time: 0.0,
                            speed,
                        });
                    }
                } else {
                    // NoteOff — find and remove matching voice
                    if let Some(pos) = active_voices.iter().position(|v| v.key == key && v.end_sample == 0) {
                        active_voices[pos].end_sample = sample;
                    }
                }
            }
            event_cursor += 1;
        }

        // Remove voices that have ended
        active_voices.retain(|v| v.end_sample == 0 || v.end_sample > rendered);

        // ── 7b. Build GPU voice states ──
        gpu_voices.clear();
        for v in &active_voices {
            // Calculate gain from velocity (simple linear mapping)
            let gain = v.velocity as f32 / 127.0;
            // Calculate current time in the sample
            let sample_offset = (rendered).saturating_sub(v.start_sample) as f32;
            let time = sample_offset * v.speed;

            gpu_voices.push(GpuVoiceState {
                sample_offset: v.sample_offset,
                sample_length: v.sample_length,
                speed: v.speed,
                gain,
                time,
                envelope: 0.0,  // will be set by GPU shader (attack)
                env_stage: 0,   // attack
                env_level: gain,
                _pad: 0,
            });
        }

        // ── 7c. GPU render ──
        output.fill(0.0);
        if !gpu_voices.is_empty() {
            let gpu_out = renderer.render_block(&gpu_voices, frames as u32, sample_rate);
            // Copy GPU output to our buffer (handling frame count mismatch)
            let copy_frames = frames.min(gpu_out.len() / 2);
            for i in 0..copy_frames {
                output[i * 2] = gpu_out[i * 2];
                output[i * 2 + 1] = gpu_out[i * 2 + 1];
            }
        }

        // ── 7d. Write to WAV ──
        let buf = &output[..frames * 2];
        write_samples(&mut writer, buf, bit_depth)?;

        rendered = block_end;

        // Progress update every ~100 blocks
        if rendered % (block_size * 100) < block_size {
            let pct = 0.08 + (rendered as f32 / main_duration as f32) * 0.90;
            progress(pct, &format!("GPU 渲染中 {:.0}%", pct * 100.0));
        }
    }

    progress(0.99, "写入文件...");
    writer.finalize()?;
    let total = t_start.elapsed();
    let rtf = audio_secs / total.as_secs_f64();
    eprintln!("[gpu] Export done: {:.2?} (rtf={:.1}x, audio={:.1}s)", total, rtf, audio_secs);
    progress(1.0, "导出完成");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_ratio_test() {
        // Middle C (60) relative to itself should be 1.0
        assert!((pitch_ratio(60, 60) - 1.0).abs() < 0.001);
        // One octave up should be 2.0
        assert!((pitch_ratio(72, 60) - 2.0).abs() < 0.001);
        // One octave down should be 0.5
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

        // Create GPU device/queue
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
                    elapsed,
                    file_size / 1024 / 1024,
                    rtf,
                    audio_secs,
                );
            }
            Err(e) => {
                eprintln!("GPU export failed: {e}");
            }
        }
    }
}
