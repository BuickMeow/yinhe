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
use yinhe_types::{BusId, ChannelId, LogicalChannelId, MixerChannelId};

use crate::app::App;
use crate::widgets::meter::{METER_GAP, METER_W, SCALE_W};

use super::MixAction;
use controls::{
    SendSource, focus_send_block, master_controls, section, send_block, send_height,
    strip_controls, strip_frame,
};

mod controls;
mod picker;

#[cfg(test)]
mod tests;

pub(crate) use controls::auto_panel_height;
pub(crate) use picker::{
    PickerTab, instrument_picker, picker_viewport_id, plugin_picker, send_popup, sends_viewport_id,
};

/// 通道条宽度（px）。
pub(crate) const STRIP_WIDTH: f32 = 88.0;
/// 通道条间距：0——相邻通道由各自的描边分隔，形成网格。
pub(crate) const STRIP_GAP: f32 = 0.0;
/// 内容左右边距（px）。
const PAD_X: f32 = 6.0;
/// 内容宽度（px）。
const CONTENT_W: f32 = STRIP_WIDTH - PAD_X * 2.0;
/// 顶部色带高度（px）。
const HEADER_H: f32 = 20.0;
/// 通用按钮圆角（px）。
const RADIUS: f32 = 4.0;
/// 小按钮边长（px，M/S、图标按钮）。
const BTN: f32 = 18.0;
/// 旋钮直径（px，发送/声像）。
const KNOB_D: f32 = 26.0;
/// 分区之间的竖直间隙（px）。
const GAP: f32 = 4.0;
/// 发送区一行高度（px）。
const SEND_ROW_H: f32 = KNOB_D;
/// dB 读数行高（px）。
const DB_ROW_H: f32 = 18.0;
/// 电平表 + 右侧按钮的固定模块高度（px）。不再用剩余高度拉伸。
const METER_MODULE_H: f32 = 140.0;
/// 固定模块右侧控制列宽度：dB / 声像 / M / S。
const CONTROLS_W: f32 = CONTENT_W - (SCALE_W + METER_W * 2.0 + METER_GAP) - GAP;
/// 卡片上下内边距（色带下 4px、卡片底 6px）。
const BODY_TOP_PAD: f32 = 4.0;
const BODY_BOTTOM_PAD: f32 = 6.0;

