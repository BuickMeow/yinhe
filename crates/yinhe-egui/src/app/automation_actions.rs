//! 自动化锚点的复制/粘贴/重复/删除/变速/翻转操作。
//!
//! 当 Select/SelectVertical 工具激活且自动化锚点被选中时，
//! copy/cut/duplicate/delete/变速/水平翻转作用于锚点而非音符。
//! paste 由 `App::paste_clipboard` 按剪贴板内容类型分派。
//!
//! 选中锚点来源：PR 底部自动化面板（`controller_panels`）与
//! AR 展开的自动化 lane（`arr_am_views`）统一处理。

use rust_i18n::t;
use yinhe_editor_core::clipboard::{AutomationClipboard, AutomationSelection, PasteMode};
use yinhe_types::{
    AnchorSelRect, AutomationEdit, AutomationEvent, AutomationPanelView, AutomationTarget,
    SegmentShape,
};

use crate::app::App;
use crate::widgets::tools_panel::Tool;

/// 选中锚点所属面板：PR 底部面板（`controller_panels` 下标）或
/// AR lane（`arr_am_views` 键 `(track, target)`）。
#[derive(Clone, Debug, PartialEq)]
enum PanelRef {
    Pr(usize),
    Ar(u16, AutomationTarget),
}

/// 根据锚点列表计算 sel_rect（用于 paste/duplicate 后设置选中范围）。
/// `vertical` = true 时 value_range 为 None（垂直全选），否则取 value 的 min/max。
fn sel_rect_from_anchors(anchors: &[(u32, f32)], vertical: bool) -> Option<AnchorSelRect> {
    if anchors.is_empty() {
        return None;
    }
    let tick_start = anchors
        .iter()
        .map(|(t, _)| *t as f64)
        .fold(f64::INFINITY, f64::min);
    let tick_end = anchors
        .iter()
        .map(|(t, _)| *t as f64)
        .fold(f64::NEG_INFINITY, f64::max);
    let value_range = if vertical {
        None
    } else {
        let vmin = anchors
            .iter()
            .map(|(_, v)| *v)
            .fold(f32::INFINITY, f32::min);
        let vmax = anchors
            .iter()
            .map(|(_, v)| *v)
            .fold(f32::NEG_INFINITY, f32::max);
        Some((vmin, vmax))
    };
    Some(AnchorSelRect {
        tick_start,
        tick_end,
        value_range,
    })
}

/// `(id, tick, value, shape)` 的锚点快照。
type AnchorSnap = (u32, u32, f32, SegmentShape);

/// 重复操作的结果：`(面板, target, 新锚点 (tick, value))`。
type DupResult = (PanelRef, AutomationTarget, Vec<(u32, f32)>);

/// 变速/翻转的逐面板移动计划：
/// `(面板, track_idx, lane_idx, target, t0, t1_or_factor, moves)`。
/// 变速时第 6 项为 factor（t0 为缩放原点）；翻转时第 6 项为 t1（t0..t1 镜像）。
type MovePlan = (
    PanelRef,
    u16,
    usize,
    AutomationTarget,
    f64,
    f64,
    Vec<(u32, u32, f32)>,
);

/// 从 Document 读取的信息包：复制/删除/重复/变速/翻转共用。
struct AnchorCtx {
    /// 所属面板（PR 下标或 AR 键）。
    panel: PanelRef,
    /// 锚点所属 target。
    target: AutomationTarget,
    /// 锚点所属 track_idx（用于 AutomationEdit）。
    track_idx: u16,
    /// lane 在 tracks[track].automation_lanes 中的索引（Tempo 用 0）。
    lane_idx: usize,
    /// 选中锚点快照 `(id, tick, value, shape)`。
    /// 成员态按 id 判定，矩形态回退选框几何（`panel.accepts_anchor`）。
    selected: Vec<AnchorSnap>,
}

