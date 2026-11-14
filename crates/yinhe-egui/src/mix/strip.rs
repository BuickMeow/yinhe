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

use crate::app::App;
use crate::widgets::meter::{METER_GAP, METER_W, SCALE_W};

use super::{MixAction, channel_label};
use controls::{
    SendSource, master_controls, section, send_block, send_height, strip_controls, strip_frame,
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
    let params = app.workspace.documents[idx].mixer.strip(channel);
    let bus_count = app.workspace.documents[idx].mixer.bus_count();
    let sends = app.workspace.documents[idx]
        .mixer
        .sends
        .get(channel as usize)
        .cloned()
        .unwrap_or_default();
    let title = channel_label(channel);

    let rect = strip_frame(
        ui,
        ("mix_strip_midi", channel),
        color,
        &title,
        height,
        |ui, _| {
            section(
                ui,
                send_height(bus_count),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    send_block(ui, SendSource::Channel(channel), &sends, bus_count, actions);
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(ui, &params, level, hold, true, true) {
                actions.push(MixAction::SetStrip { channel, params: p });
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
    let params = app.workspace.documents[idx].mixer.audio_strip(channel);
    let title = crate::mix::audio_label(channel);
    let audio_sends = app.workspace.documents[idx]
        .mixer
        .audio_sends
        .get(channel as usize)
        .cloned()
        .unwrap_or_default();
    let bus_count = app.workspace.documents[idx].mixer.bus_count();

    let rect = strip_frame(
        ui,
        ("mix_strip_audio", channel),
        crate::theme::accent_active(),
        &title,
        height,
        |ui, _| {
            section(
                ui,
                send_height(bus_count),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    send_block(
                        ui,
                        SendSource::Audio(channel),
                        &audio_sends,
                        bus_count,
                        actions,
                    );
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(ui, &params, level, hold, true, true) {
                actions.push(MixAction::SetAudioStrip { channel, params: p });
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
    );
}

/// 总线条（bus / return）：与通道条同构，额外在色带右侧提供删除入口。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
pub(crate) fn bus_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    bus: u8,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let params = app.workspace.documents[idx]
        .mixer
        .buses
        .get(bus as usize)
        .copied()
        .unwrap_or_default();
    let title = format!("BUS {}", bus + 1);
    let bus_sends = app.workspace.documents[idx]
        .mixer
        .bus_sends
        .get(bus as usize)
        .cloned()
        .unwrap_or_default();
    let bus_count = app.workspace.documents[idx].mixer.bus_count();

    let rect = strip_frame(
        ui,
        ("mix_strip_bus", bus),
        crate::theme::accent_active(),
        &title,
        height,
        |ui, _| {
            section(
                ui,
                send_height(bus_count),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    send_block(ui, SendSource::Bus(bus), &bus_sends, bus_count, actions);
                },
            );
            ui.add_space(GAP);
            if let Some(p) = strip_controls(ui, &params, level, hold, true, true) {
                actions.push(MixAction::SetBusStrip { bus, params: p });
            }
        },
    );

    // 色带行右侧：删除总线按钮（与标题同一行）。图标取色与色带文字同源
    // （色带对比色），悬停转危险色。
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
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
    let params = app.workspace.documents[idx].mixer.master;
    let title = t!("mix.master").to_string();

    strip_frame(
        ui,
        "mix_strip_master",
        crate::theme::accent_active(),
        &title,
        height,
        |ui, _| {
            if let Some(g) = master_controls(ui, params.gain, level, hold) {
                actions.push(MixAction::SetMaster {
                    params: MasterParams { gain: g },
                });
            }
        },
    );
    header_double_click_opens_master_dock(app, ui, idx, ui.max_rect());
}

/// Master 条双击色带：打开设备链并切到主输出效果链（选中 Conductor 轨，
/// 让 dock 的“跟随选中轨”逻辑下一帧不会把语境覆盖回普通通道）。
fn header_double_click_opens_master_dock(
    app: &mut App,
    ui: &egui::Ui,
    idx: usize,
    rect: egui::Rect,
) {
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
    let resp = ui.interact(
        header,
        ui.scope_id().with("mix_strip_master_header"),
        egui::Sense::click(),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.header_double_click"));
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
) {
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
    let resp = ui.interact(header, ui.scope_id().with(id_salt), egui::Sense::click());
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.header_double_click"));
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
