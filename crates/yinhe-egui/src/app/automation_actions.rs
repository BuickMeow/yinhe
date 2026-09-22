//! 自动化锚点的复制/粘贴/重复/删除快捷键操作。
//!
//! 当 Select/SelectVertical 工具激活且有锚点选中时，
//! copy/cut/duplicate/delete 作用于自动化锚点而非音符。
//! paste 由 `App::paste_clipboard` 按剪贴板内容类型分派。

use rust_i18n::t;
use yinhe_editor_core::clipboard::{AutomationClipboard, AutomationSelection, PasteMode};
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
    /// 面板的选中矩形快照（复制时在其中筛锚点）。
    sel_rects: Vec<AnchorSelRect>,
}

impl App {
    /// 是否有任意面板选中了自动化锚点（用于快捷键路由）。
    ///
    /// 仅 Select/SelectVertical 工具下返回 true。
    /// 此时 copy/cut/duplicate/delete 作用于锚点而非音符。
    pub(crate) fn has_selected_automation_anchors(&self) -> bool {
        let Some(idx) = self.workspace.active_doc else {
            return false;
        };
        if !matches!(self.active_tool, Tool::Select | Tool::SelectVertical) {
            return false;
        }
        self.workspace.documents[idx]
            .edit
            .controller_panels
            .iter()
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

        let selections: Vec<AutomationSelection> = doc
            .edit
            .controller_panels
            .iter()
            .filter(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
            .map(|p| AutomationSelection {
                target: p.selected_target.clone(),
                sel_rects: p.anchor_sel_rects.clone(),
            })
            .collect();
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
        doc.edit.pianoroll_view.base.dirty = true;
        crate::right_panel::automation_undo::push_automation_actions(
            doc,
            actions,
            t!("undo.paste_automation").as_ref(),
            before,
        );
        // 粘贴后选中改为根据新锚点范围设置 sel_rect（音符粘贴的选区跟随对齐）
        for (panel_idx, anchors) in &panel_anchors {
            doc.edit.controller_panels[*panel_idx].anchor_sel_rects =
                sel_rect_from_anchors(anchors, vertical)
                    .map(|r| vec![r])
                    .unwrap_or_default();
            doc.edit.controller_panels[*panel_idx].dirty = true;
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

        let Some(ctx) = Self::collect_anchor_ctxs(doc).into_iter().next() else {
            return;
        };
        let ppq = doc.data.model.meta.ppq;
        let quantize = doc.edit.quantize_pianoroll;

        // 收集落在任一 sel_rect 内的选中锚点（按 tick 升序）
        let mut selected: Vec<(u32, f32, SegmentShape)> = ctx
            .events
            .iter()
            .filter(|(tick, value, _)| ctx.sel_rects.iter().any(|r| r.contains(*tick, *value)))
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
            doc.edit.pianoroll_view.base.dirty = true;
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
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];

        let Some(ctx) = Self::collect_anchor_ctxs(doc).into_iter().next() else {
            return;
        };

        // 收集落在任一 sel_rect 内的锚点 tick
        let mut edits = Vec::new();
        for (tick, value, _) in &ctx.events {
            if ctx.sel_rects.iter().any(|r| r.contains(*tick, *value)) {
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
            doc.edit.pianoroll_view.base.dirty = true;
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

    /// 从 Document 收集所有「有选中锚点」面板的操作上下文。
    fn collect_anchor_ctxs(doc: &yinhe_editor_core::document::Document) -> Vec<AnchorCtx> {
        let mut result = Vec::new();
        for (panel_idx, panel) in doc.edit.controller_panels.iter().enumerate() {
            if panel.show_velocity || panel.anchor_sel_rects.is_empty() {
                continue;
            }
            let target = panel.selected_target.clone();
            let Some(track_idx) = Self::track_idx_for(doc, &target) else {
                continue;
            };

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
                    let Some(track) = doc.data.model.tracks.get(track_idx as usize) else {
                        continue;
                    };
                    let Some((lane_idx, lane)) = track
                        .automation_lanes
                        .iter()
                        .enumerate()
                        .find(|(_, l)| l.target == target)
                    else {
                        continue;
                    };
                    let events = lane
                        .events
                        .iter()
                        .map(|e| (e.tick, e.value, e.shape))
                        .collect();
                    (lane_idx, events)
                };

            result.push(AnchorCtx {
                panel_idx,
                target,
                track_idx,
                lane_idx,
                events,
                sel_rects: panel.anchor_sel_rects.clone(),
            });
        }
        result
    }

    /// 获取 target 对应的 track_idx。
    /// Tempo → conductor_track_idx；其他 → 主音轨（非 conductor）。
    /// 主音轨在 PR 强制可见（layout pr_visible），不要求 track_visible 勾选。
    fn track_idx_for(
        doc: &yinhe_editor_core::document::Document,
        target: &AutomationTarget,
    ) -> Option<u16> {
        if matches!(target, AutomationTarget::Tempo) {
            doc.edit.conductor_track_idx
        } else {
            doc.edit
                .main_track()
                .filter(|&t| Some(t) != doc.edit.conductor_track_idx)
        }
    }
}
