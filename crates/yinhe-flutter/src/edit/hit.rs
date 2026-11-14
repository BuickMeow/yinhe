use super::*;

/// PR 命中音符：返回 `(track, start_tick, end_tick, key, side)`，side 0=移动 1=左缘 2=右缘。
pub(crate) fn hit_note(s: &RenderScene, geom: &PrGeom, x: f32, y: f32) -> Option<NoteHit> {
    let doc = s.document.as_ref()?;
    let key = geom.y_to_key(y);
    let tick = geom.x_to_tick(x);
    if tick < 0.0 {
        return None;
    }
    let tick_u = tick.floor() as u32;
    let write = write_track(s)?;
    // 时间轴方向允许外扩 NOTE_HIT_PAD 像素；命中多个时取离按下点最近的音符，
    // 避免外扩后抢到相邻音符。
    let pad_ticks = (NOTE_HIT_PAD / geom.ppu.max(0.0001)).ceil() as i64;
    let mut best: Option<(f32, NoteHit)> = None;
    for note in doc.data.model.key_notes(key).iter() {
        if f64::from(note.start_tick) > f64::from(tick_u) + f64::from(pad_ticks as u32)
            || i64::from(note.end_tick) < i64::from(tick_u) - pad_ticks
        {
            continue;
        }
        let t = note.track as usize;
        if t != write {
            continue;
        }
        let (x0, x1) = (
            geom.tick_to_x(f64::from(note.start_tick)),
            geom.tick_to_x(f64::from(note.end_tick)),
        );
        let (y0, y1) = (geom.key_to_y(key), geom.key_to_y(key) + geom.kh);
        if y < y0 || y > y1 {
            continue;
        }
        let dist = if x < x0 {
            x0 - x
        } else if x > x1 {
            x - x1
        } else {
            0.0
        };
        if dist > NOTE_HIT_PAD {
            continue;
        }
        let edge = EDGE.min((x1 - x0) / 3.0);
        let side = if x < x0 {
            1
        } else if x > x1 || x1 - x <= edge {
            2
        } else if x - x0 <= edge {
            1
        } else {
            0
        };
        let hit = (
            note.track,
            note.start_tick,
            note.end_tick,
            key,
            side,
            note.id,
        );
        if best.as_ref().is_none_or(|(best_dist, _)| dist < *best_dist) {
            best = Some((dist, hit));
        }
    }
    best.map(|(_, hit)| hit)
}

/// PR 已选框命中（像素坐标，取第一个）。
pub(crate) fn pr_box_at(s: &RenderScene, x: f32, y: f32) -> Option<(f64, f64, u8, u8)> {
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
pub(crate) fn arr_box_at(
    s: &RenderScene,
    geom: &ArGeom,
    x: f32,
    y: f32,
) -> Option<(f64, f64, usize, usize)> {
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
