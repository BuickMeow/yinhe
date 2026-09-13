//! 混音台通道条控件：轨道色条 / 标签 / insert / M-S / 声像 / 推子 / 电平表。
//!
//! 视觉参考 Bitwig / Studio One：顶部轨道色条、紧凑标签、自绘推子
//! （手柄 + 0dB 刻度 + 双击归零）、自绘电平表（绿黄红 + 0dB 刻度）、
//! insert 空态收缩为一行。通道条撑满混音台高度，底部推子区从下往上排，
//! insert 高度变化不影响推子对齐。

use eframe::egui;
use rust_i18n::t;
use yinhe_mixer::{MasterParams, StripParams};

use crate::app::App;

use super::{MixAction, channel_label, db_to_gain, gain_to_db};

/// 通道条宽度（px）。
pub(crate) const STRIP_WIDTH: f32 = 72.0;
/// 通道条间距（px）。
pub(crate) const STRIP_GAP: f32 = 4.0;
/// 内容左右边距（px）。
const PAD_X: i8 = 6;
/// 内容宽度（px）。
const CONTENT_W: f32 = STRIP_WIDTH - PAD_X as f32 * 2.0;
/// 顶部轨道色条高度（px）。
const COLOR_BAR_H: f32 = 4.0;
/// insert 区最大高度（px）。
const INSERT_MAX_H: f32 = 72.0;
/// insert 行高（px）。
const INSERT_ROW_H: f32 = 18.0;
/// 推子槽宽（px）。
const FADER_SLOT_W: f32 = 4.0;
/// 推子手柄尺寸（px）。
const FADER_HANDLE_W: f32 = 24.0;
const FADER_HANDLE_H: f32 = 12.0;
/// 电平表单条宽 + 条间距（px）。
const METER_W: f32 = 5.0;
const METER_GAP: f32 = 2.0;
/// M/S 按钮边长（px）。
const BTN: f32 = 18.0;
/// 推子/电平表 dB 范围。
const DB_MIN: f32 = -60.0;
const DB_MAX: f32 = 6.0;
/// 推子最小高度（px）。
const FADER_MIN_H: f32 = 64.0;
/// 推子之下的固定区高度（M/S + 声像 + dB 读数 + 间距）。
const BOTTOM_H: f32 = 62.0;

/// dB 值 → 纵向占比（0 = DB_MIN，1 = DB_MAX）。
fn db_frac(db: f32) -> f32 {
    ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0)
}

/// 顶部工具条：扫描插件 + 状态信息。
pub(crate) fn show_toolbar(app: &mut App, ui: &mut egui::Ui, actions: &mut Vec<MixAction>) {
    ui.horizontal(|ui| {
        if ui.button(t!("mix.scan_plugins")).clicked() {
            actions.push(MixAction::RescanPlugins);
        }
        if let Some(plugins) = &app.mix.scanned {
            let effects = plugins.iter().filter(|p| p.is_audio_effect()).count();
            ui.label(
                egui::RichText::new(t!("mix.scan_status", count = effects))
                    .small()
                    .color(crate::theme::text_secondary()),
            );
            if app.mix.scan_errors > 0 {
                ui.label(
                    egui::RichText::new(t!("mix.scan_errors", count = app.mix.scan_errors))
                        .small()
                        .color(crate::theme::warning_gold()),
                );
            }
        }
        // 机架最近一次错误（加载/激活失败）。
        if let Some(idx) = app.workspace.active_doc
            && let Some(err) = app.mixer_racks.get(idx).and_then(|r| r.last_error.as_ref())
        {
            ui.label(
                egui::RichText::new(err)
                    .small()
                    .color(crate::theme::danger_text()),
            );
        }
    });
    ui.separator();
}

