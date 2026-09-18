use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use yinhe_dsp::dsp::limiter::VolumeLimiter;
use yinhe_types::Interpolation;
#[cfg(feature = "gpu")]
use yinhe_types::SynthEngine;

use crate::export::ExportJob;

use crate::audio_ring::AudioRingProducer;
use crate::engine::AudioEngine;
use crate::preview_engine::PreviewEngine;
use crate::spawn::{AmMsMap, AudioCommand, WorkerCmd, WorkerResult};

mod commands;
mod export;
mod worker;

const STEREO_CHANNELS: usize = 2;
/// 暂停淡出/恢复淡入时长（帧）：10ms。直接停渲染会让 ring 里的预渲染音频
/// 播完后与静音形成阶跃（"滋"）；恢复/seek 后从静音跳到任意相位同样有阶跃。
fn pause_fade_frames(sample_rate: u32) -> usize {
    (sample_rate as usize / 100).max(1)
}
const RENDER_CHUNK_FRAMES: usize = 512;
/// GPU 合成器模式的渲染块（帧）：GPU 每块有一次 submit + voice 状态读回
/// 的 CPU↔GPU 往返，块越小往返越频繁、音符多时抖动越明显。
/// 4096 帧把往返次数降低 8 倍（代价：输出批延迟增大，GPU 模式可接受）。
#[cfg(feature = "gpu")]
const GPU_RENDER_CHUNK_FRAMES: usize = crate::engine::MAX_ENGINE_BLOCK_FRAMES;

// 引擎任何路径的块长都不得超过 MAX_ENGINE_BLOCK_FRAMES（插件激活值依据）。
const _: () = assert!(RENDER_CHUNK_FRAMES <= crate::engine::MAX_ENGINE_BLOCK_FRAMES);
#[cfg(feature = "gpu")]
const _: () = assert!(GPU_RENDER_CHUNK_FRAMES <= crate::engine::MAX_ENGINE_BLOCK_FRAMES);
const TARGET_BUFFER_FRAMES: usize = 4096;
/// 预览激活时的 ring 目标（帧数）：降低输出延迟（≈10ms @48k）。
/// 安卓：MIUI 等 ROM 对后台线程调度抖动大（线程 sleep 实际延迟可达 10ms+），
/// 10ms 缓冲极易欠载 → 声音卡顿。放大到 4096 帧（≈85ms）换取稳定性，
/// 预览延迟增加但不再断音。桌面保持 10ms 低延迟。
#[cfg(target_os = "android")]
const PREVIEW_TARGET_FRAMES: usize = 4096;
#[cfg(not(target_os = "android"))]
const PREVIEW_TARGET_FRAMES: usize = 512;
const WAKE_SLEEP: Duration = Duration::from_millis(1);

/// 合并一批传输命令：只把**连续**的 `Seek` 折叠成最后一个（绝对位置，
/// 中间值没有渲染意义）；其余命令保序。Play/Pause/Stop 不与 Seek 跨类合并。
fn merge_transport_batch(batch: Vec<AudioCommand>) -> Vec<AudioCommand> {
    let mut merged: Vec<AudioCommand> = Vec::with_capacity(batch.len());
    for cmd in batch {
        match (merged.last_mut(), cmd) {
            (Some(AudioCommand::Seek { sample: prev }), AudioCommand::Seek { sample }) => {
                *prev = sample;
            }
            (_, cmd) => merged.push(cmd),
        }
    }
    merged
}

