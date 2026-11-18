//! 通道条共享控件：外框、电平表、发送区、声像、M/S、dB 读数。

use eframe::egui;
use rust_i18n::t;
use yinhe_mixer::StripParams;
use yinhe_types::ChannelId;

use crate::widgets::knob::{knob, knob_centered};
use crate::widgets::meter::{DB_MAX, DB_MIN, db_to_gain, gain_at_frac, gain_frac, gain_to_db};

use super::*;

/// 发送区高度：C 的 SEND 是聚焦模式——每条通道只显示一个目标旋钮，
/// 不再按目标数展开。
pub(super) fn send_height() -> f32 {
    SEND_ROW_H
}

/// 混音台的理想面板高度：基础布局 + 聚焦发送区一行。
///
/// 这是「自动高度」模式下的高度；用户拖拽分割条后转为手动高度。
pub(crate) fn auto_panel_height() -> f32 {
    HEADER_H + BODY_TOP_PAD + send_height() + GAP + METER_MODULE_H + BODY_BOTTOM_PAD
}

/// 电平表：线性增益 → 表内 y 坐标（top = 最大 dB）。
pub(super) fn gain_y(gain: f32, top: f32, bottom: f32) -> f32 {
    bottom - gain_frac(gain) * (bottom - top)
}

/// 电平表：表内 y 坐标 → 线性增益（超出范围夹到两端）。
pub(super) fn gain_at_y(y: f32, top: f32, bottom: f32) -> f32 {
    let t = ((bottom - y) / (bottom - top)).clamp(0.0, 1.0);
    gain_at_frac(t)
}

/// 通道条外框：直角矩形 + 单层描边 + 顶部色带（写标题）。
/// `add_contents` 在色带下方的内容区排布，参数为其可用高度。
/// 返回整条卡片矩形（调用方可在色带行上叠加交互，如右键菜单）。
pub(super) fn strip_frame(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    color: egui::Color32,
    title: &str,
    height: f32,
    selected: bool,
    add_contents: impl FnOnce(&mut egui::Ui, f32),
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(STRIP_WIDTH, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    // 卡片背景：选中时整条用全局 selection.bg_fill（与 AR 轨道行选中同色），
    // 未选中用默认背景色。
    let bg = if selected {
        ui.visuals().selection.bg_fill
    } else {
        crate::theme::app_bg()
    };
    painter.rect_filled(rect, 0.0, bg);

    // 顶部色带（轨道/通道色）。
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEADER_H));
    painter.rect_filled(header, 0.0, color);
    painter.text(
        egui::pos2(header.min.x + PAD_X, header.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        // 色带文字用「色带颜色的对比色」：与音轨面板色带图标（箭头/加号）
        // 同一套取色（contrast_text），随色带颜色变化。
        crate::theme::contrast_text(color),
    );

    let content = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + PAD_X, header.max.y + BODY_TOP_PAD),
        egui::pos2(rect.max.x - PAD_X, rect.max.y - BODY_BOTTOM_PAD),
    );
    // 用 `new_child`（**不**推进父布局游标）排布内容：通道条宽度由上面的
    // `allocate_exact_size(STRIP_WIDTH)` 决定。若用 `scope_builder`，它会按子
    // min_rect（被 PAD_X 内缩、更窄）回退父游标，导致下一张卡覆盖本卡右侧。
    // 每条通道条必须有独立 id_salt：否则所有条的 M/S、旋钮、dB 等自动 Id
    // 会重复，egui 只让其中一条真正可交互（其余条点击无反应）。
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    // clip 放宽到整卡片：电平表/发送区不受内容区左右内缩裁剪。
    child.set_clip_rect(rect);
    child.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    add_contents(&mut child, content.height());

    // 单层描边确定一个通道。**最后画**：色带等不透明填充会盖住左右描边，
    // 描边放最后保证通道之间的分界线贯穿全高、不被内容打断。
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, crate::theme::line_fg()),
        egui::StrokeKind::Inside,
    );
    rect
}

/// 固定尺寸分区容器（宽度 = 内容宽，高度 = `h`）。
pub(super) fn section<R>(
    ui: &mut egui::Ui,
    h: f32,
    layout: egui::Layout,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(egui::vec2(CONTENT_W, h), layout, add)
        .inner
}

/// 音量自动化目标：LC / AP / Master 统一走 MixerGain。
fn volume_automation_action(target: ChannelId) -> Option<MixAction> {
    Some(MixAction::AddMixerGainAutomation { channel: target })
}

