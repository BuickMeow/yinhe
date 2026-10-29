//! 工具手势（移动端子集）：PR 选框/铅笔/橡皮/刷子，AR 选框。
//!
//! 手形/平移由 Flutter 视口手势处理；本模块只处理"编辑类"工具。
//! 单指按下 → 编辑；双指 → Flutter 取消本模块拖拽后进入缩放/平移。
//! 所有提交都在释放时进行一次（一步一 undo），拖拽期间只更新覆盖层预览。

use std::collections::HashMap;

use yinhe_core::TrackKind;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::NoteSource;
use yinhe_types::automation::{AutomationEvent, AutomationTarget};

use crate::scene::{self, RenderScene, Tool, ViewMode};

/// 框选/拖动判定阈值（逻辑像素，与桌面一致）。
const DRAG_THRESHOLD: f32 = 3.0;
/// 已选框长按待定时，移动超过此距离就取消待定并转平移。
const PENDING_CANCEL: f32 = 12.0;
/// 音符边缘判定（逻辑像素，与桌面一致）。
const EDGE: f32 = 6.0;
/// 新音符默认力度（桌面无记忆时回退 100）。
const DEFAULT_VELOCITY: u8 = 100;
/// 铅笔“单音符精调选择 → 二次拖动移动”功能开关。
///
/// 交互与实现先保留（[`kind::PENCIL_PICK`]），当前暂时关闭：铅笔压在已有音符上
/// 仍按原行为新建音符。后续要启用时只改这里。
const PENCIL_PICK_ENABLED: bool = false;
/// 单音符直接拖拽/边缘伸缩开关。
///
/// 手机端直接按在音符上拖动太容易误触，当前关闭；保留选框长按后的整体移动。
/// 后续要恢复桌面式单音符拖拽时只改这里。
const DIRECT_NOTE_DRAG_ENABLED: bool = false;
/// 选框控制点命中半径（逻辑像素）。
const SEL_HANDLE_R: f32 = 14.0;

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
    /// 拖动选框四边控制点，微调选区范围。
    pub const SEL_RESIZE: u8 = 12;
    /// 拖动选框右下角方块，按选区宽度重复 Riff。
    pub const RIFF_FILL: u8 = 13;
    /// 直线工具：按量化列在起止点之间生成直线音符。
    pub const LINE: u8 = 14;
}

/// 整体拖动音符：`(track, start_tick, end_tick, velocity)`（key 由外层桶给出）。
type DragNote = (u16, u32, u32, u8);
/// 整体拖动音符按 key 分桶。
type DragByKey = Vec<Vec<DragNote>>;
/// 原位置隐藏键：`(track, start_tick, key)`。
type HiddenKey = (u16, u32, u8);

#[derive(Clone)]
struct DragState {
    kind: u8,
    x0: f32,
    y0: f32,
    /// 最后指针位置（内容逻辑坐标；自动滚动时用于重算预览）。
    px: f32,
    py: f32,
    /// 内容锚点（拖动开始时吸附后的 tick/key/row；视图滚动后仍固定）。
    anchor_tick: f64,
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
    /// 选区整体移动的预计算音符，按 key 分桶：每项 `(track, start, end)`。
    drag_by_key: DragByKey,
    /// 选区原位置隐藏集合（拖动开始时算一次，避免每帧重扫）。
    drag_hidden: Option<std::sync::Arc<std::collections::HashSet<HiddenKey>>>,
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
            anchor_key: 0,
            anchor_row: 0,
            box_ticks: None,
            box_rows: None,
            track: 0,
            start_tick: 0,
            end_tick: 0,
            key: 0,
            side: 0,
            drag_by_key: Vec::new(),
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
    anchor_key: 0,
    anchor_row: 0,
    box_ticks: None,
    box_rows: None,
    track: 0,
    start_tick: 0,
    end_tick: 0,
    key: 0,
    side: 0,
    drag_by_key: Vec::new(),
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
});

// ── 几何 ──

struct PrGeom {
    ppu: f32,
    scroll_x: f32,
    kh: f32,
    /// 键盘底边（key 0 行的底缘，含 scroll；与官方 `key_to_y` 同基准）。
    bottom: f32,
    kbw: f32,
}

impl PrGeom {
    fn new(view: &yinhe_types::PianoRollView) -> Self {
        let kh = view.key_height.max(0.001);
        Self {
            ppu: view.base.pixels_per_tick,
            scroll_x: view.base.scroll_x,
            kh,
            // key_to_y(0) = bottom - kh。
            bottom: view.key_to_y(0) + kh,
            kbw: view.keyboard_width(),
        }
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        f64::from((x - self.kbw + self.scroll_x) / self.ppu.max(0.0001))
    }
    /// 与官方 `PianoRollView::y_to_key` 同式（ceil - 1）。
    fn y_to_key(&self, y: f32) -> u8 {
        let key_f = ((self.bottom - y) / self.kh).clamp(0.0, 128.0);
        (key_f.ceil() as u8).saturating_sub(1)
    }
    fn tick_to_x(&self, tick: f64) -> f32 {
        tick as f32 * self.ppu - self.scroll_x + self.kbw
    }
    fn key_to_y(&self, key: u8) -> f32 {
        self.bottom - (f32::from(key) + 1.0) * self.kh
    }
}

struct ArGeom {
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
struct SelHandle {
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

/// 控制点命中：四边中点圆形，右下角方形。
fn handle_hit(x0: f32, y0: f32, x1: f32, y1: f32, x: f32, y: f32) -> Option<u8> {
    let (x0, x1) = (x0.min(x1), x0.max(x1));
    let (y0, y1) = (y0.min(y1), y0.max(y1));
    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.5;
    let near = |hx: f32, hy: f32| {
        let dx = x - hx;
        let dy = y - hy;
        dx * dx + dy * dy <= SEL_HANDLE_R * SEL_HANDLE_R
    };
    if (x - x1).abs() <= 10.0 && (y - y1).abs() <= 10.0 {
        return Some(4);
    }
    if near(cx, y0) {
        return Some(0);
    }
    if near(cx, y1) {
        return Some(1);
    }
    if near(x0, cy) {
        return Some(2);
    }
    if near(x1, cy) {
        return Some(3);
    }
    None
}

fn pr_selection_handle(s: &RenderScene, geom: &PrGeom, x: f32, y: f32) -> Option<SelHandle> {
    let doc = s.document.as_ref()?;
    if doc.edit.sel_rect.rects.len() != 1 {
        return None;
    }
    let (t0, t1, k0, k1) = doc.edit.sel_rect.rects[0];
    let (tr_lo, tr_hi) = doc
        .edit
        .selected
        .rects
        .first()
        .map(|r| (r.4, r.5))
        .unwrap_or((0, s.num_tracks.saturating_sub(1) as u16));
    let x0 = geom.tick_to_x(t0);
    let x1 = geom.tick_to_x(t1);
    let y0 = geom.key_to_y(k1);
    let y1 = geom.key_to_y(k0) + geom.kh;
    let edge = handle_hit(x0, y0, x1, y1, x, y)?;
    Some(SelHandle {
        edge,
        t0,
        t1,
        k0,
        k1,
        tr_lo,
        tr_hi,
        r0: 0,
        r1: 0,
    })
}

fn ar_selection_handle(s: &RenderScene, geom: &ArGeom, x: f32, y: f32) -> Option<SelHandle> {
    let doc = s.document.as_ref()?;
    if doc.edit.arr_sel_rect.len() != 1 {
        return None;
    }
    let (t0, t1, r0, r1) = doc.edit.arr_sel_rect[0];
    let x0 = geom.tick_to_x(t0);
    let x1 = geom.tick_to_x(t1);
    let y0 = geom.row_to_y(r0);
    let y1 = geom.row_to_y(r1 + 1);
    let edge = handle_hit(x0, y0, x1, y1, x, y)?;
    Some(SelHandle {
        edge,
        t0,
        t1,
        k0: 0,
        k1: yinhe_types::MAX_KEY,
        tr_lo: r0 as u16,
        tr_hi: r1 as u16,
        r0,
        r1,
    })
}

/// 工具写入目标轨：显示过滤轨 → 当前编辑轨 → 第一条非 conductor 轨。
fn write_track(s: &RenderScene) -> Option<usize> {
    let n = s.num_tracks;
    if n == 0 {
        return None;
    }
    // PR 只显示 Conductor 时没有可编辑音符。以前这里会落到第 1 轨，
    // 导致框选/落笔改到看不见的轨道上（还会画出别的轨的 ghost）。
    if s.display_track == Some(0) {
        return None;
    }
    let t = s.display_track.or(s.active_track).unwrap_or(1).min(n - 1);
    if t == 0 {
        if n > 1 { Some(1) } else { None }
    } else {
        Some(t)
    }
}

/// 当前视图量化档位的 tick 间隔（AM 选框按量化吸附用）。
pub fn quantize_tick_interval() -> u32 {
    let Ok(s) = scene::scene().lock() else {
        return 1;
    };
    quantize_of(&s).tick_interval(s.ppq).max(1)
}

/// 当前量化档位（Auto 按视图缩放解析）。
fn quantize_of(s: &RenderScene) -> QuantizePreset {
    let raw = s
        .document
        .as_ref()
        .map(|d| match s.view_mode {
            ViewMode::Pianoroll => d.edit.quantize_pianoroll,
            ViewMode::Arrangement => d.edit.quantize_arrange,
        })
        .unwrap_or(QuantizePreset::Auto);
    let ppu = match s.view_mode {
        ViewMode::Pianoroll => s.pr_view.base.pixels_per_tick,
        ViewMode::Arrangement => s.arr_view.base.pixels_per_tick,
    };
    let (num, den_pow) = s
        .document
        .as_ref()
        .map(|d| {
            d.data
                .model
                .tempo_map
                .time_sig_at_tick(s.cursor_tick.max(0.0) as u32)
        })
        .unwrap_or((4, 2));
    raw.resolved_with_time_sig(
        ppu,
        s.ppq,
        num,
        den_pow,
        matches!(s.view_mode, ViewMode::Pianoroll),
    )
}

fn bump(s: &mut RenderScene) {
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
}

// ── 拖拽生命周期 ──

fn begin_sel_handle(d: &mut DragState, s: &mut RenderScene, h: SelHandle) {
    d.kind = if h.edge == 4 {
        kind::RIFF_FILL
    } else {
        kind::SEL_RESIZE
    };
    d.sel_edge = h.edge;
    d.box_ticks = Some((h.t0, h.t1, h.k0, h.k1));
    d.box_rows = Some((h.t0, h.t1, h.r0, h.r1));
    d.sel_tr_lo = h.tr_lo;
    d.sel_tr_hi = h.tr_hi;
    if h.edge == 4 {
        // Riff 预览只画重复选框，不预先扫描/物化选中音符。
        d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::new()));
    }
    bump(s);
}

/// 单指按下：按当前工具开始编辑拖拽。
///
/// 返回 1 = 进入编辑；0 = 此处不响应编辑，Flutter 应把它当平移/点击处理。
pub fn begin(x: f32, y: f32) -> i32 {
    let Ok(mut d) = DRAG.lock() else {
        return 0;
    };
    *d = DragState {
        x0: x,
        y0: y,
        px: x,
        py: y,
        ..Default::default()
    };
    crate::audio::stop_preview();
    let Ok(mut s) = scene::scene().lock() else {
        return 0;
    };
    crate::selection_materializer::apply_pending(&mut s);
    if s.document.is_none() {
        return 0;
    }
    // 新一次按下：先清掉上一次可能残留的 ghost/hidden。
    s.preview.clear();
    let tool = s.tool;
    if tool == Tool::Pan {
        return 0; // 手形：平移交给 Flutter 视口手势
    }
    match s.view_mode {
        ViewMode::Pianoroll => begin_pr(&mut d, &mut s, x, y),
        ViewMode::Arrangement => begin_arr(&mut d, &mut s, x, y),
    }
    i32::from(d.kind != kind::NONE)
}

