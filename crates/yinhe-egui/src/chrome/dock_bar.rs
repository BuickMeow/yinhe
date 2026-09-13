//! 三视图通用底部设备栏（Ableton 风格设备链）。
//!
//! 布局：上方设备链（乐器在最左 + 效果器链 + 「+」添加），下方选中设备的参数区。
//! - 普通 MIDI 轨的乐器位显示 **XSynth 虚拟乐器**，参数区列出 xsynth 支持的
//!   自动化目标（Pitch Bend / RPN 0-2 / 常用 CC），可一键「显示自动化」建 lane；
//! - 乐器插件 / 效果器插件的参数区提供「参数面板」「插件界面」入口。
//!
//! 效果器链当前按**源 MIDI 通道**索引（与 MIX/引擎一致）；同通道多轨共享一条链。
//! 开关/高度持久化到 `LayoutSettings`；设备选中态不持久化。

use eframe::egui;
use rust_i18n::t;
use yinhe_core::TrackKind;
use yinhe_types::AutomationTarget;

use crate::app::App;

/// dock 参数区当前选中的设备。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DockDevice {
    /// 普通 MIDI 轨的 XSynth 虚拟乐器。
    XSynth,
    /// 该轨的乐器插件（乐器通道上的实例）。
    Instrument,
    /// 效果器链槽位（按通道链索引）。
    Insert(usize),
}

/// xsynth 支持且有实际效果的自动化目标（Pitch Bend 到 Coarse Tune + 内建 CC）。
fn xsynth_targets() -> Vec<AutomationTarget> {
    vec![
        AutomationTarget::CC { controller: 7 },
        AutomationTarget::CC { controller: 10 },
        AutomationTarget::CC { controller: 11 },
        AutomationTarget::CC { controller: 64 },
        AutomationTarget::CC { controller: 71 },
        AutomationTarget::CC { controller: 72 },
        AutomationTarget::CC { controller: 73 },
        AutomationTarget::CC { controller: 74 },
        AutomationTarget::PitchBend,
        AutomationTarget::Rpn { parameter: 0 },
        AutomationTarget::Rpn { parameter: 1 },
        AutomationTarget::Rpn { parameter: 2 },
    ]
}

/// 每帧绘制底部设备栏（mode_bar 之后、compute_layout 之前调用）。
pub(crate) fn show(app: &mut App, ui: &mut egui::Ui) {
    if !app.show_bottom_dock {
        return;
    }
    let Some(idx) = app.workspace.active_doc else {
        return;
    };

    let resp = egui::Panel::bottom("bottom_dock")
        .resizable(true)
        .default_size(app.bottom_dock_height)
        .min_size(120.0)
        .max_size((ui.available_height() - 160.0).max(200.0))
        .frame(egui::Frame {
            fill: crate::theme::app_bg(),
            inner_margin: egui::Margin::symmetric(8, 4),
            ..Default::default()
        })
        .show(ui, |ui| show_body(app, idx, ui));

    // 拖动结束：同步高度并持久化。
    if resp.response.drag_stopped() {
        app.bottom_dock_height = resp.response.rect.height().max(120.0);
        app.layout_needs_save = true;
    }
}

