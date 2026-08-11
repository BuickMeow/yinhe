use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
#[cfg(feature = "gpu")]
use xsynth_core::channel::ControlEvent;
use xsynth_core::effects::VolumeLimiter;

use crate::audio_ring::AudioRingProducer;
use crate::engine::AudioEngine;
use crate::preview_engine::PreviewEngine;
use crate::spawn::{AudioCommand, WorkerCmd, WorkerResult};

const STEREO_CHANNELS: usize = 2;
const RENDER_CHUNK_FRAMES: usize = 512;
const TARGET_BUFFER_FRAMES: usize = 4096;
/// 预览激活时的 ring 目标（帧数）：降低输出延迟（≈10ms @48k）。
const PREVIEW_TARGET_FRAMES: usize = 512;
const WAKE_SLEEP: Duration = Duration::from_millis(1);

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
    /// 已加载完成的音色库 port 数（每 port 一条 `LoadedSoundFont` 结果 +1）。
    /// UI 据此驱动"加载音色库"stage 的真实进度（完成计数，不预填）。
    pub(crate) sf_loaded: Arc<AtomicUsize>,
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
        }
    }
}

struct AudioRenderer {
    engine: AudioEngine,
    ring: AudioRingProducer,
    state: RendererSharedState,
    limiter: VolumeLimiter,
    cmd_rx: Receiver<AudioCommand>,
    worker_tx: Sender<WorkerCmd>,
    prepared_rx: Receiver<WorkerResult>,
    shutdown: Arc<AtomicBool>,
    scratch: Vec<f32>,
    /// 预览合成器（独立 ChannelGroup/音色/状态）：预览音不占主引擎 voice。
    preview_engine: PreviewEngine,
    /// 预览叠加用临时缓冲。
    preview_scratch: Vec<f32>,
    /// 预览 Stop 快速路径标志（与 AudioHandle 共享）：每轮消费，通道满丢命令也必达。
    preview_stop_flag: Arc<AtomicBool>,
    /// 预览激活时的 ring 目标（帧数）：≥ cpal 回调帧数，避免回调欠载静音卡顿。
    preview_target_frames: usize,
    /// 是否启用 GPU 合成器。启用后加载音色库时初始化 GpuSynth，渲染走 engine.gpu_synth。
    #[cfg(feature = "gpu")]
    use_gpu_synth: bool,
}

impl AudioRenderer {
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    fn new(
        engine: AudioEngine,
        preview_engine: PreviewEngine,
        ring: AudioRingProducer,
        state: RendererSharedState,
        channels: u16,
        cmd_rx: Receiver<AudioCommand>,
        worker_tx: Sender<WorkerCmd>,
        prepared_rx: Receiver<WorkerResult>,
        shutdown: Arc<AtomicBool>,
        preview_stop_flag: Arc<AtomicBool>,
        // cpal 回调每次请求的帧数（预览时 ring 目标下限，避免回调欠载静音）。
        callback_frames: usize,
        #[cfg(feature = "gpu")] use_gpu_synth: bool,
    ) -> Self {
        Self {
            engine,
            ring,
            state,
            limiter: VolumeLimiter::new(channels),
            cmd_rx,
            worker_tx,
            prepared_rx,
            shutdown,
            scratch: vec![0.0; RENDER_CHUNK_FRAMES * STEREO_CHANNELS],
            preview_engine,
            preview_scratch: vec![0.0; RENDER_CHUNK_FRAMES * STEREO_CHANNELS],
            preview_stop_flag,
            preview_target_frames: PREVIEW_TARGET_FRAMES.max(callback_frames),
            #[cfg(feature = "gpu")]
            use_gpu_synth,
        }
    }

    fn run(&mut self) {
        while !self.shutdown.load(Ordering::Relaxed) {
            let mut did_work = self.process_commands() | self.process_worker_results();
            // 预览 Stop 快速路径：命令通道满（渲染忙时 PreviewStop 可能被丢弃）也保证
            // 松手即停。必须在 process_commands 之后消费：处理命令期间 flag 保持置位，
            // PreviewNotes 分支借此跳过堆积的旧预览组（松手后不再触发）。
            if self.preview_stop_flag.swap(false, Ordering::AcqRel) {
                self.preview_engine.stop_all();
                did_work = true;
            }
            did_work |= self.render_if_needed();

            self.publish_state();

            if !did_work {
                thread::sleep(WAKE_SLEEP);
            }
        }
    }