/// 单指移动更新拖拽。
///
/// 返回 1 = 仍在编辑；0 = 已选框待定被移动取消，后续应转平移。
pub fn update(x: f32, y: f32) -> i32 {
    let Ok(mut d) = DRAG.lock() else {
        return 0;
    };
    if d.kind == kind::NONE {
        return 0;
    }
    d.px = x;
    d.py = y;
    // 已选框长按未完成就移动：取消待定，交给 Flutter 平移。
    if d.kind == kind::PENDING {
        let moved = ((x - d.x0).powi(2) + (y - d.y0).powi(2)).sqrt();
        if moved >= PENDING_CANCEL {
            d.kind = kind::NONE;
            crate::audio::stop_preview();
            return 0;
        }
        return 1;
    }
    // 空白处长按未完成时开始拖动：转为普通框选，不能把长按当成粘贴。
    let mut started_marquee = false;
    if d.kind == kind::PENDING_PASTE {
        let moved = ((x - d.x0).powi(2) + (y - d.y0).powi(2)).sqrt();
        if moved >= DRAG_THRESHOLD {
            d.kind = kind::MARQUEE;
            started_marquee = true;
        }
    }
    let Ok(mut s) = scene::scene().lock() else {
        return 1;
    };
    if d.kind == kind::PASTE_READY {
        // 长按完成后仍可移动手指微调落点（指示线跟随）。
        update_paste_ready(&mut d, &s, x, y);
        return 1;
    }
    if started_marquee {
        // 新框选一开始就清掉旧选框/旧成员，而不是等拖拽结束才清。
        if let Some(doc) = s.document.as_mut() {
            doc.edit.selected.clear();
            doc.edit.sel_rect.clear();
            doc.edit.arr_sel_rect.clear();
            doc.edit.chan_ar_sel.clear();
            doc.edit.track_selected.clear();
        }
        bump(&mut s);
    }
    refresh_preview(&mut d, &mut s);
    1
}

/// 边缘自动滚动（Flutter 每帧调用）：指针贴近视口边缘时持续滚动视图。
pub fn tick_scroll() -> i32 {
    let Ok(mut d) = DRAG.lock() else {
        return 0;
    };
    if d.kind == kind::NONE {
        return 0;
    }
    // 长按待定/粘贴态不自动滚动；长按由 Flutter recognizer 推进。
    if matches!(
        d.kind,
        kind::PENDING | kind::PENDING_PASTE | kind::PASTE_READY
    ) {
        return 0;
    }
    let Ok(mut s) = scene::scene().lock() else {
        return 0;
    };
    let (lw, lh) = crate::logical_size();
    let (px, py) = (d.px, d.py);
    if lw < 1.0 || lh < 1.0 {
        return 0;
    }
    const ZONE: f32 = 28.0;
    // 垂直：内容区顶部/底部边缘。
    let mut vy = 0.0f32;
    if py < ZONE {
        vy = (ZONE - py) * 0.9;
    }
    if py > lh - ZONE {
        vy = -((py - (lh - ZONE)) * 0.9);
    }
    let total = s.total_ticks;
    let num_tracks = s.num_tracks;
    let moved = match s.view_mode {
        ViewMode::Pianoroll => {
            // 水平：以内容区左缘（键盘右缘）为判定基准；指针拖到键盘列
            // 甚至更左（区域外）也继续视为向左移动。
            let kbw = s.pr_view.keyboard_width();
            let left = kbw + ZONE;
            let mut vx = 0.0f32;
            if px < left {
                vx = ((left - px) * 0.9).min(30.0);
            }
            if px > lw - ZONE {
                vx = -((px - (lw - ZONE)) * 0.9).min(30.0);
            }
            let mut changed = false;
            if vx != 0.0 {
                *s.pr_view.main_scroll() -= vx;
                changed = true;
            }
            if vy != 0.0 {
                *s.pr_view.cross_scroll() -= vy;
                changed = true;
            }
            if changed {
                s.pr_view.clamp_scroll(lw, lh, total);
            }
            changed
        }
        ViewMode::Arrangement => {
            // 水平：以轨道面板右缘为判定基准；拖到面板上（区域外）也继续滚。
            let panel = s.arr_view.base.left_panel_width;
            let left = panel + ZONE;
            let mut vx = 0.0f32;
            if px < left {
                vx = ((left - px) * 0.9).min(30.0);
            }
            if px > lw - ZONE {
                vx = -((px - (lw - ZONE)) * 0.9).min(30.0);
            }
            let mut changed = false;
            if vx != 0.0 {
                s.arr_view.base.scroll_x -= vx;
                changed = true;
            }
            if vy != 0.0 {
                s.arr_view.base.scroll_y -= vy;
                changed = true;
            }
            if changed {
                s.arr_view.clamp_scroll(lw, lh, total, num_tracks + 1);
            }
            changed
        }
    };
    if !moved {
        return 0;
    }
    refresh_preview(&mut d, &mut s);
    1
}

/// PASTE_READY 期间按当前指针位置刷新落点（量化吸附）。
fn update_paste_ready(d: &mut DragState, s: &RenderScene, x: f32, y: f32) {
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            d.anchor_tick = snap.round(q, geom.x_to_tick(x));
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            d.anchor_tick = snap.round(q, geom.x_to_tick(x));
            d.anchor_row = geom.y_to_row(y).min(s.num_tracks.saturating_sub(1));
        }
    }
}

/// 长按落点 tick（Dart 每帧更新指示线用；非长按态返回当前记录值）。
pub fn long_press_tick() -> f64 {
    DRAG.lock().map(|d| d.anchor_tick).unwrap_or(0.0)
}

/// 长按落点轨道（AR = 行；PR = 当前写入轨；-1 = 无）。
pub fn long_press_track() -> i32 {
    let Ok(d) = DRAG.lock() else {
        return -1;
    };
    let Ok(s) = scene::scene().lock() else {
        return -1;
    };
    match s.view_mode {
        ViewMode::Arrangement => d.anchor_row as i32,
        ViewMode::Pianoroll => write_track(&s).map(|t| t as i32).unwrap_or(-1),
    }
}

/// 长按空白处的上下文（插入事件用）：
/// `{"tick":..,"track":..,"conductor":..,"pr":..}`；无上下文时为 `{}`。
pub fn long_press_context_json() -> String {
    let Ok(d) = DRAG.lock() else {
        return "{}".into();
    };
    if d.kind != kind::PASTE_READY && d.kind != kind::PENDING_PASTE {
        return "{}".into();
    }
    let Ok(s) = scene::scene().lock() else {
        return "{}".into();
    };
    let (track, is_pr) = match s.view_mode {
        ViewMode::Arrangement => (d.anchor_row, false),
        ViewMode::Pianoroll => match write_track(&s) {
            Some(t) => (t, true),
            None => (usize::MAX, true),
        },
    };
    let conductor = track == 0
        || s.model
            .as_ref()
            .and_then(|m| m.tracks.get(track))
            .is_some_and(|t| t.kind == yinhe_core::TrackKind::Master);
    let track_out: i64 = if track == usize::MAX {
        -1
    } else {
        track as i64
    };
    format!(
        "{{\"tick\":{},\"track\":{},\"conductor\":{},\"pr\":{}}}",
        d.anchor_tick, track_out, conductor, is_pr
    )
}

/// PR 单击：命中音符则删除，否则在量化位置插入一个音符。返回 1 = 有改动。
///
/// 只处理内容区（键盘列不响应）；描述符与现有编辑路径共用同一套轨道可见性。
pub fn tap_note(x: f32, y: f32) -> i32 {
    let Ok(mut s) = scene::scene().lock() else {
        return 0;
    };
    if s.view_mode != ViewMode::Pianoroll {
        return 0;
    }
    let geom = PrGeom::new(&s.pr_view);
    if x < geom.kbw {
        return 0;
    }
    let tick_f = geom.x_to_tick(x).max(0.0);
    let tick = tick_f.floor() as u32;
    let key = geom.y_to_key(y);
    let q = quantize_of(&s);
    let snap = scene::SnapCtx::new(&s);
    let start = snap.round(q, tick_f).max(0.0) as u32;
    let interval = q.tick_interval(s.ppq).max(1);
    let write = write_track(&s);
    let visible = s
        .document
        .as_ref()
        .map(|d| d.edit.pr_visible())
        .unwrap_or_default();
    let only = s.display_track;
    // 命中：同一个 key 上，起点 <= tick < 终点，且轨道可见。
    let mut hit: Option<(u16, u32)> = None;
    if let Some(doc) = s.document.as_ref() {
        for n in doc.data.model.key_notes_in_range(key, tick, tick + 1) {
            if n.start_tick <= tick
                && tick < n.end_tick
                && visible.get(n.track as usize).copied().unwrap_or(true)
                && only.is_none_or(|o| o == n.track as usize)
            {
                hit = Some((n.track, n.start_tick));
                break;
            }
        }
    }
    if hit.is_none() && write.is_none() {
        return 0;
    }
    let edited = {
        let Some(doc) = s.document.as_mut() else {
            return 0;
        };
        let before = doc.capture_snapshot();
        if let Some((track, note_start)) = hit {
            doc.delete_notes(&[(track, note_start, key)]).map(|action| {
                doc.push_undo(action, "undo.delete_note", before);
            })
        } else {
            let Some(track) = write else {
                return 0;
            };
            let note = yinhe_core::NoteEvent {
                id: 0,
                key,
                start_tick: start,
                end_tick: start.saturating_add(interval),
                velocity: DEFAULT_VELOCITY,
            };
            doc.add_note(track as u16, note).map(|action| {
                doc.push_undo(action, "undo.add_note", before);
            })
        }
    };
    if edited.is_none() {
        return 0;
    }
    let model = after_edit(&mut s);
    drop(s);
    if let Some(model) = model {
        crate::audio::update_notes(model);
    }
    1
}

/// 长按位置上下文（不进入拖拽/插入微调状态，只用于弹菜单）。
pub fn context_at(x: f32, y: f32) -> String {
    let Ok(s) = scene::scene().lock() else {
        return "{}".into();
    };
    let q = quantize_of(&s);
    let snap = scene::SnapCtx::new(&s);
    let (tick, track, pr) = match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            (snap.round(q, geom.x_to_tick(x)), write_track(&s), true)
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            (
                snap.round(q, geom.x_to_tick(x)),
                Some(geom.y_to_row(y)),
                false,
            )
        }
    };
    let track_out: i64 = track.map(|t| t as i64).unwrap_or(-1);
    let conductor = track == Some(0)
        || track
            .and_then(|t| s.model.as_ref().and_then(|m| m.tracks.get(t)))
            .is_some_and(|t| t.kind == yinhe_core::TrackKind::Master);
    format!(
        "{{\"tick\":{},\"track\":{},\"conductor\":{},\"pr\":{}}}",
        tick, track_out, conductor, pr
    )
}

/// 选区整体移动的当前增量：PR 返回 `(tick 差, dk, 0)`，AR 返回 `(tick 差, 0, dtr)`。
///
/// tick 差保留未取整值（选框矩形预览用），ghost/提交再 round；已按选框
/// 左边界 clamp 到 >= 0。预览、提交、顶部栏读数共用这一份换算，避免漂移。
fn move_box_delta(s: &RenderScene, d: &DragState, x: f32, y: f32) -> Option<(f64, i32, i32)> {
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    match s.view_mode {
        ViewMode::Pianoroll => {
            let (t0, _, _, _) = d.box_ticks?;
            let geom = PrGeom::new(&s.pr_view);
            let mut dt = snap.round(q, geom.x_to_tick(x)) - d.anchor_tick;
            if t0 + dt < 0.0 {
                dt = -t0;
            }
            let dk = i32::from(geom.y_to_key(y)) - i32::from(d.anchor_key);
            Some((dt, dk, 0))
        }
        ViewMode::Arrangement => {
            let (t0, _, _, _) = d.box_rows?;
            let geom = ArGeom::new(&s.arr_view);
            let mut dt = snap.round(q, geom.x_to_tick(x)) - d.anchor_tick;
            if t0 + dt < 0.0 {
                dt = -t0;
            }
            let dtr = geom.y_to_row(y) as i32 - d.anchor_row as i32;
            Some((dt, 0, dtr))
        }
    }
}

