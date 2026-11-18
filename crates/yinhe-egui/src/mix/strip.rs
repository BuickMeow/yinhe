//! 混音台通道条控件。
//!
//! 每个通道 = 一个**直角矩形 + 单层描边**（通道框不用圆角，避免卡片出界）。按钮类
//! 控件用通用圆角矩形。自上而下：
//! 1. 顶部**色带**（轨道色）+ 通道号；
//! 2. **效果发送**（BUS 1 旋钮；无总线时首次拖动自动创建）；
//! 3. **固定高度的电平表模块**：
//!    - 左侧电平表（刻度 + 峰值保持）+ 表内增益菱形（拖动改电平）；
//!    - 右侧自上而下：dB 读数、声像旋钮、M、S。
//!
//! 模块高度固定（面板更高时贴底），因此所有通道条的电平表/按钮**等高**。
//! 设备链只在底部设备链视图编辑，混音台不再绘制设备链。

use eframe::egui;
use rust_i18n::t;
use yinhe_mixer::MasterParams;
use yinhe_types::{LogicalChannelId, MixerChannelId};

use crate::app::App;
use crate::widgets::meter::{METER_GAP, METER_W, SCALE_W};

use super::MixAction;
use controls::{
    focus_send_block, master_controls, section, send_height, strip_controls, strip_frame,
};

mod controls;
mod picker;

#[cfg(test)]
mod tests;

pub(crate) use controls::auto_panel_height;
pub(crate) use picker::{PickerTab, instrument_picker, picker_viewport_id, plugin_picker};

/// 通道条宽度（px）。
pub(crate) const STRIP_WIDTH: f32 = 88.0;
/// 通道条间距：0——相邻通道由各自的描边分隔，形成网格。
pub(crate) const STRIP_GAP: f32 = 0.0;
/// 内容左右边距（px）。
pub(crate) const PAD_X: f32 = 6.0;
/// 内容宽度（px）。
pub(crate) const CONTENT_W: f32 = STRIP_WIDTH - PAD_X * 2.0;
/// 顶部色带高度（px）。
pub(crate) const HEADER_H: f32 = 20.0;
/// 通用按钮圆角（px）。
pub(crate) const RADIUS: f32 = 4.0;
/// 小按钮边长（px，M/S、图标按钮）。
pub(crate) const BTN: f32 = 18.0;
/// 旋钮直径（px，发送/声像）。
pub(crate) const KNOB_D: f32 = 26.0;
/// 分区之间的竖直间隙（px）。
pub(crate) const GAP: f32 = 4.0;
/// 发送区一行高度（px）。
pub(crate) const SEND_ROW_H: f32 = KNOB_D;
/// dB 读数行高（px）。
pub(crate) const DB_ROW_H: f32 = 18.0;
/// 电平表 + 右侧按钮的固定模块高度（px）。不再用剩余高度拉伸。
pub(crate) const METER_MODULE_H: f32 = 140.0;
/// 固定模块右侧控制列宽度：dB / 声像 / M / S。
pub(crate) const CONTROLS_W: f32 = CONTENT_W - (SCALE_W + METER_W * 2.0 + METER_GAP) - GAP;
/// 卡片上下内边距（色带下 4px、卡片底 6px）。
pub(crate) const BODY_TOP_PAD: f32 = 4.0;
pub(crate) const BODY_BOTTOM_PAD: f32 = 6.0;

pub(crate) fn focus_send_info(
    app: &App,
    idx: usize,
    source_channel: MixerChannelId,
) -> controls::FocusSend {
    let mixer = &app.workspace.documents[idx].mixer;
    let source = mixer.channel_id_for_ui(source_channel);
    let target = app
        .mix
        .send_focus
        .map(|focus| mixer.channel_id_for_ui(focus));
    let (amount, pre_fader) = target
        .and_then(|target| mixer.channel_send(source, target))
        .map(|send| (send.amount, send.pre_fader))
        .unwrap_or((0.0, false));
    controls::FocusSend {
        source,
        target,
        amount,
        pre_fader,
    }
}