fn focus_send_info(
    app: &App,
    idx: usize,
    source_channel: MixerChannelId,
) -> (Option<ChannelId>, Option<ChannelId>, String, f32, bool) {
    let mixer = &app.workspace.documents[idx].mixer;
    let source = crate::mix::channel_id_for_ui(mixer, source_channel);
    let target = app
        .mix
        .send_focus
        .and_then(|focus| crate::mix::channel_id_for_ui(mixer, focus));
    let label = target.map(crate::mix::channel_id_label).unwrap_or_default();
    let (amount, pre_fader) = source
        .zip(target)
        .and_then(|(source, target)| mixer.channel_send(source, target))
        .map(|send| (send.amount, send.pre_fader))
        .unwrap_or((0.0, false));
    (source, target, label, amount, pre_fader)
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
    let params = app.workspace.documents[idx]
        .mixer
        .node_strip(MixerChannelId::Midi(channel));
    let (send_source, send_target, send_label, send_amount, send_pre) =
        focus_send_info(app, idx, MixerChannelId::Midi(channel));
    let title = crate::mix::channel_title(&app.workspace.documents[idx].mixer, channel);

    let rect = strip_frame(
        ui,
        ("mix_strip_midi", channel),
        color,
        &title,
        height,
        |ui, _| {
            section(
                ui,
                send_height(0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    focus_send_block(
                        ui,
                        send_source,
                        send_target,
                        &send_label,
                        send_amount,
                        send_pre,
                        actions,
                    );
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(
                ui,
                &params,
                level,
                hold,
                true,
                true,
                controls::StripAutomationTarget::Midi(channel),
                actions,
            ) {
                actions.push(MixAction::SetNodeStrip {
                    node: MixerChannelId::Midi(channel),
                    params: p,
                });
            }
        },
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
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let params = app.workspace.documents[idx]
        .mixer
        .node_strip(MixerChannelId::Audio(channel));
    let title = crate::mix::audio_label(channel);
    let (send_source, send_target, send_label, send_amount, send_pre) =
        focus_send_info(app, idx, MixerChannelId::Audio(channel));

    let rect = strip_frame(
        ui,
        ("mix_strip_audio", channel),
        crate::theme::accent_active(),
        &title,
        height,
        |ui, _| {
            section(
                ui,
                send_height(0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    focus_send_block(
                        ui,
                        send_source,
                        send_target,
                        &send_label,
                        send_amount,
                        send_pre,
                        actions,
                    );
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(
                ui,
                &params,
                level,
                hold,
                true,
                true,
                controls::StripAutomationTarget::Audio(channel),
                actions,
            ) {
                actions.push(MixAction::SetNodeStrip {
                    node: MixerChannelId::Audio(channel),
                    params: p,
                });
            }
        },
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
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let node = MixerChannelId::TrackLc(lc);
    let mixer = &app.workspace.documents[idx].mixer;
    let params = mixer.node_strip(node);
    let (send_source, send_target, send_label, send_amount, send_pre) =
        focus_send_info(app, idx, node);
    let track_name = app.workspace.documents[idx]
        .data
        .model
        .tracks
        .get(track as usize)
        .map(|t| t.name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Track {}", track + 1));
    let color = app.workspace.documents[idx]
        .edit
        .track_cache
        .colors
        .get(track as usize)
        .copied()
        .map(|c| crate::theme::rgba_to_color32((c[0], c[1], c[2], c[3])))
        .unwrap_or_else(crate::theme::accent_active);

    let rect = strip_frame(
        ui,
        ("mix_strip_track_lc", lc.0),
        color,
        &track_name,
        height,
        |ui, _| {
            section(
                ui,
                send_height(0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    focus_send_block(
                        ui,
                        send_source,
                        send_target,
                        &send_label,
                        send_amount,
                        send_pre,
                        actions,
                    );
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(
                ui,
                &params,
                level,
                hold,
                true,
                true,
                controls::StripAutomationTarget::TrackLc(lc),
                actions,
            ) {
                actions.push(MixAction::SetNodeStrip { node, params: p });
            }
        },
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

/// 总线条（bus / return）：与通道条同构，额外在色带右侧提供删除入口。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
pub(crate) fn bus_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    bus: BusId,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let mixer = &app.workspace.documents[idx].mixer;
    let node = MixerChannelId::Bus(bus);
    let params = mixer.node_strip(node);
    let title = format!("BUS {}", bus.index() + 1);
    let bus_sends = mixer
        .node_sends(node)
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    let bus_ids = mixer.bus_ids();

    let rect = strip_frame(
        ui,
        ("mix_strip_bus", bus),
        crate::theme::accent_active(),
        &title,
        height,
        |ui, _| {
            section(
                ui,
                send_height(bus_ids.len()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    send_block(ui, SendSource::Bus(bus), &bus_sends, &bus_ids, actions);
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(
                ui,
                &params,
                level,
                hold,
                true,
                true,
                controls::StripAutomationTarget::Bus(bus),
                actions,
            ) {
                actions.push(MixAction::SetNodeStrip {
                    node: MixerChannelId::Bus(bus),
                    params: p,
                });
            }
        },
    );

    // 色带行右侧：删除总线按钮（与标题同一行）。图标取色与色带文字同源
    // （色带对比色），悬停转危险色。
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
    let header_resp = ui.interact(
        header,
        ui.scope_id().with(("mix_bus_header", bus)),
        egui::Sense::click(),
    );
    header_context_menu(
        app,
        &header_resp,
        idx,
        crate::chrome::device_chain::DockContext::Bus(bus),
        actions,
    );
    if header_resp.clicked() {
        focus_channel_info(app, MixerChannelId::Bus(bus));
    }
    let del_rect = egui::Rect::from_center_size(
        egui::pos2(header.max.x - PAD_X - 6.0, header.center().y),
        egui::vec2(14.0, 14.0),
    );
    let del_resp = ui.interact(
        del_rect,
        ui.scope_id().with(("mix_bus_remove", bus)),
        egui::Sense::click(),
    );
    let del_color = if del_resp.hovered() {
        crate::theme::danger_text_bright()
    } else {
        crate::theme::contrast_text(crate::theme::accent_active())
    };
    ui.painter().text(
        del_rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_material_icons::icons::ICON_DELETE.codepoint,
        egui::FontId::new(
            crate::theme::ICON_FONT_SM,
            egui_material_icons::icons::ICON_DELETE.font_family(),
        ),
        del_color,
    );
    crate::widgets::hint::hover(ui.ctx(), &del_resp, t!("hint.mix.remove_bus"));
    if del_resp.clicked() {
        actions.push(MixAction::RemoveBus { bus });
    }
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

    let rect = strip_frame(
        ui,
        "mix_strip_master",
        crate::theme::accent_active(),
        &title,
        height,
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
fn header_double_click_opens_master_dock(
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

/// 色带双击：打开底部设备链（dock）并选中该通道（通道条 / 音频条共用）。
/// 同步 `dock_track` 防止下一帧被「跟随选中轨」覆盖；总线/主输出没有 dock
/// 语境，不调用本函数。
fn header_double_click_opens_dock(
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
fn focus_channel_info(app: &mut App, ch: MixerChannelId) {
    app.info_content = Some(crate::right_panel::InfoContent::Channel(ch));
    app.right_tab = Some(crate::right_panel::RightTab::Info);
    app.right_panel_layout
        .focus_tab(yinhe_editor_core::right_panel_layout::PanelKind::Channel);
}

/// 展开底部设备链并定位到通道语境（双击/右键共用）。
fn open_dock_context(app: &mut App, idx: usize, context: crate::chrome::device_chain::DockContext) {
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
fn header_context_menu(
    app: &mut App,
    resp: &egui::Response,
    idx: usize,
    context: crate::chrome::device_chain::DockContext,
    actions: &mut Vec<MixAction>,
) {
    let ch = context;
    crate::widgets::menu::context_menu(resp, 180.0, |ui| {
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("panel.section.channel").as_ref(),
                "",
            ))
            .clicked()
        {
            focus_channel_info(app, ch);
            ui.close();
        }
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.open_device_chain").as_ref(),
                "",
            ))
            .clicked()
        {
            open_dock_context(app, idx, context);
            ui.close();
        }
        ui.separator();
        if let Some(action) = volume_automation(ch)
            && ui
                .add(crate::widgets::menu::menu_item_action(
                    ui,
                    false,
                    t!("mix.add_volume_automation").as_ref(),
                    "",
                ))
                .clicked()
        {
            actions.push(action);
            ui.close();
        }
        if ch != MixerChannelId::Master
            && let Some(action) = pan_automation(ch)
            && ui
                .add(crate::widgets::menu::menu_item_action(
                    ui,
                    false,
                    t!("mix.add_pan_automation").as_ref(),
                    "",
                ))
                .clicked()
        {
            actions.push(action);
            ui.close();
        }
        if matches!(
            ch,
            MixerChannelId::Midi(_) | MixerChannelId::Audio(_) | MixerChannelId::TrackLc(_)
        ) {
            let node = ch.node_id();
            let frozen = app.workspace.documents[idx]
                .mixer
                .node(node)
                .is_some_and(|node| node.freeze.is_some());
            let stale = app.workspace.documents[idx].freeze_is_stale(node);
            ui.separator();
            if frozen
                && ui
                    .add(crate::widgets::menu::menu_item_action(
                        ui,
                        false,
                        t!("mix.unfreeze").as_ref(),
                        "",
                    ))
                    .clicked()
            {
                actions.push(MixAction::UnfreezeChannel { node });
                ui.close();
            }
            let label = if stale {
                t!("mix.refreeze")
            } else {
                t!("mix.freeze")
            };
            if ui
                .add(crate::widgets::menu::menu_item_action(
                    ui,
                    false,
                    label.as_ref(),
                    "",
                ))
                .clicked()
            {
                actions.push(MixAction::FreezeChannel { channel: ch });
                ui.close();
            }
        }
        ui.separator();
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("mix.reset_gain").as_ref(),
                "",
            ))
            .clicked()
        {
            actions.push(reset_gain_action(app, idx, ch));
            ui.close();
        }
        if ch != MixerChannelId::Master
            && ui
                .add(crate::widgets::menu::menu_item_action(
                    ui,
                    false,
                    t!("mix.reset_pan").as_ref(),
                    "",
                ))
                .clicked()
            && let Some(action) = reset_pan_action(app, idx, ch)
        {
            actions.push(action);
            ui.close();
        }
        if let MixerChannelId::Bus(bus) = ch {
            ui.separator();
            if ui
                .add(crate::widgets::menu::menu_item_action(
                    ui,
                    false,
                    t!("arrange.remove_bus").as_ref(),
                    "",
                ))
                .clicked()
            {
                actions.push(MixAction::RemoveBus { bus });
                ui.close();
            }
        }
    });
}

fn volume_automation(ch: MixerChannelId) -> Option<MixAction> {
    Some(MixAction::AddMixerGainAutomation { channel: ch })
}

fn pan_automation(ch: MixerChannelId) -> Option<MixAction> {
    (ch != MixerChannelId::Master).then_some(MixAction::AddMixerPanAutomation { channel: ch })
}

fn reset_gain_action(app: &App, idx: usize, ch: MixerChannelId) -> MixAction {
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
        MixerChannelId::Bus(bus) => {
            let mut params = mixer.bus_strip(bus);
            params.gain = 1.0;
            MixAction::SetNodeStrip {
                node: MixerChannelId::Bus(bus),
                params,
            }
        }
        MixerChannelId::Master => MixAction::SetMaster {
            params: MasterParams { gain: 1.0 },
        },
    }
}

fn reset_pan_action(app: &App, idx: usize, ch: MixerChannelId) -> Option<MixAction> {
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
        MixerChannelId::Bus(bus) => {
            let mut params = mixer.bus_strip(bus);
            params.pan = 0.0;
            Some(MixAction::SetNodeStrip {
                node: MixerChannelId::Bus(bus),
                params,
            })
        }
        MixerChannelId::Master => None,
    }
}
