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
use yinhe_types::{AutomationEvent, AutomationTarget};

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

/// 旋钮拖动会话（一次拖动 = 一条 undo）。
pub(crate) struct KnobDrag {
    track_idx: usize,
    target: AutomationTarget,
    /// 写入位置（编辑光标 tick）。
    tick: u32,
    /// 拖动开始时的界面快照（undo 用）。
    snapshot: yinhe_editor_core::history::EditSnapshot,
    /// 拖动开始前该 lane 的完整事件（lane 尚未创建时为空）。
    before: Vec<AutomationEvent>,
    /// 当前 lane 索引（懒创建后记录）。
    lane_idx: Option<usize>,
}

/// 单帧内旋钮产生的动作（渲染后统一应用，避开借用冲突）。
enum KnobAction {
    DragStart(AutomationTarget),
    /// 拖动到归一化值 `norm`。
    Drag(AutomationTarget, f32),
    DragStop(AutomationTarget),
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
    // 乐器旁通状态：该通道所有轨道都 muted 视为旁通（任一未 mute = 开着）。
    let inst_powered = !model
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.global_channel() == channel)
        .all(|(ti, _)| {
            app.workspace.documents[idx]
                .edit
                .track_overrides
                .get(ti)
                .map(|o| o.muted)
                .unwrap_or(false)
        });

    // 编辑光标位置（旋钮写入位置）与各自动化 target 的当前值。
    let tick = {
        let doc = &app.workspace.documents[idx];
        doc.edit.cursor_tick.unwrap_or(0.0).max(0.0) as u32
    };
    let lane_current: Vec<(AutomationTarget, Option<f32>)> = xsynth_targets()
        .into_iter()
        .map(|target| {
            // 光标处应显示的值：<= tick 的最后一条事件（tick 0 也要命中，
            // 与 chase 的 value_at 语义不同——那是"事件之后才生效"）。
            let value = model
                .tracks
                .get(lane_track_ti)
                .and_then(|t| t.automation_lanes.iter().find(|l| l.target == target))
                .and_then(|l| {
                    l.events
                        .iter()
                        .rev()
                        .find(|e| e.tick <= tick)
                        .map(|e| e.value)
                });
            (target, value)
        })
        .collect();
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

    // 默认选中乐器设备；上次选中项在当前链上无效时回退（换通道/插件被移除）。
    let is_xsynth = instrument_plugin.is_none();
    let default_device = if is_xsynth {
        DockDevice::XSynth
    } else {
        DockDevice::Instrument
    };
    let mut selected = app.dock_selected.unwrap_or(default_device);
    let valid = match selected {
        DockDevice::XSynth => is_xsynth,
        DockDevice::Instrument => !is_xsynth,
        DockDevice::Insert(slot) => slot < inserts.len(),
    };
    if !valid {
        selected = default_device;
    }
    let mut open_picker = false;
    let mut knob_actions: Vec<KnobAction> = Vec::new();
    let mut open_params: Option<DockDevice> = None;
    let mut toggle_bypass: Option<(usize, bool)> = None;
    let mut toggle_instrument: Option<bool> = None;
    let mut open_gui: Option<DockDevice> = None;

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

    // ── 主体：左侧设备大卡片（含参数列表）+ 效果器小卡片 + 「+」竖条 ──
    let inst_title = instrument_plugin.as_deref().unwrap_or("XSynth");

    let avail = ui.available_size();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        const FX_W: f32 = 108.0;
        const ADD_W: f32 = 44.0;
        let fx_total = inserts.len() as f32 * (FX_W + 6.0);
        let big_w = (avail.x - fx_total - ADD_W - 12.0).clamp(200.0, 240.0);

        // 设备大卡片：标题 + 参数（XSynth 为旋钮纵向列表；插件为入口按钮）。
        ui.allocate_ui_with_layout(
            egui::vec2(big_w, avail.y),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                big_device_card(
                    ui,
                    selected,
                    inst_title,
                    &lane_current,
                    &inserts,
                    &mut app.dock_param_search,
                    inst_powered,
                    &mut knob_actions,
                    &mut open_params,
                    &mut toggle_bypass,
                    &mut toggle_instrument,
                    &mut open_gui,
                );
            },
        );

        // 效果器小卡片（横向）。
        for (slot, (name, bypassed)) in inserts.iter().enumerate() {
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

        // 「+」竖条：撑满高度，点击添加效果器。
        if add_column(ui, avail.y).clicked() {
            open_picker = true;
        }
    });

    // ── 应用动作 ──
    app.dock_selected = Some(selected);
    if open_picker {
        app.mix.picker_for = Some(yinhe_audio::InsertTarget::Channel(channel));
    }
    for action in knob_actions {
        apply_knob_action(app, idx, lane_track_ti, tick, action);
    }
    if let Some(device) = open_params
        && let Some(panel) = open_param_panel(app, idx, device, channel, instrument_channel)
    {
        app.mix.param_panel = Some(panel);
    }
    if let Some(muted) = toggle_instrument {
        // 乐器旁通 = 该通道所有轨道 mute（AR 读 track_overrides，自动同步）。
        let doc = &mut app.workspace.documents[idx];
        let track_ids: Vec<usize> = doc
            .data
            .model
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.global_channel() == channel)
            .map(|(ti, _)| ti)
            .collect();
        for ti in track_ids {
            if let Some(ov) = doc.edit.track_overrides.get_mut(ti) {
                ov.muted = muted;
            }
        }
        let audio = app.audio_state.handle.as_ref();
        crate::right_panel::info_panel::send_skip_tracks(doc, audio);
    }
    if let Some((slot, bypassed)) = toggle_bypass {
        if let Some(r) = app.workspace.documents[idx]
            .mixer_mut()
            .channel_inserts
            .get_mut(channel as usize)
            .and_then(|chain| chain.get_mut(slot))
        {
            r.bypassed = bypassed;
        }
        if let Some(rack) = app.mixer_racks.get_mut(idx) {
            rack.set_bypass(Some(channel), slot, bypassed);
        }
    }
    if let Some(DockDevice::Insert(slot)) = open_gui {
        #[cfg(target_os = "macos")]
        if let Err(e) = app.mixer_rack_mut(idx).toggle_gui(Some(channel), slot) {
            app.mixer_rack_mut(idx).last_error = Some(e.0);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = slot;
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

/// 「+」添加效果器竖条（高度撑满主体区）。
fn add_column(ui: &mut egui::Ui, height: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, height), egui::Sense::click());
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

/// 设备大卡片：标题行（电源 / 名称 / 搜索 / 打开 GUI）+ 内容
/// （XSynth 旋钮纵向列表；插件设备参数面板入口）。
#[allow(clippy::too_many_arguments)] // UI 上下文透传，见 AGENTS 约定
fn big_device_card(
    ui: &mut egui::Ui,
    selected: DockDevice,
    inst_title: &str,
    lane_current: &[(AutomationTarget, Option<f32>)],
    inserts: &[(String, bool)],
    search: &mut String,
    inst_powered: bool,
    knob_actions: &mut Vec<KnobAction>,
    open_params: &mut Option<DockDevice>,
    toggle_bypass: &mut Option<(usize, bool)>,
    toggle_instrument: &mut Option<bool>,
    open_gui: &mut Option<DockDevice>,
) {
    egui::Frame::new()
        .fill(crate::theme::track_bg())
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());

            // ── 标题行 ──
            ui.horizontal(|ui| {
                // 电源：强调色 = 开着；点击切换旁通
                // （效果器 = 自身旁通；乐器/XSynth = 该通道所有轨道 mute）。
                let powered = match selected {
                    DockDevice::Insert(slot) => {
                        !inserts.get(slot).map(|(_, b)| *b).unwrap_or(false)
                    }
                    _ => inst_powered,
                };
                let power = egui_material_icons::icons::ICON_POWER_SETTINGS_NEW;
                let power_color = if powered {
                    crate::theme::accent_active()
                } else {
                    crate::theme::text_muted()
                };
                let power_resp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(power.codepoint)
                            .font(egui::FontId::new(14.0, power.font_family()))
                            .color(power_color),
                    )
                    .frame(false),
                );
                if power_resp.clicked() {
                    match selected {
                        DockDevice::Insert(slot) => {
                            if let Some((_, bypassed)) = inserts.get(slot) {
                                *toggle_bypass = Some((slot, !*bypassed));
                            }
                        }
                        // 目标 muted 值 = 当前是否开着（true→全 mute，false→全恢复）。
                        _ => *toggle_instrument = Some(inst_powered),
                    }
                }
                power_resp.on_hover_text(t!("mix.bypass"));

                // 名称。
                let name = match selected {
                    DockDevice::XSynth => "XSynth".to_string(),
                    DockDevice::Instrument => inst_title.to_string(),
                    DockDevice::Insert(slot) => inserts
                        .get(slot)
                        .map(|(n, _)| n.clone())
                        .unwrap_or_else(|| "?".into()),
                };
                ui.label(
                    egui::RichText::new(name)
                        .size(crate::theme::SMALL_FONT + 2.0)
                        .color(crate::theme::text_primary()),
                );

                // 搜索（仅 XSynth 参数列表）。
                if matches!(selected, DockDevice::XSynth) {
                    let search_font = egui::FontId::proportional(crate::theme::SMALL_FONT);
                    ui.add(
                        egui::TextEdit::singleline(search)
                            .desired_width(88.0)
                            .hint_text(
                                egui::RichText::new(t!("mix.search")).font(search_font.clone()),
                            )
                            .font(search_font),
                    );
                }

                // 打开插件原生 GUI（仅插件设备；XSynth 无原生界面）。
                if matches!(selected, DockDevice::Instrument | DockDevice::Insert(_)) {
                    let home = egui_material_icons::icons::ICON_HOME_STORAGE;
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new(home.codepoint)
                                .font(egui::FontId::new(14.0, home.font_family()))
                                .color(crate::theme::text_secondary()),
                        )
                        .frame(false),
                    );
                    if resp.clicked() {
                        *open_gui = Some(selected);
                    }
                    resp.on_hover_text(t!("mix.toggle_gui"));
                }
            });
            ui.add_space(6.0);

            // ── 内容 ──
            match selected {
                DockDevice::XSynth => {
                    let needle = search.trim().to_lowercase();
                    let filtered: Vec<&(AutomationTarget, Option<f32>)> = lane_current
                        .iter()
                        .filter(|(t, _)| {
                            needle.is_empty() || t.display_name().to_lowercase().contains(&needle)
                        })
                        .collect();
                    egui::ScrollArea::vertical()
                        .id_salt("xsynth_knobs")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            xsynth_params(ui, &filtered, knob_actions);
                        });
                }
                device @ (DockDevice::Instrument | DockDevice::Insert(_)) => {
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
            }
        });
}

