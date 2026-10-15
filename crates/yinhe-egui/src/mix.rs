//! MIX 模式：混音台界面。
//!
//! 通道条按**源 MIDI 通道**（A01..P16）组织：每条 strip 对应一个被工程音轨
//! 使用的通道，音频域的增益/声像/静音/独奏/insert 全部生效（多条轨道共享
//! 同一通道时共享一条 strip，UI 上标注使用该通道的轨道名）。
//!
//! 数据流：
//! - 参数读写：`doc.mixer`（持久化，mixer_mut 标脏）→ `AudioCommand::Set*` 推引擎；
//! - 电平表：渲染线程 → `AudioHandle` 的 MeterReading → 这里做 UI 侧峰值衰减；
//! - insert 生命周期：`MixerRack`（见 rack.rs）。

#[cfg(target_os = "macos")]
pub(crate) mod gui_window;
pub(crate) mod instrument_rack;
pub(crate) mod param_panel;
pub(crate) mod plugin_instance;
pub(crate) mod rack;
mod strip;
mod tool_bar;

use eframe::egui;
use rust_i18n::t;
use yinhe_audio::InsertTarget;
use yinhe_audio::channel_layout::ChannelLayout;
use yinhe_mixer::{MasterParams, StripParams};

use self::plugin_instance::PluginEntry;
use crate::plugin_scan::ScanProgress;

use crate::app::App;
use crate::widgets::meter::{db_to_gain, gain_to_db};

pub(crate) use instrument_rack::InstrumentRack;
pub(crate) use param_panel::ParamPanel;
pub(crate) use rack::MixerRack;

/// 源通道总数（16 port × 16 通道）。
const SOURCE_CHANNELS: usize = yinhe_mixer::CHANNEL_COUNT;

/// 源通道号 → 显示标签（0 → "MIDI-A01"，255 → "MIDI-P16"）。
pub(crate) fn channel_label(ch: u8) -> String {
    let port = (b'A' + ch / 16) as char;
    format!("MIDI-{}{:02}", port, ch % 16 + 1)
}

/// 音频通道号 → 显示标签（0 → "Audio-01"）。
pub(crate) fn audio_label(ch: u16) -> String {
    format!("Audio-{:02}", u32::from(ch) + 1)
}

/// 电平表回落速度（dB / 秒）：行业惯用约 20 dB/s（PPM 弹道：瞬时起、匀速落）。
const METER_DB_PER_SEC: f32 = 20.0;
/// 峰值保持时间（秒）：超过后峰值线开始按同样速度回落。
const METER_HOLD_SECS: f32 = 1.0;

/// 单个电平表的 UI 侧弹道状态（L/R 各一路：当前电平 + 峰值保持）。
///
/// 起跳瞬时跟随原始峰值，回落在 **dB 域**匀速（线性域递减在接近 0 时会显得忽快忽慢，
/// 非行业惯用）。峰值保持线先停留 `METER_HOLD_SECS`，之后同样按 dB 速度回落。
#[derive(Clone, Copy, Default)]
pub(crate) struct MeterState {
    level: [f32; 2],
    hold: [f32; 2],
    hold_age: [f32; 2],
}

impl MeterState {
    fn update(&mut self, raw: (f32, f32), dt: f32) {
        let drop_db = METER_DB_PER_SEC * dt;
        let raw = [raw.0, raw.1];
        for (i, &r) in raw.iter().enumerate() {
            let prev_db = gain_to_db(self.level[i]);
            let raw_db = gain_to_db(r);
            let new_db = if raw_db >= prev_db {
                raw_db
            } else {
                (prev_db - drop_db).max(raw_db)
            };
            self.level[i] = db_to_gain(new_db);
            if r >= self.hold[i] {
                self.hold[i] = r;
                self.hold_age[i] = 0.0;
            } else {
                self.hold_age[i] += dt;
                if self.hold_age[i] > METER_HOLD_SECS {
                    self.hold[i] = db_to_gain(
                        (gain_to_db(self.hold[i]) - drop_db).max(gain_to_db(self.level[i])),
                    );
                }
            }
        }
    }

    fn level(&self) -> (f32, f32) {
        (self.level[0], self.level[1])
    }

    fn hold(&self) -> (f32, f32) {
        (self.hold[0], self.hold[1])
    }
}

/// MIX 界面的非持久化 UI 状态。
#[derive(Default)]
pub(crate) struct MixUiState {
    /// 各 dense 通道的电平表弹道状态，UI 侧衰减用。
    smoothed: Vec<MeterState>,
    smoothed_master: MeterState,
    /// 本帧总输出电平显示值 `((level L,R), (hold L,R))`：main_loop 每帧只更新
    /// 一次（[`update_master_meter`]），混音台 master 条与走带栏时间码表共用，
    /// 避免同一帧两次衰减。
    pub(crate) master_display: ((f32, f32), (f32, f32)),
    /// 总线条电平表弹道状态（key = "bus{n}"，增删总线时旧键闲置无害）。
    smoothed_buses: std::collections::HashMap<String, MeterState>,
    /// 每轨力度表弹道状态（索引 = 音轨；仅播放时有值）。
    smoothed_tracks: Vec<MeterState>,
    /// 本帧每轨力度表显示值 `(level, hold)`，main_loop 每帧更新（仅播放时非零）。
    pub(crate) track_display: Vec<(f32, f32)>,
    /// 插件扫描结果（后台线程填充；None = 尚未完成首次扫描）。
    pub(crate) scanned: Option<Vec<PluginEntry>>,
    /// 扫描中失败的包数量（诊断展示）。
    pub(crate) scan_errors: usize,
    /// 扫描 worker 进度接收端（Some = 扫描进行中）。
    pub(crate) scan_rx: Option<std::sync::mpsc::Receiver<ScanProgress>>,
    /// 后台扫描进行中（UI 状态展示用）。
    pub(crate) scan_in_progress: bool,
    /// 插件选择器打开目标（None = 关闭）。
    pub(crate) picker_for: Option<InsertTarget>,
    /// 发送面板打开目标（None = 关闭）。
    pub(crate) sends_for: Option<u8>,
    /// XSynth 配置窗口目标（源通道；None = 关闭）。
    pub(crate) xsynth_config_for: Option<u8>,
    /// 乐器选择器目标：MIDI 通道（挂载点）；None = 未打开。
    pub(crate) instrument_picker_for: Option<u8>,
    pub(crate) picker_filter: String,
    /// 插件参数面板（同一时间至多一个）。
    pub(crate) param_panel: Option<ParamPanel>,
    /// 通道 AR（MIX 主区域）面板的通道电平弹道状态（dense 索引）。
    /// 与混音台 strip 的 `smoothed` **相互独立**：各自读取音频线程原始
    /// meter 并施加弹道，互不干扰（同源数据、两条 UI 衰减轨迹）。
    pub(crate) ca_smoothed: Vec<MeterState>,
}