/// 选框拖拽状态：0=无、1=待定（长按中）、2=移动态、3=空白处长按完成（待粘贴）。
pub fn selection_state() -> i32 {
    DRAG.lock()
        .map(|d| match d.kind {
            kind::PENDING | kind::PENDING_PASTE => 1,
            kind::MOVE_BOX => 2,
            kind::PASTE_READY => 3,
            _ => 0,
        })
        .unwrap_or(0)
}

/// 当前选框整体移动增量（顶部栏读数用）。返回 1 = 正在 MOVE_BOX。
///
/// 与 `refresh_preview` / `commit_move` 共用 [`move_box_delta`]，读数与实际
/// 落点一致（包含量化吸附和左边界 clamp）。
#[allow(clippy::not_unsafe_ptr_arg_deref)] // dart:ffi 入口，指针有效性由调用方保证
pub fn drag_delta(out_dt: *mut i64, out_dk: *mut i32, out_dtr: *mut i32) -> i32 {
    let Ok(d) = DRAG.lock() else {
        return 0;
    };
    if d.kind != kind::MOVE_BOX {
        return 0;
    }
    let Ok(s) = scene::scene().lock() else {
        return 0;
    };
    let Some((dt, dk, dtr)) = move_box_delta(&s, &d, d.px, d.py) else {
        return 0;
    };
    unsafe {
        if !out_dt.is_null() {
            *out_dt = dt.round() as i64;
        }
        if !out_dk.is_null() {
            *out_dk = dk;
        }
        if !out_dtr.is_null() {
            *out_dtr = dtr;
        }
    }
    1
}

/// Flutter 长按识别完成：待定选框进入移动态，空白处进入粘贴态。成功 1。
pub fn long_press() -> i32 {
    let Ok(mut d) = DRAG.lock() else {
        return 0;
    };
    let Ok(mut s) = scene::scene().lock() else {
        return 0;
    };
    match d.kind {
        kind::PENDING => {
            d.kind = kind::MOVE_BOX;
            // 长按成立时立即给出 0 位移 ghost，不必等第一次 move。
            refresh_preview(&mut d, &mut s);
            1
        }
        kind::PENDING_PASTE => {
            d.kind = kind::PASTE_READY;
            bump(&mut s);
            1
        }
        _ => 0,
    }
}

/// 橡皮框命中预览：不隐藏原音符，只把将被删除的音符用选中态叠加。
fn build_erase_preview(s: &mut RenderScene, d: &DragState, x: f32, y: f32) {
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    let Some(doc) = s.document.as_ref() else {
        s.preview.clear();
        return;
    };
    let mut ghost: Vec<(u32, u32, u8, u16)> = Vec::new();
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            let ta = snap.round(q, geom.x_to_tick(d.x0.min(x))) as u32;
            let mut tb = snap.round(q, geom.x_to_tick(d.x0.max(x))) as u32;
            if tb <= ta {
                tb = ta + q.tick_interval(s.ppq).max(1);
            }
            let ka = geom.y_to_key(d.y0.max(y));
            let kb = geom.y_to_key(d.y0.min(y));
            let (klo, khi) = (ka.min(kb), ka.max(kb));
            let write = write_track(s);
            for key in klo..=khi {
                for n in doc.data.model.key_notes_in_range(key, ta, tb) {
                    if n.start_tick < ta || n.start_tick >= tb {
                        continue;
                    }
                    if let Some(w) = write
                        && n.track as usize != w
                    {
                        continue;
                    }
                    ghost.push((n.start_tick, n.end_tick, key, n.track));
                }
            }
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            let ta = snap.round(q, geom.x_to_tick(d.x0.min(x))) as u32;
            let mut tb = snap.round(q, geom.x_to_tick(d.x0.max(x))) as u32;
            if tb <= ta {
                tb = ta + q.tick_interval(s.ppq).max(1);
            }
            let ra = geom.y_to_row(d.y0.min(y));
            let rb = geom.y_to_row(d.y0.max(y));
            let max_row = s.num_tracks.saturating_sub(1);
            let (rlo, rhi) = (ra.min(rb).min(max_row), ra.max(rb).min(max_row));
            for key in 0..=yinhe_types::MAX_KEY {
                for n in doc.data.model.key_notes_in_range(key, ta, tb) {
                    if n.start_tick < ta || n.start_tick >= tb {
                        continue;
                    }
                    let row = n.track as usize;
                    if row < rlo || row > rhi {
                        continue;
                    }
                    ghost.push((n.start_tick, n.end_tick, key, n.track));
                }
            }
        }
    }
    apply_preview(s, d, ghost, true);
}

/// 播放单个音符预览（`duration=0` = 持续到 stop）。
fn preview_note(
    s: &RenderScene,
    track: u16,
    key: u8,
    velocity: u8,
    target_tick: u32,
    duration_ticks: u32,
) {
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    let Some(t) = doc.data.model.tracks.get(track as usize) else {
        return;
    };
    crate::audio::preview_notes(
        vec![yinhe_audio::PreviewNoteParams {
            channel: t.global_channel(),
            key,
            velocity,
            target_tick,
            duration_ticks,
        }],
        true,
    );
}

fn note_velocity(s: &RenderScene, track: u16, start_tick: u32, key: u8) -> Option<u8> {
    let doc = s.document.as_ref()?;
    doc.data
        .model
        .key_notes(key)
        .iter()
        .find(|n| n.track == track && n.start_tick == start_tick)
        .map(|n| n.velocity)
}

/// 精确匹配 `(track, start, end, key)` 的音符。
fn note_at_bounds(
    s: &RenderScene,
    track: u16,
    start_tick: u32,
    end_tick: u32,
    key: u8,
) -> Option<yinhe_types::Note> {
    let doc = s.document.as_ref()?;
    doc.data
        .model
        .key_notes(key)
        .iter()
        .find(|n| n.track == track && n.start_tick == start_tick && n.end_tick == end_tick)
        .copied()
}

/// 把单个音符设为唯一选中：选区矩形覆盖该音符，但成员位图只含它的 id，
/// 避免 gate 内其他起点被矩形顺带选中。
fn select_note(s: &mut RenderScene, track: u16, start_tick: u32, end_tick: u32, key: u8) {
    let Some(note) = note_at_bounds(s, track, start_tick, end_tick, key) else {
        return;
    };
    let Some(doc) = s.document.as_mut() else {
        return;
    };
    let end = end_tick.max(start_tick.saturating_add(1));
    doc.edit.selected.clear();
    doc.edit.sel_rect.clear();
    doc.edit
        .sel_rect
        .push_rect((f64::from(start_tick), f64::from(end), key, key), false);
    doc.edit
        .selected
        .add_rect_track(start_tick, end, key, key, track, track);
    doc.edit.selected.set_members([note.id]);
}

/// 选区整体移动的听觉预览：只取当前视口内目标音符，避免超大选区全量送进音频线程。
fn preview_group(s: &RenderScene, d: &DragState, dt: i64, dk: i32) {
    if s.view_mode != ViewMode::Pianoroll {
        return;
    }
    let (lw, lh) = crate::logical_size();
    let (tick_lo, tick_hi) = s
        .pr_view
        .visible_main_range(s.pr_view.main_axis_len(lw, lh));
    let (key_lo, key_hi) = s
        .pr_view
        .visible_cross_range(s.pr_view.cross_axis_len(lw, lh));
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    let src_lo = (i32::from(key_lo) - dk).clamp(0, 127) as usize;
    let src_hi = (i32::from(key_hi) - dk).clamp(0, 127) as usize;
    let mut notes = Vec::new();
    for src_key in src_lo..=src_hi {
        let new_key = (src_key as i32 + dk).clamp(0, 127) as u8;
        if new_key < key_lo || new_key > key_hi {
            continue;
        }
        for &(track, start, end, velocity) in &d.drag_by_key[src_key] {
            let new_start = (i64::from(start) + dt).max(0) as u32;
            let new_end = new_start.saturating_add(end.saturating_sub(start));
            if f64::from(new_start) >= tick_hi || f64::from(new_end) <= tick_lo {
                continue;
            }
            let Some(t) = doc.data.model.tracks.get(track as usize) else {
                continue;
            };
            notes.push(yinhe_audio::PreviewNoteParams {
                channel: t.global_channel(),
                key: new_key,
                velocity,
                target_tick: new_start,
                duration_ticks: end.saturating_sub(start),
            });
        }
    }
    crate::audio::preview_notes(notes, true);
}

fn apply_preview(
    s: &mut RenderScene,
    d: &DragState,
    ghost: Vec<(u32, u32, u8, u16)>,
    selected: bool,
) {
    if let Some(hidden) = &d.drag_hidden {
        s.preview.set_hidden_arc(std::sync::Arc::clone(hidden));
    } else {
        s.preview.clear_hidden();
    }
    s.preview.set_ghost(ghost, selected);
}

fn view_tick_at(s: &RenderScene, x: f32) -> f64 {
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            snap.round(q, geom.x_to_tick(x)).max(0.0)
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            snap.round(q, geom.x_to_tick(x)).max(0.0)
        }
    }
}

