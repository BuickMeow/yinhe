//! 全选、仅选音符、选择筛选。

use crate::app::App;

impl App {
    /// Select all notes — PR or AR depending on the focused view.
    /// Arrange 模式下底部同时显示 PR 时，以鼠标所在视图为准。
    pub(crate) fn select_all(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let is_pr = self.view_focus == crate::app::ViewFocus::Pianoroll;
        if is_pr {
            self.workspace.documents[idx].select_all_pr();
        } else {
            // select_all_ar 内部会同步设置 doc.edit.arr_sel_rect（AR 选框）。
            self.workspace.documents[idx].select_all_ar();
        }
        self.workspace.documents[idx].data.bump_revision();
        self.workspace.documents[idx].edit.pianoroll_view.base.dirty = true;
        self.workspace.documents[idx].edit.arrange_view.base.dirty = true;
    }

    /// 「仅选择音符」：清除所有自动化锚点选择（PR 面板 + AR 展开 lane），
    /// 只保留音符选框。
    pub(crate) fn select_notes_only(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        let mut changed = false;
        for panel in &mut doc.edit.controller_panels {
            if !panel.anchor_sel_rects.is_empty() || panel.anchor_members.is_some() {
                panel.anchor_sel_rects.clear();
                panel.anchor_members = None;
                panel.materialized_anchor_rects = 0;
                panel.dirty = true;
                changed = true;
            }
        }
        for view in doc.edit.arr_am_views.values_mut() {
            if !view.anchor_sel_rects.is_empty() || view.anchor_members.is_some() {
                view.anchor_sel_rects.clear();
                view.anchor_members = None;
                view.materialized_anchor_rects = 0;
                view.dirty = true;
                changed = true;
            }
        }
        // AR 删除自动化走共享 Selection 的成员位图；只清面板选框会留下
        // 不可见的“幽灵”自动化选择，Delete 仍会删掉它们。
        if doc.edit.selected.has_explicit_automation_members() {
            doc.edit.selected.clear_automation_selection();
            changed = true;
        }
        if changed {
            doc.edit.pianoroll_view.base.dirty = true;
            doc.edit.arrange_view.base.dirty = true;
        }
    }

    /// 打开选择筛选对话框（从当前选区/筛选初始化）。
    pub(crate) fn open_filter_dialog(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &self.workspace.documents[idx];
        self.filter_dialog.open(&doc.edit.selected, &doc.data.model);
    }

    /// 应用筛选：写属性边界（选框矩形保持不动）。
    pub(crate) fn apply_filter_dialog(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        doc.edit.selected.filter = self.filter_dialog.build_filter();
        doc.edit.pianoroll_view.base.dirty = true;
        doc.edit.arrange_view.base.dirty = true;
    }

    /// 清除筛选边界（保留选框与已收窄的空间范围）。
    pub(crate) fn clear_filter(&mut self) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[idx];
        doc.edit.selected.filter = Default::default();
        doc.edit.pianoroll_view.base.dirty = true;
        doc.edit.arrange_view.base.dirty = true;
    }
}
