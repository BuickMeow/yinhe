//! 三视图通用设备链（Ableton 风格设备链）。
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
use yinhe_types::{AutomationEvent, AutomationTarget};

use crate::app::App;

use cards::{add_column, effect_card, instrument_card, instrument_targets};
use knobs::apply_knob_action;

mod cards;
mod knobs;

#[cfg(test)]
mod tests;

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

/// 卡片标题行高度。收起态卡片宽度与它相同：折叠后就是标题条本身。
const CARD_TITLE_H: f32 = 24.0;
/// 收起状态下的卡片宽度（竖排标题条）。
const COLLAPSED_CARD_W: f32 = CARD_TITLE_H;
/// 折叠集合里表示「乐器卡片」的 key。
const INSTRUMENT_KEY: usize = usize::MAX;

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
    /// 是否居中归零（参数表 `center`：Pan/Cutoff/Resonance 等）。
    centered: bool,
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

/// dock 高度下限（与旧 Panel min_size 一致）。
const DOCK_MIN_H: f32 = 120.0;
/// dock 内容水平内边距（等价旧 frame `inner_margin(8, 4)` 的横向值）。
const DOCK_PAD_X: f32 = 8.0;

/// 每帧绘制设备链（mode_bar 之后、compute_layout 之前调用）。
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
    let panel_resp = egui::Panel::bottom("bottom_dock")
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

    // 点击设备链以外的任何地方（卷帘/走带/混音台/其它面板）→ 取消设备选中。
    clear_dock_selection_on_outside_click(app, ui, panel_resp.response.rect);
}

/// 浮动窗口形态：在给定矩形内渲染设备链（无分割条/无面板边框），
/// 供 `app::floating` 的独立窗口调用。
pub(crate) fn show_floating(app: &mut App, idx: usize, ui: &mut egui::Ui, rect: egui::Rect) {
    let content = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + DOCK_PAD_X, rect.min.y + 4.0),
        egui::pos2(rect.max.x - DOCK_PAD_X, rect.max.y - 4.0),
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
    clear_dock_selection_on_outside_click(app, ui, rect);
}

/// 点击 dock 外 → 清空设备选中（成熟 DAW 的通用行为：选中不跨区域残留）。
fn clear_dock_selection_on_outside_click(app: &mut App, ui: &egui::Ui, dock_rect: egui::Rect) {
    if app.dock_selected.is_none() {
        return;
    }
    let (clicked, pos) = ui.input(|i| (i.pointer.primary_clicked(), i.pointer.interact_pos()));
    if !clicked {
        return;
    }
    let Some(pos) = pos else {
        return;
    };
    if dock_rect.contains(pos) || crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return;
    }
    app.dock_selected = None;
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

