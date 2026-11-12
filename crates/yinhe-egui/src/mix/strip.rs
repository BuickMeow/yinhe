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
use yinhe_audio::InsertTarget;
use yinhe_mixer::{MasterParams, SendParams, StripParams};

use crate::app::App;
use crate::widgets::knob::{knob, knob_centered};

use super::plugin_instance::PluginEntry;
use super::{MixAction, channel_label};
use crate::widgets::meter::{
    DB_MAX, DB_MIN, METER_GAP, METER_W, SCALE_W, db_frac, db_to_gain, gain_to_db,
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

/// 发送区高度：每条总线一行，全部可见（总线多了面板随之上移）。
fn send_height(bus_count: usize) -> f32 {
    SEND_ROW_H * bus_count.max(1) as f32
}

/// 混音台的理想面板高度：基础布局 + 发送区按总线数扩展。
///
/// 这是「自动高度」模式下的高度；用户拖拽分割条后转为手动高度。
pub(crate) fn auto_panel_height(bus_count: usize) -> f32 {
    HEADER_H + BODY_TOP_PAD + send_height(bus_count) + GAP + METER_MODULE_H + BODY_BOTTOM_PAD
}

/// 电平表：线性增益 → 表内 y 坐标（top = 最大 dB）。
fn gain_y(gain: f32, top: f32, bottom: f32) -> f32 {
    bottom - db_frac(gain_to_db(gain)) * (bottom - top)
}

/// 电平表：表内 y 坐标 → 线性增益（超出范围夹到两端）。
fn gain_at_y(y: f32, top: f32, bottom: f32) -> f32 {
    let t = ((bottom - y) / (bottom - top)).clamp(0.0, 1.0);
    db_to_gain(DB_MIN + t * (DB_MAX - DB_MIN))
}

/// 通道条外框：直角矩形 + 单层描边 + 顶部色带（写标题）。
/// `add_contents` 在色带下方的内容区排布，参数为其可用高度。
/// 返回整条卡片矩形（调用方可在色带行上叠加交互，如总线删除按钮）。
fn strip_frame(
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
fn section<R>(
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

/// 发送区来源：通道 / 音频通道 / 总线。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SendSource {
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
fn send_block(
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
fn strip_controls(
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
fn master_controls(
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

/// 发送面板（某源通道对各总线的发送量 / 推子前推子后）。
pub(crate) fn send_popup(
    app: &mut App,
    ctx: &egui::Context,
    channel: u8,
    actions: &mut Vec<MixAction>,
) {
    let Some(idx) = app.workspace.active_doc else {
        return;
    };
    let bus_count = app.workspace.documents[idx].mixer.buses.len();
    let current: Vec<(f32, bool)> = (0..bus_count)
        .map(|b| {
            app.workspace.documents[idx]
                .mixer
                .sends
                .get(channel as usize)
                .and_then(|list| list.iter().find(|s| s.bus as usize == b))
                .map(|s| (s.amount, s.pre_fader))
                .unwrap_or((0.0, false))
        })
        .collect();

    // 独立 OS 窗口（viewport）：发送面板。
    let id = egui::ViewportId::from_hash_of("mix_send_popup");
    crate::chrome::dialog::raise_on_open(ctx, id);
    let title = format!("{} {}", t!("mix.sends"), crate::mix::channel_label(channel));
    let mut close = false;
    ctx.show_viewport_immediate(
        id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [300.0, 260.0], false),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            let mut closed = close;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut closed, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            if bus_count == 0 {
                                ui.label(
                                    egui::RichText::new(t!("mix.no_buses"))
                                        .color(crate::theme::text_muted()),
                                );
                                return;
                            }
                            for (b, (amount, pre)) in current.iter().enumerate() {
                                crate::widgets::rows::inline_row(ui, |ui| {
                                    ui.label(format!("BUS {}", b + 1));
                                    let mut val = *amount;
                                    let slider = crate::widgets::slider::control_slider(
                                        ui,
                                        &mut val,
                                        0.0..=2.0,
                                        ui.available_width().min(160.0),
                                        None,
                                        false,
                                    );
                                    crate::widgets::hint::hover(
                                        ui.ctx(),
                                        &slider,
                                        t!("hint.mix.send_amount"),
                                    );
                                    if slider.changed() {
                                        actions.push(MixAction::SetSend {
                                            channel,
                                            bus: b as u8,
                                            amount: val,
                                            pre_fader: *pre,
                                        });
                                    }
                                    let db = if val <= 0.0 {
                                        "-∞".to_string()
                                    } else {
                                        format!("{:+.1} dB", gain_to_db(val))
                                    };
                                    ui.label(
                                        egui::RichText::new(db)
                                            .size(crate::theme::SMALL_FONT)
                                            .color(crate::theme::text_secondary()),
                                    );
                                    let mut pre_flag = *pre;
                                    let pre_resp = ui.checkbox(&mut pre_flag, t!("mix.pre_fader"));
                                    crate::widgets::hint::hover(
                                        ui.ctx(),
                                        &pre_resp,
                                        t!("hint.mix.pre_fader"),
                                    );
                                    if pre_resp.changed() {
                                        actions.push(MixAction::SetSend {
                                            channel,
                                            bus: b as u8,
                                            amount: val,
                                            pre_fader: pre_flag,
                                        });
                                    }
                                });
                            }
                        });
                });
            if closed {
                close = true;
            }
        },
    );
    if close {
        crate::chrome::dialog::mark_viewport_closed(ctx, id);
        app.mix.sends_for = None;
    }
}

