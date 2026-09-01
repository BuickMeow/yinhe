//! 边缘自动滚动的共享实现（拖拽接近视口边缘时自动平移）。
//!
//! 复用 `selection/drag` 与 `time_ruler` 的 `MARGIN/BASE_SPEED` 逻辑，消除
//! 桌面/移动三份拷贝与 `time_ruler` 的局部重复。调用方只需提供 `rect` 与
//! `pos`，返回 `(dx,dy)` 并自行 `clamp`，保持原有“先加再 clamp 取实际位移”语义。

use eframe::egui;
use yinhe_types::PianoRollView;
use yinhe_types::view_base::TimelineViewBase;

/// 触发自动滚动的边缘阈值（像素）。
pub const MARGIN: f32 = 20.0;
/// 基础速度（像素/秒/像素越界），`delta = overshoot * BASE_SPEED * dt`。
pub const BASE_SPEED: f32 = 15.0;

/// 纯计算：指针接近 `rect` 边缘时的滚动速度 `(dx, dy)`。
pub fn delta(ui: &egui::Ui, rect: egui::Rect, pos: egui::Pos2) -> (f32, f32) {
    let dt = ui.input(|i| i.unstable_dt);
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;

    if pos.x < rect.min.x + MARGIN {
        dx = -(rect.min.x + MARGIN - pos.x) * BASE_SPEED * dt;
    } else if pos.x > rect.max.x - MARGIN {
        dx = (pos.x - (rect.max.x - MARGIN)) * BASE_SPEED * dt;
    }

    if pos.y < rect.min.y + MARGIN {
        dy = -(rect.min.y + MARGIN - pos.y) * BASE_SPEED * dt;
    } else if pos.y > rect.max.y - MARGIN {
        dy = (pos.y - (rect.max.y - MARGIN)) * BASE_SPEED * dt;
    }

    (dx, dy)
}

/// 基础版（`TimelineViewBase`）：AR 等横向视图使用。
pub fn auto_scroll_on_drag(
    ui: &egui::Ui,
    base: &mut TimelineViewBase,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut TimelineViewBase, f32, f32),
) -> (f32, f32) {
    let (dx, dy) = delta(ui, content_rect, pos);
    if dx != 0.0 || dy != 0.0 {
        let old_x = base.scroll_x;
        let old_y = base.scroll_y;
        base.scroll_x += dx;
        base.scroll_y += dy;
        clamp_fn(base, content_rect.width(), content_rect.height());
        let actual_dx = base.scroll_x - old_x;
        let actual_dy = base.scroll_y - old_y;
        if actual_dx != 0.0 || actual_dy != 0.0 {
            base.dirty = true;
            ui.ctx().request_repaint();
            return (actual_dx, actual_dy);
        }
    }
    (0.0, 0.0)
}

/// 方向感知版（`PianoRollView`）：PR 拖拽/铅笔/框选共用。
pub fn auto_scroll_on_drag_dir(
    ui: &egui::Ui,
    view: &mut PianoRollView,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut PianoRollView, f32, f32),
) -> (f32, f32) {
    let (dx, dy) = delta(ui, content_rect, pos);
    if dx != 0.0 || dy != 0.0 {
        let old_x = view.base.scroll_x;
        let old_y = view.base.scroll_y;
        view.base.scroll_x += dx;
        view.base.scroll_y += dy;
        clamp_fn(view, content_rect.width(), content_rect.height());
        let actual_dx = view.base.scroll_x - old_x;
        let actual_dy = view.base.scroll_y - old_y;
        if actual_dx != 0.0 || actual_dy != 0.0 {
            view.base.dirty = true;
            ui.ctx().request_repaint();
            return (actual_dx, actual_dy);
        }
    }
    (0.0, 0.0)
}
