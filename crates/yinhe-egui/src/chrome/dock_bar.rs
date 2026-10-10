//! 三视图通用底部设备栏（Ableton 风格设备链）。
//!
//! 布局：上方设备链（乐器在最左 + 效果器链 + 「+」添加），下方选中设备的参数区。
//! - 普通 MIDI 轨的乐器位显示 **XSynth 虚拟乐器**，参数区是音源面板：通道
//!   处理参数（Volume/Expression/Pan/Cutoff/Resonance，通道处理段消费）在前，
//!   XSynth 设备参数（Sustain/Release/Pitch Bend/RPN 等）在后，拖动旋钮写
//!   对应 AM lane；
//! - 效果器链卡片（CLAP/VST3 插件）提供「参数面板」「插件界面」入口。
//!
//! 效果器链当前按**源 MIDI 通道**索引（与 MIX/引擎一致）；同通道多轨共享一条链。
//! 开关/高度持久化到 `LayoutSettings`；设备选中态不持久化。

use eframe::egui;
use rust_i18n::t;
use yinhe_core::TrackKind;
use yinhe_editor_core::document::Document;
use yinhe_types::automation::{CHANNEL_DSP_PARAMS, XSYNTH_PARAMS};
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

/// 单帧内旋钮产生的动作（渲染后统一应用，避开借用冲突）。
enum KnobAction {
    DragStart(AutomationTarget),
    /// 拖动到归一化值 `norm`。
    Drag(AutomationTarget, f32),
    DragStop(AutomationTarget),
}

/// 音源面板的自动化目标（通道处理参数在前、XSynth 音源参数在后）。
///
/// 通道处理（Volume/Expression/Pan/Cutoff/Resonance）已内置到音源的通道
/// 处理段，与 XSynth 的 CC 绑定参数一样走低层 `CC lane`（回放广播给通道
/// 处理段/合成器）；PB/RPN 无低层 lane 形态，保留设备参数
/// （`ChannelInstrument`）。返回 (target, 显示名)，显示名取参数表自己的
/// 名字（如 "Sustain"），不用 CC 目标的通用显示名（"CC 064 Sustain"）。
fn instrument_targets(channel: u8) -> Vec<(AutomationTarget, String, f32)> {
    CHANNEL_DSP_PARAMS
        .iter()
        .chain(XSYNTH_PARAMS)
        .map(|p| {
            (
                crate::piano_view::automation_panel::builtin_target(p, channel),
                p.name.to_string(),
                // 默认值取参数表（引擎无事件时的实际值，如 Volume/Expression
                // 满增益 1.0）；不能用 CC 目标的通用默认值（CC7 会得 0）。
                p.default,
            )
        })
        .collect()
}

/// 效果器链槽位在 dock 的展示数据。
pub(crate) struct DockInsert {
    name: String,
    bypassed: bool,
}

/// dock 旋钮的一项参数。
///
/// 显示名是**XSynth/通道处理自己的参数名**；`target` 是统一参数模型的目标：
/// CC 绑定参数走低层 CC lane（回放广播给通道处理段/合成器），PB/RPN 保留
/// 设备参数（`Param`）。
#[derive(Clone)]
pub(crate) struct DockParam {
    name: String,
    target: AutomationTarget,
    /// 无事件时的默认值（归一化 0..1）。
    default: f32,
    /// 光标处的当前值（None = 未设置，显示默认值；归一化 0..1）。
    current: Option<f32>,
}

/// 光标处应显示的值。
///
/// 顺序查该通道的**所有轨**：MIDI 导入会把自动化 lane 挂在独立的 "CC xx"
/// 轨上（不在音符轨），只看第一条轨会永远找不到 lane（旋钮卡在"默认"）。
/// 值复用 [`yinhe_types::AutomationLane::value_at`]（chase/预览/UI 共用）：
/// 二分查找 + 曲线段内实时插值，播放中旋钮随自动化平滑转动。
fn lane_current_value(
    model: &yinhe_core::YinModel,
    track_tis: &[usize],
    tick: u32,
    target: &AutomationTarget,
) -> Option<f32> {
    track_tis.iter().find_map(|&ti| {
        model
            .tracks
            .get(ti)
            .and_then(|t| t.automation_lanes.iter().find(|l| l.target == *target))
            .and_then(|l| l.value_at(tick).map(|(v, _)| v))
    })
}

