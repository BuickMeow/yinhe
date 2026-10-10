//! 混音台通道条控件。
//!
//! 每个通道 = 一个**直角矩形 + 单层描边**（通道框不用圆角，避免卡片出界）。按钮类
//! 控件用通用圆角矩形。自上而下：
//! 1. 顶部**色带**（轨道色）+ 通道号；
//! 2. **设备链**（若干乐器 / 效果器，固定高度、整行滚动，不撑破通道）；
//! 3. **效果发送**（FX 1 旋钮；无总线时首次拖动自动创建）；
//! 4. **M/S**（居中）；
//! 5. **声像旋钮**（居中）；
//! 6. **当前增益 dB 读数** + **左侧电平表（带刻度 + 峰值保持）+ 右侧推子**。
//!
//! 各分区高度固定，因此所有通道条的推子/电平表**等高**。

use eframe::egui;
use rust_i18n::t;
use yinhe_audio::InsertTarget;
use yinhe_mixer::{InsertRef, MasterParams, MixerParams, StripParams};

use crate::app::App;
use crate::widgets::knob::knob;

use super::plugin_instance::PluginEntry;
use super::rack::SlotRuntime;
use super::{MixAction, channel_label, db_to_gain, gain_to_db};

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
const DB_ROW_H: f32 = 15.0;
/// 「完整条」（通道 / 音频 / 总线）底部固定区高度。
const BOTTOM_FULL: f32 = SEND_ROW_H + MS_ROW_H + PAN_ROW_H + DB_ROW_H + GAP * 4.0;
/// 主输出条底部固定区高度（只有 dB 读数）。
const BOTTOM_MASTER: f32 = DB_ROW_H + GAP;
/// 推子槽宽（px）。
const FADER_SLOT_W: f32 = 4.0;
/// 推子手柄尺寸（px）。
const FADER_HANDLE_W: f32 = 22.0;
const FADER_HANDLE_H: f32 = 11.0;
/// 电平表单条宽 + 条间距（px）。
const METER_W: f32 = 6.0;
const METER_GAP: f32 = 2.0;
/// 电平表左侧 dB 数字刻度栏宽（px）。
const SCALE_W: f32 = 20.0;
/// dB 读数垂直拖动灵敏度（dB / px）。
const DB_PER_PX: f32 = 0.4;
/// 推子/电平表 dB 范围。
const DB_MIN: f32 = -60.0;
const DB_MAX: f32 = 6.0;
/// 推子最小高度（px）。
const FADER_MIN_H: f32 = 60.0;

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

/// 顶部工具条：扫描插件 + 添加总线 + 状态信息。
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