/// 选中轨 → dock 通道语境。Master/Conductor 轨没有通道语境（返回 None）。
fn track_context(t: &yinhe_core::TrackData) -> Option<DockContext> {
    match t.kind {
        TrackKind::Audio => Some(DockContext::Audio(t.audio_channel.unwrap_or(0))),
        TrackKind::Midi => Some(DockContext::Midi(t.global_channel())),
        TrackKind::Master => None,
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
    selected: Option<DockDevice>,
}

/// 单帧收集到的 dock 动作（绘制后按原顺序统一应用，避开借用冲突）。
enum DockAction {
    SetSelected(Option<DockDevice>),
    OpenInsertPicker,
    Knob(KnobAction),
    OpenParams(DockDevice),
    OpenInstrumentPicker(u8),
    ToggleInstrument(bool),
    ToggleBypass(usize, bool),
    OpenGui(DockDevice),
    OpenSynthParams(u8),
    RemoveInsert(usize),
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
    apply_dock_actions(app, ui.ctx(), idx, &state, actions);
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
            .and_then(|t| track_context(t))
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
        .map(|(target, name, default, centered)| {
            let current = lane_current_value(&model, &lane_track_tis, display_tick, &target);
            DockParam {
                name,
                default,
                target,
                current,
                centered,
            }
        })
        .collect();
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
    let mut remove_insert: Option<usize> = None;
    let mut open_instrument_picker: Option<u8> = None;
    let mut open_synth_params: Option<u8> = None;
    let mut open_picker = false;

    // ── 主体：最左乐器卡片 + 效果器链（+ 紧贴最后一个设备）──
    let mut inst_area = egui::Rect::NOTHING;
    let mut fx_area = egui::Rect::NOTHING;
    const CARD_W: f32 = 208.0;

    let collapse_id = ui.scope_id().with("dock_collapsed_cards");
    let mut collapsed: std::collections::HashSet<usize> =
        ui.data_mut(|d| d.get_temp(collapse_id)).unwrap_or_default();

    let inst_w = if collapsed.contains(&INSTRUMENT_KEY) {
        COLLAPSED_CARD_W
    } else {
        CARD_W
    };
    crate::widgets::grid::two_columns(ui, 6.0, inst_w, |cells| {
        // 乐器卡片：永远在最左，固定显示乐器（XSynth 旋钮 / 插件入口）。
        cells.cell(|ui| {
            inst_area = ui.max_rect();
            let clicked = instrument_card(
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
                &mut open_synth_params,
                state.midi_channel,
                &mut collapsed,
                selected == Some(DockDevice::Instrument),
            );
            if clicked {
                selected = Some(DockDevice::Instrument);
            }
        });

        // 效果器链（横向滚动）。
        cells.cell(|ui| {
            fx_area = ui.max_rect();
            egui::ScrollArea::horizontal()
                .id_salt("dock_fx_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for (slot, ins) in state.inserts.iter().enumerate() {
                            let w = if collapsed.contains(&slot) {
                                COLLAPSED_CARD_W
                            } else {
                                CARD_W
                            };
                            ui.allocate_ui_with_layout(
                                egui::vec2(w, ui.available_height()),
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
                                        &mut remove_insert,
                                        &mut collapsed,
                                    ) {
                                        selected = Some(DockDevice::Insert(slot));
                                    }
                                },
                            );
                        }
                        // 「+」紧贴最后一个设备。
                        if add_column(ui, ui.available_height()).clicked() {
                            open_picker = true;
                        }
                    });
                });
        });
    });
    ui.data_mut(|d| d.insert_temp(collapse_id, collapsed));

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
    if let Some(ch) = open_synth_params {
        actions.push(DockAction::OpenSynthParams(ch));
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
    if let Some(slot) = remove_insert {
        actions.push(DockAction::RemoveInsert(slot));
    }
    actions
}

/// 应用阶段：按绘制收集的动作顺序统一写回 app。
///
/// 弹窗类动作在**打开动作处**调 `raise_viewport`：窗口已存在则拉回前台
/// （只靠渲染处的 `raise_on_open` 做不到——窗口一直声明着，不会重建）。
fn apply_dock_actions(
    app: &mut App,
    ctx: &egui::Context,
    idx: usize,
    state: &DockState,
    actions: Vec<DockAction>,
) {
    for action in actions {
        match action {
            DockAction::SetSelected(selected) => app.dock_selected = selected,
            DockAction::OpenInsertPicker => {
                app.mix.picker_for = Some(state.insert_target);
                crate::chrome::dialog::raise_viewport(ctx, crate::mix::picker_viewport_id());
            }
            DockAction::Knob(action) => {
                apply_knob_action(app, idx, &state.lane_track_tis, state.tick, action);
            }
            DockAction::OpenParams(device) => {
                match open_param_panel(app, idx, device, state.insert_target, state.midi_channel) {
                    Some(panel) => {
                        app.mix.param_panel = Some(panel);
                        crate::chrome::dialog::raise_viewport(
                            ctx,
                            crate::mix::param_panel::viewport_id(),
                        );
                    }
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
            DockAction::OpenInstrumentPicker(ch) => {
                app.mix.instrument_picker_for = Some(ch);
                crate::chrome::dialog::raise_viewport(ctx, crate::mix::picker_viewport_id());
            }
            DockAction::OpenSynthParams(ch) => {
                app.mix.synth_params_panel =
                    Some(crate::dialogs::instrument_params::InstrumentParamsPanel::new(ch));
                crate::chrome::dialog::raise_viewport(
                    ctx,
                    crate::dialogs::instrument_params::viewport_id(),
                );
            }
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
            DockAction::RemoveInsert(slot) => {
                if let Some(chain) = crate::mix::insert_refs(
                    &mut app.workspace.documents[idx].mixer,
                    state.insert_target,
                ) && slot < chain.len()
                {
                    chain.remove(slot);
                }
                let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
                if idx < app.mixer_racks.len() {
                    app.mixer_racks[idx].remove_slot(state.insert_target, slot, handle);
                }
                app.dock_selected = None;
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
                        crate::chrome::dialog::raise_viewport(
                            ctx,
                            crate::dialogs::xsynth_config::viewport_id(ch),
                        );
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
