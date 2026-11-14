use super::*;

/// 选区整体移动的当前增量：PR 返回 `(tick 差, dk, 0)`，AR 返回 `(tick 差, 0, dtr)`。
///
/// tick 差保留未取整值（选框矩形预览用），ghost/提交再 round；已按选框
/// 左边界 clamp 到 >= 0。预览、提交、顶部栏读数共用这一份换算，避免漂移。
/// 拖动位移按整格量化：指针越过一条网格线才走一格。
///
/// 不能用 round：按在半格附近时，指针只动一点就会因为四舍五入跨到下一格，
/// 在 DAW 里会表现为“刚拖就跳一格”。向右取 floor、向左取 ceil，保持迟滞。
pub(crate) fn drag_delta_ticks(
    s: &RenderScene,
    q: QuantizePreset,
    anchor_raw: f64,
    current_raw: f64,
) -> f64 {
    let interval = f64::from(q.tick_interval(s.ppq).max(1));
    let delta = current_raw - anchor_raw;
    if delta >= 0.0 {
        (delta / interval).floor() * interval
    } else {
        (delta / interval).ceil() * interval
    }
}

/// 选区整体移动的 delta：锚定选区起点，而不是手指按下点。
///
/// 起点不在量化格上时（例如导入 MIDI 落在工程末尾的非整格处），第一次
/// 拖动把起点吸到拖动方向的下一条量化线；起点已在格上时保持旧的整格移动。
pub(crate) fn selection_move_delta(
    s: &RenderScene,
    q: QuantizePreset,
    sel_start: f64,
    anchor_raw: f64,
    current_raw: f64,
) -> f64 {
    let interval = f64::from(q.tick_interval(s.ppq).max(1));
    let ratio = sel_start / interval;
    if (ratio - ratio.round()).abs() < 1e-6 {
        return drag_delta_ticks(s, q, anchor_raw, current_raw);
    }
    let raw = current_raw - anchor_raw;
    if raw.abs() < 1e-9 {
        return 0.0;
    }
    let target = sel_start + raw;
    let snap = scene::SnapCtx::new(s);
    let snapped = if raw > 0.0 {
        snap.ceil(q, target)
    } else {
        snap.floor(q, target)
    };
    snapped - sel_start
}

/// 单音符拖动的绝对坐标吸附：按“手指 - 抓取偏移”算出音符新起点，再吸附到
/// 绝对网格，而不是用相对位移累加。这样重拍/切分音等非整格起点也能跟随
/// 指针落到它所在的绝对格位。
pub(crate) fn note_move_start_abs(
    s: &RenderScene,
    q: QuantizePreset,
    d: &DragState,
    x: f32,
) -> u32 {
    let geom = PrGeom::new(&s.pr_view);
    let grab_offset = d.anchor_tick_raw - f64::from(d.start_tick);
    let target = geom.x_to_tick(x) - grab_offset;
    scene::SnapCtx::new(s)
        .round(q, target)
        .max(0.0)
        .min(f64::from(u32::MAX))
        .round() as u32
}

pub(crate) fn move_box_delta(
    s: &RenderScene,
    d: &DragState,
    x: f32,
    y: f32,
) -> Option<(f64, i32, i32)> {
    let q = quantize_of(s);
    match s.view_mode {
        ViewMode::Pianoroll => {
            let (t0, _, _, _) = d.box_ticks?;
            let geom = PrGeom::new(&s.pr_view);
            let mut dt = selection_move_delta(s, q, t0, d.anchor_tick_raw, geom.x_to_tick(x));
            if t0 + dt < 0.0 {
                dt = -t0;
            }
            let dk = i32::from(geom.y_to_key(y)) - i32::from(d.anchor_key);
            Some((dt, dk, 0))
        }
        ViewMode::Arrangement => {
            let (t0, _, _, _) = d.box_rows?;
            let geom = ArGeom::new(&s.arr_view);
            let mut dt = selection_move_delta(s, q, t0, d.anchor_tick_raw, geom.x_to_tick(x));
            if t0 + dt < 0.0 {
                dt = -t0;
            }
            let dtr = geom.y_to_row(y) as i32 - d.anchor_row as i32;
            Some((dt, 0, dtr))
        }
    }
}