/// 从 `apply_automation_edits` 返回的动作里按 target 汇总新增事件 id
/// （Add 的 after；用于粘贴/重复后让选区跟随副本）。
fn added_ids_by_target(
    actions: &[yinhe_editor_core::UndoAction],
) -> std::collections::HashMap<AutomationTarget, Vec<u32>> {
    let mut map: std::collections::HashMap<AutomationTarget, Vec<u32>> =
        std::collections::HashMap::new();
    for action in actions {
        if let yinhe_editor_core::UndoAction::Automation(delta) = action
            && delta.before.is_empty()
        {
            map.entry(delta.target.clone())
                .or_default()
                .extend(delta.after.iter().map(|e| e.id));
        }
    }
    map
}

/// 把 `&mut AutomationPanelView` 的 anchor_sel_rects 的 tick 范围相对 `t0`
/// 等比缩放（value_range 不动）。PR/AR 共用。
fn scale_anchor_rects(panel: &mut AutomationPanelView, t0: f64, factor: f64) {
    for r in &mut panel.anchor_sel_rects {
        let ts = r.tick_start.min(r.tick_end);
        let te = r.tick_start.max(r.tick_end);
        let nts = (t0 + (ts - t0) * factor).round();
        let nte = (t0 + (te - t0) * factor).round().max(nts + 1.0);
        r.tick_start = nts;
        r.tick_end = nte;
    }
}

/// 在 `[t0, t1]` 内镜像 anchor_sel_rects 的 tick 范围（value_range 不动）。
fn mirror_anchor_rects(panel: &mut AutomationPanelView, t0: f64, t1: f64) {
    for r in &mut panel.anchor_sel_rects {
        let ts = r.tick_start.min(r.tick_end);
        let te = r.tick_start.max(r.tick_end);
        r.tick_start = t0 + (t1 - te);
        r.tick_end = t0 + (t1 - ts);
    }
}

/// 选区（anchor_sel_rects）的 tick 跨度 `[t0, t1]`；无有效跨度返回 None。
fn anchor_span(panel: &AutomationPanelView) -> Option<(f64, f64)> {
    if panel.anchor_sel_rects.is_empty() {
        return None;
    }
    let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
    for r in &panel.anchor_sel_rects {
        t0 = t0.min(r.tick_start.min(r.tick_end));
        t1 = t1.max(r.tick_start.max(r.tick_end));
    }
    (t1 > t0).then_some((t0, t1))
}