/// 混音节点通道条主体（MIDI / 音频 / 每轨 LC 共用）。
///
/// 三者只有标题、颜色与自动化宿主不同，其余（发送聚焦、推子/声像/M-S、
/// 节点写入）完全一致，统一在这里实现，避免三份复制粘贴各自漂移。
#[allow(clippy::too_many_arguments)]
pub(crate) fn node_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    node: MixerChannelId,
    color: egui::Color32,
    title: &str,
    target: controls::StripAutomationTarget,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) -> egui::Rect {
    let params = app.workspace.documents[idx].mixer.node_strip(node);
    let send = focus_send_info(app, idx, node);
    let selected = strip_selected(app, idx, node);
    strip_frame(
        ui,
        ("mix_strip", node),
        color,
        title,
        height,
        selected,
        |ui, _| {
            section(
                ui,
                send_height(0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    focus_send_block(ui, send, actions);
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(ui, &params, level, hold, true, true, target, actions) {
                actions.push(MixAction::SetNodeStrip { node, params: p });
            }
        },
    )
}

/// 单条 MIDI 通道条。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
pub(crate) fn channel_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    channel: u8,
    color: egui::Color32,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let node = MixerChannelId::Midi(channel);
    let title = crate::mix::channel_title(&app.workspace.documents[idx].mixer, channel);
    let rect = node_strip(
        app,
        ui,
        idx,
        node,
        color,
        &title,
        controls::StripAutomationTarget::Midi(channel),
        level,
        hold,
        height,
        actions,
    );
    // 双击色带：打开底部设备链并选中该通道。
    header_double_click_opens_dock(
        app,
        ui,
        idx,
        rect,
        crate::chrome::device_chain::DockContext::Midi(channel),
        ("mix_header_dock_midi", channel),
        actions,
    );
}

/// 音频通道条（多条音频轨共享同一音频通道 = 共享本条）。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
pub(crate) fn audio_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    channel: u16,
    display: u32,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let node = MixerChannelId::Audio(channel);
    let title = crate::mix::lc_label(display);
    let rect = node_strip(
        app,
        ui,
        idx,
        node,
        crate::theme::accent_active(),
        &title,
        controls::StripAutomationTarget::Audio(channel),
        level,
        hold,
        height,
        actions,
    );
    // 双击色带：打开底部设备链并选中该音频通道。
    header_double_click_opens_dock(
        app,
        ui,
        idx,
        rect,
        crate::chrome::device_chain::DockContext::Audio(channel),
        ("mix_header_dock_audio", channel),
        actions,
    );
}

/// 每轨独立 LC 条（Rack per-track 输出）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn track_lc_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    track: u16,
    lc: LogicalChannelId,
    display: u32,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let node = MixerChannelId::TrackLc(lc);
    let title = crate::mix::lc_label(display);
    let color = app.workspace.documents[idx]
        .edit
        .track_cache
        .colors
        .get(track as usize)
        .copied()
        .map(|c| crate::theme::rgba_to_color32((c[0], c[1], c[2], c[3])))
        .unwrap_or_else(crate::theme::accent_active);
    let rect = node_strip(
        app,
        ui,
        idx,
        node,
        color,
        &title,
        controls::StripAutomationTarget::TrackLc(lc),
        level,
        hold,
        height,
        actions,
    );
    header_double_click_opens_dock(
        app,
        ui,
        idx,
        rect,
        crate::chrome::device_chain::DockContext::TrackLc(lc),
        ("mix_header_dock_track_lc", lc.0),
        actions,
    );
}

/// 主输出条：固定高度电平表 + dB 读数（无 M/S/声像/发送）。
pub(crate) fn master_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let params = app.workspace.documents[idx].mixer.master.strip;
    let title = t!("mix.master").to_string();

    let selected = strip_selected(app, idx, MixerChannelId::Master);
    let rect = strip_frame(
        ui,
        "mix_strip_master",
        crate::theme::accent_active(),
        &title,
        height,
        selected,
        |ui, _| {
            if let Some(g) = master_controls(ui, params.gain, level, hold, actions) {
                actions.push(MixAction::SetMaster {
                    params: MasterParams { gain: g },
                });
            }
        },
    );
    header_double_click_opens_master_dock(app, ui, idx, rect, actions);
}

