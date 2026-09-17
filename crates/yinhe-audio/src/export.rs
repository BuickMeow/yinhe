use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use yinhe_dsp::dsp::limiter::VolumeLimiter;

use crate::engine::AudioEngine;

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
    /// 导出已结束（成功/失败/中止）：UI 轮询此标志收尾。
    pub finished: bool,
    /// 失败原因（None = 无错误；中止时为 "已中止"）。
    pub error: Option<String>,
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
            finished: false,
            error: None,
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
        self.finished = false;
        self.error = None;
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
/// Safety limit: stop rendering tails after this many seconds even if voices
/// are still active (prevents infinite loop on stuck voices).
const MAX_TAIL_SECONDS: f64 = 30.0;

// ── 渲染线程导出任务 ──

/// 渲染线程的导出任务：逐块离线渲染实时引擎（含全部 insert/乐器插件与
/// PDC 延迟补偿）并写 WAV。
///
/// 与实时播放共用同一个 [`AudioEngine`]：插件实例始终在原本的线程上，
/// 无需重新加载/重建，导出结果与实时听感一致（含混音台与插件链）。
pub(crate) struct ExportJob {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    bit_depth: WavBitDepth,
    limiter: VolumeLimiter,
    buf: Vec<f32>,
    sample_rate: u32,
    /// 主内容时长（采样）。
    main_duration: u64,
    /// 主内容已渲染（采样）。
    rendered: u64,
    /// 尾音已渲染（采样）。
    tail_rendered: u64,
    /// 尾音上限（采样；防 voice 卡死导致无限循环）。
    max_tail_samples: u64,
    /// 是否已进入尾音阶段。
    in_tail: bool,
    progress: Arc<Mutex<ExportProgress>>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    prev_instant: Instant,
    prev_rendered_secs: f64,
}

impl ExportJob {
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub(crate) fn new(
        path: &Path,
        bit_depth: WavBitDepth,
        sample_rate: u32,
        main_duration: u64,
        chunk_frames: usize,
        progress: Arc<Mutex<ExportProgress>>,
        cancel: Arc<AtomicBool>,
        pause: Arc<AtomicBool>,
    ) -> Result<Self, ExportError> {
        let spec = hound::WavSpec {
            channels: STEREO_CHANNELS as u16,
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
        let writer = hound::WavWriter::create(path, spec).map_err(ExportError::from)?;
        if let Ok(mut p) = progress.lock() {
            p.reset();
            p.total_duration_secs = main_duration as f64 / sample_rate as f64;
        }
        Ok(Self {
            writer,
            bit_depth,
            limiter: VolumeLimiter::new(sample_rate),
            buf: vec![0.0; chunk_frames * STEREO_CHANNELS],
            sample_rate,
            main_duration,
            rendered: 0,
            tail_rendered: 0,
            max_tail_samples: (MAX_TAIL_SECONDS * sample_rate as f64) as u64,
            in_tail: false,
            progress,
            cancel,
            pause,
            prev_instant: Instant::now(),
            prev_rendered_secs: 0.0,
        })
    }

    /// 是否被用户暂停（渲染线程跳过本块，保持命令处理）。
    pub(crate) fn paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }

    /// 是否被用户取消。
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 渲染下一块并写盘；返回 false = 导出内容已全部完成。
    pub(crate) fn step(&mut self, engine: &mut AudioEngine) -> Result<bool, ExportError> {
        let chunk = self.buf.len() / STEREO_CHANNELS;
        if !self.in_tail {
            if self.rendered >= self.main_duration {
                self.in_tail = true;
            } else {
                let n = ((self.main_duration - self.rendered) as usize).min(chunk);
                self.render_block(engine, n)?;
                self.rendered += n as u64;
                self.report_progress(engine);
                return Ok(true);
            }
        }
        // 尾音阶段：让 release 尾音自然衰减。
        if self.tail_rendered >= self.max_tail_samples {
            return Ok(false);
        }
        // 主内容刚结束：先探测是否还有尾音（voice/插件），已静音则立即收尾，
        // 不写多余的静音块。
        if self.tail_rendered == 0 && engine.voice_count() == 0 && !engine.has_active_plugins() {
            return Ok(false);
        }
        let remaining = (self.max_tail_samples - self.tail_rendered) as usize;
        let n = remaining.min(chunk);
        self.render_block(engine, n)?;
        self.tail_rendered += n as u64;
        self.report_progress(engine);
        // xsynth voice 与插件都静了才算尾音结束。插件（insert/乐器）的尾音
        // 无法逐块探测，有插件时按上限渲染满。
        if engine.voice_count() == 0 && !engine.has_active_plugins() {
            return Ok(false);
        }
        Ok(true)
    }

    fn render_block(&mut self, engine: &mut AudioEngine, n: usize) -> Result<(), ExportError> {
        let buf = &mut self.buf[..n * STEREO_CHANNELS];
        engine.render(buf);
        // 非浮点输出需要限幅（与实时输出一致）；32-bit float 保留原始幅度。
        if self.bit_depth != WavBitDepth::Bit32Float {
            self.limiter.limit(buf);
        }
        write_samples(&mut self.writer, buf, self.bit_depth)?;
        Ok(())
    }