impl App {
    /// 是否有任意面板选中了自动化锚点（用于快捷键/按钮路由）。
    ///
    /// 仅 Select/SelectVertical 工具下返回 true。
    /// 此时 copy/cut/duplicate/delete/变速/水平翻转作用于锚点而非音符。
    /// 覆盖 PR 底部面板与 AR 自动化 lane。
    pub(crate) fn has_selected_automation_anchors(&self) -> bool {
        let Some(idx) = self.workspace.active_doc else {
            return false;
        };
        if !matches!(self.active_tool, Tool::Select | Tool::SelectVertical) {
            return false;
        }
        let edit = &self.workspace.documents[idx].edit;
        edit.controller_panels
            .iter()
            .any(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
            || edit
                .arr_am_views
                .values()
                .any(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
    }

    /// 复制所有有选中锚点的面板到剪贴板。
    ///
    /// 与音符剪贴板同构：只存「选择范围 + 轨道/Conductor 结构共享快照」，
    /// 复制 O(1)，源之后被改/删不影响粘贴内容。
    pub(crate) fn copy_automation_anchors(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &self.workspace.documents[idx];

        let mut selections: Vec<AutomationSelection> = doc
            .edit
            .controller_panels
            .iter()
            .filter(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
            .map(|p| AutomationSelection {
                target: p.selected_target.clone(),
                sel_rects: p.anchor_sel_rects.clone(),
                members: p.anchor_members.clone(),
            })
            .collect();
        selections.extend(
            doc.edit
                .arr_am_views
                .values()
                .filter(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
                .map(|p| AutomationSelection {
                    target: p.selected_target.clone(),
                    sel_rects: p.anchor_sel_rects.clone(),
                    members: p.anchor_members.clone(),
                }),
        );
        if selections.is_empty() {
            return;
        }

        self.clipboard =
            yinhe_editor_core::ClipboardContent::Automation(AutomationClipboard::from_snapshot(
                doc.data.model.tracks.clone(),
                doc.data.model.conductor.clone(),
                selections,
            ));
        self.paste_chain = None;
        self.export_clipboard_to_system();
    }

    /// 剪切选中锚点：复制到剪贴板后删除。
    pub(crate) fn cut_automation_anchors(&mut self) {
        self.copy_automation_anchors();
        self.delete_automation_anchors();
    }

    /// 粘贴自动化剪贴板内容（逐 target 粘贴，一次 undo）。
    ///
    /// 返回粘贴内容的 tick 跨度（多 clip 取最大），供连续粘贴递增；
    /// 没有实际粘贴时返回 None。
    pub(crate) fn paste_automation_clipboard(
        &mut self,
        clipboard: &AutomationClipboard,
        mode: PasteMode,
    ) -> Option<u32> {
        let idx = self.workspace.active_doc?;
        // 物化复制内容（快照按选择范围查询 / 跨实例数据直接使用）。
        let clips = clipboard.collect();
        if clips.is_empty() {
            return None;
        }
        let doc = &mut self.workspace.documents[idx];

        let cursor_tick = doc.edit.cursor_tick.unwrap_or(0.0);
        let vertical = self.active_tool == Tool::SelectVertical;

        let mut edits = Vec::new();
        let mut panel_anchors: Vec<(usize, Vec<(u32, f32)>)> = Vec::new();
        let mut max_span = 0u32;
        for clip in &clips {
            // 找 target 匹配的面板
            let Some(panel_idx) = doc
                .edit
                .controller_panels
                .iter()
                .position(|p| !p.show_velocity && p.selected_target == clip.target)
            else {
                continue;
            };
            let Some(track_idx) = Self::track_idx_for(doc, &clip.target) else {
                continue;
            };
            let min_tick = clip.events.iter().map(|(t, _, _)| *t).min().unwrap_or(0);
            let max_tick = clip.events.iter().map(|(t, _, _)| *t).max().unwrap_or(0);
            let span = max_tick.saturating_sub(min_tick);
            max_span = max_span.max(span);
            let mut new_anchors = Vec::with_capacity(clip.events.len());
            for (tick, value, shape) in &clip.events {
                let new_tick = match mode {
                    PasteMode::AtOriginal => *tick,
                    PasteMode::AtCursor => {
                        (cursor_tick as i64 + (*tick as i64 - min_tick as i64)).max(0) as u32
                    }
                    PasteMode::Flipped => {
                        // 时间镜像：新 tick = 光标 + (跨度 - (源 tick - 源最早 tick))
                        (cursor_tick as i64 + span as i64 - (*tick as i64 - min_tick as i64)).max(0)
                            as u32
                    }
                };
                edits.push(yinhe_types::AutomationEdit::Add {
                    track_idx,
                    target: clip.target.clone(),
                    tick: new_tick,
                    value: *value,
                    shape: *shape,
                });
                new_anchors.push((new_tick, *value));
            }
            // 镜像后 tick 顺序反转；选区范围与展示要求升序。
            if mode == PasteMode::Flipped {
                new_anchors.sort_by_key(|(t, _)| *t);
            }
            panel_anchors.push((panel_idx, new_anchors));
        }

        if edits.is_empty() {
            return None;
        }

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if actions.is_empty() {
            return None;
        }
        let added = added_ids_by_target(&actions);
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.paste_automation").as_ref(),
            before,
        );
        // 粘贴后选中改为根据新锚点范围设置 sel_rect（音符粘贴的选区跟随对齐）
        for (panel_idx, anchors) in &panel_anchors {
            let panel = &mut doc.edit.controller_panels[*panel_idx];
            panel.anchor_sel_rects = sel_rect_from_anchors(anchors, vertical)
                .map(|r| vec![r])
                .unwrap_or_default();
            // 选区精确跟随副本（新 id），落点已有锚点不纳入。
            if let Some(ids) = added.get(&panel.selected_target) {
                panel.set_anchor_members(ids.iter().copied());
            }
            panel.dirty = true;
        }
        self.notify_audio_model_changed();
        Some(max_span)
    }

    /// 重复选中锚点（Cmd+D）。
    /// 副本偏移 = 选区跨度；单锚点时用量化间隔作为最小偏移。
    pub(crate) fn duplicate_automation_anchors(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let ctxs = Self::collect_anchor_ctxs(doc);
        if ctxs.is_empty() {
            return;
        }
        let ppq = doc.data.model.meta.ppq;
        let quantize = doc
            .edit
            .quantize_pianoroll
            .resolved(doc.edit.pianoroll_view.base.pixels_per_tick, ppq);

        // 每个 ctx 生成副本（偏移 = 该 ctx 选区跨度，单锚点时用量化间隔）。
        let mut edits = Vec::new();
        let mut new_by_panel: Vec<DupResult> = Vec::new();
        for ctx in &ctxs {
            let min_tick = ctx
                .selected
                .iter()
                .map(|(_, t, _, _)| *t)
                .min()
                .unwrap_or(0);
            let max_tick = ctx
                .selected
                .iter()
                .map(|(_, t, _, _)| *t)
                .max()
                .unwrap_or(0);
            let span = max_tick.saturating_sub(min_tick);
            let offset = if span == 0 {
                quantize.tick_interval(ppq).max(1)
            } else {
                span
            };
            let mut new_anchors: Vec<(u32, f32)> = Vec::with_capacity(ctx.selected.len());
            for (_, tick, value, shape) in &ctx.selected {
                let new_tick = (*tick as i64 + offset as i64).max(0) as u32;
                edits.push(AutomationEdit::Add {
                    track_idx: ctx.track_idx,
                    target: ctx.target.clone(),
                    tick: new_tick,
                    value: *value,
                    shape: *shape,
                });
                new_anchors.push((new_tick, *value));
            }
            new_anchors.sort_by_key(|(t, _)| *t);
            new_by_panel.push((ctx.panel.clone(), ctx.target.clone(), new_anchors));
        }

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if actions.is_empty() {
            return;
        }
        let added = added_ids_by_target(&actions);
        let vertical = self.active_tool == Tool::SelectVertical;
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.duplicate_automation").as_ref(),
            before,
        );
        // 重复后选中改为根据新锚点范围设置 sel_rect + 新 id 成员。
        for (panel_ref, target, anchors) in &new_by_panel {
            if let Some(view) = Self::panel_ref_view_mut(doc, panel_ref) {
                view.anchor_sel_rects = sel_rect_from_anchors(anchors, vertical)
                    .map(|r| vec![r])
                    .unwrap_or_default();
                if let Some(ids) = added.get(target) {
                    view.set_anchor_members(ids.iter().copied());
                }
                view.dirty = true;
            }
        }
        self.notify_audio_model_changed();
    }

    /// 删除选中锚点（PR 底部面板 + AR lane）。
    pub(crate) fn delete_automation_anchors(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let ctxs = Self::collect_anchor_ctxs(doc);
        if ctxs.is_empty() {
            return;
        }

        let mut edits = Vec::new();
        for ctx in &ctxs {
            for (_, tick, _, _) in &ctx.selected {
                edits.push(AutomationEdit::Delete {
                    track_idx: ctx.track_idx,
                    lane_idx: ctx.lane_idx,
                    target: ctx.target.clone(),
                    tick: *tick,
                });
            }
        }

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if actions.is_empty() {
            return;
        }
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.delete_automation").as_ref(),
            before,
        );
        for ctx in &ctxs {
            if let Some(view) = Self::panel_ref_view_mut(doc, &ctx.panel) {
                view.clear_anchor_selection();
                view.dirty = true;
            }
        }
        self.notify_audio_model_changed();
    }

    /// 变速：选中锚点按选区跨度 ×`num`/`den` 缩放 tick（value 不变）。
    /// 与音符变速同义（拉长/缩短同一时间轴区间内的锚点分布）。
    pub(crate) fn stretch_automation_anchors(&mut self, num: u64, den: u64) {
        if num == 0 || den == 0 {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let ctxs = Self::collect_anchor_ctxs(doc);
        if ctxs.is_empty() {
            return;
        }

        // 每个 ctx：按选区跨度算 factor，生成 tick 缩放 moves。
        let mut plan: Vec<MovePlan> = Vec::new();
        for ctx in &ctxs {
            let Some(view) = Self::panel_ref_view(doc, &ctx.panel) else {
                continue;
            };
            let Some((t0, t1)) = anchor_span(view) else {
                continue;
            };
            let span = (t1 - t0) as u64;
            let new_span = (span.saturating_mul(num) / den).max(1);
            if new_span == span {
                continue;
            }
            let factor = new_span as f64 / span as f64;
            let mut moves = Vec::new();
            for (_, tick, value, _) in &ctx.selected {
                let nt = (t0 + (*tick as f64 - t0) * factor)
                    .round()
                    .clamp(0.0, u32::MAX as f64) as u32;
                if nt != *tick {
                    moves.push((*tick, nt, *value));
                }
            }
            if moves.is_empty() {
                continue;
            }
            plan.push((
                ctx.panel.clone(),
                ctx.track_idx,
                ctx.lane_idx,
                ctx.target.clone(),
                t0,
                factor,
                moves,
            ));
        }
        if plan.is_empty() {
            return;
        }

        let before = doc.capture_snapshot();
        let mut actions = Vec::new();
        for (_, track_idx, lane_idx, target, _, _, moves) in &plan {
            if let Some(a) =
                doc.move_automation_events_batch(*track_idx as usize, *lane_idx, target, moves)
            {
                actions.push(a);
            }
        }
        if actions.is_empty() {
            return;
        }
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.rescale_span").as_ref(),
            before,
        );
        for (panel_ref, _, _, _, t0, factor, _) in &plan {
            if let Some(view) = Self::panel_ref_view_mut(doc, panel_ref) {
                scale_anchor_rects(view, *t0, *factor);
                view.dirty = true;
            }
        }
        self.notify_audio_model_changed();
    }

    /// 水平翻转：选中锚点在选区跨度 `[t0, t1]` 内做时间镜像（tick 镜像、value 不变）。
    pub(crate) fn flip_automation_anchors_horizontal(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let ctxs = Self::collect_anchor_ctxs(doc);
        if ctxs.is_empty() {
            return;
        }

        let mut plan: Vec<MovePlan> = Vec::new();
        for ctx in &ctxs {
            let Some(view) = Self::panel_ref_view(doc, &ctx.panel) else {
                continue;
            };
            let Some((t0, t1)) = anchor_span(view) else {
                continue;
            };
            let mut moves = Vec::new();
            for (_, tick, value, _) in &ctx.selected {
                let nt = (t0 + (t1 - *tick as f64))
                    .round()
                    .clamp(0.0, u32::MAX as f64) as u32;
                if nt != *tick {
                    moves.push((*tick, nt, *value));
                }
            }
            if moves.is_empty() {
                continue;
            }
            plan.push((
                ctx.panel.clone(),
                ctx.track_idx,
                ctx.lane_idx,
                ctx.target.clone(),
                t0,
                t1,
                moves,
            ));
        }
        if plan.is_empty() {
            return;
        }

        let before = doc.capture_snapshot();
        let mut actions = Vec::new();
        for (_, track_idx, lane_idx, target, _, _, moves) in &plan {
            if let Some(a) =
                doc.move_automation_events_batch(*track_idx as usize, *lane_idx, target, moves)
            {
                actions.push(a);
            }
        }
        if actions.is_empty() {
            return;
        }
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.flip_horizontal").as_ref(),
            before,
        );
        for (panel_ref, _, _, _, t0, t1, _) in &plan {
            if let Some(view) = Self::panel_ref_view_mut(doc, panel_ref) {
                mirror_anchor_rects(view, *t0, *t1);
                view.dirty = true;
            }
        }
        self.notify_audio_model_changed();
    }

