//! 混音台通道条控件：轨道色条 / 标签 / insert / M-S / 声像 / 推子 / 电平表。
//!
//! 视觉参考 Bitwig / Studio One：顶部轨道色条、紧凑标签、自绘推子
//! （手柄 + 0dB 刻度 + 双击归零）、自绘电平表（绿黄红 + 0dB 刻度）、
//! insert 空态收缩为一行。通道条撑满混音台高度，底部推子区从下往上排，
//! insert 高度变化不影响推子对齐。

use eframe::egui;
use rust_i18n::t;
use yinhe_audio::InsertTarget;
use yinhe_mixer::{InsertRef, MasterParams, MixerParams, StripParams};

use crate::app::App;

use super::plugin_instance::PluginEntry;
use super::rack::SlotRuntime;
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
const BOTTOM_H: f32 = 80.0;

/// dB 值 → 纵向占比（0 = DB_MIN，1 = DB_MAX）。
fn db_frac(db: f32) -> f32 {
    ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0)
}

/// 推子：线性增益 → 推子矩形内的 y 坐标（top = 最大 dB）。
fn fader_y(gain: f32, top: f32, bottom: f32) -> f32 {
    bottom - db_frac(gain_to_db(gain)) * (bottom - top)
}

/// 推子：推子矩形内的 y 坐标 → 线性增益（超出范围夹到两端）。
fn fader_gain_at_y(y: f32, top: f32, bottom: f32) -> f32 {
    let t = ((bottom - y) / (bottom - top)).clamp(0.0, 1.0);
    db_to_gain(DB_MIN + t * (DB_MAX - DB_MIN))
}

/// 声像：声像值（-1..1）→ 声像条矩形内的 x 坐标。
fn pan_x(pan: f32, center: f32, half_width: f32) -> f32 {
    center + pan.clamp(-1.0, 1.0) * half_width
}

/// 声像：声像条矩形内的 x 坐标 → 声像值（超出范围夹到 -1..1）。
fn pan_value_at_x(x: f32, center: f32, half_width: f32) -> f32 {
    ((x - center) / half_width).clamp(-1.0, 1.0)
}

/// insert 链的展示视图：名称/旁通借用自持久化链，GUI 状态借用自机架。
/// 避免每帧为每个 strip 克隆 insert 名称。
struct InsertView<'a> {
    refs: &'a [InsertRef],
    runtime: &'a [SlotRuntime],
}

impl InsertView<'_> {
    /// 某槽位的插件 GUI 是否打开（机架槽位缺失时 false）。
    fn gui_open(&self, slot: usize) -> bool {
        self.runtime.get(slot).is_some_and(|rt| rt.gui_open)
    }
}

