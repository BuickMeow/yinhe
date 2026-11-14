use super::*;

pub(crate) fn begin_sel_handle(d: &mut DragState, s: &mut RenderScene, h: SelHandle, x: f32) {
    d.kind = match h.edge {
        4 => kind::RIFF_FILL,
        3 => kind::SEL_SCALE,
        5 => kind::SEL_GATE,
        _ => return,
    };
    d.sel_edge = h.edge;
    d.box_ticks = Some((h.t0, h.t1, h.k0, h.k1));
    d.box_rows = Some((h.t0, h.t1, h.r0, h.r1));
    d.sel_tr_lo = h.tr_lo;
    d.sel_tr_hi = h.tr_hi;
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    d.anchor_tick_raw = match s.view_mode {
        ViewMode::Pianoroll => PrGeom::new(&s.pr_view).x_to_tick(x),
        ViewMode::Arrangement => ArGeom::new(&s.arr_view).x_to_tick(x),
    };
    d.anchor_tick = snap.round(q, d.anchor_tick_raw);
    if h.edge == 4 {
        // Riff 预览只画重复选框，不预先扫描/物化选中音符。
        d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::default()));
    } else {
        d.drag = crate::selection_materializer::take_drag(s);
        d.drag_hidden = d
            .drag
            .as_ref()
            .map(|drag| std::sync::Arc::clone(&drag.hidden));
        if d.drag.is_none() {
            crate::selection_materializer::schedule_drag(s);
        }
        s.sel_drag_active = true;
        // 整体变速/批量 gate 都保持左、上、右三条边，只让右缘跟手。
        s.hold_sel_edge(0);
        s.hold_sel_edge(1);
        s.hold_sel_edge(3);
    }
    // 锚点拖动沿用选框预览的缓动：先以当前矩形为起点，下一次 update 才有跳变量。
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            s.set_sel_target([
                geom.tick_to_x(h.t0),
                geom.key_to_y(h.k1),
                geom.tick_to_x(h.t1),
                geom.key_to_y(h.k0) + geom.kh,
            ]);
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            s.set_sel_target([
                geom.tick_to_x(h.t0),
                geom.row_to_y(h.r0),
                geom.tick_to_x(h.t1),
                geom.row_to_y(h.r1 + 1),
            ]);
        }
    }
    bump(s);
}

/// 单指按下：按当前工具开始编辑拖拽。
///
/// 返回 1 = 进入编辑；0 = 此处不响应编辑，Flutter 应把它当平移/点击处理。
pub fn begin(x: f32, y: f32) -> i32 {
    begin_with_tool(x, y, None)
}

/// 不修改全局工具状态、直接以选框语义起手（长按拖选框入口）。
pub fn begin_marquee(x: f32, y: f32) -> i32 {
    begin_with_tool(x, y, Some(Tool::Select))
}