    /// 选区（rects）的 tick 跨度 `[t0, t1]`；无有效跨度返回 None。
    fn selection_span(selection: &yinhe_core::Selection) -> Option<(f64, f64)> {
        let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(ts, te, _, _, _, _) in &selection.rects {
            t0 = t0.min(ts as f64);
            t1 = t1.max(te as f64);
        }
        (t1 > t0).then_some((t0, t1))
    }

    /// AR 框选是否选中了自动化事件（`selected` 物化出自动化成员）。
    ///
    /// AR 主框选把区域内音符与自动化事件一起物化进共享 `selected`，
    /// 与 PR 底部面板 / AM lane 的 `anchor_sel_rects` 是两套不同的选区表示。
    pub(crate) fn has_selected_ar_automation(&self) -> bool {
        let Some(idx) = self.workspace.active_doc else {
            return false;
        };
        if !matches!(self.active_tool, Tool::Select | Tool::SelectVertical) {
            return false;
        }
        self.workspace.documents[idx]
            .edit
            .selected
            .automation_member_count()
            .is_some_and(|c| c > 0)
    }

    /// 汇总 AR 框选选中的自动化事件，按 `(track, lane)` 分组。
    ///
    /// 成员态按事件 id 判定，矩形态回退 rect + 属性筛选（与 AR 搬运口径一致）；
    /// 不含 conductor.tempo（与 AR 框选物化范围一致）。
    fn collect_selected_automation(
        doc: &yinhe_editor_core::document::Document,
    ) -> Vec<(u16, usize, AutomationTarget, Vec<AutomationEvent>)> {
        let selection = &doc.edit.selected;
        let mut map: std::collections::HashMap<
            (u16, usize),
            (AutomationTarget, Vec<AutomationEvent>),
        > = std::collections::HashMap::new();
        for &(_ts, _te, _kl, _kh, tl, th) in &selection.rects {
            for track_u16 in tl..=th {
                let track_idx = track_u16 as usize;
                let Some(track) = doc.data.model.tracks.get(track_idx) else {
                    continue;
                };
                for (lane_idx, lane) in track.automation_lanes.iter().enumerate() {
                    for ev in &lane.events {
                        if selection.accepts_automation_event(track_u16, &lane.target, ev) {
                            let entry = map
                                .entry((track_u16, lane_idx))
                                .or_insert_with(|| (lane.target.clone(), Vec::new()));
                            if !entry.1.iter().any(|e| e.id == ev.id) {
                                entry.1.push(*ev);
                            }
                        }
                    }
                }
            }
        }
        map.into_iter()
            .map(|((track, lane), (target, events))| (track, lane, target, events))
            .collect()
    }