/// 采集某目标的 insert 展示视图（链或机架槽位缺失时为空）。
fn insert_view<'a>(
    mixer: &'a MixerParams,
    rack: Option<&'a super::MixerRack>,
    target: InsertTarget,
) -> InsertView<'a> {
    let refs: &[InsertRef] = match target {
        InsertTarget::Channel(ch) => mixer
            .channel_inserts
            .get(ch as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        InsertTarget::Audio(ch) => mixer
            .audio_inserts
            .get(ch as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        InsertTarget::Bus(bus) => mixer
            .bus_inserts
            .get(bus as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        InsertTarget::Master => &mixer.master_inserts,
    };
    InsertView {
        refs,
        runtime: rack.map(|r| r.chain(target)).unwrap_or(&[]),
    }
}

/// 顶部工具条：扫描插件 + 状态信息。
pub(crate) fn show_toolbar(app: &mut App, ui: &mut egui::Ui, actions: &mut Vec<MixAction>) {
    ui.horizontal(|ui| {
        let scan_resp = crate::widgets::flat::flat_button(ui, t!("mix.scan_plugins"));
        crate::widgets::hint::hover(ui.ctx(), &scan_resp, t!("hint.mix.scan_plugins"));
        if scan_resp.clicked() {
            actions.push(MixAction::RescanPlugins);
        }
        let add_bus_resp = crate::widgets::flat::flat_button(ui, t!("mix.add_bus"));
        crate::widgets::hint::hover(ui.ctx(), &add_bus_resp, t!("hint.mix.add_bus"));
        if add_bus_resp.clicked() {
            actions.push(MixAction::AddBus);
        }
        if app.mix.scan_in_progress {
            ui.label(
                egui::RichText::new(t!("mix.scanning"))
                    .small()
                    .color(crate::theme::text_secondary()),
            );
        } else if let Some(plugins) = &app.mix.scanned {
            let effects = plugins.iter().filter(|p| p.is_effect).count();
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
    let target = InsertTarget::Channel(channel);
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        target,
    );
    let send_count = app.workspace.documents[idx]
        .mixer
        .sends
        .get(channel as usize)
        .map(|l| l.iter().filter(|s| s.amount > 0.0).count())
        .unwrap_or(0);

    let plugin_name = app.workspace.documents[idx]
        .mixer
        .instruments
        .get(channel as usize)
        .and_then(|o| o.as_ref())
        .map(|r| r.name.clone());

    strip_frame(ui, color, height, |ui| {
        label_block(ui, channel_label(channel), &names);
        instrument_slot(ui, channel, plugin_name.as_deref(), actions);

        strip_body(
            ui,
            target,
            params.gain,
            peak,
            &view,
            |ui, _| ui.add_space(4.0),
            |g| MixAction::SetStrip {
                channel,
                params: StripParams { gain: g, ..params },
            },
            actions,
        );
        if strip_bottom(
            ui,
            params.gain,
            BottomMs::Pan(&params),
            SendSlot::Button { send_count },
            |p| actions.push(MixAction::SetStrip { channel, params: p }),
        ) {
            actions.push(MixAction::OpenSends { channel });
        }
    });
}

/// MIDI 通道条上的乐器设备行：当前乐器名（内置 XSynth / 插件名）+
/// 界面（插件 GUI / XSynth 音色库）、参数、更换乐器入口。
/// 内置 XSynth 与插件走同一套入口；在乐器选择器里可互相切换。
fn instrument_slot(
    ui: &mut egui::Ui,
    channel: u8,
    plugin_name: Option<&str>,
    actions: &mut Vec<MixAction>,
) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(t!("mix.instrument"))
            .size(crate::theme::SMALL_LABEL_FONT)
            .color(crate::theme::text_muted()),
    );
    let name = plugin_name.unwrap_or("XSynth");
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(name)
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_primary()),
        )
        .truncate()
        .sense(egui::Sense::click()),
    );
    if resp.clicked() {
        if plugin_name.is_some() {
            actions.push(MixAction::ToggleInstrumentGui { channel });
        } else {
            actions.push(MixAction::OpenXsynthConfig { channel });
        }
    }
    crate::widgets::hint::hover(
        ui.ctx(),
        &resp,
        if plugin_name.is_some() {
            t!("hint.mix.toggle_gui")
        } else {
            t!("hint.mix.instrument_xsynth")
        },
    );
    ui.horizontal_wrapped(|ui| {
        let params_resp = ui.small_button(t!("mix.params"));
        crate::widgets::hint::hover(ui.ctx(), &params_resp, t!("hint.mix.instrument_params"));
        if params_resp.clicked() {
            if plugin_name.is_some() {
                actions.push(MixAction::OpenInstrumentParams { channel });
            } else {
                actions.push(MixAction::OpenXsynthConfig { channel });
            }
        }
        let change_resp = ui.small_button(t!("mix.change_instrument"));
        crate::widgets::hint::hover(ui.ctx(), &change_resp, t!("hint.mix.change_instrument"));
        if change_resp.clicked() {
            actions.push(MixAction::OpenInstrumentPicker { channel });
        }
    });
}

