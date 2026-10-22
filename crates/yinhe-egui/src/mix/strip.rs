//! 混音台通道条控件。
//!
//! 每个通道 = 一个**直角矩形 + 单层描边**（通道框不用圆角，避免卡片出界）。按钮类
//! 控件用通用圆角矩形。自上而下：
//! 1. 顶部**色带**（轨道色）+ 通道号；
//! 2. **设备链**（若干乐器 / 效果器，固定高度、整行滚动，不撑破通道）；
//! 3. **效果发送**（BUS 1 旋钮；无总线时首次拖动自动创建）；
//! 4. **M/S**（居中）；
//! 5. **声像旋钮**（居中）；
//! 6. **当前增益 dB 读数** + **左侧电平表（带刻度 + 峰值保持）+ 右侧推子**。
//!
//! 各分区高度固定，因此所有通道条的推子/电平表**等高**。

use eframe::egui;
use rust_i18n::t;
use yinhe_audio::InsertTarget;
use yinhe_mixer::{InsertRef, MasterParams, MixerParams, SendParams, StripParams};

use crate::app::App;
use crate::widgets::knob::knob;

use super::plugin_instance::PluginEntry;
use super::rack::SlotRuntime;
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
/// 设备链单行高度（px）。
const DEVICE_ROW_H: f32 = 17.0;
/// 通用按钮圆角（px）。
const RADIUS: f32 = 4.0;
/// 小按钮边长（px，M/S、图标按钮）。
const BTN: f32 = 18.0;
/// 旋钮直径（px，发送/声像）。
const KNOB_D: f32 = 26.0;
/// 分区之间的竖直间隙（px）。
const GAP: f32 = 4.0;
/// 发送 / M-S / 声像 / dB 读数各区高度（px，固定以保证所有条等高）。
const SEND_ROW_H: f32 = KNOB_D;
const MS_ROW_H: f32 = BTN;
const PAN_ROW_H: f32 = KNOB_D;
const DB_ROW_H: f32 = 18.0;
/// 「完整条」（通道 / 音频 / 总线）底部固定区高度。
const BOTTOM_FULL: f32 = SEND_ROW_H + MS_ROW_H + PAN_ROW_H + DB_ROW_H + GAP * 4.0;
/// 主输出条底部固定区高度（只有 dB 读数）。
const BOTTOM_MASTER: f32 = DB_ROW_H + GAP;
/// 推子槽宽（px）。
const FADER_SLOT_W: f32 = 4.0;
/// 推子手柄尺寸（px）。
const FADER_HANDLE_W: f32 = 22.0;
const FADER_HANDLE_H: f32 = 11.0;
/// 推子最小高度（px）。
const FADER_MIN_H: f32 = 60.0;

/// 推子：线性增益 → 推子矩形内的 y 坐标（top = 最大 dB）。
fn fader_y(gain: f32, top: f32, bottom: f32) -> f32 {
    bottom - db_frac(gain_to_db(gain)) * (bottom - top)
}

/// 推子：推子矩形内的 y 坐标 → 线性增益（超出范围夹到两端）。
fn fader_gain_at_y(y: f32, top: f32, bottom: f32) -> f32 {
    let t = ((bottom - y) / (bottom - top)).clamp(0.0, 1.0);
    db_to_gain(DB_MIN + t * (DB_MAX - DB_MIN))
}

/// insert 链的展示视图：名称/旁通借用自持久化链，GUI 状态借用自机架。
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