/// 单条通道条。`peak` 是 UI 侧衰减后的 (L, R) 峰值；`color` 是轨道色条颜色；
/// `height` 为撑满混音台的目标高度。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
pub(crate) fn channel_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    channel: u8,
    track_names: &[String],
    color: egui::Color32,
    peak: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let params = app.workspace.documents[idx].mixer.strip(channel);
    let names = track_names.join(", ");
    let insert_names: Vec<String> = app.workspace.documents[idx].mixer.channel_inserts
        [channel as usize]
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let bypassed: Vec<bool> = app.workspace.documents[idx].mixer.channel_inserts[channel as usize]
        .iter()
        .map(|r| r.bypassed)
        .collect();
    let gui_open: Vec<bool> = app
        .mixer_racks
        .get(idx)
        .map(|rack| {
            rack.chain(Some(channel))
                .iter()
                .map(|rt| rt.gui_open)
                .collect()
        })
        .unwrap_or_default();

    strip_frame(ui, color, height, |ui| {
        label_block(ui, channel_label(channel), &names);

        ui.add_space(4.0);
        insert_area(
            ui,
            Some(channel),
            &insert_names,
            &bypassed,
            &gui_open,
            actions,
        );
        ui.add_space(4.0);

        // 推子占满 insert 之下的剩余空间（底部固定区之外），
        // 因此 insert 高度变化只影响推子顶部，底部对齐不变。
        let fader_h = (ui.available_height() - BOTTOM_H).max(FADER_MIN_H);
        fader_and_meter(ui, params.gain, peak, fader_h, |gain| {
            let mut p = params;
            p.gain = gain;
            actions.push(MixAction::SetStrip { channel, params: p });
        });
        ui.add_space(4.0);
        ms_pan_block(ui, &params, |new_params| {
            actions.push(MixAction::SetStrip {
                channel,
                params: new_params,
            });
        });
        ui.add_space(4.0);
        db_label(ui, params.gain);
    });
}

/// 主输出条。
pub(crate) fn master_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    peak: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let params = app.workspace.documents[idx].mixer.master;
    let insert_names: Vec<String> = app.workspace.documents[idx]
        .mixer
        .master_inserts
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let bypassed: Vec<bool> = app.workspace.documents[idx]
        .mixer
        .master_inserts
        .iter()
        .map(|r| r.bypassed)
        .collect();
    let gui_open: Vec<bool> = app
        .mixer_racks
        .get(idx)
        .map(|rack| rack.chain(None).iter().map(|rt| rt.gui_open).collect())
        .unwrap_or_default();

    strip_frame(ui, crate::theme::accent_active(), height, |ui| {
        label_block(ui, t!("mix.master").to_string(), "");

        ui.add_space(4.0);
        insert_area(ui, None, &insert_names, &bypassed, &gui_open, actions);
        ui.add_space(4.0);

        let fader_h = (ui.available_height() - BOTTOM_H).max(FADER_MIN_H);
        fader_and_meter(ui, params.gain, peak, fader_h, |gain| {
            actions.push(MixAction::SetMaster {
                params: MasterParams { gain },
            });
        });
        ui.add_space(4.0);
        // master 无 M/S/声像：按 ms_pan_block 实际高度占位，dB 与通道条对齐。
        let ms_h = BTN + ui.spacing().item_spacing.y + 14.0;
        ui.allocate_space(egui::vec2(CONTENT_W, ms_h));
        ui.add_space(4.0);
        db_label(ui, params.gain);
    });
}

