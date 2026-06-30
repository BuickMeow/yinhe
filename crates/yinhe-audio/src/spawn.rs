use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, bounded, unbounded};
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;

use crate::audio_renderer::{RendererSharedState, spawn_renderer};
use crate::audio_ring::AudioRing;
use crate::engine::PreparedModel;

const STEREO_CHANNELS: usize = 2;
const RING_BUFFER_FRAMES: usize = 16_384;

/// Command sent from UI thread to the audio renderer thread.
pub enum AudioCommand {
    Play {
        from_sample: u64,
    },
    Resume,
    Pause,
    Stop,
    Seek {
        sample: u64,
    },
    LoadModel {
        model: Arc<YinModel>,
    },
    /// Like LoadModel but does NOT stop playback.
    /// Replaces the model reference and resets note cursors.
    ReloadNotes {
        model: Arc<YinModel>,
    },
    LoadSoundFont {
        port: u8,
        paths: Vec<String>,
    },
    /// `skip[i] == true` means track i is hidden (not audible).
    SkipTracks {
        skip: Vec<bool>,
    },
    /// Set per-key layer count (None = unlimited).
    SetLayerCount {
        count: Option<usize>,
    },
}

/// Handle used by the UI to control audio playback.
pub struct AudioHandle {
    pub(crate) cmd_tx: Sender<AudioCommand>,
    sample_position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    duration_samples: Arc<AtomicU64>,
}

impl AudioHandle {
    pub fn send(&self, cmd: AudioCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }

    pub fn sample_position_arc(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.sample_position)
    }

    pub fn playing_arc(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.playing)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn duration_samples(&self) -> u64 {
        self.duration_samples.load(Ordering::Relaxed)
    }
}

/// Result of spawning the audio backend.
pub struct CpalAudioHandle {
    pub handle: AudioHandle,
    pub sample_rate: u32,
    pub(crate) _stream: cpal::Stream,
}

/// Compute the global channel byte for a track: `(port << 4) | channel`.
#[inline]
pub(crate) fn track_global_channel(model: &YinModel, track_idx: usize) -> u8 {
    let t = match model.tracks.get(track_idx) {
        Some(t) => t,
        None => return 0,
    };
    (t.port & 0x0F) << 4 | (t.channel & 0x0F)
}

/// Analyse a YinModel and return (num_channels, active_mask).
///
/// A channel is "active" if any note with vel>1 lives on it, OR any
/// non-note control event is present on the owning track.
pub fn channels_for_model(model: &YinModel) -> (u32, Vec<bool>) {
    let mut ch_active = [0u32; 256];

    for bucket in model.notes.iter() {
        for n in bucket {
            if n.velocity > 1 {
                let ch = track_global_channel(model, n.track as usize) as usize;
                if ch < 256 {
                    ch_active[ch] = ch_active[ch].saturating_add(1);
                }
            }
        }
    }

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let ch = track_global_channel(model, track_idx) as usize;
        let has_ctrl = !track.cc.is_empty()
            || !track.pitch_bend.is_empty()
            || !track.program_change.is_empty()
            || !track.rpn.is_empty();
        if has_ctrl && ch < 256 {
            ch_active[ch] = ch_active[ch].max(1);
        }
    }

    let max_active_ch = ch_active.iter().rposition(|&c| c > 0).unwrap_or(0);
    let num_channels = (max_active_ch + 1).max(1) as u32;

    let active_mask: Vec<bool> = ch_active[..num_channels as usize]
        .iter()
        .map(|&c| c > 0)
        .collect();
    (num_channels, active_mask)
}

/// Internal command sent from the renderer thread to the worker thread.
pub(crate) enum WorkerCmd {
    PrepareModel(Arc<YinModel>),
    LoadSoundFont {
        port: u8,
        paths: Vec<String>,
        dense_channels: Vec<u32>,
    },
}

pub(crate) enum WorkerResult {
    PreparedModel(PreparedModel),
    LoadedSoundFont {
        port: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
        dense_channels: Vec<u32>,
    },
}

