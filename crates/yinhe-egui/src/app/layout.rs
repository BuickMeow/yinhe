use eframe::egui;

use crate::app::App;

pub mod chord;
pub mod content;
pub mod sel_hint;

pub(crate) use sel_hint::SelHintInfo;

/// Layout geometry computed once per frame, shared by arrangement and pianoroll.
pub(in crate::app) struct LayoutInfo {
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub bottom_y: f32,
    pub right_panel_total_w: f32,
}

impl App {
    pub(in crate::app) fn compute_layout(&mut self, ui: &mut egui::Ui) -> LayoutInfo {
        let mut remaining = ui.available_rect_before_wrap();

        let has_arr = self.view_mode.show_transport() && self.workspace.active_doc.is_some();
        let has_piano = self
            .view_mode
            .show_pianoroll(self.show_pianoroll_in_arrange)
            && self.workspace.active_doc.is_some();

        let right_panel_total_w = if self.right_tab.is_some() {
            let max_w = (remaining.width() - 60.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0);
            let pw = (self.right_panel_width + 4.0)
                .clamp(crate::theme::RIGHT_PANEL_MIN_WIDTH + 4.0, max_w);
            self.right_panel_width = (pw - 4.0).max(crate::theme::RIGHT_PANEL_MIN_WIDTH);
            pw
        } else {
            0.0
        };
        remaining.max.x -= right_panel_total_w;

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
            right_panel_total_w,
        }
    }
}
