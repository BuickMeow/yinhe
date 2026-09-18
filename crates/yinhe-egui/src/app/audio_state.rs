//! Audio subsystem state — fields related to the audio engine and playback.

use yinhe_audio::channel_layout::ChannelLayout;
use yinhe_types::{Interpolation, SynthEngine};

/// 影响音频引擎 spawn 的设置字段快照。
///
/// 两处使用：
/// - 设置对话框：区分「设置真的改了」与「只是关掉了设置窗口」（`show_viewport`
///   的返回值是"窗口关闭"而非"有修改"）；
/// - 引擎复用：`AudioState::engine_key` 记录引擎创建时的输入，跨文档复用前
///   要求其与当前设置一致。
///
/// 覆盖 `rebuild_audio_if_needed` / `resolve_sf_config` 的全部 spawn 输入：
/// 采样率、缓冲大小、输出设备、合成后端、全局音色库列表。
/// `xsynth_layers` 由 `SetLayerCount` 在线应用，不参与。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EngineSpawnKey {
    sample_rate: u32,
    buffer_size: u32,
    output_device_name: Option<String>,
    synth_engine: SynthEngine,
    interpolation: Interpolation,
    sf_entries: Vec<(String, String, bool)>,
}

impl EngineSpawnKey {
    pub(crate) fn of(settings: &crate::audio_settings::AudioSettings) -> Self {
        Self {
            sample_rate: settings.sample_rate,
            buffer_size: settings.buffer_size,
            output_device_name: settings.output_device_name.clone(),
            synth_engine: settings.synth_engine,
            interpolation: settings.interpolation,
            sf_entries: settings
                .global_sf_config
                .entries
                .iter()
                .map(|e| (e.path.clone(), e.name.clone(), e.enabled))
                .collect(),
        }
    }
}

/// 加载完成但等音频就绪（`audio_ready`）后才激活显示的文档。
pub(crate) struct PendingDocActivate {
    /// 文档在 `workspace.documents` 中的索引。
    pub idx: usize,
    /// 是否替换初始 Untitled（激活时删除 index 0）。
    pub replace_untitled: bool,
    /// "加载完成"toast 的文件名（激活时才弹）。
    pub file_name: String,
    /// 完成卡 detail（加载耗时）。
    pub detail: Option<String>,
}

/// 后台 teardown 回传的 insert 处理器批次（+ 归属文档索引）。
type InsertReturnBatch = (
    std::sync::mpsc::Receiver<Vec<Box<dyn yinhe_mixer::InsertProcessor>>>,
    usize,
);