/// 通道条主体（insert 链 + 推子），四种条（MIDI/音频/总线/主输出）共用。
/// `pre_insert` 画 insert 前的条头（总线的删除按钮；其余仅留 4px 间距），
/// `on_strip` 把新 StripParams 转成对应命名空间的 MixAction 变体。
#[allow(clippy::too_many_arguments)] // strip 渲染上下文透传
fn strip_body(
    ui: &mut egui::Ui,
    target: InsertTarget,
    gain: f32,
    peak: (f32, f32),
    insert: &InsertView<'_>,
    mut pre_insert: impl FnMut(&mut egui::Ui, &mut Vec<MixAction>),
    mut on_gain: impl FnMut(f32) -> MixAction,
    actions: &mut Vec<MixAction>,
) {
    pre_insert(ui, actions);
    insert_area(ui, target, insert, actions);
    ui.add_space(4.0);

    // 推子占满 insert 之下的剩余空间（底部固定区之外），
    // 因此 insert 高度变化只影响推子顶部，底部对齐不变。
    let fader_h = (ui.available_height() - BOTTOM_H).max(FADER_MIN_H);
    fader_and_meter(ui, gain, peak, fader_h, |g| actions.push(on_gain(g)));
}

/// 底部 M/S/声像区模式。
enum BottomMs<'a> {
    /// M/S 按钮 + 声像条（用该 strip 的参数）。
    Pan(&'a StripParams),
    /// 无 M/S/声像：按 `ms_pan_block` 实际高度占位（主输出）。
    Placeholder,
}

/// 发送槽模式。
enum SendSlot {
    /// 发送按钮（通道条）。
    Button { send_count: usize },
    /// 与发送按钮同高占位（总线/主输出）。
    Placeholder,
    /// 无发送槽（音频条）。
    None,
}

/// 推子之后的底部固定区：M/S/声像（或等高占位）→ 发送槽 → dB 读数。
/// 返回发送按钮是否被点击。
fn strip_bottom(
    ui: &mut egui::Ui,
    gain: f32,
    ms: BottomMs<'_>,
    send: SendSlot,
    mut on_change: impl FnMut(StripParams),
) -> bool {
    ui.add_space(4.0);
    match ms {
        BottomMs::Pan(params) => ms_pan_block(ui, params, &mut on_change),
        BottomMs::Placeholder => {
            let ms_h = BTN + ui.spacing().item_spacing.y + 14.0;
            ui.allocate_space(egui::vec2(CONTENT_W, ms_h));
        }
    }
    ui.add_space(4.0);
    let mut send_clicked = false;
    match send {
        SendSlot::Button { send_count } => {
            send_clicked = send_button(ui, send_count);
            ui.add_space(4.0);
        }
        SendSlot::Placeholder => {
            ui.allocate_space(egui::vec2(CONTENT_W, 16.0));
            ui.add_space(4.0);
        }
        SendSlot::None => {}
    }
    db_label(ui, gain);
    send_clicked
}

/// 总线条（bus / return）：与通道条同构（insert 链 + 推子 + M/S/声像）。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
pub(crate) fn bus_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    bus: u8,
    peak: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let params = app.workspace.documents[idx]
        .mixer
        .buses
        .get(bus as usize)
        .copied()
        .unwrap_or_default();
    let target = InsertTarget::Bus(bus);
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        target,
    );

    strip_frame(ui, crate::theme::accent_active(), height, |ui| {
        label_block(ui, format!("BUS {}", bus + 1), "");
        strip_body(
            ui,
            target,
            params.gain,
            peak,
            &view,
            |ui, actions| {
                // 删除总线（右上角小按钮；其 send 清理、更高索引前移）。
                ui.horizontal(|ui| {
                    ui.add_space(CONTENT_W - BTN);
                    let del_resp = toggle_button(
                        ui,
                        egui_material_icons::icons::ICON_DELETE.codepoint,
                        false,
                        crate::theme::danger_text(),
                    );
                    crate::widgets::hint::hover(ui.ctx(), &del_resp, t!("hint.mix.remove_bus"));
                    if del_resp.clicked() {
                        actions.push(MixAction::RemoveBus { bus });
                    }
                });
                ui.add_space(2.0);
            },
            |g| MixAction::SetBusStrip {
                bus,
                params: StripParams { gain: g, ..params },
            },
            actions,
        );
        strip_bottom(
            ui,
            params.gain,
            BottomMs::Pan(&params),
            SendSlot::Placeholder,
            |p| {
                actions.push(MixAction::SetBusStrip { bus, params: p });
            },
        );
    });
}

