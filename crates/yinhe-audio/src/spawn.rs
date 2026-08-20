use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, bounded, unbounded};
use xsynth_core::soundfont::SoundfontBase;

use yinhe_core::YinModel;
use yinhe_mixer::{InsertProcessor, MasterParams, MeterReading, MixerParams, StripParams};
use yinhe_types::{KEY_COUNT, SegmentShape};

/// AR 自动化 lane 的 M/S 试听旁通集（跨线程共享，Empty 由 default 提供）。
pub type AmMsMap =
    std::collections::HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>;

use crate::audio_model::{PreparedModel, SortedCC};
use crate::audio_renderer::{RendererSharedState, spawn_renderer};
use crate::audio_ring::AudioRing;
use crate::channel::ChannelState;
use crate::channel_layout::ChannelLayout;

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
const _: () = assert!(
    RING_CAPACITY.is_power_of_two(),
    "RING_CAPACITY must be a power of two"
);

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
    /// AR 自动化 lane 的 M/S 试听旁通集：只更新 dispatch 动态掩码 + 异步 chase，
    /// 不重建模型、不 seek（与 SkipTracks 同机制）。
    SetAmMs {
        am_ms: Arc<AmMsMap>,
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
    /// 音符听觉预览：替换旧组的待触发音符；已在响的旧组音符继续响满自己的 gate
    /// （快速拖拽时若立即 NoteOff，音符会在被渲染前死掉，永远听不到）。
    /// 每组音符应用目标位置自动化状态并 NoteOn。停止由 PreviewStop（释放）负责。
    /// 预览走独立合成器（PreviewEngine）：不占主引擎 voice、不改播放状态。
    PreviewNotes {
        notes: Vec<PreviewNoteParams>,
    },
    /// 停止全部预览音（余音自然衰减完才停）。
    PreviewStop,
    /// 全量同步混音台参数（引擎 spawn / 工程加载后由 UI 推一次；Box 避免命令枚举过大）。
    SetMixerParams {
        params: Box<MixerParams>,
    },
    /// 更新某源通道（0..=255，A01..P16）的 strip 参数（推子拖动高频路径，幂等）。
    SetChannelStrip {
        channel: u8,
        params: StripParams,
    },
    /// 更新主输出参数。
    SetMasterParams {
        params: MasterParams,
    },
    /// 在 channel（None = master）insert 链的 slot 处插入处理器。
    InsertAdd {
        channel: Option<u8>,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    },
    /// 移除 channel（None = master）insert 链 slot 处的处理器（经 return 通道送回 UI 回收）。
    InsertRemove {
        channel: Option<u8>,
        slot: usize,
    },
    /// 替换 channel（None = master）insert 链 slot 处的处理器（插件 restart 用）。
    InsertReplace {
        channel: Option<u8>,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    },
    /// 安装/替换/移除某**乐器通道**（0 起，与 `TrackData::instrument_channel` 对齐）
    /// 上的 CLAP 乐器实例。`Some(processor)` = 安装/替换；`None` = 移除。
    /// 被替换/移除的旧处理器经乐器 return 通道送回 UI 线程 deactivate。
    SetInstrument {
        channel: u16,
        /// 处理器较大（含渲染缓冲），用 Box 避免枚举体积膨胀。
        processor: Option<Box<yinhe_clap::ClapProcessor>>,
    },
}

/// 单个预览音符的参数（tick 域，与编辑层一致；渲染线程转 sample 差喂预览引擎）。
pub struct PreviewNoteParams {
    /// 全局通道（port<<4 | channel）。
    pub channel: u8,
    pub key: u8,
    pub velocity: u8,
    /// 目标位置 tick：该处的自动化状态（volume/pan/PBS/Program 等）用于预览。
    pub target_tick: u32,
    /// 预览时长（tick）；0 = 持续音（等 `PreviewStop`）。
    pub duration_ticks: u32,
}