/// All audio-engine-related state, extracted from `App` to reduce the God Object.
pub(crate) struct AudioState {
    /// The active audio backend handle (None if not initialized yet).
    pub handle: Option<yinhe_audio::CpalAudioHandle>,
    /// Which document index the audio engine is currently bound to.
    /// Used to detect document switches that require an audio rebuild.
    pub active_doc: Option<usize>,
    /// 引擎创建时的设置快照（`EngineSpawnKey::of`）。`None` = 无引擎。
    /// 文档切换时若与当前设置一致、且布局/音色库被覆盖，可复用引擎不重建。
    pub engine_key: Option<EngineSpawnKey>,
    /// 引擎创建时发送的音色库配置（源通道 → paths）。
    /// 文档复用时要求新文档需要的配置逐通道完全相同（避免重复加载/替换）。
    pub engine_sf_configs: Vec<(u8, Vec<String>)>,
    /// 当前引擎创建时使用的 `ChannelLayout` 快照。
    ///
    /// 用于在 `notify_notes_changed` / `notify_audio_model_changed` 里检测
    /// channel 激活状态是否翻转：若 `layout.covers_model(model)` 为 false，
    /// 说明 model 用到了引擎未激活的通道，必须 teardown + 重建引擎。
    /// 覆盖成立（含"占用减少"）则走便宜的 `UpdateNotes` / `ReloadNotes` 路径。
    ///
    /// `None` 表示引擎尚未 spawn，下一帧 `rebuild_audio_if_needed` 会用新 model
    /// 重新算 layout 并填入此字段。
    pub last_channel_layout: Option<ChannelLayout>,
    /// Last known sample position from the audio engine, and the instant we read it.
    /// Used to interpolate cursor position between callback updates.
    pub playback_anchor: Option<(u64, std::time::Instant)>,
    /// Set when Play/Resume is sent but the audio thread hasn't acknowledged yet.
    /// Ensures request_repaint() keeps firing until is_playing() returns true.
    pub pending_playback: bool,
    /// 诊断：Play/Resume 发出时刻（音频确认播放时打印等待时长）。
    pub pending_playback_since: Option<std::time::Instant>,
    /// teardown 后台线程回收的 insert 处理器（渲染线程 join 完成后回传）
    /// 与归属文档索引。`poll_insert_returns` 每帧尝试收取。
    pub pending_insert_returns: Option<InsertReturnBatch>,
    /// 待激活的文档（音频就绪后由 `poll_pending_doc_activate` 激活显示）。
    pub pending_doc_activate: Option<PendingDocActivate>,
    /// 设备切换对话框是否需要显示。
    ///
    /// 两种触发场景：
    /// - cpal `stream_error` 置位（设备热拔/驱动崩溃，流已死，必须切换）
    /// - 设备列表变更（插拔耳机，流还活着，可选切换）
    ///
    /// 用户选了新设备且 spawn 成功后置回 false；
    /// spawn 失败则保持 true 并把错误信息塞进 `device_switch_error`。
    pub device_switch_pending: bool,
    /// true = 流已死（stream_error），必须切换或退出，对话框不显示"保持当前设备"按钮。
    /// false = 设备列表变更（插拔耳机），流还活着，对话框显示"保持当前设备"按钮。
    pub device_switch_required: bool,
    /// 上一次设备切换 spawn 失败的错误信息（仅当 `device_switch_pending` 为 true 时有意义）。
    pub device_switch_error: Option<String>,
    /// 上一次轮询到的系统输出设备列表，用于检测设备插拔。
    /// 空 Vec 表示还没初始化过（首次轮询只记录、不触发对话框）。
    pub last_known_devices: Vec<String>,
    /// 音色库加载进度状态：期望加载的 port 总数（完成计数基准）与等待标志。
    /// `sf_pending = true` 时每帧 `poll_audio_progress` 轮询实际完成数。
    pub sf_total: usize,
    pub sf_pending: bool,
    /// 引擎重建/切换的等待 toast 起始时刻（`Some` = 进行中）。
    /// `rebuild_audio_if_needed` 建卡时置位，音频就绪/失败时收尾清空。
    pub engine_toast: Option<std::time::Instant>,
    /// 上一次轮询设备列表的时间。每秒轮询一次，避免每帧调用 cpal 枚举。
    pub last_device_poll: Option<std::time::Instant>,
    /// spawn_cpal_audio 失败的错误信息。Some 表示失败，rebuild_audio_if_needed
    /// 不再重试，直到用户切换设备/文档/设置清除它。
    pub spawn_error: Option<String>,
    /// `spawn_error` 归属的文档，`Some(idx)` 时仅该文档不重试，切到其他文档可重活。
    pub spawn_error_doc: Option<usize>,
    /// 后台 spawn 状态：spawn 是为哪个 doc 发起的（完成时对比 active_doc，
    /// 不一致则丢弃结果）与结果通道。Some = spawn 进行中。
    pub spawn_for_doc: Option<usize>,
    /// spawn 结果携带**发起时的设置快照**：完成时与当前设置比对，不一致
    /// （在飞期间用户改了插值/采样率/后端等）则丢弃重来，避免旧设置引擎被
    /// 安装并被误记为最新（导致新设置永不生效）。
    pub spawn_rx: Option<
        std::sync::mpsc::Receiver<Result<(yinhe_audio::CpalAudioHandle, EngineSpawnKey), String>>,
    >,
    /// 设备切换时保存的播放位置：spawn 完成后发送 Seek 恢复。
    pub spawn_restore_sample: Option<u64>,
    /// spawn 期间暂存的 layout 快照（flip 检测用），完成后写入 last_channel_layout。
    pub pending_layout: Option<ChannelLayout>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            handle: None,
            active_doc: None,
            engine_key: None,
            engine_sf_configs: Vec::new(),
            last_channel_layout: None,
            playback_anchor: None,
            pending_playback: false,
            pending_playback_since: None,
            pending_insert_returns: None,
            pending_doc_activate: None,
            device_switch_pending: false,
            device_switch_required: false,
            device_switch_error: None,
            last_known_devices: Vec::new(),
            sf_total: 0,
            sf_pending: false,
            engine_toast: None,
            last_device_poll: None,
            spawn_error: None,
            spawn_error_doc: None,
            spawn_for_doc: None,
            spawn_rx: None,
            spawn_restore_sample: None,
            pending_layout: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_settings::AudioSettings;

    /// 回归：EngineSpawnKey 必须覆盖 spawn 的全部设置输入——任一输入变化都要
    /// 让 key 不同。`interpolation` 曾漏出 key：改插值后关闭设置不重建、
    /// 设置永不生效（用户报告「关闭设置无法重置音频流」的确定性原因）。
    #[test]
    fn engine_spawn_key_covers_spawn_inputs() {
        let mk = AudioSettings::default;
        let k0 = EngineSpawnKey::of(&mk());

        let mut s = mk();
        s.sample_rate = s.sample_rate.wrapping_add(1);
        assert_ne!(
            EngineSpawnKey::of(&s),
            k0,
            "sample_rate 必须纳入 EngineSpawnKey"
        );

        let mut s = mk();
        s.buffer_size = s.buffer_size.wrapping_add(1);
        assert_ne!(
            EngineSpawnKey::of(&s),
            k0,
            "buffer_size 必须纳入 EngineSpawnKey"
        );

        let mut s = mk();
        s.output_device_name = Some("__none__".into());
        assert_ne!(
            EngineSpawnKey::of(&s),
            k0,
            "output_device_name 必须纳入 EngineSpawnKey"
        );

        let mut s = mk();
        s.synth_engine = match s.synth_engine {
            SynthEngine::XSynthCpu => SynthEngine::YinheGpu,
            _ => SynthEngine::XSynthCpu,
        };
        assert_ne!(
            EngineSpawnKey::of(&s),
            k0,
            "synth_engine 必须纳入 EngineSpawnKey"
        );

        let mut s = mk();
        s.interpolation = match s.interpolation {
            Interpolation::Nearest => Interpolation::Linear,
            _ => Interpolation::Nearest,
        };
        assert_ne!(
            EngineSpawnKey::of(&s),
            k0,
            "interpolation 必须纳入 EngineSpawnKey"
        );

        let mut s = mk();
        s.global_sf_config.entries.push(Default::default());
        assert_ne!(
            EngineSpawnKey::of(&s),
            k0,
            "global_sf_config 必须纳入 EngineSpawnKey"
        );
    }
}
