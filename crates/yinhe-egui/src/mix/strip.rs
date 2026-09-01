//! 混音台通道条控件：标签/insert 槽位/M-S/声像/推子/电平表。

use eframe::egui;
use rust_i18n::t;
use yinhe_mixer::{MasterParams, StripParams};

use crate::app::App;

use super::{MixAction, channel_label, db_to_gain, gain_to_db};

/// 通道条宽度（px）。
pub(crate) const STRIP_WIDTH: f32 = 96.0;
/// 推子高度（px）。
const FADER_HEIGHT: f32 = 180.0;
/// 电平表单条宽度（px）。
const METER_WIDTH: f32 = 6.0;
/// insert 区高度（px）。
const INSERT_AREA_HEIGHT: f32 = 84.0;

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

/// 单条通道条。`peak` 是 UI 侧衰减后的 (L, R) 峰值。
pub(crate) fn channel_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    channel: u8,
    track_names: &[String],
    peak: (f32, f32),
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

    strip_frame(ui, |ui| {
        // 通道标签 + 轨道名。
        ui.label(
            egui::RichText::new(channel_label(channel))
                .strong()
                .color(crate::theme::text_bright()),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new(&names)
                    .small()
                    .color(crate::theme::text_secondary()),
            )
            .truncate(),
        )
        .on_hover_text(&names);

        insert_area(
            ui,
            Some(channel),
            &insert_names,
            &bypassed,
            &gui_open,
            actions,
        );
        strip_controls(ui, &params, |new_params| {
            actions.push(MixAction::SetStrip {
                channel,
                params: new_params,
            });
        });
        fader_and_meter(ui, params.gain, peak, |gain| {
            let mut p = params;
            p.gain = gain;
            actions.push(MixAction::SetStrip { channel, params: p });
        });
    });
}

/// 主输出条。
pub(crate) fn master_strip(
    app: &mut App,
    ui: &mut egui::Ui,
    idx: usize,
    peak: (f32, f32),
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

    strip_frame(ui, |ui| {
        ui.label(
            egui::RichText::new(t!("mix.master"))
                .strong()
                .color(crate::theme::text_bright()),
        );
        ui.label(
            egui::RichText::new(" ")
                .small()
                .color(crate::theme::text_secondary()),
        );

        insert_area(ui, None, &insert_names, &bypassed, &gui_open, actions);
        // master 无 M/S/声像：占位保持与通道条等高对齐。
        ui.allocate_space(egui::vec2(STRIP_WIDTH - 12.0, 24.0));
        fader_and_meter(ui, params.gain, peak, |gain| {
            actions.push(MixAction::SetMaster {
                params: MasterParams { gain },
            });
        });
    });
}

/// 通道条外框。
fn strip_frame(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(crate::theme::control_bg())
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 6))
        .show(ui, |ui| {
            ui.set_width(STRIP_WIDTH - 12.0);
            ui.vertical_centered(add_contents);
        });
}

/// insert 槽位区：已有槽位（旁通/名称/移除）+ 添加按钮。
#[allow(clippy::too_many_arguments)] // 通道条渲染上下文透传
fn insert_area(
    ui: &mut egui::Ui,
    channel: Option<u8>,
    names: &[String],
    bypassed: &[bool],
    gui_open: &[bool],
    actions: &mut Vec<MixAction>,
) {
    egui::Frame::new()
        .fill(crate::theme::track_bg())
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(3, 3))
        .show(ui, |ui| {
            ui.set_height(INSERT_AREA_HEIGHT);
            ui.set_width(STRIP_WIDTH - 18.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (slot, name) in names.iter().enumerate() {
                    ui.horizontal(|ui| {
                        // 旁通开关（⏻）：旁通中显示为高亮。
                        let is_bypassed = bypassed.get(slot).copied().unwrap_or(false);
                        let b = egui::Button::new(egui::RichText::new("⏻").small().color(
                            if is_bypassed {
                                crate::theme::warning_gold()
                            } else {
                                crate::theme::text_muted()
                            },
                        ))
                        .frame_when_inactive(false);
                        if ui.add(b).on_hover_text(t!("mix.bypass")).clicked() {
                            actions.push(MixAction::BypassInsert {
                                channel,
                                slot,
                                bypassed: !is_bypassed,
                            });
                        }
                        let text = egui::RichText::new(name).small().color(if is_bypassed {
                            crate::theme::text_muted()
                        } else {
                            crate::theme::text_primary()
                        });
                        ui.add(egui::Label::new(text).truncate());
                        // 原生界面开关（浮动窗口，插件自管理）。
                        let is_open = gui_open.get(slot).copied().unwrap_or(false);
                        let g = egui::Button::new(egui::RichText::new("UI").small().color(
                            if is_open {
                                crate::theme::contrast_fg()
                            } else {
                                crate::theme::text_muted()
                            },
                        ))
                        .fill(if is_open {
                            crate::theme::accent_active()
                        } else {
                            crate::theme::btn_bg()
                        });
                        if ui.add(g).on_hover_text(t!("mix.toggle_gui")).clicked() {
                            actions.push(MixAction::ToggleGui { channel, slot });
                        }
                        if ui
                            .small_button("✕")
                            .on_hover_text(t!("mix.remove_insert"))
                            .clicked()
                        {
                            actions.push(MixAction::RemoveInsert { channel, slot });
                        }
                    });
                }
                if ui
                    .small_button(t!("mix.add_insert"))
                    .on_hover_text(t!("mix.add_insert_hint"))
                    .clicked()
                {
                    actions.push(MixAction::OpenPicker { channel });
                }
            });
        });
}