pub(crate) fn begin_with_tool(x: f32, y: f32, tool_override: Option<Tool>) -> i32 {
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
    s.clear_preview_anim();
    if s.document.is_none() {
        return 0;
    }
    // 新一次按下：先清掉上一次可能残留的 ghost/hidden。
    s.preview.clear();
    let tool = tool_override.unwrap_or(s.tool);
    if tool == Tool::Pan {
        return 0; // 手形：平移交给 Flutter 视口手势
    }
    match s.view_mode {
        ViewMode::Pianoroll => begin_pr(&mut d, &mut s, x, y, tool),
        ViewMode::Arrangement => begin_arr(&mut d, &mut s, x, y, tool),
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
    // 边缘自动滚动触发区（逻辑像素）。手机上 28px 太窄，手指容易落在
    // 触发区外导致“拖到边了但画面不动”；放宽到 48px。
    const ZONE: f32 = 48.0;
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
pub(crate) fn update_paste_ready(d: &mut DragState, s: &RenderScene, x: f32, y: f32) {
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
            // 标尺没有行坐标：轨道沿用起手时的活动轨，只微调 tick。
            if !d.from_ruler {
                d.anchor_row = geom.y_to_row(y).min(s.num_tracks.saturating_sub(1));
            }
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

/// 从选框外音符起手时取消旧选框：专心拖这一颗音符。
pub(crate) fn clear_selection_for_note_drag(s: &mut RenderScene) {
    if let Some(doc) = s.document.as_mut() {
        doc.edit.selected.clear();
        doc.edit.sel_rect.clear();
        doc.edit.arr_sel_rect.clear();
        doc.edit.chan_ar_sel.clear();
        doc.edit.track_selected.clear();
    }
    scene::selection_changed(s);
    bump(s);
}

/// 单指从音符左/右端起手：直接进入 NOTE_RESIZE（仅供 Flutter 在平移前尝试）。
pub fn begin_note_resize(x: f32, y: f32) -> i32 {
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
    if s.view_mode != ViewMode::Pianoroll || s.document.is_none() {
        return 0;
    }
    s.clear_preview_anim();
    let geom = PrGeom::new(&s.pr_view);
    if x < geom.kbw {
        return 0;
    }
    let Some((track, start_tick, end_tick, key, side, id)) = hit_note(&s, &geom, x, y) else {
        return 0;
    };
    if side == 0 {
        return 0; // 中部留给移动
    }
    // 按点不在选框内才会走到这里：取消旧选框，专心改这一颗音符。
    clear_selection_for_note_drag(&mut s);
    d.kind = kind::NOTE_RESIZE;
    d.track = track;
    d.start_tick = start_tick;
    d.end_tick = end_tick;
    d.key = key;
    d.side = side;
    d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::single(
        id, key,
    )));
    if let Some(velocity) = note_velocity(&s, track, start_tick, key) {
        preview_note(
            &s,
            track,
            key,
            velocity,
            start_tick,
            end_tick.saturating_sub(start_tick),
        );
    }
    1
}

/// 直接起手拖动选框控制点（PR/AR 共用）。返回 1 = 命中锚点。
pub fn begin_selection_handle(x: f32, y: f32) -> i32 {
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
    let Ok(mut s) = scene::scene().lock() else {
        return 0;
    };
    if s.document.is_none() {
        return 0;
    }
    // Conductor 只在 PR 里不可编辑；AR 第 0 行是合法轨道行。
    if s.view_mode == ViewMode::Pianoroll && s.active_track == Some(0) {
        return 0;
    }
    let handle = match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            pr_selection_handle(&s, &geom, x, y)
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            ar_selection_handle(&s, &geom, x, y)
        }
    };
    let Some(h) = handle else {
        return 0;
    };
    s.clear_preview_anim();
    begin_sel_handle(&mut d, &mut s, h, x);
    1
}

/// 直接起手拖动已有选框（按在选框内任意位置，含选框内音符）。返回 1 = 命中。
pub fn begin_selection_move(x: f32, y: f32) -> i32 {
    let Ok(mut d) = DRAG.lock() else {
        return 0;
    };
    let Ok(mut s) = scene::scene().lock() else {
        return 0;
    };
    if s.document.is_none() || s.active_track == Some(0) {
        return 0;
    }
    // 按下时 `begin_pr`/`begin_arr` 已尝试取过后台物化的 ghost；手势真正
    // 开始时只重置起点并复用，避免同一选区被 O(N) 扫描两遍。
    let prev_drag = d.drag.clone();
    let prev_hidden = d.drag_hidden.clone();
    *d = DragState {
        x0: x,
        y0: y,
        px: x,
        py: y,
        ..Default::default()
    };
    let q = quantize_of(&s);
    let snap = scene::SnapCtx::new(&s);
    match s.view_mode {
        ViewMode::Pianoroll => {
            let Some(rect) = pr_box_at(&s, x, y) else {
                return 0;
            };
            let geom = PrGeom::new(&s.pr_view);
            d.anchor_tick_raw = geom.x_to_tick(x);
            d.anchor_tick = snap.round(q, d.anchor_tick_raw);
            d.anchor_key = geom.y_to_key(y);
            d.box_ticks = Some(rect);
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            let Some(rect) = arr_box_at(&s, &geom, x, y) else {
                return 0;
            };
            d.anchor_tick_raw = geom.x_to_tick(x);
            d.anchor_tick = snap.round(q, d.anchor_tick_raw);
            d.anchor_row = geom.y_to_row(y);
            d.box_rows = Some(rect);
        }
    }
    s.clear_preview_anim();
    d.drag = prev_drag.or_else(|| crate::selection_materializer::take_drag(&s));
    d.drag_hidden = d
        .drag
        .as_ref()
        .map(|drag| std::sync::Arc::clone(&drag.hidden))
        .or(prev_hidden);
    if d.drag.is_none() {
        crate::selection_materializer::schedule_drag(&s);
    }
    d.kind = kind::MOVE_BOX;
    refresh_preview(&mut d, &mut s);
    bump(&mut s);
    1
}

