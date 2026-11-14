//! 工具手势（移动端子集）：PR 选框/铅笔/橡皮/刷子，AR 选框。
//!
//! 手形/平移由 Flutter 视口手势处理；本模块只处理"编辑类"工具。
//! 单指按下 → 编辑；双指 → Flutter 取消本模块拖拽后进入缩放/平移。
//! 所有提交都在释放时进行一次（一步一 undo），拖拽期间只更新覆盖层预览。

use std::collections::HashMap;

use yinhe_core::TrackKind;
use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::NoteSource;
use yinhe_types::automation::{AutomationEvent, AutomationTarget};

use crate::scene::{self, RenderScene, Tool, ViewMode};

mod commit;
mod geometry;
mod hit;
mod lifecycle;
mod overlay;
mod preview;

pub(crate) use commit::*;
pub(crate) use geometry::*;
pub(crate) use hit::*;
pub(crate) use lifecycle::*;
pub(crate) use overlay::*;
pub(crate) use preview::*;

/// 框选/拖动判定阈值（逻辑像素，与桌面一致）。
const DRAG_THRESHOLD: f32 = 3.0;

/// 已选框长按待定时，移动超过此距离就取消待定并转平移。
const PENDING_CANCEL: f32 = 12.0;

/// 音符右端改长度的命中宽度（逻辑像素）。
///
/// 6px 对手机太窄（约 2mm），成熟移动端编曲软件一般在 10~16dp；
/// 这里取 12dp，比 44pt 按钮标准小，但足以稳定按住音符末端。
const EDGE: f32 = 24.0;

/// 音符整体拖动的额外命中外扩（像素，仅时间轴方向）。
/// 触屏上按在音符边缘外一点也应该能拖动它。
const NOTE_HIT_PAD: f32 = 12.0;

/// 新音符默认力度（桌面无记忆时回退 100）。
const DEFAULT_VELOCITY: u8 = 100;

/// 铅笔“单音符精调选择 → 二次拖动移动”功能开关。
///
/// 交互与实现先保留（[`kind::PENCIL_PICK`]），当前暂时关闭：铅笔压在已有音符上
/// 仍按原行为新建音符。后续要启用时只改这里。
const PENCIL_PICK_ENABLED: bool = false;

/// 选框方块控制点命中半径（逻辑像素）。三个方块间距近，半径过大会互相抢。
const SEL_HANDLE_R: f32 = 18.0;

/// 拖拽类型。
mod kind {
    pub const NONE: u8 = 0;
    pub const MARQUEE: u8 = 1;
    pub const MOVE_BOX: u8 = 2;
    pub const NOTE_MOVE: u8 = 3;
    pub const NOTE_RESIZE: u8 = 4;
    pub const PENCIL: u8 = 5;
    pub const BRUSH: u8 = 6;
    pub const ERASE: u8 = 7;
    /// 选框内按下、长按未完成（等待圆环转满）。
    pub const PENDING: u8 = 8;
    /// 空白处按下、长按未完成（等待圆环转满后弹出粘贴菜单）。
    pub const PENDING_PASTE: u8 = 9;
    /// 空白处长按完成：松手时弹出粘贴菜单。
    pub const PASTE_READY: u8 = 10;
    /// 铅笔精调选择：手指在音符上滑动，松手选中最后命中的音符。
    pub const PENCIL_PICK: u8 = 11;
    /// 拖动选框右下角方块，按选区宽度重复 Riff。
    pub const RIFF_FILL: u8 = 13;
    /// 拖动选框右缘内的方块，整体改选中音符的 gate。
    pub const SEL_GATE: u8 = 15;
    /// 拖动选框右缘方块，整体缩放选中音符的时间跨度。
    pub const SEL_SCALE: u8 = 16;
    /// 直线工具：按量化列在起止点之间生成直线音符。
    pub const LINE: u8 = 14;
}

/// 整体拖动音符：`(track, start_tick, end_tick, velocity)`（key 由外层桶给出）。
pub(crate) type DragNote = (u16, u32, u32, u8);

/// 整体拖动音符按 key 分桶。
pub(crate) type DragByKey = Vec<Vec<DragNote>>;
/// 原位置隐藏键：`(track, start_tick, key)`。

#[derive(Clone)]
pub(crate) struct DragState {
    kind: u8,
    x0: f32,
    y0: f32,
    /// 最后指针位置（内容逻辑坐标；自动滚动时用于重算预览）。
    px: f32,
    py: f32,
    /// 内容锚点（拖动开始时吸附后的 tick/key/row；视图滚动后仍固定）。
    anchor_tick: f64,
    /// 按下点的**原始** tick（未吸附）：拖动位移用它计算整格步进，
    /// 避免按在半格上时一移动就跳两格。
    anchor_tick_raw: f64,
    anchor_key: u8,
    anchor_row: usize,
    /// 已选框的内容坐标范围（PR: tick/key，AR: tick/row）。
    box_ticks: Option<(f64, f64, u8, u8)>,
    box_rows: Option<(f64, f64, usize, usize)>,
    /// 单音符拖动：目标音符。
    track: u16,
    start_tick: u32,
    end_tick: u32,
    key: u8,
    side: u8,
    /// 大选区整体拖动的 ghost/隐藏集：后台物化后按 Arc 共享，
    /// 同一选区反复拖动不重算；单音符拖动保持 `None`。
    drag: Option<std::sync::Arc<crate::selection_materializer::DragData>>,
    /// 原位置隐藏集合（大选区时与 `drag.hidden` 相同；单音符拖动单独设置）。
    drag_hidden: Option<std::sync::Arc<yinhe_types::HiddenNotes>>,
    /// 整体拖动上一次 ghost 的 `(dt, dk, 视图 hash)`；未变化时跳过重建。
    last_ghost: Option<(i64, i32, u64)>,
    /// 预览音上次播放时的 key 偏移，避免同一位置重复触发。
    preview_last_dk: i32,
    /// 选框控制点：0=上、1=下、2=左、3=右、4=右下角 Riff。
    sel_edge: u8,
    /// PR 选框控制点拖动时保留的轨道范围。
    sel_tr_lo: u16,
    sel_tr_hi: u16,
    /// 铅笔：吸附后起点与当前终点。
    start_f: f64,
    cur_end: f64,
    /// 刷子：已落格与最后一次格。
    cells: Vec<(u32, u8)>,
    last_cell: (u32, u8),
    /// 本次长按来自时间标尺（插入落点微调只在标尺上开放）。
    from_ruler: bool,
}