/// Handle used by the UI to control audio playback.
pub struct AudioHandle {
    pub(crate) cmd_tx: Sender<AudioCommand>,
    sample_position: Arc<AtomicU64>,
    /// 渲染线程已产出（推入 ring）的采样位置。UI 用它限制播放指示线
    /// 不能超过实际已渲染的音频（防止"准备播放"期间指示线空跑）。
    producer_sample_position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    duration_samples: Arc<AtomicU64>,
    /// 由 cpal 流错误回调置位。UI 每帧查询，若为 true 应弹窗提示用户重启。
    stream_error: Arc<AtomicBool>,
    /// 预览 Stop 快速路径标志：渲染线程每轮消费。命令通道满（渲染忙时
    /// `PreviewStop` 命令可能被丢弃）也保证松手即停。
    preview_stop_flag: Arc<AtomicBool>,
    /// 已完成加载的音色库 port 数（worker 每完成一 port +1）。
    sf_loaded: Arc<AtomicUsize>,
    /// latest-wins：轨道 mute/solo 掩码必达（命令通道满合并时不丢最新值）。
    pending_skip: Arc<Mutex<Option<Vec<bool>>>>,
    /// latest-wins：AM lane M/S 试听旁通集必达。
    pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>>,
    /// 混音台各 dense 通道的电平表读数端（引擎创建时收集，Arc 共享、随引擎重建换新）。
    mixer_channel_readings: Vec<MeterReading>,
    /// 主输出电平表读数端。
    mixer_master_reading: MeterReading,
    /// 渲染线程退回的 insert 处理器（插件 deactivate 必须在 UI/管理线程做）。
    insert_return_rx: crossbeam_channel::Receiver<Vec<Box<dyn InsertProcessor>>>,
    /// 渲染线程退回的乐器处理器（deactivate 同样必须在 UI/管理线程做）。
    instrument_return_rx: crossbeam_channel::Receiver<(u16, yinhe_clap::ClapProcessor)>,
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

    /// 请求立即停止全部预览音。不依赖命令通道（渲染忙时通道满会丢命令），
    /// 由渲染线程每轮消费标志，保证松手即停。
    pub fn request_preview_stop(&self) {
        self.preview_stop_flag.store(true, Ordering::Release);
    }

    /// latest-wins：轨道 mute/solo 掩码必达。命令通道满合并时 `SkipTracks`
    /// 命令可能被丢弃，本槽保证 renderer 每轮总能拿到**最新**掩码。
    pub fn set_skip_tracks(&self, skip: Vec<bool>) {
        *self.pending_skip.lock().unwrap_or_else(|e| e.into_inner()) = Some(skip);
    }

    /// latest-wins：AM lane M/S 旁通集必达（见 [`Self::set_skip_tracks`]）。
    pub fn set_am_ms(&self, am_ms: Arc<AmMsMap>) {
        *self.pending_am_ms.lock().unwrap_or_else(|e| e.into_inner()) = Some(am_ms);
    }

    /// 开始新预览组时清除待消费的停止请求，避免新组被渲染器当作
    /// "松手后的堆积旧组"跳过。
    pub fn clear_preview_stop(&self) {
        self.preview_stop_flag.store(false, Ordering::Release);
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }

    /// 渲染线程已推入 ring 的采样位置（producer），总是 `>= sample_position()`。
    ///
    /// 播放指示线以此为上限：Play/Seek 清空 ring 后、首个 chunk 渲染完成前
    /// 没有可听的音频，指示线必须停在当前位置等待，不能按墙钟空跑。
    pub fn producer_sample_position(&self) -> u64 {
        self.producer_sample_position.load(Ordering::Relaxed)
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

    /// 已完成加载的音色库 port 数（`LoadSoundFont` 结果回传时 +1）。
    pub fn sf_loaded_count(&self) -> usize {
        self.sf_loaded.load(Ordering::Relaxed)
    }

    /// 混音台 dense 通道数（= 引擎 compacted 通道数）。
    pub fn mixer_channel_count(&self) -> usize {
        self.mixer_channel_readings.len()
    }

    /// 读某 dense 通道的电平（L, R 峰值，0 起）。
    pub fn channel_meter_read(&self, dense: usize) -> Option<(f32, f32)> {
        self.mixer_channel_readings.get(dense).map(|r| r.read())
    }

    /// 读主输出电平。
    pub fn master_meter_read(&self) -> (f32, f32) {
        self.mixer_master_reading.read()
    }

    /// 取回渲染线程退回的 insert 处理器（每帧轮询；插件 deactivate 在 UI 线程做）。
    pub fn drain_insert_returns(&self) -> Vec<Box<dyn InsertProcessor>> {
        let mut out = Vec::new();
        while let Ok(mut batch) = self.insert_return_rx.try_recv() {
            out.append(&mut batch);
        }
        out
    }

    /// 取回渲染线程退回的乐器处理器（每帧轮询；deactivate 在 UI 线程做）。
    pub fn drain_instrument_returns(&self) -> Vec<(u16, yinhe_clap::ClapProcessor)> {
        let mut out = Vec::new();
        while let Ok(p) = self.instrument_return_rx.try_recv() {
            out.push(p);
        }
        out
    }

    /// 克隆退回通道的接收端：teardown 时先克隆、再 drop 句柄（join 渲染线程），
    /// 之后仍能从克隆的接收端取回渲染线程关机时退回的全部处理器。
    pub fn clone_insert_return_rx(
        &self,
    ) -> crossbeam_channel::Receiver<Vec<Box<dyn InsertProcessor>>> {
        self.insert_return_rx.clone()
    }

    /// 克隆乐器退回通道接收端（同 `clone_insert_return_rx` 的用途）。
    pub fn clone_instrument_return_rx(
        &self,
    ) -> crossbeam_channel::Receiver<(u16, yinhe_clap::ClapProcessor)> {
        self.instrument_return_rx.clone()
    }
}

/// Result of spawning the audio backend.
pub struct CpalAudioHandle {
    pub handle: AudioHandle,
    pub sample_rate: u32,
    /// 共享给 cpal 错误回调（采样率变化时恢复流）。用 Arc<Mutex<Option>> 而非裸
    /// Stream：错误回调在流创建之前就注册，只能通过共享句柄访问。回调只持 Weak，
    /// handle drop 时 Arc 归零 → Stream 正常释放，不形成循环引用。
    pub(crate) _stream: Arc<Mutex<Option<cpal::Stream>>>,
    /// 设置为 true 时通知 renderer 线程退出。
    pub(crate) shutdown: Arc<AtomicBool>,
    /// renderer 线程的 JoinHandle。
    ///
    /// Drop 时同步 join，确保 AudioEngine（含 rayon 线程池）完全释放。
    /// 不 join 会导致反复 teardown+rebuild（如蓝牙耳机抖动触发设备切换）时
    /// rayon 工作线程累积，最终触发 EAGAIN (code 35) panic。
    pub(crate) renderer_handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for CpalAudioHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // 同步 join renderer 线程。WAKE_SLEEP=1ms，renderer 最多 1ms 后退出，
        // join 阻塞时间可忽略。确保 AudioEngine → ChannelGroup → 2 个 rayon::ThreadPool
        // 在返回前被 drop，避免线程泄漏。
        if let Some(handle) = self.renderer_handle.take()
            && let Err(payload) = handle.join()
        {
            tracing::error!("Audio renderer thread panicked during shutdown: {payload:?}");
        }
    }
}

impl CpalAudioHandle {
    /// Notify the audio thread that the MIDI model has changed (full rebuild:
    /// cc_events + audible_notes + chase). Use for automation edits / undo / redo.
    /// AM lane M/S 旁通独立于模型存在，由 [`Self::set_am_ms`] 维护。
    pub fn reload_notes(&self, model: Arc<YinModel>) {
        self.handle.send(AudioCommand::ReloadNotes { model });
    }

    /// 更新 AR 自动化 lane 的 M/S 试听旁通集：只换动态掩码 + 异步 chase，
    /// 不重建模型、不 seek——与 SkipTracks 对轨道 mute 的处理同机制。
    /// 走 latest-wins 槽（通道满也不丢最新状态）。
    pub fn set_am_ms(&self, am_ms: Arc<AmMsMap>) {
        self.handle.set_am_ms(am_ms);
    }

    /// 轨道 mute/solo 掩码（latest-wins 槽，见 [`Self::set_am_ms`]）。
    pub fn set_skip_tracks(&self, skip: Vec<bool>) {
        self.handle.set_skip_tracks(skip);
    }

    /// Notify the audio thread that only notes have changed (no automation, no
    /// chase). Use for pure note edits — keeps current channel state intact.
    pub fn update_notes(&self, model: Arc<YinModel>) {
        self.handle.send(AudioCommand::UpdateNotes { model });
    }
}

/// Analyse a YinModel and return the `ChannelLayout` (active_mask + channel_map).
///
/// A channel is "active" if any note with vel>1 lives on it, OR any
/// non-note control event is present on the owning track.
///
/// 返回的 `ChannelLayout` 在 `AudioEngine` 创建时定型，之后不可变。
/// 若 model 结构变化（增减音轨、改 channel/port），必须 teardown + 重建引擎。
pub fn channels_for_model(model: &YinModel) -> ChannelLayout {
    ChannelLayout::from_model(model)
}