/// 选择器左侧分类页签。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PickerTab {
    /// 全部（所有插件 + 内置 XSynth）。
    #[default]
    All,
    /// 乐器（内置 XSynth + 插件乐器）。
    Instrument,
    /// 效果器（插件效果器）。
    Effect,
    /// 扩展（全部已安装的 VST / CLAP 插件，不含内置）。
    Extension,
}

/// 选择器中内置 XSynth 行的选中 id（非插件条目）。
pub(crate) const XS_SELECTED_ID: &str = "__xsynth__";

impl PickerTab {
    const ALL: [Self; 4] = [Self::All, Self::Instrument, Self::Effect, Self::Extension];

    fn icon(self) -> egui_material_icons::MaterialIcon {
        use egui_material_icons::icons::*;
        match self {
            Self::All => ICON_APPS,
            Self::Instrument => ICON_PIANO,
            Self::Effect => ICON_GRADIENT,
            Self::Extension => ICON_EXTENSION,
        }
    }

    fn label(self) -> String {
        match self {
            Self::All => t!("mix.picker_tab_all").to_string(),
            Self::Instrument => t!("mix.picker_tab_instrument").to_string(),
            Self::Effect => t!("mix.picker_tab_effect").to_string(),
            Self::Extension => t!("mix.picker_tab_extension").to_string(),
        }
    }

    /// 该条目是否属于本分类。
    fn matches(self, plugin: &PluginEntry) -> bool {
        match self {
            Self::All => true,
            Self::Instrument => plugin.is_instrument,
            Self::Effect => plugin.is_effect,
            Self::Extension => true,
        }
    }

    /// 是否显示内置 XSynth 项（全部 / 乐器分类）。
    fn shows_xsynth(self) -> bool {
        matches!(self, Self::All | Self::Instrument)
    }
}

/// 选择器用途：效果器（insert 链）或乐器（MIDI 通道挂载）。
#[derive(Clone, Copy)]
enum PickerKind {
    /// 效果器：选中插件加到 `target` 的 insert 链。
    Effect { target: InsertTarget },
    /// 乐器：选中插件挂到 MIDI 通道；内置 XSynth 项 = 清除插件挂载。
    Instrument { channel: u8 },
}