/// 通道条外框：直角矩形 + 单层描边 + 顶部色带（写标题）。
/// `add_contents` 在色带下方的内容区排布，参数为其可用高度。
/// 返回整条卡片矩形（调用方可在色带行上叠加交互，如总线删除按钮）。
fn strip_frame(
    ui: &mut egui::Ui,
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
        egui::pos2(rect.min.x + PAD_X, header.max.y + 4.0),
        egui::pos2(rect.max.x - PAD_X, rect.max.y - 6.0),
    );
    // 用 `new_child`（**不**推进父布局游标）排布内容：通道条宽度由上面的
    // `allocate_exact_size(STRIP_WIDTH)` 决定。若用 `scope_builder`，它会按子
    // min_rect（被 PAD_X 内缩、更窄）回退父游标，导致下一张卡覆盖本卡右侧。
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    // clip 放宽到整卡片：设备链行整宽贴边（突破内容区左右内缩）不被裁。
    child.set_clip_rect(rect);
    child.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    add_contents(&mut child, content.height());

    // 单层描边确定一个通道。**最后画**：设备链行整宽贴边（不透明填充）会盖住
    // 左右描边，色带同理——描边放最后保证通道之间的分界线贯穿全高、不被内容
    // 打断（色带与设备行都在描边之下）。
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
    let target = InsertTarget::Channel(channel);
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        target,
    );
    let bus_count = app.workspace.documents[idx].mixer.bus_count();
    // 本通道的发送列表：每条总线一个旋钮（数量很小，clone 免借用纠缠）。
    let sends = app.workspace.documents[idx]
        .mixer
        .sends
        .get(channel as usize)
        .cloned()
        .unwrap_or_default();
    let instruments: Vec<String> = app.workspace.documents[idx]
        .mixer
        .instruments
        .get(channel as usize)
        .map(|refs| refs.iter().map(|r| r.name.clone()).collect())
        .unwrap_or_default();
    let title = channel_label(channel);

    let rect = strip_frame(ui, color, &title, height, |ui, content_h| {
        let dev_h = strip_metrics(content_h, true);
        section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
            device_chain(
                ui,
                target,
                &view,
                Some(channel),
                &instruments,
                dev_h,
                actions,
            );
        });
        ui.add_space(GAP);
        section(
            ui,
            SEND_ROW_H,
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                send_block(ui, SendSource::Channel(channel), &sends, bus_count, actions);
            },
        );
        ui.add_space(GAP);
        section(
            ui,
            MS_ROW_H,
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ms_row(ui, &params, |p| {
                    actions.push(MixAction::SetStrip { channel, params: p })
                });
            },
        );
        ui.add_space(GAP);
        section(
            ui,
            PAN_ROW_H,
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                pan_knob(ui, params.pan, |pan| {
                    actions.push(MixAction::SetStrip {
                        channel,
                        params: StripParams { pan, ..params },
                    })
                });
            },
        );
        ui.add_space(GAP);
        if let Some(g) = db_row(ui, params.gain) {
            actions.push(MixAction::SetStrip {
                channel,
                params: StripParams { gain: g, ..params },
            });
        }
        // 推子/电平表吸收剩余高度：底边贴住内容区底边（无下方空白）。
        let bar_h = ui.available_height().max(FADER_MIN_H);
        section(
            ui,
            bar_h,
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                meter_fader(ui, params.gain, level, hold, bar_h, |g| {
                    actions.push(MixAction::SetStrip {
                        channel,
                        params: StripParams { gain: g, ..params },
                    })
                });
            },
        );
    });
    // 双击色带：打开底部设备链并选中该通道。
    header_double_click_opens_dock(
        app,
        ui,
        idx,
        rect,
        crate::chrome::dock_bar::DockContext::Midi(channel),
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
    let target = InsertTarget::Audio(channel);
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        target,
    );
    let title = crate::mix::audio_label(channel);
    // 本音频通道的发送列表与总线总数（发送区行数）。
    let audio_sends = app.workspace.documents[idx]
        .mixer
        .audio_sends
        .get(channel as usize)
        .cloned()
        .unwrap_or_default();
    let bus_count = app.workspace.documents[idx].mixer.bus_count();

    let rect = strip_frame(
        ui,
        crate::theme::accent_active(),
        &title,
        height,
        |ui, content_h| {
            let dev_h = strip_metrics(content_h, true);
            section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
                device_chain(ui, target, &view, None, &[], dev_h, actions);
            });
            ui.add_space(GAP);
            section(
                ui,
                SEND_ROW_H,
                egui::Layout::top_down(egui::Align::LEFT),
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
            section(
                ui,
                MS_ROW_H,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ms_row(ui, &params, |p| {
                        actions.push(MixAction::SetAudioStrip { channel, params: p })
                    });
                },
            );
            ui.add_space(GAP);
            section(
                ui,
                PAN_ROW_H,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    pan_knob(ui, params.pan, |pan| {
                        actions.push(MixAction::SetAudioStrip {
                            channel,
                            params: StripParams { pan, ..params },
                        })
                    });
                },
            );
            ui.add_space(GAP);
            if let Some(g) = db_row(ui, params.gain) {
                actions.push(MixAction::SetAudioStrip {
                    channel,
                    params: StripParams { gain: g, ..params },
                });
            }
            // 推子/电平表吸收剩余高度：底边贴住内容区底边（无下方空白）。
            let bar_h = ui.available_height().max(FADER_MIN_H);
            section(
                ui,
                bar_h,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    meter_fader(ui, params.gain, level, hold, bar_h, |g| {
                        actions.push(MixAction::SetAudioStrip {
                            channel,
                            params: StripParams { gain: g, ..params },
                        })
                    });
                },
            );
        },
    );
    // 双击色带：打开底部设备链并选中该音频通道。
    header_double_click_opens_dock(
        app,
        ui,
        idx,
        rect,
        crate::chrome::dock_bar::DockContext::Audio(channel),
        ("mix_header_dock_audio", channel),
    );
}