/// 声像自动化目标：LC / AP 走 MixerPan；Master 暂不支持声像。
fn pan_automation_action(target: ChannelId) -> Option<MixAction> {
    (target != ChannelId::Master).then_some(MixAction::AddMixerPanAutomation { channel: target })
}

/// 音量控件的右键菜单：添加自动化 + 重置 0 dB。
fn volume_context_menu(ui: &mut egui::Ui, target: ChannelId, actions: &mut Vec<MixAction>) -> bool {
    let mut reset = false;
    if let Some(action) = volume_automation_action(target)
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
    if ui
        .add(crate::widgets::menu::menu_item_action(
            ui,
            false,
            t!("mix.reset_gain").as_ref(),
            "",
        ))
        .clicked()
    {
        reset = true;
        ui.close();
    }
    reset
}

/// 声像控件的右键菜单：添加自动化 + 居中。
fn pan_context_menu(ui: &mut egui::Ui, target: ChannelId, actions: &mut Vec<MixAction>) -> bool {
    let mut reset = false;
    if let Some(action) = pan_automation_action(target)
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
    if ui
        .add(crate::widgets::menu::menu_item_action(
            ui,
            false,
            t!("mix.reset_pan").as_ref(),
            "",
        ))
        .clicked()
    {
        reset = true;
        ui.close();
    }
    reset
}

/// 发送旋钮的 dB 映射：0 = -inf/OFF，1 = 0 dB。
fn send_norm(amount: f32) -> f32 {
    if amount <= 0.0 {
        0.0
    } else {
        ((gain_to_db(amount).clamp(-60.0, 0.0) + 60.0) / 60.0).clamp(0.0, 1.0)
    }
}

fn send_amount(norm: f32) -> f32 {
    if norm <= 0.0 {
        0.0
    } else {
        db_to_gain(norm * 60.0 - 60.0)
    }
}

/// 发送量文案：`OFF`（-inf，不保留 0 条目）或实际 dB。
fn send_db_label(amount: f32) -> String {
    if amount <= 0.0 {
        "OFF".to_string()
    } else {
        format!("{:+.1} dB", gain_to_db(amount))
    }
}

/// 聚焦发送的当前状态（源 → 目标、发送量、推子前/后）。
pub(crate) struct FocusSend {
    pub source: ChannelId,
    pub target: Option<ChannelId>,
    pub amount: f32,
    pub pre_fader: bool,
}

/// C 的聚焦发送：一个目标、一个 dB 旋钮；源 == 目标或目标不可发送时留空。
pub(super) fn focus_send_block(ui: &mut egui::Ui, send: FocusSend, actions: &mut Vec<MixAction>) {
    let FocusSend {
        source,
        target,
        amount,
        pre_fader,
    } = send;
    let Some(target) = target else {
        return;
    };
    if source == target || source == ChannelId::Master {
        return;
    }
    ui.horizontal(|ui| {
        let mut norm = send_norm(amount);
        let resp = knob(ui, &mut norm, KNOB_D);
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.send_amount"));
        if resp.changed() {
            actions.push(MixAction::SetChannelSend {
                source,
                target,
                amount: send_amount(norm),
                pre_fader,
            });
        }
        send_context_menu(&resp, source, target, amount, pre_fader, actions);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(send_db_label(amount))
                    .size(crate::theme::SMALL_LABEL_FONT)
                    .color(crate::theme::text_secondary()),
            );
        });
    });
}

/// 发送旋钮右键菜单：启用/关闭、推子前/后。
fn send_context_menu(
    resp: &egui::Response,
    source: ChannelId,
    target: ChannelId,
    amount: f32,
    pre_fader: bool,
    actions: &mut Vec<MixAction>,
) {
    crate::widgets::menu::context_menu(resp, 180.0, |ui| {
        let enable_label = if amount > 0.0 {
            t!("mix.send_disable")
        } else {
            t!("mix.send_enable")
        };
        crate::widgets::menu::action(ui, enable_label.to_string(), "", |_| {
            actions.push(MixAction::SetChannelSend {
                source,
                target,
                amount: if amount > 0.0 { 0.0 } else { 1.0 },
                pre_fader,
            });
        });
        let (pre_label, next_pre) = if pre_fader {
            (t!("mix.send_post_fader"), false)
        } else {
            (t!("mix.pre_fader"), true)
        };
        crate::widgets::menu::action(ui, pre_label.to_string(), "", |_| {
            actions.push(MixAction::SetChannelSend {
                source,
                target,
                amount: amount.max(f32::MIN_POSITIVE),
                pre_fader: next_pre,
            });
        });
    });
}

