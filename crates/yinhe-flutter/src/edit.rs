//! 工具手势（移动端子集）：PR 选框/铅笔/橡皮/刷子，AR 选框。
//!
//! 手形（平移）由 `input.rs` 的通用手势处理；本模块只处理"编辑类"工具。
//! 单指手势 → 编辑；双指 → 由 `input.rs` 取消本模块拖拽后进入缩放/平移。
//! 所有提交都在释放时进行一次（一步一 undo），拖拽期间只更新覆盖层预览。

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::NoteSource;

use crate::scene::{self, RenderScene, Tool, ViewMode};

/// 框选/拖动判定阈值（逻辑像素，与桌面一致）。
const DRAG_THRESHOLD: f32 = 3.0;
/// 音符边缘判定（逻辑像素，与桌面一致）。
const EDGE: f32 = 6.0;
/// 新音符默认力度（桌面无记忆时回退 100）。
const DEFAULT_VELOCITY: u8 = 100;

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
}

#[derive(Clone)]
struct DragState {
    kind: u8,
    x0: f32,
    y0: f32,
    /// 已选框拖动时的起始框（内容逻辑坐标）。
    box0: Option<[f32; 4]>,
    /// 单音符拖动：目标音符。
    track: u16,
    start_tick: u32,
    key: u8,
    side: u8,
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
            box0: None,
            track: 0,
            start_tick: 0,
            key: 0,
            side: 0,
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
    box0: None,
    track: 0,
    start_tick: 0,
    key: 0,
    side: 0,
    start_f: 0.0,
    cur_end: 0.0,
    cells: Vec::new(),
    last_cell: (0, 0),
});

/// 是否有编辑拖拽进行中（`input.rs` 据此决定是否走平移）。
pub fn is_active() -> bool {
    DRAG.lock().map(|d| d.kind != kind::NONE).unwrap_or(false)
}

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
    fn y_to_row(&self, y: f32) -> usize {
        ((y + self.scroll_y) / self.lane_h).max(0.0) as usize
    }
    fn row_to_y(&self, row: usize) -> f32 {
        row as f32 * self.lane_h - self.scroll_y
    }
}

/// 工具写入目标轨：显示过滤轨 → 当前编辑轨 → 第一条非 conductor 轨。
fn write_track(s: &RenderScene) -> Option<usize> {
    let n = s.num_tracks;
    if n == 0 {
        return None;
    }
    let t = s.display_track.or(s.active_track).unwrap_or(1).min(n - 1);
    if t == 0 {
        if n > 1 { Some(1) } else { None }
    } else {
        Some(t)
    }
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
    raw.resolved(ppu, s.ppq)
}

fn bump(s: &mut RenderScene) {
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
}

// ── 拖拽生命周期 ──

/// 单指按下：按当前工具开始编辑拖拽（不启动则退回平移）。
pub fn begin(x: f32, y: f32) {
    let Ok(mut d) = DRAG.lock() else {
        return;
    };
    *d = DragState {
        x0: x,
        y0: y,
        ..Default::default()
    };
    let Ok(mut s) = scene::scene().lock() else {
        return;
    };
    if s.document.is_none() {
        return;
    }
    let tool = s.tool;
    if tool == Tool::Pan {
        return; // 手形：平移交给 input.rs
    }
    match s.view_mode {
        ViewMode::Pianoroll => begin_pr(&mut d, &mut s, x, y),
        ViewMode::Arrangement => begin_arr(&mut d, &mut s, x, y),
    }
}

/// 单指移动更新拖拽。
pub fn update(x: f32, y: f32) {
    let Ok(mut d) = DRAG.lock() else {
        return;
    };
    if d.kind == kind::NONE {
        return;
    }
    let Ok(mut s) = scene::scene().lock() else {
        return;
    };
    let x0 = d.x0.min(x);
    let y0 = d.y0.min(y);
    let x1 = d.x0.max(x);
    let y1 = d.y0.max(y);
    match d.kind {
        kind::MARQUEE | kind::ERASE => {
            s.overlay_box = Some([x0, y0, x1, y1]);
            bump(&mut s);
        }
        kind::MOVE_BOX => {
            if let Some(b) = d.box0 {
                let (dx, dy) = (x - d.x0, y - d.y0);
                s.overlay_box = Some([b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy]);
                bump(&mut s);
            }
        }
        kind::PENCIL => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(&s);
            let end = q.snap_tick_ceil(geom.x_to_tick(x).max(0.0), s.ppq);
            let gate = f64::from(q.tick_interval(s.ppq).max(1));
            d.cur_end = end.max(d.start_f + gate);
            pencil_box(&mut s, &d, &geom);
        }
        kind::BRUSH => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(&s);
            let interval = u64::from(q.tick_interval(s.ppq).max(1));
            let tick = q.snap_tick(geom.x_to_tick(x).max(0.0), s.ppq).max(0.0);
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
            brush_cells(&mut s, &d, &geom, interval as u32);
        }
        _ => {}
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
    let mut click_tick: Option<f64> = None;
    let edited = match k {
        kind::MARQUEE => {
            click_tick = commit_marquee(&mut s, &d, x, y, moved);
            None
        }
        kind::MOVE_BOX => commit_move(&mut s, &d, x, y, moved),
        kind::NOTE_MOVE => commit_note_move(&mut s, &d, x, y, moved),
        kind::NOTE_RESIZE => commit_note_resize(&mut s, &d, x, y, moved),
        kind::PENCIL => commit_pencil(&mut s, &d),
        kind::BRUSH => commit_brush(&mut s, &d),
        kind::ERASE => commit_erase(&mut s, &d, x, y, moved),
        _ => None,
    };
    d.kind = kind::NONE;
    s.overlay_box = None;
    s.overlay_cells.clear();
    bump(&mut s);
    drop(s);
    drop(d);
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
    d.kind = kind::NONE;
    drop(d);
    if let Ok(mut s) = scene::scene().lock() {
        s.overlay_box = None;
        s.overlay_cells.clear();
        bump(&mut s);
    }
}

