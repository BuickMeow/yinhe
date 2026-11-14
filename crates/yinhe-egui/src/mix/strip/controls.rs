//! 通道条共享控件：外框、电平表、发送区、声像、M/S、dB 读数。

use eframe::egui;
use rust_i18n::t;
use yinhe_mixer::{SendParams, StripParams};

use crate::widgets::knob::{knob, knob_centered};
use crate::widgets::meter::{DB_MAX, DB_MIN, db_frac, db_to_gain, gain_to_db};

use super::*;

/// 发送区高度：每条总线一行，全部可见（总线多了面板随之上移）。
pub(super) fn send_height(bus_count: usize) -> f32 {
    SEND_ROW_H * bus_count.max(1) as f32
}

/// 混音台的理想面板高度：基础布局 + 发送区按总线数扩展。
///
/// 这是「自动高度」模式下的高度；用户拖拽分割条后转为手动高度。
pub(crate) fn auto_panel_height(bus_count: usize) -> f32 {
    HEADER_H + BODY_TOP_PAD + send_height(bus_count) + GAP + METER_MODULE_H + BODY_BOTTOM_PAD
}

/// 电平表：线性增益 → 表内 y 坐标（top = 最大 dB）。
pub(super) fn gain_y(gain: f32, top: f32, bottom: f32) -> f32 {
    bottom - db_frac(gain_to_db(gain)) * (bottom - top)
}

/// 电平表：表内 y 坐标 → 线性增益（超出范围夹到两端）。
pub(super) fn gain_at_y(y: f32, top: f32, bottom: f32) -> f32 {
    let t = ((bottom - y) / (bottom - top)).clamp(0.0, 1.0);
    db_to_gain(DB_MIN + t * (DB_MAX - DB_MIN))
}

/// 通道条外框：直角矩形 + 单层描边 + 顶部色带（写标题）。
/// `add_contents` 在色带下方的内容区排布，参数为其可用高度。
/// 返回整条卡片矩形（调用方可在色带行上叠加交互，如总线删除按钮）。
pub(super) fn strip_frame(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    color: egui::Color32,
    title: &str,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui, f32),
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(STRIP_WIDTH, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    // 卡片背景用默认背景色（不再用偏灰的 control_bg）。
    painter.rect_filled(rect, 0.0, crate::theme::app_bg());

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

/// 单条 MIDI 通道条。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
/// 发送区来源：通道 / 音频通道 / 总线。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SendSource {
    Channel(u8),
    Audio(u16),
    Bus(u8),
}

/// 效果发送区（独占一区，竖向滚动）：**每条总线一行**（旋钮 + 「BUS n」），
/// 超出显示区（一行高）的部分竖向滚动查看；无总线时保留一个 BUS 1 占位
/// （首次拖动自动创建第一条总线）。所有通道/总线的发送区共用同一个滚动位置
/// （`ScrollArea` 用同一 id_salt）——滚动任一通道即整排同步滚动。
///
/// 总线源发送到**自己**时显示 ICON_REFRESH 开关（点击启用/禁用自发送）：
/// 自发送经块级延迟防止反馈环路，提示见 `hint.mix.bus_self_send`。
/// 通道源右键任一旋钮打开完整发送面板（含推子前/后、多总线）。
pub(super) fn send_block(
    ui: &mut egui::Ui,
    source: SendSource,
    sends: &[SendParams],
    bus_count: usize,
    actions: &mut Vec<MixAction>,
) {
    let knobs = bus_count.max(1);
    egui::ScrollArea::vertical()
        // 发送区各自滚动（id 在通道条自己的作用域内唯一）。滚动条 id 派生自
        // id_salt，若整排共用同一 id 会触发 egui 的 widget ID 冲突；发送区高度
        // 已按总线数展开（全部行可见），仅窗口过矮时需滚动，故隐藏滚动条。
        .id_salt("mix_send_scroll")
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysHidden)
        // egui 默认 min_scrolled_size=64 会把发送区撑高，这里放开下限，
        // 让发送区严格等于「总线数 × 一行高」。
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            for b in 0..knobs {
                let target = b as u8;
                ui.horizontal(|ui| {
                    let send = sends.iter().find(|s| s.bus == target);
                    let (amount, pre_fader) = send
                        .map(|s| (s.amount, s.pre_fader))
                        .unwrap_or((0.0, false));
                    // 总线自发送（总线 → 自己）：旋钮照常可调量，行尾图标开关
                    // 占用「BUS n」标签位置（点击启用/禁用；启用后经块级延迟防反馈环）。
                    if source == SendSource::Bus(target) {
                        let mut norm = (amount / 2.0).clamp(0.0, 1.0);
                        let knob_resp = knob(ui, &mut norm, KNOB_D);
                        if knob_resp.changed() {
                            actions.push(MixAction::SetBusSend {
                                bus: target,
                                target_bus: target,
                                amount: norm * 2.0,
                            });
                        }
                        crate::widgets::hint::hover(
                            ui.ctx(),
                            &knob_resp,
                            t!("hint.mix.send_amount"),
                        );
                        ui.add_space(4.0);
                        let icon_resp = self_send_toggle(ui, amount > 0.0);
                        if icon_resp.clicked() {
                            actions.push(MixAction::SetBusSend {
                                bus: target,
                                target_bus: target,
                                amount: if amount > 0.0 { 0.0 } else { 1.0 },
                            });
                        }
                        return;
                    }
                    let mut norm = (amount / 2.0).clamp(0.0, 1.0);
                    let resp = knob(ui, &mut norm, KNOB_D);
                    if resp.changed() {
                        match source {
                            SendSource::Channel(channel) => {
                                if bus_count == 0 {
                                    actions.push(MixAction::AddBus);
                                }
                                actions.push(MixAction::SetSend {
                                    channel,
                                    bus: target,
                                    amount: norm * 2.0,
                                    pre_fader,
                                });
                            }
                            SendSource::Audio(channel) => {
                                actions.push(MixAction::SetAudioSend {
                                    channel,
                                    bus: target,
                                    amount: norm * 2.0,
                                    pre_fader,
                                });
                            }
                            SendSource::Bus(_) => unreachable!("总线源在上方提前返回"),
                        }
                    }
                    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.send_amount"));
                    // 完整发送面板目前只支持通道源。
                    if let SendSource::Channel(channel) = source {
                        resp.context_menu(|ui| {
                            let item = ui.add(crate::widgets::menu::menu_item_button(
                                ui,
                                false,
                                t!("mix.sends").as_ref(),
                            ));
                            crate::widgets::hint::hover(ui.ctx(), &item, t!("hint.mix.sends"));
                            if item.clicked() {
                                actions.push(MixAction::OpenSends { channel });
                                ui.close();
                            }
                        });
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("BUS {}", b + 1))
                            .size(crate::theme::SMALL_LABEL_FONT)
                            .color(crate::theme::text_secondary()),
                    );
                });
            }
        });
}