/// 橡皮框命中预览：不隐藏原音符，只把将被删除的音符用选中态叠加。
pub(crate) fn build_erase_preview(s: &mut RenderScene, d: &DragState, x: f32, y: f32) {
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
                tb = snap
                    .min_selection_end(f64::from(ta), f64::from(q.tick_interval(s.ppq).max(1)))
                    .round() as u32;
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
                tb = snap
                    .min_selection_end(f64::from(ta), f64::from(q.tick_interval(s.ppq).max(1)))
                    .round() as u32;
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
pub(crate) fn preview_note(
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

pub(crate) fn note_velocity(s: &RenderScene, track: u16, start_tick: u32, key: u8) -> Option<u8> {
    let doc = s.document.as_ref()?;
    doc.data
        .model
        .key_notes(key)
        .iter()
        .find(|n| n.track == track && n.start_tick == start_tick)
        .map(|n| n.velocity)
}

/// 精确匹配 `(track, start, end, key)` 的音符。
pub(crate) fn note_at_bounds(
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
pub(crate) fn select_note(
    s: &mut RenderScene,
    track: u16,
    start_tick: u32,
    end_tick: u32,
    key: u8,
) {
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
pub(crate) fn preview_group(s: &RenderScene, d: &DragState, dt: i64, dk: i32) {
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
    let Some(drag) = d.drag.as_ref() else {
        return;
    };
    let src_lo = (i32::from(key_lo) - dk).clamp(0, 255) as usize;
    let src_hi = (i32::from(key_hi) - dk).clamp(0, 255) as usize;
    let mut notes = Vec::new();
    for src_key in src_lo..=src_hi {
        let new_key = (src_key as i32 + dk).clamp(0, 255) as u8;
        if new_key < key_lo || new_key > key_hi {
            continue;
        }
        for &(track, start, end, velocity) in &drag.by_key[src_key] {
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

pub(crate) fn apply_preview(
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

pub(crate) fn view_tick_at(s: &RenderScene, x: f32) -> f64 {
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

pub(crate) fn riff_extent_x(d: &DragState, s: &RenderScene, x: f32) -> i64 {
    let Some((t0, t1, _, _)) = d.box_ticks else {
        return 0;
    };
    let cur = view_tick_at(s, x);
    if cur > t1 {
        (cur - t1).round() as i64
    } else if cur < t0 {
        // 左侧不能越过 tick 0；裁到选区起点即可。
        (-((t0 - cur).round() as i64)).max(-(t0.round() as i64))
    } else {
        0
    }
}

/// 纵向步数（轴坐标）：PR 正 = key 增大（屏幕向上），AR 正 = track 增大（屏幕向下）。
///
/// 按 key/轨道边界夹住，避免在完全越界的方向上生成预览/复制步。
pub(crate) fn riff_extent_y(d: &DragState, s: &RenderScene, y: f32) -> i64 {
    match s.view_mode {
        ViewMode::Pianoroll => {
            let Some((_, _, k0, k1)) = d.box_ticks else {
                return 0;
            };
            let cur = i64::from(PrGeom::new(&s.pr_view).y_to_key(y));
            if cur < i64::from(k0) {
                cur - i64::from(k0)
            } else if cur > i64::from(k1) {
                cur - i64::from(k1)
            } else {
                0
            }
        }
        ViewMode::Arrangement => {
            let Some((t0, t1, _, _)) = d.box_ticks else {
                return 0;
            };
            let (_, _, r0, r1) = d.box_rows.unwrap_or((t0, t1, 0, 0));
            let cur = ArGeom::new(&s.arr_view).y_to_row(y) as i64;
            if cur > r1 as i64 {
                cur - r1 as i64
            } else if cur < r0 as i64 {
                cur - r0 as i64
            } else {
                0
            }
        }
    }
}

pub(crate) fn riff_steps(d: &DragState, s: &RenderScene, x: f32, y: f32) -> RiffSteps {
    RiffSteps {
        extent_x: riff_extent_x(d, s, x),
        extent_y: riff_extent_y(d, s, y),
    }
}

pub(crate) fn update_riff_fill(d: &mut DragState, s: &mut RenderScene, x: f32, y: f32) {
    let steps = riff_steps(d, s, x, y);
    if steps.is_zero() {
        d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::default()));
        s.riff_preview = false;
        s.overlay_cells.clear();
        s.preview.clear();
        s.clear_box_preview();
        bump(s);
        return;
    }
    let Some((t0, t1, k0, k1)) = d.box_ticks else {
        return;
    };
    let rect = match s.view_mode {
        ViewMode::Pianoroll => {
            let geom = PrGeom::new(&s.pr_view);
            let new_t0 = (t0 + steps.extent_x.min(0) as f64).max(0.0);
            let new_t1 = t1 + steps.extent_x.max(0) as f64;
            let new_k0 = (i64::from(k0) + steps.extent_y.min(0))
                .clamp(0, i64::from(yinhe_types::MAX_KEY)) as u8;
            let new_k1 = (i64::from(k1) + steps.extent_y.max(0))
                .clamp(0, i64::from(yinhe_types::MAX_KEY)) as u8;
            [
                geom.tick_to_x(new_t0),
                geom.key_to_y(new_k1),
                geom.tick_to_x(new_t1),
                geom.key_to_y(new_k0) + geom.kh,
            ]
        }
        ViewMode::Arrangement => {
            let geom = ArGeom::new(&s.arr_view);
            let (_, _, r0, r1) = d.box_rows.unwrap_or((t0, t1, 0, 0));
            let last = s.num_tracks.saturating_sub(1) as i64;
            let new_t0 = (t0 + steps.extent_x.min(0) as f64).max(0.0);
            let new_t1 = t1 + steps.extent_x.max(0) as f64;
            let mut new_r0 = (r0 as i64 + steps.extent_y.min(0)).clamp(0, last) as usize;
            let new_r1 = (r1 as i64 + steps.extent_y.max(0)).clamp(0, last) as usize;
            let conductor = s.document.as_ref().is_some_and(|d| {
                d.data
                    .model
                    .tracks
                    .first()
                    .is_some_and(|t| t.kind == yinhe_core::TrackKind::Master)
            });
            if conductor && new_r1 > 0 {
                new_r0 = new_r0.max(1);
            }
            [
                geom.tick_to_x(new_t0),
                geom.row_to_y(new_r0),
                geom.tick_to_x(new_t1),
                geom.row_to_y(new_r1 + 1),
            ]
        }
    };
    // 重复段不再逐段画框：整个“原选区 + 重复范围”只保留一个选框。
    s.overlay_cells.clear();
    s.riff_preview = true;
    s.set_box_target(rect);
    s.preview.clear();
    bump(s);
}

/// 按最新指针位置与视图刷新拖拽预览（选框/移动框实时吸附量化网格）。
/// AR ghost 目标轨道：与 `offset_track_skip_conductor` 同规则。
pub(crate) fn ghost_track(src: u16, delta: i32, num_tracks: i32, conductor: Option<u16>) -> u16 {
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

/// 构建 AR 自动化竖线 ghost：选中锚点按 `(dt, dtr)` 移动后的副本。
pub(crate) fn build_ar_auto_ghost(s: &RenderScene, dt: i64, dtr: i32) -> Vec<scene::AutoGhostLane> {
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
pub(crate) fn line_cells(d: &DragState, s: &RenderScene, x: f32, y: f32) -> Vec<(u32, u8)> {
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
        let key = (k0 + (k1 - k0) * f).round().clamp(0.0, 255.0) as u8;
        cells.push((idx as u32, key));
    }
    cells
}

/// 把直线格子推给 ghost 层（gate = 一个量化间隔）。
pub(crate) fn preview_line(s: &mut RenderScene, d: &mut DragState) {
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

pub(crate) fn refresh_preview(d: &mut DragState, s: &mut RenderScene) {
    // 大选区 ghost 由后台物化：结果到了就补上（按下那一刻可能还没算完）。
    if d.drag.is_none()
        && matches!(d.kind, kind::MOVE_BOX | kind::SEL_GATE | kind::SEL_SCALE)
        && let Some(drag) = crate::selection_materializer::take_drag(s)
    {
        d.drag_hidden = Some(std::sync::Arc::clone(&drag.hidden));
        d.drag = Some(drag);
    }
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
                        tb = snap.min_selection_end(ta, interval);
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
                        tb = snap.min_selection_end(ta, interval);
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
            s.set_box_target(b);
            // 新建选框：起手那条边固定，只让另一端跟随手指（可越过起点翻转）。
            if d.kind == kind::MARQUEE {
                let (cur_tick, y_edge) = match s.view_mode {
                    ViewMode::Pianoroll => {
                        let geom = PrGeom::new(&s.pr_view);
                        let q = quantize_of(s);
                        let snap = scene::SnapCtx::new(s);
                        let k_cur = geom.y_to_key(y);
                        (
                            snap.round(q, geom.x_to_tick(x)),
                            if d.anchor_key >= k_cur { 1 } else { 3 },
                        )
                    }
                    ViewMode::Arrangement => {
                        let geom = ArGeom::new(&s.arr_view);
                        let q = quantize_of(s);
                        let snap = scene::SnapCtx::new(s);
                        let r_cur = geom.y_to_row(y);
                        (
                            snap.round(q, geom.x_to_tick(x)),
                            if r_cur >= d.anchor_row { 1 } else { 3 },
                        )
                    }
                };
                let x_edge = if cur_tick >= d.anchor_tick { 0 } else { 2 };
                s.hold_box_edge(x_edge);
                s.hold_box_edge(y_edge);
            }
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
                    let klo2 = (i32::from(klo) + dk).clamp(0, 255) as u8;
                    let khi2 = (i32::from(khi) + dk).clamp(0, 255) as u8;
                    s.set_box_target([
                        geom.tick_to_x(t0 + dt_f).max(geom.kbw),
                        geom.key_to_y(khi2),
                        geom.tick_to_x(t1 + dt_f),
                        geom.key_to_y(klo2) + geom.kh,
                    ]);
                    let stamp = (dt, dk, s.pr_view.render_hash());
                    if d.last_ghost != Some(stamp) {
                        if let Some((old_dt, old_dk, old_hash)) = d.last_ghost
                            && old_hash == stamp.2
                        {
                            s.nudge_ghost(
                                (old_dt - dt) as f32 * geom.ppu,
                                // PR 的 key 越大 y 越小，这里要取反，否则动画会从下一格倒着回来。
                                (dk - old_dk) as f32 * geom.kh,
                            );
                        }
                        d.last_ghost = Some(stamp);
                        let (tick_lo, tick_hi) = s
                            .pr_view
                            .visible_main_range(s.pr_view.main_axis_len(lw, lh));
                        let (key_lo, key_hi) = s
                            .pr_view
                            .visible_cross_range(s.pr_view.cross_axis_len(lw, lh));
                        let mut ghost = Vec::new();
                        let src_lo = (i32::from(key_lo) - dk).clamp(0, 255) as usize;
                        let src_hi = (i32::from(key_hi) - dk).clamp(0, 255) as usize;
                        if let Some(drag) = d.drag.as_ref() {
                            for src_key in src_lo..=src_hi {
                                let new_key = (src_key as i32 + dk).clamp(0, 255) as u8;
                                for &(track, start, end, _vel) in &drag.by_key[src_key] {
                                    let new_start = (i64::from(start) + dt).max(0) as u32;
                                    let new_end =
                                        new_start.saturating_add(end.saturating_sub(start));
                                    if f64::from(new_start) >= tick_hi
                                        || f64::from(new_end) <= tick_lo
                                    {
                                        continue;
                                    }
                                    ghost.push((new_start, new_end, new_key, track));
                                }
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
                    s.set_box_target([
                        geom.tick_to_x(t0 + dt_f).max(geom.panel_w),
                        geom.row_to_y(rlo2),
                        geom.tick_to_x(t1 + dt_f),
                        geom.row_to_y(rhi2 + 1),
                    ]);
                    let stamp = (dt, dtr, s.arr_view.render_hash());
                    if d.last_ghost != Some(stamp) {
                        if let Some((old_dt, old_dtr, old_hash)) = d.last_ghost
                            && old_hash == stamp.2
                        {
                            s.nudge_ghost(
                                (old_dt - dt) as f32 * s.arr_view.base.pixels_per_tick,
                                (old_dtr - dtr) as f32 * s.arr_view.lane_height(),
                            );
                        }
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
                        if let Some(drag) = d.drag.as_ref() {
                            for (key, bucket) in drag.by_key.iter().enumerate() {
                                for &(track, start, end, _vel) in bucket {
                                    let new_start = (i64::from(start) + dt).max(0) as u32;
                                    let new_end =
                                        new_start.saturating_add(end.saturating_sub(start));
                                    let new_track =
                                        (i32::from(track) + dtr).clamp(0, max_row) as u16;
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
            if let Some((track, start, end, key, _, id)) = hit_note(s, &geom, x, y) {
                let changed =
                    d.track != track || d.start_tick != start || d.end_tick != end || d.key != key;
                if changed {
                    d.track = track;
                    d.start_tick = start;
                    d.end_tick = end;
                    d.key = key;
                    d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::single(
                        id, key,
                    )));
                    if let Some(velocity) = note_velocity(s, track, start, key) {
                        preview_note(s, track, key, velocity, start, end.saturating_sub(start));
                    }
                }
                apply_preview(s, d, vec![(start, end, key, track)], true);
            } else {
                // 只有真正压在音符上才算候选；空白处不吸附到最近行。
                d.drag_hidden = Some(std::sync::Arc::new(yinhe_types::HiddenNotes::default()));
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
        kind::SEL_GATE => {
            let Some((t0, t1, k0, k1)) = d.box_ticks else {
                return;
            };
            let dt = selection_gate_dt(s, d, x).round() as i64;
            let min_gate = quantize_of(s).tick_interval(s.ppq).max(1);
            let stamp = (
                dt,
                0,
                match s.view_mode {
                    ViewMode::Pianoroll => s.pr_view.render_hash(),
                    ViewMode::Arrangement => s.arr_view.render_hash(),
                },
            );
            match s.view_mode {
                ViewMode::Pianoroll => {
                    let geom = PrGeom::new(&s.pr_view);
                    s.set_sel_target([
                        geom.tick_to_x(t0),
                        geom.key_to_y(k1),
                        geom.tick_to_x(t1 + dt as f64),
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
                            for (key, bucket) in drag.by_key.iter().enumerate() {
                                if (key as u8) < key_lo || (key as u8) > key_hi {
                                    continue;
                                }
                                for &(track, start, end, _vel) in bucket {
                                    let new_end = clamp_gate_end(start, end, dt, min_gate);
                                    if f64::from(start) >= tick_hi || f64::from(new_end) <= tick_lo
                                    {
                                        continue;
                                    }
                                    ghost.push((start, new_end, key as u8, track));
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
                        geom.tick_to_x(t1 + dt as f64),
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
                            for (key, bucket) in drag.by_key.iter().enumerate() {
                                for &(track, start, end, _vel) in bucket {
                                    let new_end = clamp_gate_end(start, end, dt, min_gate);
                                    if f64::from(start) >= tick_hi || f64::from(new_end) <= tick_lo
                                    {
                                        continue;
                                    }
                                    ghost.push((start, new_end, key as u8, track));
                                }
                            }
                        }
                        apply_preview(s, d, std::mem::take(&mut ghost), true);
                    }
                }
            }
            bump(s);
        }
        kind::NOTE_MOVE => {
            let geom = PrGeom::new(&s.pr_view);
            let q = quantize_of(s);
            let new_start = note_move_start_abs(s, q, d, x);
            let dt = i64::from(new_start) - i64::from(d.start_tick);
            let dk = i32::from(geom.y_to_key(y)) - i32::from(d.anchor_key);
            let new_key = (i32::from(d.key) + dk).clamp(0, 255) as u8;
            let len = d.end_tick.saturating_sub(d.start_tick);
            let stamp = (dt, dk, s.pr_view.render_hash());
            if d.last_ghost != Some(stamp) {
                // 首次移动从音符原位置（dt=0）开始缓动，而不是直接跳到第一格。
                let (old_dt, old_dk, same_view) = match d.last_ghost {
                    Some((odt, odk, hash)) => (odt, odk, hash == stamp.2),
                    None => (0, 0, true),
                };
                if same_view {
                    s.nudge_ghost(
                        (old_dt - dt) as f32 * geom.ppu,
                        // 同上：key 增长方向与屏幕 y 相反。
                        (dk - old_dk) as f32 * geom.kh,
                    );
                }
                d.last_ghost = Some(stamp);
                apply_preview(
                    s,
                    d,
                    vec![(new_start, new_start + len, new_key, d.track)],
                    true,
                );
            }
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
        kind::SEL_SCALE => update_sel_scale(d, s, x),
        kind::RIFF_FILL => update_riff_fill(d, s, x, y),
        _ => {
            s.preview.clear();
        }
    }
}