/// 一帧内 strip 交互产出的动作（渲染完统一应用，避开借用冲突）。
pub(crate) enum MixAction {
    SetStrip {
        channel: u8,
        params: StripParams,
    },
    /// 更新某音频通道的 strip 参数（推子/声像/M/S 高频路径）。
    SetAudioStrip {
        channel: u16,
        params: StripParams,
    },
    SetMaster {
        params: MasterParams,
    },
    OpenPicker {
        target: InsertTarget,
    },
    AddInsert {
        target: InsertTarget,
        plugin: PluginEntry,
    },
    BypassInsert {
        target: InsertTarget,
        slot: usize,
        bypassed: bool,
    },
    ToggleGui {
        target: InsertTarget,
        slot: usize,
    },
    RemoveInsert {
        target: InsertTarget,
        slot: usize,
    },
    /// 新增一条总线（追加到末尾）。
    AddBus,
    /// 删除总线 `bus`（其 send 清理、更高索引前移）。
    RemoveBus {
        bus: u8,
    },
    /// 更新某总线的 strip 参数。
    SetBusStrip {
        bus: u8,
        params: StripParams,
    },
    /// 打开某源通道的发送面板。
    OpenSends {
        channel: u8,
    },
    /// 更新某源通道对某总线的发送。
    SetSend {
        channel: u8,
        bus: u8,
        amount: f32,
        pre_fader: bool,
    },
    /// 打开乐器选择器（channel = MIDI 通道；含内置 XSynth 与插件）。
    OpenInstrumentPicker {
        channel: u8,
    },
    /// 为 MIDI 通道**追加**一个插件乐器到链尾（InsertRef 入持久化层 +
    /// 机架加载 + 安装引擎）。
    AssignInstrument {
        channel: u8,
        plugin: PluginEntry,
    },
    /// 移除 MIDI 通道链内第 `index` 个插件乐器（卸载机架 + 持久化层移除，
    /// 链路空后回到 XSynth）。
    RemoveInstrument {
        channel: u8,
        index: usize,
    },
    /// 清空某 MIDI 通道的全部插件乐器（回到内置 XSynth）。
    ClearInstruments {
        channel: u8,
    },
    /// 打开 insert 槽位的参数面板。
    OpenInsertParams {
        target: InsertTarget,
        slot: usize,
    },
    /// 打开乐器槽位的参数面板（`index` = 该通道链内序号）。
    OpenInstrumentParams {
        channel: u8,
        index: usize,
    },
    /// 打开/关闭乐器插件原生界面（`index` = 该通道链内序号）。
    ToggleInstrumentGui {
        channel: u8,
        index: usize,
    },
    /// 打开某 MIDI 通道内置 XSynth 的音色库配置窗口。
    OpenXsynthConfig {
        channel: u8,
    },
    RescanPlugins,
}