/// XSynth 参数纵向列表：每项「旋钮 + 右侧两行（名称、数值）」。
fn xsynth_params(
    ui: &mut egui::Ui,
    values: &[&(AutomationTarget, Option<f32>)],
    actions: &mut Vec<KnobAction>,
) {
    for (target, current) in values {
        knob_row(ui, target, *current, actions);
    }
}

/// 单个参数行：左旋钮 + 右两行（第一行名称、第二行数值）。
fn knob_row(
    ui: &mut egui::Ui,
    target: &AutomationTarget,
    current: Option<f32>,
    actions: &mut Vec<KnobAction>,
) {
    let max = target.max_value();
    let raw = current.unwrap_or_else(|| target.default_value());
    let mut norm = (raw / max.max(1.0)).clamp(0.0, 1.0);

    ui.horizontal(|ui| {
        let resp = crate::widgets::knob::knob(ui, &mut norm, 28.0);
        if resp.drag_started() {
            actions.push(KnobAction::DragStart(target.clone()));
        }
        if resp.dragged() {
            actions.push(KnobAction::Drag(target.clone(), norm));
        }
        if resp.drag_stopped() {
            actions.push(KnobAction::DragStop(target.clone()));
        }

        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(target.display_name())
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_primary()),
            );
            let (text, color) = match current {
                Some(v) => (format_value(v), crate::theme::accent_active()),
                None => (
                    format!("{}（默认）", format_value(target.default_value())),
                    crate::theme::text_muted(),
                ),
            };
            ui.label(
                egui::RichText::new(text)
                    .size(crate::theme::SMALL_FONT)
                    .monospace()
                    .color(color),
            );
        });
    });
    ui.add_space(6.0);
}

