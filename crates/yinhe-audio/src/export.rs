use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use xsynth_core::effects::VolumeLimiter;
use yinhe_core::YinModel;

use crate::engine::AudioEngine;
use crate::spawn::channels_for_model;

/// Shared export progress state, updated from the background thread.
#[derive(Clone)]
pub struct ExportProgress {
    pub visible: bool,
    pub progress: f32,
    pub status: String,
    pub total_duration_secs: f64,
    pub rendered_secs: f64,
    pub started_at: Option<Instant>,
    pub voice_count: u64,
    /// Real-time speed of the most recent render chunk.
    pub render_speed: f64,
    /// Overall average speed since rendering started.
    pub overall_speed: f64,
}

impl ExportProgress {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            visible: false,
            progress: 0.0,
            status: String::new(),
            total_duration_secs: 0.0,
            rendered_secs: 0.0,
            started_at: None,
            voice_count: 0,
            render_speed: 0.0,
            overall_speed: 0.0,
        }))
    }

    pub fn reset(&mut self) {
        self.visible = true;
        self.progress = 0.0;
        self.status = "准备中…".into();
        self.total_duration_secs = 0.0;
        self.rendered_secs = 0.0;
        self.started_at = Some(Instant::now());
        self.voice_count = 0;
        self.render_speed = 0.0;
        self.overall_speed = 0.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WavBitDepth {
    Bit16,
    Bit24,
    Bit32Float,
}

#[derive(Debug)]
pub enum ExportError {
    Io(String),
    Render(String),
    Cancelled,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(msg) => write!(f, "IO error: {}", msg),
            ExportError::Render(msg) => write!(f, "Render error: {}", msg),
            ExportError::Cancelled => write!(f, "Export cancelled"),
        }
    }
}

impl From<hound::Error> for ExportError {
    fn from(e: hound::Error) -> Self {
        ExportError::Io(e.to_string())
    }
}

const STEREO_CHANNELS: usize = 2;
const RENDER_CHUNK_FRAMES: usize = 1024;
/// Safety limit: stop rendering tails after this many seconds even if voices
/// are still active (prevents infinite loop on stuck voices).
const MAX_TAIL_SECONDS: f64 = 30.0;

