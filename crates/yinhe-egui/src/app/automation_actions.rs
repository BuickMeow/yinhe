//! 自动化锚点的复制/粘贴/重复/删除快捷键操作。
//!
//! 当 Select/SelectVertical 工具激活且有锚点选中时，
//! copy/paste/duplicate/delete 作用于自动化锚点而非音符。

use rust_i18n::t;
use yinhe_types::{AnchorSelRect, AutomationTarget, SegmentShape};

use crate::app::App;
use crate::widgets::tools_panel::Tool;

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

/// 自动化锚点剪贴板。
///
/// 存储复制的锚点事件 `(tick, value, shape)` + 源 `target`。
/// 粘贴时只应用到 `target` 匹配的面板，跨文档共享（app 级状态）。
#[derive(Clone, Debug, Default)]
pub(crate) struct AutomationClipboard {
    /// 源 target。`None` 表示剪贴板为空。
    pub target: Option<AutomationTarget>,
    /// 复制的锚点事件（按 tick 升序）。
    pub events: Vec<(u32, f32, SegmentShape)>,
}

/// 从 Document 读取的信息包：复制/删除/重复共用。
struct AnchorCtx {
    /// 面板索引（controller_panels 中的位置）。
    panel_idx: usize,
    /// 锚点所属 target。
    target: AutomationTarget,
    /// 锚点所属 track_idx（用于 AutomationEdit）。
    track_idx: u16,
    /// lane 在 tracks[track].automation_lanes 中的索引（Tempo 用 0）。
    lane_idx: usize,
    /// lane 的 events 快照（用于查找选中锚点的 value/shape）。
    events: Vec<(u32, f32, SegmentShape)>,
}