fn ms_column(ui: &mut egui::Ui, params: &StripParams, width: f32) -> Option<StripParams> {
    ui.allocate_ui_with_layout(
        egui::vec2(width, BTN * 2.0 + GAP),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let mut p = *params;
            let audibility = p.audibility();
            // 单按钮：始终显示 M；左键静音、右键独奏，亮色区分。
            let (label, active, color) = crate::widgets::audibility::visual(audibility);
            let resp = toggle_button(ui, label, active, color);
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.mute"));
            if resp.clicked() {
                p.set_audibility(crate::widgets::audibility::next_on_primary(audibility));
                return Some(p);
            }
            if resp.secondary_clicked() {
                p.set_audibility(crate::widgets::audibility::next_on_secondary(audibility));
                return Some(p);
            }
            None
        },
    )
    .inner
}

/// 声像旋钮/// 声像旋钮（右列）：旋钮居中，左右分别标 L / R。
/// 返回 `Some(新 pan)` 表示被改动。
fn pan_knob(
    ui: &mut egui::Ui,
    pan: f32,
    width: f32,
    target: ChannelId,
    actions: &mut Vec<MixAction>,
) -> Option<f32> {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, KNOB_D), egui::Sense::hover());
    let painter = ui.painter();
    let font = egui::FontId::proportional((crate::theme::SMALL_FONT - 1.0).max(8.0));
    let color = crate::theme::text_muted();
    painter.text(
        egui::pos2(rect.min.x + 1.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "L",
        font.clone(),
        color,
    );
    painter.text(
        egui::pos2(rect.max.x - 1.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "R",
        font,
        color,
    );
    let knob_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(KNOB_D, KNOB_D));
    let mut norm = (pan + 1.0) / 2.0;
    let resp = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(knob_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| knob_centered(ui, &mut norm, KNOB_D),
        )
        .inner;
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.pan"));
    let mut reset = false;
    crate::widgets::menu::context_menu(&resp, 180.0, |ui| {
        reset = pan_context_menu(ui, target, actions);
    });
    if reset {
        return Some(0.0);
    }
    if resp.changed() {
        Some(norm * 2.0 - 1.0)
    } else {
        None
    }
}

/// dB 读数（右列顶部）：可点击**直接输入**、也可上下拖动。
/// 返回 `Some(新增益)` 表示本次被改动。
fn db_row(
    ui: &mut egui::Ui,
    gain: f32,
    width: f32,
    target: ChannelId,
    actions: &mut Vec<MixAction>,
) -> Option<f32> {
    let mut db = gain_to_db(gain);
    let resp = ui.add_sized(
        [width, DB_ROW_H],
        egui::DragValue::new(&mut db)
            .speed(0.2)
            .range(DB_MIN..=DB_MAX)
            .max_decimals(1)
            // 推子底部是静音，不显示 -60.0；直接显示 -∞。
            .custom_formatter(|db, _| {
                if db <= f64::from(DB_MIN) + 1e-6 {
                    "-∞".to_string()
                } else {
                    format!("{db:.1}")
                }
            })
            .custom_parser(|s| {
                let t = s.trim();
                if matches!(t, "-∞" | "-inf" | "-infinity") {
                    Some(f64::from(DB_MIN))
                } else {
                    t.parse().ok()
                }
            }),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.fader"));
    let mut reset = false;
    crate::widgets::menu::context_menu(&resp, 180.0, |ui| {
        reset = volume_context_menu(ui, target, actions);
    });
    if reset {
        return Some(1.0);
    }
    if resp.changed() {
        Some(db_to_gain(db))
    } else {
        None
    }
}

/// 自绘小开关按钮（M/S），激活时填充激活色；圆角矩形。
fn toggle_button(
    ui: &mut egui::Ui,
    label: &str,
    active: bool,
    active_color: egui::Color32,
) -> egui::Response {
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
    painter.rect_filled(rect, RADIUS, bg);
    painter.rect_stroke(
        rect,
        RADIUS,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        fg,
    );
    resp
}

/// 电平表 + 增益推子：推子贴在 L/R 条**左侧外侧**，是向右（指向音频）
/// 的实心三角，拖动改增益，双击回 0dB。返回 `Some(新增益)`。
fn gain_meter(
    ui: &mut egui::Ui,
    gain: f32,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    target: ChannelId,
    actions: &mut Vec<MixAction>,
) -> Option<f32> {
    let meter_w = SCALE_W + METER_W * 2.0 + METER_GAP;
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(meter_w, height), egui::Sense::click_and_drag());
    crate::widgets::meter::paint_level_meter(ui.painter(), rect, level, hold);

    // 推子：底边贴 L/R 条左缘外侧，尖端指向电平表（向右）。上下夹住，
    // 满增益/静音时三角不越出模块。
    let half = 5.0;
    let y = gain_y(gain, rect.min.y, rect.max.y).clamp(rect.min.y + half, rect.max.y - half);
    let tip_x = rect.min.x + SCALE_W - 1.0;
    let base_x = tip_x - 7.0;
    let color = if resp.hovered() || resp.dragged() {
        crate::theme::accent_active()
    } else {
        crate::theme::text_primary()
    };
    let points = vec![
        egui::pos2(base_x, y - half),
        egui::pos2(base_x, y + half),
        egui::pos2(tip_x, y),
    ];
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));

    let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.fader"));
    let mut reset = false;
    crate::widgets::menu::context_menu(&resp, 180.0, |ui| {
        reset = volume_context_menu(ui, target, actions);
    });
    if reset {
        return Some(1.0);
    }
    if resp.dragged()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        return Some(gain_at_y(pos.y, rect.min.y, rect.max.y));
    }
    if resp.double_clicked() {
        return Some(1.0);
    }
    None
}