pub fn export_wav(
    model: Arc<YinModel>,
    sample_rate: u32,
    port_soundfonts: &[(u8, Vec<String>)],
    skip_tracks: &[bool],
    path: &Path,
    bit_depth: WavBitDepth,
    layer_count: Option<usize>,
    progress: impl Fn(f32, &str),
    export_progress: Option<Arc<Mutex<ExportProgress>>>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<(), ExportError> {
    let t_start = Instant::now();
    let (_num_ch, active_mask) = channels_for_model(&model);

    let mut engine = AudioEngine::new(sample_rate, 0, active_mask);

    progress(0.0, "加载 MIDI");
    engine.handle_command(crate::spawn::AudioCommand::LoadModel {
        model: Arc::clone(&model),
    });
    let t_model = t_start.elapsed();

    engine.set_layer_count(layer_count);

    let total_sf: usize = port_soundfonts.iter().map(|(_, p)| p.len()).sum();
    let mut sf_loaded = 0usize;
    for (port, paths) in port_soundfonts {
        for (i, _p) in paths.iter().enumerate() {
            sf_loaded += 1;
            progress(
                sf_loaded as f32 / total_sf.max(1) as f32 * 0.05,
                &format!("加载音色库 {}/{} …", sf_loaded, total_sf),
            );
            engine.handle_command(crate::spawn::AudioCommand::LoadSoundFont {
                port: *port,
                paths: paths[i..i + 1].to_vec(),
            });
        }
    }
    let t_sf = t_start.elapsed() - t_model;

    progress(0.05, "应用音轨静音");
    engine.handle_command(crate::spawn::AudioCommand::SkipTracks {
        skip: skip_tracks.to_vec(),
    });

    let main_duration = engine.duration_samples();
    if main_duration == 0 {
        return Err(ExportError::Render("歌曲时长为零，没有可导出的内容".into()));
    }

    if let Some(ref ep) = export_progress {
        if let Ok(mut p) = ep.lock() {
            p.total_duration_secs = main_duration as f64 / sample_rate as f64;
        }
    }

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

    engine.handle_command(crate::spawn::AudioCommand::Play { from_sample: 0 });

    let use_limiter = bit_depth != WavBitDepth::Bit32Float;
    let mut limiter = VolumeLimiter::new(STEREO_CHANNELS as u16);

    let mut chunk = vec![0.0f32; RENDER_CHUNK_FRAMES * STEREO_CHANNELS];
    let mut rendered: u64 = 0;
    let mut prev_rendered_secs: f64 = 0.0;
    let mut prev_instant = Instant::now();

    // ── Phase 1: render the main content (notes + CC events) ──
    while rendered < main_duration {
        if cancel.as_ref().map_or(false, |c| c.load(Ordering::Relaxed)) {
            return Err(ExportError::Cancelled);
        }
        let frames = ((main_duration - rendered) as usize).min(RENDER_CHUNK_FRAMES);
        let buf = &mut chunk[..frames * STEREO_CHANNELS];
        engine.render(buf);
        if use_limiter {
            limiter.limit(buf);
        }

        write_samples(&mut writer, buf, bit_depth)?;

        rendered += frames as u64;
        let pct = 0.05 + (rendered as f32 / main_duration as f32) * 0.85;
        progress(pct, &format!("渲染中 {:.0}%", pct * 100.0));

        // Update export progress every ~100 blocks to reduce lock overhead
        if let Some(ref ep) = export_progress {
            if rendered % (RENDER_CHUNK_FRAMES as u64 * 100) < RENDER_CHUNK_FRAMES as u64 {
                if let Ok(mut p) = ep.lock() {
                    p.rendered_secs = rendered as f64 / sample_rate as f64;
                    p.voice_count = engine.voice_count();
                    let now = Instant::now();
                    let dt_wall = prev_instant.elapsed().as_secs_f64();
                    let dt_rendered = p.rendered_secs - prev_rendered_secs;
                    if dt_wall > 0.0 {
                        p.render_speed = dt_rendered / dt_wall;
                    }
                    if let Some(start) = p.started_at {
                        let elapsed = start.elapsed().as_secs_f64();
                        if elapsed > 0.0 {
                            p.overall_speed = p.rendered_secs / elapsed;
                        }
                    }
                    prev_rendered_secs = p.rendered_secs;
                    prev_instant = now;
                }
            }
        }
    }

    // ── Phase 2: tail — let release tails decay naturally ──
    let max_tail_samples = (MAX_TAIL_SECONDS * sample_rate as f64) as u64;
    let mut tail_rendered: u64 = 0;

    loop {
        if cancel.as_ref().map_or(false, |c| c.load(Ordering::Relaxed)) {
            return Err(ExportError::Cancelled);
        }
        let frames = RENDER_CHUNK_FRAMES.min((max_tail_samples - tail_rendered) as usize);
        if frames == 0 {
            break;
        }
        let buf = &mut chunk[..frames * STEREO_CHANNELS];
        engine.render(buf);
        if use_limiter {
            limiter.limit(buf);
        }

        write_samples(&mut writer, buf, bit_depth)?;

        tail_rendered += frames as u64;

        // Check if all voices have finished (including release phase)
        let vc = engine.voice_count();
        if vc == 0 {
            break;
        }

        let tail_pct = tail_rendered as f32 / max_tail_samples as f32;
        let overall = 0.90 + tail_pct * 0.09;
        progress(
            overall,
            &format!("余韵衰减中 (剩余 {} 音色)", vc),
        );

        if let Some(ref ep) = export_progress {
            if let Ok(mut p) = ep.lock() {
                p.rendered_secs = (rendered + tail_rendered) as f64 / sample_rate as f64;
                p.voice_count = vc;
                let now = Instant::now();
                let dt_wall = prev_instant.elapsed().as_secs_f64();
                let dt_rendered = p.rendered_secs - prev_rendered_secs;
                if dt_wall > 0.0 {
                    p.render_speed = dt_rendered / dt_wall;
                }
                if let Some(start) = p.started_at {
                    let elapsed = start.elapsed().as_secs_f64();
                    if elapsed > 0.0 {
                        p.overall_speed = p.rendered_secs / elapsed;
                    }
                }
                prev_rendered_secs = p.rendered_secs;
                prev_instant = now;
            }
        }
    }

    progress(0.99, "写入文件");
    let t_render = t_start.elapsed() - t_sf - t_model;
    writer.finalize()?;
    let t_total = t_start.elapsed();
    progress(1.0, "导出完成");

    eprintln!(
        "[export_wav timing] model={:.2?} sf={:.2?} render={:.2?} total={:.2?}",
        t_model, t_sf, t_render, t_total,
    );

    Ok(())
}

// ── GPU 导出路径 ──

/// 使用 GPU 合成器（GpuSynth）导出 WAV。
///
/// 与 `export_wav` 统一代码路径：创建 GpuSynth → 构建 SynthEvent → render 循环。
#[cfg(feature = "gpu")]
pub fn export_wav_gpu(
    model: Arc<YinModel>,
    sample_rate: u32,
    sfz_path: &Path,
    skip_tracks: &[bool],
    path: &Path,
    bit_depth: WavBitDepth,
    progress: impl Fn(f32, &str),
    device: Arc<yinhe_synth::wgpu::Device>,
    queue: Arc<yinhe_synth::wgpu::Queue>,
) -> Result<(), ExportError> {
    let t_start = Instant::now();

    // ── 1. 初始化 GpuSynth ──
    progress(0.0, "初始化 GPU 合成器...");
    let mut synth = yinhe_synth::GpuSynth::new(device, queue, sfz_path, sample_rate)
        .map_err(|e| ExportError::Render(format!("GpuSynth 初始化失败: {}", e)))?;
    eprintln!("[gpu-export] GpuSynth initialized: {:.2?}", t_start.elapsed());

    // ── 2. 构建 SynthEvent 列表 ──
    progress(0.02, "构建事件列表...");
    let t_events = Instant::now();
    let audio_model = crate::audio_model::AudioModel::from_model(&model);
    let segments = &model.tempo_map.tempo_segments;
    let tpb = model.tempo_map.ticks_per_beat;
    let sr = sample_rate as f64;
    let (_num_ch, active_mask) = crate::spawn::channels_for_model(&model);

    let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();
    for key in 0..128usize {
        for note in model.notes[key].iter() {
            if note.velocity <= 1 { continue; }
            let track = note.track as usize;
            if track < skip_tracks.len() && skip_tracks[track] { continue; }
            let ch = audio_model.track_channel(track) as usize;
            if !active_mask.get(ch).copied().unwrap_or(false) { continue; }

            let start_sample = crate::audio_model::tick_to_sample(note.start_tick as u64, segments, tpb, sr);
            let end_sample = crate::audio_model::tick_to_sample(note.end_tick as u64, segments, tpb, sr);

            events.push(yinhe_synth::SynthEvent {
                sample: start_sample,
                key: key as u8,
                velocity: note.velocity,
                is_on: true,
            });
            events.push(yinhe_synth::SynthEvent {
                sample: end_sample,
                key: key as u8,
                velocity: 0,
                is_on: false,
            });
        }
    }
    events.sort_by_key(|e| e.sample);
    eprintln!("[gpu-export] Built {} events in {:.2?}", events.len(), t_events.elapsed());

    // ── 3. 加载事件到合成器 ──
    progress(0.04, "加载事件到 GPU 合成器...");
    synth.load_events(events);

    // ── 4. 计算总时长 ──
    let main_duration = {
        let mut max_sample = 0u64;
        for key in 0..128usize {
            if let Some(last_note) = model.notes[key].last() {
                let end = crate::audio_model::tick_to_sample(last_note.end_tick as u64, segments, tpb, sr);
                if end > max_sample { max_sample = end; }
            }
        }
        max_sample
    };
    if main_duration == 0 {
        return Err(ExportError::Render("歌曲时长为零，没有可导出的内容".into()));
    }
    let audio_secs = main_duration as f64 / sample_rate as f64;

    // ── 5. 设置 WAV 写入器 ──
    progress(0.05, "准备写入 WAV...");
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

    // ── 6. 渲染循环 ──
    progress(0.06, "GPU 渲染中...");
    let _t_render = Instant::now();
    let mut chunk = vec![0.0f32; RENDER_CHUNK_FRAMES * STEREO_CHANNELS];
    let mut rendered: u64 = 0;

    while rendered < main_duration {
        let frames = ((main_duration - rendered) as usize).min(RENDER_CHUNK_FRAMES);
        let buf = &mut chunk[..frames * STEREO_CHANNELS];
        synth.render(buf);
        // GpuSynth::render 内部已经做了限幅
        write_samples(&mut writer, buf, bit_depth)?;

        rendered = synth.sample_position();
        let pct = 0.06 + (rendered as f32 / main_duration as f32) * 0.90;
        if rendered % (RENDER_CHUNK_FRAMES as u64 * 50) < RENDER_CHUNK_FRAMES as u64 {
            progress(pct, &format!("GPU 渲染中 {:.0}%", pct * 100.0));
        }
    }

    // ── 7. 余韵衰减（让 release 尾音自然消失）──
    let max_tail_samples = (MAX_TAIL_SECONDS * sample_rate as f64) as u64;
    let mut tail_rendered: u64 = 0;

    loop {
        let frames = RENDER_CHUNK_FRAMES.min((max_tail_samples - tail_rendered) as usize);
        if frames == 0 { break; }
        let buf = &mut chunk[..frames * STEREO_CHANNELS];
        synth.render(buf);
        write_samples(&mut writer, buf, bit_depth)?;

        tail_rendered += frames as u64;
        // GpuSynth 没有 voice_count()，用余韵时长上限判断
        let tail_pct = tail_rendered as f32 / max_tail_samples as f32;
        let overall = 0.96 + tail_pct * 0.03;
        progress(overall, "余韵衰减中...");

        if tail_rendered >= max_tail_samples {
            break;
        }
    }

    progress(0.99, "写入文件...");
    writer.finalize()?;
    let total = t_start.elapsed();
    let rtf = audio_secs / total.as_secs_f64();
    eprintln!("[gpu-export] Done: {:.2?} (rtf={:.1}x, audio={:.1}s)", total, rtf, audio_secs);
    progress(1.0, "导出完成");

    Ok(())
}

pub(crate) fn write_samples(
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    buf: &[f32],
    bit_depth: WavBitDepth,
) -> Result<(), hound::Error> {
    match bit_depth {
        WavBitDepth::Bit16 => {
            for &s in buf.iter() {
                let clamped = s.clamp(-1.0, 1.0);
                writer.write_sample((clamped * i16::MAX as f32) as i16)?;
            }
        }
        WavBitDepth::Bit24 => {
            for &s in buf.iter() {
                let clamped = s.clamp(-1.0, 1.0);
                let val = (clamped * 8_388_607.0) as i32;
                writer.write_sample(val)?;
            }
        }
        WavBitDepth::Bit32Float => {
            for &s in buf.iter() {
                writer.write_sample(s)?;
            }
        }
    }
    Ok(())
}