// ── PR 起始分发 ──

fn begin_pr(d: &mut DragState, s: &mut RenderScene, x: f32, y: f32) {
    let geom = PrGeom::new(&s.pr_view);
    if x < geom.kbw {
        return; // 键盘区不响应编辑
    }
    match s.tool {
        Tool::Select => {
            if let Some((track, start_tick, key, side)) = hit_note(s, &geom, x, y) {
                d.kind = if side == 0 {
                    kind::NOTE_MOVE
                } else {
                    kind::NOTE_RESIZE
                };
                d.track = track;
                d.start_tick = start_tick;
                d.key = key;
                d.side = side;
                return;
            }
            if let Some(b) = pr_box_at(s, x, y) {
                d.kind = kind::MOVE_BOX;
                d.box0 = Some(b);
                s.overlay_box = Some(b);
                bump(s);
                return;
            }
            d.kind = kind::MARQUEE;
            s.overlay_box = Some([x, y, x, y]);
            bump(s);
        }
        Tool::Pencil => {
            let q = quantize_of(s);
            let tick = snap_round(&q, geom.x_to_tick(x), s.ppq);
            let key = geom.y_to_key(y);
            let gate = f64::from(q.tick_interval(s.ppq).max(1));
            d.kind = kind::PENCIL;
            d.start_f = tick;
            d.cur_end = tick + gate;
            d.key = key;
            pencil_box(s, d, &geom);
        }
        Tool::Eraser => {
            d.kind = kind::ERASE;
            s.overlay_box = Some([x, y, x, y]);
            bump(s);
        }
        Tool::Brush => {
            let q = quantize_of(s);
            let interval = u64::from(q.tick_interval(s.ppq).max(1));
            let tick = snap_round(&q, geom.x_to_tick(x), s.ppq);
            let cell = ((tick as u64 / interval) as u32, geom.y_to_key(y));
            d.kind = kind::BRUSH;
            d.last_cell = cell;
            d.cells.clear();
            d.cells.push(cell);
            brush_cells(s, d, &geom, interval as u32);
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
    if let Some(b) = arr_box_at(s, &geom, x, y) {
        d.kind = kind::MOVE_BOX;
        d.box0 = Some(b);
        s.overlay_box = Some(b);
        bump(s);
        return;
    }
    d.kind = kind::MARQUEE;
    s.overlay_box = Some([x, y, x, y]);
    bump(s);
}

// ── 命中测试与选框查询 ──

/// PR 命中音符：返回 `(track, start_tick, key, side)`，side 0=移动 1=左缘 2=右缘。
fn hit_note(s: &RenderScene, geom: &PrGeom, x: f32, y: f32) -> Option<(u16, u32, u8, u8)> {
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
                return Some((note.track, note.start_tick, key, side));
            }
        }
    }
    None
}

/// PR 已选框命中（像素坐标，取第一个）。
fn pr_box_at(s: &RenderScene, x: f32, y: f32) -> Option<[f32; 4]> {
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
            return Some(b);
        }
    }
    None
}

/// AR 已选框命中。
fn arr_box_at(s: &RenderScene, geom: &ArGeom, x: f32, y: f32) -> Option<[f32; 4]> {
    let doc = s.document.as_ref()?;
    for (t0, t1, tr_lo, tr_hi) in &doc.edit.arr_sel_rect {
        let b = [
            s.arr_view.base.tick_to_x(*t0),
            geom.row_to_y(*tr_lo),
            s.arr_view.base.tick_to_x(*t1),
            geom.row_to_y(tr_hi + 1),
        ];
        if x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3] {
            return Some(b);
        }
    }
    None
}