impl Default for DragState {
    fn default() -> Self {
        Self {
            kind: kind::NONE,
            x0: 0.0,
            y0: 0.0,
            px: 0.0,
            py: 0.0,
            anchor_tick: 0.0,
            anchor_tick_raw: 0.0,
            anchor_key: 0,
            anchor_row: 0,
            box_ticks: None,
            box_rows: None,
            track: 0,
            start_tick: 0,
            end_tick: 0,
            key: 0,
            side: 0,
            drag: None,
            drag_hidden: None,
            last_ghost: None,
            preview_last_dk: 0,
            sel_edge: 0,
            sel_tr_lo: 0,
            sel_tr_hi: 0,
            start_f: 0.0,
            cur_end: 0.0,
            cells: Vec::new(),
            last_cell: (0, 0),
            from_ruler: false,
        }
    }
}

static DRAG: std::sync::Mutex<DragState> = std::sync::Mutex::new(DragState {
    kind: kind::NONE,
    x0: 0.0,
    y0: 0.0,
    px: 0.0,
    py: 0.0,
    anchor_tick: 0.0,
    anchor_tick_raw: 0.0,
    anchor_key: 0,
    anchor_row: 0,
    box_ticks: None,
    box_rows: None,
    track: 0,
    start_tick: 0,
    end_tick: 0,
    key: 0,
    side: 0,
    drag: None,
    drag_hidden: None,
    last_ghost: None,
    preview_last_dk: 0,
    sel_edge: 0,
    sel_tr_lo: 0,
    sel_tr_hi: 0,
    start_f: 0.0,
    cur_end: 0.0,
    cells: Vec::new(),
    last_cell: (0, 0),
    from_ruler: false,
});

// ── 几何 ──

pub(crate) struct PrGeom {
    ppu: f32,
    scroll_x: f32,
    scroll_y: f32,
    kh: f32,
    layout: yinhe_types::KeyLayout,
    kbw: f32,
}

impl PrGeom {
    fn new(view: &yinhe_types::PianoRollView) -> Self {
        Self {
            ppu: view.base.pixels_per_tick,
            scroll_x: view.base.scroll_x,
            scroll_y: view.base.scroll_y,
            kh: view.key_height.max(0.001),
            layout: view.layout,
            kbw: view.keyboard_width(),
        }
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        f64::from((x - self.kbw + self.scroll_x) / self.ppu.max(0.0001))
    }
    fn y_to_key(&self, y: f32) -> u8 {
        self.layout.y_to_key(y, self.kh, self.scroll_y) as u8
    }
    fn tick_to_x(&self, tick: f64) -> f32 {
        tick as f32 * self.ppu - self.scroll_x + self.kbw
    }
    fn key_to_y(&self, key: u8) -> f32 {
        self.layout.key_to_y(u16::from(key), self.kh, self.scroll_y)
    }
}

pub(crate) struct ArGeom {
    ppu: f32,
    scroll_x: f32,
    scroll_y: f32,
    lane_h: f32,
    panel_w: f32,
}

impl ArGeom {
    fn new(view: &yinhe_types::ArrangementView) -> Self {
        Self {
            ppu: view.base.pixels_per_tick,
            scroll_x: view.base.scroll_x,
            scroll_y: view.base.scroll_y,
            lane_h: view.lane_height().max(0.001),
            panel_w: view.base.left_panel_width,
        }
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        f64::from((x - self.panel_w + self.scroll_x) / self.ppu.max(0.0001))
    }
    fn tick_to_x(&self, tick: f64) -> f32 {
        tick as f32 * self.ppu - self.scroll_x + self.panel_w
    }
    fn y_to_row(&self, y: f32) -> usize {
        ((y + self.scroll_y) / self.lane_h).max(0.0) as usize
    }
    fn row_to_y(&self, row: usize) -> f32 {
        row as f32 * self.lane_h - self.scroll_y
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SelHandle {
    edge: u8,
    t0: f64,
    t1: f64,
    k0: u8,
    k1: u8,
    tr_lo: u16,
    tr_hi: u16,
    r0: usize,
    r1: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct RiffSteps {
    /// 横向延伸 tick（带符号；不是完整选区宽度的步数）。
    extent_x: i64,
    /// 纵向延伸行数/key 数（带符号；不是完整选区高度的步数）。
    extent_y: i64,
}

impl RiffSteps {
    fn is_zero(self) -> bool {
        self.extent_x == 0 && self.extent_y == 0
    }
}

pub(crate) type AutoGhostBucket = (f32, Vec<AutomationEvent>, Vec<u32>);

pub(crate) type NoteHit = (u16, u32, u32, u8, u8, u32);

#[cfg(test)]
mod tests;