    fn report_progress(&mut self, engine: &AudioEngine) {
        let progress = Arc::clone(&self.progress);
        let Ok(mut p) = progress.lock() else {
            return;
        };
        p.rendered_secs = (self.rendered + self.tail_rendered) as f64 / self.sample_rate as f64;
        p.voice_count = engine.voice_count();
        let now = Instant::now();
        let dt_wall = self.prev_instant.elapsed().as_secs_f64();
        let dt_rendered = p.rendered_secs - self.prev_rendered_secs;
        if dt_wall > 0.0 {
            p.render_speed = dt_rendered / dt_wall;
        }
        if let Some(start) = p.started_at {
            let elapsed = start.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                p.overall_speed = p.rendered_secs / elapsed;
            }
        }
        let frac = if self.in_tail {
            let tail_pct = self.tail_rendered as f32 / self.max_tail_samples.max(1) as f32;
            0.90 + tail_pct * 0.09
        } else {
            0.05 + (self.rendered as f32 / self.main_duration.max(1) as f32) * 0.85
        };
        p.progress = frac.min(0.99);
        p.status = if self.in_tail {
            "余韵衰减中".to_string()
        } else {
            format!("渲染中 {:.0}%", frac * 100.0)
        };
        self.prev_rendered_secs = p.rendered_secs;
        self.prev_instant = now;
    }

    /// 收尾写盘（成功路径；取消路径不调用，保留不完整文件）。
    pub(crate) fn finalize(mut self) -> Result<(), ExportError> {
        // 补上限幅器延迟线残留（末段 lookahead，约 3ms），否则尾部缺一小段。
        if self.bit_depth != WavBitDepth::Bit32Float {
            let latency = self.limiter.latency_frames();
            let mut buf = vec![0.0f32; latency * STEREO_CHANNELS];
            let frames = self.limiter.flush(&mut buf);
            if frames > 0 {
                write_samples(
                    &mut self.writer,
                    &buf[..frames * STEREO_CHANNELS],
                    self.bit_depth,
                )?;
            }
        }
        self.writer.finalize().map_err(ExportError::from)
    }

    /// 进度共享句柄（收尾时写完成状态）。
    pub(crate) fn progress_handle(&self) -> Arc<Mutex<ExportProgress>> {
        Arc::clone(&self.progress)
    }
}

/// 按位深写样本（导出写盘共用）。
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

#[cfg(test)]
mod job_tests {
    use super::*;
    use crate::channel_layout::ChannelLayout;
    use crate::engine::AudioEngine;
    use yinhe_core::{ConductorData, NoteEvent, ProjectMeta, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    /// 1 拍、单个音符的模型（120 BPM / PPQ 480）。
    fn tiny_model() -> Arc<YinModel> {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(vec![vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        }]]);
        model.rebuild();
        Arc::new(model)
    }

    /// 导出任务应写出与主内容等长的 WAV（无插件、无 voice 时尾音立即结束）。
    #[test]
    fn export_job_writes_main_duration() {
        let model = tiny_model();
        let layout = ChannelLayout::from_model(&model);
        let mut engine = AudioEngine::new(48000, layout);
        engine.handle_command(crate::spawn::AudioCommand::LoadModel { model });
        let main_duration = engine.duration_samples();
        assert!(
            main_duration >= 24000,
            "1 拍 @120BPM/48kHz = 24000 采样，实际 {main_duration}"
        );
        engine.handle_command(crate::spawn::AudioCommand::Play { from_sample: 0 });

        let dir = std::env::temp_dir().join("yinhe_export_job_test");
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let path = dir.join("export_job_test.wav");
        let progress = ExportProgress::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let mut job = ExportJob::new(
            &path,
            WavBitDepth::Bit16,
            48000,
            main_duration,
            512,
            Arc::clone(&progress),
            cancel,
            pause,
        )
        .expect("创建导出任务");

        let mut steps = 0u32;
        while job.step(&mut engine).expect("导出渲染") {
            steps += 1;
            assert!(steps < 100_000, "导出未收敛（死循环）");
        }
        job.finalize().expect("写盘收尾");

        let reader = hound::WavReader::open(&path).expect("读回 WAV");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 48000);
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.bits_per_sample, 16);
        let frames = reader.len() as u64 / spec.channels as u64;
        // finalize 会补齐前瞻限幅器的延迟线残留（约 3ms @48k）：WAV 总帧数
        // = 主内容 + 前瞻帧数（实时输出同样有该延迟，不丢音频）。
        let latency = VolumeLimiter::new(48_000).latency_frames() as u64;
        assert_eq!(
            frames,
            main_duration + latency,
            "帧数应等于主内容长度 + 限幅器前瞻"
        );

        if let Ok(p) = progress.lock() {
            let expect = main_duration as f64 / 48000.0;
            assert!((p.total_duration_secs - expect).abs() < 1e-9);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// 取消后不再推进（step 上层会看到 cancelled 标志）。
    #[test]
    fn export_job_reports_cancel_flag() {
        let model = tiny_model();
        let layout = ChannelLayout::from_model(&model);
        let mut engine = AudioEngine::new(48000, layout);
        engine.handle_command(crate::spawn::AudioCommand::LoadModel { model });
        let main_duration = engine.duration_samples();
        let progress = ExportProgress::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(true));
        let job = ExportJob::new(
            &std::env::temp_dir().join("yinhe_export_job_cancel.wav"),
            WavBitDepth::Bit16,
            48000,
            main_duration,
            512,
            progress,
            Arc::clone(&cancel),
            pause,
        )
        .expect("创建导出任务");
        assert!(!job.cancelled());
        assert!(job.paused());
        cancel.store(true, Ordering::Relaxed);
        assert!(job.cancelled());
    }
}