/// 通道条外框：固定 STRIP_WIDTH×height 尺寸分配（否则 Frame 会占满外层
/// 剩余宽度），顶部轨道色条 + 内容区。
fn strip_frame(
    ui: &mut egui::Ui,
    color: egui::Color32,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(STRIP_WIDTH, height),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            egui::Frame::new()
                .fill(crate::theme::control_bg())
                .stroke(egui::Stroke::new(1.0, crate::theme::grid_sub_beat()))
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.set_min_height(height);
                    // 顶部轨道色条：贴顶，上角跟随外框圆角。
                    let (bar, _) = ui.allocate_exact_size(
                        egui::vec2(STRIP_WIDTH, COLOR_BAR_H),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(
                        bar,
                        egui::CornerRadius {
                            nw: 4,
                            ne: 4,
                            sw: 0,
                            se: 0,
                        },
                        color,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: PAD_X,
                            right: PAD_X,
                            top: 4,
                            bottom: 6,
                        })
                        .show(ui, |ui| {
                            ui.set_min_width(CONTENT_W);
                            add_contents(ui);
                        });
                });
        },
    );
}

/// 标签区：通道号（强）+ 轨道名（小字截断，hover 显示全名）。
fn label_block(ui: &mut egui::Ui, title: String, names: &str) {
    ui.label(
        egui::RichText::new(title)
            .strong()
            .size(crate::theme::BODY_FONT)
            .color(crate::theme::text_bright()),
    );
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(names)
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_secondary()),
        )
        .truncate(),
    );
    if !names.is_empty() {
        resp.on_hover_text(names);
    }
}

/// insert 槽位区：空态收缩为一行「+」；有内容时紧凑列表（最多 INSERT_MAX_H）。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
fn insert_area(
    ui: &mut egui::Ui,
    channel: Option<u8>,
    names: &[String],
    bypassed: &[bool],
    gui_open: &[bool],
    actions: &mut Vec<MixAction>,
) {
    if names.is_empty() {
        // 空态：一行弱框 + 居中「+」，点击打开插件选择器。
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(CONTENT_W, INSERT_ROW_H), egui::Sense::click());
        let painter = ui.painter();
        let bg = if resp.hovered() {
            crate::theme::hover_color(crate::theme::btn_bg())
        } else {
            crate::theme::btn_bg().gamma_multiply(0.6)
        };
        painter.rect_filled(rect, 3.0, bg);
        painter.rect_stroke(
            rect,
            3.0,
            egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
            egui::StrokeKind::Inside,
        );
        let add = egui_material_icons::icons::ICON_ADD;
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            add.codepoint,
            egui::FontId::new(crate::theme::ICON_FONT_SM, add.font_family()),
            crate::theme::text_muted(),
        );
        if resp.on_hover_text(t!("mix.add_insert_hint")).clicked() {
            actions.push(MixAction::OpenPicker { channel });
        }
        return;
    }

    egui::Frame::new()
        .fill(crate::theme::track_bg())
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(2, 2))
        .show(ui, |ui| {
            ui.set_width(CONTENT_W - 4.0);
            egui::ScrollArea::vertical()
                .max_height(INSERT_MAX_H)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (slot, name) in names.iter().enumerate() {
                        insert_row(
                            ui,
                            channel,
                            slot,
                            name,
                            bypassed.get(slot).copied().unwrap_or(false),
                            gui_open.get(slot).copied().unwrap_or(false),
                            actions,
                        );
                    }
                });
        });
}