/// 设备链 + 参数区。
fn show_body(app: &mut App, idx: usize, ui: &mut egui::Ui) {
    // ── 通道选择（Studio One 风格：设备链按源通道组织）──
    let selected_track = {
        let doc = &app.workspace.documents[idx];
        doc.edit.track_selected.iter().min().copied()
    };
    if selected_track != app.dock_track {
        // 切换选中轨：dock 跟随该轨所在通道，并重置设备选中。
        app.dock_track = selected_track;
        if let Some(ch) = selected_track
            .and_then(|ti| {
                app.workspace.documents[idx]
                    .data
                    .model
                    .tracks
                    .get(ti as usize)
            })
            .map(|t| t.global_channel())
        {
            app.dock_channel = Some(ch);
            app.dock_selected = None;
        }
    }
    let Some(channel) = app.dock_channel else {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(t!("dock.select_channel"))
                    .color(crate::theme::text_muted())
                    .size(crate::theme::SMALL_FONT),
            );
        });
        return;
    };

    // ── 收集链数据（后续 UI 不再借 workspace）──
    let model = app.workspace.documents[idx].data.model.clone();
    // 该通道上的乐器轨（乐器插件挂在 instrument_channel 上）。
    let instrument_channel: Option<u16> = model
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Instrument && t.global_channel() == channel)
        .and_then(|t| t.instrument_channel);
    let instrument_plugin: Option<String> = instrument_channel.and_then(|ich| {
        app.workspace.documents[idx]
            .mixer
            .instruments
            .get(ich as usize)
            .and_then(|o| o.as_ref())
            .map(|r| r.name.clone())
    });
    let inserts: Vec<(String, bool)> = app.workspace.documents[idx].mixer.channel_inserts
        [channel as usize]
        .iter()
        .map(|r| (r.name.clone(), r.bypassed))
        .collect();
    // lane 归属轨：该通道上的乐器轨优先，否则该通道第一条轨
    //（xsynth 事件本就按通道走，多条轨共享时 lane 只挂一条）。
    let lane_track_ti: usize = model
        .tracks
        .iter()
        .position(|t| t.kind == TrackKind::Instrument && t.global_channel() == channel)
        .or_else(|| {
            model
                .tracks
                .iter()
                .position(|t| t.global_channel() == channel)
        })
        .unwrap_or(0);
    let existing_lanes: Vec<AutomationTarget> = model
        .tracks
        .get(lane_track_ti)
        .map(|t| {
            t.automation_lanes
                .iter()
                .map(|l| l.target.clone())
                .collect()
        })
        .unwrap_or_default();
    let track_names: Vec<String> = model
        .tracks
        .iter()
        .filter(|t| t.global_channel() == channel)
        .map(|t| t.name.clone())
        .collect();
    let active_channels: Vec<u8> = {
        let layout = yinhe_audio::channel_layout::ChannelLayout::from_model(&model);
        (0..256u16)
            .filter(|&c| layout.is_active(c as usize))
            .map(|c| c as u8)
            .collect()
    };

    let mut selected = app.dock_selected.unwrap_or(DockDevice::XSynth);
    let mut open_picker = false;
    let mut create_lane: Option<AutomationTarget> = None;
    let mut open_params: Option<DockDevice> = None;

    // ── 顶部：通道选择 + 使用该通道的轨道 ──
    ui.horizontal(|ui| {
        let mut picked: Option<u8> = None;
        ui.menu_button(
            egui::RichText::new(format!("{} \u{25be}", crate::mix::channel_label(channel)))
                .size(crate::theme::SMALL_FONT + 1.0)
                .color(crate::theme::text_primary()),
            |ui| {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for ch in &active_channels {
                            if ui
                                .selectable_label(*ch == channel, crate::mix::channel_label(*ch))
                                .clicked()
                            {
                                picked = Some(*ch);
                                ui.close();
                            }
                        }
                    });
            },
        );
        if let Some(ch) = picked {
            app.dock_channel = Some(ch);
            app.dock_selected = None;
        }
        if !track_names.is_empty() {
            ui.label(
                egui::RichText::new(track_names.join(", "))
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_muted()),
            );
        }
    });

    // ── 设备链（横向滚动）──
    egui::ScrollArea::horizontal()
        .id_salt("dock_chain")
        .auto_shrink([false, true])
        .max_height(58.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                // 乐器（XSynth 或插件）。
                let is_xsynth = instrument_plugin.is_none();
                let inst_device = if is_xsynth {
                    DockDevice::XSynth
                } else {
                    DockDevice::Instrument
                };
                let title = instrument_plugin.as_deref().unwrap_or("XSynth");
                let subtitle = if is_xsynth {
                    t!("dock.builtin_synth").to_string()
                } else {
                    t!("dock.instrument_plugin").to_string()
                };
                if device_card(
                    ui,
                    egui_material_icons::icons::ICON_MUSIC_NOTE.codepoint,
                    title,
                    &subtitle,
                    selected == inst_device,
                )
                .clicked()
                {
                    selected = inst_device;
                }

                // 效果器链。
                for (slot, (name, bypassed)) in inserts.iter().enumerate() {
                    ui.label(
                        egui::RichText::new("→")
                            .color(crate::theme::text_muted())
                            .size(crate::theme::SMALL_FONT),
                    );
                    let subtitle = if *bypassed {
                        t!("mix.bypass").to_string()
                    } else {
                        t!("dock.effect").to_string()
                    };
                    if device_card(
                        ui,
                        egui_material_icons::icons::ICON_TUNE.codepoint,
                        name,
                        &subtitle,
                        selected == DockDevice::Insert(slot),
                    )
                    .clicked()
                    {
                        selected = DockDevice::Insert(slot);
                    }
                }

                // 添加效果器。
                ui.label(
                    egui::RichText::new("→")
                        .color(crate::theme::text_muted())
                        .size(crate::theme::SMALL_FONT),
                );
                if add_card(ui).clicked() {
                    open_picker = true;
                }
            });
        });

    ui.separator();

    // ── 参数区 ──
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match selected {
            DockDevice::XSynth => {
                xsynth_params(ui, &existing_lanes, &mut create_lane);
            }
            device @ (DockDevice::Instrument | DockDevice::Insert(_)) => {
                plugin_device_params(
                    ui,
                    instrument_plugin.as_deref(),
                    &inserts,
                    device,
                    &mut open_params,
                );
            }
        });

    // ── 应用动作 ──
    app.dock_selected = Some(selected);
    if open_picker {
        app.mix.picker_for = Some(Some(channel));
    }
    if let Some(target) = create_lane {
        app.with_undo(t!("undo.create_automation").as_ref(), |doc| {
            let r = doc.add_automation_lane(lane_track_ti, target);
            if r.is_some()
                && let Some(e) = doc.edit.arr_am_expanded.get_mut(lane_track_ti)
            {
                *e = true;
            }
            r.map(|(_, a)| a)
        });
    }
    if let Some(device) = open_params
        && let Some(panel) = open_param_panel(app, idx, device, channel, instrument_channel)
    {
        app.mix.param_panel = Some(panel);
    }
}