/// 通道条外框：直角矩形 + 单层描边 + 顶部色带（写标题）。
/// `add_contents` 在色带下方的内容区排布，参数为其可用高度。
fn strip_frame(
    ui: &mut egui::Ui,
    color: egui::Color32,
    title: &str,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui, f32),
) {
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
        crate::theme::contrast_fg(),
    );

    // 单层描边确定一个通道。
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, crate::theme::line_fg()),
        egui::StrokeKind::Inside,
    );

    let content = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + PAD_X, header.max.y + 4.0),
        egui::pos2(rect.max.x - PAD_X, rect.max.y - 6.0),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
        |ui| {
            ui.set_clip_rect(content);
            ui.set_min_width(content.width());
            ui.set_max_width(content.width());
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            add_contents(ui, content.height());
        },
    );
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
    let has_bus = app.workspace.documents[idx].mixer.bus_count() > 0;
    let send = app.workspace.documents[idx]
        .mixer
        .sends
        .get(channel as usize)
        .and_then(|list| list.first())
        .map(|s| (s.amount, s.pre_fader))
        .unwrap_or((0.0, false));
    let instrument = app.workspace.documents[idx]
        .mixer
        .instruments
        .get(channel as usize)
        .and_then(|o| o.as_ref())
        .map(|r| r.name.clone());
    let title = channel_label(channel);

    strip_frame(ui, color, &title, height, |ui, content_h| {
        let (dev_h, meter_h) = strip_metrics(content_h, true);
        section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
            device_chain(
                ui,
                target,
                &view,
                Some((channel, instrument.as_deref())),
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
                send_block(ui, channel, send, has_bus, actions);
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
        let bar_h = (meter_h - DB_ROW_H).max(FADER_MIN_H);
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

    strip_frame(
        ui,
        crate::theme::accent_active(),
        &title,
        height,
        |ui, content_h| {
            let (dev_h, meter_h) = strip_metrics(content_h, true);
            section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
                device_chain(ui, target, &view, None, dev_h, actions);
            });
            ui.add_space(GAP);
            section(
                ui,
                SEND_ROW_H,
                egui::Layout::top_down(egui::Align::LEFT),
                |_ui| {},
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
            let bar_h = (meter_h - DB_ROW_H).max(FADER_MIN_H);
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

    strip_frame(
        ui,
        crate::theme::accent_active(),
        &title,
        height,
        |ui, content_h| {
            let (dev_h, meter_h) = strip_metrics(content_h, true);
            section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let del_resp = icon_button(
                            ui,
                            egui_material_icons::icons::ICON_DELETE.codepoint,
                            crate::theme::danger_text(),
                        );
                        crate::widgets::hint::hover(ui.ctx(), &del_resp, t!("hint.mix.remove_bus"));
                        if del_resp.clicked() {
                            actions.push(MixAction::RemoveBus { bus });
                        }
                    });
                });
                device_chain(ui, target, &view, None, dev_h - BTN, actions);
            });
            ui.add_space(GAP);
            section(
                ui,
                SEND_ROW_H,
                egui::Layout::top_down(egui::Align::LEFT),
                |_ui| {},
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
            let bar_h = (meter_h - DB_ROW_H).max(FADER_MIN_H);
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
            let (dev_h, meter_h) = strip_metrics(content_h, false);
            section(ui, dev_h, egui::Layout::top_down(egui::Align::LEFT), |ui| {
                device_chain(ui, InsertTarget::Master, &view, None, dev_h, actions);
            });
            ui.add_space(GAP);
            if let Some(g) = db_row(ui, params.gain) {
                actions.push(MixAction::SetMaster {
                    params: MasterParams { gain: g },
                });
            }
            let bar_h = (meter_h - DB_ROW_H).max(FADER_MIN_H);
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

/// 设备区与推子/电平表区高度：`full` = 预留发送/M-S/声像（通道/音频/总线），
/// 保证这些条等高；主输出只预留 dB 读数。
fn strip_metrics(content_h: f32, full: bool) -> (f32, f32) {
    let reserved = if full { BOTTOM_FULL } else { BOTTOM_MASTER };
    // 设备区最多占 35%（避免把额外高度全吃掉、把推子/电平表压到最小）。
    let max_dev = (content_h * 0.35).max(DEVICE_ROW_H);
    let dev_h = (content_h - reserved - FADER_MIN_H).clamp(DEVICE_ROW_H, max_dev);
    let meter_h = (content_h - reserved - dev_h).max(FADER_MIN_H);
    (dev_h, meter_h)
}

/// 设备链：乐器（0/1 个）+ 效果器列表 + 「+」添加入口，按整行滚动。
fn device_chain(
    ui: &mut egui::Ui,
    target: InsertTarget,
    view: &InsertView<'_>,
    instrument: Option<(u8, Option<&str>)>,
    max_h: f32,
    actions: &mut Vec<MixAction>,
) {
    let step = DEVICE_ROW_H + 1.0;
    let max_rows = (max_h / step).floor().max(1.0) as usize;
    crate::widgets::scroll::rows_scroll(
        ui,
        ("mix_device_chain", target),
        DEVICE_ROW_H,
        Some(max_rows),
        |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if let Some((channel, name)) = instrument {
                instrument_row(ui, channel, name, actions);
            }
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
            add_row(ui, target, actions);
        },
    );
}