impl App {
    /// 活跃文档的机架（不存在则补默认——机架与 documents 平行，正常路径必然同长）。
    pub(crate) fn mixer_rack_mut(&mut self, idx: usize) -> &mut MixerRack {
        if idx >= self.mixer_racks.len() {
            self.mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        &mut self.mixer_racks[idx]
    }

    /// 记录混音台编辑的 undo 起点（若尚无暂存）。
    ///
    /// 高频路径（推子/旋钮拖动）每帧调用；只在首次变更时 clone 一份
    /// `MixerParams` 起点快照，指针空闲后由
    /// [`App::commit_pending_mixer_undo`] 合并提交。
    pub(crate) fn note_mixer_edit(&mut self, label: &str) {
        if self.mixer_undo_pending.is_none()
            && let Some(idx) = self.workspace.active_doc
        {
            let doc = &self.workspace.documents[idx];
            self.mixer_undo_pending =
                Some((doc.doc_id, Box::new(doc.mixer.clone()), label.to_string()));
        }
    }

    /// 指针空闲（未按住任何鼠标键）时，把暂存的混音台编辑提交为一条 undo。
    /// 无实际变化（如点了 M/S 又点回来）时不产生 undo。
    pub(crate) fn commit_pending_mixer_undo(&mut self, ctx: &egui::Context) {
        let Some((doc_id, before, label)) = self.mixer_undo_pending.take() else {
            return;
        };
        if ctx.input(|i| i.pointer.any_down()) {
            self.mixer_undo_pending = Some((doc_id, before, label));
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        // 拖动期间用户切换过文档（极少数场景）：暂存与当前文档不符则丢弃。
        if doc.doc_id != doc_id {
            return;
        }
        if *before == doc.mixer {
            return;
        }
        let after = Box::new(doc.mixer.clone());
        let snapshot = doc.capture_snapshot();
        doc.push_undo(
            yinhe_editor_core::history::UndoAction::MixerParams { before, after },
            &label,
            snapshot,
        );
        doc.mixer_dirty = true;
    }

    /// 通道 AR 面板的通道电平显示值：`(MIDI 源通道 0..256, 音频通道)` 两组
    /// `(level, hold)`（L/R）。用独立的 `ca_smoothed` 弹道，与混音台 strip
    /// 的 `smoothed` 互不影响。
    pub(crate) fn ca_channel_levels(
        &mut self,
        idx: usize,
        dt: f32,
    ) -> (
        Vec<crate::widgets::meter::StereoMeter>,
        Vec<crate::widgets::meter::StereoMeter>,
    ) {
        let model = self.workspace.documents[idx].data.model.clone();
        let layout = ChannelLayout::from_model(&model);
        // 引擎重建后通道数变化 → 重置弹道（同 mix.smoothed）。
        let channel_count = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.mixer_channel_count())
            .unwrap_or(0);
        if self.mix.ca_smoothed.len() != channel_count {
            self.mix.ca_smoothed = vec![MeterState::default(); channel_count];
        }
        let handle = self.audio_state.handle.as_ref();
        let mut midi = vec![((0.0, 0.0), (0.0, 0.0)); SOURCE_CHANNELS];
        for (c, slot) in midi.iter_mut().enumerate() {
            if !layout.is_active(c) {
                continue;
            }
            let dense = layout.dense_for(c);
            if dense != u32::MAX {
                *slot = smoothed_peak(handle, &mut self.mix.ca_smoothed, dense as usize, dt);
            }
        }
        let audio_channels = layout.audio_channels().to_vec();
        let mut audio = vec![((0.0, 0.0), (0.0, 0.0)); audio_channels.len()];
        for (i, &ach) in audio_channels.iter().enumerate() {
            let dense = layout.audio_dense_for(ach);
            if dense != u32::MAX {
                audio[i] = smoothed_peak(handle, &mut self.mix.ca_smoothed, dense as usize, dt);
            }
        }
        (midi, audio)
    }

    /// 更新某源通道的 strip 参数：写持久化层 + 推引擎。
    pub(crate) fn apply_strip(&mut self, idx: usize, channel: u8, params: StripParams) {
        self.workspace.documents[idx].mixer_mut().channels[channel as usize] = params;
        if let Some(audio) = &self.audio_state.handle {
            audio
                .handle
                .send(yinhe_audio::AudioCommand::SetChannelStrip { channel, params });
        }
    }

    /// 更新某总线的 strip 参数：写持久化层 + 推引擎（高频路径）。
    pub(crate) fn apply_bus_strip(&mut self, idx: usize, bus: u8, params: StripParams) {
        if let Some(slot) = self.workspace.documents[idx]
            .mixer_mut()
            .buses
            .get_mut(bus as usize)
        {
            *slot = params;
        }
        if let Some(a) = &self.audio_state.handle {
            a.handle
                .send(yinhe_audio::AudioCommand::SetBusStrip { bus, params });
        }
    }

    /// 更新某音频通道的 strip 参数：写持久化层 + 推引擎（高频路径）。
    pub(crate) fn apply_audio_strip(&mut self, idx: usize, channel: u16, params: StripParams) {
        let mixer = self.workspace.documents[idx].mixer_mut();
        if mixer.audio_channels.len() <= channel as usize {
            mixer
                .audio_channels
                .resize(channel as usize + 1, StripParams::default());
        }
        mixer.audio_channels[channel as usize] = params;
        if let Some(a) = &self.audio_state.handle {
            a.handle
                .send(yinhe_audio::AudioCommand::SetAudioStrip { channel, params });
        }
    }

    /// 全量同步总线配置（增删总线 / 改发送后推一次）。
    pub(crate) fn sync_bus_config_to_engine(&mut self, idx: usize) {
        let (buses, sends) = {
            let mixer = self.workspace.documents[idx].mixer_mut();
            mixer.ensure_len();
            (mixer.buses.clone(), mixer.sends.clone())
        };
        if let Some(a) = &self.audio_state.handle {
            a.handle.send(yinhe_audio::AudioCommand::SyncBusConfig {
                buses: Box::new(buses),
                sends: Box::new(sends),
            });
        }
    }

    pub(crate) fn apply_master(&mut self, idx: usize, params: MasterParams) {
        self.workspace.documents[idx].mixer_mut().master = params;
        if let Some(audio) = &self.audio_state.handle {
            audio
                .handle
                .send(yinhe_audio::AudioCommand::SetMasterParams { params });
        }
    }

    /// 工程加载后：按 MixerParams 的 InsertRef 重建机架。
    /// 实例只加载不激活——引擎此时尚未重建，spawn 完成后由
    /// `push_mixer_state_to_engine` → `ensure_all_sent` 统一激活补发。
    pub(crate) fn restore_mixer_rack(&mut self, idx: usize) {
        self.migrate_builtin_inserts(idx);
        let mixer = self.workspace.documents[idx].mixer.clone();
        let mut rack = MixerRack::default();
        for ch in 0..SOURCE_CHANNELS {
            for r in &mixer.channel_inserts[ch] {
                let _ = rack.load_plugin(
                    Some(ch as u8),
                    r.format,
                    &r.plugin_path,
                    &r.plugin_id,
                    &r.name,
                    r.state.as_deref(),
                    r.bypassed,
                );
            }
        }
        for (b, chain) in mixer.bus_inserts.iter().enumerate() {
            for r in chain {
                let _ = rack.load_plugin(
                    InsertTarget::Bus(b as u8),
                    r.format,
                    &r.plugin_path,
                    &r.plugin_id,
                    &r.name,
                    r.state.as_deref(),
                    r.bypassed,
                );
            }
        }
        for r in &mixer.master_inserts {
            let _ = rack.load_plugin(
                None,
                r.format,
                &r.plugin_path,
                &r.plugin_id,
                &r.name,
                r.state.as_deref(),
                r.bypassed,
            );
        }
        if idx >= self.mixer_racks.len() {
            self.mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        self.mixer_racks[idx] = rack;
    }

    /// 迁移：移除旧工程残留的内置效果器（ChannelGain/Pan/Filter）insert。
    /// 通道处理已内置到音源的通道处理段（yinhe-audio `ChannelDspChain`），
    /// 保留会双重处理 CC7/10/11/71/74；确有移除时标脏以便保存清理结果。
    fn migrate_builtin_inserts(&mut self, idx: usize) {
        let doc = &mut self.workspace.documents[idx];
        if strip_builtin_inserts(&mut doc.mixer) {
            doc.mixer_dirty = true;
        }
    }

    /// 工程加载后：按 MixerParams.instruments 重建乐器机架。
    /// 与 restore_mixer_rack 同理——只加载不激活，引擎重建后由
    /// push_mixer_state_to_engine → ensure_all_sent 统一激活补发。
    pub(crate) fn restore_instrument_rack(&mut self, idx: usize) {
        let mixer = self.workspace.documents[idx].mixer.clone();
        let mut rack = InstrumentRack::default();
        for (ch, refs) in mixer.instruments.iter().enumerate() {
            for r in refs {
                let _ = rack.add(
                    ch as u8,
                    r.uid,
                    r.format,
                    &r.plugin_path,
                    &r.plugin_id,
                    &r.name,
                    r.state.as_deref(),
                );
            }
        }
        if idx >= self.instrument_racks.len() {
            self.instrument_racks
                .resize_with(idx + 1, InstrumentRack::default);
        }
        self.instrument_racks[idx] = rack;
    }

    /// 引擎 spawn 完成后：全量同步混音台（参数 + 各 insert 处理器补发 + 乐器安装）。
    pub(crate) fn push_mixer_state_to_engine(&mut self, idx: usize) {
        let Self {
            audio_state,
            workspace,
            mixer_racks,
            instrument_racks,
            ..
        } = self;
        let Some(audio) = audio_state.handle.as_ref() else {
            return;
        };
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SetMixerParams {
                params: Box::new(workspace.documents[idx].mixer.clone()),
            });
        if idx >= mixer_racks.len() {
            mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        mixer_racks[idx].ensure_all_sent(&audio.handle, audio.sample_rate);
        if idx >= instrument_racks.len() {
            instrument_racks.resize_with(idx + 1, InstrumentRack::default);
        }
        instrument_racks[idx].ensure_all_sent(&audio.handle, audio.sample_rate);
    }

    /// 每帧：回收渲染线程退回的 insert 处理器 + 轮询插件反向请求。
    /// restart/移除退回的槽位在同一帧由 ensure_all_sent 补发（幂等，只补
    /// sent=false 的槽位）。
    pub(crate) fn poll_mixer_plugins(&mut self) {
        let Self {
            audio_state,
            mixer_racks,
            instrument_racks,
            ..
        } = self;
        let Some(audio) = audio_state.handle.as_ref() else {
            return;
        };
        let returned = audio.handle.drain_insert_returns();
        let instrument_returned = audio.handle.drain_instrument_returns();
        let Some(idx) = audio_state.active_doc else {
            if !returned.is_empty() {
                tracing::warn!(
                    "引擎退回 {} 个 insert 处理器，但无绑定文档可回收",
                    returned.len()
                );
            }
            if !instrument_returned.is_empty() {
                tracing::warn!(
                    "引擎退回 {} 个乐器处理器，但无绑定文档可回收",
                    instrument_returned.len()
                );
            }
            return;
        };
        if idx >= mixer_racks.len() {
            mixer_racks.resize_with(idx + 1, MixerRack::default);
        }
        if idx >= instrument_racks.len() {
            instrument_racks.resize_with(idx + 1, InstrumentRack::default);
        }
        let rack = &mut mixer_racks[idx];
        if !returned.is_empty() {
            rack.on_returns(returned);
        }
        let irack = &mut instrument_racks[idx];
        if !instrument_returned.is_empty() {
            irack.on_returns(instrument_returned);
        }
        rack.poll_requests(Some(&audio.handle));
        irack.poll_requests(&audio.handle);
        rack.ensure_all_sent(&audio.handle, audio.sample_rate);
        irack.ensure_all_sent(&audio.handle, audio.sample_rate);
    }
}

/// 读某 dense 通道电平并做 UI 侧弹道处理，返回 ((level L,R), (hold L,R))。
fn smoothed_peak(
    handle: Option<&yinhe_audio::CpalAudioHandle>,
    smoothed: &mut [MeterState],
    dense: usize,
    dt: f32,
) -> ((f32, f32), (f32, f32)) {
    let raw = handle
        .and_then(|a| a.handle.channel_meter_read(dense))
        .unwrap_or((0.0, 0.0));
    let Some(s) = smoothed.get_mut(dense) else {
        return (raw, raw);
    };
    s.update(raw, dt);
    (s.level(), s.hold())
}

/// 更新总输出电平弹道（每帧一次，main_loop 在走带栏之前调用）。结果存入
/// [`MixUiState::master_display`]，供混音台 master 条与走带栏时间码表复用。
pub(crate) fn update_master_meter(app: &mut App, dt: f32) -> ((f32, f32), (f32, f32)) {
    let raw = app
        .audio_state
        .handle
        .as_ref()
        .map(|a| a.handle.master_meter_read())
        .unwrap_or((0.0, 0.0));
    let s = &mut app.mix.smoothed_master;
    s.update(raw, dt);
    app.mix.master_display = (s.level(), s.hold());
    app.mix.master_display
}

/// 更新每轨力度表弹道（每帧一次，main_loop 在走带栏/编排视图之前调用）。
/// 读数来自引擎（仅播放时非零），结果存入 [`MixUiState::track_display`]。
pub(crate) fn update_track_meters(app: &mut App, dt: f32) {
    let n = app
        .workspace
        .active_doc
        .map(|idx| app.workspace.documents[idx].data.model.tracks.len())
        .unwrap_or(0);
    if app.mix.smoothed_tracks.len() != n {
        app.mix.smoothed_tracks = vec![MeterState::default(); n];
    }
    app.mix.track_display.clear();
    for i in 0..n {
        let raw = app
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.track_meter_read(i))
            .unwrap_or(0.0);
        let s = &mut app.mix.smoothed_tracks[i];
        s.update((raw, raw), dt);
        app.mix.track_display.push((s.level().0, s.hold().0));
    }
}

/// 混音主体几何：通道区 + 主输出区。主输出贴面板右缘（无右边距），
/// 与通道区之间留 4px 分隔缝（缝隙底色由调用方铺轨道色）。
pub(crate) fn body_layout(body: egui::Rect) -> (egui::Rect, egui::Rect) {
    let mut channels_rect = body;
    channels_rect.max.x = (body.max.x - (strip::STRIP_WIDTH + 4.0)).max(body.min.x);
    let master_rect =
        egui::Rect::from_min_max(egui::pos2(channels_rect.max.x + 4.0, body.min.y), body.max);
    (channels_rect, master_rect)
}

/// MIX 模式主入口（layout.rs 在 Mix 模式且已打开工程时调用）。
pub(crate) fn show(app: &mut App, ui: &mut egui::Ui, rect: egui::Rect) {
    let Some(idx) = app.workspace.active_doc else {
        return;
    };
    crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.mix"));

    // 本帧的只读数据快照（Arc 克隆便宜；layout 与引擎同源，dense 映射一致）。
    let model = app.workspace.documents[idx].data.model.clone();
    let layout = ChannelLayout::from_model(&model);
    let active: Vec<u8> = (0..SOURCE_CHANNELS)
        .filter(|&c| layout.is_active(c))
        .map(|c| c as u8)
        .collect();
    // 取该通道首个使用轨道的颜色作为通道条色带（与 AR/PR 轨道色同源，含
    // Conductor 主题色）。Conductor 是 Master 轨（AR 里不显示通道号），不归入
    // 任何通道条。
    let edit = &app.workspace.documents[idx].edit;
    let track_colors = &edit.track_cache.colors;
    let conductor_idx = edit.track_cache.conductor_idx;
    let colors: Vec<egui::Color32> = active
        .iter()
        .map(|&ch| {
            let first_track = model.tracks.iter().enumerate().find_map(|(ti, t)| {
                (Some(ti as u16) != conductor_idx && t.global_channel() == ch).then_some(ti)
            });
            let c = first_track
                .and_then(|ti| track_colors.get(ti).copied())
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
            crate::theme::rgba_to_color32((c[0], c[1], c[2], c[3]))
        })
        .collect();

    let dt = ui.ctx().input(|i| i.stable_dt).min(0.1);
    // 引擎重建后通道数变化 → 重置滑动峰值。
    let channel_count = app
        .audio_state
        .handle
        .as_ref()
        .map(|a| a.handle.mixer_channel_count())
        .unwrap_or(0);
    if app.mix.smoothed.len() != channel_count {
        app.mix.smoothed = vec![MeterState::default(); channel_count];
    }

    let mut actions: Vec<MixAction> = Vec::new();

    // 铺背景。
    ui.painter().rect_filled(rect, 0.0, crate::theme::app_bg());

    // 左侧工具栏（PR 面板工具条同款）：从面板左缘扣出竖条，右侧为混音主体。
    // 工具栏在主体之后绘制（见下方），保证层级高于混音台元素。
    let tool_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.min.x + crate::theme::TOOL_BAR_W, rect.max.y),
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(tool_rect.max.x, rect.min.y),
                rect.max,
            ))
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
        |ui| {
            // 主体：左侧横向滚动通道条 + 右侧固定 Master（贴面板右缘，无右边距）。
            let body = ui.available_rect_before_wrap();
            let (channels_rect, master_rect) = body_layout(body);
            // 通道区与主输出之间的 4px 缝隙铺轨道底色作分隔：亮色主题下
            // 不再露出一条突兀的 app_bg 白带（暗色下与面板底差异极小）。
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(channels_rect.max.x, body.min.y),
                    egui::pos2(master_rect.min.x, body.max.y),
                ),
                0.0,
                crate::theme::track_bg(),
            );

            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(channels_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::TOP)),
                |ui| {
                    egui::ScrollArea::horizontal()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = strip::STRIP_GAP;
                            let strip_h = ui.available_height();
                            ui.horizontal_top(|ui| {
                                for (i, &ch) in active.iter().enumerate() {
                                    let dense = layout.dense_for(ch as usize);
                                    let (level, hold) = if dense != u32::MAX {
                                        smoothed_peak(
                                            app.audio_state.handle.as_ref(),
                                            &mut app.mix.smoothed,
                                            dense as usize,
                                            dt,
                                        )
                                    } else {
                                        ((0.0, 0.0), (0.0, 0.0))
                                    };
                                    strip::channel_strip(
                                        app,
                                        ui,
                                        idx,
                                        ch,
                                        colors[i],
                                        level,
                                        hold,
                                        strip_h,
                                        &mut actions,
                                    );
                                }
                                // 音频条：MIDI 条后分隔 + 每个音频通道一条。
                                let audio_channels: Vec<u16> = layout.audio_channels().to_vec();
                                if !audio_channels.is_empty() {
                                    ui.separator();
                                    for &ach in audio_channels.iter() {
                                        let dense = layout.audio_dense_for(ach);
                                        let (level, hold) = if dense != u32::MAX {
                                            smoothed_peak(
                                                app.audio_state.handle.as_ref(),
                                                &mut app.mix.smoothed,
                                                dense as usize,
                                                dt,
                                            )
                                        } else {
                                            ((0.0, 0.0), (0.0, 0.0))
                                        };
                                        strip::audio_strip(
                                            app,
                                            ui,
                                            idx,
                                            ach,
                                            level,
                                            hold,
                                            strip_h,
                                            &mut actions,
                                        );
                                    }
                                }
                                // 总线条（bus / return）。
                                let bus_count = app.workspace.documents[idx].mixer.buses.len();
                                if bus_count > 0 {
                                    ui.separator();
                                    for b in 0..bus_count {
                                        let raw = app
                                            .audio_state
                                            .handle
                                            .as_ref()
                                            .map(|a| a.handle.bus_meter_read(b))
                                            .unwrap_or((0.0, 0.0));
                                        let (level, hold) = {
                                            let key = format!("bus{b}");
                                            let slot =
                                                app.mix.smoothed_buses.entry(key).or_default();
                                            slot.update(raw, dt);
                                            (slot.level(), slot.hold())
                                        };
                                        strip::bus_strip(
                                            app,
                                            ui,
                                            idx,
                                            b as u8,
                                            level,
                                            hold,
                                            strip_h,
                                            &mut actions,
                                        );
                                    }
                                }
                            });
                        });
                },
            );

            // Master 条（分隔由 master 通道条自身的描边提供，无需再画 vline）。
            // 电平弹道已由 main_loop 每帧更新一次（master_display），此处只读取。
            let (master_level, master_hold) = app.mix.master_display;
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(master_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
                |ui| {
                    // 极窄面板时条宽可能超出 master_rect：裁到区域，防画到通道区/工具栏
                    ui.set_clip_rect(master_rect.intersect(ui.clip_rect()));
                    strip::master_strip(
                        app,
                        ui,
                        idx,
                        master_level,
                        master_hold,
                        master_rect.height(),
                        &mut actions,
                    )
                },
            );
        },
    );

    // 左侧工具栏：在主体之后绘制（后注册的 widget 命中优先），层级高于混音台
    // 元素；只投递 MixAction、按钮 id 稳定，不存在状态或时序问题。
    tool_bar::show(app, ui, tool_rect, &mut actions);

    // 统一应用本帧动作。
    for action in actions {
        apply_action(app, idx, action);
    }

    // 电平表动画：播放中或衰减未归零时保持约 30fps 重绘。
    let any_level = app.mix.smoothed.iter().any(|s| {
        let (l, r) = s.level();
        l > 0.001 || r > 0.001
    }) || {
        let (l, r) = app.mix.smoothed_master.level();
        l > 0.001 || r > 0.001
    };
    let playing = app
        .audio_state
        .handle
        .as_ref()
        .is_some_and(|a| a.handle.is_playing());
    if playing || any_level {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// 全局浮层：插件选择器 + 参数面板。
///
/// 从 MIX 视图的 `show` 里拆出（三视图通用：底部设备栏在 AR/EDIT 下也能打开
/// 选择器与参数面板），由 main_loop 的 overlay 阶段统一调用。
pub(crate) fn show_global_overlays(app: &mut App, ctx: &egui::Context) {
    let Some(idx) = app.workspace.active_doc else {
        return;
    };
    let mut actions: Vec<MixAction> = Vec::new();
    if let Some(target) = app.mix.picker_for {
        strip::plugin_picker(app, ctx, target, &mut actions);
    }
    if let Some(ch) = app.mix.instrument_picker_for {
        strip::instrument_picker(app, ctx, ch, &mut actions);
    }
    if let Some(ch) = app.mix.sends_for {
        strip::send_popup(app, ctx, ch, &mut actions);
    }
    for action in actions {
        apply_action(app, idx, action);
    }
    param_panel::show(app, ctx);
}

/// insert 目标的持久化链（越界/不存在返回 None）。
pub(crate) fn insert_refs(
    mixer: &mut yinhe_mixer::MixerParams,
    target: InsertTarget,
) -> Option<&mut Vec<yinhe_mixer::InsertRef>> {
    match target {
        InsertTarget::Channel(ch) => mixer.channel_inserts.get_mut(ch as usize),
        InsertTarget::Audio(ch) => {
            if mixer.audio_inserts.len() <= ch as usize {
                mixer.audio_inserts.resize(ch as usize + 1, Vec::new());
            }
            mixer.audio_inserts.get_mut(ch as usize)
        }
        InsertTarget::Bus(bus) => mixer.bus_inserts.get_mut(bus as usize),
        InsertTarget::Master => Some(&mut mixer.master_inserts),
    }
}

fn apply_action(app: &mut App, idx: usize, action: MixAction) {
    match action {
        MixAction::SetStrip { channel, params } => {
            app.note_mixer_edit(t!("undo.mix_param").as_ref());
            app.apply_strip(idx, channel, params);
        }
        MixAction::SetAudioStrip { channel, params } => {
            app.note_mixer_edit(t!("undo.mix_param").as_ref());
            app.apply_audio_strip(idx, channel, params);
        }
        MixAction::SetMaster { params } => {
            app.note_mixer_edit(t!("undo.mix_param").as_ref());
            app.apply_master(idx, params);
        }
        MixAction::OpenPicker { target } => {
            app.mix.picker_for = Some(target);
            app.mix.picker_filter.clear();
        }
        MixAction::AddInsert { target, plugin } => {
            if let Some(refs) = insert_refs(app.workspace.documents[idx].mixer_mut(), target) {
                refs.push(yinhe_mixer::InsertRef {
                    uid: 0,
                    plugin_path: plugin.path.clone(),
                    plugin_id: plugin.id.clone(),
                    name: plugin.name.clone(),
                    format: plugin.format,
                    bypassed: false,
                    state: None,
                });
            }
            let rack = app.mixer_rack_mut(idx);
            if let Err(e) = rack.load_plugin(
                target,
                plugin.format,
                &plugin.path,
                &plugin.id,
                &plugin.name,
                None,
                false,
            ) {
                // 加载失败：引用已入持久化层（保存不丢），但机架无实例；
                // 状态行提示用户。
                rack.last_error = Some(e.0);
            }
            app.push_mixer_state_to_engine(idx);
            app.mix.picker_for = None;
        }
        MixAction::BypassInsert {
            target,
            slot,
            bypassed,
        } => {
            if let Some(refs) = insert_refs(app.workspace.documents[idx].mixer_mut(), target)
                && let Some(r) = refs.get_mut(slot)
            {
                r.bypassed = bypassed;
            }
            app.mixer_rack_mut(idx).set_bypass(target, slot, bypassed);
        }
        MixAction::RemoveInsert { target, slot } => {
            if let Some(refs) = insert_refs(app.workspace.documents[idx].mixer_mut(), target)
                && slot < refs.len()
            {
                refs.remove(slot);
            }
            // 字段级借用分裂：audio_state 只读、mixer_racks 可变。
            let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
            if idx < app.mixer_racks.len() {
                app.mixer_racks[idx].remove_slot(target, slot, handle);
            }
        }
        MixAction::AddBus => {
            app.note_mixer_edit(t!("undo.mix_bus").as_ref());
            app.workspace.documents[idx].mixer_mut().add_bus();
            app.sync_bus_config_to_engine(idx);
        }
        MixAction::RemoveBus { bus } => {
            app.note_mixer_edit(t!("undo.mix_bus").as_ref());
            app.workspace.documents[idx].mixer_mut().remove_bus(bus);
            let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
            if idx < app.mixer_racks.len() {
                app.mixer_racks[idx].remove_bus_chain(bus, handle);
            }
            app.sync_bus_config_to_engine(idx);
        }
        MixAction::SetBusStrip { bus, params } => {
            app.note_mixer_edit(t!("undo.mix_param").as_ref());
            app.apply_bus_strip(idx, bus, params);
        }
        MixAction::SetSend {
            channel,
            bus,
            amount,
            pre_fader,
        } => {
            app.note_mixer_edit(t!("undo.mix_send").as_ref());
            {
                let mixer = app.workspace.documents[idx].mixer_mut();
                if let Some(list) = mixer.sends.get_mut(channel as usize) {
                    if let Some(send) = list.iter_mut().find(|s| s.bus == bus) {
                        send.amount = amount;
                        send.pre_fader = pre_fader;
                    } else if amount > 0.0 {
                        list.push(yinhe_mixer::SendParams {
                            bus,
                            amount,
                            pre_fader,
                        });
                    }
                    // 发送量为 0 的条目不再保留（避免持久化层堆积）。
                    list.retain(|s| s.amount > 0.0);
                }
            }
            app.sync_bus_config_to_engine(idx);
        }
        MixAction::OpenSends { channel } => app.mix.sends_for = Some(channel),
        MixAction::ToggleGui { target, slot } => {
            match app.mixer_rack_mut(idx).toggle_gui(target, slot) {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.0.clone();
                    app.mixer_rack_mut(idx).last_error = Some(msg);
                }
            }
        }
        MixAction::OpenInstrumentPicker { channel } => {
            app.mix.instrument_picker_for = Some(channel);
            app.mix.picker_filter.clear();
        }
        MixAction::OpenXsynthConfig { channel } => {
            app.mix.xsynth_config_for = Some(channel);
        }
        MixAction::ToggleInstrumentGui { channel, index } => {
            let result = app
                .instrument_racks
                .get_mut(idx)
                .map(|rack| rack.toggle_gui(channel, index));
            if let Some(Err(e)) = result
                && let Some(rack) = app.instrument_racks.get_mut(idx)
            {
                rack.last_error = Some(e.0);
            }
        }
        MixAction::AssignInstrument { channel, plugin } => {
            let uid;
            {
                let doc = &mut app.workspace.documents[idx];
                let m = doc.mixer_mut();
                m.ensure_len();
                uid = m.alloc_instrument_uid();
                m.instruments[channel as usize].push(yinhe_mixer::InsertRef {
                    uid,
                    plugin_path: plugin.path.clone(),
                    plugin_id: plugin.id.clone(),
                    name: plugin.name.clone(),
                    format: plugin.format,
                    bypassed: false,
                    state: None,
                });
            }
            if idx < app.instrument_racks.len() {
                let rack = &mut app.instrument_racks[idx];
                if let Err(e) = rack.add(
                    channel,
                    uid,
                    plugin.format,
                    &plugin.path,
                    &plugin.id,
                    &plugin.name,
                    None,
                ) {
                    rack.last_error = Some(e.0);
                }
            }
            app.push_mixer_state_to_engine(idx);
            app.mix.instrument_picker_for = None;
        }
        MixAction::RemoveInstrument { channel, index } => {
            {
                let doc = &mut app.workspace.documents[idx];
                let refs = &mut doc.mixer_mut().instruments[channel as usize];
                if index < refs.len() {
                    refs.remove(index);
                }
            }
            if idx < app.instrument_racks.len() {
                let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
                let rack = &mut app.instrument_racks[idx];
                rack.remove(channel, index, handle);
            }
        }
        MixAction::ClearInstruments { channel } => {
            app.workspace.documents[idx].mixer_mut().instruments[channel as usize].clear();
            if idx < app.instrument_racks.len() {
                let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
                let rack = &mut app.instrument_racks[idx];
                while rack.slot_count(channel) > 0 {
                    rack.remove(channel, 0, handle);
                }
            }
        }
        MixAction::OpenInsertParams { target, slot } => {
            let panel = app
                .mixer_racks
                .get_mut(idx)
                .and_then(|rack| rack.instance_mut(target, slot))
                .map(|instance| {
                    let title = instance.name().to_string();
                    ParamPanel::open(
                        param_panel::ParamTarget::Insert { target, slot },
                        title,
                        instance,
                    )
                });
            if let Some(panel) = panel {
                app.mix.param_panel = Some(panel);
            }
        }
        MixAction::OpenInstrumentParams { channel, index } => {
            let uid = app.workspace.documents[idx]
                .mixer
                .instruments
                .get(channel as usize)
                .and_then(|refs| refs.get(index))
                .map(|r| r.uid)
                .unwrap_or(0);
            let panel = app
                .instrument_racks
                .get_mut(idx)
                .and_then(|rack| rack.instance_mut(channel, index))
                .map(|instance| {
                    let title = instance.name().to_string();
                    ParamPanel::open(
                        param_panel::ParamTarget::Instrument {
                            channel,
                            index,
                            uid,
                        },
                        title,
                        instance,
                    )
                });
            if let Some(panel) = panel {
                app.mix.param_panel = Some(panel);
            }
        }
        MixAction::RescanPlugins => start_plugin_scan(app),
    }
}