/// 单指从音符中部起手：直接进入 NOTE_MOVE（仅供 Flutter 在平移前尝试）。
///
/// 右端仍由 [`begin_note_resize`] 优先接管；左端和中部都算移动。
pub fn begin_note_move(x: f32, y: f32) -> i32 {
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
    if s.view_mode != ViewMode::Pianoroll || s.document.is_none() {
        return 0;
    }
    s.clear_preview_anim();
    let geom = PrGeom::new(&s.pr_view);
    if x < geom.kbw {
        return 0;
    }
    let Some((track, start_tick, end_tick, key, side, id)) = hit_note(&s, &geom, x, y) else {
        return 0;
    };
    if side == 2 {
        return 0; // 右端留给改长度
    }
    // 走到这里说明按点不在选框内（begin_selection_move 已先尝试）：
    // 按在选框外音符上时取消原选框，专心移动这一颗音符。
    clear_selection_for_note_drag(&mut s);
    // 移动量按“按下点 → 当前点”计算，锚点必须是按下点本身，不能是音符起点。
    let q = quantize_of(&s);
    let snap = scene::SnapCtx::new(&s);
    d.kind = kind::NOTE_MOVE;
    d.anchor_tick_raw = geom.x_to_tick(x);
    d.anchor_tick = snap.round(q, d.anchor_tick_raw);
    d.anchor_key = key;
    d.track = track;
    d.start_tick = start_tick;
    d.end_tick = end_tick;
    d.key = key;
    d.side = side;
    d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::single(
        id, key,
    )));
    if let Some(velocity) = note_velocity(&s, track, start_tick, key) {
        preview_note(
            &s,
            track,
            key,
            velocity,
            start_tick,
            end_tick.saturating_sub(start_tick),
        );
    }
    1
}

/// 时间标尺长按：从标尺位置起手插入/粘贴定位（主视口不再画插入线）。
///
/// 返回 1 = 已进入待长按状态；后续由 Flutter 调用 `long_press` 推进。
pub fn begin_ruler_insert(x: f32, y: f32) -> i32 {
    let Ok(mut d) = DRAG.lock() else {
        return 0;
    };
    *d = DragState {
        kind: kind::PENDING_PASTE,
        x0: x,
        y0: y,
        px: x,
        py: y,
        from_ruler: true,
        ..Default::default()
    };
    let Ok(s) = scene::scene().lock() else {
        return 0;
    };
    if s.document.is_none() {
        return 0;
    }
    let q = quantize_of(&s);
    let snap = scene::SnapCtx::new(&s);
    d.anchor_tick = snap.round(q, scene::tick_at_x(&s, x));
    d.anchor_row = match s.view_mode {
        ViewMode::Arrangement => s
            .active_track
            .unwrap_or(1)
            .min(s.num_tracks.saturating_sub(1)),
        ViewMode::Pianoroll => write_track(&s).unwrap_or(0),
    };
    1
}

/// 本次长按是否来自时间标尺。
pub fn insert_from_ruler() -> i32 {
    DRAG.lock().map(|d| i32::from(d.from_ruler)).unwrap_or(0)
}

