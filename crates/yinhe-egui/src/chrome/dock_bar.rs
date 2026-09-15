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
    /// 该 MIDI 通道的乐器设备：默认内置 XSynth，可挂载 CLAP/VST3 插件。
    Instrument,
    /// 效果器链槽位（按通道链索引）。
    Insert(usize),
}

/// 旋钮拖动会话（一次拖动 = 一条 undo）。
pub(crate) struct KnobDrag {
    /// 最近一次拖动到的归一化值（松手落 lane 用）。
    last_norm: f32,
    /// 本会话是否真的拖动过（单击不写事件）。
    moved: bool,
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

/// 旋钮归属：决定拖动走"实时预览"还是"直写 lane"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KnobOwner {
    /// 乐器参数（走 lane → 合成器；当前无实时预览通道，松手落 lane）。
    Instrument,
    /// 效果器槽位（拖动实时预览走参数队列，不写模型）。
    Insert(usize),
}

/// 单帧内旋钮产生的动作（渲染后统一应用，避开借用冲突）。
enum KnobAction {
    DragStart(KnobOwner, AutomationTarget),
    /// 拖动到归一化值 `norm`。
    Drag(KnobOwner, AutomationTarget, f32),
    DragStop(KnobOwner, AutomationTarget),
}

/// xsynth 支持的**音源层**自动化目标。
///
/// 通道级 DSP CC（7/10/11/71/74）已迁移到 yinhe-dsp 模块（效果器卡片上的旋钮），
/// 不再由 xsynth 处理——见 `docs/spec-yinhe-dsp.md`。
fn xsynth_targets() -> Vec<AutomationTarget> {
    vec![
        AutomationTarget::CC { controller: 64 },
        AutomationTarget::CC { controller: 72 },
        AutomationTarget::CC { controller: 73 },
        AutomationTarget::PitchBend,
        AutomationTarget::Rpn { parameter: 0 },
        AutomationTarget::Rpn { parameter: 1 },
        AutomationTarget::Rpn { parameter: 2 },
    ]
}

/// 效果器链槽位在 dock 的展示数据。
pub(crate) struct DockInsert {
    name: String,
    bypassed: bool,
    /// 内置效果器的参数（插件为空，走参数面板入口）。
    params: Vec<DockParam>,
}

/// dock 旋钮的一项参数。
///
/// 显示名是**效果器/XSynth 自己的参数名**；底层 `target` 是工程存储与导出
/// 用的自动化目标（内置效果器参数在底层伪装成 MIDI CC，仅内部映射，
/// 不外显为 CC 概念）。
#[derive(Clone)]
pub(crate) struct DockParam {
    name: String,
    target: AutomationTarget,
    /// 无事件时的默认值。
    default: f32,
    /// 光标处的当前值（None = 未设置，显示默认值）。
    current: Option<f32>,
}