/// 移除混音台里所有内置效果器（`PluginFormat::Builtin`）insert（迁移）。
/// 返回是否有移除：无移除时调用方不应标脏（打开旧工程不该弹保存）。
fn strip_builtin_inserts(mixer: &mut yinhe_mixer::MixerParams) -> bool {
    use yinhe_mixer::PluginFormat;
    let mut removed = false;
    for chain in mixer
        .channel_inserts
        .iter_mut()
        .chain(mixer.bus_inserts.iter_mut())
        .chain(mixer.audio_inserts.iter_mut())
    {
        let before = chain.len();
        chain.retain(|r| r.format != PluginFormat::Builtin);
        removed |= chain.len() != before;
    }
    let before = mixer.master_inserts.len();
    mixer
        .master_inserts
        .retain(|r| r.format != PluginFormat::Builtin);
    removed |= mixer.master_inserts.len() != before;
    removed
}

/// 启动插件扫描子进程（不阻塞 UI）；已有扫描进行中则忽略。
pub(crate) fn start_plugin_scan(app: &mut App) {
    if app.mix.scan_in_progress {
        return;
    }
    match crate::plugin_scan::spawn_scan_worker() {
        Some(rx) => {
            app.mix.scan_rx = Some(rx);
            app.mix.scan_in_progress = true;
            app.mix.scanned = None;
            app.mix.scan_errors = 0;
        }
        None => {
            // worker 起不来也标记"扫描完成（空结果）"，否则每帧重试。
            tracing::warn!("启动插件扫描子进程失败");
            app.mix.scanned = Some(Vec::new());
        }
    }
}