/// 通道/音频/每轨 LC 共用：固定高度的电平表 + 右侧 dB / 声像 / M / S。
///
/// 模块高度取 `METER_MODULE_H` 与剩余高度的较小值；面板更高时用空白把模块
/// 顶到卡片底部，不再随面板高度拉伸。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
pub(super) fn strip_controls(
    ui: &mut egui::Ui,
    params: &StripParams,
    level: (f32, f32),
    hold: (f32, f32),
    show_pan: bool,
    show_ms: bool,
    target: ChannelId,
    actions: &mut Vec<MixAction>,
) -> Option<StripParams> {
    let avail = ui.available_height();
    let module_h = METER_MODULE_H.min(avail).max(1.0);
    let mut out = None;
    // 上方剩余空白 + 底部固定模块：与 master_controls 共用 content_footer。
    crate::widgets::grid::content_footer(ui, 0.0, module_h, |cells| {
        cells.cell(|_| {});
        cells.cell(|ui| {
            out = section(
                ui,
                module_h,
                egui::Layout::left_to_right(egui::Align::TOP),
                |ui| {
                    let mut p = *params;
                    let mut changed = false;
                    if let Some(g) = gain_meter(ui, p.gain, level, hold, module_h, target, actions)
                    {
                        p.gain = g;
                        changed = true;
                    }
                    ui.add_space(GAP);
                    ui.allocate_ui_with_layout(
                        egui::vec2(CONTROLS_W, module_h),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if let Some(g) = db_row(ui, p.gain, CONTROLS_W, target, actions) {
                                p.gain = g;
                                changed = true;
                            }
                            if show_pan {
                                ui.add_space(GAP);
                                if let Some(pan) = pan_knob(ui, p.pan, CONTROLS_W, target, actions)
                                {
                                    p.pan = pan;
                                    changed = true;
                                }
                            }
                            if show_ms {
                                ui.add_space(GAP);
                                if let Some(next) = ms_column(ui, &p, CONTROLS_W) {
                                    p = next;
                                    changed = true;
                                }
                            }
                        },
                    );
                    changed.then_some(p)
                },
            );
        });
    });
    out
}

/// 主输出：只有电平表 + dB 读数，固定高度模块。
pub(super) fn master_controls(
    ui: &mut egui::Ui,
    gain: f32,
    level: (f32, f32),
    hold: (f32, f32),
    actions: &mut Vec<MixAction>,
) -> Option<f32> {
    let avail = ui.available_height();
    let module_h = METER_MODULE_H.min(avail).max(1.0);
    let mut out = None;
    crate::widgets::grid::content_footer(ui, 0.0, module_h, |cells| {
        cells.cell(|_| {});
        cells.cell(|ui| {
            out = section(
                ui,
                module_h,
                egui::Layout::left_to_right(egui::Align::TOP),
                |ui| {
                    let mut g = gain;
                    let mut changed = false;
                    if let Some(v) =
                        gain_meter(ui, g, level, hold, module_h, ChannelId::Master, actions)
                    {
                        g = v;
                        changed = true;
                    }
                    ui.add_space(GAP);
                    ui.allocate_ui_with_layout(
                        egui::vec2(CONTROLS_W, module_h),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if let Some(v) = db_row(ui, g, CONTROLS_W, ChannelId::Master, actions) {
                                g = v;
                                changed = true;
                            }
                        },
                    );
                    changed.then_some(g)
                },
            );
        });
    });
    out
}
