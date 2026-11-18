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
use yinhe_types::ChannelId;
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

/// dock 的通道语境 = 混音通道身份。
///
/// MIDI 通道同时是乐器（内置音源或插件）的挂载点；音频通道、每轨 LC、
/// Master 各自独立。`DockContext` 与 `InsertTarget` 都是 `ChannelId`，
/// 不再维护多套枚举转换。
pub(crate) type DockContext = yinhe_types::ChannelId;

/// 选中轨 → dock 通道语境。Master/Conductor 轨映射到主输出效果链。
fn track_context(
    doc: &yinhe_editor_core::document::Document,
    track_idx: usize,
) -> Option<DockContext> {
    doc.track_channel_id(track_idx)
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

/// 语境标签（AP / 音频/每轨 LC / Master）。
pub(crate) fn context_label(context: DockContext) -> String {
    match context {
        ChannelId::Ap(ap) => crate::mix::channel_label(ap.group.0 as u8),
        ChannelId::Lc(lc) => format!("LC {}", lc.0),
        ChannelId::Master => t!("mix.master").to_string(),
    }
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
    let context = sync_dock_context(app, idx)?;

    // ── 收集链数据（后续 UI 不再借 workspace）──
    let model = app.workspace.documents[idx].data.model.clone();
    // 语境 → 相关通道（MIDI / 音频两套命名空间各一条链）。
    let midi_channel = match context {
        ChannelId::Ap(ap) => Some(ap.group.0 as u8),
        ChannelId::Lc(lc) => match app.workspace.documents[idx]
            .mixer
            .source_of(ChannelId::Lc(lc))
        {
            Some(yinhe_types::ChannelSource::Track(t)) => app.workspace.documents[idx]
                .data
                .model
                .tracks
                .get(t as usize)
                .map(|t| t.global_channel()),
            _ => None,
        },
        ChannelId::Master => None,
    };
    let insert_target: yinhe_audio::InsertTarget = context;
    // 该 MIDI 通道挂载的插件乐器名（None = 默认内置 XSynth）。
    let instrument_plugin: Option<String> = midi_channel.and_then(|ch| {
        app.workspace.documents[idx]
            .mixer
            .racks
            .get(ch as usize)
            .and_then(|rack| rack.instruments.first())
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
    let context_tracks: Vec<usize> = (0..model.tracks.len())
        .filter(|&ti| app.workspace.documents[idx].track_channel_id(ti) == Some(context))
        .collect();
    // lane 归属轨：该通道的所有轨（导入的自动化挂在独立 "CC xx" 轨上，
    // 不能用"第一条轨"定位）。
    let lane_track_ti: usize = context_tracks.first().copied().unwrap_or(0);
    let powered_tracks: Vec<usize> = context_tracks;
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
    let lane_current = collect_dock_params(&model, midi_channel, &lane_track_tis, display_tick);
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

/// 选中轨变化时同步 dock 语境；返回当前语境。
fn sync_dock_context(app: &mut App, idx: usize) -> Option<DockContext> {
    let selected_track = {
        let doc = &app.workspace.documents[idx];
        doc.edit.track_selected.iter().min().copied()
    };
    if selected_track != app.dock_track {
        // 切换选中轨：dock 跟随该轨所在通道（音频轨→音频通道，乐器轨→乐器通道），
        // 并重置设备选中。
        app.dock_track = selected_track;
        if let Some(context) =
            selected_track.and_then(|ti| track_context(&app.workspace.documents[idx], ti as usize))
        {
            app.dock_context = Some(context);
            app.dock_selected = None;
        }
    }
    app.dock_context
}

/// 各内置乐器参数在当前 tick 的显示值。
fn collect_dock_params(
    model: &yinhe_core::YinModel,
    midi_channel: Option<u8>,
    lane_track_tis: &[usize],
    display_tick: u32,
) -> Vec<DockParam> {
    midi_channel
        .map(instrument_targets)
        .unwrap_or_default()
        .into_iter()
        .map(|(target, name, default, centered)| {
            let current = lane_current_value(model, lane_track_tis, display_tick, &target);
            DockParam {
                name,
                default,
                target,
                current,
                centered,
            }
        })
        .collect()
}

/// 绘制阶段：顶部通道选择 + 设备链卡片，返回本帧动作。
fn draw_dock(ui: &mut egui::Ui, state: &DockState, search: &mut String) -> Vec<DockAction> {
    let mut draw = DockDraw {
        selected: state.selected,
        ..DockDraw::default()
    };
    let collapse_id = ui.scope_id().with("dock_collapsed_cards");
    let mut collapsed: std::collections::HashSet<usize> =
        ui.data_mut(|d| d.get_temp(collapse_id)).unwrap_or_default();

    const CARD_W: f32 = 208.0;
    let inst_w = if collapsed.contains(&INSTRUMENT_KEY) {
        COLLAPSED_CARD_W
    } else {
        CARD_W
    };
    crate::widgets::grid::two_columns(ui, 6.0, inst_w, |cells| {
        cells.cell(|ui| {
            draw_instrument_card(ui, state, search, &mut collapsed, &mut draw);
        });
        cells.cell(|ui| {
            draw_effect_chain(ui, state, &mut collapsed, &mut draw);
        });
    });
    ui.data_mut(|d| d.insert_temp(collapse_id, collapsed));

    draw.into_actions()
}

/// 设备链绘制期间收集的动作。
#[derive(Default)]
struct DockDraw {
    selected: Option<DockDevice>,
    knob_actions: Vec<KnobAction>,
    open_params: Option<DockDevice>,
    toggle_bypass: Option<(usize, bool)>,
    toggle_instrument: Option<bool>,
    open_gui: Option<DockDevice>,
    remove_insert: Option<usize>,
    open_instrument_picker: Option<u8>,
    open_synth_params: Option<u8>,
    open_picker: bool,
}

impl DockDraw {
    fn into_actions(self) -> Vec<DockAction> {
        let mut actions = Vec::new();
        actions.push(DockAction::SetSelected(self.selected));
        if self.open_picker {
            actions.push(DockAction::OpenInsertPicker);
        }
        actions.extend(self.knob_actions.into_iter().map(DockAction::Knob));
        if let Some(device) = self.open_params {
            actions.push(DockAction::OpenParams(device));
        }
        if let Some(ch) = self.open_instrument_picker {
            actions.push(DockAction::OpenInstrumentPicker(ch));
        }
        if let Some(ch) = self.open_synth_params {
            actions.push(DockAction::OpenSynthParams(ch));
        }
        if let Some(muted) = self.toggle_instrument {
            actions.push(DockAction::ToggleInstrument(muted));
        }
        if let Some((slot, bypassed)) = self.toggle_bypass {
            actions.push(DockAction::ToggleBypass(slot, bypassed));
        }
        if let Some(device) = self.open_gui {
            actions.push(DockAction::OpenGui(device));
        }
        if let Some(slot) = self.remove_insert {
            actions.push(DockAction::RemoveInsert(slot));
        }
        actions
    }
}

/// 最左乐器卡片（XSynth 旋钮 / 插件入口）。
fn draw_instrument_card(
    ui: &mut egui::Ui,
    state: &DockState,
    search: &mut String,
    collapsed: &mut std::collections::HashSet<usize>,
    draw: &mut DockDraw,
) {
    let clicked = instrument_card(
        ui,
        state.context,
        state.instrument_plugin.as_deref(),
        &state.lane_current,
        search,
        state.inst_powered,
        &mut draw.knob_actions,
        &mut draw.open_params,
        &mut draw.toggle_instrument,
        &mut draw.open_gui,
        &mut draw.open_instrument_picker,
        &mut draw.open_synth_params,
        state.midi_channel,
        collapsed,
        draw.selected == Some(DockDevice::Instrument),
    );
    if clicked {
        draw.selected = Some(DockDevice::Instrument);
    }
}

/// 效果器链（横向滚动）+ 末尾「+」。
fn draw_effect_chain(
    ui: &mut egui::Ui,
    state: &DockState,
    collapsed: &mut std::collections::HashSet<usize>,
    draw: &mut DockDraw,
) {
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
                        208.0
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(w, ui.available_height()),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            if effect_card(
                                ui,
                                slot,
                                ins,
                                draw.selected,
                                &mut draw.open_params,
                                &mut draw.toggle_bypass,
                                &mut draw.open_gui,
                                &mut draw.remove_insert,
                                collapsed,
                            ) {
                                draw.selected = Some(DockDevice::Insert(slot));
                            }
                        },
                    );
                }
                // 「+」紧贴最后一个设备。
                if add_column(ui, ui.available_height()).clicked() {
                    draw.open_picker = true;
                }
            });
        });
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
                open_dock_params(app, ctx, idx, state, device);
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
            DockAction::ToggleInstrument(muted) => toggle_dock_instrument(app, idx, state, muted),
            DockAction::ToggleBypass(slot, bypassed) => {
                toggle_dock_bypass(app, idx, state, slot, bypassed);
            }
            DockAction::RemoveInsert(slot) => remove_dock_insert(app, idx, state, slot),
            DockAction::OpenGui(device) => open_dock_gui(app, ctx, idx, state, device),
        }
    }
}