/// 自发送开关图标（ICON_REFRESH，占用行尾「BUS n」标签位置）：
/// 启用 = 强调色，未启用 = 弱化色；悬停提示「启用后会引入一个缓冲区延迟」。
fn self_send_toggle(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, KNOB_D), egui::Sense::click());
    let color = if enabled {
        crate::theme::accent_active()
    } else if resp.hovered() {
        crate::theme::text_primary()
    } else {
        crate::theme::text_muted()
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_material_icons::icons::ICON_REFRESH.codepoint,
        egui::FontId::new(
            crate::theme::ICON_FONT_SM,
            egui_material_icons::icons::ICON_REFRESH.font_family(),
        ),
        color,
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.bus_self_send"));
    resp
}

/// M/S 竖向按钮列（右列底部）：M 上、S 下。
fn ms_column(ui: &mut egui::Ui, params: &StripParams, width: f32) -> Option<StripParams> {
    ui.allocate_ui_with_layout(
        egui::vec2(width, BTN * 2.0 + GAP),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let mut p = *params;
            let mut changed = false;
            let m_resp = toggle_button(ui, "M", p.mute, crate::theme::mute_active());
            crate::widgets::hint::hover(ui.ctx(), &m_resp, t!("hint.mix.mute"));
            if m_resp.clicked() {
                p.mute = !p.mute;
                changed = true;
            }
            ui.add_space(GAP);
            let s_resp = toggle_button(ui, "S", p.solo, crate::theme::solo_active());
            crate::widgets::hint::hover(ui.ctx(), &s_resp, t!("hint.mix.solo"));
            if s_resp.clicked() {
                p.solo = !p.solo;
                changed = true;
            }
            changed.then_some(p)
        },
    )
    .inner
}

/// 声像旋钮（右列）：旋钮居中，左右分别标 L / R。
/// 返回 `Some(新 pan)` 表示被改动。
fn pan_knob(ui: &mut egui::Ui, pan: f32, width: f32) -> Option<f32> {
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
    if resp.changed() {
        Some(norm * 2.0 - 1.0)
    } else {
        None
    }
}

/// dB 读数（右列顶部）：可点击**直接输入**、也可上下拖动。
/// 返回 `Some(新增益)` 表示本次被改动。
fn db_row(ui: &mut egui::Ui, gain: f32, width: f32) -> Option<f32> {
    let mut db = gain_to_db(gain);
    let resp = ui.add_sized(
        [width, DB_ROW_H],
        egui::DragValue::new(&mut db)
            .speed(0.2)
            .range(DB_MIN..=DB_MAX)
            .max_decimals(1),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.fader"));
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

/// 通道/音频/总线共用：固定高度的电平表 + 右侧 dB / 声像 / M / S。
///
/// 模块高度取 `METER_MODULE_H` 与剩余高度的较小值；面板更高时用空白把模块
/// 顶到卡片底部，不再随面板高度拉伸。
pub(super) fn strip_controls(
    ui: &mut egui::Ui,
    params: &StripParams,
    level: (f32, f32),
    hold: (f32, f32),
    show_pan: bool,
    show_ms: bool,
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
                    if let Some(g) = gain_meter(ui, p.gain, level, hold, module_h) {
                        p.gain = g;
                        changed = true;
                    }
                    ui.add_space(GAP);
                    ui.allocate_ui_with_layout(
                        egui::vec2(CONTROLS_W, module_h),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if let Some(g) = db_row(ui, p.gain, CONTROLS_W) {
                                p.gain = g;
                                changed = true;
                            }
                            if show_pan {
                                ui.add_space(GAP);
                                if let Some(pan) = pan_knob(ui, p.pan, CONTROLS_W) {
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
                    if let Some(v) = gain_meter(ui, g, level, hold, module_h) {
                        g = v;
                        changed = true;
                    }
                    ui.add_space(GAP);
                    ui.allocate_ui_with_layout(
                        egui::vec2(CONTROLS_W, module_h),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            if let Some(v) = db_row(ui, g, CONTROLS_W) {
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