impl PickerKind {
    fn viewport_id(self) -> egui::ViewportId {
        // 统一窗口：乐器/效果器共用（两入口互斥，同一时刻只开一个）。
        egui::ViewportId::from_hash_of("mix_plugin_picker")
    }

    /// 语境是否允许加载该插件（效果器语境不接受乐器插件）。
    fn can_pick(self, plugin: &PluginEntry) -> bool {
        match self {
            Self::Effect { .. } => plugin.is_effect,
            Self::Instrument { .. } => true,
        }
    }

    /// 选中一个插件条目：按语境与插件类型分派——
    /// 乐器语境选乐器 = 挂到通道；乐器语境选效果器 = 加到该通道 insert 链；
    /// 效果器语境只接受效果器。
    fn pick_plugin(self, plugin: &PluginEntry, actions: &mut Vec<MixAction>) {
        match self {
            Self::Effect { target } if plugin.is_effect => actions.push(MixAction::AddInsert {
                target,
                plugin: plugin.clone(),
            }),
            Self::Instrument { channel } if plugin.is_instrument => {
                actions.push(MixAction::AssignInstrument {
                    channel,
                    plugin: plugin.clone(),
                })
            }
            Self::Instrument { channel } => actions.push(MixAction::AddInsert {
                target: InsertTarget::Channel(channel),
                plugin: plugin.clone(),
            }),
            _ => {}
        }
    }

    /// 点击内置 XSynth（仅乐器）：清空该通道的插件乐器链（回到默认 XSynth）。
    fn pick_builtin(self, actions: &mut Vec<MixAction>) {
        if let Self::Instrument { channel } = self {
            actions.push(MixAction::ClearInstruments { channel });
        }
    }

    /// 窗口关闭后清理打开标记（两个入口共用同一窗口）。
    fn clear_open(self, app: &mut App) {
        app.mix.picker_for = None;
        app.mix.instrument_picker_for = None;
    }
}

/// 加载当前选中项（双击 / 「好」共用）：插件按语境分派，XSynth 清空乐器链。
fn pick_selected(app: &App, kind: PickerKind, actions: &mut Vec<MixAction>) {
    let Some(sel) = app.mix.picker_selected.clone() else {
        return;
    };
    if sel == XS_SELECTED_ID {
        kind.pick_builtin(actions);
        return;
    }
    if let Some(p) = app
        .mix
        .scanned
        .as_ref()
        .and_then(|list| list.iter().find(|p| p.id == sel))
        && p.error.is_none()
        && kind.can_pick(p)
    {
        kind.pick_plugin(p, actions);
    }
}

/// 左侧分类栏（PR 工具栏同款 44px 竖条）：乐器 / 效果器 / 扩展 / 收藏。
fn picker_tab_bar(app: &mut App, ui: &mut egui::Ui) {
    let bar_w = crate::theme::TOOL_BAR_W;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(bar_w, ui.available_height()),
        egui::Sense::hover(),
    );
    // 贴窗口左缘、无底色（与窗口背景一体）。
    let btn = crate::theme::TRANSPORT_BTN_SIZE;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            for tab in PickerTab::ALL {
                let selected = app.mix.picker_tab == tab;
                let color = if selected {
                    crate::theme::accent_active()
                } else {
                    crate::theme::text_primary()
                };
                let sel_bg = selected.then(crate::theme::selected_bg);
                let resp = ui
                    .push_id(("picker_tab", tab as usize), |ui| {
                        crate::widgets::flat::flat_button_filled_ghost(
                            ui,
                            tab.icon()
                                .rich_text()
                                .size(crate::theme::TRANSPORT_BTN_FONT)
                                .color(color),
                            egui::vec2(btn, btn),
                            sel_bg,
                            true,
                        )
                    })
                    .inner;
                if resp.clicked() {
                    app.mix.picker_tab = tab;
                    app.mix.picker_selected = None;
                }
                // 独立窗口内用鼠标旁 tooltip（模式栏讲解不覆盖子窗口）。
                resp.on_hover_text(tab.label());
            }
        });
    });
}

