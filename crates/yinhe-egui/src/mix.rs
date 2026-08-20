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
pub(crate) mod rack;
mod strip;

use eframe::egui;
use yinhe_audio::channel_layout::ChannelLayout;
use yinhe_clap::PluginInfo;
use yinhe_mixer::{MasterParams, StripParams};

use crate::app::App;

pub(crate) use instrument_rack::InstrumentRack;
pub(crate) use rack::MixerRack;

/// 源通道总数（16 port × 16 通道）。
const SOURCE_CHANNELS: usize = yinhe_mixer::CHANNEL_COUNT;

/// 源通道号 → 显示标签（0 → "A01"，255 → "P16"）。
pub(crate) fn channel_label(ch: u8) -> String {
    let port = (b'A' + ch / 16) as char;
    format!("{}{:02}", port, ch % 16 + 1)
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
    /// 插件扫描结果（首次进入 MIX 或点「扫描」时填充）。
    pub(crate) scanned: Option<Vec<PluginInfo>>,
    /// 扫描中失败的包数量（诊断展示）。
    pub(crate) scan_errors: usize,
    /// 插件选择器打开目标：Some(Some(ch)) = 通道 ch，Some(None) = master。
    pub(crate) picker_for: Option<Option<u8>>,
    /// 乐器插件选择器目标：乐器通道号（0 起）；None = 未打开。
    pub(crate) instrument_picker_for: Option<u16>,
    pub(crate) picker_filter: String,
}

impl Default for MixUiState {
    fn default() -> Self {
        Self {
            smoothed: Vec::new(),
            smoothed_master: (0.0, 0.0),
            scanned: None,
            scan_errors: 0,
            picker_for: None,
            instrument_picker_for: None,
            picker_filter: String::new(),
        }
    }
}

/// 一帧内 strip 交互产出的动作（渲染完统一应用，避开借用冲突）。
pub(crate) enum MixAction {
    SetStrip {
        channel: u8,
        params: StripParams,
    },
    SetMaster {
        params: MasterParams,
    },
    OpenPicker {
        channel: Option<u8>,
    },
    AddInsert {
        channel: Option<u8>,
        plugin: PluginInfo,
    },
    BypassInsert {
        channel: Option<u8>,
        slot: usize,
        bypassed: bool,
    },
    ToggleGui {
        channel: Option<u8>,
        slot: usize,
    },
    RemoveInsert {
        channel: Option<u8>,
        slot: usize,
    },
    /// 打开乐器插件选择器（channel = 乐器通道，0 起）。
    OpenInstrumentPicker {
        channel: u16,
    },
    /// 为乐器通道分配插件（InsertRef 入持久化层 + 机架加载 + 安装引擎）。
    AssignInstrument {
        channel: u16,
        plugin: PluginInfo,
    },
    /// 移除乐器通道的插件（卸下载机架 + 持久化层置 None）。
    RemoveInstrument {
        channel: u16,
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
        self.documents[idx].mixer_mut().channels[channel as usize] = params;
        if let Some(audio) = &self.audio_state.handle {
            audio
                .handle
                .send(yinhe_audio::AudioCommand::SetChannelStrip { channel, params });
        }
    }

