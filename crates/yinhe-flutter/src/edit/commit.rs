use super::*;
use yinhe_editor_core::Document;

/// PR/AR 框选提交；`moved < 阈值` 视为点击（清空选择）。
///
/// 返回点击位置（tick，需在**解锁后**由调用方设置播放光标——锁内二次加锁会死锁）。
pub(crate) fn commit_marquee(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    y: f32,
    moved: bool,
) -> Option<f64> {
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
                    tb = snap
                        .min_selection_end(f64::from(ta), f64::from(interval))
                        .round() as u32;
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
                    tb = snap
                        .min_selection_end(f64::from(ta), f64::from(interval))
                        .round() as u32;
                }
                let r_cur = geom_ar.y_to_row(y);
                let max_row = num_tracks.saturating_sub(1);
                let (rlo, rhi) = (
                    d.anchor_row.min(r_cur).min(max_row),
                    d.anchor_row.max(r_cur).min(max_row),
                );
                let (tr_lo, tr_hi) = (rlo, rhi);
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

/// 选框右缘整体变速：新跨度（t0 固定，右端跟随指针并按量化吸附）。
pub(crate) fn selection_scale_span(s: &RenderScene, d: &DragState, x: f32) -> f64 {
    let Some((t0, t1, _, _)) = d.box_ticks else {
        return 1.0;
    };
    let q = quantize_of(s);
    let min_ticks = f64::from(q.tick_interval(s.ppq).max(1));
    let min_span = (t1 - t0).min(min_ticks).max(1.0);
    let current = match s.view_mode {
        ViewMode::Pianoroll => PrGeom::new(&s.pr_view).x_to_tick(x),
        ViewMode::Arrangement => ArGeom::new(&s.arr_view).x_to_tick(x),
    };
    let raw = drag_delta_ticks(s, q, d.anchor_tick_raw, current);
    (t1 + raw - t0).max(min_span)
}

/// 整体变速预览：选中音符相对 t0 等比缩放，原位置隐藏。
pub(crate) fn update_sel_scale(d: &mut DragState, s: &mut RenderScene, x: f32) {
    let Some((t0, t1, k0, k1)) = d.box_ticks else {
        return;
    };
    let new_span = selection_scale_span(s, d, x);
    let old_span = (t1 - t0).max(1.0);
    let factor = new_span / old_span;
    let stamp = (
        new_span.round() as i64,
        0,
        match s.view_mode {
            ViewMode::Pianoroll => s.pr_view.render_hash(),
            ViewMode::Arrangement => s.arr_view.render_hash(),
        },
    );
    let (lw, lh) = crate::logical_size();
    match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            s.set_sel_target([
                geom.tick_to_x(t0),
                geom.key_to_y(k1),
                geom.tick_to_x(t0 + new_span),
                geom.key_to_y(k0) + geom.kh,
            ]);
            s.hold_sel_edge(0);
            s.hold_sel_edge(1);
            s.hold_sel_edge(3);
            if d.last_ghost != Some(stamp) {
                d.last_ghost = Some(stamp);
                let (tick_lo, tick_hi) = s
                    .pr_view
                    .visible_main_range(s.pr_view.main_axis_len(lw, lh));
                let (key_lo, key_hi) = s
                    .pr_view
                    .visible_cross_range(s.pr_view.cross_axis_len(lw, lh));
                let mut ghost = Vec::new();
                if let Some(drag) = d.drag.as_ref() {
                    let scale = |v: f64| (t0 + (v - t0) * factor).round().max(t0);
                    for (key, bucket) in drag.by_key.iter().enumerate() {
                        if (key as u8) < key_lo || (key as u8) > key_hi {
                            continue;
                        }
                        for &(track, start, end, _vel) in bucket {
                            let new_start = scale(f64::from(start)) as u32;
                            let new_end =
                                (scale(f64::from(end)) as u32).max(new_start.saturating_add(1));
                            if f64::from(new_start) >= tick_hi || f64::from(new_end) <= tick_lo {
                                continue;
                            }
                            ghost.push((new_start, new_end, key as u8, track));
                        }
                    }
                }
                apply_preview(s, d, std::mem::take(&mut ghost), true);
            }
        }
        ViewMode::Arrangement => {
            let Some((_, _, r0, r1)) = d.box_rows else {
                return;
            };
            let geom = ArGeom::new(&s.arr_view);
            s.set_sel_target([
                geom.tick_to_x(t0),
                geom.row_to_y(r0),
                geom.tick_to_x(t0 + new_span),
                geom.row_to_y(r1 + 1),
            ]);
            s.hold_sel_edge(0);
            s.hold_sel_edge(1);
            s.hold_sel_edge(3);
            if d.last_ghost != Some(stamp) {
                d.last_ghost = Some(stamp);
                let (tick_lo, tick_hi) = s.arr_view.visible_tick_range(lw);
                let mut ghost = Vec::new();
                if let Some(drag) = d.drag.as_ref() {
                    let scale = |v: f64| (t0 + (v - t0) * factor).round().max(t0);
                    for (key, bucket) in drag.by_key.iter().enumerate() {
                        for &(track, start, end, _vel) in bucket {
                            let new_start = scale(f64::from(start)) as u32;
                            let new_end =
                                (scale(f64::from(end)) as u32).max(new_start.saturating_add(1));
                            if f64::from(new_start) >= tick_hi || f64::from(new_end) <= tick_lo {
                                continue;
                            }
                            ghost.push((new_start, new_end, key as u8, track));
                        }
                    }
                }
                apply_preview(s, d, std::mem::take(&mut ghost), true);
            }
        }
    }
    bump(s);
}