/// 自动化原始值 → 显示文本（整数域取整）。
fn format_value(value: f32) -> String {
    format!("{value:.0}")
}

/// 应用单帧旋钮动作：拖动中 upsert 事件，松手 push 一条 undo。
fn apply_knob_action(app: &mut App, idx: usize, track_idx: usize, tick: u32, action: KnobAction) {
    match action {
        KnobAction::DragStart(target) => {
            let doc = &mut app.workspace.documents[idx];
            if track_idx >= doc.data.model.tracks.len() {
                return;
            }
            let lane_pos = doc.data.model.tracks[track_idx]
                .automation_lanes
                .iter()
                .position(|l| l.target == target);
            let before = lane_pos
                .map(|li| {
                    doc.data.model.tracks[track_idx].automation_lanes[li]
                        .events
                        .clone()
                })
                .unwrap_or_default();
            app.knob_drag = Some(KnobDrag {
                track_idx,
                target,
                tick,
                snapshot: doc.capture_snapshot(),
                before,
                lane_idx: lane_pos,
            });
        }
        KnobAction::Drag(target, norm) => {
            let Some(mut drag) = app.knob_drag.take() else {
                return;
            };
            if drag.target == target {
                let raw = norm * drag.target.max_value();
                upsert_automation_event(app, idx, &mut drag, raw);
                app.notify_audio_model_changed();
            }
            app.knob_drag = Some(drag);
        }
        KnobAction::DragStop(target) => {
            let Some(drag) = app.knob_drag.take_if(|d| d.target == target) else {
                return;
            };
            let Some(lane_idx) = drag.lane_idx else {
                return;
            };
            let doc = &mut app.workspace.documents[idx];
            let after = crate::right_panel::automation_undo::snapshot_lane_events(
                doc,
                drag.track_idx as u16,
                lane_idx,
                &drag.target,
            );
            crate::right_panel::automation_undo::push_automation_undo(
                doc,
                drag.track_idx as u16,
                lane_idx,
                &drag.target,
                drag.before,
                after,
                t!("undo.edit_automation").as_ref(),
                drag.snapshot,
            );
        }
    }
}