/// 总线条（bus / return）：与通道条同构，额外在设备链上方提供删除入口。
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
    let target = InsertTarget::Bus(bus);
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        target,
    );
    let title = format!("BUS {}", bus + 1);
    // 本总线的发送列表（总线 → 总线，含自发送）与总线总数（发送区行数）。
    let bus_sends = app.workspace.documents[idx]
        .mixer
        .bus_sends
        .get(bus as usize)
        .cloned()
        .unwrap_or_default();
    let bus_count = app.workspace.documents[idx].mixer.bus_count();

    let rect = strip_frame(
        ui,
        crate::theme::accent_active(),
        &title,
        height,
        |ui, content_h| {
            let dev_h = strip_metrics(content_h, true);
            section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
                device_chain(ui, target, &view, None, &[], dev_h, actions);
            });
            ui.add_space(GAP);
            section(
                ui,
                SEND_ROW_H,
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    send_block(ui, SendSource::Bus(bus), &bus_sends, bus_count, actions);
                },
            );
            ui.add_space(GAP);
            section(
                ui,
                MS_ROW_H,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ms_row(ui, &params, |p| {
                        actions.push(MixAction::SetBusStrip { bus, params: p })
                    });
                },
            );
            ui.add_space(GAP);
            section(
                ui,
                PAN_ROW_H,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    pan_knob(ui, params.pan, |pan| {
                        actions.push(MixAction::SetBusStrip {
                            bus,
                            params: StripParams { pan, ..params },
                        })
                    });
                },
            );
            ui.add_space(GAP);
            if let Some(g) = db_row(ui, params.gain) {
                actions.push(MixAction::SetBusStrip {
                    bus,
                    params: StripParams { gain: g, ..params },
                });
            }
            // 推子/电平表吸收剩余高度：底边贴住内容区底边（无下方空白）。
            let bar_h = ui.available_height().max(FADER_MIN_H);
            section(
                ui,
                bar_h,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    meter_fader(ui, params.gain, level, hold, bar_h, |g| {
                        actions.push(MixAction::SetBusStrip {
                            bus,
                            params: StripParams { gain: g, ..params },
                        })
                    });
                },
            );
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

/// 主输出条：设备链 + dB 读数 + 电平/推子（无 M/S/声像/发送）。
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
    let view = insert_view(
        &app.workspace.documents[idx].mixer,
        app.mixer_racks.get(idx),
        InsertTarget::Master,
    );
    let title = t!("mix.master").to_string();

    strip_frame(
        ui,
        crate::theme::accent_active(),
        &title,
        height,
        |ui, content_h| {
            let dev_h = strip_metrics(content_h, false);
            section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
                device_chain(ui, InsertTarget::Master, &view, None, &[], dev_h, actions);
            });
            ui.add_space(GAP);
            if let Some(g) = db_row(ui, params.gain) {
                actions.push(MixAction::SetMaster {
                    params: MasterParams { gain: g },
                });
            }
            // 推子/电平表吸收剩余高度：底边贴住内容区底边（无下方空白）。
            let bar_h = ui.available_height().max(FADER_MIN_H);
            section(
                ui,
                bar_h,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    meter_fader(ui, params.gain, level, hold, bar_h, |g| {
                        actions.push(MixAction::SetMaster {
                            params: MasterParams { gain: g },
                        })
                    });
                },
            );
        },
    );
}