/// 厂家分组标题行（列表按厂家分组时的组头）。
fn vendor_header(ui: &mut egui::Ui, vendor: &str) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.min.x + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        vendor,
        egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
        crate::theme::text_primary(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.max.y - 0.5,
        egui::Stroke::new(1.0, crate::theme::line_fg()),
    );
}

/// 中间列表：自绘搜索框 + 星标筛选 + 按厂家分组 + 单击选中 / 双击加载。
fn picker_list(
    app: &mut App,
    ui: &mut egui::Ui,
    kind: PickerKind,
    actions: &mut Vec<MixAction>,
    close: &mut bool,
) {
    // 列宽由外部 three_columns 分配，这里直接使用当前 cell。
    // 搜索行：自绘输入框 + 星标筛选开关（点亮 = 只看收藏）。
    let star_w = 26.0;
    let mut filter = app.mix.picker_filter.clone();
    let fav_only = app.mix.picker_fav_only;
    let mut next_fav = fav_only;
    let search_h = crate::widgets::control::h(ui.ctx());
    crate::widgets::grid::fixed_columns(
        ui,
        6.0,
        search_h,
        &[
            egui_extras::Size::remainder(),
            egui_extras::Size::exact(star_w),
        ],
        |cells| {
            cells.cell(|ui| {
                let search_resp = crate::widgets::text_input::control_text_input(
                    ui,
                    &mut filter,
                    ui.available_width().max(1.0),
                    "mix_picker_search",
                    Some(&t!("mix.search")),
                );
                search_resp.on_hover_text(t!("hint.mix.picker_search"));
            });
            cells.cell(|ui| {
                let (icon, color) = if fav_only {
                    (
                        egui_material_icons::icons::ICON_STAR,
                        crate::theme::warning_gold(),
                    )
                } else {
                    (
                        egui_material_icons::icons::ICON_STAR.outlined(),
                        crate::theme::text_muted(),
                    )
                };
                let star_resp = crate::widgets::flat::flat_button_filled_ghost(
                    ui,
                    icon.rich_text().size(crate::theme::ICON_FONT).color(color),
                    egui::vec2(star_w, crate::widgets::control::h(ui.ctx())),
                    None,
                    true,
                )
                .on_hover_text(t!("hint.mix.picker_fav_filter"));
                if star_resp.clicked() {
                    next_fav = !fav_only;
                }
            });
        },
    );
    app.mix.picker_filter = filter;
    app.mix.picker_fav_only = next_fav;
    ui.separator();
    let filter = app.mix.picker_filter.to_lowercase();
    let favorites = app.audio_settings.favorite_plugins.clone();
    let fav_only = app.mix.picker_fav_only;
    let tab = app.mix.picker_tab;
    let mut clicked: Option<String> = None;
    let mut activated: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("mix_plugin_picker_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut any = false;
            // 内置 XSynth：全部/乐器分类（选择 = 清除插件挂载）；
            // 星标筛选（只看收藏）时隐藏（XSynth 不是插件、不可收藏）。
            if tab.shows_xsynth() && !fav_only && (filter.is_empty() || "xsynth".contains(&filter))
            {
                any = true;
                let selected = app.mix.picker_selected.as_deref() == Some(XS_SELECTED_ID);
                let resp = plugin_row(
                    ui,
                    egui_material_icons::icons::ICON_PIANO,
                    "XSynth",
                    yinhe_mixer::PluginFormat::Builtin,
                    true,
                    selected,
                )
                .on_hover_text(t!("hint.mix.picker_xsynth"));
                if resp.clicked() {
                    clicked = Some(XS_SELECTED_ID.to_string());
                }
                if resp.double_clicked() {
                    activated = Some(XS_SELECTED_ID.to_string());
                }
            }
            if let Some(plugins) = app.mix.scanned.as_ref() {
                // 过滤（分类 + 星标筛选 + 搜索）后**按厂家分组**，
                // 组内按名称排序；组头仅在厂家非空时绘制。
                let mut list: Vec<&PluginEntry> = plugins
                    .iter()
                    .filter(|p| tab.matches(p))
                    .filter(|p| !fav_only || favorites.iter().any(|f| f == &p.id))
                    .filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter))
                    .collect();
                list.sort_by(|a, b| a.vendor.cmp(&b.vendor).then_with(|| a.name.cmp(&b.name)));
                let mut last_vendor: Option<&str> = None;
                for p in list {
                    if !p.vendor.is_empty() && last_vendor != Some(p.vendor.as_str()) {
                        vendor_header(ui, &p.vendor);
                        last_vendor = Some(p.vendor.as_str());
                    }
                    any = true;
                    let enabled = p.error.is_none() && kind.can_pick(p);
                    let selected = app.mix.picker_selected.as_deref() == Some(p.id.as_str());
                    let resp = plugin_row(
                        ui,
                        plugin_kind_icon(p),
                        &p.display_name(),
                        p.format,
                        enabled,
                        selected,
                    );
                    if let Some(err) = &p.error {
                        // 加载失败的插件：灰色不可选，hover 显示原因。
                        resp.on_hover_text(err);
                        continue;
                    }
                    if !kind.can_pick(p) {
                        resp.on_hover_text(t!("mix.picker_not_applicable"));
                        continue;
                    }
                    let resp = resp.on_hover_text(t!("hint.mix.picker_plugin"));
                    if resp.clicked() {
                        clicked = Some(p.id.clone());
                    }
                    if resp.double_clicked() {
                        activated = Some(p.id.clone());
                    }
                }
            }
            if !any {
                ui.label(
                    egui::RichText::new(t!("mix.picker_empty").to_string())
                        .color(crate::theme::text_muted()),
                );
            }
        });
    if let Some(sel) = clicked {
        app.mix.picker_selected = Some(sel);
    }
    if let Some(sel) = activated {
        app.mix.picker_selected = Some(sel);
        pick_selected(app, kind, actions);
        *close = true;
    }
}

