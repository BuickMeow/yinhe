use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, bounded, unbounded};
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;

use crate::audio_renderer::{RendererSharedState, spawn_renderer};
use crate::audio_ring::AudioRing;
use crate::audio_model::{AudibleNote, PreparedModel, SortedCC};
use crate::channel::ChannelState;

const STEREO_CHANNELS: usize = 2;
const RING_BUFFER_FRAMES: usize = 16_384;

/// UI → renderer 命令通道容量。
///
/// 16 足够吸收任何合理的用户操作突发（按钮连点、设置切换、文档切换序列），
/// 同时硬性防止 renderer 卡死时命令无限堆积导致内存爆炸 + 鬼畜执行。
///
/// 队列满时 `AudioHandle::send` 走 `try_send` 丢弃新命令并记日志 ——
/// 不阻塞 UI 线程。renderer 已对 `ReloadNotes`/`UpdateNotes` 做同类型合并，
/// worker 对 `PrepareModel`/`PrepareNotes`/`PrepareChase` 也做合并，
/// 因此偶发丢弃只会导致短暂的 UI/音频错位，下一次用户操作即重新同步。
const AUDIO_CMD_CHANNEL_CAPACITY: usize = 16;
/// 编译期保证 `AudioRing` 的容量是 2 的幂（`AudioRing::new` 依赖此不变量做位运算取模）。
/// 任何修改 `RING_BUFFER_FRAMES` / `STEREO_CHANNELS` 或去掉 `.next_power_of_two()` 的改动
/// 都会在编译期触发 assert，而不是等到运行期才 panic。
const RING_CAPACITY: usize = (RING_BUFFER_FRAMES * STEREO_CHANNELS).next_power_of_two();
const _: () = assert!(RING_CAPACITY.is_power_of_two(), "RING_CAPACITY must be a power of two");

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
    /// Full rebuild: cc_events + audible_notes + chase.
    /// Used for automation edits / undo / redo / arrange drag (notes+automation).
    ReloadNotes {
        model: Arc<YinModel>,
    },
    /// Only rebuild `audible_notes` — no CC rebuild, no chase.
    /// Used for pure note edits (move/drag/add/delete/paste/duplicate/transpose)
    /// where automation lanes are untouched. Keeps current playback position and
    /// channel state intact, only affects future note dispatch.
    UpdateNotes {
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
    /// Set automation Linear/Curve intermediate event density (tick interval).
    /// Triggers a cc_events rebuild if a model is loaded.
    SetAutomationDensity {
        density: u32,
    },
}

/// Handle used by the UI to control audio playback.
pub struct AudioHandle {
    pub(crate) cmd_tx: Sender<AudioCommand>,
    sample_position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    duration_samples: Arc<AtomicU64>,
    /// 由 cpal 流错误回调置位。UI 每帧查询，若为 true 应弹窗提示用户重启。
    stream_error: Arc<AtomicBool>,
}

impl AudioHandle {
    /// 发命令给 renderer 线程。
    ///
    /// 通道容量 `AUDIO_CMD_CHANNEL_CAPACITY`（16）。满时 `try_send` 失败 →
    /// 丢弃新命令 + `warn!` 日志，绝不阻塞 UI 线程。
    /// - `Full`：renderer 处理不过来。renderer 已对 `ReloadNotes`/`UpdateNotes`
    ///   做同类型合并，worker 对 `PrepareModel`/`PrepareNotes`/`PrepareChase`
    ///   也做合并，因此偶发丢弃只造成短暂 UI/音频错位，下一次操作即重新同步。
    /// - `Disconnected`：renderer 线程已退出。仅记日志，不 panic ——
    ///   渲染线程死亡不应该让 UI 也跟着崩。
    pub fn send(&self, cmd: AudioCommand) {
        match self.cmd_tx.try_send(cmd) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                tracing::warn!("AudioHandle::send: channel full, dropping command");
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                tracing::warn!("AudioHandle::send: channel disconnected, dropping command");
            }
        }
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

    /// 查询 cpal 流是否已报错（设备热拔、驱动崩溃等）。
    /// 一旦置位就不会清零，UI 应弹出"需要重启"对话框。
    pub fn stream_error(&self) -> bool {
        self.stream_error.load(Ordering::Relaxed)
    }
}