/// 在拖动会话的 tick 处写入/覆盖自动化事件（lane 懒创建）。
fn upsert_automation_event(app: &mut App, idx: usize, drag: &mut KnobDrag, raw: f32) {
    let doc = &mut app.workspace.documents[idx];
    let lane_pos = doc.data.model.tracks[drag.track_idx]
        .automation_lanes
        .iter()
        .position(|l| l.target == drag.target);
    match lane_pos {
        Some(lane_idx) => {
            drag.lane_idx = Some(lane_idx);
            let has_event = doc.data.model.tracks[drag.track_idx].automation_lanes[lane_idx]
                .events
                .iter()
                .any(|e| e.tick == drag.tick);
            if has_event {
                doc.move_automation_event(
                    drag.track_idx,
                    lane_idx,
                    &drag.target,
                    drag.tick,
                    drag.tick,
                    raw,
                );
            } else {
                doc.add_automation_event(
                    drag.track_idx,
                    drag.target.clone(),
                    AutomationEvent {
                        tick: drag.tick,
                        value: raw,
                        shape: drag.target.default_shape(),
                    },
                );
            }
        }
        None => {
            doc.add_automation_event(
                drag.track_idx,
                drag.target.clone(),
                AutomationEvent {
                    tick: drag.tick,
                    value: raw,
                    shape: drag.target.default_shape(),
                },
            );
            drag.lane_idx = doc.data.model.tracks[drag.track_idx]
                .automation_lanes
                .iter()
                .position(|l| l.target == drag.target);
        }
    }
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
            let title = instance.name().to_string();
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
            let title = instance.name().to_string();
            Some(ParamPanel::open(
                ParamTarget::Insert {
                    target: yinhe_audio::InsertTarget::Channel(channel),
                    slot,
                },
                title,
                instance,
            ))
        }
        DockDevice::XSynth => None,
    }
}