/// Master 条双击色带：打开设备链并切到主输出效果链（选中 Conductor 轨，
/// 让 dock 的“跟随选中轨”逻辑下一帧不会把语境覆盖回普通通道）。
pub(crate) fn header_double_click_opens_master_dock(
    app: &mut App,
    ui: &egui::Ui,
    idx: usize,
    rect: egui::Rect,
    actions: &mut Vec<MixAction>,
) {
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
    let resp = ui.interact(
        header,
        ui.scope_id().with("mix_strip_master_header"),
        egui::Sense::click(),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.header_double_click"));
    header_context_menu(
        app,
        &resp,
        idx,
        crate::chrome::device_chain::DockContext::Master,
        actions,
    );
    if resp.clicked() {
        focus_channel_info(app, MixerChannelId::Master);
    }
    if !resp.double_clicked() {
        return;
    }
    let split_hit_pad = crate::theme::SPLIT_GAP + 4.0;
    if ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| p.y < rect.min.y + split_hit_pad)
    {
        return;
    }
    crate::chrome::mode_bar::set_dock_visible(
        &mut app.show_pianoroll_in_arrange,
        &mut app.show_bottom_dock,
        &mut app.show_mixer_panel,
        &mut app.show_meter_panel,
        true,
    );
    app.layout_needs_save = true;
    let conductor = app.workspace.documents[idx].edit.track_cache.conductor_idx;
    let doc = &mut app.workspace.documents[idx];
    doc.edit.track_selected.clear();
    if let Some(ci) = conductor {
        doc.edit.track_selected.insert(ci);
    }
    app.dock_track = conductor;
    app.dock_context = Some(crate::chrome::device_chain::DockContext::Master);
    app.dock_selected = None;
}

/// 该混音通道是否被选中（同一套 `doc.edit.track_selected`，与 AR 行一致）。
pub(crate) fn strip_selected(app: &App, idx: usize, ch: MixerChannelId) -> bool {
    strip_selected_doc(&app.workspace.documents[idx], ch)
}

/// 纯函数版本（测试用）。
pub(crate) fn strip_selected_doc(
    doc: &yinhe_editor_core::document::Document,
    ch: MixerChannelId,
) -> bool {
    match ch {
        MixerChannelId::Midi(c) => doc.data.model.tracks.iter().enumerate().any(|(i, t)| {
            t.kind == yinhe_core::TrackKind::Midi
                && t.global_channel() == c
                && doc.edit.track_selected.contains(&(i as u16))
        }),
        MixerChannelId::Audio(c) => doc.data.model.tracks.iter().enumerate().any(|(i, t)| {
            t.kind == yinhe_core::TrackKind::Audio
                && t.audio_channel == Some(c)
                && doc.edit.track_selected.contains(&(i as u16))
        }),
        MixerChannelId::TrackLc(lc) => doc
            .data
            .model
            .tracks
            .iter()
            .enumerate()
            .any(|(i, t)| t.lc == Some(lc) && doc.edit.track_selected.contains(&(i as u16))),
        MixerChannelId::Master => doc
            .edit
            .track_cache
            .conductor_idx
            .is_some_and(|c| doc.edit.track_selected.contains(&c)),
    }
}

/// 点击通道条时把该通道对应的轨道设为选中（让通道条亮起，同时保持
/// 右栏「音轨属性」与 PR 当前轨一致）。
pub(crate) fn select_channel_tracks(app: &mut App, idx: usize, ch: MixerChannelId) {
    let anchor = {
        let doc = &mut app.workspace.documents[idx];
        doc.edit.track_selected.clear();
        match ch {
            MixerChannelId::Midi(c) => {
                for (i, t) in doc.data.model.tracks.iter().enumerate() {
                    if t.kind == yinhe_core::TrackKind::Midi && t.global_channel() == c {
                        doc.edit.track_selected.insert(i as u16);
                    }
                }
            }
            MixerChannelId::Audio(c) => {
                for (i, t) in doc.data.model.tracks.iter().enumerate() {
                    if t.kind == yinhe_core::TrackKind::Audio && t.audio_channel == Some(c) {
                        doc.edit.track_selected.insert(i as u16);
                    }
                }
            }
            MixerChannelId::TrackLc(lc) => {
                for (i, t) in doc.data.model.tracks.iter().enumerate() {
                    if t.lc == Some(lc) {
                        doc.edit.track_selected.insert(i as u16);
                    }
                }
            }
            MixerChannelId::Master => {
                if let Some(c) = doc.edit.track_cache.conductor_idx {
                    doc.edit.track_selected.insert(c);
                }
            }
        }
        doc.edit.track_selected.iter().min().copied()
    };
    app.track_selection_anchor = anchor;
}