/// 设备链可用高度：`full` = 预留发送/M-S/声像（通道/音频/总线），
/// 主输出只预留 dB 读数。推子/电平表吸收剩余高度（各条 `bar_h` 取剩余可用）。
fn strip_metrics(content_h: f32, full: bool) -> f32 {
    let reserved = if full { BOTTOM_FULL } else { BOTTOM_MASTER };
    // 设备区最多占 35%（避免把额外高度全吃掉、把推子/电平表压到最小）。
    let max_dev = (content_h * 0.35).max(DEVICE_ROW_H);
    (content_h - reserved - FADER_MIN_H).clamp(DEVICE_ROW_H, max_dev)
}

/// 色带双击：打开底部设备链（dock）并选中该通道（通道条 / 音频条共用）。
/// 同步 `dock_track` 防止下一帧被「跟随选中轨」覆盖；总线/主输出没有 dock
/// 语境，不调用本函数。
fn header_double_click_opens_dock(
    app: &mut App,
    ui: &egui::Ui,
    idx: usize,
    rect: egui::Rect,
    context: crate::chrome::dock_bar::DockContext,
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

/// 设备链：乐器（0/1 个）+ 效果器列表 + 「+」添加入口，按整行滚动。
#[allow(clippy::too_many_arguments)] // 设备链渲染上下文透传
fn device_chain(
    ui: &mut egui::Ui,
    target: InsertTarget,
    view: &InsertView<'_>,
    channel: Option<u8>,
    instruments: &[String],
    max_h: f32,
    actions: &mut Vec<MixAction>,
) {
    // 行距 0（行间靠底部 1px 分隔线分隔），整行取高。
    let step = DEVICE_ROW_H;
    let max_rows = (max_h / step).floor().max(1.0) as usize;
    crate::widgets::scroll::rows_scroll(
        ui,
        ("mix_device_chain", target),
        DEVICE_ROW_H,
        Some(max_rows),
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // 乐器段（仅 MIDI 通道）：空链显示内置 XSynth，否则逐个列出可叠加
            // 的插件乐器。
            if let Some(ch) = channel {
                if instruments.is_empty() {
                    xsynth_row(ui, ch, actions);
                } else {
                    for (i, name) in instruments.iter().enumerate() {
                        instrument_row(ui, ch, i, name, actions);
                    }
                }
            }
            // 效果器段（四种条通用）。
            for (slot, r) in view.refs.iter().enumerate() {
                effect_row(
                    ui,
                    target,
                    slot,
                    &r.name,
                    r.bypassed,
                    view.gui_open(slot),
                    actions,
                );
            }
            // 统一的「+」入口（原乐器/效果器两个加号合并）：MIDI 通道打开
            // 统一选择器（初始分类乐器，可切到效果器）；其他条初始分类效果器。
            if add_row(ui, t!("hint.mix.add_insert")) {
                match channel {
                    Some(ch) => actions.push(MixAction::OpenInstrumentPicker { channel: ch }),
                    None => actions.push(MixAction::OpenPicker { target }),
                }
            }
        },
    );
}

/// 设备链行框架：**整宽贴边**（突破内容区左右内缩，铺满通道条宽度）、直角，
/// 返回行矩形与点击响应（布局高度照常推进）。
fn device_row_frame(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
    let rect = egui::Rect::from_min_size(
        egui::pos2(ui.max_rect().min.x - PAD_X, ui.cursor().min.y),
        egui::vec2(STRIP_WIDTH, DEVICE_ROW_H),
    );
    let resp = ui.interact(rect, ui.next_auto_id(), egui::Sense::click());
    ui.allocate_space(egui::vec2(0.0, DEVICE_ROW_H));
    (rect, resp)
}

/// 设备行的统一外观（背景 + 底部细分隔线 + 左对齐名称）。
fn device_row_visual(ui: &mut egui::Ui, label: &str) -> (egui::Rect, egui::Response) {
    let (rect, resp) = device_row_frame(ui);
    let painter = ui.painter();
    let bg = if resp.hovered() {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        crate::theme::app_bg()
    };
    painter.rect_filled(rect, 0.0, bg);
    // 行间仅底部 1px 分隔线（去掉整圈边框，接近色带式的连续列表）。
    painter.hline(
        rect.x_range(),
        rect.max.y - 0.5,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
    );
    // 名称左侧内边距；右侧留出「更换乐器」按钮的悬停区。
    let name_clip = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - 20.0, rect.max.y));
    painter.with_clip_rect(name_clip).text(
        egui::pos2(rect.min.x + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        crate::theme::text_primary(),
    );
    (rect, resp)
}