/// 右侧信息面板：选中插件的名称/厂商/格式/路径 + 收藏 star 开关。
fn picker_info(app: &mut App, ui: &mut egui::Ui) {
    let info_w = ui.available_width().max(1.0);
    ui.allocate_ui_with_layout(
        egui::vec2(info_w, ui.available_height()),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.add_space(4.0);
            let Some(sel) = app.mix.picker_selected.clone() else {
                ui.label(
                    egui::RichText::new(t!("mix.picker_info_empty").to_string())
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
                return;
            };
            if sel == XS_SELECTED_ID {
                ui.label(
                    egui::RichText::new("XSynth")
                        .size(crate::theme::SUB_TITLE_FONT)
                        .color(crate::theme::text_primary()),
                );
                ui.label(
                    egui::RichText::new(t!("mix.picker_tab_instrument").to_string())
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_label()),
                );
                return;
            }
            let Some(p) = app
                .mix
                .scanned
                .as_ref()
                .and_then(|l| l.iter().find(|p| p.id == sel))
                .cloned()
            else {
                return;
            };
            let is_fav = app
                .audio_settings
                .favorite_plugins
                .iter()
                .any(|f| f == &p.id);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&p.name)
                        .size(crate::theme::SUB_TITLE_FONT)
                        .color(crate::theme::text_primary()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (icon, color) = if is_fav {
                        (
                            egui_material_icons::icons::ICON_STAR,
                            crate::theme::warning_gold(),
                        )
                    } else {
                        (
                            egui_material_icons::icons::ICON_STAR.outlined(),
                            crate::theme::text_muted(),
                        )
                    };
                    let resp = crate::widgets::flat::flat_button_filled_ghost(
                        ui,
                        icon.rich_text().size(crate::theme::ICON_FONT).color(color),
                        egui::vec2(24.0, 24.0),
                        None,
                        true,
                    )
                    .on_hover_text(t!("hint.mix.picker_favorite"));
                    if resp.clicked() {
                        if is_fav {
                            app.audio_settings.favorite_plugins.retain(|f| f != &p.id);
                        } else {
                            app.audio_settings.favorite_plugins.push(p.id.clone());
                        }
                        app.audio_settings.save();
                    }
                });
            });
            ui.add_space(4.0);
            let kind_text = if p.is_instrument {
                t!("mix.picker_tab_instrument")
            } else if p.is_effect {
                t!("mix.picker_tab_effect")
            } else {
                t!("mix.picker_tab_extension")
            };
            ui.label(
                egui::RichText::new(format!(
                    "{} · {}",
                    kind_text,
                    match p.format {
                        yinhe_mixer::PluginFormat::Clap => "CLAP",
                        yinhe_mixer::PluginFormat::Vst3 => "VST3",
                        yinhe_mixer::PluginFormat::Builtin => "Yinhe",
                    }
                ))
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_label()),
            );
            ui.label(
                egui::RichText::new(&p.vendor)
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_label()),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(p.path.display().to_string())
                    .size(crate::theme::SMALL_LABEL_FONT)
                    .color(crate::theme::text_muted()),
            );
        },
    );
}