/// 长按点是否落在已有选框内（PR/AR）。
pub fn selection_box_at(x: f32, y: f32) -> i32 {
    let Ok(s) = scene::scene().lock() else {
        return 0;
    };
    let hit = match s.view_mode {
        ViewMode::Pianoroll => pr_box_at(&s, x, y).is_some(),
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            arr_box_at(&s, &geom, x, y).is_some()
        }
    };
    i32::from(hit)
}

/// 命中音符的像素矩形（删除淡出动画用）；无命中返回 `{}`。
pub fn note_rect_at(x: f32, y: f32) -> String {
    let Ok(s) = scene::scene().lock() else {
        return "{}".into();
    };
    if s.view_mode != ViewMode::Pianoroll || s.document.is_none() {
        return "{}".into();
    }
    let geom = PrGeom::new(&s.pr_view);
    let Some((track, start, end, key, _, _id)) = hit_note(&s, &geom, x, y) else {
        return "{}".into();
    };
    let x0 = geom.tick_to_x(f64::from(start));
    let x1 = geom.tick_to_x(f64::from(end));
    let y0 = geom.key_to_y(key);
    let y1 = y0 + geom.kh;
    format!("{{\"x0\":{x0},\"y0\":{y0},\"x1\":{x1},\"y1\":{y1},\"track\":{track}}}")
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
    if s.active_track == Some(0) {
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
    let mut preview: Option<yinhe_audio::PreviewNoteParams> = None;
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
            let action = doc.add_note(track as u16, note).map(|action| {
                doc.push_undo(action, "undo.add_note", before);
            });
            // 点按落音也要试听，和拖动音符改音高一致。
            if action.is_some()
                && let Some(t) = doc.data.model.tracks.get(track)
            {
                preview = Some(yinhe_audio::PreviewNoteParams {
                    channel: t.global_channel(),
                    key,
                    velocity: DEFAULT_VELOCITY,
                    target_tick: start,
                    duration_ticks: interval,
                });
            }
            action
        }
    };
    if edited.is_none() {
        return 0;
    }
    let model = after_edit(&mut s);
    drop(s);
    if let Some(note) = preview {
        crate::audio::preview_notes(vec![note], true);
    }
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
        kind::SEL_GATE => commit_selection_gate(&mut s, &d, x, y, moved),
        kind::SEL_SCALE => commit_selection_scale(&mut s, &d, x, moved),
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
    // 锚点缩放松手后让缓动继续跑完，避免最后一下直接跳到目标。
    let sel_tail = if matches!(k, kind::SEL_SCALE | kind::SEL_GATE) {
        s.sel_target.map(|t| (t, s.sel_anim))
    } else {
        None
    };
    d.kind = kind::NONE;
    s.preview.clear();
    s.clear_preview_anim();
    if let Some((target, anim)) = sel_tail {
        s.sel_target = Some(target);
        s.sel_anim = anim;
        s.anim_last = Some(std::time::Instant::now());
    }
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
    d.kind = kind::NONE;
    drop(d);
    if let Ok(mut s) = scene::scene().lock() {
        s.preview.clear();
        s.clear_preview_anim();
        s.overlay_cells.clear();
        bump(&mut s);
    }
    crate::audio::stop_preview();
}

// ── PR 起始分发 ──