impl App {
    /// 是否有任意面板选中了自动化锚点（用于快捷键路由）。
    ///
    /// 仅 Select/SelectVertical 工具下返回 true。
    /// 此时 copy/paste/duplicate/delete 作用于锚点而非音符。
    pub(crate) fn has_selected_automation_anchors(&self) -> bool {
        let Some(idx) = self.active_doc else {
            return false;
        };
        if !matches!(self.active_tool, Tool::Select | Tool::SelectVertical) {
            return false;
        }
        self.documents[idx]
            .edit
            .controller_panels
            .iter()
            .any(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
    }

    /// 复制选中锚点到剪贴板。
    pub(crate) fn copy_automation_anchors(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let doc = &self.documents[idx];

        let Some(ctx) = Self::collect_anchor_ctx(doc) else {
            return;
        };
        let panel = &doc.edit.controller_panels[ctx.panel_idx];
        if panel.anchor_sel_rects.is_empty() {
            return;
        }

        // 收集落在任一 sel_rect 内的锚点
        let mut copied: Vec<(u32, f32, SegmentShape)> = ctx
            .events
            .iter()
            .filter(|(tick, value, _)| {
                panel
                    .anchor_sel_rects
                    .iter()
                    .any(|r| r.contains(*tick, *value))
            })
            .copied()
            .collect();
        if copied.is_empty() {
            return;
        }
        copied.sort_by_key(|(t, _, _)| *t);

        self.automation_clipboard = AutomationClipboard {
            target: Some(ctx.target),
            events: copied,
        };
    }

    /// 粘贴剪贴板锚点到 cursor_tick 位置。
    pub(crate) fn paste_automation_anchors(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let clipboard = self.automation_clipboard.clone();
        let Some(target) = clipboard.target else {
            return;
        };
        if clipboard.events.is_empty() {
            return;
        }

        let doc = &mut self.documents[idx];

        // 找 target 匹配的面板
        let panel_idx = doc
            .edit
            .controller_panels
            .iter()
            .position(|p| !p.show_velocity && p.selected_target == target);
        let Some(panel_idx) = panel_idx else { return };

        // 获取 track_idx（与 collect_anchor_ctx 一致）
        let Some(track_idx) = Self::track_idx_for(doc, &target) else {
            return;
        };

        let cursor_tick = doc.edit.cursor_tick.unwrap_or(0.0) as u32;
        let min_tick = clipboard
            .events
            .iter()
            .map(|(t, _, _)| *t)
            .min()
            .unwrap_or(0);
        let offset = cursor_tick as i64 - min_tick as i64;

        let mut edits = Vec::with_capacity(clipboard.events.len());
        let mut new_anchors: Vec<(u32, f32)> = Vec::new();
        for (tick, value, shape) in &clipboard.events {
            let new_tick = (*tick as i64 + offset).max(0) as u32;
            edits.push(yinhe_types::AutomationEdit::Add {
                track_idx,
                target: target.clone(),
                tick: new_tick,
                value: *value,
                shape: *shape,
            });
            new_anchors.push((new_tick, *value));
        }

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if !actions.is_empty() {
            self.pianoroll_view.base.dirty = true;
            crate::right_panel::automation_undo::push_automation_actions(
                doc,
                actions,
                t!("undo.paste_automation").as_ref(),
                before,
            );
            // 粘贴后选中改为根据新锚点范围设置 sel_rect
            let vertical = self.active_tool == Tool::SelectVertical;
            doc.edit.controller_panels[panel_idx].anchor_sel_rects =
                sel_rect_from_anchors(&new_anchors, vertical)
                    .map(|r| vec![r])
                    .unwrap_or_default();
            doc.edit.controller_panels[panel_idx].dirty = true;
            self.notify_audio_model_changed();
        }
    }

    /// 重复选中锚点（Cmd+D）。
    /// 副本偏移 = 选区跨度；单锚点时用量化间隔作为最小偏移。
    pub(crate) fn duplicate_automation_anchors(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];

        let Some(ctx) = Self::collect_anchor_ctx(doc) else {
            return;
        };
        let ppq = doc.data.model.meta.ppq;
        let quantize = doc.edit.quantize_pianoroll;
        let panel = &doc.edit.controller_panels[ctx.panel_idx];
        if panel.anchor_sel_rects.is_empty() {
            return;
        }

        // 收集落在任一 sel_rect 内的选中锚点（按 tick 升序）
        let mut selected: Vec<(u32, f32, SegmentShape)> = ctx
            .events
            .iter()
            .filter(|(tick, value, _)| {
                panel
                    .anchor_sel_rects
                    .iter()
                    .any(|r| r.contains(*tick, *value))
            })
            .copied()
            .collect();
        if selected.is_empty() {
            return;
        }
        selected.sort_by_key(|(t, _, _)| *t);

        // 偏移：选区跨度，单锚点时用量化间隔
        let min_tick = selected.first().map(|(t, _, _)| *t).unwrap_or(0);
        let max_tick = selected.last().map(|(t, _, _)| *t).unwrap_or(0);
        let span = max_tick.saturating_sub(min_tick);
        let offset = if span == 0 {
            quantize.tick_interval(ppq).max(1)
        } else {
            span
        };

        // 生成副本
        let mut edits = Vec::with_capacity(selected.len());
        let mut new_anchors: Vec<(u32, f32)> = Vec::new();
        for (tick, value, shape) in &selected {
            let new_tick = (*tick as i64 + offset as i64).max(0) as u32;
            edits.push(yinhe_types::AutomationEdit::Add {
                track_idx: ctx.track_idx,
                target: ctx.target.clone(),
                tick: new_tick,
                value: *value,
                shape: *shape,
            });
            new_anchors.push((new_tick, *value));
        }

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if !actions.is_empty() {
            self.pianoroll_view.base.dirty = true;
            crate::right_panel::automation_undo::push_automation_actions(
                doc,
                actions,
                t!("undo.duplicate_automation").as_ref(),
                before,
            );
            // 重复后选中改为根据新锚点范围设置 sel_rect
            let vertical = self.active_tool == Tool::SelectVertical;
            doc.edit.controller_panels[ctx.panel_idx].anchor_sel_rects =
                sel_rect_from_anchors(&new_anchors, vertical)
                    .map(|r| vec![r])
                    .unwrap_or_default();
            doc.edit.controller_panels[ctx.panel_idx].dirty = true;
            self.notify_audio_model_changed();
        }
    }

