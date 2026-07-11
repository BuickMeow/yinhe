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
    let mut block_count: u64 = 0;
    let mut total_render_time: u128 = 0;
    let mut max_voice: u64 = 0;

    // ── Phase 1: render the main content (notes + CC events) ──
    while rendered < main_duration {
        if cancel.as_ref().map_or(false, |c| c.load(Ordering::Relaxed)) {
            return Err(ExportError::Cancelled);
        }
        let frames = ((main_duration - rendered) as usize).min(RENDER_CHUNK_FRAMES);
        let buf = &mut chunk[..frames * STEREO_CHANNELS];
        let t_render = Instant::now();
        engine.render(buf);
        total_render_time += t_render.elapsed().as_micros();
        block_count += 1;
        let vc = engine.voice_count();
        if vc > max_voice {
            max_voice = vc;
        }
        if block_count % 200 == 0 {
            let avg_us = total_render_time / block_count as u128;
            let rendered_secs = rendered as f64 / sample_rate as f64;
            eprintln!(
                "[export] block={block_count} rendered={rendered_secs:.1}s voices={vc} avg_block={avg_us}µs"
            );
        }
        if use_limiter {
            limiter.limit(buf);
        }

        write_samples(&mut writer, buf, bit_depth)?;

        rendered += frames as u64;
        let pct = 0.05 + (rendered as f32 / main_duration as f32) * 0.85;
        progress(pct, &format!("渲染中 {:.0}%", pct * 100.0));

        if let Some(ref ep) = export_progress {
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
        "[export_wav timing] model={:.2?} sf={:.2?} render={:.2?} total={:.2?} blocks={block_count} avg_block={avg_block}µs max_voice={max_voice}",
        t_model, t_sf, t_render, t_total,
        avg_block = if block_count > 0 { total_render_time / block_count as u128 } else { 0 },
    );

    Ok(())
}

fn write_samples(
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