/// 单条 insert 行：状态点（点击旁通）+ 名称（截断，点击开关插件界面）；
/// 右键菜单：旁通 / 移除。
fn insert_row(
    ui: &mut egui::Ui,
    channel: Option<u8>,
    slot: usize,
    name: &str,
    is_bypassed: bool,
    is_open: bool,
    actions: &mut Vec<MixAction>,
) {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), INSERT_ROW_H),
        egui::Sense::click(),
    );
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(
            rect,
            2.0,
            crate::theme::hover_color(crate::theme::track_bg()),
        );
    }

    // 状态点（点击旁通）：正常 = 强调色；旁通 = 灰。
    let dot_center = egui::pos2(rect.min.x + 5.0, rect.center().y);
    let dot_color = if is_bypassed {
        crate::theme::text_disabled()
    } else if is_open {
        crate::theme::accent_active()
    } else {
        crate::theme::text_secondary()
    };
    painter.circle_filled(dot_center, 3.0, dot_color);
    let dot_hit = egui::Rect::from_center_size(dot_center, egui::vec2(14.0, INSERT_ROW_H));
    let dot_resp = ui.interact(
        dot_hit,
        ui.id().with(("mix_insert_bypass", channel, slot)),
        egui::Sense::click(),
    );
    if dot_resp.clicked() {
        actions.push(MixAction::BypassInsert {
            channel,
            slot,
            bypassed: !is_bypassed,
        });
    }
    dot_resp.on_hover_text(t!("mix.bypass"));

    // 名称：截断显示，hover 全名（右侧给参数按钮留位）。
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(dot_hit.max.x + 2.0, rect.min.y),
        egui::pos2(rect.max.x - 18.0, rect.max.y),
    );
    let name_color = if is_bypassed {
        crate::theme::text_muted()
    } else {
        crate::theme::text_primary()
    };
    ui.put(
        name_rect,
        egui::Label::new(
            egui::RichText::new(name)
                .size(crate::theme::SMALL_FONT)
                .color(name_color),
        )
        .truncate(),
    );

    // 参数按钮（行右侧小图标）：打开通用参数面板。
    let tune = egui_material_icons::icons::ICON_TUNE;
    let btn_rect = egui::Rect::from_min_max(
        egui::pos2(rect.max.x - 17.0, rect.min.y + 2.0),
        egui::pos2(rect.max.x - 1.0, rect.max.y - 2.0),
    );
    let params_resp = ui.interact(
        btn_rect,
        ui.id().with(("mix_insert_params", channel, slot)),
        egui::Sense::click(),
    );
    let icon_color = if params_resp.hovered() {
        crate::theme::accent_active()
    } else {
        crate::theme::text_muted()
    };
    ui.painter().text(
        btn_rect.center(),
        egui::Align2::CENTER_CENTER,
        tune.codepoint,
        egui::FontId::new(11.0, tune.font_family()),
        icon_color,
    );
    if params_resp.on_hover_text(t!("mix.params")).clicked() {
        actions.push(MixAction::OpenInsertParams { channel, slot });
    }

    // 行点击：打开/关闭插件原生界面。
    if resp.clicked() {
        actions.push(MixAction::ToggleGui { channel, slot });
    }
    resp.clone().on_hover_text(t!("mix.toggle_gui"));
    resp.context_menu(|ui| {
        ui.set_min_width(96.0);
        ui.set_max_width(96.0);
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("mix.bypass").as_ref(),
            ))
            .clicked()
        {
            actions.push(MixAction::BypassInsert {
                channel,
                slot,
                bypassed: !is_bypassed,
            });
            ui.close();
        }
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("mix.toggle_gui").as_ref(),
            ))
            .clicked()
        {
            actions.push(MixAction::ToggleGui { channel, slot });
            ui.close();
        }
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("mix.params").as_ref(),
            ))
            .clicked()
        {
            actions.push(MixAction::OpenInsertParams { channel, slot });
            ui.close();
        }
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("mix.remove_insert").as_ref(),
            ))
            .clicked()
        {
            actions.push(MixAction::RemoveInsert { channel, slot });
            ui.close();
        }
    });
}

/// M/S 按钮 + 声像条（紧凑两行，居中）。
fn ms_pan_block(ui: &mut egui::Ui, params: &StripParams, mut on_change: impl FnMut(StripParams)) {
    ui.horizontal(|ui| {
        let total = BTN * 2.0 + 4.0;
        ui.add_space(((CONTENT_W - total) / 2.0).max(0.0));
        if toggle_button(ui, "M", params.mute, crate::theme::mute_active()) {
            let mut p = *params;
            p.mute = !p.mute;
            on_change(p);
        }
        ui.add_space(4.0);
        if toggle_button(ui, "S", params.solo, crate::theme::solo_active()) {
            let mut p = *params;
            p.solo = !p.solo;
            on_change(p);
        }
    });
    if let Some(pan) = pan_bar(ui, params.pan) {
        let mut p = *params;
        p.pan = pan;
        on_change(p);
    }
}