/// 整体变速提交：按新跨度等比缩放选中音符（一步 undo）。
pub(crate) fn commit_selection_scale(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    moved: bool,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    if !moved {
        return None;
    }
    let new_span = selection_scale_span(s, d, x).round().max(1.0) as u64;
    let view_mode = s.view_mode;
    let doc = s.document.as_mut()?;
    let before = doc.capture_snapshot();
    let action = match view_mode {
        ViewMode::Pianoroll => doc.rescale_selection_span(new_span),
        ViewMode::Arrangement => doc.rescale_selection_span_with_automation(new_span),
    };
    if let Some(action) = action {
        doc.push_undo(action, "整体变速", before);
        return after_edit(s);
    }
    None
}

/// 已选框整体移动提交。
/// gate 拉伸预览/提交共用的最短长度：长音符至少一个量化格，短音符至少 1 tick。
pub(crate) fn clamp_gate_end(start: u32, end: u32, dt: i64, min_gate: u32) -> u32 {
    let min = end.saturating_sub(start).min(min_gate).max(1);
    (i64::from(end) + dt).max(i64::from(start) + i64::from(min)) as u32
}

/// 选框右端整体 gate 拖动：最终 dt（已按量化格对齐并保证选区至少一格）。
pub(crate) fn selection_gate_dt(s: &RenderScene, d: &DragState, x: f32) -> f64 {
    let Some((t0, t1, _, _)) = d.box_ticks else {
        return 0.0;
    };
    let q = quantize_of(s);
    let min_ticks = f64::from(q.tick_interval(s.ppq).max(1));
    // 原选框本身小于一个量化格时，以原宽度为下限，不要一按就把它撑大。
    let min_span = (t1 - t0).min(min_ticks).max(1.0);
    let current = match s.view_mode {
        ViewMode::Pianoroll => PrGeom::new(&s.pr_view).x_to_tick(x),
        ViewMode::Arrangement => ArGeom::new(&s.arr_view).x_to_tick(x),
    };
    let raw = drag_delta_ticks(s, q, d.anchor_tick_raw, current);
    (t1 + raw).max(t0 + min_span) - t1
}