/// 每帧轮询扫描子进程结果。
pub(crate) fn poll_plugin_scan(app: &mut App) {
    let Some(rx) = &app.mix.scan_rx else {
        return;
    };
    match rx.try_recv() {
        Ok(ScanProgress::Batch(entries)) => {
            let scanned = app.mix.scanned.get_or_insert_with(Vec::new);
            scanned.extend(entries);
            scanned.sort_by(|a, b| a.name.cmp(&b.name));
        }
        Ok(ScanProgress::Finished { errors }) => {
            app.mix.scan_errors = errors;
            app.mix.scan_rx = None;
            app.mix.scan_in_progress = false;
            // 零结果（无插件目录/全部未产出条目）也要落 Some，
            // 否则"scanned.is_none() 才扫描"的条件会每帧重扫。
            app.mix.scanned.get_or_insert_with(Vec::new);
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.mix.scan_rx = None;
            app.mix.scan_in_progress = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_mixer::{InsertRef, MixerParams, PluginFormat};

    fn insert_ref(format: PluginFormat, id: &str) -> InsertRef {
        InsertRef {
            plugin_id: id.to_string(),
            format,
            ..Default::default()
        }
    }

    /// 迁移只移除内置效果器 insert（channel/bus/master），插件保留；
    /// 无内置时不报告移除（调用方由此避免把刚打开的工程标脏）。
    #[test]
    fn strip_builtin_inserts_removes_builtin_only() {
        let mut mixer = MixerParams::default();
        mixer.channel_inserts[3] = vec![
            insert_ref(PluginFormat::Builtin, "channel_gain"),
            insert_ref(PluginFormat::Clap, "clap-fx"),
        ];
        mixer.bus_inserts.push(vec![
            insert_ref(PluginFormat::Builtin, "channel_pan"),
            insert_ref(PluginFormat::Vst3, "vst-fx"),
        ]);
        mixer.master_inserts = vec![
            insert_ref(PluginFormat::Vst3, "vst-master"),
            insert_ref(PluginFormat::Builtin, "channel_filter"),
        ];

        assert!(strip_builtin_inserts(&mut mixer));
        assert_eq!(mixer.channel_inserts[3].len(), 1);
        assert_eq!(mixer.channel_inserts[3][0].plugin_id, "clap-fx");
        assert_eq!(mixer.bus_inserts[0].len(), 1);
        assert_eq!(mixer.bus_inserts[0][0].plugin_id, "vst-fx");
        assert_eq!(mixer.master_inserts.len(), 1);
        assert_eq!(mixer.master_inserts[0].plugin_id, "vst-master");

        assert!(!strip_builtin_inserts(&mut mixer), "无内置 insert 不算迁移");
    }

    /// 回归：主输出贴面板右缘（无右边距），与通道区之间仅 4px 分隔缝；
    /// 常规宽度下主输出条宽度 = 通道条宽度（不溢出、不居中留缝）。
    #[test]
    fn master_rect_is_flush_with_panel_right() {
        let body = egui::Rect::from_min_max(egui::pos2(44.0, 100.0), egui::pos2(1440.0, 700.0));
        let (channels, master) = body_layout(body);
        assert_eq!(master.max.x, body.max.x, "主输出应贴右缘");
        assert_eq!(master.width(), strip::STRIP_WIDTH, "条宽 = 通道条宽");
        assert_eq!(master.min.x - channels.max.x, 4.0, "中间留 4px 分隔缝");
        assert_eq!(channels.min.x, body.min.x, "通道区从主体左缘开始");
    }
}