/// 自绘小开关按钮（M/S），激活时填充激活色。
fn toggle_button(
    ui: &mut egui::Ui,
    label: &str,
    active: bool,
    active_color: egui::Color32,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(BTN, BTN), egui::Sense::click());
    let bg = if active {
        active_color
    } else if resp.hovered() {
        crate::theme::hover_color(crate::theme::btn_bg())
    } else {
        crate::theme::btn_bg()
    };
    let fg = if active {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_secondary()
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, bg);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        fg,
    );
    resp.clicked()
}

/// 自绘声像条：中心刻度 + 拖动手柄，双击回中。
/// 返回 `Some(new_pan)` 表示值变化。
fn pan_bar(ui: &mut egui::Ui, pan: f32) -> Option<f32> {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(CONTENT_W, 14.0), egui::Sense::click_and_drag());
    let painter = ui.painter();
    let bar = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), 4.0));
    painter.rect_filled(bar, 2.0, crate::theme::track_bg());
    painter.vline(
        rect.center().x,
        bar.y_range(),
        egui::Stroke::new(1.0, crate::theme::text_muted().gamma_multiply(0.6)),
    );
    let x = rect.center().x + pan.clamp(-1.0, 1.0) * rect.width() / 2.0;
    let handle =
        egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(6.0, 14.0));
    let handle_color = if resp.dragged() {
        crate::theme::pressed_color(crate::theme::btn_bg())
    } else if resp.hovered() {
        crate::theme::hover_color(crate::theme::btn_bg())
    } else {
        crate::theme::text_secondary()
    };
    painter.rect_filled(handle, 2.0, handle_color);

    let pan_text = if pan.abs() < 0.01 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L{:.0}", -pan * 100.0)
    } else {
        format!("R{:.0}", pan * 100.0)
    };
    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            return Some(((pos.x - rect.center().x) / (rect.width() / 2.0)).clamp(-1.0, 1.0));
        }
    } else if resp.double_clicked() {
        return Some(0.0);
    }
    resp.on_hover_text(format!("Pan {pan_text}"));
    None
}

/// dB 读数（推子下方，双击推子归零）。
fn db_label(ui: &mut egui::Ui, gain: f32) {
    let text = if gain <= 0.0001 {
        "-∞".to_string()
    } else {
        format!("{:+.1}", gain_to_db(gain))
    };
    ui.label(
        egui::RichText::new(text)
            .size(crate::theme::SMALL_FONT)
            .color(crate::theme::text_secondary()),
    );
}

/// 推子 + 电平表（同一行，等高）。
fn fader_and_meter(
    ui: &mut egui::Ui,
    gain: f32,
    peak: (f32, f32),
    height: f32,
    mut on_gain: impl FnMut(f32),
) {
    ui.horizontal(|ui| {
        let total = FADER_HANDLE_W + 4.0 + METER_W * 2.0 + METER_GAP;
        ui.add_space(((CONTENT_W - total) / 2.0).max(0.0));
        fader(ui, gain, height, &mut on_gain);
        ui.add_space(4.0);
        meter(ui, peak, height);
    });
}