/// 选框右端整体 gate 提交：所有选中音符 end + dt，选区右边界同步。
pub(crate) fn commit_selection_gate(
    s: &mut RenderScene,
    d: &DragState,
    x: f32,
    _y: f32,
    moved: bool,
) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    if !moved {
        return None;
    }
    let dt = selection_gate_dt(s, d, x).round() as i64;
    if dt == 0 {
        return None;
    }
    let min_gate = quantize_of(s).tick_interval(s.ppq).max(1);
    let doc = s.document.as_mut()?;
    let before = doc.capture_snapshot();
    if let Some(action) = doc.resize_selected_notes_min_gate(ResizeSide::Right, dt, min_gate) {
        doc.push_undo(action, "undo.resize_notes", before);
        match s.view_mode {
            ViewMode::Pianoroll => {
                if let Some(r) = doc.edit.sel_rect.rects.first_mut() {
                    r.1 = (r.1 + dt as f64).max(r.0 + 1.0);
                }
            }
            ViewMode::Arrangement => {
                if let Some(r) = doc.edit.arr_sel_rect.first_mut() {
                    r.1 = (r.1 + dt as f64).max(r.0 + 1.0);
                }
            }
        }
        return after_edit(s);
    }
    None
}

pub(crate) fn commit_move(
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
                    let klo = (i32::from(r.2) + dk).clamp(0, 255) as u8;
                    let khi = (i32::from(r.3) + dk).clamp(0, 255) as u8;
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
pub(crate) fn commit_pick(s: &mut RenderScene, x: f32, y: f32) -> bool {
    let geom = PrGeom::new(&s.pr_view);
    let Some((track, start, end, key, _, _id)) = hit_note(s, &geom, x, y) else {
        return false;
    };
    select_note(s, track, start, end, key);
    after_selection_change(s);
    true
}

/// 右下角 Riff 方块释放：按选区宽度向左右重复，一步 undo。
pub(crate) fn commit_riff(
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
    let sel_start = t0.round() as i64;
    let sel_end = t1.round() as i64;
    let view_mode = s.view_mode;
    let changed = {
        let doc = s.document.as_mut()?;
        if doc.edit.selected.is_empty() {
            return None;
        }
        let before = doc.capture_snapshot();
        let extent = steps.extent_x;
        let new_t0 = (t0 + extent.min(0) as f64).max(0.0);
        let new_t1 = t1 + extent.max(0) as f64;
        match view_mode {
            ViewMode::Pianoroll => {
                let action = doc.repeat_selected_keys_extent(
                    sel_start,
                    sel_end,
                    steps.extent_x,
                    i64::from(k0),
                    i64::from(k1) + 1,
                    steps.extent_y,
                )?;
                let new_k0 = (i64::from(k0) + steps.extent_y.min(0))
                    .clamp(0, i64::from(yinhe_types::MAX_KEY)) as u8;
                let new_k1 = (i64::from(k1) + steps.extent_y.max(0))
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
                let action = doc.repeat_selected_tracks_extent(
                    sel_start,
                    sel_end,
                    steps.extent_x,
                    r0 as i64,
                    r1 as i64 + 1,
                    steps.extent_y,
                )?;
                let last = s.num_tracks.saturating_sub(1) as i64;
                let mut new_r0 = (r0 as i64 + steps.extent_y.min(0)).clamp(0, last) as usize;
                let new_r1 = (r1 as i64 + steps.extent_y.max(0)).clamp(0, last) as usize;
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
pub(crate) fn commit_note_move(
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
    let new_start = note_move_start_abs(s, q, d, x);
    let dt = i64::from(new_start) - i64::from(d.start_tick);
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
            let new_key = (i32::from(d.key) + dk).clamp(0, 255) as u8;
            let new_end = new_start.saturating_add(d.end_tick.saturating_sub(d.start_tick));
            select_note(s, d.track, new_start, new_end, new_key);
        }
        return after_edit(s);
    }
    None
}

/// 单音符边缘伸缩提交。
pub(crate) fn commit_note_resize(
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
pub(crate) fn commit_pencil(
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
pub(crate) fn commit_brush(
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
pub(crate) fn commit_erase(
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
                tb = snap
                    .min_selection_end(f64::from(ta), f64::from(interval))
                    .round() as u32;
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
                tb = snap
                    .min_selection_end(f64::from(ta), f64::from(interval))
                    .round() as u32;
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