/// 乐器设备行：名称（点击开插件界面 / XSynth 音色库），右键菜单。
fn instrument_row(
    ui: &mut egui::Ui,
    channel: u8,
    name: Option<&str>,
    actions: &mut Vec<MixAction>,
) {
    let label = name.unwrap_or("XSynth");
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), DEVICE_ROW_H),
        egui::Sense::click(),
    );
    let painter = ui.painter();
    let bg = if resp.hovered() {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        crate::theme::app_bg()
    };
    painter.rect_filled(rect, RADIUS, bg);
    painter.rect_stroke(
        rect,
        RADIUS,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        egui::pos2(rect.min.x + 5.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        crate::theme::text_primary(),
    );
    crate::widgets::hint::hover(
        ui.ctx(),
        &resp,
        if name.is_some() {
            t!("hint.mix.toggle_gui")
        } else {
            t!("hint.mix.instrument_xsynth")
        },
    );
    if resp.clicked() {
        if name.is_some() {
            actions.push(MixAction::ToggleInstrumentGui { channel });
        } else {
            actions.push(MixAction::OpenXsynthConfig { channel });
        }
    }
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
            if name.is_some() {
                actions.push(MixAction::ToggleInstrumentGui { channel });
            } else {
                actions.push(MixAction::OpenXsynthConfig { channel });
            }
            ui.close();
        }
        let params_item = ui.add(crate::widgets::menu::menu_item_button(
            ui,
            false,
            t!("mix.params").as_ref(),
        ));
        crate::widgets::hint::hover(ui.ctx(), &params_item, t!("hint.mix.instrument_params"));
        if params_item.clicked() {
            if name.is_some() {
                actions.push(MixAction::OpenInstrumentParams { channel });
            } else {
                actions.push(MixAction::OpenXsynthConfig { channel });
            }
            ui.close();
        }
        let change_item = ui.add(crate::widgets::menu::menu_item_button(
            ui,
            false,
            t!("mix.change_instrument").as_ref(),
        ));
        crate::widgets::hint::hover(ui.ctx(), &change_item, t!("hint.mix.change_instrument"));
        if change_item.clicked() {
            actions.push(MixAction::OpenInstrumentPicker { channel });
            ui.close();
        }
        if name.is_some() {
            let remove_item = ui.add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("mix.remove_insert").as_ref(),
            ));
            if remove_item.clicked() {
                actions.push(MixAction::RemoveInstrument { channel });
                ui.close();
            }
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
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), DEVICE_ROW_H),
        egui::Sense::click(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, RADIUS, crate::theme::app_bg());
    if resp.hovered() {
        painter.rect_filled(
            rect,
            RADIUS,
            crate::theme::hover_color(crate::theme::app_bg()),
        );
    }
    painter.rect_stroke(
        rect,
        RADIUS,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
        egui::StrokeKind::Inside,
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
        ui.id().with(("mix_dev_bypass", target, slot)),
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
        ui.id().with(("mix_dev_params", target, slot)),
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

/// 「+」添加入口：点击打开效果器选择器。
fn add_row(ui: &mut egui::Ui, target: InsertTarget, actions: &mut Vec<MixAction>) {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), DEVICE_ROW_H),
        egui::Sense::click(),
    );
    let painter = ui.painter();
    let bg = if resp.hovered() {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        crate::theme::app_bg()
    };
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
        egui_material_icons::icons::ICON_ADD.codepoint,
        egui::FontId::new(
            crate::theme::ICON_FONT_SM,
            egui_material_icons::icons::ICON_ADD.font_family(),
        ),
        crate::theme::text_muted(),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.add_insert"));
    if resp.clicked() {
        actions.push(MixAction::OpenPicker { target });
    }
}

/// 效果发送区（单行）：FX 1 旋钮。无总线时首次拖动自动创建一条总线。
/// 右键打开完整发送面板（含推子前/后、多总线）。
fn send_block(
    ui: &mut egui::Ui,
    channel: u8,
    send: (f32, bool),
    has_bus: bool,
    actions: &mut Vec<MixAction>,
) {
    let (amount, pre_fader) = send;
    let group = KNOB_D + 4.0 + 26.0;
    ui.allocate_ui_with_layout(
        egui::vec2(group, SEND_ROW_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let mut norm = (amount / 2.0).clamp(0.0, 1.0);
            let resp = knob(ui, &mut norm, KNOB_D);
            if resp.changed() {
                if !has_bus {
                    actions.push(MixAction::AddBus);
                }
                actions.push(MixAction::SetSend {
                    channel,
                    bus: 0,
                    amount: norm * 2.0,
                    pre_fader,
                });
            }
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.send_amount"));
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
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("FX 1")
                    .size(crate::theme::SMALL_LABEL_FONT)
                    .color(crate::theme::text_secondary()),
            );
        },
    );
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

/// 当前增益 dB 读数（居中，推子/电平表之上）。可垂直拖动改增益，双击回 0 dB。
/// 返回 `Some(新增益)` 表示本次被改动。
fn db_row(ui: &mut egui::Ui, gain: f32) -> Option<f32> {
    section(
        ui,
        DB_ROW_H,
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let (rect, mut resp) = ui.allocate_exact_size(
                egui::vec2(CONTENT_W, DB_ROW_H),
                egui::Sense::click_and_drag(),
            );
            let id = resp.id;
            let mut new_gain = None;
            // 起始值 + 指针位移重算，避免 `drag_delta` 累计语义回弹。
            if resp.drag_started()
                && let Some(y) = resp.interact_pointer_pos().map(|p| p.y)
            {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        id,
                        DbDrag {
                            start_gain: gain,
                            start_y: y,
                        },
                    )
                });
            }
            if resp.dragged()
                && let Some(y) = resp.interact_pointer_pos().map(|p| p.y)
                && let Some(s) = ui.ctx().data(|d| d.get_temp::<DbDrag>(id))
            {
                // 向上拖 = 增大。
                let db = gain_to_db(s.start_gain) + (s.start_y - y) * DB_PER_PX;
                new_gain = Some(db_to_gain(db));
                resp.mark_changed();
            }
            if resp.drag_stopped() {
                ui.ctx().data_mut(|d| d.remove::<DbDrag>(id));
            }
            if resp.double_clicked() {
                new_gain = Some(1.0);
            }
            let text = if gain <= 0.0001 {
                "-∞ dB".to_string()
            } else {
                format!("{:+.1} dB", gain_to_db(gain))
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(crate::theme::SMALL_FONT),
                crate::theme::text_secondary(),
            );
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.mix.fader"));
            resp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
            new_gain
        },
    )
}