/// 写入目标轨：优先已有该 target lane 的轨（保持导入的 "CC xx" 轨结构），
/// 都没有时用第一条轨（lane 懒创建）。
fn lane_write_track(
    model: &yinhe_core::YinModel,
    track_tis: &[usize],
    target: &AutomationTarget,
) -> usize {
    track_tis
        .iter()
        .copied()
        .find(|&ti| {
            model
                .tracks
                .get(ti)
                .is_some_and(|t| t.automation_lanes.iter().any(|l| l.target == *target))
        })
        .or_else(|| track_tis.first().copied())
        .unwrap_or(0)
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
            crate::widgets::hint::hover(ui.ctx(), &handle, t!("hint.dock.split"));

            // 内容区：等价原 frame `inner_margin(8, 4)`，顶部再让出分割线。
            let content = egui::Rect::from_min_max(
                egui::pos2(panel_rect.min.x + DOCK_PAD_X, handle_rect.max.y + 4.0),
                egui::pos2(panel_rect.max.x - DOCK_PAD_X, panel_rect.max.y - 4.0),
            );
            if ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| content.contains(p))
            {
                crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.dock"));
            }
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

/// dock 单帧快照：收集阶段读出、绘制阶段只读、应用阶段回查的数据。
struct DockState {
    context: DockContext,
    midi_channel: Option<u8>,
    insert_target: yinhe_audio::InsertTarget,
    instrument_plugin: Option<String>,
    inserts: Vec<DockInsert>,
    lane_track_tis: Vec<usize>,
    powered_tracks: Vec<usize>,
    inst_powered: bool,
    /// 写入位置（编辑光标 tick）。
    tick: u32,
    lane_current: Vec<DockParam>,
    track_names: Vec<String>,
    midi_active: Vec<u8>,
    audio_active: Vec<u16>,
    selected: Option<DockDevice>,
}

/// 单帧收集到的 dock 动作（绘制后按原顺序统一应用，避开借用冲突）。
enum DockAction {
    SetContext(DockContext),
    SetSelected(Option<DockDevice>),
    OpenInsertPicker,
    Knob(KnobAction),
    OpenParams(DockDevice),
    OpenInstrumentPicker(u8),
    ToggleInstrument(bool),
    ToggleBypass(usize, bool),
    OpenGui(DockDevice),
}

/// 设备链 + 参数区（收集 → 绘制 → 应用）。
fn show_body(app: &mut App, idx: usize, ui: &mut egui::Ui) {
    let Some(state) = collect_dock_state(app, idx) else {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(t!("dock.select_channel"))
                    .color(crate::theme::text_muted())
                    .size(crate::theme::SMALL_FONT),
            );
        });
        return;
    };
    let actions = draw_dock(ui, &state, &mut app.dock_param_search);
    apply_dock_actions(app, idx, &state, actions);
}

