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
    pub(crate) reset_generation: Arc<AtomicU64>,
    pub(crate) reset_ack: Arc<AtomicU64>,
}

impl RendererSharedState {
    pub(crate) fn new() -> Self {
        Self {
            producer_sample_position: Arc::new(AtomicU64::new(0)),
            playing: Arc::new(AtomicBool::new(false)),
            duration_samples: Arc::new(AtomicU64::new(0)),
            initialized: Arc::new(AtomicBool::new(false)),
            reset_generation: Arc::new(AtomicU64::new(0)),
            reset_ack: Arc::new(AtomicU64::new(0)),
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
                            pending_reload = Some(model);
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
                        }
                        AudioCommand::Stop => {
                            self.engine.handle_command(AudioCommand::Stop);
                            #[cfg(feature = "gpu")]
                            if let Some(ref mut synth) = self.engine.gpu_synth {
                                synth.seek(0);
                            }
                            self.clear_buffered_audio();
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
                    did_work = true;
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

    /// 从 engine 的当前 model 构建 SynthEvent 列表
    #[cfg(feature = "gpu")]
    fn build_gpu_synth_events(&self) -> Vec<yinhe_synth::SynthEvent> {
        use crate::audio_model::tick_to_sample;

        let yin_model = match self.engine.yin_model.as_ref() {
            Some(m) => m,
            None => return Vec::new(),
        };
        let audio_model = match self.engine.model.as_ref() {
            Some(m) => m,
            None => return Vec::new(),
        };

        let segments = &yin_model.tempo_map.tempo_segments;
        let tpb = yin_model.tempo_map.ticks_per_beat;
        let sr = self.engine.sample_rate as f64;

        let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();
        for key in 0..128usize {
            for note in yin_model.notes[key].iter() {
                if note.velocity <= 1 { continue; }
                let track = note.track as usize;
                if self.engine.skip_track.get(track).copied().unwrap_or(false) { continue; }
                let ch = audio_model.track_channel(track) as usize;
                if !self.engine.active_mask.get(ch).copied().unwrap_or(false) { continue; }

                let start_sample = tick_to_sample(note.start_tick as u64, segments, tpb, sr);
                let end_sample = tick_to_sample(note.end_tick as u64, segments, tpb, sr);

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
        events
    }

    fn render_if_needed(&mut self) -> bool {
        if !self.state.initialized.load(Ordering::Acquire) || !self.engine.playing() {
            return false;
        }

        if self.state.reset_generation.load(Ordering::Acquire)
            != self.state.reset_ack.load(Ordering::Acquire)
        {
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
        self.ring.clear();
        self.state
            .producer_sample_position
            .store(self.engine.sample_position(), Ordering::Release);
        let generation = self.state.reset_generation.fetch_add(1, Ordering::AcqRel) + 1;
        while self.state.reset_ack.load(Ordering::Acquire) != generation
            && !self.shutdown.load(Ordering::Relaxed)
        {
            thread::sleep(WAKE_SLEEP);
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
) -> JoinHandle<()> {
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
        })
        .expect("Failed to spawn audio renderer thread")
}
