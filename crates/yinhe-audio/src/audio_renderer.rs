use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// cpal 回调入口对比自己记录的 acknowledged_generation，不一致就 clear ring。
    /// 生产者**不再等 ack** —— cpal 回调停了的话，等 ack 会永久卡死 renderer（P0-3）。
    pub(crate) reset_generation: Arc<AtomicU64>,
}

impl RendererSharedState {
    pub(crate) fn new() -> Self {
        Self {
            producer_sample_position: Arc::new(AtomicU64::new(0)),
            playing: Arc::new(AtomicBool::new(false)),
            duration_samples: Arc::new(AtomicU64::new(0)),
            initialized: Arc::new(AtomicBool::new(false)),
            reset_generation: Arc::new(AtomicU64::new(0)),
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
    /// 是否启用 GPU 合成器。启用后加载音色库时初始化 GpuSynth，渲染走 engine.gpu_synth。
    #[cfg(feature = "gpu")]
    use_gpu_synth: bool,
}

impl AudioRenderer {
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
            #[cfg(feature = "gpu")]
            use_gpu_synth,
        }
    }

    fn run(&mut self) {
        while !self.shutdown.load(Ordering::Relaxed) {
            let did_work = self.process_commands()
                | self.process_worker_results()
                | self.render_if_needed();

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
                                self.engine.handle_command(AudioCommand::Play { from_sample });
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
    /// `chase_generation` 用于丢弃过期结果（cc_events 被 PrepareModel 替换后）。
    fn request_chase(&self, target_sample: u64) {
        let cc_events = Arc::clone(&self.engine.cc_events);
        let generation = self.engine.chase_generation;
        let _ = self.worker_tx.send(WorkerCmd::PrepareChase {
            cc_events,
            target_sample,
            generation,
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
                    self.engine.apply_notes_only(model, yin_model, audible_notes, duration_samples);
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
                    if self.use_gpu_synth && self.engine.gpu_synth.is_none() {
                        if let Some(first_path) = paths.first() {
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
                if self.engine.skip_track.get(track).copied().unwrap_or(false) { continue; }
                let ch = audio_model.track_channel(track) as usize;
                if !self.engine.active_mask.get(ch).copied().unwrap_or(false) { continue; }

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
        if !self.state.initialized.load(Ordering::Acquire) || !self.engine.playing() {
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

        // 统一渲染路径：engine.render() 内部根据 gpu_synth 是否存在自动选择 GPU/CPU
        // GPU 路径：GpuSynth.render() → 限幅由 GpuSynth 内部完成
        // CPU 路径：xsynth → 需要外部限幅
        self.engine.render(&mut self.scratch);

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
        // 改用 `reset_generation` 通知 cpal 回调自己 clear（spawn.rs:517-522），
        // cpal 回调入口是单线程的，clear 和后续 pop_into 串行，无竞态。
        self.state
            .producer_sample_position
            .store(self.engine.sample_position(), Ordering::Release);
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