/// M/S 按钮 + 声像滑杆。
fn strip_controls(ui: &mut egui::Ui, params: &StripParams, mut on_change: impl FnMut(StripParams)) {
    ui.horizontal(|ui| {
        let m = egui::Button::new(egui::RichText::new("M").small().color(if params.mute {
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_secondary()
        }))
        .fill(if params.mute {
            crate::theme::mute_active()
        } else {
            crate::theme::btn_bg()
        });
        if ui.add(m).on_hover_text(t!("mix.mute")).clicked() {
            let mut p = *params;
            p.mute = !p.mute;
            on_change(p);
        }
        let s = egui::Button::new(egui::RichText::new("S").small().color(if params.solo {
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_secondary()
        }))
        .fill(if params.solo {
            crate::theme::solo_active()
        } else {
            crate::theme::btn_bg()
        });
        if ui.add(s).on_hover_text(t!("mix.solo")).clicked() {
            let mut p = *params;
            p.solo = !p.solo;
            on_change(p);
        }
    });

    // 声像：-1..1，双击回中。
    let mut pan = params.pan;
    let resp = ui.add(
        egui::Slider::new(&mut pan, -1.0..=1.0)
            .show_value(false)
            .trailing_fill(true),
    );
    if resp.double_clicked() {
        pan = 0.0;
    }
    if resp.changed() {
        let mut p = *params;
        p.pan = pan.clamp(-1.0, 1.0);
        on_change(p);
    }
    let pan_text = if params.pan.abs() < 0.01 {
        "C".to_string()
    } else if params.pan < 0.0 {
        format!("L{:.0}", -params.pan * 100.0)
    } else {
        format!("R{:.0}", params.pan * 100.0)
    };
    ui.label(
        egui::RichText::new(pan_text)
            .small()
            .color(crate::theme::text_muted()),
    );
}

/// 推子 + 电平表 + dB 读数。
fn fader_and_meter(ui: &mut egui::Ui, gain: f32, peak: (f32, f32), mut on_gain: impl FnMut(f32)) {
    ui.horizontal(|ui| {
        // 推子：dB 域 -60..+6，双击回 0 dB。
        let mut db = gain_to_db(gain);
        let fader_size = egui::vec2(20.0, FADER_HEIGHT);
        let inner = ui.allocate_ui(fader_size, |ui| {
            ui.centered_and_justified(|ui| {
                ui.add(
                    egui::Slider::new(&mut db, -60.0..=6.0)
                        .vertical()
                        .show_value(false)
                        .trailing_fill(true),
                )
            })
            .inner
        });
        let resp = inner.inner;
        if resp.double_clicked() {
            on_gain(1.0);
        } else if resp.changed() {
            on_gain(db_to_gain(db));
        }

        // 电平表（L/R 双条）。
        let meter_size = egui::vec2(METER_WIDTH * 2.0 + 2.0, FADER_HEIGHT);
        let (rect, _) = ui.allocate_exact_size(meter_size, egui::Sense::hover());
        paint_meter(ui.painter(), rect, peak);
    });
    let db_text = if gain <= 0.0001 {
        "-∞".to_string()
    } else {
        format!("{:+.1}", gain_to_db(gain))
    };
    ui.label(
        egui::RichText::new(db_text)
            .small()
            .color(crate::theme::text_secondary()),
    );
}

/// 画一对电平条（dB 映射：-60..+6 → 0..1；>0dB 金色，≥0dBFS 红色顶格）。
fn paint_meter(painter: &egui::Painter, rect: egui::Rect, peak: (f32, f32)) {
    painter.rect_filled(rect, 2.0, crate::theme::track_bg());
    for (i, &p) in [peak.0, peak.1].iter().enumerate() {
        let x0 = rect.min.x + i as f32 * (METER_WIDTH + 2.0);
        let bar = egui::Rect::from_min_size(
            egui::pos2(x0, rect.min.y),
            egui::vec2(METER_WIDTH, rect.height()),
        );
        let db = gain_to_db(p);
        let frac = ((db + 60.0) / 66.0).clamp(0.0, 1.0);
        if frac <= 0.0 {
            continue;
        }
        let h = bar.height() * frac;
        let fill = egui::Rect::from_min_max(egui::pos2(bar.min.x, bar.max.y - h), bar.max);
        let color = if p >= 1.0 {
            crate::theme::danger_text()
        } else if db > -6.0 {
            crate::theme::warning_gold()
        } else {
            crate::theme::accent_active()
        };
        painter.rect_filled(fill, 1.0, color);
    }
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
    actions: &mut Vec<MixAction>,
) {
    let name = app.workspace.documents[idx]
        .mixer
        .instruments
        .get(channel as usize)
        .and_then(|o| o.as_ref())
        .map(|r| r.name.clone());
    strip_frame(ui, |ui| {
        ui.label(
            egui::RichText::new(format!("{} {}", t!("mix.instrument"), channel + 1))
                .strong()
                .color(crate::theme::text_bright()),
        );
        match &name {
            Some(n) => {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(n)
                            .small()
                            .color(crate::theme::text_secondary()),
                    )
                    .truncate(),
                )
                .on_hover_text(n);
                ui.horizontal(|ui| {
                    if ui.small_button(t!("mix.change_instrument")).clicked() {
                        actions.push(MixAction::OpenInstrumentPicker { channel });
                    }
                    if ui.small_button(t!("mix.remove_insert")).clicked() {
                        actions.push(MixAction::RemoveInstrument { channel });
                    }
                });
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