/// 统一插件选择器窗口：左侧分类（全部/乐器/效果器/扩展，贴窗口左缘）+ 中间列表 +
/// 右侧插件信息与收藏 + 底部按钮。单击选中、双击或「好」加载选中项。
fn picker_window(
    app: &mut App,
    ctx: &egui::Context,
    kind: PickerKind,
    actions: &mut Vec<MixAction>,
) {
    let id = kind.viewport_id();
    crate::chrome::dialog::raise_on_open(ctx, id);
    let title = t!("mix.picker_title").to_string();
    let mut close = false;
    ctx.show_viewport_immediate(
        id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [680.0, 460.0], true),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            let mut closed = close;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut closed, false);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            // 左栏贴窗口左缘（无内边距、无底色），列表与信息保留右/下边距。
                            left: 0,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            // 左栏分类 + 中间列表上下贯通到窗口底缘；只有右栏
                            // （插件信息）带底部按钮行，按钮只占右侧栏宽度。
                            let btn_zone = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::widgets::grid::three_columns(
                                ui,
                                12.0,
                                crate::theme::TOOL_BAR_W,
                                190.0,
                                |cells| {
                                    cells.cell(|ui| picker_tab_bar(app, ui));
                                    cells.cell(|ui| {
                                        picker_list(app, ui, kind, actions, &mut close)
                                    });
                                    cells.cell(|ui| {
                                        crate::widgets::grid::content_footer(
                                            ui,
                                            0.0,
                                            btn_zone,
                                            |cells| {
                                                cells.cell(|ui| picker_info(app, ui));
                                                cells.cell(|ui| {
                                                    // 底部按钮贴底（bottom_up）：下边距与右边距一致。
                                                    ui.with_layout(
                                                        egui::Layout::bottom_up(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            let ok_text = t!("mix.picker_ok");
                                                            let cancel_text =
                                                                t!("mix.picker_cancel");
                                                            let ok_enabled = app
                                                                .mix
                                                                .picker_selected
                                                                .is_some();
                                                            if let Some(idx) =
                                                                crate::chrome::dialog_buttons::dialog_button_row(
                                                                    ui,
                                                                    &[
                                                                        crate::chrome::dialog_buttons::DialogButton::secondary(
                                                                            cancel_text.as_ref(),
                                                                        ),
                                                                        crate::chrome::dialog_buttons::DialogButton::primary(
                                                                            ok_text.as_ref(),
                                                                        )
                                                                        .enabled(ok_enabled),
                                                                    ],
                                                                )
                                                            {
                                                                match idx {
                                                                    0 => close = true,
                                                                    1 => {
                                                                        pick_selected(
                                                                            app, kind, actions,
                                                                        );
                                                                        close = true;
                                                                    }
                                                                    _ => {}
                                                                }
                                                            }
                                                        },
                                                    );
                                                });
                                            },
                                        );
                                    });
                                },
                            );
                        });
                });
            if closed {
                close = true;
            }
        },
    );
    if close {
        crate::chrome::dialog::mark_viewport_closed(ctx, id);
        kind.clear_open(app);
    }
}

