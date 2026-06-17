use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, TryRecvError, unbounded};

use yinhe_core::YinModel;

/// Command sent from UI thread to the audio callback.
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
    for (track_idx, track) in model.tracks.iter().enumerate() {
        let ch = track_global_channel(model, track_idx) as usize;
        // Notes with vel > 1
        for n in &track.notes {
            if n.velocity > 1 {
                ch_active[ch] = ch_active[ch].saturating_add(1);
            }
        }
        // Any non-note event activates the channel too.
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

/// Spawn a CPAL audio stream with the engine running directly in the callback.
pub fn spawn_cpal_audio(
    sample_rate: u32,
    num_channels: u32,
    active_mask: Vec<bool>,
) -> Result<CpalAudioHandle, String> {
    let (cmd_tx, cmd_rx) = unbounded::<AudioCommand>();
    let sample_position = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let duration_samples = Arc::new(AtomicU64::new(0));

    let sp = Arc::clone(&sample_position);
    let pl = Arc::clone(&playing);
    let dur = Arc::clone(&duration_samples);

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("No output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let channels = supported.channels() as usize;

    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let mut engine = crate::engine::AudioEngine::new(sample_rate, num_channels, active_mask);
    let mut initialized = false;

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
                    // Process commands
                    loop {
                        match cmd_rx.try_recv() {
                            Ok(cmd) => {
                                let is_load = matches!(&cmd, AudioCommand::LoadModel { .. });
                                if let AudioCommand::LoadModel { ref model } = cmd {
                                    dur.store(
                                        (model.tempo_map.tick_to_seconds(model.tick_length)
                                            * engine.sample_rate_hz() as f64)
                                            as u64,
                                        Ordering::Relaxed,
                                    );
                                }
                                engine.handle_command(cmd);
                                if is_load {
                                    initialized = true;
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                data.fill(0.0);
                                return;
                            }
                        }
                    }

                    pl.store(engine.playing(), Ordering::Relaxed);

                    if initialized {
                        engine.render(data);
                    } else {
                        data.fill(0.0);
                    }

                    sp.store(engine.sample_position(), Ordering::Relaxed);
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