/// 发送入口按钮（通道条底部）：有发送时显示数量；返回是否点击。
fn send_button(ui: &mut egui::Ui, send_count: usize) -> bool {
    let label = if send_count > 0 {
        format!("SEND ({send_count})")
    } else {
        "SEND".to_string()
    };
    let color = if send_count > 0 {
        crate::theme::text_primary()
    } else {
        crate::theme::text_secondary()
    };
    let resp = crate::widgets::flat::flat_button_fixed(
        ui,
        egui::RichText::new(label)
            .size(crate::theme::SMALL_FONT - 1.0)
            .color(color),
        egui::vec2(CONTENT_W, 16.0),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.sends"));
    resp.clicked()
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
                                ui.horizontal(|ui| {
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
                                        format!("{:+.1} dB", crate::mix::gain_to_db(val))
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
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        InsertTarget::Master,
    );

    strip_frame(ui, crate::theme::accent_active(), height, |ui| {
        label_block(ui, t!("mix.master").to_string(), "");
        strip_body(
            ui,
            InsertTarget::Master,
            params.gain,
            peak,
            &view,
            |ui, _| ui.add_space(4.0),
            |g| MixAction::SetMaster {
                params: MasterParams { gain: g },
            },
            actions,
        );
        strip_bottom(
            ui,
            params.gain,
            BottomMs::Placeholder,
            SendSlot::Placeholder,
            |_| {},
        );
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
        crate::widgets::hint::hover(ui.ctx(), &resp, names);
    }
}

/// insert 槽位区：空态收缩为一行「+」；有内容时紧凑列表（最多 INSERT_MAX_H）。
fn insert_area(
    ui: &mut egui::Ui,
    target: InsertTarget,
    insert: &InsertView<'_>,
    actions: &mut Vec<MixAction>,
) {
    if insert.refs.is_empty() {
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
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.add_insert"));
        if resp.clicked() {
            actions.push(MixAction::OpenPicker { target });
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
                    for (slot, r) in insert.refs.iter().enumerate() {
                        insert_row(
                            ui,
                            target,
                            slot,
                            &r.name,
                            r.bypassed,
                            insert.gui_open(slot),
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
    target: InsertTarget,
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
        ui.id().with(("mix_insert_bypass", target, slot)),
        egui::Sense::click(),
    );
    if dot_resp.clicked() {
        actions.push(MixAction::BypassInsert {
            target,
            slot,
            bypassed: !is_bypassed,
        });
    }
    crate::widgets::hint::hover(ui.ctx(), &dot_resp, t!("hint.mix.bypass"));

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
        ui.id().with(("mix_insert_params", target, slot)),
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
    crate::widgets::hint::hover(ui.ctx(), &params_resp, t!("hint.mix.insert_params"));
    if params_resp.clicked() {
        actions.push(MixAction::OpenInsertParams { target, slot });
    }

    // 行点击：打开/关闭插件原生界面。
    if resp.clicked() {
        actions.push(MixAction::ToggleGui { target, slot });
    }
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.toggle_gui"));
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
                target,
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
            actions.push(MixAction::ToggleGui { target, slot });
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
            actions.push(MixAction::OpenInsertParams { target, slot });
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
            actions.push(MixAction::RemoveInsert { target, slot });
            ui.close();
        }
    });
}

/// M/S 按钮 + 声像条（紧凑两行，居中）。
fn ms_pan_block(ui: &mut egui::Ui, params: &StripParams, mut on_change: impl FnMut(StripParams)) {
    ui.horizontal(|ui| {
        let total = BTN * 2.0 + 4.0;
        ui.add_space(((CONTENT_W - total) / 2.0).max(0.0));
        let m_resp = toggle_button(ui, "M", params.mute, crate::theme::mute_active());
        crate::widgets::hint::hover(ui.ctx(), &m_resp, t!("hint.mix.mute"));
        if m_resp.clicked() {
            let mut p = *params;
            p.mute = !p.mute;
            on_change(p);
        }
        ui.add_space(4.0);
        let s_resp = toggle_button(ui, "S", params.solo, crate::theme::solo_active());
        crate::widgets::hint::hover(ui.ctx(), &s_resp, t!("hint.mix.solo"));
        if s_resp.clicked() {
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
    painter.rect_filled(rect, 3.0, bg);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        fg,
    );
    resp
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
    let x = pan_x(pan, rect.center().x, rect.width() / 2.0);
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

    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            return Some(pan_value_at_x(pos.x, rect.center().x, rect.width() / 2.0));
        }
    } else if resp.double_clicked() {
        return Some(0.0);
    }
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.pan"));
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
    let y = fader_y(gain, rect.min.y, rect.max.y);

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
            on_gain(fader_gain_at_y(pos.y, rect.min.y, rect.max.y));
        }
    } else if resp.double_clicked() {
        on_gain(1.0);
    }
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.fader"));
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
        match self {
            Self::Effect { .. } => egui::ViewportId::from_hash_of("mix_plugin_picker"),
            Self::Instrument { .. } => egui::ViewportId::from_hash_of("mix_instrument_picker"),
        }
    }

    fn title(self) -> String {
        match self {
            Self::Effect { .. } => t!("mix.picker_title").to_string(),
            Self::Instrument { .. } => t!("mix.instrument_picker_title").to_string(),
        }
    }

    /// 列表滚动区的 id_salt（两个窗口可能同时打开，滚动状态需区分）。
    fn list_id(self) -> &'static str {
        match self {
            Self::Effect { .. } => "mix_plugin_picker_list",
            Self::Instrument { .. } => "mix_instrument_picker_list",
        }
    }

    fn empty_text(self) -> String {
        match self {
            Self::Effect { .. } => t!("mix.no_plugins").to_string(),
            Self::Instrument { .. } => t!("mix.no_instruments").to_string(),
        }
    }

    /// 该插件是否属于本选择器。
    fn matches(self, plugin: &PluginEntry) -> bool {
        match self {
            Self::Effect { .. } => plugin.is_effect,
            Self::Instrument { .. } => plugin.is_instrument,
        }
    }

    /// 是否显示内置 XSynth 项（仅乐器选择器）。
    fn shows_xsynth(self) -> bool {
        matches!(self, Self::Instrument { .. })
    }

    /// 选中一个插件条目。
    fn pick_plugin(self, plugin: &PluginEntry, actions: &mut Vec<MixAction>) {
        match self {
            Self::Effect { target } => actions.push(MixAction::AddInsert {
                target,
                plugin: plugin.clone(),
            }),
            Self::Instrument { channel } => actions.push(MixAction::AssignInstrument {
                channel,
                plugin: plugin.clone(),
            }),
        }
    }

    /// 点击内置 XSynth（仅乐器）：清除该通道的插件挂载。
    fn pick_builtin(self, actions: &mut Vec<MixAction>) {
        if let Self::Instrument { channel } = self {
            actions.push(MixAction::RemoveInstrument { channel });
        }
    }

    /// 窗口关闭后清理对应的打开标记。
    fn clear_open(self, app: &mut App) {
        match self {
            Self::Effect { .. } => app.mix.picker_for = None,
            Self::Instrument { .. } => app.mix.instrument_picker_for = None,
        }
    }
}