fn snap_round(q: &QuantizePreset, tick: f64, ppq: u32) -> f64 {
    q.snap_tick(tick, ppq).max(0.0)
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
    let Some(doc) = s.document.as_mut() else {
        return None;
    };
    let mut click_tick: Option<f64> = None;
    let mut click_row: Option<usize> = None;
    match view_mode {
        ViewMode::Pianoroll => {
            if !moved {
                doc.edit.selected.clear();
                doc.edit.sel_rect.clear();
                // 点击空白：移动播放光标（与桌面一致）。
                click_tick = Some(geom_pr.x_to_tick(x));
            } else {
                let ta = snap_round(&q, geom_pr.x_to_tick(d.x0.min(x)), ppq) as u32;
                let mut tb = snap_round(&q, geom_pr.x_to_tick(d.x0.max(x)), ppq) as u32;
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
                let Document { edit, data, .. } = doc;
                edit.selected.clear();
                edit.sel_rect.clear();
                edit.sel_rect
                    .push_rect((f64::from(ta), f64::from(tb), klo, khi), false);
                edit.selected.add_rect_track(ta, tb, klo, khi, tr_lo, tr_hi);
                edit.selected.materialize_pending(&*data.model);
            }
        }
        ViewMode::Arrangement => {
            if !moved {
                doc.edit.selected.clear();
                doc.edit.arr_sel_rect.clear();
                // 点击空白：移动播放光标并聚焦该轨（与桌面一致）。
                click_tick = Some(geom_ar.x_to_tick(x));
                click_row = Some(geom_ar.y_to_row(y));
            } else {
                let ta = snap_round(&q, geom_ar.x_to_tick(d.x0.min(x)), ppq) as u32;
                let mut tb = snap_round(&q, geom_ar.x_to_tick(d.x0.max(x)), ppq) as u32;
                if tb <= ta {
                    tb = ta + interval;
                }
                let ra = geom_ar.y_to_row(d.y0.min(y));
                let rb = geom_ar.y_to_row(d.y0.max(y));
                let (tr_lo, tr_hi) = (ra.min(rb), ra.max(rb).min(num_tracks.saturating_sub(1)));
                let Document { edit, data, .. } = doc;
                edit.selected.clear();
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
                edit.selected.materialize_pending(&*data.model);
            }
        }
    }
    if let Some(row) = click_row {
        s.active_track = Some(row.min(s.num_tracks.saturating_sub(1)));
    }
    after_selection_change(s);
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
    let num_tracks = s.num_tracks;
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(s);
            let dt = (q.snap_tick(geom.x_to_tick(x), s.ppq)
                - q.snap_tick(geom.x_to_tick(d.x0), s.ppq))
            .round() as i64;
            let dk = i32::from(geom.y_to_key(y)) - i32::from(geom.y_to_key(d.y0));
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
            let geom = ArGeom::new(&s.arr_view);
            let q = quantize_of(s);
            let dt = (q.snap_tick(geom.x_to_tick(x), s.ppq)
                - q.snap_tick(geom.x_to_tick(d.x0), s.ppq))
            .round() as i64;
            let dtr = geom.y_to_row(y) as i32 - geom.y_to_row(d.y0) as i32;
            if dt == 0 && dtr == 0 {
                return None;
            }
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
    let dt = (q.snap_tick(geom.x_to_tick(x), s.ppq) - q.snap_tick(geom.x_to_tick(d.x0), s.ppq))
        .round() as i64;
    let dk = i32::from(geom.y_to_key(y)) - i32::from(geom.y_to_key(d.y0));
    if dt == 0 && dk == 0 {
        return None;
    }
    let doc = s.document.as_mut()?;
    let before = doc.capture_snapshot();
    let drag = yinhe_types::PencilNoteDrag::Move {
        track: d.track,
        start_tick: d.start_tick,
        key: d.key,
        delta_ticks: dt,
        delta_keys: dk,
    };
    if let Some(action) = doc.pencil_drag_note(&drag) {
        doc.push_undo(action, "undo.move_note", before);
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
    let doc = s.document.as_mut()?;
    let before = doc.capture_snapshot();
    let drag = if d.side == 1 {
        let new_start = q.snap_tick_floor(geom.x_to_tick(x), s.ppq).max(0.0) as u32;
        yinhe_types::PencilNoteDrag::ResizeLeft {
            track: d.track,
            start_tick: d.start_tick,
            key: d.key,
            new_start_tick: new_start,
        }
    } else {
        let new_end = q.snap_tick_ceil(geom.x_to_tick(x), s.ppq).max(1.0) as u32;
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
    match view_mode {
        ViewMode::Pianoroll => {
            let ta = snap_round(&q, geom_pr.x_to_tick(d.x0.min(x)), ppq) as u32;
            let mut tb = snap_round(&q, geom_pr.x_to_tick(d.x0.max(x)), ppq) as u32;
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
            let ta = snap_round(&q, geom_ar.x_to_tick(d.x0.min(x)), ppq) as u32;
            let mut tb = snap_round(&q, geom_ar.x_to_tick(d.x0.max(x)), ppq) as u32;
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

/// 纯选择变化：刷新快照（无需重载音频）。
fn after_selection_change(s: &mut RenderScene) {
    scene::refresh_snapshot(s);
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