    fn process_commands(&mut self) -> bool {
        let mut did_work = false;
        let mut pending_reload: Option<Arc<yinhe_core::YinModel>> = None;
        let mut pending_update_notes: Option<Arc<yinhe_core::YinModel>> = None;
        let mut pending_density_rebuild: bool = false;

        loop {
            match self.cmd_rx.try_recv() {
                Ok(cmd) => {
                    did_work = true;
                    match cmd {
                        AudioCommand::LoadModel { model } => {
                            self.preview_engine.stop_all();
                            self.engine.handle_command(AudioCommand::Pause);
                            self.engine.handle_command(AudioCommand::Stop);
                            self.clear_buffered_audio();
                            let density = self.engine.automation_density;
                            let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model, density));
                        }
                        AudioCommand::ReloadNotes { model } => {
                            // 全量重建优先于只更新音符 —— 丢弃 pending UpdateNotes
                            pending_update_notes = None;
                            pending_reload = Some(model);
                        }
                        AudioCommand::UpdateNotes { model } => {
                            // 只在没有 pending ReloadNotes 时记录（ReloadNotes 包含 audible_notes）
                            if pending_reload.is_none() {
                                pending_update_notes = Some(model);
                            }
                        }
                        AudioCommand::LoadSoundFont { port, paths } => {
                            let dense_channels = self.engine.dense_channels_for_port(port);
                            if !dense_channels.is_empty() {
                                let _ = self.worker_tx.send(WorkerCmd::LoadSoundFont {
                                    port,
                                    paths,
                                    dense_channels,
                                });
                            }
                        }
                        AudioCommand::Play { from_sample } => {
                            if self.engine.model_loaded() {
                                self.preview_engine.stop_all();
                                self.engine
                                    .handle_command(AudioCommand::Play { from_sample });
                                // GPU 路径：重建事件（含鼓组/复活音符）并同步位置
                                #[cfg(feature = "gpu")]
                                if self.engine.gpu_synth.is_some() {
                                    self.sync_gpu_synth_events();
                                }
                                self.clear_buffered_audio();
                                // 方案 B：seek 后异步 chase（current_tick 已由 seek 更新）
                                self.request_chase(self.engine.current_tick());
                            } else {
                                self.engine.set_pending_play(from_sample);
                            }
                        }
                        AudioCommand::Seek { sample } => {
                            self.preview_engine.stop_all();
                            self.engine.handle_command(AudioCommand::Seek { sample });
                            #[cfg(feature = "gpu")]
                            if self.engine.gpu_synth.is_some() {
                                self.sync_gpu_synth_events();
                            }
                            self.clear_buffered_audio();
                            // 方案 B：seek 后异步 chase（current_tick 已由 seek 更新）
                            self.request_chase(self.engine.current_tick());
                        }
                        AudioCommand::Stop => {
                            self.preview_engine.stop_all();
                            self.engine.handle_command(AudioCommand::Stop);
                            #[cfg(feature = "gpu")]
                            if self.engine.gpu_synth.is_some() {
                                self.sync_gpu_synth_events();
                            }
                            self.clear_buffered_audio();
                            // 方案 B：Stop 也 seek 到 0，需要 chase 恢复初始 channel state
                            self.request_chase(0);
                        }
                        AudioCommand::SetAutomationDensity { density } => {
                            self.engine.automation_density = density.max(1);
                            // 若已加载模型，触发 worker 重建 cc_events
                            if self.engine.yin_model.is_some() {
                                pending_density_rebuild = true;
                            }
                        }
                        AudioCommand::SkipTracks { skip } => {
                            self.engine.skip_track = skip;
                            // mute/solo 状态变了：旧 skip mask 的异步 chase 结果必须作废
                            //（递增 generation），否则快速连续切换时旧结果可能晚到并
                            // 覆盖新状态——GPU 路径的通道状态依赖 chase 恢复，影响更大。
                            self.engine.chase_generation =
                                self.engine.chase_generation.wrapping_add(1);
                            // GPU 路径：事件列表按新 skip mask 重建（mute/solo 即时生效，
                            // 不再需要重启引擎）；重建会 seek 到当前位置并清掉旧 voice。
                            #[cfg(feature = "gpu")]
                            if self.engine.gpu_synth.is_some() && self.engine.model_loaded() {
                                self.sync_gpu_synth_events();
                                self.clear_buffered_audio();
                            }
                            // mute 状态变了，chase 结果需要更新：
                            // unmute 的轨道的 CC 需要恢复，mute 的轨道的 CC 不再参与。
                            if self.engine.model_loaded() {
                                self.request_chase(self.engine.current_tick());
                            }
                        }
                        AudioCommand::PreviewNotes { notes } => {
                            // 用户已松手（Stop 请求尚未消费）：跳过堆积的旧预览组，
                            // 否则松手后还会触发一组在响。
                            if self.preview_stop_flag.load(Ordering::Acquire) {
                                continue;
                            }
                            // 按 channel 分组、组内按 target_tick 升序，增量 chase：
                            // 每个通道只扫一遍 cc_events，避免整组预览反复全量扫描。
                            let cc_events = self.engine.cc_events.clone();
                            let mut groups: Vec<(u32, Vec<&crate::spawn::PreviewNoteParams>)> =
                                Vec::new();
                            for n in &notes {
                                let ch = n.channel as u32;
                                if let Some((_, g)) = groups.iter_mut().find(|(c, _)| *c == ch) {
                                    g.push(n);
                                } else {
                                    groups.push((ch, vec![n]));
                                }
                            }
                            for (_, g) in &mut groups {
                                g.sort_by_key(|n| n.target_tick);
                            }
                            let mut inputs: Vec<crate::preview_engine::PreviewNoteIn> =
                                Vec::with_capacity(notes.len());
                            for (ch, g) in groups {
                                let targets: Vec<u32> = g.iter().map(|n| n.target_tick).collect();
                                let states = crate::preview_engine::chase_channel_states(
                                    &cc_events, ch, &targets,
                                );
                                for (n, state) in g.iter().zip(states.iter()) {
                                    // 预览引擎内部时钟是渲染帧（sample 域）：tick 只用于
                                    // chase 目标比较，这里把相对时值差/时长转回 sample。
                                    let target = self.engine.tick_to_sample(n.target_tick);
                                    let duration = if n.duration_ticks > 0 {
                                        let end = self.engine.tick_to_sample(
                                            n.target_tick.saturating_add(n.duration_ticks),
                                        );
                                        Some(end.saturating_sub(target))
                                    } else {
                                        None
                                    };
                                    inputs.push(crate::preview_engine::PreviewNoteIn {
                                        channel: n.channel,
                                        key: n.key,
                                        velocity: n.velocity,
                                        duration,
                                        state: *state,
                                        target_sample: target,
                                    });
                                }
                            }
                            // 提交预览组：组内按目标位置相对时值错开触发。
                            self.preview_engine.preview_notes(inputs);
                        }
                        AudioCommand::PreviewStop => {
                            self.preview_engine.stop_all();
                        }
                        other => self.engine.handle_command(other),
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return did_work,
            }
        }

        if let Some(model) = pending_reload {
            self.engine.send_all_notes_off();
            self.engine.clear_active_notes();
            self.clear_buffered_audio();
            let density = self.engine.automation_density;
            let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model, density));
            did_work = true;
        } else if let Some(model) = pending_update_notes {
            // 只更新音符，不重建 cc_events，不 chase
            let _ = self.worker_tx.send(WorkerCmd::PrepareNotes(model));
            did_work = true;
        } else if pending_density_rebuild {
            // density 改变后用当前模型重建 cc_events
            if let Some(model) = self.engine.yin_model.clone() {
                let density = self.engine.automation_density;
                let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model, density));
                did_work = true;
            }
        }

        did_work
    }

    /// 方案 B：发 `PrepareChase` 给 worker 线程异步计算 256 通道状态快照。
    /// worker 完成后回传 `ChaseResult`，`process_worker_results` 应用。
    /// `chase_generation` 用于丢弃过期结果（模型被 PrepareModel 替换后）。
    fn request_chase(&self, target_tick: u32) {
        let Some(model) = self.engine.yin_model.clone() else {
            return;
        };
        let generation = self.engine.chase_generation;
        let skip_mask = self.engine.skip_track.clone();
        let _ = self.worker_tx.send(WorkerCmd::PrepareChase {
            model,
            target_tick,
            generation,
            skip_mask,
        });
    }

    /// GPU 路径：把 worker 算好的 chase 快照应用到 GpuSynth 的通道状态。
    /// skip 掩码按 dense 通道翻译（GpuSynth 内部以 dense % 32 索引）。
    #[cfg(feature = "gpu")]
    fn apply_chase_to_gpu(
        layout: &crate::channel_layout::ChannelLayout,
        synth: &mut yinhe_synth::GpuSynth,
        states: &[crate::channel::ChannelState; 256],
    ) {
        let synth_skip = synth.chase_skip();
        let mut skip = crate::channel::ChaseSkip::default();
        for ch in 0..256usize {
            let dense = layout.dense_for(ch);
            if dense == u32::MAX {
                continue;
            }
            let idx = dense as usize % yinhe_synth::MAX_CHANNELS;
            skip.cc_mask[ch] = synth_skip.cc_mask[idx];
            skip.pitch_bend[ch] = synth_skip.pitch_bend[idx];
            skip.pbs[ch] = synth_skip.pbs[idx];
            skip.fine_tune[ch] = synth_skip.fine_tune[idx];
            skip.coarse_tune[ch] = synth_skip.coarse_tune[idx];
            skip.program[ch] = synth_skip.program[idx];
        }
        for ch in 0..256u32 {
            let dense = layout.dense_for(ch as usize);
            if dense == u32::MAX {
                continue;
            }
            let events: Vec<yinhe_synth::ControlEvent> = states[ch as usize]
                .events_to_send(ch as usize, &skip)
                .iter()
                .filter_map(to_gpu_control_event)
                .collect();
            if !events.is_empty() {
                synth.apply_chase(dense, &events);
            }
        }
    }

    fn process_worker_results(&mut self) -> bool {
        let mut did_work = false;
        loop {
            match self.prepared_rx.try_recv() {
                Ok(WorkerResult::PreparedModel(prepared)) => {
                    self.state
                        .duration_samples
                        .store(prepared.duration_samples, Ordering::Relaxed);
                    self.engine.apply_prepared_model(prepared);
                    // GPU 路径：模型应用后同步事件到 GpuSynth
                    #[cfg(feature = "gpu")]
                    self.sync_gpu_synth_events();
                    self.clear_buffered_audio();
                    self.state.initialized.store(true, Ordering::Release);
                    // 方案 B：apply_prepared_model 内部 seek_to 不再 chase，
                    // 这里发 PrepareChase 让 worker 异步算 channel state
                    self.request_chase(self.engine.current_tick());
                    did_work = true;
                }
                Ok(WorkerResult::PreparedNotes {
                    model,
                    yin_model,
                    audible_delta,
                    duration_samples,
                }) => {
                    self.state
                        .duration_samples
                        .store(duration_samples, Ordering::Relaxed);
                    self.engine
                        .apply_notes_only(model, yin_model, audible_delta, duration_samples);
                    // GPU 路径：音符变化后同步事件到 GpuSynth
                    #[cfg(feature = "gpu")]
                    self.sync_gpu_synth_events();
                    // 注意：这里**不**清 ring。UpdateNotes 不 seek、不改 cc_events，
                    // 已渲染的 ring 内容是"过去时"音频（新音符只影响未来 dispatch），
                    // 清空会把正在播放的预览余音/当前音频丢掉 → 松手停顿。
                    // 只有 PreparedModel（cc_events 重建/seek）才需要清。
                    self.state.initialized.store(true, Ordering::Release);
                    did_work = true;
                }
                Ok(WorkerResult::ChaseResult { states, generation }) => {
                    // 丢弃过期结果：cc_events 已被新 PrepareModel 替换
                    if generation == self.engine.chase_generation {
                        self.engine.apply_chase_result(&states);
                        // GPU 路径：把 chase 快照应用到 GpuSynth（跳过 seek 后
                        // 已实时处理过的控制器，避免旧值覆盖新值）。
                        #[cfg(feature = "gpu")]
                        if let Some(synth) = self.engine.gpu_synth.as_mut() {
                            Self::apply_chase_to_gpu(&self.engine.channel_layout, synth, &states);
                        }
                        did_work = true;
                    }
                }
                Ok(WorkerResult::LoadedSoundFont {
                    port,
                    soundfonts,
                    dense_channels,
                    paths,
                }) => {
                    // 音色库完成计数：UI 的"加载音色库"stage 进度 = 已完成 port 数。
                    self.state.sf_loaded.fetch_add(1, Ordering::Relaxed);
                    // 预览引擎与主引擎共享同一音色（Arc，零拷贝）。
                    self.preview_engine
                        .set_port_soundfonts(port, soundfonts.clone());
                    self.engine
                        .apply_loaded_soundfont_for_port(port, soundfonts, &dense_channels);
                    // GPU 路径：首次加载音色库时初始化 GpuSynth，后续 port 逐个加载
                    #[cfg(feature = "gpu")]
                    if self.use_gpu_synth {
                        let sr = self.engine.sample_rate;
                        let paths: Vec<std::path::PathBuf> =
                            paths.iter().map(std::path::PathBuf::from).collect();
                        if self.engine.gpu_synth.is_none() {
                            match yinhe_synth::GpuSynth::new_default(sr) {
                                Ok(mut synth) => {
                                    if let Err(e) =
                                        synth.load_port_soundfonts(port, &dense_channels, &paths)
                                    {
                                        eprintln!("[gpu] Failed to load soundfonts: {}", e);
                                    }
                                    // 加载当前模型的事件
                                    let events =
                                        self.build_gpu_synth_events(self.engine.sample_position());
                                    synth.load_events(events);
                                    synth.seek(self.engine.sample_position());
                                    self.engine.gpu_synth = Some(synth);
                                    eprintln!("[gpu] GpuSynth initialized (port {})", port);
                                }
                                Err(e) => {
                                    eprintln!("[gpu] Failed to init GpuSynth: {}", e);
                                }
                            }
                        } else if let Some(ref mut synth) = self.engine.gpu_synth {
                            // 已有合成器：追加加载其他 port 的音色库
                            if let Err(e) =
                                synth.load_port_soundfonts(port, &dense_channels, &paths)
                            {
                                eprintln!("[gpu] Failed to load port {} soundfonts: {}", port, e);
                            }
                        }
                    }
                    // 非 GPU feature 下 paths 不使用，显式标记避免 warning
                    #[cfg(not(feature = "gpu"))]
                    let _ = paths;
                    self.clear_buffered_audio();
                    did_work = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        did_work
    }

    /// GPU 路径：从 engine 当前模型构建事件列表并加载到 GpuSynth
    #[cfg(feature = "gpu")]
    fn sync_gpu_synth_events(&mut self) {
        if self.engine.gpu_synth.is_none() {
            return;
        }
        // 先构建事件列表（需要借用 engine 的数据；seek_pos 决定鼓组事件与
        // 跨点音符复活位置，与 CPU 路径 seek_to 语义一致）
        let pos = self.engine.sample_position();
        let events = self.build_gpu_synth_events(pos);
        // 再加载到 synth（需要可变借用 engine.gpu_synth）
        if let Some(ref mut synth) = self.engine.gpu_synth {
            synth.load_events(events);
            synth.seek(pos);
        }
    }

    /// 从 engine 的当前 audible_notes + cc_events 构建 SynthEvent 列表（GPU 路径）
    /// 音符事件 + 通道控制事件（CC/pitch bend/RPN）统一转 sample 域并排序。
    ///
    /// `seek_pos`：渲染起点（0 = 从头）。鼓组/乐器模式事件在起点注入（seek 后
    /// GpuSynth 通道状态重置，必须重建——CPU 路径 SetPercussionMode 在模型加载
    /// 时应用且 ResetControl 不重置 program.bank）；起点之前开始、之后才结束的
    /// 音符在起点重启（与 CPU seek_to 的跨点音符重启一致）。
    ///
    /// 顺序：鼓组/CC 事件先于音符事件构建——stable sort 后同 sample 时 CC 先处理，
    /// 与 CPU 路径 dispatch（cc_cursor 循环在 note 循环之前）一致。
    #[cfg(feature = "gpu")]
    fn build_gpu_synth_events(&self, seek_pos: u64) -> Vec<yinhe_synth::SynthEvent> {
        let audio_model = match self.engine.model.as_ref() {
            Some(m) => m,
            None => return Vec::new(),
        };

        let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();

        // ── 鼓组/乐器模式初始状态（渲染起点注入；与 CPU setup_percussion 同序）──
        // 先 GM 鼓通道（每 port 的 9 通道），再模型 bank 声明（>=120 鼓组），
        // 同通道多条声明后者覆盖前者。
        for p in 0..16u8 {
            let src = p as usize * 16 + 9;
            let dense = self.engine.channel_layout.dense_for(src);
            if dense != u32::MAX {
                events.push(yinhe_synth::SynthEvent::Control {
                    sample: seek_pos,
                    channel: dense as u8,
                    event: yinhe_synth::ControlEvent::PercussionMode(true),
                });
            }
        }
        for (track_idx, banks) in audio_model.track_banks.iter().enumerate() {
            if banks.is_empty() {
                continue;
            }
            let src = audio_model.track_channel(track_idx) as usize;
            if src >= 256 {
                continue;
            }
            let dense = self.engine.channel_layout.dense_for(src);
            if dense == u32::MAX {
                continue;
            }
            for &(_, value) in banks {
                events.push(yinhe_synth::SynthEvent::Control {
                    sample: seek_pos,
                    channel: dense as u8,
                    event: yinhe_synth::ControlEvent::PercussionMode(value >= 120),
                });
            }
        }

        // ── 通道控制事件（CC/pitch bend/RPN），tick 域转 sample 域 ──
        // 放在音符事件之前：同 sample 时 CC 先于 note 处理（与 CPU dispatch 一致）
        for cc in self.engine.cc_events.iter() {
            // mute 的音轨：跳过其自动化事件（与 CPU 路径 dispatch 一致）
            if self
                .engine
                .skip_track
                .get(cc.track as usize)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let dense = self.engine.channel_layout.dense_for(cc.channel as usize);
            if dense == u32::MAX {
                continue;
            }
            let Some(event) = to_gpu_control_event(&cc.event) else {
                continue;
            };
            events.push(yinhe_synth::SynthEvent::Control {
                sample: self.engine.tick_to_sample(cc.tick),
                channel: dense as u8,
                event,
            });
        }

        // ── 音符事件（带 dense channel）──
        for key in 0..128usize {
            for note in self.engine.audible_notes[key].iter() {
                let track = note.track as usize;
                if self.engine.skip_track.get(track).copied().unwrap_or(false) {
                    continue;
                }
                let ch = audio_model.track_channel(track) as usize;
                let dense = self.engine.channel_layout.dense_for(ch);
                if dense == u32::MAX {
                    continue;
                }

                let start_sample = self.engine.tick_to_sample(note.start_tick);
                let end_sample = self.engine.tick_to_sample(note.end_tick);
                // 跨 seek 点的音符：在 seek 点重启（CPU seek_to 同语义）
                let on_sample = if start_sample < seek_pos && end_sample > seek_pos {
                    seek_pos
                } else {
                    start_sample
                };
                events.push(yinhe_synth::SynthEvent::NoteOn {
                    sample: on_sample,
                    channel: dense as u8,
                    key: key as u8,
                    velocity: note.velocity,
                });
                events.push(yinhe_synth::SynthEvent::NoteOff {
                    sample: end_sample,
                    channel: dense as u8,
                    key: key as u8,
                });
            }
        }

        events.sort_by_key(|e| e.sample());
        events
    }

    fn render_if_needed(&mut self) -> bool {
        if !self.state.initialized.load(Ordering::Acquire) {
            return false;
        }
        // 预览组非空或有余音时强制渲染：未播放时也要输出。
        let previewing = self.preview_engine.previewing();
        if !self.engine.playing() && !previewing {
            return false;
        }

        // 预览时用更小的 ring 目标（512 帧 ≈ 10ms）：预览是交互操作，
        // 音符 NoteOn 后要等 ring 里已有音频播完才出声，目标 4096 帧会带来
        // 约 85ms 延迟，快速拖动时每个音都滞后、听感响应很慢。
        let target_samples = if previewing {
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
            self.engine.render(&mut self.scratch);
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

        // GPU 路径在 GpuSynth::render 内部已做限幅；CPU 路径需要外部限幅
        #[cfg(feature = "gpu")]
        if self.engine.gpu_synth.is_none() {
            self.limiter.limit(&mut self.scratch);
        }
        #[cfg(not(feature = "gpu"))]
        self.limiter.limit(&mut self.scratch);

        let pushed = self.ring.push_slice(&self.scratch);
        debug_assert_eq!(pushed, self.scratch.len());
        true
    }

    fn clear_buffered_audio(&mut self) {
        // 不直接调 `self.ring.clear()`：它和 cpal 回调的 `pop_into` 并发时会
        // 把 cpal 刚推进的 read 指针覆盖回 write，下次回调会把旧数据当新数据读出 → 杂音。
        // 改为记录"清空边界"并 bump `reset_generation`，由 cpal 回调入口（单线程，
        // 与 pop_into 天然串行）用 `discard_before` 只丢弃边界前的旧音频。
        // 边界之后可能已推入新音频（模型已加载时渲染很快，ack 常晚于新音频入队），
        // 整体 clear 会把新播放位置的开头一起丢掉 —— 第二次播放开头缺失的根因。
        let base = self.engine.sample_position();
        self.state
            .producer_sample_position
            .store(base, Ordering::Release);
        self.state.clear_base_sample.store(base, Ordering::Release);
        self.state
            .clear_ring_write
            .store(self.ring.write_position(), Ordering::Release);
        self.state.reset_generation.fetch_add(1, Ordering::AcqRel);
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
    channels: u16,
    cmd_rx: Receiver<AudioCommand>,
    worker_tx: Sender<WorkerCmd>,
    prepared_rx: Receiver<WorkerResult>,
    shutdown: Arc<AtomicBool>,
    preview_stop_flag: Arc<AtomicBool>,
    // cpal 回调每次请求的帧数（预览时 ring 目标下限）。
    callback_frames: usize,
    #[cfg(feature = "gpu")] use_gpu_synth: bool,
) -> Result<JoinHandle<()>, std::io::Error> {
    thread::Builder::new()
        .name("audio-renderer".into())
        .spawn(move || {
            let mut renderer = AudioRenderer::new(
                engine,
                preview_engine,
                ring,
                state,
                channels,
                cmd_rx,
                worker_tx,
                prepared_rx,
                shutdown,
                preview_stop_flag,
                callback_frames,
                #[cfg(feature = "gpu")]
                use_gpu_synth,
            );
            renderer.run();
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

/// xsynth ChannelAudioEvent → GpuSynth 控制事件（播放事件构建与 chase 应用共用）。
#[cfg(feature = "gpu")]
pub(crate) fn to_gpu_control_event(
    ev: &xsynth_core::channel::ChannelAudioEvent,
) -> Option<yinhe_synth::ControlEvent> {
    match *ev {
        xsynth_core::channel::ChannelAudioEvent::Control(ControlEvent::Raw(c, v)) => {
            Some(yinhe_synth::ControlEvent::Raw(c, v))
        }
        xsynth_core::channel::ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => {
            Some(yinhe_synth::ControlEvent::PitchBend(v))
        }
        xsynth_core::channel::ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(v)) => {
            Some(yinhe_synth::ControlEvent::PitchBendSensitivity(v))
        }
        xsynth_core::channel::ChannelAudioEvent::Control(ControlEvent::FineTune(v)) => {
            Some(yinhe_synth::ControlEvent::FineTune(v))
        }
        xsynth_core::channel::ChannelAudioEvent::Control(ControlEvent::CoarseTune(v)) => {
            Some(yinhe_synth::ControlEvent::CoarseTune(v))
        }
        xsynth_core::channel::ChannelAudioEvent::ProgramChange(p) => {
            Some(yinhe_synth::ControlEvent::ProgramChange(p))
        }
        _ => None,
    }
}