/// 插件/乐器选择器的公共窗口实现（仅过滤、选中行为与文案不同）。
fn picker_window(
    app: &mut App,
    ctx: &egui::Context,
    kind: PickerKind,
    actions: &mut Vec<MixAction>,
) {
    let id = kind.viewport_id();
    crate::chrome::dialog::raise_on_open(ctx, id);
    let title = kind.title();
    let mut close = false;
    ctx.show_viewport_immediate(
        id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [340.0, 420.0], true),
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
                            ui.horizontal(|ui| {
                                ui.label(t!("mix.search"));
                                let search_resp =
                                    ui.text_edit_singleline(&mut app.mix.picker_filter);
                                crate::widgets::hint::hover(
                                    ui.ctx(),
                                    &search_resp,
                                    t!("hint.mix.picker_search"),
                                );
                            });
                            ui.separator();
                            let filter = app.mix.picker_filter.to_lowercase();
                            let plugins = app.mix.scanned.as_ref();
                            egui::ScrollArea::vertical()
                                .id_salt(kind.list_id())
                                .auto_shrink([false, false])
                                .max_height(ui.available_height())
                                .show(ui, |ui| {
                                    let mut any = false;
                                    // 内置 XSynth：默认乐器（选择 = 清除插件挂载）。
                                    if kind.shows_xsynth()
                                        && (filter.is_empty() || "xsynth".contains(&filter))
                                    {
                                        any = true;
                                        let xsynth_resp = ui.selectable_label(false, "XSynth");
                                        crate::widgets::hint::hover(
                                            ui.ctx(),
                                            &xsynth_resp,
                                            t!("hint.mix.picker_xsynth"),
                                        );
                                        if xsynth_resp.clicked() {
                                            kind.pick_builtin(actions);
                                        }
                                    }
                                    if let Some(plugins) = plugins {
                                        for p in plugins.iter().filter(|p| kind.matches(p)) {
                                            if !filter.is_empty()
                                                && !p.name.to_lowercase().contains(&filter)
                                            {
                                                continue;
                                            }
                                            any = true;
                                            if let Some(err) = &p.error {
                                                // 加载失败的插件：灰色不可选，hover 显示原因（不静默消失）。
                                                plugin_row(ui, &p.display_name(), p.format, false)
                                                    .on_hover_text(err);
                                                continue;
                                            }
                                            let row_resp = plugin_row(ui, &p.name, p.format, true)
                                                .on_hover_text(&p.id);
                                            crate::widgets::hint::hover(
                                                ui.ctx(),
                                                &row_resp,
                                                t!("hint.mix.picker_plugin"),
                                            );
                                            if row_resp.clicked() {
                                                kind.pick_plugin(p, actions);
                                            }
                                        }
                                    }
                                    if !any {
                                        ui.label(
                                            egui::RichText::new(kind.empty_text())
                                                .color(crate::theme::text_muted()),
                                        );
                                    }
                                });
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

/// 插件选择器窗口（独立 OS 窗口；列出扫描到的效果器，按名称过滤）。
pub(crate) fn plugin_picker(
    app: &mut App,
    ctx: &egui::Context,
    target: InsertTarget,
    actions: &mut Vec<MixAction>,
) {
    picker_window(app, ctx, PickerKind::Effect { target }, actions);
}

/// 音频通道条：标签 + insert 链 + 推子/M/S/声像。
/// 多条音频轨共享同一音频通道 = 共享本条（与乐器通道同构）。
pub(crate) fn audio_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    channel: u16,
    peak: (f32, f32),
    height: f32,
    actions: &mut Vec<MixAction>,
) {
    let params = app.workspace.documents[idx].mixer.audio_strip(channel);
    let target = InsertTarget::Audio(channel);
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        target,
    );

    strip_frame(ui, crate::theme::accent_active(), height, |ui| {
        label_block(ui, crate::mix::audio_label(channel), "");
        strip_body(
            ui,
            target,
            params.gain,
            peak,
            &view,
            |ui, _| ui.add_space(4.0),
            |g| MixAction::SetAudioStrip {
                channel,
                params: StripParams { gain: g, ..params },
            },
            actions,
        );
        strip_bottom(
            ui,
            params.gain,
            BottomMs::Pan(&params),
            SendSlot::None,
            |p| {
                actions.push(MixAction::SetAudioStrip { channel, params: p });
            },
        );
    });
}