/// 内置效果器的参数 → DockParam（底层伪 CC 映射）；插件返回空。
fn builtin_params(r: &yinhe_mixer::InsertRef) -> Vec<DockParam> {
    if r.format != yinhe_mixer::PluginFormat::Builtin {
        return Vec::new();
    }
    yinhe_dsp::BuiltinEffectKind::from_id(&r.plugin_id)
        .map(|kind| {
            kind.params()
                .iter()
                .map(|p| DockParam {
                    name: p.name.to_string(),
                    target: AutomationTarget::CC { controller: p.cc },
                    default: p.default,
                    current: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 光标处应显示的值：`<= tick` 的最后一条事件（tick 0 也要命中）。
fn lane_current_value(
    model: &yinhe_core::YinModel,
    track_ti: usize,
    tick: u32,
    target: &AutomationTarget,
) -> Option<f32> {
    model
        .tracks
        .get(track_ti)
        .and_then(|t| t.automation_lanes.iter().find(|l| l.target == *target))
        .and_then(|l| {
            l.events
                .iter()
                .rev()
                .find(|e| e.tick <= tick)
                .map(|e| e.value)
        })
}

/// dock 高度下限（与旧 Panel min_size 一致）。
const DOCK_MIN_H: f32 = 120.0;
/// dock 内容水平内边距（等价旧 frame `inner_margin(8, 4)` 的横向值）。
const DOCK_PAD_X: f32 = 8.0;

/// 每帧绘制底部设备栏（mode_bar 之后、compute_layout 之前调用）。
pub(crate) fn show(app: &mut App, ui: &mut egui::Ui) {
    if !app.show_bottom_dock {
        return;
    }
    let Some(idx) = app.workspace.active_doc else {
        return;
    };

    let max_h = (ui.available_height() - 160.0).max(200.0);
    let h = app.bottom_dock_height.clamp(DOCK_MIN_H, max_h);

    // 顶部 2px 用项目统一的 `split_handle::horizontal`（同右栏/PR 样式与交互），
    // 不用 egui Panel 原生 resizable 分隔线——它的颜色取自全局 Visuals，
    // 亮暗主题下与 `theme::line_fg` 不一致（高亮发黑）。
    egui::Panel::bottom("bottom_dock")
        .exact_size(h)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame {
            fill: crate::theme::app_bg(),
            inner_margin: egui::Margin::ZERO,
            ..Default::default()
        })
        .show(ui, |ui| {
            let panel_rect = ui.max_rect();
            let handle_rect = egui::Rect::from_min_size(
                panel_rect.min,
                egui::vec2(panel_rect.width(), crate::theme::SPLIT_HANDLE_W),
            );
            let handle =
                crate::widgets::split_handle::horizontal(ui, "__dock_split__", handle_rect);
            if handle.dragged() {
                // 分割线向上拖 → dock 变高（drag_delta().y 为负）。
                app.bottom_dock_height =
                    (app.bottom_dock_height - handle.drag_delta().y).clamp(DOCK_MIN_H, max_h);
            }
            // 拖动结束：持久化高度（帧末统一落盘）。
            if handle.drag_stopped() {
                app.layout_needs_save = true;
            }

            // 内容区：等价原 frame `inner_margin(8, 4)`，顶部再让出分割线。
            let content = egui::Rect::from_min_max(
                egui::pos2(panel_rect.min.x + DOCK_PAD_X, handle_rect.max.y + 4.0),
                egui::pos2(panel_rect.max.x - DOCK_PAD_X, panel_rect.max.y - 4.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                show_body(app, idx, ui);
            });
        });
}

/// dock 的通道语境（MIDI / 音频两套命名空间独立）。
/// MIDI 通道同时是乐器（XSynth 或插件）的挂载点。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DockContext {
    /// MIDI 源通道（0..255；乐器输出也走该通道）。
    Midi(u8),
    /// 音频通道（0 起）。
    Audio(u16),
}

/// 选中轨 → dock 通道语境。
fn track_context(t: &yinhe_core::TrackData) -> DockContext {
    match t.kind {
        TrackKind::Audio => DockContext::Audio(t.audio_channel.unwrap_or(0)),
        TrackKind::Midi => DockContext::Midi(t.global_channel()),
    }
}

/// 设备链 + 参数区。
fn show_body(app: &mut App, idx: usize, ui: &mut egui::Ui) {
    // ── 通道选择（Studio One 风格：设备链按通道组织；三类通道独立）──
    let selected_track = {
        let doc = &app.workspace.documents[idx];
        doc.edit.track_selected.iter().min().copied()
    };
    if selected_track != app.dock_track {
        // 切换选中轨：dock 跟随该轨所在通道（音频轨→音频通道，乐器轨→乐器通道），
        // 并重置设备选中。
        app.dock_track = selected_track;
        if let Some(context) = selected_track
            .and_then(|ti| {
                app.workspace.documents[idx]
                    .data
                    .model
                    .tracks
                    .get(ti as usize)
            })
            .map(|t| track_context(t))
        {
            app.dock_context = Some(context);
            app.dock_selected = None;
        }
    }
    let Some(context) = app.dock_context else {
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
    // 语境 → 相关通道（MIDI / 音频两套命名空间各一条链）。
    let midi_channel = match context {
        DockContext::Midi(ch) => Some(ch),
        _ => None,
    };
    let insert_target = match context {
        DockContext::Midi(ch) => yinhe_audio::InsertTarget::Channel(ch),
        DockContext::Audio(ach) => yinhe_audio::InsertTarget::Audio(ach),
    };
    // 该 MIDI 通道挂载的插件乐器名（None = 默认内置 XSynth）。
    let instrument_plugin: Option<String> = midi_channel.and_then(|ch| {
        app.workspace.documents[idx]
            .mixer
            .instruments
            .get(ch as usize)
            .and_then(|o| o.as_ref())
            .map(|r| r.name.clone())
    });
    let inserts: Vec<DockInsert> =
        crate::mix::insert_refs(&mut app.workspace.documents[idx].mixer, insert_target)
            .map(|chain| {
                chain
                    .iter()
                    .map(|r| DockInsert {
                        name: r.name.clone(),
                        bypassed: r.bypassed,
                        params: builtin_params(r),
                    })
                    .collect()
            })
            .unwrap_or_default();
    // 语境归属轨（该通道上的轨道）。
    let belongs = |t: &std::sync::Arc<yinhe_core::TrackData>| match context {
        DockContext::Midi(ch) => t.global_channel() == ch,
        DockContext::Audio(ach) => t.audio_channel == Some(ach),
    };
    // lane 归属轨：该通道第一条轨（事件按通道走，多条轨共享时 lane 只挂一条）。
    let lane_track_ti: usize = model.tracks.iter().position(belongs).unwrap_or(0);
    // 归属轨索引与旁通状态：该通道所有轨道都 muted 视为旁通（任一未 mute = 开着）。
    let powered_tracks: Vec<usize> = model
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| belongs(t))
        .map(|(ti, _)| ti)
        .collect();
    let inst_powered = !powered_tracks.is_empty()
        && !powered_tracks.iter().all(|&ti| {
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
    let lane_current: Vec<DockParam> = xsynth_targets()
        .into_iter()
        .map(|target| {
            let current = lane_current_value(&model, lane_track_ti, tick, &target);
            DockParam {
                name: target.display_name(),
                default: target.default_value(),
                target,
                current,
            }
        })
        .collect();
    let track_names: Vec<String> = model
        .tracks
        .iter()
        .filter(|t| belongs(t))
        .map(|t| t.name.clone())
        .collect();
    // 顶部选择器：三类通道的各活跃项。
    let midi_active: Vec<u8> = {
        let layout = yinhe_audio::channel_layout::ChannelLayout::from_model(&model);
        (0..256u16)
            .filter(|&c| layout.is_active(c as usize))
            .map(|c| c as u8)
            .collect()
    };
    let mut audio_active: Vec<u16> = model
        .tracks
        .iter()
        .filter_map(|t| t.audio_channel)
        .collect();
    audio_active.sort_unstable();
    audio_active.dedup();

    // 主设备位：MIDI 通道的乐器（默认 XSynth，挂载插件时显示插件名）；
    // 音频语境无主设备（只有 insert 链）。
    let default_device = midi_channel.map(|_| DockDevice::Instrument);
    let valid = |sel: &DockDevice| match sel {
        DockDevice::Instrument => midi_channel.is_some(),
        DockDevice::Insert(slot) => *slot < inserts.len(),
    };
    let mut selected: Option<DockDevice> = app
        .dock_selected
        .filter(|sel| valid(sel))
        .or(default_device);
    let mut open_picker = false;
    let mut knob_actions: Vec<KnobAction> = Vec::new();
    let mut open_params: Option<DockDevice> = None;
    let mut toggle_bypass: Option<(usize, bool)> = None;
    let mut toggle_instrument: Option<bool> = None;
    let mut open_gui: Option<DockDevice> = None;
    let mut open_instrument_picker: Option<u8> = None;

    // ── 顶部：通道选择 + 使用该通道的轨道 ──
    ui.horizontal(|ui| {
        let mut picked: Option<DockContext> = None;
        ui.menu_button(
            egui::RichText::new(format!("{} \u{25be}", context_label(context)))
                .size(crate::theme::SMALL_FONT + 1.0)
                .color(crate::theme::text_primary()),
            |ui| {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for ch in &midi_active {
                            if ui
                                .selectable_label(
                                    context == DockContext::Midi(*ch),
                                    crate::mix::channel_label(*ch),
                                )
                                .clicked()
                            {
                                picked = Some(DockContext::Midi(*ch));
                                ui.close();
                            }
                        }
                        if !audio_active.is_empty() {
                            ui.separator();
                            for ach in &audio_active {
                                if ui
                                    .selectable_label(
                                        context == DockContext::Audio(*ach),
                                        crate::mix::audio_label(*ach),
                                    )
                                    .clicked()
                                {
                                    picked = Some(DockContext::Audio(*ach));
                                    ui.close();
                                }
                            }
                        }
                    });
            },
        );
        if let Some(ctx) = picked {
            app.dock_context = Some(ctx);
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

    // ── 主体：最左乐器大卡片（固定不随选中变化）+ 每个效果器一张大卡片 + 「+」──
    let avail = ui.available_size();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        const CARD_W: f32 = 208.0;
        const ADD_W: f32 = 44.0;

        // 乐器大卡片：永远在最左，固定显示乐器（XSynth 旋钮 / 插件入口）。
        ui.allocate_ui_with_layout(
            egui::vec2(CARD_W, avail.y),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                instrument_card(
                    ui,
                    context,
                    instrument_plugin.as_deref(),
                    &lane_current,
                    &mut app.dock_param_search,
                    inst_powered,
                    &mut knob_actions,
                    &mut open_params,
                    &mut toggle_instrument,
                    &mut open_gui,
                    &mut open_instrument_picker,
                    midi_channel,
                );
            },
        );

        // 效果器大卡片（横向滚动）：内置效果器显示自己的 CC 旋钮，插件显示参数入口。
        let fx_width = (avail.x - CARD_W - ADD_W - 20.0).max(120.0);
        ui.allocate_ui_with_layout(
            egui::vec2(fx_width, avail.y),
            egui::Layout::left_to_right(egui::Align::TOP),
            |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("dock_fx_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (slot, ins) in inserts.iter().enumerate() {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(CARD_W, avail.y),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| {
                                        if effect_card(
                                            ui,
                                            slot,
                                            ins,
                                            &model,
                                            lane_track_ti,
                                            tick,
                                            selected,
                                            &mut knob_actions,
                                            &mut open_params,
                                            &mut toggle_bypass,
                                            &mut open_gui,
                                        ) {
                                            selected = Some(DockDevice::Insert(slot));
                                        }
                                    },
                                );
                            }
                        });
                    });
            },
        );

        // 「+」竖条：撑满高度，点击添加效果器。
        if add_column(ui, avail.y).clicked() {
            open_picker = true;
        }
    });

    // ── 应用动作 ──
    app.dock_selected = selected;
    if open_picker {
        app.mix.picker_for = Some(insert_target);
    }
    for action in knob_actions {
        apply_knob_action(app, idx, insert_target, lane_track_ti, tick, action);
    }
    if let Some(device) = open_params {
        match open_param_panel(app, idx, device, insert_target, midi_channel) {
            Some(panel) => app.mix.param_panel = Some(panel),
            None => {
                // 实例不可用（未加载成功）：在 MIX 状态行提示，避免"点了没反应"。
                let msg = t!("dock.plugin_unavailable").to_string();
                match device {
                    DockDevice::Instrument => {
                        if let Some(rack) = app.instrument_racks.get_mut(idx) {
                            rack.last_error = Some(msg);
                        }
                    }
                    _ => {
                        let rack = app.mixer_rack_mut(idx);
                        rack.last_error = Some(msg);
                    }
                }
            }
        }
    }
    if let Some(ch) = open_instrument_picker {
        app.mix.instrument_picker_for = Some(ch);
    }
    if let Some(muted) = toggle_instrument {
        // 旁通 = 该通道所有轨道 mute（AR 读 track_overrides，自动同步）。
        let doc = &mut app.workspace.documents[idx];
        for &ti in &powered_tracks {
            if let Some(ov) = doc.edit.track_overrides.get_mut(ti) {
                ov.muted = muted;
            }
        }
        let audio = app.audio_state.handle.as_ref();
        crate::right_panel::info_panel::send_skip_tracks(doc, audio);
    }
    if let Some((slot, bypassed)) = toggle_bypass {
        if let Some(r) =
            crate::mix::insert_refs(&mut app.workspace.documents[idx].mixer, insert_target)
                .and_then(|chain| chain.get_mut(slot))
        {
            r.bypassed = bypassed;
        }
        if let Some(rack) = app.mixer_racks.get_mut(idx) {
            rack.set_bypass(insert_target, slot, bypassed);
        }
    }
    match open_gui {
        Some(DockDevice::Insert(slot)) => {
            #[cfg(target_os = "macos")]
            if let Err(e) = app.mixer_rack_mut(idx).toggle_gui(insert_target, slot) {
                app.mixer_rack_mut(idx).last_error = Some(e.0);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = slot;
        }
        // MIDI 通道乐器：挂插件 → 插件原生界面；默认 XSynth → 音色库配置窗口。
        Some(DockDevice::Instrument) => {
            let Some(ch) = midi_channel else { return };
            if instrument_plugin.is_some() {
                let result = app
                    .instrument_racks
                    .get_mut(idx)
                    .map(|rack| rack.toggle_gui(ch));
                if let Some(Err(e)) = result
                    && let Some(rack) = app.instrument_racks.get_mut(idx)
                {
                    rack.last_error = Some(e.0);
                }
            } else {
                // 内置 XSynth 的"界面"就是音色库配置窗口。
                app.mix.xsynth_config_for = Some(ch);
            }
        }
        None => {}
    }
}