/// 统一插件选择器（独立 OS 窗口）：效果器语境（insert 链）。
/// 左侧分类可切到乐器/扩展/收藏；乐器插件在此语境不可选。
pub(crate) fn plugin_picker(
    app: &mut App,
    ctx: &egui::Context,
    target: InsertTarget,
    actions: &mut Vec<MixAction>,
) {
    picker_window(app, ctx, PickerKind::Effect { target }, actions);
}

/// 统一插件选择器（独立 OS 窗口）：乐器语境（MIDI 通道挂载）。
/// 初始分类为乐器；选择效果器则加到该通道 insert 链；选择 XSynth 清空插件挂载。
pub(crate) fn instrument_picker(
    app: &mut App,
    ctx: &egui::Context,
    channel: u8,
    actions: &mut Vec<MixAction>,
) {
    picker_window(app, ctx, PickerKind::Instrument { channel }, actions);
}

/// 插件分类图标：乐器 / 效果器 / 扩展（VST/CLAP 但不声明类型时）。
///
/// 列表行用它做视觉分类标识；XSynth 是内置采样器，按乐器显示。
fn plugin_kind_icon(plugin: &PluginEntry) -> egui_material_icons::MaterialIcon {
    use egui_material_icons::icons::*;
    if plugin.is_instrument {
        ICON_PIANO
    } else if plugin.is_effect {
        ICON_GRADIENT
    } else {
        ICON_EXTENSION
    }
}