fn update_sel_resize(d: &DragState, s: &mut RenderScene, x: f32, y: f32) {
    let Some((t0, t1, k0, k1)) = d.box_ticks else {
        return;
    };
    let cur_tick = view_tick_at(s, x);
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            let cur_key = geom.y_to_key(y);
            let (mut nt0, mut nt1, mut nk0, mut nk1) = (t0, t1, k0, k1);
            match d.sel_edge {
                0 => nk1 = cur_key.max(nk0),
                1 => nk0 = cur_key.min(nk1),
                2 => nt0 = cur_tick.min(nt1 - 1.0).max(0.0),
                3 => nt1 = cur_tick.max(nt0 + 1.0),
                _ => return,
            }
            nt0 = nt0.max(0.0);
            nt1 = nt1.max(nt0 + 1.0);
            if let Some(doc) = s.document.as_mut()
                && let Some(rect) = doc.edit.sel_rect.rects.first_mut()
            {
                *rect = (nt0, nt1, nk0, nk1);
            }
            bump(s);
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            let cur_row = geom.y_to_row(y).min(s.num_tracks.saturating_sub(1));
            let (mut nt0, mut nt1, mut nr0, mut nr1) = (
                t0,
                t1,
                d.box_rows.map_or(0, |r| r.2),
                d.box_rows.map_or(0, |r| r.3),
            );
            match d.sel_edge {
                // AR 行号向下增大：上控制点改 r0，下控制点改 r1。
                0 => nr0 = cur_row.min(nr1),
                1 => nr1 = cur_row.max(nr0),
                2 => nt0 = cur_tick.min(nt1 - 1.0).max(0.0),
                3 => nt1 = cur_tick.max(nt0 + 1.0),
                _ => return,
            }
            nt0 = nt0.max(0.0);
            nt1 = nt1.max(nt0 + 1.0);
            if let Some(doc) = s.document.as_mut()
                && let Some(rect) = doc.edit.arr_sel_rect.first_mut()
            {
                *rect = (nt0, nt1, nr0, nr1);
            }
            bump(s);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RiffSteps {
    sx: i64,
    sy: i64,
}

impl RiffSteps {
    fn is_zero(self) -> bool {
        self.sx == 0 && self.sy == 0
    }
}

/// 带符号的 `[0, ±1, .., ±N]`；二维铺满整个矩形，调用方跳过 `(0, 0)`。
fn riff_step_offsets(steps: i64) -> Vec<i64> {
    let mut out = vec![0];
    if steps != 0 {
        out.extend((1..=steps.unsigned_abs()).map(|i| i as i64 * steps.signum()));
    }
    out
}

fn riff_steps_x(d: &DragState, s: &RenderScene, x: f32) -> i64 {
    let Some((t0, t1, _, _)) = d.box_ticks else {
        return 0;
    };
    let width = (t1 - t0).max(1.0);
    let cur = view_tick_at(s, x);
    if cur > t1 {
        ((cur - t1) / width).ceil() as i64
    } else if cur < t0 {
        let max_left = (t0 / width).floor() as i64;
        (-(((t0 - cur) / width).ceil() as i64)).max(-max_left)
    } else {
        0
    }
}

/// 纵向步数（轴坐标）：PR 正 = key 增大（屏幕向上），AR 正 = track 增大（屏幕向下）。
///
/// 按 key/轨道边界夹住，避免在完全越界的方向上生成预览/复制步。
fn riff_steps_y(d: &DragState, s: &RenderScene, y: f32) -> i64 {
    match s.view_mode {
        ViewMode::Pianoroll => {
            let Some((_, _, k0, k1)) = d.box_ticks else {
                return 0;
            };
            let span = (i64::from(k1) - i64::from(k0) + 1).max(1);
            let cur = i64::from(PrGeom::new(&s.pr_view).y_to_key(y));
            let raw = if cur < i64::from(k0) {
                -(((i64::from(k0) - cur) as f64 / span as f64).ceil() as i64)
            } else if cur > i64::from(k1) {
                ((cur - i64::from(k1)) as f64 / span as f64).ceil() as i64
            } else {
                0
            };
            let min = -(i64::from(k1) / span);
            let max = (i64::from(yinhe_types::MAX_KEY) - i64::from(k0)) / span;
            raw.clamp(min, max)
        }
        ViewMode::Arrangement => {
            let Some((t0, t1, _, _)) = d.box_ticks else {
                return 0;
            };
            let (_, _, r0, r1) = d.box_rows.unwrap_or((t0, t1, 0, 0));
            let span = (r1 - r0 + 1).max(1) as i64;
            let cur = ArGeom::new(&s.arr_view).y_to_row(y) as i64;
            let raw = if cur > r1 as i64 {
                ((cur - r1 as i64) as f64 / span as f64).ceil() as i64
            } else if cur < r0 as i64 {
                -((r0 as i64 - cur) as f64 / span as f64).ceil() as i64
            } else {
                0
            };
            let last = s.num_tracks.saturating_sub(1) as i64;
            let min = -(r0 as i64 / span);
            let max = (last - r1 as i64) / span;
            raw.clamp(min, max)
        }
    }
}

fn riff_steps(d: &DragState, s: &RenderScene, x: f32, y: f32) -> RiffSteps {
    RiffSteps {
        sx: riff_steps_x(d, s, x),
        sy: riff_steps_y(d, s, y),
    }
}

fn update_riff_fill(d: &mut DragState, s: &mut RenderScene, x: f32, y: f32) {
    let steps = riff_steps(d, s, x, y);
    if steps.is_zero() {
        d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::new()));
        s.overlay_cells.clear();
        s.preview.clear();
        bump(s);
        return;
    }
    let Some((t0, t1, k0, k1)) = d.box_ticks else {
        return;
    };
    let width = (t1 - t0).max(1.0);
    let (lw, lh) = crate::logical_size();
    let tick_lo = match s.view_mode {
        ViewMode::Pianoroll => {
            s.pr_view
                .visible_main_range(s.pr_view.main_axis_len(lw, lh))
                .0
        }
        ViewMode::Arrangement => s.arr_view.visible_tick_range(lw).0,
    };
    let tick_hi = match s.view_mode {
        ViewMode::Pianoroll => {
            s.pr_view
                .visible_main_range(s.pr_view.main_axis_len(lw, lh))
                .1
        }
        ViewMode::Arrangement => s.arr_view.visible_tick_range(lw).1,
    };
    let xs = riff_step_offsets(steps.sx);
    let ys = riff_step_offsets(steps.sy);
    let mut rects = Vec::new();
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            let span = i64::from(k1) - i64::from(k0) + 1;
            for &sx in &xs {
                for &sy in &ys {
                    if sx == 0 && sy == 0 {
                        continue;
                    }
                    let offset = width * sx as f64;
                    let copy_lo = t0 + offset;
                    let copy_hi = t1 + offset;
                    if copy_hi <= tick_lo || copy_lo >= tick_hi {
                        continue;
                    }
                    let raw_k0 = i64::from(k0) + sy * span;
                    let raw_k1 = i64::from(k1) + sy * span;
                    if raw_k1 < 0 || raw_k0 > i64::from(yinhe_types::MAX_KEY) {
                        continue;
                    }
                    let nk0 = raw_k0.max(0) as u8;
                    let nk1 = raw_k1.min(i64::from(yinhe_types::MAX_KEY)) as u8;
                    if nk0 > nk1 {
                        continue;
                    }
                    let y0 = geom.key_to_y(nk1);
                    let y1 = geom.key_to_y(nk0) + geom.kh;
                    rects.push([geom.tick_to_x(copy_lo), y0, geom.tick_to_x(copy_hi), y1]);
                }
            }
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            let (_, _, r0, r1) = d.box_rows.unwrap_or((t0, t1, 0, 0));
            let span = (r1 - r0 + 1) as i64;
            let last = s.num_tracks.saturating_sub(1) as i64;
            let conductor = s.document.as_ref().is_some_and(|d| {
                d.data
                    .model
                    .tracks
                    .first()
                    .is_some_and(|t| t.kind == yinhe_core::TrackKind::Master)
            });
            for &sx in &xs {
                for &sy in &ys {
                    if sx == 0 && sy == 0 {
                        continue;
                    }
                    let offset = width * sx as f64;
                    let copy_lo = t0 + offset;
                    let copy_hi = t1 + offset;
                    if copy_hi <= tick_lo || copy_lo >= tick_hi {
                        continue;
                    }
                    let raw_r0 = r0 as i64 + sy * span;
                    let raw_r1 = r1 as i64 + sy * span;
                    if raw_r1 < 0 || raw_r0 > last {
                        continue;
                    }
                    let mut nr0 = raw_r0.max(0) as usize;
                    let nr1 = raw_r1.min(last) as usize;
                    if conductor {
                        nr0 = nr0.max(1);
                    }
                    if nr0 > nr1 {
                        continue;
                    }
                    let y0 = geom.row_to_y(nr0);
                    let y1 = geom.row_to_y(nr1 + 1);
                    rects.push([geom.tick_to_x(copy_lo), y0, geom.tick_to_x(copy_hi), y1]);
                }
            }
        }
    }
    s.overlay_cells = rects;
    s.preview.clear();
    bump(s);
}

/// 按最新指针位置与视图刷新拖拽预览（选框/移动框实时吸附量化网格）。
/// AR ghost 目标轨道：与 `offset_track_skip_conductor` 同规则。
fn ghost_track(src: u16, delta: i32, num_tracks: i32, conductor: Option<u16>) -> u16 {
    if delta == 0 || num_tracks <= 0 {
        return src;
    }
    let raw = (i32::from(src) + delta).clamp(0, num_tracks - 1);
    if Some(raw as u16) == conductor {
        if delta < 0 {
            (raw + 1).min(num_tracks - 1) as u16
        } else {
            (raw - 1).max(0) as u16
        }
    } else {
        raw as u16
    }
}

type AutoGhostBucket = (f32, Vec<AutomationEvent>, Vec<u32>);

/// 构建 AR 自动化竖线 ghost：选中锚点按 `(dt, dtr)` 移动后的副本。
fn build_ar_auto_ghost(s: &RenderScene, dt: i64, dtr: i32) -> Vec<scene::AutoGhostLane> {
    let Some(doc) = s.document.as_ref() else {
        return Vec::new();
    };
    let num_tracks = s.num_tracks as i32;
    if num_tracks <= 0 {
        return Vec::new();
    }
    let conductor = doc.edit.track_cache.conductor_idx;

    let mut selected_tracks = std::collections::HashSet::new();
    for &(_t0, _t1, _k0, _k1, tl, th) in &doc.edit.selected.rects {
        for t in tl..=th {
            selected_tracks.insert(t as usize);
        }
    }

    // key 含源轨：跨轨移动被夹到同一目标轨时仍能分别隐藏各自的原竖线。
    let mut by_lane: HashMap<(u16, AutomationTarget, u16), AutoGhostBucket> = HashMap::new();
    for ti in selected_tracks {
        let Some(track) = doc.data.model.tracks.get(ti) else {
            continue;
        };
        if track.kind == TrackKind::Master {
            continue;
        }
        if !doc.edit.track_visible.get(ti).copied().unwrap_or(true) {
            continue;
        }
        for lane in &track.automation_lanes {
            let mut events = Vec::new();
            let mut src_ticks = Vec::new();
            for ev in &lane.events {
                if !doc
                    .edit
                    .selected
                    .accepts_automation_event(ti as u16, &lane.target, ev)
                {
                    continue;
                }
                let tick = (i64::from(ev.tick) + dt).max(0) as u32;
                events.push(AutomationEvent { tick, ..*ev });
                src_ticks.push(ev.tick);
            }
            if events.is_empty() {
                continue;
            }
            let new_track = ghost_track(ti as u16, dtr, num_tracks, conductor);
            let max_val = if lane.target == AutomationTarget::Tempo {
                lane.events
                    .iter()
                    .map(|e| e.value)
                    .fold(0.0_f32, f32::max)
                    .max(1.0)
            } else {
                1.0
            };
            by_lane
                .entry((new_track, lane.target.clone(), ti as u16))
                .or_insert((max_val, Vec::new(), Vec::new()))
                .1
                .extend(events);
            by_lane
                .get_mut(&(new_track, lane.target.clone(), ti as u16))
                .expect("entry inserted above")
                .2
                .extend(src_ticks);
        }
    }

    by_lane
        .into_iter()
        .map(
            |((track, target, src_track), (max_val, events, mut src_ticks))| {
                src_ticks.sort_unstable();
                scene::AutoGhostLane {
                    track,
                    target,
                    max_val,
                    events,
                    src_track,
                    src_ticks,
                }
            },
        )
        .collect()
}

/// 直线工具：起点 → 当前点的量化列插值。
///
/// 时间方向每列一个音符（gate = 一个量化间隔），音高沿起止 key 线性插值取整，
/// 与刷子共用同一套预览/提交路径（`d.cells`）。
fn line_cells(d: &DragState, s: &RenderScene, x: f32, y: f32) -> Vec<(u32, u8)> {
    let geom = PrGeom::new(&s.pr_view);
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    let interval = q.tick_interval(s.ppq).max(1) as f64;
    let end_tick = snap.round(q, geom.x_to_tick(x).max(0.0)).max(0.0);
    let end_key = geom.y_to_key(y);
    let start_idx = (d.anchor_tick / interval).floor().max(0.0) as i64;
    let end_idx = (end_tick / interval).floor().max(0.0) as i64;
    let span = (end_idx - start_idx).abs();
    let (lo, hi) = if start_idx <= end_idx {
        (start_idx, end_idx)
    } else {
        (end_idx, start_idx)
    };
    let k0 = f64::from(d.anchor_key);
    let k1 = f64::from(end_key);
    let mut cells = Vec::with_capacity((hi - lo + 1) as usize);
    for idx in lo..=hi {
        let f = if span == 0 {
            0.0
        } else {
            (idx - start_idx) as f64 / span as f64
        };
        let key = (k0 + (k1 - k0) * f).round().clamp(0.0, 127.0) as u8;
        cells.push((idx as u32, key));
    }
    cells
}

/// 把直线格子推给 ghost 层（gate = 一个量化间隔）。
fn preview_line(s: &mut RenderScene, d: &mut DragState) {
    let interval = quantize_of(s).tick_interval(s.ppq).max(1);
    if let Some(track) = write_track(s) {
        let ghost: Vec<(u32, u32, u8, u16)> = d
            .cells
            .iter()
            .map(|&(idx, key)| {
                let start = idx.saturating_mul(interval);
                (start, start.saturating_add(interval), key, track as u16)
            })
            .collect();
        apply_preview(s, d, ghost, false);
    } else {
        s.preview.clear();
    }
}