/// Internal command sent from the renderer thread to the worker thread.
pub(crate) enum WorkerCmd {
    /// Full prepare: cc_events + audible_notes + duration (LoadModel / ReloadNotes).
    PrepareModel(Arc<YinModel>, u32),
    /// Notes-only prepare: audible_notes + duration (UpdateNotes). No cc_events rebuild.
    PrepareNotes(Arc<YinModel>),
    /// Compute channel-state snapshot at `target_tick` by **querying the model
    /// automation lanes**（每 lane 二分 + 曲线实时插值，不再从曲首逐条累计）。
    /// `generation` matches `AudioEngine::chase_generation` so the renderer can
    /// discard stale results after a PrepareModel replaces the model.
    PrepareChase {
        model: Arc<YinModel>,
        target_tick: u32,
        generation: u64,
        /// 当前 skip_track 快照，chase 时跳过 mute 轨道的 CC。
        skip_mask: Vec<bool>,
        /// AR 自动化 lane 的 M/S 试听旁通，chase 时同步过滤。
        am_ms: Arc<AmMsMap>,
    },
    LoadSoundFont {
        port: u8,
        paths: Vec<String>,
        dense_channels: Vec<u32>,
    },
}

pub(crate) enum WorkerResult {
    PreparedModel(PreparedModel),
    /// Result of `PrepareNotes` — 只包含 dirty 桶的 audible_notes 增量 + duration + model refs。
    /// `audible_delta[key] == None` 表示该桶未变化，音频线程保留旧桶和旧 cursor。
    PreparedNotes {
        model: crate::audio_model::AudioModel,
        yin_model: Arc<YinModel>,
        audible_delta: crate::audio_model::AudibleDelta,
        duration_samples: u64,
    },
    /// Result of `PrepareChase` — 256-channel state snapshot.
    /// `Some(state)` = 该通道在目标位置有生效事件（无事件通道不触碰）。
    ChaseResult {
        states: Box<[Option<ChannelState>; 256]>,
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
            // 上次 prepare 时的 note_revisions 快照：对比当前 model 得出 dirty 桶，
            // 使 PrepareNotes 只重建变化的 key 桶（1 亿音符工程编辑不再全量扫描）。
            // None = 尚未同步过（首次 PrepareNotes 全量）。
            let mut last_synced_revisions: Option<[u64; KEY_COUNT]> = None;
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
                        );
                        last_synced_revisions = Some(latest.note_revisions);
                        let _ = result_tx.send(WorkerResult::PreparedModel(prepared));
                        // 构建 audible_notes/cc_events 的临时内存已释放：归还空闲页，
                        // 避免 RSS 跨阶段累积（大工程峰值内存只涨不跌的根因之一）。
                        yinhe_memtrace::purge_free_pages();
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
                        // 对比 note_revisions 算 dirty 桶：只重建变化的 key 桶。
                        // rebuild() 会 bump 全部 KEY_COUNT 个 revision（全量变化），
                        // 与模型侧 dirty 语义一致。
                        let dirty: [bool; KEY_COUNT] = match &last_synced_revisions {
                            Some(prev) => {
                                core::array::from_fn(|k| prev[k] != latest.note_revisions[k])
                            }
                            None => [true; KEY_COUNT], // 首次同步：全量
                        };
                        let (audio_model, yin_model, audible_delta, duration_samples) =
                            crate::prepare_model::prepare_notes_dirty(&latest, sample_rate, &dirty);
                        last_synced_revisions = Some(latest.note_revisions);
                        let _ = result_tx.send(WorkerResult::PreparedNotes {
                            model: audio_model,
                            yin_model,
                            audible_delta,
                            duration_samples,
                        });
                        yinhe_memtrace::purge_free_pages();
                    }
                    WorkerCmd::PrepareChase {
                        model,
                        target_tick,
                        generation,
                        skip_mask,
                        am_ms,
                    } => {
                        // 合并连续 PrepareChase，只保留最新（同 generation 或不同 generation 都只留最新）
                        let mut latest_model = model;
                        let mut latest_target = target_tick;
                        let mut latest_gen = generation;
                        let mut latest_mask = skip_mask;
                        let mut latest_am_ms = am_ms;
                        while let Ok(next) = cmd_rx.try_recv() {
                            match next {
                                WorkerCmd::PrepareChase {
                                    model,
                                    target_tick,
                                    generation,
                                    skip_mask,
                                    am_ms,
                                } => {
                                    latest_model = model;
                                    latest_target = target_tick;
                                    latest_gen = generation;
                                    latest_mask = skip_mask;
                                    latest_am_ms = am_ms;
                                }
                                other => {
                                    pending.push_back(other);
                                }
                            }
                        }
                        let states = compute_chase_states(
                            &latest_model,
                            latest_target,
                            &latest_mask,
                            &latest_am_ms,
                        );
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

/// 在 worker 线程上**查询式**构建 256 通道状态快照：不再从曲首逐条累计
/// cc_events，而是直接查询模型自动化 lane——每个 lane 二分定位目标位置的
/// 生效值（Linear/Curve 段实时插值，与 density 无关的真实值），PC 取最后一条。
///
/// 结果由 renderer 的 `apply_chase_result` 直接 `send_to`。
/// `skip_mask`：mute 的音轨的 lane/PC 不参与 chase，不影响同 channel 其他轨道。
/// `am_ms`：AR 自动化 lane 的 M/S 试听旁通（与播放事件流同规则过滤）。
///
/// 复杂度：O(所有未 mute 音轨的 lane 数 × log(lane 事件数))，与曲长无关。
fn compute_chase_states(
    model: &YinModel,
    target_tick: u32,
    skip_mask: &[bool],
    am_ms: &crate::spawn::AmMsMap,
) -> Box<[Option<ChannelState>; 256]> {
    use crate::audio_model::{emit_automation_event, push_program_change};

    // 每通道收集目标位置生效事件（tick 排序后顺序 apply，多 track 同 channel 自动合并）。
    let mut events: [Vec<SortedCC>; 256] = std::array::from_fn(|_| Vec::new());

    for (track_idx, track) in model.tracks.iter().enumerate() {
        if skip_mask.get(track_idx).copied().unwrap_or(false) {
            continue; // mute 轨道不参与 chase
        }
        let ch = track.global_channel() as usize;
        if ch >= 256 {
            continue;
        }
        let out = &mut events[ch];
        // 与播放事件流一致：该音轨是否有任意 lane solo（S 作用域 = 音轨内）。
        let track_has_solo = track.automation_lanes.iter().any(|l| {
            am_ms
                .get(&(track_idx as u16, l.target.clone()))
                .is_some_and(|s| s.solo)
        });
        for (lane_idx, lane) in track.automation_lanes.iter().enumerate() {
            if crate::audio_model::automation_lane_skipped(
                am_ms,
                track_idx as u16,
                lane,
                track_has_solo,
            ) {
                continue;
            }
            if let Some((value, tick)) = lane_value_at(lane, target_tick) {
                emit_automation_event(
                    &lane.target,
                    value,
                    tick,
                    ch as u32,
                    track_idx as u16,
                    lane_idx as u16,
                    out,
                );
            }
        }
        for pc in &track.program_change {
            if pc.tick < target_tick {
                push_program_change(pc, ch as u32, track_idx as u16, out);
            }
        }
    }

    let mut states: Box<[Option<ChannelState>; 256]> = Box::new(std::array::from_fn(|_| None));
    for (ch, mut evs) in events.into_iter().enumerate() {
        if evs.is_empty() {
            // 无事件：不触碰该通道（chase 应用时跳过），
            // 避免把 mute 轨的通道控制器重置回默认值。
            continue;
        }
        // 与播放事件流一致：同 tick 参数类（RPN/CC/PC）先于 PitchBendValue。
        evs.sort_by_key(|e| (e.tick, crate::audio_model::dispatch_priority(&e.event)));
        let mut state = ChannelState::default();
        for e in &evs {
            state.apply(&e.event);
        }
        states[ch] = Some(state);
    }
    states
}

/// 查询 lane 在 `target` 位置的生效值（模型 lane 声明为按 tick 排序）。
///
/// - Step：最后一条 `tick < target` 的事件值（保持语义）。
/// - Linear/Curve：target 落在段内时**实时插值**（真实值，与 flatten 的 density 无关）；
///   target == 下一事件 tick 时取曲线终点值（连续），Step 则保持上一值。
/// - 返回 `(value, tick)`；tick 用于与播放事件流一致的排序（曲线插值用 target）。
fn lane_value_at(lane: &yinhe_types::AutomationLane, target: u32) -> Option<(f32, u32)> {
    let events = &lane.events;
    let idx = events.partition_point(|e| e.tick < target);
    if idx == 0 {
        return None; // target 之前没有任何事件
    }
    let e = &events[idx - 1];
    if idx < events.len() {
        let next = &events[idx];
        if !matches!(e.shape, SegmentShape::Step) && target < next.tick {
            // 曲线段内：插值真实值（事件 tick 用 target，排序时位于本段生效点）
            let frac = (target - e.tick) as f32 / (next.tick - e.tick) as f32;
            let v = e.value + (next.value - e.value) * e.shape.interpolate(frac);
            return Some((v, target));
        }
        if !matches!(e.shape, SegmentShape::Step) && target == next.tick {
            // 曲线终点：连续到达 next.value（下一事件 tick == target 由 dispatch 处理，
            // chase 提供同值兜底，chase_skip 会跳过已 dispatch 的控制器）
            return Some((next.value, next.tick));
        }
    }
    Some((e.value, e.tick))
}

/// 测试用包装：暴露 `compute_chase_states` 给单元测试。
#[cfg(test)]
pub(crate) fn compute_chase_states_for_test(
    model: &YinModel,
    target_tick: u32,
    skip_mask: &[bool],
) -> Box<[Option<ChannelState>; 256]> {
    compute_chase_states(model, target_tick, skip_mask, &crate::spawn::AmMsMap::new())
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

/// 协商采样率：请求值不在设备任何 f32 输出配置的支持范围内时，
/// 回退到设备默认采样率。枚举失败时不做判断，交给 cpal 建流时报错。
fn negotiate_sample_rate(device: &cpal::Device, requested: u32, device_default: u32) -> u32 {
    let supported = match device.supported_output_configs() {
        Ok(configs) => configs
            .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
            .any(|c| requested >= c.min_sample_rate() && requested <= c.max_sample_rate()),
        Err(_) => return requested,
    };
    if supported {
        requested
    } else {
        tracing::warn!(
            "Sample rate {requested} Hz not supported by output device, \
             falling back to device default {device_default} Hz"
        );
        device_default
    }
}

/// 协商缓冲区：Fixed(n) 超出设备支持范围时钳制到 [min, max]。
/// 蓝牙设备常见 max 远小于内置扬声器（如 1024），不钳制会直接建流失败。
fn negotiate_buffer_size(
    requested: cpal::BufferSize,
    supported: &cpal::SupportedBufferSize,
) -> cpal::BufferSize {
    let n = match requested {
        cpal::BufferSize::Fixed(n) => n,
        default => return default,
    };
    match supported {
        cpal::SupportedBufferSize::Range { min, max } => {
            let clamped = n.clamp(*min, *max);
            if clamped != n {
                tracing::warn!(
                    "Buffer size {n} out of device range {min}..={max}, clamped to {clamped}"
                );
            }
            cpal::BufferSize::Fixed(clamped)
        }
        cpal::SupportedBufferSize::Unknown => requested,
    }
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
    layout: ChannelLayout,
    buffer_size: cpal::BufferSize,
    device_name: Option<&str>,
    #[cfg(feature = "gpu")] use_gpu_synth: bool,
) -> Result<CpalAudioHandle, String> {
    let (cmd_tx, cmd_rx) = bounded::<AudioCommand>(AUDIO_CMD_CHANNEL_CAPACITY);
    let sample_position = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let duration_samples = Arc::new(AtomicU64::new(0));
    let stream_error = Arc::new(AtomicBool::new(false));
    let preview_stop_flag = Arc::new(AtomicBool::new(false));

    let host = cpal::default_host();
    let device = match device_name {
        Some(name) => host
            .output_devices()
            .map_err(|e| format!("Failed to enumerate output devices: {e}"))?
            .find(|d| {
                d.description()
                    .ok()
                    .is_some_and(|desc| desc.to_string() == name)
            })
            .ok_or_else(|| format!("Output device not found: {name}"))?,
        None => host.default_output_device().ok_or("No output device")?,
    };
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    // 强制立体声：xsynth 是立体声合成器，ring buffer 和位置计算也硬编码 2 声道。
    // 不取设备默认声道数，避免多声道设备（HDMI/聚合设备 6/8 声道）导致声道映射错乱。
    let channels = STEREO_CHANNELS;

    // 设备能力协商：蓝牙设备（尤其 HFP 模式）的缓冲区/采样率范围往往比
    // 内置扬声器窄很多（实测小米开放式耳机缓冲区仅 14..=1024 帧）。
    // 直接把用户设置塞给 build_output_stream 会失败，而失败只进日志、
    // UI 无感知，表现为"播放键失灵、时间线冻结"。这里先钳制/回退。
    let sample_rate = negotiate_sample_rate(&device, sample_rate, supported.sample_rate());
    let buffer_size = negotiate_buffer_size(buffer_size, supported.buffer_size());

    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate,
        buffer_size,
    };
    // cpal 回调每次请求的帧数（约等于流缓冲）：预览时 ring 目标必须 ≥ 它，
    // 否则回调只能 pop 出部分帧、其余填静音 → 每个回调周期后半段静音 → 声音
    // "一闪一闪"地卡顿。Default（设备默认缓冲）时帧数未知，保守按 1024 帧估算
    // （常见设备与蓝牙 HFP 上限均 ≤ 1024）。
    let callback_frames = match config.buffer_size {
        cpal::BufferSize::Fixed(n) => n as usize,
        cpal::BufferSize::Default => 1024,
    };

    // catch_unwind 包住 AudioEngine::new + PreviewEngine::new：两者内部都调用
    // ChannelGroup::new，其内部 `rayon::ThreadPoolBuilder::build().unwrap()` 在
    // 进程线程数超限时会 panic（macOS EAGAIN / code 35）。捕获后返回 Err，
    // 让上层弹对话框而不是 abort 进程。注意：panic=abort 配置下 catch_unwind
    // 无效，根 Cargo.toml 必须保持 panic=unwind。
    let (engine, preview_engine) =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let engine = crate::engine::AudioEngine::new(sample_rate, layout);
            let preview = crate::preview_engine::PreviewEngine::new(
                &engine.channel_layout,
                engine.sample_rate,
            );
            (engine, preview)
        })) {
            Ok(pair) => pair,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                return Err(format!("Audio engine initialization failed: {msg}"));
            }
        };
    // 混音台电平表读数端：引擎即将 move 进渲染线程，先把 Arc 读数端收集给 UI。
    // 引擎生命周期内通道数不变（layout 冻结），resize 只重建缓冲不换 meter，
    // 这些读数端在整个引擎生命周期内有效。
    let mixer_channel_readings: Vec<MeterReading> = (0..engine.mixer.channel_count())
        .filter_map(|i| engine.mixer.channel_meter_reading(i))
        .collect();
    let mixer_master_reading = engine.mixer.master_meter_reading();
    // 渲染线程 → UI 的 insert 处理器退回通道（替换/移除/拆除时回收 deactivate）。
    let (insert_return_tx, insert_return_rx) = unbounded::<Vec<Box<dyn InsertProcessor>>>();
    // 渲染线程 → UI 的乐器处理器退回通道（替换/移除/拆除时回收 deactivate）。
    let (instrument_return_tx, instrument_return_rx) =
        unbounded::<(u16, yinhe_clap::ClapProcessor)>();

    let (worker_tx, prepared_rx) = spawn_worker(sample_rate)
        .map_err(|e| format!("Failed to spawn audio worker thread: {e}"))?;

    let (ring_producer, mut ring_consumer) = AudioRing::new(RING_CAPACITY).split();

    let renderer_state = RendererSharedState::new();
    // UI 播放指示线的上限：渲染器已推入 ring 的采样位置（producer）。
    let handle_producer_position = Arc::clone(&renderer_state.producer_sample_position);
    let renderer_playing = Arc::clone(&renderer_state.playing);
    let renderer_duration = Arc::clone(&renderer_state.duration_samples);
    let reset_generation = Arc::clone(&renderer_state.reset_generation);
    // 清空边界：ack 时丢弃边界前的旧音频、保留新音频（竞态安全清空）。
    let clear_base_sample = Arc::clone(&renderer_state.clear_base_sample);
    let clear_ring_write = Arc::clone(&renderer_state.clear_ring_write);
    // 音色库完成计数（renderer_state 即将 move 进 renderer，先 clone 给 handle）。
    let handle_sf_loaded = Arc::clone(&renderer_state.sf_loaded);

    let shutdown = Arc::new(AtomicBool::new(false));
    // latest-wins 槽：M/S 掩码必达（UI 写、renderer 每轮消费最新值）。
    let pending_skip: Arc<Mutex<Option<Vec<bool>>>> = Arc::new(Mutex::new(None));
    let pending_am_ms: Arc<Mutex<Option<Arc<AmMsMap>>>> = Arc::new(Mutex::new(None));
    let renderer_handle = spawn_renderer(
        engine,
        preview_engine,
        ring_producer,
        renderer_state,
        channels as u16,
        cmd_rx,
        worker_tx,
        prepared_rx,
        Arc::clone(&shutdown),
        Arc::clone(&preview_stop_flag),
        Arc::clone(&sample_position),
        Arc::clone(&pending_skip),
        Arc::clone(&pending_am_ms),
        callback_frames,
        insert_return_tx,
        instrument_return_tx,
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
    //
    // 例外：macOS 上 cpal 0.18 会给设备注册全局采样率监听，任何其他 app 改变
    // 设备采样率（视频/音乐播放、蓝牙 A2DP↔HFP 切换）都会暂停本流并报
    // "Device sample rate changed"。设备没坏，CoreAudio 会自动做采样率转换
    //（SRC），恢复流即可，不该弹"重新选择设备"。真正不可恢复的错误（设备
    // 移除、驱动崩溃等）才置位 stream_error。
    let stream_error_flag = Arc::clone(&stream_error);
    // 流在回调注册之后才创建，用 Arc<Mutex<Option>> 共享给错误回调；
    // 回调只持 Weak，handle 释放时不会形成循环引用。
    let stream_holder: Arc<Mutex<Option<cpal::Stream>>> = Arc::new(Mutex::new(None));
    let stream_holder_weak = Arc::downgrade(&stream_holder);
    let stream = match device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Audio, || {
                let generation = reset_generation.load(Ordering::Acquire);
                if generation != acknowledged_generation {
                    // 丢弃边界前的旧音频，保留边界后渲染器已推入的新音频
                    // （整体 clear 会把新播放位置的开头一起丢掉）。
                    ring_consumer.discard_before(clear_ring_write.load(Ordering::Acquire));
                    consumer_sample_position = clear_base_sample.load(Ordering::Acquire);
                    acknowledged_generation = generation;
                }

                // ring 有音频就读，没有就填静音（含未加载模型时的纯预览场景）。
                let popped = ring_consumer.pop_into(data);
                if popped < data.len() {
                    data[popped..].fill(0.0);
                }
                consumer_sample_position =
                    consumer_sample_position.saturating_add((popped / STEREO_CHANNELS) as u64);

                sp.store(consumer_sample_position, Ordering::Relaxed);
                pl.store(renderer_playing.load(Ordering::Relaxed), Ordering::Relaxed);
                dur.store(renderer_duration.load(Ordering::Relaxed), Ordering::Relaxed);
            })
        },
        move |err| {
            let is_rate_change = err.kind() == cpal::ErrorKind::StreamInvalidated
                && err
                    .message()
                    .is_some_and(|m| m.contains("sample rate changed"));
            if is_rate_change {
                // cpal 已把流暂停；设备还活着，CoreAudio 会做 SRC，恢复即可。
                tracing::warn!("Audio stream paused by device sample rate change, resuming: {err}");
                if let Some(holder) = stream_holder_weak.upgrade() {
                    let guard = holder.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(stream) = guard.as_ref()
                        && let Err(e) = stream.play()
                    {
                        tracing::error!("Failed to resume audio stream: {e}");
                        stream_error_flag.store(true, Ordering::Release);
                    }
                }
            } else {
                tracing::error!("Audio stream error: {err}");
                stream_error_flag.store(true, Ordering::Release);
            }
        },
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            // build stream 失败 —— 清理已 spawn 的 renderer 线程，避免泄漏
            shutdown.store(true, Ordering::Release);
            let _ = renderer_handle.join();
            return Err(format!("Failed to build stream: {e}"));
        }
    };
    // 流创建完成，放入共享句柄供错误回调恢复使用；启动也走同一句柄
    // （store→play 顺序：若启动前就发生采样率变化，错误回调也能从句柄拿到流恢复）。
    *stream_holder.lock().unwrap_or_else(|e| e.into_inner()) = Some(stream);
    match stream_holder
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.play())
    {
        Some(Ok(())) => {}
        Some(Err(e)) => {
            shutdown.store(true, Ordering::Release);
            let _ = renderer_handle.join();
            return Err(format!("Failed to start stream: {e}"));
        }
        None => return Err("Audio stream unexpectedly missing after build".to_string()),
    }

    Ok(CpalAudioHandle {
        handle: AudioHandle {
            cmd_tx,
            sample_position,
            producer_sample_position: handle_producer_position,
            playing,
            duration_samples,
            stream_error,
            preview_stop_flag,
            sf_loaded: handle_sf_loaded,
            pending_skip,
            pending_am_ms,
            mixer_channel_readings,
            mixer_master_reading,
            insert_return_rx,
            instrument_return_rx,
        },
        sample_rate,
        _stream: stream_holder,
        shutdown,
        renderer_handle: Some(renderer_handle),
    })
}