/// 设备卡片（图标 + 名称 + 副标题，可点击）。
fn device_card(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    subtitle: &str,
    selected: bool,
) -> egui::Response {
    let size = egui::vec2(120.0, 52.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if selected {
        crate::theme::accent_active().gamma_multiply(0.22)
    } else if resp.hovered() {
        crate::theme::hover_color(crate::theme::track_bg())
    } else {
        crate::theme::track_bg()
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, bg);
    if selected {
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.2, crate::theme::accent_active()),
            egui::StrokeKind::Inside,
        );
    }
    let text_color = if selected {
        crate::theme::text_primary()
    } else {
        crate::theme::text_secondary()
    };
    painter.text(
        egui::pos2(rect.min.x + 8.0, rect.min.y + 10.0),
        egui::Align2::LEFT_TOP,
        icon,
        egui::FontId::new(
            16.0,
            egui_material_icons::icons::ICON_MUSIC_NOTE.font_family(),
        ),
        if selected {
            crate::theme::accent_active()
        } else {
            crate::theme::text_muted()
        },
    );
    painter.text(
        egui::pos2(rect.min.x + 30.0, rect.min.y + 8.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::new(
            crate::theme::SMALL_FONT + 1.0,
            egui::FontFamily::Proportional,
        ),
        text_color,
    );
    painter.text(
        egui::pos2(rect.min.x + 8.0, rect.min.y + 32.0),
        egui::Align2::LEFT_TOP,
        subtitle,
        egui::FontId::new(crate::theme::SMALL_FONT, egui::FontFamily::Proportional),
        crate::theme::text_muted(),
    );
    resp
}

