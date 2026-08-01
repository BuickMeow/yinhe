use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use xsynth_core::effects::VolumeLimiter;

use crate::audio_ring::AudioRingProducer;
use crate::engine::AudioEngine;
use crate::spawn::{AudioCommand, WorkerCmd, WorkerResult};

const STEREO_CHANNELS: usize = 2;
const RENDER_CHUNK_FRAMES: usize = 512;
const TARGET_BUFFER_FRAMES: usize = 4096;
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
        }
    }
}

/// 渲染器侧的音符预览状态：NoteOn 后由渲染时钟自动 NoteOff（无需定时器线程）。
struct PreviewNote {
    channel: u8,
    key: u8,
    /// `Some(dur)`：渲染 `dur` 帧后自动 NoteOff；`None`：持续音（等 `PreviewStop`）。
    duration_samples: Option<u64>,
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
    /// 当前预览音符（`None` = 无预览）。
    preview: Option<PreviewNote>,
    /// 预览开始后实际渲染输出的帧数（预览音被播放的时长）。
    preview_elapsed: u64,
    /// 是否启用 GPU 合成器。启用后加载音色库时初始化 GpuSynth，渲染走 engine.gpu_synth。
    #[cfg(feature = "gpu")]
    use_gpu_synth: bool,
}

impl AudioRenderer {
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    fn new(
        engine: AudioEngine,
        ring: AudioRingProducer,
        state: RendererSharedState,
        channels: u16,
        cmd_rx: Receiver<AudioCommand>,
        worker_tx: Sender<WorkerCmd>,
        prepared_rx: Receiver<WorkerResult>,
        shutdown: Arc<AtomicBool>,
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
            preview: None,
            preview_elapsed: 0,
            #[cfg(feature = "gpu")]
            use_gpu_synth,
        }
    }

    fn run(&mut self) {
        while !self.shutdown.load(Ordering::Relaxed) {
            let did_work = self.process_commands()
                | self.process_worker_results()
                | self.render_if_needed()
                | self.check_preview_done();

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
                            self.preview_off();
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
                                self.preview_off();
                                self.engine
                                    .handle_command(AudioCommand::Play { from_sample });
                                // GPU 路径：同步 GpuSynth 位置
                                #[cfg(feature = "gpu")]
                                if let Some(ref mut synth) = self.engine.gpu_synth {
                                    synth.seek(from_sample);
                                }
                                self.clear_buffered_audio();
                                // 方案 B：seek 后异步 chase
                                self.request_chase(from_sample);
                            } else {
                                self.engine.set_pending_play(from_sample);
                            }
                        }
                        AudioCommand::Seek { sample } => {
                            self.preview_off();
                            self.engine.handle_command(AudioCommand::Seek { sample });
                            #[cfg(feature = "gpu")]
                            if let Some(ref mut synth) = self.engine.gpu_synth {
                                synth.seek(sample);
                            }
                            self.clear_buffered_audio();
                            // 方案 B：seek 后异步 chase
                            self.request_chase(sample);
                        }
                        AudioCommand::Stop => {
                            self.preview_off();
                            self.engine.handle_command(AudioCommand::Stop);
                            #[cfg(feature = "gpu")]
                            if let Some(ref mut synth) = self.engine.gpu_synth {
                                synth.seek(0);
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
                            // mute 状态变了，chase 结果需要更新：
                            // unmute 的轨道的 CC 需要恢复，mute 的轨道的 CC 不再参与。
                            if self.engine.model_loaded() {
                                self.request_chase(self.engine.sample_position());
                            }
                        }
                        AudioCommand::PreviewNote {
                            channel,
                            key,
                            velocity,
                            target_sample,
                            duration_samples,
                        } => {
                            // retrigger：先停旧预览音（NoteOff + 恢复旧通道状态）。
                            self.preview_off();
                            // 应用目标位置的自动化状态，让预览音按目标位置的音色/音量发声。
                            self.engine.preview_apply_state(channel, target_sample);
                            self.engine.preview_note_on(channel, key, velocity);
                            self.preview = Some(PreviewNote {
                                channel,
                                key,
                                duration_samples: if duration_samples > 0 {
                                    Some(duration_samples)
                                } else {
                                    None
                                },
                            });
                            self.preview_elapsed = 0;
                        }
                        AudioCommand::PreviewStop => {
                            self.preview_off();
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

    /// 停止当前预览音：NoteOff；播放中再把该通道状态恢复为当前播放位置的值
    /// （预览期间通道状态被切到目标位置，恢复后不影响正在播放的自动化）。
    fn preview_off(&mut self) {
        let Some(p) = self.preview.take() else { return };
        self.engine.preview_note_off(p.channel, p.key);
        if self.engine.playing() {
            self.engine
                .preview_apply_state(p.channel, self.engine.sample_position());
        }
        self.preview_elapsed = 0;
    }

    /// 定长预览到期检查：预览音实际渲染输出的帧数达到 duration 就 NoteOff。
    /// 未播放时渲染器持续为预览输出，elapsed 照常推进，因此无需定时器线程。
    fn check_preview_done(&mut self) -> bool {
        if let Some(p) = &self.preview
            && let Some(dur) = p.duration_samples
            && self.preview_elapsed >= dur
        {
            self.preview_off();
            return true;
        }
        false
    }

    /// 方案 B：发 `PrepareChase` 给 worker 线程异步计算 256 通道状态快照。
    /// worker 完成后回传 `ChaseResult`，`process_worker_results` 应用。
    /// `chase_generation` 用于丢弃过期结果（cc_events 被 PrepareModel 替换后）。
    fn request_chase(&self, target_sample: u64) {
        let cc_events = Arc::clone(&self.engine.cc_events);
        let generation = self.engine.chase_generation;
        let skip_mask = self.engine.skip_track.clone();
        let _ = self.worker_tx.send(WorkerCmd::PrepareChase {
            cc_events,
            target_sample,
            generation,
            skip_mask,
        });
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
                    self.request_chase(self.engine.sample_position());
                    did_work = true;
                }
                Ok(WorkerResult::PreparedNotes {
                    model,
                    yin_model,
                    audible_notes,
                    duration_samples,
                }) => {
                    self.state
                        .duration_samples
                        .store(duration_samples, Ordering::Relaxed);
                    self.engine
                        .apply_notes_only(model, yin_model, audible_notes, duration_samples);
                    // GPU 路径：音符变化后同步事件到 GpuSynth
                    #[cfg(feature = "gpu")]
                    self.sync_gpu_synth_events();
                    self.clear_buffered_audio();
                    self.state.initialized.store(true, Ordering::Release);
                    did_work = true;
                }
                Ok(WorkerResult::ChaseResult { states, generation }) => {
                    // 丢弃过期结果：cc_events 已被新 PrepareModel 替换
                    if generation == self.engine.chase_generation {
                        self.engine.apply_chase_result(states);
                        did_work = true;
                    }
                }
                Ok(WorkerResult::LoadedSoundFont {
                    port,
                    soundfonts,
                    dense_channels,
                    paths,
                }) => {
                    self.engine
                        .apply_loaded_soundfont_for_port(port, soundfonts, &dense_channels);
                    // GPU 路径：首次加载音色库时初始化 GpuSynth
                    #[cfg(feature = "gpu")]
                    if self.use_gpu_synth
                        && self.engine.gpu_synth.is_none()
                        && let Some(first_path) = paths.first()
                    {
                        let sr = self.engine.sample_rate;
                        match yinhe_synth::GpuSynth::new_default(
                            std::path::Path::new(first_path),
                            sr,
                        ) {
                            Ok(mut synth) => {
                                // 加载当前模型的事件
                                let events = self.build_gpu_synth_events();
                                synth.load_events(events);
                                synth.seek(self.engine.sample_position());
                                self.engine.gpu_synth = Some(synth);
                                eprintln!("[gpu] GpuSynth initialized from {}", first_path);
                            }
                            Err(e) => {
                                eprintln!("[gpu] Failed to init GpuSynth: {}", e);
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
        // 先构建事件列表（需要借用 engine 的数据）
        let events = self.build_gpu_synth_events();
        // 再加载到 synth（需要可变借用 engine.gpu_synth）
        let pos = self.engine.sample_position();
        if let Some(ref mut synth) = self.engine.gpu_synth {
            synth.load_events(events);
            synth.seek(pos);
        }
    }

    /// 从 engine 的当前 audible_notes 构建 SynthEvent 列表（GPU 路径）
    #[cfg(feature = "gpu")]
    fn build_gpu_synth_events(&self) -> Vec<yinhe_synth::SynthEvent> {
        let audio_model = match self.engine.model.as_ref() {
            Some(m) => m,
            None => return Vec::new(),
        };

        let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();
        for key in 0..128usize {
            for note in self.engine.audible_notes[key].iter() {
                let track = note.track as usize;
                if self.engine.skip_track.get(track).copied().unwrap_or(false) {
                    continue;
                }
                let ch = audio_model.track_channel(track) as usize;
                if self.engine.channel_layout.dense_for(ch) == u32::MAX {
                    continue;
                }

                events.push(yinhe_synth::SynthEvent {
                    sample: note.start_sample,
                    key: key as u8,
                    velocity: note.velocity,
                    is_on: true,
                });
                events.push(yinhe_synth::SynthEvent {
                    sample: note.end_sample,
                    key: key as u8,
                    velocity: 0,
                    is_on: false,
                });
            }
        }
        events.sort_by_key(|e| e.sample);
        events
    }

    fn render_if_needed(&mut self) -> bool {
        if !self.state.initialized.load(Ordering::Acquire) {
            return false;
        }
        // 预览激活时强制渲染：未播放时也要持续输出预览音（cpal 回调消费 ring）。
        let previewing = self.preview.is_some();
        if !self.engine.playing() && !previewing {
            return false;
        }

        let target_samples = TARGET_BUFFER_FRAMES * STEREO_CHANNELS;
        if self.ring.len() >= target_samples {
            return false;
        }

        let free = self.ring.free_space();
        if free < self.scratch.len() {
            return false;
        }

        if previewing && !self.engine.playing() {
            // 未播放预览：只输出现有 voice 的音频，不推进位置、不 dispatch 工程事件。
            self.engine.render_preview(&mut self.scratch);
        } else {
            // 播放路径：预览音与工程音符在 channel_group 里天然混音。
            self.engine.render(&mut self.scratch);
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
        if previewing {
            // 预览时长按实际渲染输出帧数累计（ring 满停渲时不计）。
            self.preview_elapsed += RENDER_CHUNK_FRAMES as u64;
        }
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
    ring: AudioRingProducer,
    state: RendererSharedState,
    channels: u16,
    cmd_rx: Receiver<AudioCommand>,
    worker_tx: Sender<WorkerCmd>,
    prepared_rx: Receiver<WorkerResult>,
    shutdown: Arc<AtomicBool>,
    #[cfg(feature = "gpu")] use_gpu_synth: bool,
) -> Result<JoinHandle<()>, std::io::Error> {
    thread::Builder::new()
        .name("audio-renderer".into())
        .spawn(move || {
            let mut renderer = AudioRenderer::new(
                engine,
                ring,
                state,
                channels,
                cmd_rx,
                worker_tx,
                prepared_rx,
                shutdown,
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