/// 色带双击：打开底部设备链（dock）并选中该通道（通道条 / 音频条共用）。
/// 同步 `dock_track` 防止下一帧被「跟随选中轨」覆盖；总线/主输出没有 dock
/// 语境，不调用本函数。
pub(crate) fn header_double_click_opens_dock(
    app: &mut App,
    ui: &egui::Ui,
    idx: usize,
    rect: egui::Rect,
    context: crate::chrome::device_chain::DockContext,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    actions: &mut Vec<MixAction>,
) {
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
    let resp = ui.interact(header, ui.scope_id().with(id_salt), egui::Sense::click());
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.header_double_click"));
    header_context_menu(app, &resp, idx, context, actions);
    if resp.clicked() {
        app.mix.send_focus = Some(context);
        select_channel_tracks(app, idx, context);
        focus_channel_info(app, context);
    }
    if !resp.double_clicked() {
        return;
    }
    // 分割条命中区（面板上方 2px 缝隙 + 向面板内下探 4px，见
    // `widgets::split_handle` 的 HIT_PAD）内不处理色带双击：
    // 让「双击分割条复位面板高度」优先，避免误触打开设备链。
    let split_hit_pad = crate::theme::SPLIT_GAP + 4.0;
    if ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| p.y < rect.min.y + split_hit_pad)
    {
        return;
    }
    // 展开设备链（与钢琴卷帘/混音台互斥），并选中该通道。
    crate::chrome::mode_bar::set_dock_visible(
        &mut app.show_pianoroll_in_arrange,
        &mut app.show_bottom_dock,
        &mut app.show_mixer_panel,
        &mut app.show_meter_panel,
        true,
    );
    app.layout_needs_save = true;
    let selected_track = app.workspace.documents[idx]
        .edit
        .track_selected
        .iter()
        .min()
        .copied();
    app.dock_track = selected_track;
    app.dock_context = Some(context);
    app.dock_selected = None;
}

/// 把右侧栏切到该通道的属性面板。
pub(crate) fn focus_channel_info(app: &mut App, ch: MixerChannelId) {
    app.info_content = Some(crate::right_panel::InfoContent::Channel(ch));
    app.right_tab = Some(crate::right_panel::RightTab::Info);
    app.right_panel_layout
        .focus_tab(yinhe_editor_core::right_panel_layout::PanelKind::Channel);
}

/// 展开底部设备链并定位到通道语境（双击/右键共用）。
pub(crate) fn open_dock_context(
    app: &mut App,
    idx: usize,
    context: crate::chrome::device_chain::DockContext,
) {
    crate::chrome::mode_bar::set_dock_visible(
        &mut app.show_pianoroll_in_arrange,
        &mut app.show_bottom_dock,
        &mut app.show_mixer_panel,
        &mut app.show_meter_panel,
        true,
    );
    app.layout_needs_save = true;
    let selected_track = app.workspace.documents[idx]
        .edit
        .track_selected
        .iter()
        .min()
        .copied();
    app.dock_track = selected_track;
    app.dock_context = Some(context);
    app.dock_selected = None;
}