/// 语境标签（MIDI-A01 / Audio-01）。
fn context_label(context: DockContext) -> String {
    match context {
        DockContext::Midi(ch) => crate::mix::channel_label(ch),
        DockContext::Audio(ach) => crate::mix::audio_label(ach),
    }
}

/// 效果器大卡片：内置效果器显示自己的 CC 旋钮（拖动写 lane，可自动化），
/// 插件效果器显示参数面板 / 原生界面入口。返回 true = 卡片被点击（选中高亮）。
#[allow(clippy::too_many_arguments)] // dock 卡片上下文透传，见 AGENTS 约定
fn effect_card(
    ui: &mut egui::Ui,
    slot: usize,
    ins: &DockInsert,
    model: &yinhe_core::YinModel,
    lane_track_ti: usize,
    tick: u32,
    selected: Option<DockDevice>,
    knob_actions: &mut Vec<KnobAction>,
    open_params: &mut Option<DockDevice>,
    toggle_bypass: &mut Option<(usize, bool)>,
    open_gui: &mut Option<DockDevice>,
) -> bool {
    let is_selected = selected == Some(DockDevice::Insert(slot));
    let builtin = !ins.params.is_empty();
    let mut clicked = false;

    let mut frame = egui::Frame::new()
        .fill(crate::theme::track_bg())
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 8));
    if is_selected {
        frame = frame.stroke(egui::Stroke::new(1.0, crate::theme::accent_active()));
    }
    frame.show(ui, |ui| {
        ui.set_min_size(ui.available_size());

        // ── 标题行：电源（旁通）+ 名称 + 插件入口 ──
        ui.horizontal(|ui| {
            let power = egui_material_icons::icons::ICON_POWER_SETTINGS_NEW;
            let power_color = if ins.bypassed {
                crate::theme::text_muted()
            } else {
                crate::theme::accent_active()
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
                *toggle_bypass = Some((slot, !ins.bypassed));
            }
            power_resp.on_hover_text(t!("mix.bypass"));

            let name_resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(&ins.name)
                        .size(crate::theme::SMALL_FONT + 2.0)
                        .color(crate::theme::text_primary()),
                )
                .sense(egui::Sense::click()),
            );
            if name_resp.clicked() {
                clicked = true;
            }

            // 插件效果器：参数面板 / 原生界面入口（内置效果器参数就在卡片上）。
            if !builtin {
                let params = egui_material_icons::icons::ICON_TUNE;
                let presp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(params.codepoint)
                            .font(egui::FontId::new(13.0, params.font_family()))
                            .color(crate::theme::text_secondary()),
                    )
                    .frame(false),
                );
                if presp.clicked() {
                    *open_params = Some(DockDevice::Insert(slot));
                }
                presp.on_hover_text(t!("dock.open_params"));

                let gui = egui_material_icons::icons::ICON_HOME_STORAGE;
                let gresp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(gui.codepoint)
                            .font(egui::FontId::new(13.0, gui.font_family()))
                            .color(crate::theme::text_secondary()),
                    )
                    .frame(false),
                );
                if gresp.clicked() {
                    *open_gui = Some(DockDevice::Insert(slot));
                }
                gresp.on_hover_text(t!("mix.toggle_gui"));
            }
        });
        ui.add_space(6.0);

        // ── 内容 ──
        if builtin {
            // 内置效果器：自己的参数旋钮（值取光标处 lane 值，拖动写 lane）。
            let values: Vec<DockParam> = ins
                .params
                .iter()
                .map(|p| DockParam {
                    current: lane_current_value(model, lane_track_ti, tick, &p.target),
                    ..p.clone()
                })
                .collect();
            egui::ScrollArea::vertical()
                .id_salt(("dock_fx_knobs", slot))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    param_knobs(ui, &values, KnobOwner::Insert(slot), knob_actions);
                });
        } else {
            if crate::widgets::flat::flat_button(ui, t!("dock.open_params")).clicked() {
                *open_params = Some(DockDevice::Insert(slot));
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(t!("dock.plugin_hint"))
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_muted()),
            );
        }
    });

    clicked
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