/// 收集阶段：同步选中轨语境，读出设备链/参数/通道选择数据。
fn collect_dock_state(app: &mut App, idx: usize) -> Option<DockState> {
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
    let context = app.dock_context?;

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
            .and_then(|v| v.first())
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
                    })
                    .collect()
            })
            .unwrap_or_default();
    // 语境归属轨（该通道上的轨道）。
    let belongs = |t: &std::sync::Arc<yinhe_core::TrackData>| match context {
        DockContext::Midi(ch) => t.global_channel() == ch,
        DockContext::Audio(ach) => t.audio_channel == Some(ach),
    };
    // lane 归属轨：该通道的所有轨（导入的自动化挂在独立 "CC xx" 轨上，
    // 不能用"第一条轨"定位）。
    let lane_track_ti: usize = model.tracks.iter().position(belongs).unwrap_or(0);
    // 归属轨索引与旁通状态：该通道所有轨道都 muted 视为旁通（任一未 mute = 开着）。
    let powered_tracks: Vec<usize> = model
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| belongs(t))
        .map(|(ti, _)| ti)
        .collect();
    let lane_track_tis: Vec<usize> = if powered_tracks.is_empty() {
        vec![lane_track_ti]
    } else {
        powered_tracks.clone()
    };
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
    // 显示值取值位置：播放中跟随播放头（自动化实时驱动旋钮），停止时跟随
    // 编辑光标。写入位置（`tick`）始终是编辑光标，避免拖动旋钮写到播放头处。
    let display_tick = app.workspace.documents[idx]
        .edit
        .playback
        .current_tick(&model)
        .map(|(t, _)| t.max(0.0) as u32)
        .unwrap_or(tick);
    let lane_current: Vec<DockParam> = midi_channel
        .map(instrument_targets)
        .unwrap_or_default()
        .into_iter()
        .map(|(target, name, default)| {
            let current = lane_current_value(&model, &lane_track_tis, display_tick, &target);
            DockParam {
                name,
                default,
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
    let selected: Option<DockDevice> = app
        .dock_selected
        .filter(|sel| valid(sel))
        .or(default_device);

    Some(DockState {
        context,
        midi_channel,
        insert_target,
        instrument_plugin,
        inserts,
        lane_track_tis,
        powered_tracks,
        inst_powered,
        tick,
        lane_current,
        track_names,
        midi_active,
        audio_active,
        selected,
    })
}

/// 绘制阶段：顶部通道选择 + 设备链卡片，返回本帧动作。
fn draw_dock(ui: &mut egui::Ui, state: &DockState, search: &mut String) -> Vec<DockAction> {
    let mut actions: Vec<DockAction> = Vec::new();
    let mut selected = state.selected;
    let mut knob_actions: Vec<KnobAction> = Vec::new();
    let mut open_params: Option<DockDevice> = None;
    let mut toggle_bypass: Option<(usize, bool)> = None;
    let mut toggle_instrument: Option<bool> = None;
    let mut open_gui: Option<DockDevice> = None;
    let mut open_instrument_picker: Option<u8> = None;
    let mut open_picker = false;

    // ── 顶部：通道选择 + 使用该通道的轨道 ──
    ui.horizontal(|ui| {
        let mut picked: Option<DockContext> = None;
        let ctx_resp = ui.menu_button(
            crate::widgets::icon_text::text_icon(
                &context_label(state.context),
                egui_material_icons::icons::ICON_ARROW_DROP_DOWN,
                crate::theme::SMALL_FONT + 1.0,
                crate::theme::text_primary(),
            ),
            |ui| {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for ch in &state.midi_active {
                            if ui
                                .selectable_label(
                                    state.context == DockContext::Midi(*ch),
                                    crate::mix::channel_label(*ch),
                                )
                                .clicked()
                            {
                                picked = Some(DockContext::Midi(*ch));
                                ui.close();
                            }
                        }
                        if !state.audio_active.is_empty() {
                            ui.separator();
                            for ach in &state.audio_active {
                                if ui
                                    .selectable_label(
                                        state.context == DockContext::Audio(*ach),
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
        crate::widgets::hint::hover(ui.ctx(), &ctx_resp.response, t!("hint.dock.context"));
        if let Some(ctx) = picked {
            actions.push(DockAction::SetContext(ctx));
        }
        if !state.track_names.is_empty() {
            let resp = ui.label(
                egui::RichText::new(state.track_names.join(", "))
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_muted()),
            );
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.dock.tracks"));
        }
    });

    // ── 主体：最左乐器大卡片（固定不随选中变化）+ 每个效果器一张大卡片 + 「+」──
    let avail = ui.available_size();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        const CARD_W: f32 = 208.0;
        const ADD_W: f32 = 44.0;

        // 乐器大卡片：永远在最左，固定显示乐器（XSynth 旋钮 / 插件入口）。
        let inst_area = ui
            .allocate_ui_with_layout(
                egui::vec2(CARD_W, avail.y),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    instrument_card(
                        ui,
                        state.context,
                        state.instrument_plugin.as_deref(),
                        &state.lane_current,
                        search,
                        state.inst_powered,
                        &mut knob_actions,
                        &mut open_params,
                        &mut toggle_instrument,
                        &mut open_gui,
                        &mut open_instrument_picker,
                        state.midi_channel,
                    );
                },
            )
            .response
            .rect;

        // 效果器大卡片（横向滚动）：插件效果器显示参数面板/原生界面入口。
        let fx_width = (avail.x - CARD_W - ADD_W - 20.0).max(120.0);
        let fx_area = ui
            .allocate_ui_with_layout(
                egui::vec2(fx_width, avail.y),
                egui::Layout::left_to_right(egui::Align::TOP),
                |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("dock_fx_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                for (slot, ins) in state.inserts.iter().enumerate() {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(CARD_W, avail.y),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            if effect_card(
                                                ui,
                                                slot,
                                                ins,
                                                selected,
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
            )
            .response
            .rect;

        // 「+」竖条：撑满高度，点击添加效果器。
        if add_column(ui, avail.y).clicked() {
            open_picker = true;
        }

        // 区域提示（低优先级）：乐器卡片 / 效果器链。
        if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
            if inst_area.contains(p) {
                crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.dock_instrument"));
            } else if fx_area.contains(p) {
                crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.dock_inserts"));
            }
        }
    });

    actions.push(DockAction::SetSelected(selected));
    if open_picker {
        actions.push(DockAction::OpenInsertPicker);
    }
    actions.extend(knob_actions.into_iter().map(DockAction::Knob));
    if let Some(device) = open_params {
        actions.push(DockAction::OpenParams(device));
    }
    if let Some(ch) = open_instrument_picker {
        actions.push(DockAction::OpenInstrumentPicker(ch));
    }
    if let Some(muted) = toggle_instrument {
        actions.push(DockAction::ToggleInstrument(muted));
    }
    if let Some((slot, bypassed)) = toggle_bypass {
        actions.push(DockAction::ToggleBypass(slot, bypassed));
    }
    if let Some(device) = open_gui {
        actions.push(DockAction::OpenGui(device));
    }
    actions
}

/// 应用阶段：按绘制收集的动作顺序统一写回 app。
fn apply_dock_actions(app: &mut App, idx: usize, state: &DockState, actions: Vec<DockAction>) {
    for action in actions {
        match action {
            DockAction::SetContext(ctx) => {
                app.dock_context = Some(ctx);
                app.dock_selected = None;
            }
            DockAction::SetSelected(selected) => app.dock_selected = selected,
            DockAction::OpenInsertPicker => {
                app.mix.picker_for = Some(state.insert_target);
            }
            DockAction::Knob(action) => {
                apply_knob_action(app, idx, &state.lane_track_tis, state.tick, action);
            }
            DockAction::OpenParams(device) => {
                match open_param_panel(app, idx, device, state.insert_target, state.midi_channel) {
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
            DockAction::OpenInstrumentPicker(ch) => app.mix.instrument_picker_for = Some(ch),
            DockAction::ToggleInstrument(muted) => {
                // 旁通 = 该通道所有轨道 mute（AR 读 track_overrides，自动同步）。
                let doc = &mut app.workspace.documents[idx];
                for &ti in &state.powered_tracks {
                    if let Some(ov) = doc.edit.track_overrides.get_mut(ti) {
                        ov.muted = muted;
                    }
                }
                let audio = app.audio_state.handle.as_ref();
                crate::right_panel::info_panel::send_skip_tracks(doc, audio);
            }
            DockAction::ToggleBypass(slot, bypassed) => {
                if let Some(r) = crate::mix::insert_refs(
                    &mut app.workspace.documents[idx].mixer,
                    state.insert_target,
                )
                .and_then(|chain| chain.get_mut(slot))
                {
                    r.bypassed = bypassed;
                }
                if let Some(rack) = app.mixer_racks.get_mut(idx) {
                    rack.set_bypass(state.insert_target, slot, bypassed);
                }
            }
            DockAction::OpenGui(device) => match device {
                DockDevice::Insert(slot) => {
                    #[cfg(target_os = "macos")]
                    if let Err(e) = app
                        .mixer_rack_mut(idx)
                        .toggle_gui(state.insert_target, slot)
                    {
                        app.mixer_rack_mut(idx).last_error = Some(e.0);
                    }
                    #[cfg(not(target_os = "macos"))]
                    let _ = slot;
                }
                // MIDI 通道乐器：挂插件 → 插件原生界面；默认 XSynth → 音色库配置窗口。
                DockDevice::Instrument => {
                    let Some(ch) = state.midi_channel else {
                        continue;
                    };
                    if state.instrument_plugin.is_some() {
                        let result = app
                            .instrument_racks
                            .get_mut(idx)
                            .map(|rack| rack.toggle_gui(ch, 0));
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
            },
        }
    }
}

/// 语境标签（MIDI-A01 / Audio-01）。
fn context_label(context: DockContext) -> String {
    match context {
        DockContext::Midi(ch) => crate::mix::channel_label(ch),
        DockContext::Audio(ach) => crate::mix::audio_label(ach),
    }
}

/// 无边框 Material 图标按钮（尺寸/颜色/提示可调）。
fn icon_button(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    size: f32,
    color: egui::Color32,
    hover: impl Into<egui::WidgetText>,
) -> egui::Response {
    let resp = ui.add(
        egui::Button::new(
            egui::RichText::new(icon.codepoint)
                .font(egui::FontId::new(size, icon.font_family()))
                .color(color),
        )
        .frame(false),
    );
    if resp.hovered() {
        crate::widgets::hint::set(ui.ctx(), hover.into().text().to_owned());
    }
    resp
}

/// 效果器大卡片：插件效果器显示参数面板 / 原生界面入口。
/// 返回 true = 卡片被点击（选中高亮）。
fn effect_card(
    ui: &mut egui::Ui,
    slot: usize,
    ins: &DockInsert,
    selected: Option<DockDevice>,
    open_params: &mut Option<DockDevice>,
    toggle_bypass: &mut Option<(usize, bool)>,
    open_gui: &mut Option<DockDevice>,
) -> bool {
    let is_selected = selected == Some(DockDevice::Insert(slot));
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
            let power_color = if ins.bypassed {
                crate::theme::text_muted()
            } else {
                crate::theme::accent_active()
            };
            if icon_button(
                ui,
                egui_material_icons::icons::ICON_POWER_SETTINGS_NEW,
                14.0,
                power_color,
                t!("hint.dock.insert_power"),
            )
            .clicked()
            {
                *toggle_bypass = Some((slot, !ins.bypassed));
            }

            let name_resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(&ins.name)
                        .size(crate::theme::SMALL_FONT + 2.0)
                        .color(crate::theme::text_primary()),
                )
                .sense(egui::Sense::click()),
            );
            crate::widgets::hint::hover(ui.ctx(), &name_resp, t!("hint.dock.insert_select"));
            if name_resp.clicked() {
                clicked = true;
            }

            if icon_button(
                ui,
                egui_material_icons::icons::ICON_TUNE,
                13.0,
                crate::theme::text_secondary(),
                t!("dock.open_params"),
            )
            .clicked()
            {
                *open_params = Some(DockDevice::Insert(slot));
            }

            if icon_button(
                ui,
                egui_material_icons::icons::ICON_HOME_STORAGE,
                13.0,
                crate::theme::text_secondary(),
                t!("mix.toggle_gui"),
            )
            .clicked()
            {
                *open_gui = Some(DockDevice::Insert(slot));
            }
        });
        ui.add_space(6.0);

        // ── 内容：参数面板入口 + 提示 ──
        if crate::widgets::flat::flat_button(ui, t!("dock.open_params")).clicked() {
            *open_params = Some(DockDevice::Insert(slot));
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t!("dock.plugin_hint"))
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_muted()),
        );
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
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("mix.add_insert_hint"));
    resp
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

                let power_color = if inst_powered {
                    crate::theme::accent_active()
                } else {
                    crate::theme::text_muted()
                };
                if icon_button(
                    ui,
                    egui_material_icons::icons::ICON_POWER_SETTINGS_NEW,
                    14.0,
                    power_color,
                    t!("hint.dock.instrument_power"),
                )
                .clicked()
                {
                    // 目标 muted 值 = 当前是否开着（true→全 mute，false→全恢复）。
                    *toggle_instrument = Some(inst_powered);
                }

                let name = instrument_plugin
                    .map(str::to_string)
                    .unwrap_or_else(|| "XSynth".to_string());
                let name_resp = ui.label(
                    egui::RichText::new(name)
                        .size(crate::theme::SMALL_FONT + 2.0)
                        .color(crate::theme::text_primary()),
                );
                crate::widgets::hint::hover(ui.ctx(), &name_resp, t!("hint.dock.instrument_name"));

                // 搜索（仅内置 XSynth 的旋钮参数列表）。
                if use_xsynth {
                    let search_resp = crate::widgets::text_input::control_text_input(
                        ui,
                        search,
                        88.0,
                        "mix_search",
                        Some(t!("mix.search").as_ref()),
                    );
                    crate::widgets::hint::hover(ui.ctx(), &search_resp, t!("hint.dock.search"));
                }

                // 界面按钮：插件设备打开原生 GUI；XSynth 打开音色库配置窗口。
                let (icon, hover) = if use_xsynth {
                    (
                        egui_material_icons::icons::ICON_LIBRARY_MUSIC,
                        t!("hint.dock.instrument_gui").to_string(),
                    )
                } else {
                    (
                        egui_material_icons::icons::ICON_HOME_STORAGE,
                        t!("hint.dock.instrument_gui").to_string(),
                    )
                };
                if icon_button(ui, icon, 14.0, crate::theme::text_secondary(), hover).clicked() {
                    *open_gui = Some(DockDevice::Instrument);
                }

                // 插件乐器的参数面板入口。
                if !use_xsynth
                    && icon_button(
                        ui,
                        egui_material_icons::icons::ICON_TUNE,
                        14.0,
                        crate::theme::text_secondary(),
                        t!("dock.open_params"),
                    )
                    .clicked()
                {
                    *open_params = Some(DockDevice::Instrument);
                }

                // 更换乐器：内置 XSynth 与 VST/CLAP 插件在同一个选择器里切换。
                if icon_button(
                    ui,
                    egui_material_icons::icons::ICON_SWAP_HORIZ,
                    14.0,
                    crate::theme::text_secondary(),
                    t!("hint.dock.instrument_picker"),
                )
                .clicked()
                {
                    *open_instrument_picker = midi_channel;
                }
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
                        param_knobs(ui, &filtered, knob_actions);
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
fn param_knobs(ui: &mut egui::Ui, values: &[DockParam], actions: &mut Vec<KnobAction>) {
    for param in values {
        knob_row(ui, param, actions);
    }
}

/// 单个参数行：左旋钮 + 右两行（第一行名称、第二行数值）。
fn knob_row(ui: &mut egui::Ui, param: &DockParam, actions: &mut Vec<KnobAction>) {
    // 统一参数模型：lane 存归一化值，旋钮直接用（显示换算只在文本处）。
    let mut norm = param.current.unwrap_or(param.default).clamp(0.0, 1.0);
    let target = &param.target;

    ui.horizontal(|ui| {
        let resp = crate::widgets::knob::knob(ui, &mut norm, 28.0);
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.dock.knob"));
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
                egui::RichText::new(&param.name)
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_primary()),
            );
            let (text, color) = match param.current {
                Some(v) => (
                    crate::piano_view::automation_panel::format_display_value(&param.target, v),
                    crate::theme::accent_active(),
                ),
                None => (
                    format!(
                        "{}（默认）",
                        crate::piano_view::automation_panel::format_display_value(
                            &param.target,
                            param.default,
                        )
                    ),
                    crate::theme::text_muted(),
                ),
            };
            ui.label(
                egui::RichText::new(text)
                    .size(crate::theme::SMALL_FONT)
                    .color(color),
            );
        });
    });
    ui.add_space(6.0);
}

