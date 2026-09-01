//! 边缘自动滚动的共享实现（拖拽接近视口边缘时自动平移）。
//!
//! 复用 `selection/drag` 与 `time_ruler` 的 `MARGIN/BASE_SPEED` 逻辑，消除
//! 桌面/移动三份拷贝与 `time_ruler` 的局部重复。调用方只需提供 `rect` 与
//! `pos`，返回 `(dx,dy)` 并自行 `clamp`，保持原有“先加再 clamp 取实际位移”语义。

use eframe::egui;
use yinhe_types::PianoRollView;
use yinhe_types::view_base::TimelineViewBase;

/// 自动滚动配置：`delta = overshoot * base_speed * dt`，`overshoot` 为超出 `margin` 的像素。
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub margin: f32,
    pub base_speed: f32,
}

impl Config {
    pub const DESKTOP: Self = Self {
        margin: 20.0,
        base_speed: 15.0,
    };
    /// 移动端阈值放宽（屏幕边缘难精确按住）。
    #[allow(dead_code)]
    pub const MOBILE: Self = Self {
        margin: 48.0,
        base_speed: 15.0,
    };
}

/// 触发自动滚动的边缘阈值（像素，桌面）。
#[allow(dead_code)]
pub const MARGIN: f32 = Config::DESKTOP.margin;
/// 基础速度（像素/秒/像素越界，桌面）。
#[allow(dead_code)]
pub const BASE_SPEED: f32 = Config::DESKTOP.base_speed;

/// 纯计算：指针接近 `rect` 边缘时的滚动速度 `(dx, dy)`（桌面阈值）。
pub fn delta(ui: &egui::Ui, rect: egui::Rect, pos: egui::Pos2) -> (f32, f32) {
    delta_with_config(ui, rect, pos, Config::DESKTOP)
}

/// 带配置的 `delta`，供移动端或特殊阈值复用。
pub fn delta_with_config(
    ui: &egui::Ui,
    rect: egui::Rect,
    pos: egui::Pos2,
    cfg: Config,
) -> (f32, f32) {
    let dt = ui.input(|i| i.unstable_dt);
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;

    if pos.x < rect.min.x + cfg.margin {
        dx = -(rect.min.x + cfg.margin - pos.x) * cfg.base_speed * dt;
    } else if pos.x > rect.max.x - cfg.margin {
        dx = (pos.x - (rect.max.x - cfg.margin)) * cfg.base_speed * dt;
    }

    if pos.y < rect.min.y + cfg.margin {
        dy = -(rect.min.y + cfg.margin - pos.y) * cfg.base_speed * dt;
    } else if pos.y > rect.max.y - cfg.margin {
        dy = (pos.y - (rect.max.y - cfg.margin)) * cfg.base_speed * dt;
    }

    (dx, dy)
}

/// 可滚动目标的抽象：`TimelineViewBase` 与 `PianoRollView` 统一。
pub trait Scrollable {
    fn scroll_mut(&mut self) -> (&mut f32, &mut f32);
    fn dirty_mut(&mut self) -> &mut bool;
}

impl Scrollable for TimelineViewBase {
    fn scroll_mut(&mut self) -> (&mut f32, &mut f32) {
        (&mut self.scroll_x, &mut self.scroll_y)
    }
    fn dirty_mut(&mut self) -> &mut bool {
        &mut self.dirty
    }
}

impl Scrollable for PianoRollView {
    fn scroll_mut(&mut self) -> (&mut f32, &mut f32) {
        (&mut self.base.scroll_x, &mut self.base.scroll_y)
    }
    fn dirty_mut(&mut self) -> &mut bool {
        &mut self.base.dirty
    }
}

/// 泛型核心：对任意 `Scrollable` 做边缘滚动，`clamp` 由调用方注入（需 `total_ticks` 等上下文）。
pub fn auto_scroll<T: Scrollable>(
    ui: &egui::Ui,
    target: &mut T,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut T, f32, f32),
) -> (f32, f32) {
    auto_scroll_with_config(ui, target, content_rect, pos, Config::DESKTOP, clamp_fn)
}

/// 带配置的泛型 `auto_scroll`（移动端用 `Config::MOBILE`）。
pub fn auto_scroll_with_config<T: Scrollable>(
    ui: &egui::Ui,
    target: &mut T,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    cfg: Config,
    clamp_fn: impl FnOnce(&mut T, f32, f32),
) -> (f32, f32) {
    let (dx, dy) = delta_with_config(ui, content_rect, pos, cfg);
    if dx != 0.0 || dy != 0.0 {
        let (sx, sy) = target.scroll_mut();
        let old_x = *sx;
        let old_y = *sy;
        *sx += dx;
        *sy += dy;
        clamp_fn(target, content_rect.width(), content_rect.height());
        let (sx2, sy2) = target.scroll_mut();
        let actual_dx = *sx2 - old_x;
        let actual_dy = *sy2 - old_y;
        if actual_dx != 0.0 || actual_dy != 0.0 {
            *target.dirty_mut() = true;
            ui.ctx().request_repaint();
            return (actual_dx, actual_dy);
        }
    }
    (0.0, 0.0)
}

/// 基础版（`TimelineViewBase`）：AR 等横向视图使用（保留名兼容旧调用）。
pub fn auto_scroll_on_drag(
    ui: &egui::Ui,
    base: &mut TimelineViewBase,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut TimelineViewBase, f32, f32),
) -> (f32, f32) {
    auto_scroll(ui, base, content_rect, pos, clamp_fn)
}

/// 方向感知版（`PianoRollView`）：PR 拖拽/铅笔/框选共用（保留名兼容旧调用）。
pub fn auto_scroll_on_drag_dir(
    ui: &egui::Ui,
    view: &mut PianoRollView,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut PianoRollView, f32, f32),
) -> (f32, f32) {
    auto_scroll(ui, view, content_rect, pos, clamp_fn)
}

/// 仅主轴的自动滚动（时间标尺等只需单轴的场景）。
/// `orientation` 决定主轴：`Horizontal` 时仅 `dx` 生效，`Vertical` 时仅 `dy`。
pub fn auto_scroll_main(
    ui: &egui::Ui,
    rect: egui::Rect,
    pos: egui::Pos2,
    orientation: yinhe_types::Orientation,
) -> f32 {
    let (dx, dy) = delta(ui, rect, pos);
    match orientation {
        yinhe_types::Orientation::Horizontal => dx,
        yinhe_types::Orientation::Vertical => dy,
    }
}