/// 乐器大卡片（永远在最左）：XSynth 旋钮列表 / 插件参数入口；
/// 音频语境显示通道信息。参数拖动写 lane，与效果器卡片同一套机制。
#[allow(clippy::too_many_arguments)] // dock 卡片上下文透传，见 AGENTS 约定
fn instrument_card(
    ui: &mut egui::Ui,
    context: DockContext,
    instrument_plugin: Option<&str>,
    lane_current: &[DockParam],
    search: &mut String,
    inst_powered: bool,
    knob_actions: &mut Vec<KnobAction>,
    open_params: &mut Option<DockDevice>,
    toggle_instrument: &mut Option<bool>,
    open_gui: &mut Option<DockDevice>,
    open_instrument_picker: &mut Option<u8>,
    midi_channel: Option<u8>,
) {
    let has_instrument = midi_channel.is_some();
    let use_xsynth = has_instrument && instrument_plugin.is_none();
    egui::Frame::new()
        .fill(crate::theme::track_bg())
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());

            // ── 标题行：电源 + 名称 + 搜索 + 界面 + 换乐器 ──
            ui.horizontal(|ui| {
                if !has_instrument {
                    ui.label(
                        egui::RichText::new(context_label(context))
                            .size(crate::theme::SMALL_FONT + 2.0)
                            .color(crate::theme::text_primary()),
                    );
                    return;
                }

                let power = egui_material_icons::icons::ICON_POWER_SETTINGS_NEW;
                let power_color = if inst_powered {
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
                    // 目标 muted 值 = 当前是否开着（true→全 mute，false→全恢复）。
                    *toggle_instrument = Some(inst_powered);
                }
                power_resp.on_hover_text(t!("mix.bypass"));

                let name = instrument_plugin
                    .map(str::to_string)
                    .unwrap_or_else(|| "XSynth".to_string());
                ui.label(
                    egui::RichText::new(name)
                        .size(crate::theme::SMALL_FONT + 2.0)
                        .color(crate::theme::text_primary()),
                );

                // 搜索（仅内置 XSynth 的旋钮参数列表）。
                if use_xsynth {
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

                // 界面按钮：插件设备打开原生 GUI；XSynth 打开音色库配置窗口。
                let icon = if use_xsynth {
                    egui_material_icons::icons::ICON_LIBRARY_MUSIC
                } else {
                    egui_material_icons::icons::ICON_HOME_STORAGE
                };
                let resp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(icon.codepoint)
                            .font(egui::FontId::new(14.0, icon.font_family()))
                            .color(crate::theme::text_secondary()),
                    )
                    .frame(false),
                );
                if resp.clicked() {
                    *open_gui = Some(DockDevice::Instrument);
                }
                resp.on_hover_text(if use_xsynth {
                    t!("soundfont.title").to_string()
                } else {
                    t!("mix.toggle_gui").to_string()
                });

                // 插件乐器的参数面板入口。
                if !use_xsynth {
                    let params = egui_material_icons::icons::ICON_TUNE;
                    let presp = ui.add(
                        egui::Button::new(
                            egui::RichText::new(params.codepoint)
                                .font(egui::FontId::new(14.0, params.font_family()))
                                .color(crate::theme::text_secondary()),
                        )
                        .frame(false),
                    );
                    if presp.clicked() {
                        *open_params = Some(DockDevice::Instrument);
                    }
                    presp.on_hover_text(t!("dock.open_params"));
                }

                // 更换乐器：内置 XSynth 与 VST/CLAP 插件在同一个选择器里切换。
                let swap = egui_material_icons::icons::ICON_SWAP_HORIZ;
                let resp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(swap.codepoint)
                            .font(egui::FontId::new(14.0, swap.font_family()))
                            .color(crate::theme::text_secondary()),
                    )
                    .frame(false),
                );
                if resp.clicked() {
                    *open_instrument_picker = midi_channel;
                }
                resp.on_hover_text(t!("mix.pick_instrument"));
            });
            ui.add_space(6.0);

            // ── 内容 ──
            if use_xsynth {
                let needle = search.trim().to_lowercase();
                let filtered: Vec<DockParam> = lane_current
                    .iter()
                    .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle))
                    .cloned()
                    .collect();
                egui::ScrollArea::vertical()
                    .id_salt("xsynth_knobs")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        param_knobs(ui, &filtered, KnobOwner::Instrument, knob_actions);
                    });
            } else {
                // 插件乐器：参数面板入口 + 提示。
                if crate::widgets::flat::flat_button(ui, t!("dock.open_params")).clicked() {
                    *open_params = Some(DockDevice::Instrument);
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(t!("dock.plugin_hint"))
                        .size(crate::theme::SMALL_FONT)
                        .color(crate::theme::text_muted()),
                );
            }
        });
}