    /// 删除选中锚点。
    pub(crate) fn delete_automation_anchors(&mut self) {
        let Some(idx) = self.active_doc else { return };
        let doc = &mut self.documents[idx];

        let Some(ctx) = Self::collect_anchor_ctx(doc) else {
            return;
        };
        let panel = &doc.edit.controller_panels[ctx.panel_idx];
        if panel.anchor_sel_rects.is_empty() {
            return;
        }

        // 收集落在任一 sel_rect 内的锚点 tick
        let mut edits = Vec::new();
        for (tick, value, _) in &ctx.events {
            if panel
                .anchor_sel_rects
                .iter()
                .any(|r| r.contains(*tick, *value))
            {
                edits.push(yinhe_types::AutomationEdit::Delete {
                    track_idx: ctx.track_idx,
                    lane_idx: ctx.lane_idx,
                    target: ctx.target.clone(),
                    tick: *tick,
                });
            }
        }

        let before = doc.capture_snapshot();
        let actions = doc.apply_automation_edits(edits);
        if !actions.is_empty() {
            self.pianoroll_view.base.dirty = true;
            crate::right_panel::automation_undo::push_automation_actions(
                doc,
                actions,
                t!("undo.delete_automation").as_ref(),
                before,
            );
            doc.edit.controller_panels[ctx.panel_idx]
                .anchor_sel_rects
                .clear();
            doc.edit.controller_panels[ctx.panel_idx].dirty = true;
            self.notify_audio_model_changed();
        }
    }

    // ── 辅助函数 ──

    /// 从 Document 收集锚点操作所需的上下文：面板索引、target、track_idx、lane_idx、events。
    ///
    /// 找第一个有选中锚点的非 velocity 面板。返回 `None` 表示无可操作面板。
    fn collect_anchor_ctx(doc: &yinhe_editor_core::document::Document) -> Option<AnchorCtx> {
        let panel_idx = doc
            .edit
            .controller_panels
            .iter()
            .position(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())?;
        let panel = &doc.edit.controller_panels[panel_idx];
        let target = panel.selected_target.clone();

        let track_idx = Self::track_idx_for(doc, &target)?;

        // 获取 lane events + lane_idx
        let (lane_idx, events): (usize, Vec<(u32, f32, SegmentShape)>) =
            if matches!(target, AutomationTarget::Tempo) {
                let events = doc
                    .data
                    .model
                    .conductor
                    .tempo
                    .events
                    .iter()
                    .map(|e| (e.tick, e.value, e.shape))
                    .collect();
                (0, events)
            } else {
                let track = doc.data.model.tracks.get(track_idx as usize)?;
                let (lane_idx, lane) = track
                    .automation_lanes
                    .iter()
                    .enumerate()
                    .find(|(_, l)| l.target == target)?;
                let events = lane
                    .events
                    .iter()
                    .map(|e| (e.tick, e.value, e.shape))
                    .collect();
                (lane_idx, events)
            };

        Some(AnchorCtx {
            panel_idx,
            target,
            track_idx,
            lane_idx,
            events,
        })
    }

    /// 获取 target 对应的 track_idx。
    /// Tempo → conductor_track_idx；其他 → 主音轨（需可见且非 conductor）。
    fn track_idx_for(
        doc: &yinhe_editor_core::document::Document,
        target: &AutomationTarget,
    ) -> Option<u16> {
        if matches!(target, AutomationTarget::Tempo) {
            doc.edit.conductor_track_idx
        } else {
            doc.edit
                .main_track()
                .filter(|&t| {
                    doc.edit
                        .track_visible
                        .get(t as usize)
                        .copied()
                        .unwrap_or(false)
                })
                .filter(|&t| Some(t) != doc.edit.conductor_track_idx)
        }
    }
}
