use eframe::egui;

use crate::app::App;

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