fn refresh_preview(d: &mut DragState, s: &mut RenderScene) {
    let (x, y) = (d.px, d.py);
    let (lw, lh) = crate::logical_size();
    let snap = scene::SnapCtx::new(s);
    match d.kind {
        kind::MARQUEE | kind::ERASE => {
            let b = match s.view_mode {
                ViewMode::Pianoroll => {
                    let geom = PrGeom::new(&s.pr_view);
                    let q = quantize_of(s);
                    let interval = f64::from(q.tick_interval(s.ppq).max(1));
                    let cur = snap.round(q, geom.x_to_tick(x));
                    let ta = d.anchor_tick.min(cur).max(0.0);
                    let mut tb = d.anchor_tick.max(cur).max(0.0);
                    if tb <= ta {
                        tb = ta + interval;
                    }
                    let k_cur = geom.y_to_key(y);
                    let (klo, khi) = (d.anchor_key.min(k_cur), d.anchor_key.max(k_cur));
                    [
                        geom.tick_to_x(ta).max(geom.kbw),
                        geom.key_to_y(khi),
                        geom.tick_to_x(tb),
                        geom.key_to_y(klo) + geom.kh,
                    ]
                }
                ViewMode::Arrangement => {
                    let geom = ArGeom::new(&s.arr_view);
                    let q = quantize_of(s);
                    let interval = f64::from(q.tick_interval(s.ppq).max(1));
                    let cur = snap.round(q, geom.x_to_tick(x));
                    let ta = d.anchor_tick.min(cur).max(0.0);
                    let mut tb = d.anchor_tick.max(cur).max(0.0);
                    if tb <= ta {
                        tb = ta + interval;
                    }
                    let r_cur = geom.y_to_row(y);
                    let (rlo, rhi) = (d.anchor_row.min(r_cur), d.anchor_row.max(r_cur));
                    [
                        geom.tick_to_x(ta).max(geom.panel_w),
                        geom.row_to_y(rlo),
                        geom.tick_to_x(tb),
                        geom.row_to_y(rhi + 1),
                    ]
                }
            };
            s.overlay_box = Some(b);
            if d.kind == kind::ERASE {
                build_erase_preview(s, d, x, y);
            } else {
                s.preview.clear();
            }
            bump(s);
        }
        kind::MOVE_BOX => {
            let Some((dt_f, dk, dtr)) = move_box_delta(s, d, x, y) else {
                return;
            };
            let dt = dt_f.round() as i64;
            match s.view_mode {
                ViewMode::Pianoroll => {
                    let Some((t0, t1, klo, khi)) = d.box_ticks else {
                        return;
                    };
                    let geom = PrGeom::new(&s.pr_view);
                    let klo2 = (i32::from(klo) + dk).clamp(0, 127) as u8;
                    let khi2 = (i32::from(khi) + dk).clamp(0, 127) as u8;
                    s.overlay_box = Some([
                        geom.tick_to_x(t0 + dt_f).max(geom.kbw),
                        geom.key_to_y(khi2),
                        geom.tick_to_x(t1 + dt_f),
                        geom.key_to_y(klo2) + geom.kh,
                    ]);
                    let stamp = (dt, dk, s.pr_view.render_hash());
                    if d.last_ghost != Some(stamp) {
                        d.last_ghost = Some(stamp);
                        let (tick_lo, tick_hi) = s
                            .pr_view
                            .visible_main_range(s.pr_view.main_axis_len(lw, lh));
                        let (key_lo, key_hi) = s
                            .pr_view
                            .visible_cross_range(s.pr_view.cross_axis_len(lw, lh));
                        let mut ghost = Vec::new();
                        let src_lo = (i32::from(key_lo) - dk).clamp(0, 127) as usize;
                        let src_hi = (i32::from(key_hi) - dk).clamp(0, 127) as usize;
                        for src_key in src_lo..=src_hi {
                            let new_key = (src_key as i32 + dk).clamp(0, 127) as u8;
                            for &(track, start, end, _vel) in &d.drag_by_key[src_key] {
                                let new_start = (i64::from(start) + dt).max(0) as u32;
                                let new_end = new_start.saturating_add(end.saturating_sub(start));
                                if f64::from(new_start) >= tick_hi || f64::from(new_end) <= tick_lo
                                {
                                    continue;
                                }
                                ghost.push((new_start, new_end, new_key, track));
                            }
                        }
                        apply_preview(s, d, std::mem::take(&mut ghost), true);
                        if dk != d.preview_last_dk {
                            d.preview_last_dk = dk;
                            preview_group(s, d, dt, dk);
                        }
                    }
                }
                ViewMode::Arrangement => {
                    let Some((t0, t1, rlo, rhi)) = d.box_rows else {
                        return;
                    };
                    let geom = ArGeom::new(&s.arr_view);
                    let max_row = s.num_tracks.saturating_sub(1) as i32;
                    let rlo2 = (rlo as i32 + dtr).clamp(0, max_row) as usize;
                    let rhi2 = (rhi as i32 + dtr).clamp(0, max_row) as usize;
                    s.overlay_box = Some([
                        geom.tick_to_x(t0 + dt_f).max(geom.panel_w),
                        geom.row_to_y(rlo2),
                        geom.tick_to_x(t1 + dt_f),
                        geom.row_to_y(rhi2 + 1),
                    ]);
                    let stamp = (dt, dtr, s.arr_view.render_hash());
                    if d.last_ghost != Some(stamp) {
                        d.last_ghost = Some(stamp);
                        let (tick_lo, tick_hi) = s.arr_view.visible_tick_range(lw);
                        let (first_row, last_row) =
                            yinhe_types::ArrangementView::visible_track_range_static(
                                s.arr_view.base.scroll_y,
                                lh,
                                s.arr_view.lane_height(),
                                s.num_tracks,
                            );
                        let mut ghost = Vec::new();
                        for (key, bucket) in d.drag_by_key.iter().enumerate() {
                            for &(track, start, end, _vel) in bucket {
                                let new_start = (i64::from(start) + dt).max(0) as u32;
                                let new_end = new_start.saturating_add(end.saturating_sub(start));
                                let new_track = (i32::from(track) + dtr).clamp(0, max_row) as u16;
                                let row = new_track as usize;
                                if row < first_row
                                    || row >= last_row
                                    || f64::from(new_start) >= tick_hi
                                    || f64::from(new_end) <= tick_lo
                                {
                                    continue;
                                }
                                ghost.push((new_start, new_end, key as u8, new_track));
                            }
                        }
                        apply_preview(s, d, std::mem::take(&mut ghost), false);
                        let auto_ghost = build_ar_auto_ghost(s, dt, dtr);
                        s.preview.set_auto_ghost(auto_ghost);
                    }
                }
            }
            bump(s);
        }
        kind::PENCIL => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(s);
            let end = snap.ceil(q, geom.x_to_tick(x).max(0.0));
            let gate = f64::from(q.tick_interval(s.ppq).max(1));
            d.cur_end = end.max(d.start_f + gate);
            pencil_box(s, d, &geom);
            let start = d.start_f.round().max(0.0) as u32;
            let end = d.cur_end.round().max(d.start_f + gate).max(1.0) as u32;
            if let Some(track) = write_track(s) {
                apply_preview(s, d, vec![(start, end, d.key, track as u16)], false);
            } else {
                s.preview.clear();
            }
        }
        kind::PENCIL_PICK => {
            let geom = PrGeom::new(&s.pr_view);
            if let Some((track, start, end, key, _)) = hit_note(s, &geom, x, y) {
                let changed =
                    d.track != track || d.start_tick != start || d.end_tick != end || d.key != key;
                if changed {
                    d.track = track;
                    d.start_tick = start;
                    d.end_tick = end;
                    d.key = key;
                    d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::from([(
                        track, start, key,
                    )])));
                    if let Some(velocity) = note_velocity(s, track, start, key) {
                        preview_note(s, track, key, velocity, start, end.saturating_sub(start));
                    }
                }
                apply_preview(s, d, vec![(start, end, key, track)], true);
            } else {
                // 只有真正压在音符上才算候选；空白处不吸附到最近行。
                d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::new()));
                s.preview.clear();
            }
        }
        kind::LINE => {
            d.cells = line_cells(d, s, x, y);
            preview_line(s, d);
        }
        kind::BRUSH => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(s);
            let interval = u64::from(q.tick_interval(s.ppq).max(1));
            let tick = snap.round(q, geom.x_to_tick(x).max(0.0)).max(0.0);
            let cell = ((tick as u64 / interval) as u32, geom.y_to_key(y));
            if cell != d.last_cell {
                let last = d.last_cell;
                let mut add: Vec<(u32, u8)> = Vec::new();
                walk_line(last, cell, &mut |cx, cy| add.push((cx, cy)));
                for c in add {
                    if !d.cells.contains(&c) {
                        d.cells.push(c);
                    }
                }
                d.last_cell = cell;
            }
            brush_cells(s, d, &geom, interval as u32);
            let interval = interval as u32;
            if let Some(track) = write_track(s) {
                let ghost: Vec<(u32, u32, u8, u16)> = d
                    .cells
                    .iter()
                    .map(|&(idx, key)| {
                        let start = idx.saturating_mul(interval);
                        (start, start.saturating_add(interval), key, track as u16)
                    })
                    .collect();
                apply_preview(s, d, ghost, false);
            } else {
                s.preview.clear();
            }
        }
        kind::NOTE_MOVE => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(s);
            let dt = (snap.round(q, geom.x_to_tick(x)) - d.anchor_tick).round() as i64;
            let dk = i32::from(geom.y_to_key(y)) - i32::from(d.anchor_key);
            let new_start = (i64::from(d.start_tick) + dt).max(0) as u32;
            let new_key = (i32::from(d.key) + dk).clamp(0, 127) as u8;
            let len = d.end_tick.saturating_sub(d.start_tick);
            apply_preview(
                s,
                d,
                vec![(new_start, new_start + len, new_key, d.track)],
                true,
            );
            if dk != d.preview_last_dk {
                d.preview_last_dk = dk;
                if let Some(velocity) = note_velocity(s, d.track, d.start_tick, d.key) {
                    preview_note(s, d.track, new_key, velocity, new_start, len);
                }
            }
        }
        kind::NOTE_RESIZE => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(s);
            let ghost = if d.side == 1 {
                let new_start = snap.floor(q, geom.x_to_tick(x)).max(0.0) as u32;
                (new_start, d.end_tick, d.key, d.track)
            } else {
                let new_end = snap.ceil(q, geom.x_to_tick(x)).max(1.0) as u32;
                (d.start_tick, new_end, d.key, d.track)
            };
            apply_preview(s, d, vec![ghost], true);
        }
        kind::SEL_RESIZE => update_sel_resize(d, s, x, y),
        kind::RIFF_FILL => update_riff_fill(d, s, x, y),
        _ => {
            s.preview.clear();
        }
    }
}