pub(crate) fn begin_pr(d: &mut DragState, s: &mut RenderScene, x: f32, y: f32, tool: Tool) {
    let geom = PrGeom::new(&s.pr_view);
    if x < geom.kbw {
        return; // 键盘区不响应编辑
    }
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    // 内容锚点：吸附后的 tick（滚动视图后仍固定，不随视图漂移）。
    let anchor_raw = geom.x_to_tick(x);
    let anchor_tick = snap.round(q, anchor_raw);
    let anchor_key = geom.y_to_key(y);
    d.anchor_tick_raw = anchor_raw;
    // 选框工具：先命中四边圆形控制点 / 右下角 Riff 方块。
    if tool == Tool::Select
        && let Some(h) = pr_selection_handle(s, &geom, x, y)
    {
        begin_sel_handle(d, s, h, x);
        return;
    }
    match tool {
        Tool::Select => {
            d.anchor_tick = anchor_tick;
            d.anchor_key = anchor_key;
            if let Some(rect) = pr_box_at(s, x, y) {
                // 进入长按待定；500ms 后由 Flutter long press 推进到移动态。
                d.kind = kind::PENDING;
                d.box_ticks = Some(rect);
                // ghost 来自后台物化缓存；没有就安排物化，结果到了再补上。
                d.drag = crate::selection_materializer::take_drag(s);
                d.drag_hidden = d
                    .drag
                    .as_ref()
                    .map(|drag| std::sync::Arc::clone(&drag.hidden));
                if d.drag.is_none() {
                    crate::selection_materializer::schedule_drag(s);
                }
                d.preview_last_dk = 0;
                preview_group(s, d, 0, 0);
                return;
            }
            // 空白处：先进入长按待定。拖动超过阈值会转成普通框选；
            // 按住 500ms 松手则弹出粘贴菜单。
            d.kind = kind::PENDING_PASTE;
            s.clear_preview_anim();
            bump(s);
        }
        Tool::Pencil => {
            d.anchor_tick = anchor_tick;
            d.anchor_key = anchor_key;
            // 铅笔压在已有音符上：直接拖动该音符；空白处才新建。
            if let Some((track, start, end, key, _, id)) = hit_note(s, &geom, x, y) {
                d.kind = kind::NOTE_MOVE;
                d.track = track;
                d.start_tick = start;
                d.end_tick = end;
                d.key = key;
                d.side = 0;
                d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::single(
                    id, key,
                )));
                d.preview_last_dk = 0;
                if let Some(velocity) = note_velocity(s, track, start, key) {
                    preview_note(s, track, key, velocity, start, end.saturating_sub(start));
                }
                return;
            }
            // 空白处：保持原有铅笔新建。
            let gate = f64::from(q.tick_interval(s.ppq).max(1));
            d.kind = kind::PENCIL;
            d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::default()));
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
            d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::default()));
            d.anchor_tick = anchor_tick;
            d.anchor_key = anchor_key;
            s.set_box_target([x, y, x, y]);
            bump(s);
        }
        Tool::Brush => {
            let interval = u64::from(q.tick_interval(s.ppq).max(1));
            d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::default()));
            let cell = ((anchor_tick as u64 / interval) as u32, anchor_key);
            d.kind = kind::BRUSH;
            d.last_cell = cell;
            d.cells.clear();
            d.cells.push(cell);
            brush_cells(s, d, &geom, interval as u32);
        }
        Tool::Line => {
            d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::default()));
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
pub(crate) fn begin_arr(d: &mut DragState, s: &mut RenderScene, x: f32, y: f32, tool: Tool) {
    let geom = ArGeom::new(&s.arr_view);
    if x < geom.panel_w {
        return;
    }
    if tool != Tool::Select {
        return; // AR 只开放选框与手形
    }
    let q = quantize_of(s);
    let snap = scene::SnapCtx::new(s);
    let anchor_raw = geom.x_to_tick(x);
    let anchor_tick = snap.round(q, anchor_raw);
    let anchor_row = geom.y_to_row(y);
    d.anchor_tick_raw = anchor_raw;
    d.anchor_tick = anchor_tick;
    d.anchor_row = anchor_row;
    if let Some(h) = ar_selection_handle(s, &geom, x, y) {
        begin_sel_handle(d, s, h, x);
        return;
    }
    if let Some(rect) = arr_box_at(s, &geom, x, y) {
        d.kind = kind::PENDING;
        d.box_rows = Some(rect);
        d.drag = crate::selection_materializer::take_drag(s);
        d.drag_hidden = d
            .drag
            .as_ref()
            .map(|drag| std::sync::Arc::clone(&drag.hidden));
        if d.drag.is_none() {
            crate::selection_materializer::schedule_drag(s);
        }
        return;
    }
    // 空白处：长按待定（同上），拖动后转框选。
    d.kind = kind::PENDING_PASTE;
    s.clear_preview_anim();
    bump(s);
}

// ── 命中测试与选框查询 ──