/// 通道条色带右键菜单：通道属性 / 设备链 / 自动化 / 重置 / 删除总线。
pub(crate) fn header_context_menu(
    app: &mut App,
    resp: &egui::Response,
    idx: usize,
    context: crate::chrome::device_chain::DockContext,
    actions: &mut Vec<MixAction>,
) {
    let ch = context;
    crate::widgets::menu::context_menu(resp, 180.0, |ui| {
        crate::widgets::menu::action(ui, t!("panel.section.channel").to_string(), "", |_| {
            focus_channel_info(app, ch)
        });
        crate::widgets::menu::action(ui, t!("arrange.open_device_chain").to_string(), "", |_| {
            open_dock_context(app, idx, context)
        });
        ui.separator();
        if let Some(action) = volume_automation(ch) {
            crate::widgets::menu::action(
                ui,
                t!("mix.add_volume_automation").to_string(),
                "",
                |_| actions.push(action),
            );
        }
        if ch != MixerChannelId::Master
            && let Some(action) = pan_automation(ch)
        {
            crate::widgets::menu::action(ui, t!("mix.add_pan_automation").to_string(), "", |_| {
                actions.push(action)
            });
        }
        let mixed_lc = {
            let target = app.workspace.documents[idx].mixer.channel_id_for_ui(ch);
            app.workspace.documents[idx]
                .mixer
                .channel_sends
                .iter()
                .any(|send| send.target == target)
        };
        if !mixed_lc
            && matches!(
                ch,
                MixerChannelId::Midi(_) | MixerChannelId::Audio(_) | MixerChannelId::TrackLc(_)
            )
        {
            let node = app.workspace.documents[idx].mixer.node_id_for(ch);
            let frozen = app.workspace.documents[idx]
                .mixer
                .node(node)
                .is_some_and(|node| node.freeze.is_some());
            let stale = app.workspace.documents[idx].freeze_is_stale(node);
            ui.separator();
            if frozen {
                crate::widgets::menu::action(ui, t!("mix.unfreeze").to_string(), "", |_| {
                    actions.push(MixAction::UnfreezeChannel { node })
                });
            }
            let label = if stale {
                t!("mix.refreeze")
            } else {
                t!("mix.freeze")
            };
            crate::widgets::menu::action(ui, label.to_string(), "", |_| {
                actions.push(MixAction::FreezeChannel { channel: ch })
            });
        }
        ui.separator();
        crate::widgets::menu::action(ui, t!("mix.reset_gain").to_string(), "", |_| {
            actions.push(reset_gain_action(app, idx, ch))
        });
        if ch != MixerChannelId::Master {
            crate::widgets::menu::action(ui, t!("mix.reset_pan").to_string(), "", |_| {
                if let Some(action) = reset_pan_action(app, idx, ch) {
                    actions.push(action);
                }
            });
        }
    });
}

pub(crate) fn volume_automation(ch: MixerChannelId) -> Option<MixAction> {
    Some(MixAction::AddMixerGainAutomation { channel: ch })
}

pub(crate) fn pan_automation(ch: MixerChannelId) -> Option<MixAction> {
    (ch != MixerChannelId::Master).then_some(MixAction::AddMixerPanAutomation { channel: ch })
}

pub(crate) fn reset_gain_action(app: &App, idx: usize, ch: MixerChannelId) -> MixAction {
    let mixer = &app.workspace.documents[idx].mixer;
    match ch {
        MixerChannelId::Midi(channel) => {
            let mut params = mixer.strip(channel);
            params.gain = 1.0;
            MixAction::SetNodeStrip {
                node: MixerChannelId::Midi(channel),
                params,
            }
        }
        MixerChannelId::Audio(channel) => {
            let mut params = mixer.audio_strip(channel);
            params.gain = 1.0;
            MixAction::SetNodeStrip {
                node: MixerChannelId::Audio(channel),
                params,
            }
        }
        MixerChannelId::TrackLc(id) => {
            let mut params = mixer.node_strip(MixerChannelId::TrackLc(id));
            params.gain = 1.0;
            MixAction::SetNodeStrip {
                node: MixerChannelId::TrackLc(id),
                params,
            }
        }
        MixerChannelId::Master => MixAction::SetMaster {
            params: MasterParams { gain: 1.0 },
        },
    }
}

pub(crate) fn reset_pan_action(app: &App, idx: usize, ch: MixerChannelId) -> Option<MixAction> {
    let mixer = &app.workspace.documents[idx].mixer;
    match ch {
        MixerChannelId::Midi(channel) => {
            let mut params = mixer.strip(channel);
            params.pan = 0.0;
            Some(MixAction::SetNodeStrip {
                node: MixerChannelId::Midi(channel),
                params,
            })
        }
        MixerChannelId::Audio(channel) => {
            let mut params = mixer.audio_strip(channel);
            params.pan = 0.0;
            Some(MixAction::SetNodeStrip {
                node: MixerChannelId::Audio(channel),
                params,
            })
        }
        MixerChannelId::TrackLc(id) => {
            let mut params = mixer.node_strip(MixerChannelId::TrackLc(id));
            params.pan = 0.0;
            Some(MixAction::SetNodeStrip {
                node: MixerChannelId::TrackLc(id),
                params,
            })
        }
        MixerChannelId::Master => None,
    }
}