/// dB 读数拖动会话（存 `ctx.data` temp 槽，按 widget Id 隔离）。
#[derive(Clone, Copy)]
struct DbDrag {
    start_gain: f32,
    start_y: f32,
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

/// 方形图标按钮（删除等）；圆角矩形。
fn icon_button(ui: &mut egui::Ui, codepoint: &str, color: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(BTN, BTN), egui::Sense::click());
    let bg = if resp.hovered() {
        crate::theme::hover_color(crate::theme::btn_bg())
    } else {
        crate::theme::btn_bg()
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
        codepoint,
        egui::FontId::new(
            crate::theme::ICON_FONT_SM,
            egui_material_icons::icons::ICON_DELETE.font_family(),
        ),
        color,
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
            meter(ui, level, hold, height);
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

/// 电平表刻度：`(dB, 是否标注数字)`。行业惯用标注点。
const METER_TICKS: &[(f32, bool)] = &[
    (0.0, true),
    (-6.0, true),
    (-12.0, true),
    (-18.0, false),
    (-24.0, true),
    (-36.0, false),
    (-48.0, true),
];

/// 自绘电平表：左侧 dB 数字刻度栏 + L/R 双条（直角、加宽）+ 刻度 + 峰值保持线。
/// dB 映射 -60..+6；>0dB 金色、≥0dBFS 红色顶格。
fn meter(ui: &mut egui::Ui, level: (f32, f32), hold: (f32, f32), height: f32) {
    let bars_w = METER_W * 2.0 + METER_GAP;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(SCALE_W + bars_w, height), egui::Sense::hover());
    let painter = ui.painter();
    let bars = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + SCALE_W, rect.min.y),
        egui::vec2(bars_w, height),
    );
    painter.rect_filled(bars, 0.0, crate::theme::track_bg());

    // 刻度线 + 数字标注。
    let font = egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT);
    for &(db, labeled) in METER_TICKS {
        let y = bars.max.y - db_frac(db) * bars.height();
        painter.hline(
            bars.x_range(),
            y,
            egui::Stroke::new(
                1.0,
                crate::theme::text_muted().gamma_multiply(if labeled { 0.7 } else { 0.35 }),
            ),
        );
        if labeled {
            painter.text(
                egui::pos2(bars.min.x - 3.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{}", db as i32),
                font.clone(),
                crate::theme::text_secondary(),
            );
        }
    }

    // L/R 填充条。
    for (i, &p) in [level.0, level.1].iter().enumerate() {
        let x0 = bars.min.x + i as f32 * (METER_W + METER_GAP);
        let bar = egui::Rect::from_min_size(
            egui::pos2(x0, bars.min.y),
            egui::vec2(METER_W, bars.height()),
        );
        let frac = db_frac(gain_to_db(p));
        if frac > 0.0 {
            let h = bar.height() * frac;
            let fill = egui::Rect::from_min_max(egui::pos2(bar.min.x, bar.max.y - h), bar.max);
            painter.rect_filled(fill, 0.0, meter_color(p));
        }
    }

    // 峰值保持线（比当前电平高时显示）。
    for (i, (&l, &h)) in [level.0, level.1]
        .iter()
        .zip([hold.0, hold.1].iter())
        .enumerate()
    {
        if h <= l + 1e-4 {
            continue;
        }
        let x0 = bars.min.x + i as f32 * (METER_W + METER_GAP);
        let y = bars.max.y - db_frac(gain_to_db(h)) * bars.height();
        painter.hline(
            egui::Rangef::new(x0, x0 + METER_W),
            y,
            egui::Stroke::new(1.0, meter_color(h)),
        );
    }
}

/// 电平着色：≥0dBFS 红、> -6dB 金、其余强调色。
fn meter_color(p: f32) -> egui::Color32 {
    if p >= 1.0 {
        crate::theme::danger_text()
    } else if gain_to_db(p) > -6.0 {
        crate::theme::warning_gold()
    } else {
        crate::theme::accent_active()
    }
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
                            crate::widgets::rows::inline_row(ui, |ui| {
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
            RADIUS,
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
}