/// 自绘推子：凹槽 + 填充 + 0dB 刻度 + 手柄（拖动改值，双击回 0dB）。
fn fader(ui: &mut egui::Ui, gain: f32, height: f32, mut on_gain: impl FnMut(f32)) {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(FADER_HANDLE_W, height),
        egui::Sense::click_and_drag(),
    );
    let frac = db_frac(gain_to_db(gain));
    let y = rect.max.y - frac * rect.height();

    let painter = ui.painter();
    // 凹槽。
    let slot = egui::Rect::from_center_size(rect.center(), egui::vec2(FADER_SLOT_W, rect.height()));
    painter.rect_filled(slot, 2.0, crate::theme::track_bg());
    // 已填充部分（底部 → 手柄）。
    let fill = egui::Rect::from_min_max(egui::pos2(slot.min.x, y), slot.max);
    painter.rect_filled(
        fill,
        2.0,
        crate::theme::accent_active().gamma_multiply(0.75),
    );
    // 0dB 参考线（槽两侧短横线）。
    let zero_y = rect.max.y - db_frac(0.0) * rect.height();
    let tick = egui::Stroke::new(1.0, crate::theme::text_muted().gamma_multiply(0.6));
    painter.hline(
        egui::Rangef::new(rect.min.x, slot.min.x - 1.0),
        zero_y,
        tick,
    );
    painter.hline(
        egui::Rangef::new(slot.max.x + 1.0, rect.max.x),
        zero_y,
        tick,
    );
    // 手柄。
    let handle = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, y),
        egui::vec2(FADER_HANDLE_W, FADER_HANDLE_H),
    );
    let handle_bg = if resp.dragged() {
        crate::theme::pressed_color(crate::theme::btn_bg())
    } else if resp.hovered() {
        crate::theme::hover_color(crate::theme::btn_bg())
    } else {
        crate::theme::btn_bg()
    };
    painter.rect_filled(handle, 3.0, handle_bg);
    painter.rect_stroke(
        handle,
        3.0,
        egui::Stroke::new(1.0, crate::theme::text_muted().gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    painter.hline(
        handle.x_range(),
        y,
        egui::Stroke::new(1.0, crate::theme::text_secondary().gamma_multiply(0.8)),
    );

    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = ((rect.max.y - pos.y) / rect.height()).clamp(0.0, 1.0);
            on_gain(db_to_gain(DB_MIN + t * (DB_MAX - DB_MIN)));
        }
    } else if resp.double_clicked() {
        on_gain(1.0);
    }
}

/// 自绘电平表：L/R 双条 + 0dB 刻度。dB 映射 -60..+6；
/// >0dB 金色、≥0dBFS 红色顶格。
fn meter(ui: &mut egui::Ui, peak: (f32, f32), height: f32) {
    let w = METER_W * 2.0 + METER_GAP;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, crate::theme::track_bg());
    for (i, &p) in [peak.0, peak.1].iter().enumerate() {
        let x0 = rect.min.x + i as f32 * (METER_W + METER_GAP);
        let bar = egui::Rect::from_min_size(
            egui::pos2(x0, rect.min.y),
            egui::vec2(METER_W, rect.height()),
        );
        let frac = db_frac(gain_to_db(p));
        if frac <= 0.0 {
            continue;
        }
        let h = bar.height() * frac;
        let fill = egui::Rect::from_min_max(egui::pos2(bar.min.x, bar.max.y - h), bar.max);
        let color = if p >= 1.0 {
            crate::theme::danger_text()
        } else if gain_to_db(p) > -6.0 {
            crate::theme::warning_gold()
        } else {
            crate::theme::accent_active()
        };
        painter.rect_filled(fill, 1.0, color);
    }
    // 0dB 刻度横线。
    let zero_y = rect.max.y - db_frac(0.0) * rect.height();
    painter.hline(
        rect.x_range(),
        zero_y,
        egui::Stroke::new(1.0, crate::theme::text_muted().gamma_multiply(0.5)),
    );
}