/// 播放启动诊断日志：stderr + `/tmp/yinhe-play.log`（GUI 双击启动时 stderr
/// 不可见）。诊断用，定位后删除。
pub(crate) fn play_log(msg: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let line = format!("[{ts:.3}] {msg}");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/yinhe-play.log")
    {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
    eprintln!("{line}");
}

pub(crate) struct RendererSharedState {
    pub(crate) producer_sample_position: Arc<AtomicU64>,
    pub(crate) playing: Arc<AtomicBool>,
    pub(crate) duration_samples: Arc<AtomicU64>,
    pub(crate) initialized: Arc<AtomicBool>,
    /// 每次 seek/reload 等需要让 cpal 回调清 ring 的操作都会 `fetch_add(1)`。
    /// cpal 回调入口对比自己记录的 acknowledged_generation，不一致就丢弃
    /// `clear_ring_write` 之前的旧音频并重定位消费位置。
    /// 生产者**不再等 ack** —— cpal 回调停了的话，等 ack 会永久卡死 renderer（P0-3）。
    pub(crate) reset_generation: Arc<AtomicU64>,
    /// 清空瞬间的"新音频起点"采样位置：ring 中 `clear_ring_write` 之后推入的
    /// 音频从该位置开始。cpal 回调 ack 时把消费位置对准这里。
    pub(crate) clear_base_sample: Arc<AtomicU64>,
    /// 清空瞬间 ring 的写入计数。cpal 回调 ack 时丢弃该值之前的全部内容
    /// （旧音频），保留之后推入的新音频 —— 比整体 clear 更竞态安全。
    pub(crate) clear_ring_write: Arc<AtomicUsize>,
    /// 已加载完成的通道音色库数（每通道一条 `LoadedSoundFont` 结果 +1）。
    /// UI 据此驱动"加载音色库"stage 的真实进度（完成计数，不预填）。
    pub(crate) sf_loaded: Arc<AtomicUsize>,
    /// 音频完全就绪：LoadModel 相关的全部初始化完成（GPU 路径含 GpuSynth
    /// 初始化 + 采样上传 + 管线预热）。UI 用它 gate"加载完成"提示，
    /// 保证用户看到加载完成时点播放能立即响应。
    pub(crate) audio_ready: Arc<AtomicBool>,
    /// 总线电平表读数端（增删总线时由渲染线程刷新；UI 读锁取用）。
    pub(crate) bus_readings: Arc<Mutex<Vec<yinhe_mixer::MeterReading>>>,
    /// 欠载累计样本数（立体声交错）：播放中 cpal 回调从 ring 取不到足够样本
    /// 而补零的总量。非零增长 = 渲染跟不上（音频断续/咔哒的客观指标）。
    pub(crate) underrun_samples: Arc<AtomicU64>,
}

impl RendererSharedState {
    pub(crate) fn new() -> Self {
        Self {
            producer_sample_position: Arc::new(AtomicU64::new(0)),
            playing: Arc::new(AtomicBool::new(false)),
            duration_samples: Arc::new(AtomicU64::new(0)),
            initialized: Arc::new(AtomicBool::new(false)),
            reset_generation: Arc::new(AtomicU64::new(0)),
            clear_base_sample: Arc::new(AtomicU64::new(0)),
            clear_ring_write: Arc::new(AtomicUsize::new(0)),
            sf_loaded: Arc::new(AtomicUsize::new(0)),
            audio_ready: Arc::new(AtomicBool::new(false)),
            bus_readings: Arc::new(Mutex::new(Vec::new())),
            underrun_samples: Arc::new(AtomicU64::new(0)),
        }
    }
}

struct AudioRenderer {
    engine: AudioEngine,
    ring: AudioRingProducer,
    state: RendererSharedState,
    limiter: VolumeLimiter,
    cmd_rx: Receiver<AudioCommand>,
    /// 传输命令通道（无界）：Play/Resume/Pause/Stop/Seek 优先、保序处理。
    transport_rx: Receiver<AudioCommand>,
    worker_tx: Sender<WorkerCmd>,
    prepared_rx: Receiver<WorkerResult>,
    shutdown: Arc<AtomicBool>,
    scratch: Vec<f32>,
    /// 恢复/seek 后剩余淡入帧数（见 `pause_fade_frames`）。
    fade_in_frames: usize,
    /// 预览合成器（独立 ChannelGroup/音色/状态）：预览音不占主引擎 voice。
    preview_engine: PreviewEngine,
    /// 预览叠加用临时缓冲。
    preview_scratch: Vec<f32>,
    /// 预览 Stop 快速路径标志（与 AudioHandle 共享）：每轮消费，通道满丢命令也必达。
    preview_stop_flag: Arc<AtomicBool>,
    /// cpal 回调已消费的采样位置（听音位置，由回调线程更新）。
    /// 非显式 reload / 掩码切换 / 音色加载完成时，seek 与 ring 清空都锚定它，
    /// 保证播放中的编辑/切换**不移动**听音位置（只有 Play/Seek/Stop 才动）。
    consumer_position: Arc<AtomicU64>,
    /// latest-wins 槽：轨道 mute/solo 掩码（UI 写，每轮消费最新值——
    /// 命令通道满合并时 `SkipTracks` 命令可能被丢，本槽保证必达）。
    pending_skip: Arc<Mutex<Option<Vec<bool>>>>,
    /// latest-wins 槽：AM lane M/S 试听旁通集（见 `pending_skip`）。
    pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>>,
    /// 预览激活时的 ring 目标（帧数）：≥ cpal 回调帧数，避免回调欠载静音卡顿。
    preview_target_frames: usize,
    /// 被替换/移除的 insert 处理器退回 UI 线程（渲染线程不做 deactivate）。
    insert_return_tx: Sender<Vec<Box<dyn yinhe_mixer::InsertProcessor>>>,
    /// 被替换/移除的乐器处理器退回 UI 线程（渲染线程不做 deactivate）。
    instrument_return_tx: Sender<(u8, Box<dyn yinhe_mixer::InstrumentProcessor>)>,
    /// 合成后端（spawn 时已收敛：不可用后端回退 XSynthCpu）。
    /// `YinheGpu` 时加载音色库会初始化 GpuSynth，渲染走 engine.gpu_synth。
    #[cfg(feature = "gpu")]
    synth_engine: SynthEngine,
    /// 采样插值方式（初始化 CpuSynth/GpuSynth 时写入）。
    interpolation: Interpolation,
    /// 播放启动诊断：Play 时刻与目标位置（首块渲染完成后打印一次耗时）。
    play_timing: Option<(Instant, u64)>,
    /// 导出任务（Some = 导出模式：不推 ring、不发布播放状态，连续离线渲染写 WAV）。
    export: Option<ExportJob>,
    /// 导出结束后要恢复的 xsynth 层数（导出设置不污染用户设置）。
    export_prev_layer_count: Option<Option<usize>>,
    /// GPU 模式待加载音色库的通道数（全部完成后统一上传样本）。
    gpu_sf_pending: usize,
}

impl AudioRenderer {
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    fn new(
        engine: AudioEngine,
        preview_engine: PreviewEngine,
        ring: AudioRingProducer,
        state: RendererSharedState,
        cmd_rx: Receiver<AudioCommand>,
        transport_rx: Receiver<AudioCommand>,
        worker_tx: Sender<WorkerCmd>,
        prepared_rx: Receiver<WorkerResult>,
        shutdown: Arc<AtomicBool>,
        preview_stop_flag: Arc<AtomicBool>,
        consumer_position: Arc<AtomicU64>,
        pending_skip: Arc<Mutex<Option<Vec<bool>>>>,
        pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>>,
        // cpal 回调每次请求的帧数（预览时 ring 目标下限，避免回调欠载静音）。
        callback_frames: usize,
        insert_return_tx: Sender<Vec<Box<dyn yinhe_mixer::InsertProcessor>>>,
        instrument_return_tx: Sender<(u8, Box<dyn yinhe_mixer::InstrumentProcessor>)>,
        #[cfg(feature = "gpu")] synth_engine: SynthEngine,
        interpolation: Interpolation,
    ) -> Self {
        // GPU 模式用更大的渲染块（见 GPU_RENDER_CHUNK_FRAMES）。
        #[cfg(feature = "gpu")]
        let render_chunk_frames = if synth_engine == SynthEngine::YinheGpu {
            GPU_RENDER_CHUNK_FRAMES
        } else {
            RENDER_CHUNK_FRAMES
        };
        #[cfg(not(feature = "gpu"))]
        let render_chunk_frames = RENDER_CHUNK_FRAMES;
        let sample_rate = engine.sample_rate;
        Self {
            engine,
            ring,
            state,
            limiter: VolumeLimiter::new(sample_rate),
            transport_rx,
            export: None,
            export_prev_layer_count: None,
            gpu_sf_pending: 0,
            cmd_rx,
            worker_tx,
            prepared_rx,
            shutdown,
            scratch: vec![0.0; render_chunk_frames * STEREO_CHANNELS],
            fade_in_frames: 0,
            preview_engine,
            preview_scratch: vec![0.0; render_chunk_frames * STEREO_CHANNELS],
            preview_stop_flag,
            consumer_position,
            pending_skip,
            pending_am_ms,
            preview_target_frames: PREVIEW_TARGET_FRAMES.max(callback_frames),
            insert_return_tx,
            instrument_return_tx,
            #[cfg(feature = "gpu")]
            synth_engine,
            interpolation,
            play_timing: None,
        }
    }

    /// 合成后端是否为 GPU 引擎（spawn 时已把不可用后端收敛掉，只可能是
    /// `XSynthCpu`、`YinheCpu` 或 `YinheGpu`）。块长/事件表同步等 GPU 专用路径用它。
    #[cfg(feature = "gpu")]
    #[inline]
    fn gpu_engine(&self) -> bool {
        self.synth_engine == SynthEngine::YinheGpu
    }

    /// 合成后端是否为 yinhe CPU 引擎（key map 预解析等 yinhe 后端共用路径）。
    #[cfg(feature = "gpu")]
    #[inline]
    fn yinhe_cpu_engine(&self) -> bool {
        self.synth_engine == SynthEngine::YinheCpu
    }

    /// 标记音频就绪（幂等）：UI 的"加载完成"提示以此为准。
    fn mark_audio_ready(&self) {
        if !self.state.audio_ready.swap(true, Ordering::AcqRel) {
            play_log("[play] 音频就绪（模型+音色库+采样上传+管线预热完成）");
        }
    }

    /// 把引擎攒下的退回 insert 处理器送回 UI 线程。
    fn flush_insert_returns(&mut self) {
        let returns = self.engine.drain_insert_returns();
        if !returns.is_empty() {
            let _ = self.insert_return_tx.send(returns);
        }
    }

    /// 把引擎攒下的退回乐器处理器送回 UI 线程。
    fn flush_instrument_returns(&mut self) {
        let returns = self.engine.drain_instrument_returns();
        for p in returns {
            let _ = self.instrument_return_tx.send(p);
        }
    }

    fn run(&mut self) {
        let mut last_underrun_report = std::time::Instant::now();
        let mut last_underrun_total = 0u64;
        while !self.shutdown.load(Ordering::Relaxed) {
            let mut did_work = self.process_commands() | self.process_worker_results();
            self.flush_insert_returns();
            self.flush_instrument_returns();
            // 预览 Stop 快速路径：命令通道满（渲染忙时 PreviewStop 可能被丢弃）也保证
            // 松手即停。必须在 process_commands 之后消费：处理命令期间 flag 保持置位，
            // PreviewNotes 分支借此跳过堆积的旧预览组（松手后不再触发）。
            if self.preview_stop_flag.swap(false, Ordering::AcqRel) {
                self.preview_engine.stop_all();
                did_work = true;
            }
            // GPU 后端统一同步：命令/worker 结果可能改变位置或使事件表失效，
            // 渲染（或导出）前消费一次 dirty 标志。CPU 模式下是 no-op。
            #[cfg(feature = "gpu")]
            self.engine.sync_gpu_backend();
            if self.export.is_some() {
                // 导出模式：不推 ring、不发布播放状态（UI 保持停止外观），
                // 连续离线渲染；每轮仍处理命令（取消/参数/编辑）。
                self.step_export();
                if !did_work {
                    thread::sleep(WAKE_SLEEP);
                }
                continue;
            }

            did_work |= self.render_if_needed();

            // 欠载诊断：每秒报告 ring 补零增量（>0 = 渲染跟不上实时）。
            if last_underrun_report.elapsed() >= Duration::from_secs(1) {
                let total = self.state.underrun_samples.load(Ordering::Relaxed);
                if total > last_underrun_total {
                    let delta = total - last_underrun_total;
                    let ms = delta as f64 / 2.0 / self.engine.sample_rate as f64 * 1000.0;
                    eprintln!(
                        "[audio] 欠载：+{delta} 样本（≈{ms:.1}ms，累计 {}）——渲染跟不上实时",
                        total
                    );
                    last_underrun_total = total;
                }
                last_underrun_report = std::time::Instant::now();
            }
            if let Some((t, from)) = self.play_timing
                && self.engine.sample_position() != from
            {
                play_log(&format!(
                    "[play] 首块就绪（指示线可推进）={:?}",
                    t.elapsed()
                ));
                self.play_timing = None;
            }

            self.publish_state();

            if !did_work {
                thread::sleep(WAKE_SLEEP);
            }
        }
    }

    /// 暂停前渲染一段 10ms 淡出推入 ring：pause 立即停渲染会让 ring 里
    /// 预渲染的音频（最多 ~85ms）播完后与静音形成阶跃（"滋"）。
    fn render_pause_fade_out(&mut self) {
        if !self.engine.playing() || !self.state.initialized.load(Ordering::Acquire) {
            return;
        }
        let frames = pause_fade_frames(self.engine.sample_rate);
        let mut buf = vec![0.0f32; frames * STEREO_CHANNELS];
        self.engine.render(&mut buf);
        self.limiter.limit(&mut buf);
        let total = frames as f32;
        for f in 0..frames {
            let g = 1.0 - f as f32 / total;
            let o = f * STEREO_CHANNELS;
            buf[o] *= g;
            buf[o + 1] *= g;
        }
        let _ = self.ring.push_slice(&buf);
    }

    fn render_if_needed(&mut self) -> bool {
        // 预览组非空或有余音时强制渲染：未播放时也要输出。
        // 预览引擎是独立合成器（不依赖模型），所以预览时不需要 initialized。
        let previewing = self.preview_engine.previewing();
        // 乐器插件空闲渲染：存在已安装乐器时停止状态也持续 process
        //（GUI 键盘、插件预览、插件尾音 —— 成熟 DAW 语义：乐器始终在跑）。
        let idle_instruments = !self.engine.playing() && self.engine.has_instruments();
        if !self.engine.playing() && !previewing && !idle_instruments {
            // 暂停时把待发的插件参数送达（否则调参数要等播放才生效）。
            // 不依赖 initialized：空工程挂乐器插件也要能立即调参数。
            self.engine.flush_pending_plugin_params();
            return false;
        }
        if !self.state.initialized.load(Ordering::Acquire) && !previewing && !idle_instruments {
            return false;
        }

        // 预览/空闲乐器监听用更小的 ring 目标（512 帧 ≈ 10ms）：都是交互操作，
        // NoteOn 后要等 ring 里已有音频播完才出声，目标 4096 帧会带来约 85ms
        // 延迟，快速拖动/弹键盘时每个音都滞后、听感响应很慢。
        let target_samples = if previewing || idle_instruments {
            self.preview_target_frames * STEREO_CHANNELS
        } else {
            TARGET_BUFFER_FRAMES * STEREO_CHANNELS
        };
        if self.ring.len() >= target_samples {
            return false;
        }

        let free = self.ring.free_space();
        if free < self.scratch.len() {
            return false;
        }

        if self.engine.playing() {
            let t_render = Instant::now();
            self.engine.render(&mut self.scratch);
            if self.play_timing.is_some() {
                play_log(&format!(
                    "[play] engine.render 块耗时={:?}",
                    t_render.elapsed()
                ));
            }
        } else if idle_instruments {
            // 未播放但有乐器：只驱动乐器插件与混音输出（不推进走带）。
            self.engine.render_idle(&mut self.scratch);
        } else {
            // 未播放：主引擎不渲染，输出静音，预览音单独叠加。
            self.scratch.fill(0.0);
        }
        if previewing {
            // 预览合成器独立输出，叠加到主输出；余音在 voice 自然衰减完前持续输出。
            self.preview_engine.render(&mut self.preview_scratch);
            for (a, b) in self.scratch.iter_mut().zip(self.preview_scratch.iter()) {
                *a += *b;
            }
        }

        // 输出限幅（GPU/CPU 路径统一在此处理；合成器内部不做 DSP）。
        self.limiter.limit(&mut self.scratch);

        // 恢复/seek 后的淡入（10ms）：从静音或新相位直接开始有阶跃（"滋"）。
        if self.fade_in_frames > 0 {
            let frames = self.scratch.len() / STEREO_CHANNELS;
            let n = self.fade_in_frames.min(frames);
            let total = pause_fade_frames(self.engine.sample_rate) as f32;
            for f in 0..n {
                let g = 1.0 - (self.fade_in_frames - f) as f32 / total;
                let o = f * STEREO_CHANNELS;
                self.scratch[o] *= g;
                self.scratch[o + 1] *= g;
            }
            self.fade_in_frames -= n;
        }

        let pushed = self.ring.push_slice(&self.scratch);
        debug_assert_eq!(pushed, self.scratch.len());
        true
    }

    /// 清空 ring 缓冲并记录清空边界。
    ///
    /// `base` = 清空后听音位置应锚定的采样位置：
    /// - 显式 seek（Play/Seek/Stop）：引擎刚 seek 过，用 `engine.sample_position()`；
    /// - 非显式操作（reload / 掩码切换 / 音色加载）：用 `consumer_position`（听音位置），
    ///   保证播放中的编辑/切换**不移动**听音位置。
    fn clear_buffered_audio(&mut self, base: u64) {
        // 不直接调 `self.ring.clear()`：它和 cpal 回调的 `pop_into` 并发时会
        // 把 cpal 刚推进的 read 指针覆盖回 write，下次回调会把旧数据当新数据读出 → 杂音。
        // 改为记录"清空边界"并 bump `reset_generation`，由 cpal 回调入口（单线程，
        // 与 pop_into 天然串行）用 `discard_before` 只丢弃边界前的旧音频。
        // 边界之后可能已推入新音频（模型已加载时渲染很快，ack 常晚于新音频入队），
        // 整体 clear 会把新播放位置的开头一起丢掉 —— 第二次播放开头缺失的根因。
        self.state
            .producer_sample_position
            .store(base, Ordering::Release);
        self.state.clear_base_sample.store(base, Ordering::Release);
        self.state
            .clear_ring_write
            .store(self.ring.write_position(), Ordering::Release);
        self.state.reset_generation.fetch_add(1, Ordering::AcqRel);
    }

    // ── 导出模式（渲染线程内，复用实时引擎与插件实例） ──

    /// 刷新总线电平表读数槽（增删总线后调用；UI 侧共享同一 Arc）。
    fn sync_bus_meter_readings(&self) {
        let readings: Vec<yinhe_mixer::MeterReading> = (0..self.engine.mixer.bus_count())
            .filter_map(|i| self.engine.mixer.bus_meter_reading(i))
            .collect();
        if let Ok(mut slot) = self.state.bus_readings.lock() {
            *slot = readings;
        }
    }

    fn publish_state(&self) {
        self.state
            .producer_sample_position
            .store(self.engine.sample_position(), Ordering::Release);
        self.state
            .playing
            .store(self.engine.playing(), Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn spawn_renderer(
    engine: AudioEngine,
    preview_engine: PreviewEngine,
    ring: AudioRingProducer,
    state: RendererSharedState,
    cmd_rx: Receiver<AudioCommand>,
    transport_rx: Receiver<AudioCommand>,
    worker_tx: Sender<WorkerCmd>,
    prepared_rx: Receiver<WorkerResult>,
    shutdown: Arc<AtomicBool>,
    preview_stop_flag: Arc<AtomicBool>,
    consumer_position: Arc<AtomicU64>,
    pending_skip: Arc<Mutex<Option<Vec<bool>>>>,
    pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>>,
    // cpal 回调每次请求的帧数（预览时 ring 目标下限）。
    callback_frames: usize,
    insert_return_tx: Sender<Vec<Box<dyn yinhe_mixer::InsertProcessor>>>,
    instrument_return_tx: Sender<(u8, Box<dyn yinhe_mixer::InstrumentProcessor>)>,
    #[cfg(feature = "gpu")] synth_engine: SynthEngine,
    interpolation: Interpolation,
) -> Result<JoinHandle<()>, std::io::Error> {
    thread::Builder::new()
        .name("audio-renderer".into())
        .spawn(move || {
            // 长尾衰减防非规格化拖慢（x86；Arm no-op）。
            #[cfg(feature = "gpu")]
            yinhe_synth::denormals::enable_flush_denormals();
            let mut renderer = AudioRenderer::new(
                engine,
                preview_engine,
                ring,
                state,
                cmd_rx,
                transport_rx,
                worker_tx,
                prepared_rx,
                shutdown,
                preview_stop_flag,
                consumer_position,
                pending_skip,
                pending_am_ms,
                callback_frames,
                insert_return_tx.clone(),
                instrument_return_tx.clone(),
                #[cfg(feature = "gpu")]
                synth_engine,
                interpolation,
            );
            renderer.run();
            // 导出中引擎被拆除（切文档/关工程）：把导出标记为中断，
            // 否则 UI 的进度卡永远停留在“导出中”。
            if let Some(job) = renderer.export.take()
                && let Ok(mut p) = job.progress_handle().lock()
            {
                p.finished = true;
                p.error = Some("导出被中断（音频引擎已重建）".into());
                p.status = "已中断".into();
            }
            // 引擎拆除：mixer 里的 insert 处理器全部退回 UI 线程回收
            //（CLAP deactivate 必须在管理线程做，不能在渲染线程 drop）。
            let leftovers = renderer.engine.mixer.take_all_inserts();
            if !leftovers.is_empty() {
                let _ = insert_return_tx.send(leftovers);
            }
            // 引擎里仍在位的乐器处理器也退回 UI 线程回收。
            let inst_leftovers: Vec<(u8, Box<dyn yinhe_mixer::InstrumentProcessor>)> = renderer
                .engine
                .instruments
                .iter_mut()
                .filter_map(|slot| slot.take().map(|s| (s.channel, s.processor)))
                .collect();
            for p in inst_leftovers {
                let _ = instrument_return_tx.send(p);
            }
            // 显式 drop AudioRenderer，释放 AudioEngine（含 Arc<YinModel> 和 SoundFont），
            // 然后 purge jemalloc arena 归还内存给 OS。
            drop(renderer);
            yinhe_memtrace::purge_free_pages();
        })
        .map_err(|e| {
            tracing::error!("Failed to spawn audio renderer thread: {e}");
            e
        })
}

/// GPU 模式「待加载音色库」通道计数。
///
/// 计数条件必须与 `LoadedSoundFont` 分支的递减条件（GPU 引擎且 dense
/// 槽位有效）严格一致：多计一个不递减的通道，`gpu_sf_pending` 永远归不了零，
/// `mark_audio_ready` 不触发，启动页卡在"初始化音频"（CPU 模式曾因此无法进入）。
#[cfg(feature = "gpu")]
fn count_gpu_sf_pending(
    configs: &[(u8, Vec<String>)],
    layout: &crate::channel_layout::ChannelLayout,
    gpu_engine: bool,
) -> usize {
    if !gpu_engine {
        return 0;
    }
    // 按分组计：每组只在最后一个有效通道登记完时触发一次样本上传。
    group_sf_configs(configs)
        .iter()
        .filter(|(channels, _)| {
            channels.iter().any(|ch| {
                let dense = layout.dense_for(*ch as usize);
                dense != u32::MAX && (dense as usize) < yinhe_synth::MAX_CHANNELS
            })
        })
        .count()
}

/// 把 `(channel, paths)` 配置按 paths 分组：相同音色库集合的通道合并为一条
/// `LoadSoundFont`（worker 只加载一次，key map Arc 跨通道共享）。
pub(crate) fn group_sf_configs(configs: &[(u8, Vec<String>)]) -> Vec<(Vec<u8>, Vec<String>)> {
    let mut groups: Vec<(Vec<u8>, Vec<String>)> = Vec::new();
    for (channel, paths) in configs {
        if paths.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(_, p)| p == paths) {
            Some((channels, _)) => channels.push(*channel),
            None => groups.push((vec![*channel], paths.clone())),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seek(sample: u64) -> AudioCommand {
        AudioCommand::Seek { sample }
    }

    #[test]
    fn merge_transport_folds_consecutive_seeks() {
        let out = merge_transport_batch(vec![seek(10), seek(20), seek(30)]);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], AudioCommand::Seek { sample: 30 }));
    }

    #[test]
    fn merge_transport_keeps_order_around_non_seek() {
        let out = merge_transport_batch(vec![
            AudioCommand::Play { from_sample: 0 },
            seek(10),
            seek(20),
            AudioCommand::Pause,
            seek(30),
            seek(40),
        ]);
        assert_eq!(out.len(), 4);
        assert!(matches!(out[0], AudioCommand::Play { from_sample: 0 }));
        assert!(matches!(out[1], AudioCommand::Seek { sample: 20 }));
        assert!(matches!(out[2], AudioCommand::Pause));
        assert!(matches!(out[3], AudioCommand::Seek { sample: 40 }));
    }

    #[test]
    fn merge_transport_empty() {
        assert!(merge_transport_batch(Vec::new()).is_empty());
    }

    /// 回归：CPU 模式必须为 0（曾无条件计数，导致 gpu_sf_pending 永不归零、
    /// 音频就绪不触发、启动页卡死）；未激活通道与空配置不计数。
    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_sf_pending_only_counts_gpu_eligible_channels() {
        use crate::channel_layout::ChannelLayout;

        let configs = vec![
            (0u8, vec!["a.sfz".to_string()]),
            (1u8, vec!["b.sfz".to_string()]),
            (2u8, vec!["c.sfz".to_string()]),
            (3u8, Vec::new()),
        ];
        // ch0 激活；ch1 未激活（dense = u32::MAX）；ch2/3 不在 mask 内。
        let layout = ChannelLayout::from_mask(vec![true, false]);

        assert_eq!(count_gpu_sf_pending(&configs, &layout, false), 0);
        assert_eq!(count_gpu_sf_pending(&configs, &layout, true), 1);
    }
}