/// 单指释放：提交编辑（一步一 undo）。
pub fn end(x: f32, y: f32) {
    let Ok(mut d) = DRAG.lock() else {
        return;
    };
    let k = d.kind;
    if k == kind::NONE {
        return;
    }
    let Ok(mut s) = scene::scene().lock() else {
        d.kind = kind::NONE;
        return;
    };
    let moved = ((x - d.x0).powi(2) + (y - d.y0).powi(2)).sqrt() >= DRAG_THRESHOLD;
    // 单击音符/选区时保留一次 gate 预览，不立刻 Stop（移动端音频启动有延迟）。
    let keep_gate = !moved
        && matches!(
            k,
            kind::NOTE_MOVE | kind::NOTE_RESIZE | kind::PENDING | kind::PENCIL_PICK
        );
    let mut click_tick: Option<f64> = None;
    let edited = match k {
        kind::MARQUEE => {
            click_tick = commit_marquee(&mut s, &d, x, y, moved);
            None
        }
        kind::MOVE_BOX => commit_move(&mut s, &d, x, y, moved),
        kind::NOTE_MOVE => commit_note_move(&mut s, &d, x, y, moved),
        kind::NOTE_RESIZE => commit_note_resize(&mut s, &d, x, y, moved),
        kind::SEL_RESIZE => {
            commit_selection_resize(&mut s, &d);
            None
        }
        kind::RIFF_FILL => commit_riff(&mut s, &d, x, y),
        kind::PENCIL_PICK => {
            commit_pick(&mut s, x, y);
            None
        }
        kind::PENCIL => commit_pencil(&mut s, &d),
        kind::BRUSH | kind::LINE => commit_brush(&mut s, &d),
        kind::ERASE => commit_erase(&mut s, &d, x, y, moved),
        // 空白处长按未完成就松手：按普通点击处理（清空选择/移动光标）。
        kind::PENDING_PASTE => {
            click_tick = commit_marquee(&mut s, &d, x, y, false);
            None
        }
        // 长按完成：什么都不改，松手后由 Flutter 弹粘贴菜单。
        kind::PASTE_READY => None,
        _ => None,
    };
    // 铅笔释放：先停持续音，提交后补一次目标 gate，确保快速点放也能听到声音。
    let mut pencil_preview: Option<yinhe_audio::PreviewNoteParams> = None;
    if k == kind::PENCIL
        && edited.is_some()
        && let Some(track) = write_track(&s)
    {
        let start = d.start_f.round().max(0.0) as u32;
        let end = d.cur_end.round().max(d.start_f + 1.0) as u32;
        if let Some(doc) = s.document.as_ref()
            && let Some(t) = doc.data.model.tracks.get(track)
        {
            pencil_preview = Some(yinhe_audio::PreviewNoteParams {
                channel: t.global_channel(),
                key: d.key,
                velocity: DEFAULT_VELOCITY,
                target_tick: start,
                duration_ticks: end.saturating_sub(start),
            });
        }
    }
    d.kind = kind::NONE;
    s.preview.clear();
    s.overlay_box = None;
    s.overlay_cells.clear();
    bump(&mut s);
    drop(s);
    drop(d);
    if !keep_gate {
        crate::audio::stop_preview();
    }
    if let Some(note) = pencil_preview {
        crate::audio::preview_notes(vec![note], true);
    }
    // 解锁后再设置播放光标 / 重载音频（锁内二次加锁会死锁）。
    if let Some(tick) = click_tick {
        scene::set_play_cursor(tick);
    }
    if let Some(m) = edited {
        // 纯音符编辑：增量更新，不停止播放。
        crate::audio::update_notes(m);
    }
}

/// 取消拖拽（双指介入 / surface 重建）：丢弃未提交内容。
pub fn cancel() {
    let Ok(mut d) = DRAG.lock() else {
        return;
    };
    if d.kind == kind::NONE {
        return;
    }
    let resize = d.kind == kind::SEL_RESIZE;
    let orig_ticks = d.box_ticks;
    let orig_rows = d.box_rows;
    d.kind = kind::NONE;
    drop(d);
    if let Ok(mut s) = scene::scene().lock() {
        if resize {
            match s.view_mode {
                ViewMode::Pianoroll => {
                    if let Some((t0, t1, k0, k1)) = orig_ticks
                        && let Some(doc) = s.document.as_mut()
                        && let Some(rect) = doc.edit.sel_rect.rects.first_mut()
                    {
                        *rect = (t0, t1, k0, k1);
                    }
                }
                ViewMode::Arrangement => {
                    if let Some((t0, t1, r0, r1)) = orig_rows
                        && let Some(doc) = s.document.as_mut()
                        && let Some(rect) = doc.edit.arr_sel_rect.first_mut()
                    {
                        *rect = (t0, t1, r0, r1);
                    }
                }
            }
        }
        s.preview.clear();
        s.overlay_box = None;
        s.overlay_cells.clear();
        bump(&mut s);
    }
    crate::audio::stop_preview();
}

// ── PR 起始分发 ──

fn begin_pr(d: &mut DragState, s: &mut RenderScene, x: f32, y: f32) {
    let geom = PrGeom::new(&s.pr_view);
    if x < geom.kbw {
        return; // 键盘区不响应编辑
    }
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    // 内容锚点：吸附后的 tick（滚动视图后仍固定，不随视图漂移）。
    let anchor_tick = snap.round(q, geom.x_to_tick(x));
    let anchor_key = geom.y_to_key(y);
    // 选框工具：先命中四边圆形控制点 / 右下角 Riff 方块。
    if s.tool == Tool::Select
        && let Some(h) = pr_selection_handle(s, &geom, x, y)
    {
        begin_sel_handle(d, s, h);
        return;
    }
    match s.tool {
        Tool::Select => {
            d.anchor_tick = anchor_tick;
            d.anchor_key = anchor_key;
            // 手机端默认关闭单音符直接拖拽/伸缩，避免按在音符上滑动就误改；
            // 选框长按后的整体移动仍保留。
            if DIRECT_NOTE_DRAG_ENABLED
                && let Some((track, start_tick, end_tick, key, side)) = hit_note(s, &geom, x, y)
            {
                d.kind = if side == 0 {
                    kind::NOTE_MOVE
                } else {
                    kind::NOTE_RESIZE
                };
                d.track = track;
                d.start_tick = start_tick;
                d.end_tick = end_tick;
                d.key = key;
                d.side = side;
                d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::from([(
                    track, start_tick, key,
                )])));
                if let Some(velocity) = note_velocity(s, track, start_tick, key) {
                    preview_note(
                        s,
                        track,
                        key,
                        velocity,
                        start_tick,
                        end_tick.saturating_sub(start_tick),
                    );
                }
                return;
            }
            if let Some(rect) = pr_box_at(s, x, y) {
                // 进入长按待定；500ms 后由 Flutter long press 推进到移动态。
                d.kind = kind::PENDING;
                d.box_ticks = Some(rect);
                // 框选整体移动的 ghost：拖动开始前一次收集，避免每帧扫选区。
                let (by_key, hidden) = collect_selected_notes(s);
                d.drag_by_key = by_key;
                d.drag_hidden = Some(std::sync::Arc::new(hidden));
                d.preview_last_dk = 0;
                preview_group(s, d, 0, 0);
                return;
            }
            // 空白处：先进入长按待定。拖动超过阈值会转成普通框选；
            // 按住 500ms 松手则弹出粘贴菜单。
            d.kind = kind::PENDING_PASTE;
            s.overlay_box = None;
            bump(s);
        }
        Tool::Pencil => {
            d.anchor_tick = anchor_tick;
            d.anchor_key = anchor_key;
            // 铅笔压在已有音符上：直接拖动该音符；空白处才新建。
            if let Some((track, start, end, key, _)) = hit_note(s, &geom, x, y) {
                d.kind = kind::NOTE_MOVE;
                d.track = track;
                d.start_tick = start;
                d.end_tick = end;
                d.key = key;
                d.side = 0;
                d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::from([(
                    track, start, key,
                )])));
                d.preview_last_dk = 0;
                if let Some(velocity) = note_velocity(s, track, start, key) {
                    preview_note(s, track, key, velocity, start, end.saturating_sub(start));
                }
                return;
            }
            // 空白处：保持原有铅笔新建。
            let gate = f64::from(q.tick_interval(s.ppq).max(1));
            d.kind = kind::PENCIL;
            d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::new()));
            d.start_f = anchor_tick;
            d.cur_end = anchor_tick + gate;
            d.key = anchor_key;
            pencil_box(s, d, &geom);
            if let Some(track) = write_track(s) {
                // 与 commit_pencil 的默认力度保持一致，避免试听和落音符力度不同。
                preview_note(
                    s,
                    track as u16,
                    d.key,
                    DEFAULT_VELOCITY,
                    d.start_f.max(0.0) as u32,
                    0,
                );
            }
        }
        Tool::Eraser => {
            d.kind = kind::ERASE;
            d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::new()));
            d.anchor_tick = anchor_tick;
            d.anchor_key = anchor_key;
            s.overlay_box = Some([x, y, x, y]);
            bump(s);
        }
        Tool::Brush => {
            let interval = u64::from(q.tick_interval(s.ppq).max(1));
            d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::new()));
            let cell = ((anchor_tick as u64 / interval) as u32, anchor_key);
            d.kind = kind::BRUSH;
            d.last_cell = cell;
            d.cells.clear();
            d.cells.push(cell);
            brush_cells(s, d, &geom, interval as u32);
        }
        Tool::Line => {
            d.drag_hidden = Some(std::sync::Arc::new(std::collections::HashSet::new()));
            d.kind = kind::LINE;
            d.anchor_tick = anchor_tick;
            d.anchor_key = anchor_key;
            d.cells = vec![(
                (anchor_tick / f64::from(q.tick_interval(s.ppq).max(1))) as u32,
                anchor_key,
            )];
            preview_line(s, d);
        }
        Tool::Pan => {}
    }
}

/// AR 起始分发（面板区交给 Dart：进 PR / 拖动排序）。
fn begin_arr(d: &mut DragState, s: &mut RenderScene, x: f32, y: f32) {
    let geom = ArGeom::new(&s.arr_view);
    if x < geom.panel_w {
        return;
    }
    if s.tool != Tool::Select {
        return; // AR 只开放选框与手形
    }
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    let anchor_tick = snap.round(q, geom.x_to_tick(x));
    let anchor_row = geom.y_to_row(y);
    d.anchor_tick = anchor_tick;
    d.anchor_row = anchor_row;
    if let Some(h) = ar_selection_handle(s, &geom, x, y) {
        begin_sel_handle(d, s, h);
        return;
    }
    if let Some(rect) = arr_box_at(s, &geom, x, y) {
        d.kind = kind::PENDING;
        d.box_rows = Some(rect);
        let (by_key, hidden) = collect_selected_notes(s);
        d.drag_by_key = by_key;
        d.drag_hidden = Some(std::sync::Arc::new(hidden));
        return;
    }
    // 空白处：长按待定（同上），拖动后转框选。
    d.kind = kind::PENDING_PASTE;
    s.overlay_box = None;
    bump(s);
}

// ── 命中测试与选框查询 ──

type NoteHit = (u16, u32, u32, u8, u8);

/// PR 命中音符：返回 `(track, start_tick, end_tick, key, side)`，side 0=移动 1=左缘 2=右缘。
fn hit_note(s: &RenderScene, geom: &PrGeom, x: f32, y: f32) -> Option<NoteHit> {
    let doc = s.document.as_ref()?;
    let key = geom.y_to_key(y);
    let tick = geom.x_to_tick(x);
    if tick < 0.0 {
        return None;
    }
    let tick_u = tick.floor() as u32;
    let write = write_track(s);
    for note in doc.data.model.key_notes(key).iter() {
        if note.start_tick <= tick_u && tick_u < note.end_tick {
            let t = note.track as usize;
            let allowed = match write {
                Some(w) => t == w,
                None => s.track_visible.get(t).copied().unwrap_or(true),
            };
            if !allowed {
                continue;
            }
            let (x0, x1) = (
                geom.tick_to_x(f64::from(note.start_tick)),
                geom.tick_to_x(f64::from(note.end_tick)),
            );
            let (y0, y1) = (geom.key_to_y(key), geom.key_to_y(key) + geom.kh);
            if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                let side = if x - x0 <= EDGE {
                    1
                } else if x1 - x <= EDGE {
                    2
                } else {
                    0
                };
                return Some((note.track, note.start_tick, note.end_tick, key, side));
            }
        }
    }
    None
}

/// 收集当前选区中的音符，按 key 分桶，供整体拖动 ghost 使用。
///
/// 选区可能是矩形/成员位图，统一用 `accepts_note` 判定；按 `(track,start,key)`
/// 去重，避免多个重叠矩形重复产出。返回 `(per-key (track,start,end), hidden)`。
fn collect_selected_notes(s: &RenderScene) -> (DragByKey, std::collections::HashSet<HiddenKey>) {
    let mut by_key: DragByKey = (0..yinhe_types::KEY_COUNT).map(|_| Vec::new()).collect();
    let mut hidden = std::collections::HashSet::new();
    let Some(doc) = s.document.as_ref() else {
        return (by_key, hidden);
    };
    let mut seen = std::collections::HashSet::new();
    for &(t0, t1, klo, khi, _, _) in &doc.edit.selected.rects {
        let (ts, te) = (t0.min(t1), t0.max(t1));
        if te <= ts {
            continue;
        }
        for key in klo..=khi {
            for n in doc.data.model.key_notes_in_range(key, ts, te) {
                if n.start_tick < ts || n.start_tick >= te {
                    continue;
                }
                if !doc.edit.selected.accepts_note(n, key) {
                    continue;
                }
                if seen.insert((n.track, n.start_tick, key)) {
                    by_key[key as usize].push((n.track, n.start_tick, n.end_tick, n.velocity));
                    hidden.insert((n.track, n.start_tick, key));
                }
            }
        }
    }
    (by_key, hidden)
}