/// 乐器行右侧的「更换乐器」按钮（悬停出现）：打开统一选择器（乐器语境）。
/// 行的其余部分仍执行原点击行为（打开乐器界面/配置）。
fn instrument_swap_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    channel: u8,
    row_salt: u64,
    actions: &mut Vec<MixAction>,
) {
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let swap_rect = egui::Rect::from_center_size(
        egui::pos2(rect.max.x - 12.0, rect.center().y),
        egui::vec2(18.0, DEVICE_ROW_H),
    );
    let resp = ui.interact(
        swap_rect,
        ui.scope_id().with(("mix_swap_instr", channel, row_salt)),
        egui::Sense::click(),
    );
    let icon = egui_material_icons::icons::ICON_SWAP_HORIZ;
    let color = if resp.hovered() {
        crate::theme::accent_active()
    } else {
        crate::theme::text_muted()
    };
    ui.painter().text(
        swap_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon.codepoint,
        egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
        color,
    );
    let clicked = resp.clicked();
    resp.on_hover_text(t!("hint.mix.swap_instrument"));
    if clicked {
        actions.push(MixAction::OpenInstrumentPicker { channel });
    }
}

/// 插件乐器设备行（`index` = 该通道链内序号）：点击开关插件界面，右键菜单。
fn instrument_row(
    ui: &mut egui::Ui,
    channel: u8,
    index: usize,
    name: &str,
    actions: &mut Vec<MixAction>,
) {
    let (rect, resp) = device_row_visual(ui, name);
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.toggle_gui"));
    if resp.clicked() {
        actions.push(MixAction::ToggleInstrumentGui { channel, index });
    }
    instrument_swap_button(ui, rect, channel, index as u64, actions);
    resp.context_menu(|ui| {
        ui.set_min_width(96.0);
        ui.set_max_width(96.0);
        let gui_item = ui.add(crate::widgets::menu::menu_item_button(
            ui,
            false,
            t!("mix.toggle_gui").as_ref(),
        ));
        crate::widgets::hint::hover(ui.ctx(), &gui_item, t!("hint.mix.toggle_gui"));
        if gui_item.clicked() {
            actions.push(MixAction::ToggleInstrumentGui { channel, index });
            ui.close();
        }
        let params_item = ui.add(crate::widgets::menu::menu_item_button(
            ui,
            false,
            t!("mix.params").as_ref(),
        ));
        crate::widgets::hint::hover(ui.ctx(), &params_item, t!("hint.mix.instrument_params"));
        if params_item.clicked() {
            actions.push(MixAction::OpenInstrumentParams { channel, index });
            ui.close();
        }
        let remove_item = ui.add(crate::widgets::menu::menu_item_button(
            ui,
            false,
            t!("mix.remove_insert").as_ref(),
        ));
        if remove_item.clicked() {
            actions.push(MixAction::RemoveInstrument { channel, index });
            ui.close();
        }
    });
}

/// 内置 XSynth 设备行（该通道未挂任何插件乐器时显示）。
fn xsynth_row(ui: &mut egui::Ui, channel: u8, actions: &mut Vec<MixAction>) {
    let (rect, resp) = device_row_visual(ui, "XSynth");
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.instrument_xsynth"));
    if resp.clicked() {
        actions.push(MixAction::OpenXsynthConfig { channel });
    }
    instrument_swap_button(ui, rect, channel, 0, actions);
    resp.context_menu(|ui| {
        ui.set_min_width(96.0);
        ui.set_max_width(96.0);
        let cfg_item = ui.add(crate::widgets::menu::menu_item_button(
            ui,
            false,
            t!("mix.toggle_gui").as_ref(),
        ));
        crate::widgets::hint::hover(ui.ctx(), &cfg_item, t!("hint.mix.instrument_xsynth"));
        if cfg_item.clicked() {
            actions.push(MixAction::OpenXsynthConfig { channel });
            ui.close();
        }
        let params_item = ui.add(crate::widgets::menu::menu_item_button(
            ui,
            false,
            t!("mix.params").as_ref(),
        ));
        crate::widgets::hint::hover(ui.ctx(), &params_item, t!("hint.mix.instrument_params"));
        if params_item.clicked() {
            actions.push(MixAction::OpenXsynthConfig { channel });
            ui.close();
        }
    });
}