    /// AR 框选删除：删除选中自动化事件（并删除选中的音符）。
    pub(crate) fn delete_ar_selection(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let lanes = Self::collect_selected_automation(doc);
        let mut edits = Vec::new();
        for (track, lane, target, events) in &lanes {
            for ev in events {
                edits.push(AutomationEdit::Delete {
                    track_idx: *track,
                    lane_idx: *lane,
                    target: target.clone(),
                    tick: ev.tick,
                });
            }
        }

        let before = doc.capture_snapshot();
        let mut actions = doc.apply_automation_edits(edits);
        // 同时删除选中的音符（delete_selected 会清空共享选区）。
        if let Some(a) = doc.delete_selected() {
            actions.push(a);
        }
        doc.edit.arr_sel_rect.clear();
        doc.edit.chan_ar_sel.clear();
        if actions.is_empty() {
            doc.edit.selected.clear();
            return;
        }
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.delete_automation").as_ref(),
            before,
        );
        self.notify_audio_model_changed();
    }

    /// AR 框选重复：副本偏移 = 选区跨度（单 tick 跨度时用量化间隔）。
    /// 复用 `duplicate_selected_arrange`（音符 + 自动化一并复制）。
    pub(crate) fn duplicate_ar_selection(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let Some((t0, t1)) = Self::selection_span(&doc.edit.selected) else {
            return;
        };
        let span = (t1 - t0) as i64;
        let delta = if span <= 0 {
            let ppq = doc.data.model.meta.ppq;
            doc.edit
                .quantize_arrange
                .resolved(doc.edit.arrange_view.base.pixels_per_tick, ppq)
                .tick_interval(ppq)
                .max(1) as i64
        } else {
            span
        };

        let before = doc.capture_snapshot();
        if let Some(a) = doc.duplicate_selected_arrange(delta, 0) {
            // AR 选框（arr_sel_rect）跟随副本。
            for r in &mut doc.edit.arr_sel_rect {
                r.0 += delta as f64;
                r.1 += delta as f64;
            }
            for r in &mut doc.edit.chan_ar_sel {
                r.t_start += delta as f64;
                r.t_end += delta as f64;
            }
            doc.edit.pianoroll_view.base.dirty = true;
            crate::right_panel::automation_undo::push_automation_actions(
                doc,
                vec![a],
                t!("undo.duplicate_automation").as_ref(),
                before,
            );
            self.notify_audio_model_changed();
        }
    }

    /// AR 框选变速：选中自动化事件按选区跨度 ×`num`/`den` 缩放 tick
    /// （并同步缩放选中的音符）。
    pub(crate) fn stretch_ar_selection(&mut self, num: u64, den: u64) {
        if num == 0 || den == 0 {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let Some((t0, t1)) = Self::selection_span(&doc.edit.selected) else {
            return;
        };
        let span = (t1 - t0) as u64;
        if span == 0 {
            return;
        }
        let new_span = (span.saturating_mul(num) / den).max(1);
        if new_span == span {
            return;
        }
        let factor = new_span as f64 / span as f64;

        let lanes = Self::collect_selected_automation(doc);
        let mut edits = Vec::new();
        for (track, lane, target, events) in &lanes {
            let mut moves: Vec<(u32, u32, f32)> = Vec::new();
            for ev in events {
                let nt = (t0 + (ev.tick as f64 - t0) * factor)
                    .round()
                    .clamp(0.0, u32::MAX as f64) as u32;
                if nt != ev.tick {
                    moves.push((ev.tick, nt, ev.value));
                }
            }
            if !moves.is_empty() {
                edits.push(AutomationEdit::MoveBatch {
                    track_idx: *track,
                    lane_idx: *lane,
                    target: target.clone(),
                    moves,
                });
            }
        }

        let before = doc.capture_snapshot();
        let mut actions = doc.apply_automation_edits(edits);
        // 音符 + 选框 tick 范围同步缩放。
        if let Some(a) = doc.rescale_selection_span(new_span) {
            actions.push(a);
        }
        if actions.is_empty() {
            return;
        }
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.rescale_span").as_ref(),
            before,
        );
        self.notify_audio_model_changed();
    }

    /// AR 框选水平翻转：选中自动化事件在选区跨度内做时间镜像（并翻转选中音符）。
    pub(crate) fn flip_ar_selection_horizontal(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let Some((t0, t1)) = Self::selection_span(&doc.edit.selected) else {
            return;
        };

        let lanes = Self::collect_selected_automation(doc);
        let mut edits = Vec::new();
        for (track, lane, target, events) in &lanes {
            let mut moves: Vec<(u32, u32, f32)> = Vec::new();
            for ev in events {
                let nt = (t0 + (t1 - ev.tick as f64))
                    .round()
                    .clamp(0.0, u32::MAX as f64) as u32;
                if nt != ev.tick {
                    moves.push((ev.tick, nt, ev.value));
                }
            }
            if !moves.is_empty() {
                edits.push(AutomationEdit::MoveBatch {
                    track_idx: *track,
                    lane_idx: *lane,
                    target: target.clone(),
                    moves,
                });
            }
        }

        let before = doc.capture_snapshot();
        let mut actions = doc.apply_automation_edits(edits);
        if let Some(a) = doc.flip_selected_notes(yinhe_editor_core::FlipAxis::Horizontal) {
            actions.push(a);
        }
        if actions.is_empty() {
            return;
        }
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.flip_horizontal").as_ref(),
            before,
        );
        self.notify_audio_model_changed();
    }

    // ── 辅助函数 ──

    /// 按 [`PanelRef`] 取只读面板视图。
    fn panel_ref_view<'a>(
        doc: &'a yinhe_editor_core::document::Document,
        p: &PanelRef,
    ) -> Option<&'a AutomationPanelView> {
        match p {
            PanelRef::Pr(i) => doc.edit.controller_panels.get(*i),
            PanelRef::Ar(track, target) => doc.edit.arr_am_views.get(&(*track, target.clone())),
        }
    }

    /// 按 [`PanelRef`] 取可变面板视图。
    fn panel_ref_view_mut<'a>(
        doc: &'a mut yinhe_editor_core::document::Document,
        p: &PanelRef,
    ) -> Option<&'a mut AutomationPanelView> {
        match p {
            PanelRef::Pr(i) => doc.edit.controller_panels.get_mut(*i),
            PanelRef::Ar(track, target) => doc.edit.arr_am_views.get_mut(&(*track, target.clone())),
        }
    }

    /// 收集所有「有选中锚点」面板的操作上下文（PR 底部面板 + AR lane）。
    fn collect_anchor_ctxs(doc: &yinhe_editor_core::document::Document) -> Vec<AnchorCtx> {
        let mut result = Vec::new();

        // ── PR 底部面板（非 Tempo 用主音轨）──
        for (panel_idx, panel) in doc.edit.controller_panels.iter().enumerate() {
            if panel.show_velocity || panel.anchor_sel_rects.is_empty() {
                continue;
            }
            let target = panel.selected_target.clone();
            let Some(track_idx) = Self::track_idx_for(doc, &target) else {
                continue;
            };
            if let Some(ctx) =
                Self::build_ctx(doc, PanelRef::Pr(panel_idx), track_idx, target, panel)
            {
                result.push(ctx);
            }
        }

        // ── AR lane（key 直接给出 track + target）──
        for ((track, target), panel) in doc.edit.arr_am_views.iter() {
            if panel.show_velocity || panel.anchor_sel_rects.is_empty() {
                continue;
            }
            // Tempo 锚点存于 conductor（track/lane 被忽略）；非 Tempo 用 key 的 track。
            let track_idx = if matches!(target, AutomationTarget::Tempo) {
                doc.edit.track_cache.conductor_idx.unwrap_or(*track)
            } else {
                *track
            };
            if let Some(ctx) = Self::build_ctx(
                doc,
                PanelRef::Ar(*track, target.clone()),
                track_idx,
                target.clone(),
                panel,
            ) {
                result.push(ctx);
            }
        }

        result
    }

    /// 构造单个面板的锚点上下文。
    fn build_ctx(
        doc: &yinhe_editor_core::document::Document,
        panel: PanelRef,
        track_idx: u16,
        target: AutomationTarget,
        view: &AutomationPanelView,
    ) -> Option<AnchorCtx> {
        let (lane_idx, selected): (usize, Vec<AnchorSnap>) =
            if matches!(target, AutomationTarget::Tempo) {
                let selected = doc
                    .data
                    .model
                    .conductor
                    .tempo
                    .events
                    .iter()
                    .filter(|e| view.accepts_anchor(e))
                    .map(|e| (e.id, e.tick, e.value, e.shape))
                    .collect();
                (0, selected)
            } else {
                let track = doc.data.model.tracks.get(track_idx as usize)?;
                let (lane_idx, lane) = track
                    .automation_lanes
                    .iter()
                    .enumerate()
                    .find(|(_, l)| l.target == target)?;
                let selected = lane
                    .events
                    .iter()
                    .filter(|e| view.accepts_anchor(e))
                    .map(|e| (e.id, e.tick, e.value, e.shape))
                    .collect();
                (lane_idx, selected)
            };
        if selected.is_empty() {
            return None;
        }
        Some(AnchorCtx {
            panel,
            target,
            track_idx,
            lane_idx,
            selected,
        })
    }

    /// 获取 target 对应的 track_idx（PR 规则：Tempo → conductor；其他 → 主音轨）。
    fn track_idx_for(
        doc: &yinhe_editor_core::document::Document,
        target: &AutomationTarget,
    ) -> Option<u16> {
        if matches!(target, AutomationTarget::Tempo) {
            doc.edit.track_cache.conductor_idx
        } else {
            doc.edit
                .main_track()
                .filter(|&t| Some(t) != doc.edit.track_cache.conductor_idx)
        }
    }
}