/// 打开设备参数面板；实例不可用时在 MIX 状态行提示。
fn open_dock_params(
    app: &mut App,
    ctx: &egui::Context,
    idx: usize,
    state: &DockState,
    device: DockDevice,
) {
    match open_param_panel(app, idx, device, state.insert_target, state.midi_channel) {
        Some(panel) => {
            app.mix.param_panel = Some(panel);
            crate::chrome::dialog::raise_viewport(ctx, crate::mix::param_panel::viewport_id());
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

/// 旁通 = 该通道所有轨道 mute（AR 读 track_overrides，自动同步）。
fn toggle_dock_instrument(app: &mut App, idx: usize, state: &DockState, muted: bool) {
    let doc = &mut app.workspace.documents[idx];
    for &ti in &state.powered_tracks {
        if let Some(ov) = doc.edit.track_overrides.get_mut(ti) {
            ov.muted = muted;
        }
    }
    let audio = app.audio_state.handle.as_ref();
    crate::right_panel::info_panel::send_skip_tracks(doc, audio);
}

/// 切换 insert 旁通（持久化 + 机架实例）。
fn toggle_dock_bypass(app: &mut App, idx: usize, state: &DockState, slot: usize, bypassed: bool) {
    if let Some(r) =
        crate::mix::insert_refs(&mut app.workspace.documents[idx].mixer, state.insert_target)
            .and_then(|chain| chain.get_mut(slot))
    {
        r.bypassed = bypassed;
    }
    if let Some(rack) = app.mixer_racks.get_mut(idx) {
        rack.set_bypass(state.insert_target, slot, bypassed);
    }
}

/// 移除 insert（持久化引用 + 机架实例回收）。
fn remove_dock_insert(app: &mut App, idx: usize, state: &DockState, slot: usize) {
    if let Some(chain) =
        crate::mix::insert_refs(&mut app.workspace.documents[idx].mixer, state.insert_target)
        && slot < chain.len()
    {
        chain.remove(slot);
    }
    app.workspace.documents[idx].data.bump_revision();
    let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
    if idx < app.mixer_racks.len() {
        app.mixer_racks[idx].remove_slot(state.insert_target, slot, handle);
    }
    app.dock_selected = None;
}

/// 打开插件原生界面（instrument / insert；内置音源打开音色库配置）。
fn open_dock_gui(
    app: &mut App,
    ctx: &egui::Context,
    idx: usize,
    state: &DockState,
    device: DockDevice,
) {
    match device {
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
                return;
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
                app.mix.synth_config_for = Some(ch);
                crate::chrome::dialog::raise_viewport(
                    ctx,
                    crate::dialogs::synth_config::viewport_id(ch),
                );
            }
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
                .racks
                .get(ch as usize)
                .and_then(|rack| rack.instruments.first())
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