/// 效果器设备行：状态点（点击旁通）+ 名称（点击开关界面）+ 参数图标；右键菜单。
fn effect_row(
    ui: &mut egui::Ui,
    target: InsertTarget,
    slot: usize,
    name: &str,
    is_bypassed: bool,
    is_open: bool,
    actions: &mut Vec<MixAction>,
) {
    let (rect, resp) = device_row_frame(ui);
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, crate::theme::app_bg());
    if resp.hovered() {
        painter.rect_filled(rect, 0.0, crate::theme::hover_color(crate::theme::app_bg()));
    }
    // 行间仅底部 1px 分隔线（去掉整圈边框，接近色带式的连续列表）。
    painter.hline(
        rect.x_range(),
        rect.max.y - 0.5,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
    );

    // 状态点（点击旁通）：正常 = 强调/次文字色；旁通 = 灰。
    let dot_center = egui::pos2(rect.min.x + 8.0, rect.center().y);
    let dot_color = if is_bypassed {
        crate::theme::text_disabled()
    } else if is_open {
        crate::theme::accent_active()
    } else {
        crate::theme::text_secondary()
    };
    painter.circle_filled(dot_center, 3.0, dot_color);
    let dot_hit = egui::Rect::from_center_size(dot_center, egui::vec2(15.0, DEVICE_ROW_H));
    let dot_resp = ui.interact(
        dot_hit,
        ui.scope_id().with(("mix_dev_bypass", target, slot)),
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

    // 名称（截断，hover 全名；右侧给参数图标留位）。
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(dot_hit.max.x + 2.0, rect.min.y),
        egui::pos2(rect.max.x - 17.0, rect.max.y),
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

    // 参数图标：打开通用参数面板。
    let tune = egui_material_icons::icons::ICON_TUNE;
    let btn_rect = egui::Rect::from_min_max(
        egui::pos2(rect.max.x - 16.0, rect.min.y + 2.0),
        egui::pos2(rect.max.x - 1.0, rect.max.y - 2.0),
    );
    let params_resp = ui.interact(
        btn_rect,
        ui.scope_id().with(("mix_dev_params", target, slot)),
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

/// 「+」设备添加入口：返回是否被点击（调用方决定打开哪个选择器）。
fn add_row(ui: &mut egui::Ui, hint: impl Into<String>) -> bool {
    let (rect, resp) = device_row_frame(ui);
    let painter = ui.painter();
    let bg = if resp.hovered() {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        crate::theme::app_bg()
    };
    painter.rect_filled(rect, 0.0, bg);
    // 行间仅底部 1px 分隔线（去掉整圈边框，接近色带式的连续列表）。
    painter.hline(
        rect.x_range(),
        rect.max.y - 0.5,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_material_icons::icons::ICON_ADD.codepoint,
        egui::FontId::new(
            crate::theme::ICON_FONT_SM,
            egui_material_icons::icons::ICON_ADD.font_family(),
        ),
        crate::theme::text_muted(),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, hint);
    resp.clicked()
}

/// 发送源：MIDI 通道 / 音频通道（`sends`/`audio_sends` 表按通道索引）
/// 或总线（`bus_sends` 表按总线索引）。
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
        // 同一 id_salt：全部通道条共享滚动偏移，滚动即整排同步。
        .id_salt("mix_send_scroll")
        // 滚动条控件 id 派生自 id_salt；多个 ScrollArea 共享同一 id_salt 时，
        // 同屏出现的滚动条会互相触发 egui 的 widget ID 冲突。send 区只有
        // 一行高，隐藏滚动条并保留滚轮同步滚动（hover 任一通道条滚动即可）。
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysHidden)
        // egui 默认 min_scrolled_size=64 会把一行高的发送区撑到 64px，
        // 这里放开下限，让发送区严格保持 SEND_ROW_H（一行高）。
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

/// M/S 按钮（居中，等宽）。
fn ms_row(ui: &mut egui::Ui, params: &StripParams, mut on_change: impl FnMut(StripParams)) {
    let group = BTN * 2.0 + 4.0;
    ui.allocate_ui_with_layout(
        egui::vec2(group, MS_ROW_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
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
        },
    );
}

/// 声像旋钮（居中）。
fn pan_knob(ui: &mut egui::Ui, pan: f32, mut on_pan: impl FnMut(f32)) {
    ui.allocate_ui_with_layout(
        egui::vec2(KNOB_D, PAN_ROW_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let mut norm = (pan + 1.0) / 2.0;
            let resp = knob(ui, &mut norm, KNOB_D);
            if resp.changed() {
                on_pan(norm * 2.0 - 1.0);
            }
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.pan"));
        },
    );
}

/// 当前增益 dB（居中，推子/电平表之上）：可点击**直接输入**、也可上下拖动。
/// 返回 `Some(新增益)` 表示本次被改动。
fn db_row(ui: &mut egui::Ui, gain: f32) -> Option<f32> {
    section(
        ui,
        DB_ROW_H,
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let mut db = gain_to_db(gain);
            let resp = ui.add_sized(
                [CONTENT_W, DB_ROW_H],
                egui::DragValue::new(&mut db)
                    .speed(0.2)
                    .range(DB_MIN..=DB_MAX)
                    .max_decimals(1)
                    .suffix(" dB"),
            );
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.fader"));
            if resp.changed() {
                Some(db_to_gain(db))
            } else {
                None
            }
        },
    )
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

/// 电平表（左）+ 推子（右），等高，整体居中。
fn meter_fader(
    ui: &mut egui::Ui,
    gain: f32,
    level: (f32, f32),
    hold: (f32, f32),
    height: f32,
    mut on_gain: impl FnMut(f32),
) {
    let group = (SCALE_W + METER_W * 2.0 + METER_GAP) + 6.0 + FADER_HANDLE_W;
    ui.allocate_ui_with_layout(
        egui::vec2(group, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            crate::widgets::meter::level_meter(ui, level, hold, height);
            ui.add_space(6.0);
            fader(ui, gain, height, &mut on_gain);
        },
    );
}

/// 自绘推子：凹槽 + 填充 + 0dB 刻度 + 手柄（拖动改值，双击回 0dB）。直角。
fn fader(ui: &mut egui::Ui, gain: f32, height: f32, mut on_gain: impl FnMut(f32)) {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(FADER_HANDLE_W, height),
        egui::Sense::click_and_drag(),
    );
    let y = fader_y(gain, rect.min.y, rect.max.y);
    let painter = ui.painter();

    // 凹槽。
    let slot = egui::Rect::from_center_size(rect.center(), egui::vec2(FADER_SLOT_W, rect.height()));
    painter.rect_filled(slot, 0.0, crate::theme::track_bg());
    // 已填充部分（底部 → 手柄）。
    let fill = egui::Rect::from_min_max(egui::pos2(slot.min.x, y), slot.max);
    painter.rect_filled(fill, 0.0, crate::theme::accent_active());
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
    painter.rect_filled(handle, 2.0, handle_bg);
    painter.rect_stroke(
        handle,
        2.0,
        egui::Stroke::new(1.0, crate::theme::line_fg()),
        egui::StrokeKind::Inside,
    );
    painter.hline(
        handle.x_range(),
        y,
        egui::Stroke::new(1.0, crate::theme::text_secondary()),
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
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.min.x + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        vendor,
        egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT),
        crate::theme::text_label(),
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
    let info_w = 190.0;
    let list_w = (ui.available_width() - info_w - 8.0).max(180.0);
    ui.allocate_ui_with_layout(
        egui::vec2(list_w, ui.available_height()),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            // 搜索行：自绘输入框 + 星标筛选开关（点亮 = 只看收藏）。
            ui.horizontal(|ui| {
                let star_w = 26.0;
                let input_w = (ui.available_width() - star_w - 6.0).max(80.0);
                let search_resp = crate::widgets::text_input::control_text_input(
                    ui,
                    &mut app.mix.picker_filter,
                    input_w,
                    "mix_picker_search",
                    Some(&t!("mix.search")),
                );
                search_resp.on_hover_text(t!("hint.mix.picker_search"));
                let fav_only = app.mix.picker_fav_only;
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
                    app.mix.picker_fav_only = !fav_only;
                }
            });
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
                    if tab.shows_xsynth()
                        && !fav_only
                        && (filter.is_empty() || "xsynth".contains(&filter))
                    {
                        any = true;
                        let selected = app.mix.picker_selected.as_deref() == Some(XS_SELECTED_ID);
                        let resp = plugin_row(
                            ui,
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
                            .filter(|p| {
                                filter.is_empty() || p.name.to_lowercase().contains(&filter)
                            })
                            .collect();
                        list.sort_by(|a, b| {
                            a.vendor.cmp(&b.vendor).then_with(|| a.name.cmp(&b.name))
                        });
                        let mut last_vendor: Option<&str> = None;
                        for p in list {
                            if !p.vendor.is_empty() && last_vendor != Some(p.vendor.as_str()) {
                                vendor_header(ui, &p.vendor);
                                last_vendor = Some(p.vendor.as_str());
                            }
                            any = true;
                            let enabled = p.error.is_none() && kind.can_pick(p);
                            let selected =
                                app.mix.picker_selected.as_deref() == Some(p.id.as_str());
                            let resp =
                                plugin_row(ui, &p.display_name(), p.format, enabled, selected);
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
        },
    );
}

/// 右侧信息面板：选中插件的名称/厂商/格式/路径 + 收藏 star 开关。
fn picker_info(app: &mut App, ui: &mut egui::Ui) {
    let info_w = 190.0;
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
                            // 主体（左栏 + 列表 + 信息）占满按钮行以上。
                            let btn_zone = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            ui.allocate_ui_with_layout(
                                egui::vec2(
                                    ui.available_width(),
                                    (ui.available_height() - btn_zone).max(0.0),
                                ),
                                egui::Layout::left_to_right(egui::Align::TOP),
                                |ui| {
                                    picker_tab_bar(app, ui);
                                    // 左栏贴边后，列表与左栏之间的间距。
                                    ui.add_space(12.0);
                                    picker_list(app, ui, kind, actions, &mut close);
                                    picker_info(app, ui);
                                },
                            );
                            // 底部按钮贴底（bottom_up）：下边距与右边距一致。
                            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                                let ok_text = t!("mix.picker_ok");
                                let cancel_text = t!("mix.picker_cancel");
                                let ok_enabled = app.mix.picker_selected.is_some();
                                if let Some(idx) = crate::chrome::dialog_buttons::dialog_button_row(
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
                                ) {
                                    match idx {
                                        0 => close = true,
                                        1 => {
                                            pick_selected(app, kind, actions);
                                            close = true;
                                        }
                                        _ => {}
                                    }
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

/// 插件选择器的一行：左名称（超宽裁剪）+ 右格式标签（CLAP / VST3）。
///
/// `enabled = false`（加载失败/语境不支持）时灰色且不响应点击；
/// `selected` 时整行选中底色；hover 提示由调用方追加。
fn plugin_row(
    ui: &mut egui::Ui,
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

    /// 回归：同一容器内多行设备行的交互 ID 必须互不相同。
    ///
    /// `device_row_frame` 交互 ID 取自 `ui.next_auto_id()`（只预览、不推进），
    /// 依赖随后的 `ui.allocate_space` 推进计数器——两者任一被改动/移除都会
    /// 让整列设备行共享同一 ID（debug 构建下满屏 ID 冲突警告）。
    #[test]
    fn device_rows_have_unique_interaction_ids() {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(Default::default(), |ui| {
            let (r1, resp1) = device_row_frame(ui);
            let (r2, resp2) = device_row_frame(ui);
            assert!(r1.min.y < r2.min.y, "两行应纵向排布");
            assert_ne!(resp1.id, resp2.id, "两行交互 ID 必须不同");
        });
        // 同时扫描本帧绘制文本：不应出现 egui 的 ID 冲突警告。
        let mut clash_text = None;
        for cs in &output.shapes {
            if let egui::Shape::Text(t) = &cs.shape
                && t.galley.text().contains("of widget ID")
            {
                clash_text = Some(t.galley.text().to_owned());
            }
        }
        output.drop_without_applying_deltas();
        assert!(clash_text.is_none(), "触发 ID 冲突警告: {clash_text:?}");
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
