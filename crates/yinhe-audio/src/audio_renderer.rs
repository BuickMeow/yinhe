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

        loop {
            match self.cmd_rx.try_recv() {
                Ok(cmd) => {
                    did_work = true;
                    match cmd {
                        AudioCommand::LoadModel { model } => {
                            self.engine.handle_command(AudioCommand::Pause);
                            self.engine.handle_command(AudioCommand::Stop);
                            self.clear_buffered_audio();
                            let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model));
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
                                self.clear_buffered_audio();
                            } else {
                                self.engine.set_pending_play(from_sample);
                            }
                        }
                        AudioCommand::Seek { sample } => {
                            self.engine.handle_command(AudioCommand::Seek { sample });
                            self.clear_buffered_audio();
                        }
                        AudioCommand::Stop => {
                            self.engine.handle_command(AudioCommand::Stop);
                            self.clear_buffered_audio();
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
            let _ = self.worker_tx.send(WorkerCmd::PrepareModel(model));
            did_work = true;
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
                    self.clear_buffered_audio();
                    self.state.initialized.store(true, Ordering::Release);
                    did_work = true;
                }
                Ok(WorkerResult::LoadedSoundFont {
                    port,
                    soundfonts,
                    dense_channels,
                }) => {
                    self.engine
                        .apply_loaded_soundfont_for_port(port, soundfonts, &dense_channels);
                    self.clear_buffered_audio();
                    did_work = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        did_work
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

        self.engine.render(&mut self.scratch);
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
            );
            renderer.run();
        })
        .expect("Failed to spawn audio renderer thread")
}