/// 参数纵向列表：每项「旋钮 + 右侧两行（名称、数值）」。
fn param_knobs(
    ui: &mut egui::Ui,
    values: &[DockParam],
    owner: KnobOwner,
    actions: &mut Vec<KnobAction>,
) {
    for param in values {
        knob_row(ui, param, owner, actions);
    }
}

/// 单个参数行：左旋钮 + 右两行（第一行名称、第二行数值）。
fn knob_row(ui: &mut egui::Ui, param: &DockParam, owner: KnobOwner, actions: &mut Vec<KnobAction>) {
    let max = param.target.max_value();
    let raw = param.current.unwrap_or(param.default);
    let mut norm = (raw / max.max(1.0)).clamp(0.0, 1.0);
    let target = &param.target;

    ui.horizontal(|ui| {
        let resp = crate::widgets::knob::knob(ui, &mut norm, 28.0);
        if resp.drag_started() {
            actions.push(KnobAction::DragStart(owner, target.clone()));
        }
        if resp.dragged() {
            actions.push(KnobAction::Drag(owner, target.clone(), norm));
        }
        if resp.drag_stopped() {
            actions.push(KnobAction::DragStop(owner, target.clone()));
        }

        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(&param.name)
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_primary()),
            );
            let (text, color) = match param.current {
                Some(v) => (format_value(v), crate::theme::accent_active()),
                None => (
                    format!("{}（默认）", format_value(param.default)),
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
fn apply_knob_action(
    app: &mut App,
    idx: usize,
    insert_target: yinhe_audio::InsertTarget,
    track_idx: usize,
    tick: u32,
    action: KnobAction,
) {
    match action {
        KnobAction::DragStart(_owner, target) => {
            // 效果器拖动走实时预览（不写模型）；这里只记录会话（松手才可能落 lane）。
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
                last_norm: 0.0,
                moved: false,
                track_idx,
                target,
                tick,
                snapshot: doc.capture_snapshot(),
                before,
                lane_idx: lane_pos,
            });
        }
        KnobAction::Drag(owner, target, norm) => {
            // 效果器：实时预览（参数队列，渲染线程下一块生效）——不写模型、不卡。
            if let KnobOwner::Insert(slot) = owner {
                if let Some(instance) = app.mixer_rack_mut(idx).instance_mut(insert_target, slot) {
                    let queue = instance.param_queue();
                    if let AutomationTarget::CC { controller } = target {
                        queue.push(controller as u32, norm as f64);
                    }
                }
                if let Some(drag) = app.knob_drag.as_mut()
                    && drag.target == target
                {
                    drag.last_norm = norm;
                    drag.tick = tick;
                    drag.moved = true;
                }
                return;
            }
            // 乐器：无实时预览通道，拖动中不写模型（松手落 lane，避免每帧重 flatten 卡顿）。
            let Some(mut drag) = app.knob_drag.take() else {
                return;
            };
            if drag.target == target {
                // 仅更新本地快照位置（松手时用），不写模型。
                drag.tick = tick;
                drag.last_norm = norm;
                drag.moved = true;
            }
            app.knob_drag = Some(drag);
        }
        KnobAction::DragStop(owner, target) => {
            let Some(mut drag) = app.knob_drag.take_if(|d| d.target == target) else {
                return;
            };
            // 写入开关：关（默认）时效果器只预览，不落 lane；乐器始终落 lane。
            let write =
                matches!(owner, KnobOwner::Instrument) || app.audio_settings.automation_write;
            if !write {
                return;
            }
            // 用松手时的最终值写一条事件（拖动过程只预览，避免每帧重 flatten）。
            let raw = drag.last_norm * drag.target.max_value();
            upsert_automation_event(app, idx, &mut drag, raw);
            app.notify_audio_model_changed();
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
    insert_target: yinhe_audio::InsertTarget,
    midi_channel: Option<u8>,
) -> Option<crate::mix::ParamPanel> {
    use crate::mix::param_panel::{ParamPanel, ParamTarget};
    match device {
        DockDevice::Instrument => {
            let ch = midi_channel?;
            let instance = app
                .instrument_racks
                .get_mut(idx)
                .and_then(|rack| rack.instance_mut(ch))?;
            let title = instance.name().to_string();
            Some(ParamPanel::open(
                ParamTarget::Instrument { channel: ch },
                title,
                instance,
            ))
        }
        DockDevice::Insert(slot) => {
            let instance = app
                .mixer_racks
                .get_mut(idx)
                .and_then(|rack| rack.instance_mut(insert_target, slot))?;
            let title = instance.name().to_string();
            Some(ParamPanel::open(
                ParamTarget::Insert {
                    target: insert_target,
                    slot,
                },
                title,
                instance,
            ))
        }
    }
}
