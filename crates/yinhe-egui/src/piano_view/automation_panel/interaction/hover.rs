use eframe::egui;

/// Hover/drag tooltip 数据。锚点和控制点用不同的显示内容。
#[derive(Clone, Copy, Debug)]
pub(crate) enum HoverTooltip {
    /// 锚点（或拖拽锚点）：显示 tick（小节:拍:tick）+ automation value
    Anchor {
        tick: u32,
        value: f32,
        pos: egui::Pos2,
    },
    /// 贝塞尔控制点（或拖拽控制点）：显示 CSS 风格 4 值 (x1, y1, x2, y2)
    ControlPoint {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        pos: egui::Pos2,
    },
}

/// 控制点端别：cubic Bézier 有两个控制点，分别是 P1（起点出）和 P2（终点入）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CtrlEnd {
    /// 第一控制点 P1 = P0 + (P3-P0) * (x1, y1)，对应 CSS `cubic-bezier(x1, y1, _, _)`
    Out,
    /// 第二控制点 P2 = P0 + (P3-P0) * (x2, y2)，对应 CSS `cubic-bezier(_, _, x2, y2)`
    In,
}

/// 命中曲线控制点的结果（`dist_sq` 仅用于选最近命中，调用方不消费）。
#[derive(Clone, Copy)]
pub(crate) struct ControlPointHit {
    pub(crate) prev_tick: u32,
    pub(crate) which: CtrlEnd,
    pub(crate) x1: f32,
    pub(crate) y1: f32,
    pub(crate) x2: f32,
    pub(crate) y2: f32,
    pub(crate) pos: egui::Pos2,
    pub(crate) dist_sq: f32,
}