/// 插件选择器的一行：左分类图标 + 名称（超宽裁剪）+ 右格式标签（CLAP / VST3）。
///
/// `enabled = false`（加载失败/语境不支持）时灰色且不响应点击；
/// `selected` 时整行选中底色；hover 提示由调用方追加。
fn plugin_row(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    name: &str,
    format: yinhe_mixer::PluginFormat,
    enabled: bool,
    selected: bool,
) -> egui::Response {
    let row_h = 24.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );
    let text_color = if enabled {
        crate::theme::text_primary()
    } else {
        crate::theme::text_muted()
    };
    if selected {
        ui.painter()
            .rect_filled(rect, 0.0, crate::theme::selected_bg());
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(
            rect,
            0.0,
            crate::theme::hover_color(crate::theme::track_bg()),
        );
    }
    let font = egui::FontId::proportional(crate::theme::SMALL_FONT);
    // 名称（左侧，超宽裁剪；右侧留 56px 给格式标签）。图标与文字同一 LayoutJob，
    // 保证基线与行高一致；图标用 Material 家族，避免被正文私有字形抢占。
    let name_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - 56.0, rect.max.y));
    let job =
        crate::widgets::icon_text::icon_text(icon, name, crate::theme::SMALL_FONT, text_color);
    let galley = ui.painter().layout_job(job);
    let pos = egui::pos2(rect.min.x + 6.0, rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(name_rect)
        .galley(pos, galley, text_color);
    // 格式标签（右对齐）。
    let fmt = match format {
        yinhe_mixer::PluginFormat::Clap => "CLAP",
        yinhe_mixer::PluginFormat::Vst3 => "VST3",
        // 内置内容（如 XSynth）归 yinhe 自身，而非 DSP 插件格式。
        yinhe_mixer::PluginFormat::Builtin => "Yinhe",
    };
    ui.painter().text(
        egui::pos2(rect.max.x - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        fmt,
        font,
        crate::theme::text_muted(),
    );
    if enabled {
        resp
    } else {
        resp.on_hover_cursor(egui::CursorIcon::NotAllowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 电平表矩形：y 从 0（顶 = +6dB）到 100（底 = -60dB）。
    const TOP: f32 = 0.0;
    const BOTTOM: f32 = 100.0;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3
    }

    #[test]
    fn gain_y_boundaries() {
        // 静音（≤ -60dB）= 最底部。
        assert!(approx(gain_y(0.0, TOP, BOTTOM), BOTTOM));
        // 0dB（gain = 1）位于 DB_MIN..DB_MAX 的 60/66 处。
        assert!(approx(
            gain_y(1.0, TOP, BOTTOM),
            BOTTOM - 60.0 / 66.0 * 100.0
        ));
        // 超过 DB_MAX 的增益顶格。
        assert!(approx(gain_y(db_to_gain(DB_MAX), TOP, BOTTOM), TOP));
        assert!(approx(gain_y(10.0, TOP, BOTTOM), TOP));
    }

    #[test]
    fn gain_at_y_boundaries() {
        assert_eq!(gain_at_y(BOTTOM, TOP, BOTTOM), 0.0);
        assert!(approx(gain_at_y(TOP, TOP, BOTTOM), db_to_gain(DB_MAX)));
        // y 超出矩形：夹到两端。
        assert_eq!(gain_at_y(BOTTOM + 50.0, TOP, BOTTOM), 0.0);
        assert!(approx(
            gain_at_y(TOP - 50.0, TOP, BOTTOM),
            db_to_gain(DB_MAX)
        ));
    }

    #[test]
    fn gain_position_round_trip() {
        for gain in [0.0, 0.01, 0.1, 0.5, 1.0, 1.5, db_to_gain(DB_MAX)] {
            let y = gain_y(gain, TOP, BOTTOM);
            let back = gain_at_y(y, TOP, BOTTOM);
            assert!(approx(back, gain), "gain={gain} y={y} back={back}");
        }
    }

    /// 自动高度：基础布局随总线数按发送区行高线性增长，默认总线数下
    /// 与 `LayoutSettings::default().mix_height` 一致。
    #[test]
    fn auto_height_tracks_bus_rows() {
        let base = auto_panel_height(0);
        assert!((base - 200.0).abs() < 1e-3, "基础高度 {base}");
        assert!((auto_panel_height(1) - base).abs() < 1e-3);
        assert!((auto_panel_height(4) - (base + 3.0 * SEND_ROW_H)).abs() < 1e-3);
    }

    /// 实验：两个通道条共享同一 `mix_send_scroll` 滚动位置时，
    /// 检查 ScrollArea 内部是否产生 ID 冲突（滚动条 / 拖拽捕获区）。
    #[test]
    fn send_blocks_shared_scroll_no_id_clash() {
        let ctx = egui::Context::default();
        let mut actions: Vec<MixAction> = Vec::new();
        // 两帧：滚动条显隐由上一帧内容尺寸决定。
        for _ in 0..2 {
            let output = ctx.run_ui(Default::default(), |ui| {
                ui.horizontal_top(|ui| {
                    // 固定一行高：4 条总线内容溢出 → 滚动条出现（与通道条一致）。
                    section(
                        ui,
                        SEND_ROW_H,
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| send_block(ui, SendSource::Channel(0), &[], 4, &mut actions),
                    );
                    section(
                        ui,
                        SEND_ROW_H,
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| send_block(ui, SendSource::Channel(1), &[], 4, &mut actions),
                    );
                });
            });
            let mut clashes: Vec<String> = Vec::new();
            for cs in &output.shapes {
                if let egui::Shape::Text(t) = &cs.shape
                    && t.galley.text().contains("of widget ID")
                {
                    clashes.push(t.galley.text().to_owned());
                }
            }
            output.drop_without_applying_deltas();
            assert!(clashes.is_empty(), "send_block ID 冲突: {clashes:?}");
        }
    }
}