/// 插件选择器的一行：左名称（超宽裁剪）+ 右格式标签（CLAP / VST3）。
///
/// `enabled = false`（加载失败）时灰色且不响应点击；hover 提示由调用方追加。
fn plugin_row(
    ui: &mut egui::Ui,
    name: &str,
    format: yinhe_mixer::PluginFormat,
    enabled: bool,
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
    if enabled && resp.hovered() {
        ui.painter().rect_filled(
            rect,
            3.0,
            crate::theme::hover_color(crate::theme::track_bg()),
        );
    }
    let font = egui::FontId::proportional(crate::theme::SMALL_FONT);
    // 名称（左侧，超宽裁剪；右侧留 56px 给格式标签）。
    let name_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - 56.0, rect.max.y));
    ui.painter().with_clip_rect(name_rect).text(
        egui::pos2(rect.min.x + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        font.clone(),
        text_color,
    );
    // 格式标签（右对齐）。
    let fmt = match format {
        yinhe_mixer::PluginFormat::Clap => "CLAP",
        yinhe_mixer::PluginFormat::Vst3 => "VST3",
        yinhe_mixer::PluginFormat::Builtin => "DSP",
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

/// 乐器选择器（独立 OS 窗口）：内置 XSynth + 全部 is_instrument() 插件。
/// 选择 XSynth 即清除该通道的插件挂载（回到默认内置乐器）。
pub(crate) fn instrument_picker(
    app: &mut App,
    ctx: &egui::Context,
    channel: u8,
    actions: &mut Vec<MixAction>,
) {
    picker_window(app, ctx, PickerKind::Instrument { channel }, actions);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 推子矩形：y 从 0（顶 = +6dB）到 100（底 = -60dB）。
    const TOP: f32 = 0.0;
    const BOTTOM: f32 = 100.0;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3
    }

    #[test]
    fn fader_y_boundaries() {
        // 静音（≤ -60dB）= 最底部。
        assert!(approx(fader_y(0.0, TOP, BOTTOM), BOTTOM));
        // 0dB（gain = 1）位于 DB_MIN..DB_MAX 的 60/66 处。
        assert!(approx(
            fader_y(1.0, TOP, BOTTOM),
            BOTTOM - 60.0 / 66.0 * 100.0
        ));
        // 超过 DB_MAX 的增益顶格。
        assert!(approx(fader_y(db_to_gain(DB_MAX), TOP, BOTTOM), TOP));
        assert!(approx(fader_y(10.0, TOP, BOTTOM), TOP));
    }

    #[test]
    fn fader_gain_at_y_boundaries() {
        assert_eq!(fader_gain_at_y(BOTTOM, TOP, BOTTOM), 0.0);
        assert!(approx(
            fader_gain_at_y(TOP, TOP, BOTTOM),
            db_to_gain(DB_MAX)
        ));
        // y 超出矩形：夹到两端。
        assert_eq!(fader_gain_at_y(BOTTOM + 50.0, TOP, BOTTOM), 0.0);
        assert!(approx(
            fader_gain_at_y(TOP - 50.0, TOP, BOTTOM),
            db_to_gain(DB_MAX)
        ));
    }

    #[test]
    fn fader_gain_position_round_trip() {
        for gain in [0.0, 0.01, 0.1, 0.5, 1.0, 1.5, db_to_gain(DB_MAX)] {
            let y = fader_y(gain, TOP, BOTTOM);
            let back = fader_gain_at_y(y, TOP, BOTTOM);
            assert!(approx(back, gain), "gain={gain} y={y} back={back}");
        }
    }

    #[test]
    fn pan_x_boundaries_and_clamp() {
        assert!(approx(pan_x(0.0, 100.0, 50.0), 100.0));
        assert!(approx(pan_x(-1.0, 100.0, 50.0), 50.0));
        assert!(approx(pan_x(1.0, 100.0, 50.0), 150.0));
        // 越界值夹到左右端。
        assert!(approx(pan_x(-2.0, 100.0, 50.0), 50.0));
        assert!(approx(pan_x(2.0, 100.0, 50.0), 150.0));
    }

    #[test]
    fn pan_value_at_x_boundaries_and_clamp() {
        assert!(approx(pan_value_at_x(100.0, 100.0, 50.0), 0.0));
        assert!(approx(pan_value_at_x(50.0, 100.0, 50.0), -1.0));
        assert!(approx(pan_value_at_x(150.0, 100.0, 50.0), 1.0));
        // 超出声像条：夹到 -1..1。
        assert!(approx(pan_value_at_x(0.0, 100.0, 50.0), -1.0));
        assert!(approx(pan_value_at_x(200.0, 100.0, 50.0), 1.0));
    }

    #[test]
    fn pan_round_trip() {
        for pan in [-1.0, -0.5, -0.01, 0.0, 0.25, 1.0] {
            let x = pan_x(pan, 100.0, 50.0);
            assert!(approx(pan_value_at_x(x, 100.0, 50.0), pan), "pan={pan}");
        }
    }
}
