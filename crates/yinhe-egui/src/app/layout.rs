use eframe::egui;

use crate::app::{App, ViewFocus};

pub mod chord;
pub mod content;
pub mod sel_hint;

pub(crate) use sel_hint::SelHintInfo;

/// `Panel::right` 占位：扣出右栏区域并返回其矩形（含左缘分割条）。
/// 必须在 dock 之前调用，保证右栏占整条、dock 只在左区（见回归测试）。
pub(in crate::app) fn show_right_panel_placeholder(ui: &mut egui::Ui, total_w: f32) -> egui::Rect {
    egui::Panel::right("right_panel_area")
        .exact_size(total_w)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE)
        .show(ui, |_| {})
        .response
        .rect
}

/// Layout geometry computed once per frame, shared by arrangement and pianoroll.
pub(in crate::app) struct LayoutInfo {
    /// 中央内容区（Panel 扣减后：右栏、dock、底栏、走带栏都已排除）。
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub bottom_y: f32,
    /// 右栏完整矩形（含左缘分割条；从走带栏下沿到底栏上沿，dock 不截断它）。
    /// 右栏未打开时为 [`egui::Rect::NOTHING`]。
    pub right_panel_rect: egui::Rect,
}

impl App {
    /// 右栏总宽（含左缘分割条）；未打开时返回 0。
    ///
    /// 宽度 clamp 的唯一来源：布局（`right_panel_placeholder`）与右栏拖拽共用，
    /// 避免两处 clamp 基准不一致（Dock/主内容依赖这里扣出的宽度）。
    pub(in crate::app) fn right_panel_total_width(&mut self, avail_w: f32) -> f32 {
        if self.right_tab.is_none() {
            return 0.0;
        }
        let max_w = (avail_w - 60.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0);
        let pw =
            (self.right_panel_width + 4.0).clamp(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0, max_w);
        self.right_panel_width = (pw - 4.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH);
        pw
    }

    /// 右栏布局占位：只做可用区扣减（不绘制内容），必须在 dock 之前调用。
    /// `Panel::right` 占满右侧整条，之后 show 的底栏只能占左区，主内容也在左区；
    /// 内容仍由 `right_panel::show` 按返回的同一 rect 手绘（拖拽/持久化不变）。
    pub(in crate::app) fn right_panel_placeholder(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        if self.right_tab.is_none() {
            return egui::Rect::NOTHING;
        }
        let total_w = self.right_panel_total_width(ui.available_width());
        show_right_panel_placeholder(ui, total_w)
    }

    /// 主内容区几何。`right_panel_rect` 由 `right_panel_placeholder`（Panel::right
    /// 占位）返回，`ui` 的可用区已扣除右栏与 dock，因此这里不再重复扣减。
    pub(in crate::app) fn compute_layout(
        &mut self,
        ui: &mut egui::Ui,
        right_panel_rect: egui::Rect,
    ) -> LayoutInfo {
        let remaining = ui.available_rect_before_wrap();

        let has_arr = self.view_mode.show_transport() && self.workspace.active_doc.is_some();
        let has_piano = self
            .view_mode
            .show_pianoroll(self.show_pianoroll_in_arrange)
            && self.workspace.active_doc.is_some();

        let total = remaining.size();
        let arr_h = if has_arr {
            if has_piano {
                (total.y * self.arr_split).max(crate::theme::MIN_ARR_HEIGHT)
            } else {
                total.y
            }
        } else {
            0.0
        };
        let bottom_y = remaining.min.y
            + arr_h
            + if has_arr && has_piano {
                crate::theme::SPLIT_GAP
            } else {
                0.0
            };

        LayoutInfo {
            remaining,
            arr_h,
            bottom_y,
            right_panel_rect,
        }
    }

    /// 维护聚焦视图：记录最近一次鼠标按下（"最近点过哪里"）所在的视图；
    /// 焦点视图不可见时切到可见的那个。Ctrl+A 等快捷键按 `view_focus` 路由。
    pub(in crate::app) fn update_view_focus(&mut self, ui: &egui::Ui, layout: &LayoutInfo) {
        let has_doc = self.workspace.active_doc.is_some();
        let has_arr = has_doc && self.view_mode.show_transport();
        let has_piano = has_doc
            && self
                .view_mode
                .show_pianoroll(self.show_pianoroll_in_arrange);
        let arr_rect = egui::Rect::from_min_max(
            layout.remaining.min,
            egui::pos2(
                layout.remaining.max.x,
                layout.remaining.min.y + layout.arr_h,
            ),
        );
        let pr_rect = egui::Rect::from_min_max(
            egui::pos2(layout.remaining.min.x, layout.bottom_y),
            layout.remaining.max,
        );
        // 仅在按下的那一帧读取按下位置：指针移动/拖拽不会漂移焦点。
        let press_pos = ui.input(|i| {
            if i.pointer.any_pressed() {
                i.pointer.press_origin()
            } else {
                None
            }
        });
        self.view_focus = resolve_view_focus(
            self.view_focus,
            press_pos,
            arr_rect,
            pr_rect,
            has_arr,
            has_piano,
        );
    }
}