/// 「+」添加效果器卡片。
fn add_card(ui: &mut egui::Ui) -> egui::Response {
    let size = egui::vec2(120.0, 52.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if resp.hovered() {
        crate::theme::hover_color(crate::theme::track_bg())
    } else {
        crate::theme::btn_bg().gamma_multiply(0.6)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, bg);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, crate::theme::grid_sub_beat()),
        egui::StrokeKind::Inside,
    );
    let add = egui_material_icons::icons::ICON_ADD;
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        add.codepoint,
        egui::FontId::new(16.0, add.font_family()),
        crate::theme::text_muted(),
    );
    resp.on_hover_text(t!("mix.add_insert_hint"))
}

/// XSynth 虚拟乐器的参数列表：每行可一键「显示自动化」。
fn xsynth_params(
    ui: &mut egui::Ui,
    existing: &[AutomationTarget],
    create_lane: &mut Option<AutomationTarget>,
) {
    for target in xsynth_targets() {
        let has_lane = existing.contains(&target);
        ui.horizontal(|ui| {
            ui.add_sized(
                [150.0, 20.0],
                egui::Label::new(
                    egui::RichText::new(target.display_name())
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_primary()),
                )
                .truncate(),
            );
            let default = target.default_value();
            ui.add_sized(
                [80.0, 20.0],
                egui::Label::new(
                    egui::RichText::new(format!("{default:.0}"))
                        .size(crate::theme::SMALL_FONT)
                        .monospace()
                        .color(crate::theme::text_muted()),
                ),
            );
            if has_lane {
                ui.label(
                    egui::RichText::new(t!("dock.lane_exists"))
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::accent_active()),
                );
            } else if ui.small_button(t!("dock.show_automation")).clicked() {
                *create_lane = Some(target.clone());
            }
        });
    }
}

/// 插件设备（乐器/效果器）的参数区：提供参数面板与原生界面入口。
fn plugin_device_params(
    ui: &mut egui::Ui,
    instrument_plugin: Option<&str>,
    inserts: &[(String, bool)],
    device: DockDevice,
    open_params: &mut Option<DockDevice>,
) {
    let name = match device {
        DockDevice::Instrument => instrument_plugin.unwrap_or("?").to_string(),
        DockDevice::Insert(slot) => inserts
            .get(slot)
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| "?".into()),
        DockDevice::XSynth => return,
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(name)
            .size(crate::theme::SMALL_FONT + 1.0)
            .color(crate::theme::text_primary()),
    );
    ui.add_space(6.0);
    if ui.button(t!("dock.open_params")).clicked() {
        *open_params = Some(device);
    }
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t!("dock.plugin_hint"))
            .size(crate::theme::SMALL_FONT)
            .color(crate::theme::text_muted()),
    );
}

/// 打开设备对应的参数面板（复用浮窗 ParamPanel）。
fn open_param_panel(
    app: &mut App,
    idx: usize,
    device: DockDevice,
    channel: u8,
    instrument_channel: Option<u16>,
) -> Option<crate::mix::ParamPanel> {
    use crate::mix::param_panel::{ParamPanel, ParamTarget};
    match device {
        DockDevice::Instrument => {
            let ich = instrument_channel?;
            let instance = app
                .instrument_racks
                .get_mut(idx)
                .and_then(|rack| rack.instance_mut(ich))?;
            let title = instance.info().name.clone();
            Some(ParamPanel::open(
                ParamTarget::Instrument { channel: ich },
                title,
                instance,
            ))
        }
        DockDevice::Insert(slot) => {
            let instance = app
                .mixer_racks
                .get_mut(idx)
                .and_then(|rack| rack.instance_mut(Some(channel), slot))?;
            let title = instance.info().name.clone();
            Some(ParamPanel::open(
                ParamTarget::Insert {
                    channel: Some(channel),
                    slot,
                },
                title,
                instance,
            ))
        }
        DockDevice::XSynth => None,
    }
}