/// 插件选择器窗口（列出扫描到的效果器，按名称过滤）。
pub(crate) fn plugin_picker(
    app: &mut App,
    ctx: &egui::Context,
    target: Option<u8>,
    actions: &mut Vec<MixAction>,
) {
    let mut open = true;
    egui::Window::new(t!("mix.picker_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(320.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(t!("mix.search"));
                ui.text_edit_singleline(&mut app.mix.picker_filter);
            });
            ui.separator();
            let filter = app.mix.picker_filter.to_lowercase();
            let plugins = app.mix.scanned.as_ref();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    let mut any = false;
                    if let Some(plugins) = plugins {
                        for p in plugins.iter().filter(|p| p.is_audio_effect()) {
                            if !filter.is_empty() && !p.name.to_lowercase().contains(&filter) {
                                continue;
                            }
                            any = true;
                            if ui
                                .add(crate::widgets::menu::menu_item_button(ui, false, &p.name))
                                .on_hover_text(&p.id)
                                .clicked()
                            {
                                actions.push(MixAction::AddInsert {
                                    channel: target,
                                    plugin: p.clone(),
                                });
                            }
                        }
                    }
                    if !any {
                        ui.label(
                            egui::RichText::new(t!("mix.no_plugins"))
                                .color(crate::theme::text_muted()),
                        );
                    }
                });
        });
    if !open {
        // 用户关了窗口：清空选择器状态（无动作）。
        app.mix.picker_for = None;
    }
}

/// 乐器通道条：标签 + 插件名/选择按钮 + 更换/移除。乐器音频走独立 dense 通道。
pub(crate) fn instrument_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    channel: u16,
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let name = app.workspace.documents[idx]
        .mixer
        .instruments
        .get(channel as usize)
        .and_then(|o| o.as_ref())
        .map(|r| r.name.clone());
    strip_frame(ui, crate::theme::accent_active(), height, |ui| {
        label_block(ui, format!("{} {}", t!("mix.instrument"), channel + 1), "");
        ui.add_space(4.0);
        match &name {
            Some(n) => {
                let resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(n)
                            .size(crate::theme::SMALL_FONT)
                            .color(crate::theme::text_primary()),
                    )
                    .truncate(),
                );
                resp.on_hover_text(n);
                ui.add_space(4.0);
                if ui.small_button(t!("mix.params")).clicked() {
                    actions.push(MixAction::OpenInstrumentParams { channel });
                }
                if ui.small_button(t!("mix.change_instrument")).clicked() {
                    actions.push(MixAction::OpenInstrumentPicker { channel });
                }
                if ui.small_button(t!("mix.remove_insert")).clicked() {
                    actions.push(MixAction::RemoveInstrument { channel });
                }
            }
            None => {
                if ui.button(t!("mix.pick_instrument")).clicked() {
                    actions.push(MixAction::OpenInstrumentPicker { channel });
                }
            }
        }
    });
}

/// 乐器插件选择器：只列 is_instrument() 插件。
pub(crate) fn instrument_picker(
    app: &mut App,
    ctx: &egui::Context,
    channel: u16,
    actions: &mut Vec<MixAction>,
) {
    let mut open = true;
    egui::Window::new(t!("mix.instrument_picker_title"))
        .collapsible(false)
        .resizable(true)
        .default_width(320.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(t!("mix.search"));
                ui.text_edit_singleline(&mut app.mix.picker_filter);
            });
            ui.separator();
            let filter = app.mix.picker_filter.to_lowercase();
            let plugins = app.mix.scanned.as_ref();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    let mut any = false;
                    if let Some(plugins) = plugins {
                        for p in plugins.iter().filter(|p| p.is_instrument()) {
                            if !filter.is_empty() && !p.name.to_lowercase().contains(&filter) {
                                continue;
                            }
                            any = true;
                            if ui
                                .add(crate::widgets::menu::menu_item_button(ui, false, &p.name))
                                .on_hover_text(&p.id)
                                .clicked()
                            {
                                actions.push(MixAction::AssignInstrument {
                                    channel,
                                    plugin: p.clone(),
                                });
                            }
                        }
                    }
                    if !any {
                        ui.label(
                            egui::RichText::new(t!("mix.no_instruments"))
                                .color(crate::theme::text_muted()),
                        );
                    }
                });
        });
    if !open {
        app.mix.instrument_picker_for = None;
    }
}