/// 聚焦视图决策（纯函数，便于测试）：
/// 最近一次按下在可见视图内 → 该视图；否则保持当前焦点；焦点视图不可见 → 切到可见视图。
pub(in crate::app) fn resolve_view_focus(
    current: ViewFocus,
    press_pos: Option<egui::Pos2>,
    arr_rect: egui::Rect,
    pr_rect: egui::Rect,
    has_arr: bool,
    has_piano: bool,
) -> ViewFocus {
    if let Some(pos) = press_pos {
        if has_piano && pr_rect.contains(pos) {
            return ViewFocus::Pianoroll;
        }
        if has_arr && arr_rect.contains(pos) {
            return ViewFocus::Arrange;
        }
    }
    match current {
        ViewFocus::Arrange if !has_arr && has_piano => ViewFocus::Pianoroll,
        ViewFocus::Pianoroll if !has_piano && has_arr => ViewFocus::Arrange,
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 聚焦视图决策：最近按下所在视图优先；焦点不可见时自动切换；MIX 下保持。
    #[test]
    fn view_focus_resolution() {
        let arr = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 300.0));
        let pr = egui::Rect::from_min_max(egui::pos2(0.0, 300.0), egui::pos2(800.0, 600.0));
        let in_arr = Some(egui::pos2(400.0, 150.0));
        let in_pr = Some(egui::pos2(400.0, 450.0));
        let outside = Some(egui::pos2(900.0, 100.0));

        // 指针在 PR / AR 内 → 对应视图
        assert_eq!(
            resolve_view_focus(ViewFocus::Arrange, in_pr, arr, pr, true, true),
            ViewFocus::Pianoroll
        );
        assert_eq!(
            resolve_view_focus(ViewFocus::Pianoroll, in_arr, arr, pr, true, true),
            ViewFocus::Arrange
        );
        // 指针在外 / 无指针 → 保持
        assert_eq!(
            resolve_view_focus(ViewFocus::Arrange, outside, arr, pr, true, true),
            ViewFocus::Arrange
        );
        assert_eq!(
            resolve_view_focus(ViewFocus::Pianoroll, None, arr, pr, true, true),
            ViewFocus::Pianoroll
        );
        // 焦点视图不可见 → 切到可见视图
        assert_eq!(
            resolve_view_focus(ViewFocus::Arrange, None, arr, pr, false, true),
            ViewFocus::Pianoroll
        );
        assert_eq!(
            resolve_view_focus(ViewFocus::Pianoroll, None, arr, pr, true, false),
            ViewFocus::Arrange
        );
        // 都不可见（MIX）→ 保持
        assert_eq!(
            resolve_view_focus(ViewFocus::Pianoroll, in_pr, arr, pr, false, false),
            ViewFocus::Pianoroll
        );
        // 指针落在不可见视图区域内 → 忽略
        assert_eq!(
            resolve_view_focus(ViewFocus::Arrange, in_pr, arr, pr, true, false),
            ViewFocus::Arrange
        );
    }

    /// 回归：右栏占位（Panel::right）先于 dock（Panel::bottom）时，
    /// 右栏占整条高度、dock 只占右栏左侧（与 PR 同规则）。
    #[test]
    fn right_panel_priority_over_bottom_dock() {
        let ctx = egui::Context::default();
        let mut right_rect = egui::Rect::NOTHING;
        let mut dock_rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                right_rect = show_right_panel_placeholder(ui, 300.0);
                dock_rect = egui::Panel::bottom("bottom_dock")
                    .exact_size(150.0)
                    .show(ui, |_| {})
                    .response
                    .rect;
            },
        );
        output.drop_without_applying_deltas();
        assert!(
            dock_rect.max.x <= right_rect.min.x + 0.5,
            "dock 应止于右栏左缘: dock={dock_rect:?} right={right_rect:?}"
        );
        assert!(
            right_rect.height() >= 690.0,
            "右栏应占整条高度（dock 不截断）: right={right_rect:?}"
        );
    }
}