/// Spawn a background worker thread that processes heavy commands
/// (model preparation, soundfont loading) off the renderer thread.
pub(crate) fn spawn_worker(
    sample_rate: u32,
    active_mask: Vec<bool>,
    channel_map: Box<[u32; 256]>,
) -> (Sender<WorkerCmd>, crossbeam_channel::Receiver<WorkerResult>) {
    let (cmd_tx, cmd_rx) = unbounded::<WorkerCmd>();
    let (result_tx, result_rx) = bounded::<WorkerResult>(1);

    thread::Builder::new()
        .name("audio-worker".into())
        .spawn(move || {
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    WorkerCmd::PrepareModel(model) => {
                        let mut latest = model;
                        let mut pending_other: Option<WorkerCmd> = None;
                        while let Ok(next) = cmd_rx.try_recv() {
                            match next {
                                WorkerCmd::PrepareModel(m) => latest = m,
                                other => {
                                    pending_other = Some(other);
                                    break;
                                }
                            }
                        }
                        let prepared = crate::engine::prepare_model(
                            &latest,
                            sample_rate,
                            &active_mask,
                            &channel_map,
                        );
                        let _ = result_tx.send(WorkerResult::PreparedModel(prepared));
                        if let Some(other) = pending_other {
                            match other {
                                WorkerCmd::LoadSoundFont {
                                    port,
                                    paths,
                                    dense_channels,
                                } => {
                                    if let Ok(soundfonts) =
                                        crate::engine::AudioEngine::load_soundfont_paths(
                                            sample_rate,
                                            &paths,
                                        )
                                    {
                                        let _ = result_tx.send(WorkerResult::LoadedSoundFont {
                                            port,
                                            soundfonts,
                                            dense_channels,
                                        });
                                    }
                                }
                                WorkerCmd::PrepareModel(_) => unreachable!(),
                            }
                        }
                    }
                    WorkerCmd::LoadSoundFont {
                        port,
                        paths,
                        dense_channels,
                    } => {
                        if let Ok(soundfonts) =
                            crate::engine::AudioEngine::load_soundfont_paths(sample_rate, &paths)
                        {
                            let _ = result_tx.send(WorkerResult::LoadedSoundFont {
                                port,
                                soundfonts,
                                dense_channels,
                            });
                        }
                    }
                }
            }
        })
        .expect("Failed to spawn audio worker thread");

    (cmd_tx, result_rx)
}

/// Spawn a CPAL audio stream backed by a producer/consumer audio FIFO.
///
/// The CPAL callback only consumes already-rendered contiguous samples from the
/// ring buffer. All command processing, model application and XSynth rendering
/// live on the renderer thread.
pub fn spawn_cpal_audio(
    sample_rate: u32,
    num_channels: u32,
    active_mask: Vec<bool>,
    buffer_size: cpal::BufferSize,
) -> Result<CpalAudioHandle, String> {
    let (cmd_tx, cmd_rx) = unbounded::<AudioCommand>();
    let sample_position = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let duration_samples = Arc::new(AtomicU64::new(0));

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("No output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let channels = supported.channels() as usize;

    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate,
        buffer_size,
    };

    let engine = crate::engine::AudioEngine::new(sample_rate, num_channels, active_mask);
    let channel_map = engine.channel_map_clone();
    let (worker_tx, prepared_rx) =
        spawn_worker(sample_rate, engine.active_mask().to_vec(), channel_map);

    let ring_capacity = (RING_BUFFER_FRAMES * STEREO_CHANNELS).next_power_of_two();
    let (ring_producer, mut ring_consumer) = AudioRing::new(ring_capacity).split();

    let renderer_state = RendererSharedState::new();
    let renderer_position = Arc::clone(&renderer_state.producer_sample_position);
    let renderer_playing = Arc::clone(&renderer_state.playing);
    let renderer_duration = Arc::clone(&renderer_state.duration_samples);
    let initialized = Arc::clone(&renderer_state.initialized);
    let reset_generation = Arc::clone(&renderer_state.reset_generation);
    let reset_ack = Arc::clone(&renderer_state.reset_ack);

    let shutdown = Arc::new(AtomicBool::new(false));
    let _renderer_handle = spawn_renderer(
        engine,
        ring_producer,
        renderer_state,
        channels as u16,
        cmd_rx,
        worker_tx,
        prepared_rx,
        Arc::clone(&shutdown),
    );

    let sp = Arc::clone(&sample_position);
    let pl = Arc::clone(&playing);
    let dur = Arc::clone(&duration_samples);
    let mut consumer_sample_position = 0u64;
    let mut acknowledged_generation = 0u64;

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
                    let generation = reset_generation.load(Ordering::Acquire);
                    if generation != acknowledged_generation {
                        ring_consumer.clear();
                        consumer_sample_position = renderer_position.load(Ordering::Acquire);
                        acknowledged_generation = generation;
                        reset_ack.store(generation, Ordering::Release);
                    }

                    if initialized.load(Ordering::Acquire) {
                        let popped = ring_consumer.pop_into(data);
                        if popped < data.len() {
                            data[popped..].fill(0.0);
                        }
                        consumer_sample_position = consumer_sample_position
                            .saturating_add((popped / STEREO_CHANNELS) as u64);
                    } else {
                        data.fill(0.0);
                    }

                    sp.store(consumer_sample_position, Ordering::Relaxed);
                    pl.store(renderer_playing.load(Ordering::Relaxed), Ordering::Relaxed);
                    dur.store(renderer_duration.load(Ordering::Relaxed), Ordering::Relaxed);
                })
            },
            |err| eprintln!("Audio stream error: {}", err),
            None,
        )
        .map_err(|e| format!("Failed to build stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start stream: {e}"))?;

    Ok(CpalAudioHandle {
        handle: AudioHandle {
            cmd_tx,
            sample_position,
            playing,
            duration_samples,
        },
        sample_rate,
        _stream: stream,
    })
}