    pub(crate) fn apply_master(&mut self, idx: usize, params: MasterParams) {
        self.documents[idx].mixer_mut().master = params;
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
        let mixer = self.documents[idx].mixer.clone();
        let mut rack = MixerRack::default();
        for ch in 0..SOURCE_CHANNELS {
            for r in &mixer.channel_inserts[ch] {
                let _ = rack.load_plugin(
                    Some(ch as u8),
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

    /// 工程加载后：按 MixerParams.instruments 重建乐器机架。
    /// 与 restore_mixer_rack 同理——只加载不激活，引擎重建后由
    /// push_mixer_state_to_engine → ensure_all_sent 统一激活补发。
    pub(crate) fn restore_instrument_rack(&mut self, idx: usize) {
        let mixer = self.documents[idx].mixer.clone();
        let mut rack = InstrumentRack::default();
        for (ch, r) in mixer.instruments.iter().enumerate() {
            if let Some(r) = r {
                let _ = rack.load(
                    ch as u16,
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
            documents,
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
                params: Box::new(documents[idx].mixer.clone()),
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
    let Some(idx) = app.active_doc else { return };

    // 首次进入 MIX：扫描默认目录（进程内扫描，见 yinhe-clap scan 安全性说明）。
    if app.mix.scanned.is_none() {
        rescan(app);
    }

    // 本帧的只读数据快照（Arc 克隆便宜；layout 与引擎同源，dense 映射一致）。
    let model = app.documents[idx].data.model.clone();
    let layout = ChannelLayout::from_model(&model);
    let active: Vec<u8> = (0..SOURCE_CHANNELS)
        .filter(|&c| layout.is_active(c))
        .map(|c| c as u8)
        .collect();
    // 乐器通道（0 起），绘制独立的乐器条。
    let inst_channels: Vec<u16> = layout.instrument_channels().to_vec();
    // 每通道列出使用该通道的轨道名（共享通道的轨道全部列出）。
    let names: Vec<Vec<String>> = active
        .iter()
        .map(|&ch| {
            model
                .tracks
                .iter()
                .filter(|t| t.global_channel() == ch)
                .map(|t| t.name.clone())
                .collect()
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
                                        peak,
                                        &mut actions,
                                    );
                                }
                                // 乐器条：MIDI 条后分隔 + 每个乐器通道一条。
                                if !inst_channels.is_empty() {
                                    ui.separator();
                                    for &ich in inst_channels.iter() {
                                        strip::instrument_strip(app, ui, idx, ich, &mut actions);
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
                |ui| strip::master_strip(app, ui, idx, master_peak, &mut actions),
            );
        },
    );

    // 插件选择器（窗口）。
    if let Some(target) = app.mix.picker_for {
        strip::plugin_picker(app, ui.ctx(), target, &mut actions);
    }
    // 乐器插件选择器（窗口）。
    if let Some(ich) = app.mix.instrument_picker_for {
        strip::instrument_picker(app, ui.ctx(), ich, &mut actions);
    }

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

fn apply_action(app: &mut App, idx: usize, action: MixAction) {
    match action {
        MixAction::SetStrip { channel, params } => app.apply_strip(idx, channel, params),
        MixAction::SetMaster { params } => app.apply_master(idx, params),
        MixAction::OpenPicker { channel } => {
            app.mix.picker_for = Some(channel);
            app.mix.picker_filter.clear();
        }
        MixAction::AddInsert { channel, plugin } => {
            {
                let doc = &mut app.documents[idx];
                let refs = match channel {
                    Some(ch) => &mut doc.mixer_mut().channel_inserts[ch as usize],
                    None => &mut doc.mixer_mut().master_inserts,
                };
                refs.push(yinhe_mixer::InsertRef {
                    plugin_path: plugin.path.clone(),
                    plugin_id: plugin.id.clone(),
                    name: plugin.name.clone(),
                    bypassed: false,
                    state: None,
                });
            }
            let rack = app.mixer_rack_mut(idx);
            if let Err(e) =
                rack.load_plugin(channel, &plugin.path, &plugin.id, &plugin.name, None, false)
            {
                // 加载失败：引用已入持久化层（保存不丢），但机架无实例；
                // 状态行提示用户。
                rack.last_error = Some(e.0);
            }
            app.push_mixer_state_to_engine(idx);
            app.mix.picker_for = None;
        }
        MixAction::BypassInsert {
            channel,
            slot,
            bypassed,
        } => {
            {
                let doc = &mut app.documents[idx];
                let refs = match channel {
                    Some(ch) => &mut doc.mixer_mut().channel_inserts[ch as usize],
                    None => &mut doc.mixer_mut().master_inserts,
                };
                if let Some(r) = refs.get_mut(slot) {
                    r.bypassed = bypassed;
                }
            }
            app.mixer_rack_mut(idx).set_bypass(channel, slot, bypassed);
        }
        MixAction::RemoveInsert { channel, slot } => {
            {
                let doc = &mut app.documents[idx];
                let refs = match channel {
                    Some(ch) => &mut doc.mixer_mut().channel_inserts[ch as usize],
                    None => &mut doc.mixer_mut().master_inserts,
                };
                if slot < refs.len() {
                    refs.remove(slot);
                }
            }
            // 字段级借用分裂：audio_state 只读、mixer_racks 可变。
            let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
            if idx < app.mixer_racks.len() {
                app.mixer_racks[idx].remove_slot(channel, slot, handle);
            }
        }
        MixAction::ToggleGui { channel, slot } => {
            match app.mixer_rack_mut(idx).toggle_gui(channel, slot) {
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
        MixAction::AssignInstrument { channel, plugin } => {
            {
                let doc = &mut app.documents[idx];
                let c = channel as usize;
                let m = doc.mixer_mut();
                if m.instruments.len() <= c {
                    m.instruments.resize(c + 1, None);
                }
                m.instruments[c] = Some(yinhe_mixer::InsertRef {
                    plugin_path: plugin.path.clone(),
                    plugin_id: plugin.id.clone(),
                    name: plugin.name.clone(),
                    bypassed: false,
                    state: None,
                });
            }
            if idx < app.instrument_racks.len() {
                let rack = &mut app.instrument_racks[idx];
                if let Err(e) = rack.load(channel, &plugin.path, &plugin.id, &plugin.name, None) {
                    rack.last_error = Some(e.0);
                }
            }
            app.push_mixer_state_to_engine(idx);
            app.mix.instrument_picker_for = None;
        }
        MixAction::RemoveInstrument { channel } => {
            {
                let doc = &mut app.documents[idx];
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
        MixAction::RescanPlugins => rescan(app),
    }
}

/// 扫描默认 CLAP 目录（进程内加载元数据；崩溃风险见 yinhe-clap scan 文档）。
fn rescan(app: &mut App) {
    let dirs = yinhe_clap::scan::default_plugin_dirs();
    let outcomes = yinhe_clap::scan::scan_dirs(&dirs);
    let mut plugins = Vec::new();
    let mut errors = 0;
    for outcome in outcomes {
        match outcome {
            yinhe_clap::scan::ScanOutcome::Loaded(infos) => plugins.extend(infos),
            yinhe_clap::scan::ScanOutcome::Failed { path, error } => {
                errors += 1;
                tracing::warn!("扫描插件包失败 {:?}: {error}", path);
            }
        }
    }
    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    app.mix.scanned = Some(plugins);
    app.mix.scan_errors = errors;
}