/// Result of spawning the audio backend.
pub struct CpalAudioHandle {
    pub handle: AudioHandle,
    pub sample_rate: u32,
    pub(crate) _stream: cpal::Stream,
    /// 设置为 true 时通知 renderer 线程退出。
    pub(crate) shutdown: Arc<AtomicBool>,
}

impl Drop for CpalAudioHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

impl CpalAudioHandle {
    /// Notify the audio thread that the MIDI model has changed (full rebuild:
    /// cc_events + audible_notes + chase). Use for automation edits / undo / redo.
    pub fn reload_notes(&self, model: Arc<YinModel>) {
        let _ = self.handle.send(AudioCommand::ReloadNotes { model });
    }

    /// Notify the audio thread that only notes have changed (no automation, no
    /// chase). Use for pure note edits — keeps current channel state intact.
    pub fn update_notes(&self, model: Arc<YinModel>) {
        let _ = self.handle.send(AudioCommand::UpdateNotes { model });
    }
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
        for n in bucket.iter() {
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
        let has_ctrl = !track.automation_lanes.is_empty()
            || !track.program_change.is_empty();
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
    /// Full prepare: cc_events + audible_notes + duration (LoadModel / ReloadNotes).
    PrepareModel(Arc<YinModel>, u32),
    /// Notes-only prepare: audible_notes + duration (UpdateNotes). No cc_events rebuild.
    PrepareNotes(Arc<YinModel>),
    /// Compute channel-state snapshot at `target_sample` by linear-scanning `cc_events`.
    /// `generation` matches `AudioEngine::chase_generation` so the renderer can
    /// discard stale results after a PrepareModel replaces cc_events.
    PrepareChase {
        cc_events: Arc<Vec<SortedCC>>,
        target_sample: u64,
        generation: u64,
    },
    LoadSoundFont {
        port: u8,
        paths: Vec<String>,
        dense_channels: Vec<u32>,
    },
}

pub(crate) enum WorkerResult {
    PreparedModel(PreparedModel),
    /// Result of `PrepareNotes` — only audible_notes + duration + model refs.
    PreparedNotes {
        model: crate::audio_model::AudioModel,
        yin_model: Arc<YinModel>,
        audible_notes: Box<[Vec<AudibleNote>; 128]>,
        duration_samples: u64,
    },
    /// Result of `PrepareChase` — 256-channel state snapshot.
    ChaseResult {
        states: Box<[ChannelState; 256]>,
        generation: u64,
    },
    LoadedSoundFont {
        port: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
        dense_channels: Vec<u32>,
        /// 原始路径列表 — GPU 路径用其初始化 GpuPlayer
        paths: Vec<String>,
    },
}

/// Spawn a background worker thread that processes heavy commands
/// (model preparation, soundfont loading) off the renderer thread.
///
/// 返回 `Err` 而非 `.expect()`：线程 spawn 失败属于环境/资源问题（ulimit、线程数上限等），
/// 调用方应给出用户可见的错误，而不是直接 abort 进程。
pub(crate) fn spawn_worker(
    sample_rate: u32,
    active_mask: Vec<bool>,
    channel_map: Box<[u32; 256]>,
) -> Result<(Sender<WorkerCmd>, crossbeam_channel::Receiver<WorkerResult>), std::io::Error> {
    let (cmd_tx, cmd_rx) = unbounded::<WorkerCmd>();
    let (result_tx, result_rx) = bounded::<WorkerResult>(1);

    thread::Builder::new()
        .name("audio-worker".into())
        .spawn(move || {
            // 内部 pending 缓冲：处理某个命令时，try_recv 到的非同类型命令存这里。
            // 下次循环优先从 pending 取，避免饿死后续命令。
            let mut pending: std::collections::VecDeque<WorkerCmd> =
                std::collections::VecDeque::new();
            loop {
                let cmd = match pending.pop_front() {
                    Some(c) => c,
                    None => match cmd_rx.recv() {
                        Ok(c) => c,
                        Err(_) => break,
                    },
                };
                match cmd {
                    WorkerCmd::PrepareModel(model, density) => {
                        // 合并连续 PrepareModel，只保留最新
                        let mut latest = model;
                        let mut latest_density = density;
                        while let Ok(next) = cmd_rx.try_recv() {
                            match next {
                                WorkerCmd::PrepareModel(m, d) => {
                                    latest = m;
                                    latest_density = d;
                                }
                                other => {
                                    pending.push_back(other);
                                }
                            }
                        }
                        let prepared = crate::prepare_model::prepare_model(
                            &latest,
                            sample_rate,
                            latest_density,
                            &active_mask,
                            &channel_map,
                        );
                        let _ = result_tx.send(WorkerResult::PreparedModel(prepared));
                    }
                    WorkerCmd::PrepareNotes(model) => {
                        // 合并连续 PrepareNotes，只保留最新
                        let mut latest = model;
                        while let Ok(next) = cmd_rx.try_recv() {
                            match next {
                                WorkerCmd::PrepareNotes(m) => {
                                    latest = m;
                                }
                                other => {
                                    pending.push_back(other);
                                }
                            }
                        }
                        let (audio_model, yin_model, audible_notes, duration_samples) =
                            crate::prepare_model::prepare_notes(&latest, sample_rate);
                        let _ = result_tx.send(WorkerResult::PreparedNotes {
                            model: audio_model,
                            yin_model,
                            audible_notes,
                            duration_samples,
                        });
                    }
                    WorkerCmd::PrepareChase {
                        cc_events,
                        target_sample,
                        generation,
                    } => {
                        // 合并连续 PrepareChase，只保留最新（同 generation 或不同 generation 都只留最新）
                        let mut latest_cc = cc_events;
                        let mut latest_target = target_sample;
                        let mut latest_gen = generation;
                        while let Ok(next) = cmd_rx.try_recv() {
                            match next {
                                WorkerCmd::PrepareChase {
                                    cc_events,
                                    target_sample,
                                    generation,
                                } => {
                                    latest_cc = cc_events;
                                    latest_target = target_sample;
                                    latest_gen = generation;
                                }
                                other => {
                                    pending.push_back(other);
                                }
                            }
                        }
                        let states = compute_chase_states(&latest_cc, latest_target);
                        let _ = result_tx.send(WorkerResult::ChaseResult {
                            states,
                            generation: latest_gen,
                        });
                    }
                    WorkerCmd::LoadSoundFont {
                        port,
                        paths,
                        dense_channels,
                    } => {
                        // 不合并，但把 try_recv 到的命令存到 pending 避免饿死
                        while let Ok(next) = cmd_rx.try_recv() {
                            pending.push_back(next);
                        }
                        if let Ok(soundfonts) =
                            crate::engine::AudioEngine::load_soundfont_paths(sample_rate, &paths)
                        {
                            let _ = result_tx.send(WorkerResult::LoadedSoundFont {
                                port,
                                soundfonts,
                                dense_channels,
                                paths,
                            });
                        }
                    }
                }
            }
        })
        .map_err(|e| {
            tracing::error!("Failed to spawn audio worker thread: {e}");
            e
        })?;

    Ok((cmd_tx, result_rx))
}

/// 在 worker 线程上从 `cc_events[0]` 线性扫到 `target_sample`，构建 256 通道状态快照。
/// 这部分计算从 renderer 线程移出来（方案 B），避免 seek 时 renderer 阻塞几十万次
/// `ChannelState::apply`。结果由 renderer 的 `apply_chase_result` 直接 `send_to`。
fn compute_chase_states(cc_events: &[SortedCC], target_sample: u64) -> Box<[ChannelState; 256]> {
    let mut states: Box<[ChannelState; 256]> = Box::new([ChannelState::default(); 256]);
    for cc in cc_events {
        if cc.sample >= target_sample {
            break;
        }
        let ch = cc.channel as usize;
        if ch >= 256 {
            continue;
        }
        states[ch].apply(&cc.event);
    }
    states
}

/// 列出系统所有可用输出设备的描述名（cpal `Device::description()`）。
///
/// 用于设置面板和"音频设备切换"对话框。任何错误都被吞掉返回空 Vec ——
/// 列设备是 UI 辅助，失败不应阻塞音频引擎本身。
pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| {
            devices
                .filter_map(|d| d.description().ok().map(|desc| desc.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Spawn a CPAL audio stream backed by a producer/consumer audio FIFO.
///
/// The CPAL callback only consumes already-rendered contiguous samples from the
/// ring buffer. All command processing, model application and XSynth rendering
/// live on the renderer thread.
///
/// `device_name`: 指定输出设备名（来自 `list_output_devices()`）。`None` 表示用
/// 系统默认输出设备。设备热拔后用户在切换对话框里挑一个名字传进来重建流。
pub fn spawn_cpal_audio(
    sample_rate: u32,
    num_channels: u32,
    active_mask: Vec<bool>,
    buffer_size: cpal::BufferSize,
    device_name: Option<&str>,
    #[cfg(feature = "gpu")] use_gpu_synth: bool,
) -> Result<CpalAudioHandle, String> {
    let (cmd_tx, cmd_rx) = bounded::<AudioCommand>(AUDIO_CMD_CHANNEL_CAPACITY);
    let sample_position = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let duration_samples = Arc::new(AtomicU64::new(0));
    let stream_error = Arc::new(AtomicBool::new(false));

    let host = cpal::default_host();
    let device = match device_name {
        Some(name) => host
            .output_devices()
            .map_err(|e| format!("Failed to enumerate output devices: {e}"))?
            .find(|d| d.description().ok().is_some_and(|desc| desc.to_string() == name))
            .ok_or_else(|| format!("Output device not found: {name}"))?,
        None => host
            .default_output_device()
            .ok_or("No output device")?,
    };
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
        spawn_worker(sample_rate, engine.active_mask().to_vec(), channel_map)
            .map_err(|e| format!("Failed to spawn audio worker thread: {e}"))?;

    let (ring_producer, mut ring_consumer) = AudioRing::new(RING_CAPACITY).split();

    let renderer_state = RendererSharedState::new();
    let renderer_position = Arc::clone(&renderer_state.producer_sample_position);
    let renderer_playing = Arc::clone(&renderer_state.playing);
    let renderer_duration = Arc::clone(&renderer_state.duration_samples);
    let initialized = Arc::clone(&renderer_state.initialized);
    let reset_generation = Arc::clone(&renderer_state.reset_generation);

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
        #[cfg(feature = "gpu")]
        use_gpu_synth,
    )
    .map_err(|e| format!("Failed to spawn audio renderer thread: {e}"))?;

    let sp = Arc::clone(&sample_position);
    let pl = Arc::clone(&playing);
    let dur = Arc::clone(&duration_samples);
    let mut consumer_sample_position = 0u64;
    let mut acknowledged_generation = 0u64;

    // cpal 流错误回调：用 tracing 而不是 eprintln!，同时置 stream_error 标志，
    // UI 每帧查询后弹出对话框。错误不可逆，置位后不再清零。
    let stream_error_flag = Arc::clone(&stream_error);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
                    let generation = reset_generation.load(Ordering::Acquire);
                    if generation != acknowledged_generation {
                        ring_consumer.clear();
                        consumer_sample_position = renderer_position.load(Ordering::Acquire);
                        acknowledged_generation = generation;
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
            move |err| {
                tracing::error!("Audio stream error: {err}");
                stream_error_flag.store(true, Ordering::Release);
            },
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
            stream_error,
        },
        sample_rate,
        _stream: stream,
        shutdown,
    })
}
