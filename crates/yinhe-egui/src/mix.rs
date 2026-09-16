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

use eframe::egui;
use yinhe_audio::InsertTarget;
use yinhe_audio::channel_layout::ChannelLayout;
use yinhe_mixer::{MasterParams, StripParams};

use self::plugin_instance::PluginEntry;
use crate::plugin_scan::ScanProgress;

use crate::app::App;

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

/// 线性增益 → dB（0 以下按 -60 显示）。
pub(crate) fn gain_to_db(g: f32) -> f32 {
    if g <= 0.0001 { -60.0 } else { 20.0 * g.log10() }
}

/// dB → 线性增益（≤ -60 dB 视为静音）。
pub(crate) fn db_to_gain(db: f32) -> f32 {
    if db <= -59.9 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// 电平 UI 侧峰值衰减速度（线性域 / 秒）。
const METER_FALLOFF_PER_SEC: f32 = 2.5;

/// MIX 界面的非持久化 UI 状态。
pub(crate) struct MixUiState {
    /// 各 dense 通道的滑动峰值（L, R），UI 侧衰减用。
    smoothed: Vec<(f32, f32)>,
    smoothed_master: (f32, f32),
    /// 总线条电平表滑动峰值（key = "bus{n}"，增删总线时旧键闲置无害）。
    smoothed_buses: std::collections::HashMap<String, (f32, f32)>,
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
}

impl Default for MixUiState {
    fn default() -> Self {
        Self {
            smoothed: Vec::new(),
            smoothed_master: (0.0, 0.0),
            smoothed_buses: std::collections::HashMap::new(),
            scanned: None,
            scan_errors: 0,
            scan_rx: None,
            scan_in_progress: false,
            picker_for: None,
            sends_for: None,
            xsynth_config_for: None,
            instrument_picker_for: None,
            picker_filter: String::new(),
            param_panel: None,
        }
    }
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
    /// 为 MIDI 通道分配插件乐器（InsertRef 入持久化层 + 机架加载 + 安装引擎）。
    AssignInstrument {
        channel: u8,
        plugin: PluginEntry,
    },
    /// 移除 MIDI 通道的插件乐器（卸载机架 + 持久化层置 None → 回到 XSynth）。
    RemoveInstrument {
        channel: u8,
    },
    /// 打开 insert 槽位的参数面板。
    OpenInsertParams {
        target: InsertTarget,
        slot: usize,
    },
    /// 打开乐器槽位的参数面板。
    OpenInstrumentParams {
        channel: u8,
    },
    /// 打开/关闭乐器插件原生界面。
    ToggleInstrumentGui {
        channel: u8,
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
        for (ch, r) in mixer.instruments.iter().enumerate() {
            if let Some(r) = r {
                let _ = rack.load(
                    ch as u8,
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

/// 读某 dense 通道电平并做 UI 侧衰减。
fn smoothed_peak(
    handle: Option<&yinhe_audio::CpalAudioHandle>,
    smoothed: &mut [(f32, f32)],
    dense: usize,
    dt: f32,
) -> (f32, f32) {
    let raw = handle
        .and_then(|a| a.handle.channel_meter_read(dense))
        .unwrap_or((0.0, 0.0));
    let Some(s) = smoothed.get_mut(dense) else {
        return raw;
    };
    s.0 = raw.0.max(s.0 - METER_FALLOFF_PER_SEC * dt);
    s.1 = raw.1.max(s.1 - METER_FALLOFF_PER_SEC * dt);
    *s
}

/// MIX 模式主入口（layout.rs 在 Mix 模式且已打开工程时调用）。
pub(crate) fn show(app: &mut App, ui: &mut egui::Ui, rect: egui::Rect) {
    let Some(idx) = app.workspace.active_doc else {
        return;
    };

    // 本帧的只读数据快照（Arc 克隆便宜；layout 与引擎同源，dense 映射一致）。
    let model = app.workspace.documents[idx].data.model.clone();
    let layout = ChannelLayout::from_model(&model);
    let active: Vec<u8> = (0..SOURCE_CHANNELS)
        .filter(|&c| layout.is_active(c))
        .map(|c| c as u8)
        .collect();
    // 每通道列出使用该通道的轨道名（共享通道的轨道全部列出）+ 取首个轨道的
    // 颜色作为通道条色条（与 AR/PR 轨道色同源，含 Conductor 主题色）。
    // Conductor 是 Master 轨（AR 里不显示通道号），不归入任何通道条。
    let edit = &app.workspace.documents[idx].edit;
    let track_colors = &edit.track_colors_cache;
    let conductor_idx = edit.conductor_track_idx;
    let (names, colors): (Vec<Vec<String>>, Vec<egui::Color32>) = active
        .iter()
        .map(|&ch| {
            let mut names = Vec::new();
            let mut first_track = None;
            for (ti, t) in model.tracks.iter().enumerate() {
                if Some(ti as u16) == conductor_idx {
                    continue;
                }
                if t.global_channel() == ch {
                    names.push(t.name.clone());
                    if first_track.is_none() {
                        first_track = Some(ti);
                    }
                }
            }
            let c = first_track
                .and_then(|ti| track_colors.get(ti).copied())
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
            (
                names,
                crate::theme::rgba_to_color32((c[0], c[1], c[2], c[3])),
            )
        })
        .unzip();

    let dt = ui.ctx().input(|i| i.stable_dt).min(0.1);
    // 引擎重建后通道数变化 → 重置滑动峰值。
    let channel_count = app
        .audio_state
        .handle
        .as_ref()
        .map(|a| a.handle.mixer_channel_count())
        .unwrap_or(0);
    if app.mix.smoothed.len() != channel_count {
        app.mix.smoothed = vec![(0.0, 0.0); channel_count];
    }

    let mut actions: Vec<MixAction> = Vec::new();

    // 铺背景。
    ui.painter().rect_filled(rect, 0.0, crate::theme::app_bg());

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
        |ui| {
            strip::show_toolbar(app, ui, &mut actions);

            // 主体：左侧横向滚动通道条 + 右侧固定 Master。
            let master_w = strip::STRIP_WIDTH + 16.0;
            let body = ui.available_rect_before_wrap();
            let mut channels_rect = body;
            channels_rect.max.x = (body.max.x - master_w).max(body.min.x);
            let master_rect = egui::Rect::from_min_max(
                egui::pos2(channels_rect.max.x + 4.0, body.min.y),
                body.max,
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
                                    let peak = if dense != u32::MAX {
                                        smoothed_peak(
                                            app.audio_state.handle.as_ref(),
                                            &mut app.mix.smoothed,
                                            dense as usize,
                                            dt,
                                        )
                                    } else {
                                        (0.0, 0.0)
                                    };
                                    strip::channel_strip(
                                        app,
                                        ui,
                                        idx,
                                        ch,
                                        &names[i],
                                        colors[i],
                                        peak,
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
                                        let peak = if dense != u32::MAX {
                                            smoothed_peak(
                                                app.audio_state.handle.as_ref(),
                                                &mut app.mix.smoothed,
                                                dense as usize,
                                                dt,
                                            )
                                        } else {
                                            (0.0, 0.0)
                                        };
                                        strip::audio_strip(
                                            app,
                                            ui,
                                            idx,
                                            ach,
                                            peak,
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
                                        let key = format!("bus{b}");
                                        let slot =
                                            app.mix.smoothed_buses.entry(key).or_insert((0.0, 0.0));
                                        slot.0 = raw.0.max(slot.0 - METER_FALLOFF_PER_SEC * dt);
                                        slot.1 = raw.1.max(slot.1 - METER_FALLOFF_PER_SEC * dt);
                                        let peak = *slot;
                                        strip::bus_strip(
                                            app,
                                            ui,
                                            idx,
                                            b as u8,
                                            peak,
                                            strip_h,
                                            &mut actions,
                                        );
                                    }
                                }
                            });
                        });
                },
            );

            // 分隔线 + Master 条。
            ui.painter().vline(
                channels_rect.max.x + 2.0,
                master_rect.y_range(),
                egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
            );
            let master_peak = {
                let raw = app
                    .audio_state
                    .handle
                    .as_ref()
                    .map(|a| a.handle.master_meter_read())
                    .unwrap_or((0.0, 0.0));
                let s = &mut app.mix.smoothed_master;
                s.0 = raw.0.max(s.0 - METER_FALLOFF_PER_SEC * dt);
                s.1 = raw.1.max(s.1 - METER_FALLOFF_PER_SEC * dt);
                *s
            };
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(master_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
                |ui| {
                    strip::master_strip(
                        app,
                        ui,
                        idx,
                        master_peak,
                        master_rect.height(),
                        &mut actions,
                    )
                },
            );
        },
    );

    // 统一应用本帧动作。
    for action in actions {
        apply_action(app, idx, action);
    }

    // 电平表动画：播放中或衰减未归零时保持约 30fps 重绘。
    let any_level = app.mix.smoothed.iter().any(|s| s.0 > 0.001 || s.1 > 0.001)
        || app.mix.smoothed_master.0 > 0.001
        || app.mix.smoothed_master.1 > 0.001;
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
        MixAction::SetStrip { channel, params } => app.apply_strip(idx, channel, params),
        MixAction::SetAudioStrip { channel, params } => app.apply_audio_strip(idx, channel, params),
        MixAction::SetMaster { params } => app.apply_master(idx, params),
        MixAction::OpenPicker { target } => {
            app.mix.picker_for = Some(target);
            app.mix.picker_filter.clear();
        }
        MixAction::AddInsert { target, plugin } => {
            if let Some(refs) = insert_refs(app.workspace.documents[idx].mixer_mut(), target) {
                refs.push(yinhe_mixer::InsertRef {
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
            app.workspace.documents[idx].mixer_mut().add_bus();
            app.sync_bus_config_to_engine(idx);
        }
        MixAction::RemoveBus { bus } => {
            app.workspace.documents[idx].mixer_mut().remove_bus(bus);
            let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
            if idx < app.mixer_racks.len() {
                app.mixer_racks[idx].remove_bus_chain(bus, handle);
            }
            app.sync_bus_config_to_engine(idx);
        }
        MixAction::SetBusStrip { bus, params } => app.apply_bus_strip(idx, bus, params),
        MixAction::SetSend {
            channel,
            bus,
            amount,
            pre_fader,
        } => {
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
        MixAction::ToggleInstrumentGui { channel } => {
            let result = app
                .instrument_racks
                .get_mut(idx)
                .map(|rack| rack.toggle_gui(channel));
            if let Some(Err(e)) = result
                && let Some(rack) = app.instrument_racks.get_mut(idx)
            {
                rack.last_error = Some(e.0);
            }
        }
        MixAction::AssignInstrument { channel, plugin } => {
            {
                let doc = &mut app.workspace.documents[idx];
                let m = doc.mixer_mut();
                m.ensure_len();
                let c = channel as usize;
                m.instruments[c] = Some(yinhe_mixer::InsertRef {
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
                if let Err(e) = rack.load(
                    channel,
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
        MixAction::RemoveInstrument { channel } => {
            {
                let doc = &mut app.workspace.documents[idx];
                let c = channel as usize;
                if c < doc.mixer_mut().instruments.len() {
                    doc.mixer_mut().instruments[c] = None;
                }
            }
            if idx < app.instrument_racks.len() {
                let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
                let rack = &mut app.instrument_racks[idx];
                rack.unload(channel, handle);
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
        MixAction::OpenInstrumentParams { channel } => {
            let panel = app
                .instrument_racks
                .get_mut(idx)
                .and_then(|rack| rack.instance_mut(channel))
                .map(|instance| {
                    let title = instance.name().to_string();
                    ParamPanel::open(
                        param_panel::ParamTarget::Instrument { channel },
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
}