/// PR 已选框命中（像素坐标，取第一个）。
fn pr_box_at(s: &RenderScene, x: f32, y: f32) -> Option<(f64, f64, u8, u8)> {
    let doc = s.document.as_ref()?;
    let view = &s.pr_view;
    for (t0, t1, klo, khi) in doc.edit.sel_rect.effective_rects() {
        let b = [
            view.tick_to_x(t0),
            view.key_to_y(khi),
            view.tick_to_x(t1),
            view.key_to_y(klo) + view.key_height,
        ];
        if x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3] {
            return Some((t0, t1, klo, khi));
        }
    }
    None
}

/// AR 已选框命中。
fn arr_box_at(s: &RenderScene, geom: &ArGeom, x: f32, y: f32) -> Option<(f64, f64, usize, usize)> {
    let doc = s.document.as_ref()?;
    for (t0, t1, tr_lo, tr_hi) in &doc.edit.arr_sel_rect {
        let b = [
            s.arr_view.base.tick_to_x(*t0),
            geom.row_to_y(*tr_lo),
            s.arr_view.base.tick_to_x(*t1),
            geom.row_to_y(tr_hi + 1),
        ];
        if x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3] {
            return Some((*t0, *t1, *tr_lo, *tr_hi));
        }
    }
    None
}

// ── 提交 ──

/// PR/AR 框选提交；`moved < 阈值` 视为点击（清空选择）。
///
/// 返回点击位置（tick，需在**解锁后**由调用方设置播放光标——锁内二次加锁会死锁）。
fn commit_marquee(s: &mut RenderScene, d: &DragState, x: f32, y: f32, moved: bool) -> Option<f64> {
    let view_mode = s.view_mode;
    let num_tracks = s.num_tracks;
    let q = quantize_of(s);
    let ppq = s.ppq;
    let interval = q.tick_interval(ppq).max(1);
    let write = write_track(s);
    let geom_pr = PrGeom::new(&s.pr_view);
    let geom_ar = ArGeom::new(&s.arr_view);
    let snap = scene::SnapCtx::new(s);
    // Conductor 视图没有音符：框选只清空遗留选区，不生成任何成员。
    let conductor_view = s.display_track == Some(0);
    let doc = s.document.as_mut()?;
    let mut click_tick: Option<f64> = None;
    let mut click_row: Option<usize> = None;
    match view_mode {
        ViewMode::Pianoroll => {
            if !moved {
                doc.edit.selected.clear();
                doc.edit.sel_rect.clear();
                doc.edit.arr_sel_rect.clear();
                doc.edit.chan_ar_sel.clear();
                doc.edit.track_selected.clear();
                // 点击空白：移动播放光标（与桌面一致）。
                click_tick = Some(geom_pr.x_to_tick(x));
            } else {
                let cur = snap.round(q, geom_pr.x_to_tick(x));
                let ta = d.anchor_tick.min(cur).max(0.0) as u32;
                let mut tb = d.anchor_tick.max(cur).max(0.0) as u32;
                if tb <= ta {
                    tb = ta + interval;
                }
                let k_cur = geom_pr.y_to_key(y);
                let (klo, khi) = (d.anchor_key.min(k_cur), d.anchor_key.max(k_cur));
                let (tr_lo, tr_hi) = match write {
                    Some(t) => (t as u16, t as u16),
                    None => (0, num_tracks.saturating_sub(1) as u16),
                };
                let Document { edit, .. } = doc;
                edit.selected.clear();
                edit.sel_rect.clear();
                edit.arr_sel_rect.clear();
                edit.chan_ar_sel.clear();
                if !conductor_view {
                    edit.sel_rect
                        .push_rect((f64::from(ta), f64::from(tb), klo, khi), false);
                    edit.selected.add_rect_track(ta, tb, klo, khi, tr_lo, tr_hi);
                }
            }
        }
        ViewMode::Arrangement => {
            if !moved {
                doc.edit.selected.clear();
                doc.edit.sel_rect.clear();
                doc.edit.arr_sel_rect.clear();
                doc.edit.chan_ar_sel.clear();
                doc.edit.track_selected.clear();
                // 点击空白：移动播放光标并聚焦该轨（与桌面一致）。
                click_tick = Some(geom_ar.x_to_tick(x));
                click_row = Some(geom_ar.y_to_row(y));
            } else {
                let cur = snap.round(q, geom_ar.x_to_tick(x));
                let ta = d.anchor_tick.min(cur).max(0.0) as u32;
                let mut tb = d.anchor_tick.max(cur).max(0.0) as u32;
                if tb <= ta {
                    tb = ta + interval;
                }
                let r_cur = geom_ar.y_to_row(y);
                let (rlo, rhi) = (d.anchor_row.min(r_cur), d.anchor_row.max(r_cur));
                let (tr_lo, tr_hi) = (rlo, rhi.min(num_tracks.saturating_sub(1)));
                let Document { edit, .. } = doc;
                edit.selected.clear();
                edit.sel_rect.clear();
                edit.chan_ar_sel.clear();
                edit.arr_sel_rect.clear();
                edit.arr_sel_rect
                    .push((f64::from(ta), f64::from(tb), tr_lo, tr_hi));
                edit.selected.add_rect_track(
                    ta,
                    tb,
                    0,
                    yinhe_types::MAX_KEY,
                    tr_lo as u16,
                    tr_hi as u16,
                );
            }
        }
    }
    if let Some(row) = click_row {
        s.active_track = Some(row.min(s.num_tracks.saturating_sub(1)));
    }
    after_selection_change(s);
    if moved {
        crate::selection_materializer::schedule(s);
    }
    click_tick
}

/// 已选框整体移动提交。
fn commit_move(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    y: f32,
    moved: bool,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    if !moved {
        return None;
    }
    match s.view_mode {
        ViewMode::Pianoroll => {
            let (dt_f, dk, _) = move_box_delta(s, d, x, y)?;
            let dt = dt_f.round() as i64;
            if dt == 0 && dk == 0 {
                return None;
            }
            let doc = s.document.as_mut()?;
            let before = doc.capture_snapshot();
            if let Some(action) = doc.move_selected_notes(dt, dk) {
                doc.push_undo(action, "undo.move_notes", before);
                // 可视选框跟随移动（与桌面一致）。
                for r in &mut doc.edit.sel_rect.rects {
                    let klo = (i32::from(r.2) + dk).clamp(0, 127) as u8;
                    let khi = (i32::from(r.3) + dk).clamp(0, 127) as u8;
                    r.0 += dt as f64;
                    r.1 += dt as f64;
                    r.2 = klo;
                    r.3 = khi;
                }
                return after_edit(s);
            }
            None
        }
        ViewMode::Arrangement => {
            let (dt_f, _, dtr) = move_box_delta(s, d, x, y)?;
            let dt = dt_f.round() as i64;
            if dt == 0 && dtr == 0 {
                return None;
            }
            let num_tracks = s.num_tracks;
            let doc = s.document.as_mut()?;
            let before = doc.capture_snapshot();
            if let Some(action) = doc.move_selected_arrange(dt, dtr) {
                doc.push_undo(action, "undo.move_in_arrange", before);
                // 可视选框跟随移动（轨道方向 clamp 到有效范围，与 ghost 预览一致）。
                let max_track = num_tracks.saturating_sub(1) as i64;
                for r in &mut doc.edit.arr_sel_rect {
                    r.0 += dt as f64;
                    r.1 += dt as f64;
                    r.2 = (r.2 as i64 + i64::from(dtr)).clamp(0, max_track) as usize;
                    r.3 = (r.3 as i64 + i64::from(dtr)).clamp(0, max_track) as usize;
                }
                return after_edit(s);
            }
            None
        }
    }
}

/// 铅笔精调选择提交：松手时以最后一次真实命中的音符为准。
fn commit_pick(s: &mut RenderScene, x: f32, y: f32) -> bool {
    let geom = PrGeom::new(&s.pr_view);
    let Some((track, start, end, key, _)) = hit_note(s, &geom, x, y) else {
        return false;
    };
    select_note(s, track, start, end, key);
    after_selection_change(s);
    true
}

/// 选框边缘控制点释放：按新矩形更新选区。
///
/// 保持矩形选区模式，不在这里全量物化音符位图；选中判定按矩形惰性完成，
/// 避免大选区松手时扫描整片区域导致卡顿。
fn commit_selection_resize(s: &mut RenderScene, d: &DragState) {
    match s.view_mode {
        ViewMode::Pianoroll => {
            let Some((t0, t1, k0, k1)) = s
                .document
                .as_ref()
                .and_then(|doc| doc.edit.sel_rect.rects.first().copied())
            else {
                return;
            };
            let Some(doc) = s.document.as_mut() else {
                return;
            };
            let Document { edit, .. } = doc;
            edit.selected.clear_rects();
            edit.selected.add_rect_track(
                t0.max(0.0) as u32,
                t1.max(t0 + 1.0) as u32,
                k0,
                k1,
                d.sel_tr_lo,
                d.sel_tr_hi,
            );
        }
        ViewMode::Arrangement => {
            let Some((t0, t1, r0, r1)) = s
                .document
                .as_ref()
                .and_then(|doc| doc.edit.arr_sel_rect.first().copied())
            else {
                return;
            };
            let Some(doc) = s.document.as_mut() else {
                return;
            };
            let Document { edit, .. } = doc;
            edit.selected.clear_rects();
            edit.selected.add_rect_track(
                t0.max(0.0) as u32,
                t1.max(t0 + 1.0) as u32,
                0,
                yinhe_types::MAX_KEY,
                r0 as u16,
                r1 as u16,
            );
        }
    }
    after_selection_change(s);
    crate::selection_materializer::schedule(s);
}

/// 右下角 Riff 方块释放：按选区宽度向左右重复，一步 undo。
fn commit_riff(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    y: f32,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    let steps = riff_steps(d, s, x, y);
    if steps.is_zero() {
        return None;
    }
    let (t0, t1, k0, k1) = d.box_ticks?;
    let width = (t1 - t0).max(1.0).round() as i64;
    let view_mode = s.view_mode;
    let changed = {
        let doc = s.document.as_mut()?;
        if doc.edit.selected.is_empty() {
            return None;
        }
        let before = doc.capture_snapshot();
        let x_off = steps.sx * width;
        let new_t0 = (t0 + x_off.min(0) as f64).max(0.0);
        let new_t1 = t1 + x_off.max(0) as f64;
        match view_mode {
            ViewMode::Pianoroll => {
                let span = (i64::from(k1) - i64::from(k0) + 1) as u16;
                let action = doc.repeat_selected_keys(width, steps.sx, span, steps.sy)?;
                let key_off = steps.sy * i64::from(span);
                let new_k0 = (i64::from(k0) + key_off.min(0))
                    .clamp(0, i64::from(yinhe_types::MAX_KEY)) as u8;
                let new_k1 = (i64::from(k1) + key_off.max(0))
                    .clamp(0, i64::from(yinhe_types::MAX_KEY)) as u8;

                // Riff 完成后把选框扩到“原选区 + 所有重复段”，并将新范围设为
                // 唯一矩形选区；副本和原音符都会落在这个范围内。
                doc.edit.sel_rect.clear();
                doc.edit
                    .sel_rect
                    .push_rect((new_t0, new_t1, new_k0, new_k1), false);
                doc.edit.selected.clear_rects();
                doc.edit.selected.add_rect_track(
                    new_t0 as u32,
                    new_t1.max(new_t0 + 1.0) as u32,
                    new_k0,
                    new_k1,
                    d.sel_tr_lo,
                    d.sel_tr_hi,
                );
                doc.push_undo(action, "重复 Riff", before);
            }
            ViewMode::Arrangement => {
                let (_, _, r0, r1) = d.box_rows.unwrap_or((t0, t1, 0, 0));
                let span = (r1 - r0 + 1) as u16;
                let action = doc.repeat_selected_tracks(width, steps.sx, span, steps.sy)?;
                let row_off = steps.sy * i64::from(span);
                let last = s.num_tracks.saturating_sub(1) as i64;
                let mut new_r0 = (r0 as i64 + row_off.min(0)).clamp(0, last) as usize;
                let new_r1 = (r1 as i64 + row_off.max(0)).clamp(0, last) as usize;
                // Conductor 不参与 Riff：选框也不要把 0 号轨圈进去。
                if new_r1 > 0
                    && doc
                        .data
                        .model
                        .tracks
                        .first()
                        .is_some_and(|t| t.kind == yinhe_core::TrackKind::Master)
                {
                    new_r0 = new_r0.max(1);
                }

                doc.edit.arr_sel_rect = vec![(new_t0, new_t1, new_r0, new_r1)];
                doc.edit.selected.clear_rects();
                doc.edit.selected.add_rect_track(
                    new_t0 as u32,
                    new_t1.max(new_t0 + 1.0) as u32,
                    0,
                    yinhe_types::MAX_KEY,
                    new_r0 as u16,
                    new_r1 as u16,
                );
                doc.push_undo(action, "重复 Riff", before);
            }
        }
        true
    };
    if changed {
        let model = after_edit(s);
        crate::selection_materializer::schedule(s);
        model
    } else {
        None
    }
}