/// 应用单帧旋钮动作：拖动中记录会话，松手写事件并 push 一条 undo。
fn apply_knob_action(
    app: &mut App,
    idx: usize,
    track_tis: &[usize],
    tick: u32,
    action: KnobAction,
) {
    match action {
        KnobAction::DragStart(target) => {
            // 这里只记录会话（松手才可能落 lane）。
            let doc = &mut app.workspace.documents[idx];
            if let Some(drag) = begin_knob_drag(doc, track_tis, target, tick) {
                app.knob_drag = Some(drag);
            }
        }
        KnobAction::Drag(target, norm) => {
            // 无实时预览通道，拖动中不写模型（松手落 lane，避免每帧重 flatten 卡顿）。
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
        KnobAction::DragStop(target) => {
            let Some(drag) = app.knob_drag.take_if(|d| d.target == target) else {
                return;
            };
            // 用松手时的最终值写一条事件（拖动过程不写模型，避免每帧重 flatten）。
            let doc = &mut app.workspace.documents[idx];
            finish_knob_drag(doc, drag);
            app.notify_audio_model_changed();
        }
    }
}

/// 开始旋钮拖动会话：记录目标轨与 lane 编辑前状态（undo 用）。
fn begin_knob_drag(
    doc: &mut Document,
    track_tis: &[usize],
    target: AutomationTarget,
    tick: u32,
) -> Option<KnobDrag> {
    // 目标轨：优先已有该 target lane 的轨（保持导入的 "CC xx" 轨结构）。
    let track_idx = lane_write_track(&doc.data.model, track_tis, &target);
    if track_idx >= doc.data.model.tracks.len() {
        return None;
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
    Some(KnobDrag {
        last_norm: 0.0,
        moved: false,
        track_idx,
        target,
        tick,
        snapshot: doc.capture_snapshot(),
        before,
        lane_idx: lane_pos,
    })
}

/// 结束拖动会话：按 `drag.last_norm` 写事件并 push 一条 undo。
fn finish_knob_drag(doc: &mut Document, mut drag: KnobDrag) {
    let norm = drag.last_norm;
    upsert_automation_event(doc, &mut drag, norm);
    let Some(lane_idx) = drag.lane_idx else {
        return;
    };
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

/// 在拖动会话的 tick 处写入/覆盖自动化事件（lane 懒创建）。
fn upsert_automation_event(doc: &mut Document, drag: &mut KnobDrag, norm: f32) {
    let lane_idx = doc.data.model.tracks[drag.track_idx]
        .automation_lanes
        .iter()
        .position(|l| l.target == drag.target);
    // 已有同 tick 事件时只更新其值（保留 id/shape），否则新增。
    let has_event = lane_idx.is_some_and(|li| {
        doc.data.model.tracks[drag.track_idx].automation_lanes[li]
            .events
            .iter()
            .any(|e| e.tick == drag.tick)
    });
    match lane_idx {
        Some(li) if has_event => {
            drag.lane_idx = Some(li);
            doc.move_automation_event(drag.track_idx, li, &drag.target, drag.tick, drag.tick, norm);
        }
        _ => {
            doc.add_automation_event(
                drag.track_idx,
                drag.target.clone(),
                AutomationEvent {
                    id: 0,
                    tick: drag.tick,
                    value: norm,
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
                .and_then(|rack| rack.instance_mut(ch, 0))?;
            let title = instance.name().to_string();
            let uid = app.workspace.documents[idx]
                .mixer
                .instruments
                .get(ch as usize)
                .and_then(|v| v.first())
                .map(|r| r.uid)
                .unwrap_or(0);
            Some(ParamPanel::open(
                ParamTarget::Instrument {
                    channel: ch,
                    index: 0,
                    uid,
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::automation::MidiBinding;

    /// 音源面板列表：通道处理参数（前 5）+ XSynth 参数（后 7）；显示名用参数表
    /// 名字（如 "Sustain"），CC 绑定参数走低层 CC lane，PB/RPN 保留设备参数。
    #[test]
    fn instrument_targets_merge_channel_dsp_and_xsynth() {
        let params = instrument_targets(4);
        assert_eq!(params.len(), CHANNEL_DSP_PARAMS.len() + XSYNTH_PARAMS.len());

        // 通道处理的 Volume/Expression 默认满增益（表默认 1.0），
        // 不能是 CC 通用默认值 0——否则面板无事件时显示 0。
        assert_eq!(params[0].2, 1.0, "Volume 默认应为满增益");
        assert_eq!(params[1].2, 1.0, "Expression 默认应为满增益");

        let names: Vec<&str> = params.iter().map(|(_, name, _)| name.as_str()).collect();
        let expect: Vec<&str> = CHANNEL_DSP_PARAMS
            .iter()
            .chain(XSYNTH_PARAMS)
            .map(|p| p.name)
            .collect();
        assert_eq!(names, expect, "顺序应为通道处理参数在前、XSynth 在后");

        let targets: Vec<AutomationTarget> =
            params.into_iter().map(|(target, _, _)| target).collect();
        // 通道处理参数全部是 CC 绑定（Volume=CC7 / Expression=CC11 / Pan=CC10 /
        // Cutoff=CC74 / Resonance=CC71）。
        for p in CHANNEL_DSP_PARAMS {
            let MidiBinding::Cc(cc) = p.midi else {
                panic!("通道处理参数应绑定 CC：{}", p.name);
            };
            assert!(
                targets.contains(&AutomationTarget::CC { controller: cc }),
                "CC{cc} 应列出"
            );
        }
        // XSynth：全部内置参数统一为低层 MIDI 目标（CC / PitchBend / RPN）。
        for cc in [64u8, 72, 73] {
            assert!(targets.contains(&AutomationTarget::CC { controller: cc }));
        }
        assert!(targets.contains(&AutomationTarget::PitchBend));
        for parameter in [0u16, 1, 2] {
            assert!(
                targets.contains(&AutomationTarget::Rpn { parameter }),
                "RPN{parameter} 应列出"
            );
        }
    }

    /// 拖动会话（空 lane）：松手写事件并 push undo，undo/redo 往返一致。
    #[test]
    fn knob_drag_writes_event_and_undo_roundtrip() {
        let mut doc = Document::empty();
        let target = AutomationTarget::CC { controller: 1 };
        let lane = |doc: &Document| -> Vec<AutomationEvent> {
            doc.data.model.tracks[0]
                .automation_lanes
                .iter()
                .find(|l| l.target == target)
                .map(|l| l.events.clone())
                .unwrap_or_default()
        };

        let mut drag = begin_knob_drag(&mut doc, &[0], target.clone(), 240).expect("目标轨存在");
        assert!(drag.lane_idx.is_none(), "空 lane 无索引");
        assert!(drag.before.is_empty(), "编辑前无事件");

        drag.last_norm = 0.75;
        drag.moved = true;
        finish_knob_drag(&mut doc, drag);

        let events = lane(&doc);
        assert_eq!(events.len(), 1, "lane 懒创建并写入事件");
        assert_eq!(events[0].tick, 240);
        assert_eq!(events[0].value, 0.75);
        assert!(doc.history.can_undo());

        assert!(doc.undo());
        assert!(lane(&doc).is_empty(), "undo 应移除新增事件");

        assert!(doc.redo());
        let events = lane(&doc);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].value, 0.75);
    }

    /// 拖动会话（已有同 tick 事件）：只更新值并保留 id，undo 恢复旧值。
    #[test]
    fn knob_drag_updates_existing_event_and_undo_restores() {
        let mut doc = Document::empty();
        let target = AutomationTarget::CC { controller: 7 };
        doc.add_automation_event(
            0,
            target.clone(),
            AutomationEvent {
                id: 0,
                tick: 100,
                value: 0.2,
                shape: target.default_shape(),
            },
        )
        .expect("lane 懒创建");
        let old_id = doc.data.model.tracks[0].automation_lanes[0].events[0].id;
        assert_ne!(old_id, 0, "新增事件应已发号");

        let mut drag = begin_knob_drag(&mut doc, &[0], target.clone(), 100).expect("目标轨存在");
        assert!(drag.lane_idx.is_some(), "已有 lane 应记录索引");
        assert_eq!(drag.before.len(), 1, "编辑前快照应含已有事件");

        drag.last_norm = 0.9;
        drag.moved = true;
        finish_knob_drag(&mut doc, drag);

        let events = doc.data.model.tracks[0].automation_lanes[0].events.clone();
        assert_eq!(events.len(), 1, "同 tick 只更新不新增");
        assert_eq!(events[0].value, 0.9);
        assert_eq!(events[0].id, old_id, "更新保留事件 id");

        assert!(doc.undo());
        let events = doc.data.model.tracks[0].automation_lanes[0].events.clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].value, 0.2, "undo 恢复旧值");
        assert_eq!(events[0].id, old_id);
    }
}