/// 单音符移动提交（铅笔/选框共用入口，桌面 `pencil_drag_note`）。
fn commit_note_move(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    y: f32,
    moved: bool,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    if !moved {
        return None;
    }
    let geom = PrGeom::new(&s.pr_view);
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    let dt = (snap.round(q, geom.x_to_tick(x)) - d.anchor_tick).round() as i64;
    let dk = i32::from(geom.y_to_key(y)) - i32::from(d.anchor_key);
    if dt == 0 && dk == 0 {
        return None;
    }
    let drag = yinhe_types::PencilNoteDrag::Move {
        track: d.track,
        start_tick: d.start_tick,
        key: d.key,
        delta_ticks: dt,
        delta_keys: dk,
    };
    let moved_note = {
        let doc = s.document.as_mut()?;
        let before = doc.capture_snapshot();
        doc.pencil_drag_note(&drag).map(|action| {
            doc.push_undo(action, "undo.move_note", before);
        })
    };
    if moved_note.is_some() {
        // 铅笔移动后选区跟随新位置，下一笔仍可直接拖这个音符。
        if PENCIL_PICK_ENABLED && matches!(s.tool, Tool::Pencil) {
            let new_key = (i32::from(d.key) + dk).clamp(0, 127) as u8;
            let new_start = (i64::from(d.start_tick) + dt).max(0) as u32;
            let new_end = new_start.saturating_add(d.end_tick.saturating_sub(d.start_tick));
            select_note(s, d.track, new_start, new_end, new_key);
        }
        return after_edit(s);
    }
    None
}

/// 单音符边缘伸缩提交。
fn commit_note_resize(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    _y: f32,
    moved: bool,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    if !moved {
        return None;
    }
    let geom = PrGeom::new(&s.pr_view);
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    let doc = s.document.as_mut()?;
    let before = doc.capture_snapshot();
    let drag = if d.side == 1 {
        let new_start = snap.floor(q, geom.x_to_tick(x)) as u32;
        yinhe_types::PencilNoteDrag::ResizeLeft {
            track: d.track,
            start_tick: d.start_tick,
            key: d.key,
            new_start_tick: new_start,
        }
    } else {
        let new_end = snap.ceil(q, geom.x_to_tick(x)).max(1.0) as u32;
        yinhe_types::PencilNoteDrag::ResizeRight {
            track: d.track,
            start_tick: d.start_tick,
            key: d.key,
            new_end_tick: new_end,
        }
    };
    if let Some(action) = doc.pencil_drag_note(&drag) {
        doc.push_undo(action, "undo.resize_note", before);
        return after_edit(s);
    }
    None
}

/// 铅笔落音符提交。
fn commit_pencil(
    s: &mut RenderScene,
    d: &DragState,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    let track = write_track(s)?;
    let q = quantize_of(s);
    let gate = f64::from(q.tick_interval(s.ppq).max(1));
    let start = d.start_f.round().max(0.0) as u32;
    let end = d.cur_end.round().max(d.start_f + gate).max(1.0) as u32;
    let doc = s.document.as_mut()?;
    let before = doc.capture_snapshot();
    let note = yinhe_core::NoteEvent {
        id: 0,
        key: d.key,
        start_tick: start,
        end_tick: end,
        velocity: DEFAULT_VELOCITY,
    };
    if let Some(action) = doc.add_note(track as u16, note) {
        doc.push_undo(action, "undo.add_note", before);
        return after_edit(s);
    }
    log::warn!("铅笔提交被拒: track={track} start={start} end={end}");
    None
}

/// 刷子一笔提交（路径格一次批量，1 条 undo）。
fn commit_brush(
    s: &mut RenderScene,
    d: &DragState,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    let track = write_track(s)?;
    if d.cells.is_empty() {
        return None;
    }
    let q = quantize_of(s);
    let interval = q.tick_interval(s.ppq).max(1);
    let notes: Vec<yinhe_core::NoteEvent> = d
        .cells
        .iter()
        .map(|(idx, key)| yinhe_core::NoteEvent {
            id: 0,
            key: *key,
            start_tick: idx.saturating_mul(interval),
            end_tick: idx.saturating_mul(interval).saturating_add(interval),
            velocity: DEFAULT_VELOCITY,
        })
        .collect();
    let doc = s.document.as_mut()?;
    let before = doc.capture_snapshot();
    if let Some(action) = doc.add_notes_batch(track as u16, &notes) {
        doc.push_undo(action, "undo.add_note", before);
        return after_edit(s);
    }
    None
}

/// 橡皮框擦提交（≥阈值才删；与桌面一致为矩形框擦）。
fn commit_erase(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    y: f32,
    moved: bool,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    if !moved {
        return None;
    }
    let view_mode = s.view_mode;
    let num_tracks = s.num_tracks;
    let q = quantize_of(s);
    let ppq = s.ppq;
    let interval = q.tick_interval(ppq).max(1);
    let write = write_track(s);
    let geom_pr = PrGeom::new(&s.pr_view);
    let geom_ar = ArGeom::new(&s.arr_view);
    let snap = scene::SnapCtx::new(s);
    match view_mode {
        ViewMode::Pianoroll => {
            let ta = snap.round(q, geom_pr.x_to_tick(d.x0.min(x))) as u32;
            let mut tb = snap.round(q, geom_pr.x_to_tick(d.x0.max(x))) as u32;
            if tb <= ta {
                tb = ta + interval;
            }
            let ka = geom_pr.y_to_key(d.y0.max(y));
            let kb = geom_pr.y_to_key(d.y0.min(y));
            let (klo, khi) = (ka.min(kb), ka.max(kb));
            let (tr_lo, tr_hi) = match write {
                Some(t) => (t as u16, t as u16),
                None => (0, num_tracks.saturating_sub(1) as u16),
            };
            let doc = s.document.as_mut()?;
            {
                let Document { edit, data, .. } = doc;
                edit.selected.clear();
                edit.selected.add_rect_track(ta, tb, klo, khi, tr_lo, tr_hi);
                edit.selected.materialize_pending(&*data.model);
            }
            let before = doc.capture_snapshot();
            if let Some(action) = doc.delete_selected() {
                doc.push_undo(action, "undo.eraser_delete", before);
                return after_edit(s);
            }
            None
        }
        ViewMode::Arrangement => {
            let ta = snap.round(q, geom_ar.x_to_tick(d.x0.min(x))) as u32;
            let mut tb = snap.round(q, geom_ar.x_to_tick(d.x0.max(x))) as u32;
            if tb <= ta {
                tb = ta + interval;
            }
            let ra = geom_ar.y_to_row(d.y0.min(y));
            let rb = geom_ar.y_to_row(d.y0.max(y));
            let (tr_lo, tr_hi) = (ra.min(rb), ra.max(rb).min(num_tracks.saturating_sub(1)));
            let doc = s.document.as_mut()?;
            {
                let Document { edit, data, .. } = doc;
                edit.selected.clear();
                edit.selected.add_rect_track(
                    ta,
                    tb,
                    0,
                    yinhe_types::MAX_KEY,
                    tr_lo as u16,
                    tr_hi as u16,
                );
                edit.selected.materialize_pending(&*data.model);
            }
            let before = doc.capture_snapshot();
            if let Some(action) = doc.delete_selected() {
                doc.push_undo(action, "undo.eraser_arrange", before);
                return after_edit(s);
            }
            None
        }
    }
}

// ── 覆盖层预览 ──

fn pencil_box(s: &mut RenderScene, d: &DragState, geom: &PrGeom) {
    let x0 = geom.tick_to_x(d.start_f);
    let x1 = geom.tick_to_x(d.cur_end).max(x0 + 2.0);
    let y = geom.key_to_y(d.key);
    s.overlay_box = Some([x0, y, x1, y + geom.kh]);
    bump(s);
}

fn brush_cells(s: &mut RenderScene, d: &DragState, geom: &PrGeom, interval: u32) {
    s.overlay_cells.clear();
    for (idx, key) in &d.cells {
        let t0 = f64::from(idx.saturating_mul(interval));
        let t1 = f64::from(idx.saturating_mul(interval).saturating_add(interval));
        let x0 = geom.tick_to_x(t0);
        let x1 = geom.tick_to_x(t1).max(x0 + 2.0);
        let y = geom.key_to_y(*key);
        s.overlay_cells.push([x0, y, x1, y + geom.kh]);
    }
    bump(s);
}

/// Bresenham 连线（刷子路径补格）。
fn walk_line(from: (u32, u8), to: (u32, u8), add: &mut impl FnMut(u32, u8)) {
    let (x0, y0) = (i64::from(from.0), i64::from(from.1));
    let (x1, y1) = (i64::from(to.0), i64::from(to.1));
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut cx, mut cy) = (x0, y0);
    loop {
        if cx >= 0 && (0..=127).contains(&cy) {
            add(cx as u32, cy as u8);
        }
        if cx == x1 && cy == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            err += dx;
            cy += sy;
        }
    }
}

// ── 收尾刷新 ──

/// 纯选择变化：只刷新覆盖层，不重建音符层（无需重载音频）。
fn after_selection_change(s: &mut RenderScene) {
    scene::selection_changed(s);
}

/// 内容编辑：刷新快照；返回模型供调用方在**解锁后**重载音频。
///
/// 音频首次初始化可能耗时较长，必须避免在持有场景锁时执行（否则渲染线程被卡住，
/// 编辑结果要等音频初始化完才可见）。
fn after_edit(s: &mut RenderScene) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    let model = s.document.as_ref().map(|d| d.data.model.clone());
    scene::refresh_snapshot(s);
    model
}

// Document 解构辅助（拆借用）。
use yinhe_editor_core::Document;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::RenderScene;

    fn scene(display: Option<usize>, active: Option<usize>, tracks: usize) -> RenderScene {
        let mut s = RenderScene::default();
        s.num_tracks = tracks;
        s.display_track = display;
        s.active_track = active;
        s
    }

    #[test]
    fn conductor_view_has_no_write_track() {
        assert_eq!(write_track(&scene(Some(0), Some(0), 3)), None);
    }

    #[test]
    fn explicit_track_writes_that_track() {
        assert_eq!(write_track(&scene(Some(2), Some(0), 3)), Some(2));
        assert_eq!(write_track(&scene(None, Some(2), 3)), Some(2));
    }

    #[test]
    fn all_tracks_falls_back_to_first_note_track() {
        assert_eq!(write_track(&scene(None, Some(0), 3)), Some(1));
        assert_eq!(write_track(&scene(None, None, 3)), Some(1));
    }
}
