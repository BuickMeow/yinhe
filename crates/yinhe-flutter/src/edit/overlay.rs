use super::*;

pub(crate) fn pencil_box(s: &mut RenderScene, d: &DragState, geom: &PrGeom) {
    let x0 = geom.tick_to_x(d.start_f);
    let x1 = geom.tick_to_x(d.cur_end).max(x0 + 2.0);
    let y = geom.key_to_y(d.key);
    s.set_box_static([x0, y, x1, y + geom.kh]);
    bump(s);
}

pub(crate) fn brush_cells(s: &mut RenderScene, d: &DragState, geom: &PrGeom, interval: u32) {
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
pub(crate) fn walk_line(from: (u32, u8), to: (u32, u8), add: &mut impl FnMut(u32, u8)) {
    let (x0, y0) = (i64::from(from.0), i64::from(from.1));
    let (x1, y1) = (i64::from(to.0), i64::from(to.1));
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut cx, mut cy) = (x0, y0);
    loop {
        if cx >= 0 && (0..=255).contains(&cy) {
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
pub(crate) fn after_selection_change(s: &mut RenderScene) {
    scene::selection_changed(s);
}

/// 内容编辑：刷新快照；返回模型供调用方在**解锁后**重载音频。
///
/// 音频首次初始化可能耗时较长，必须避免在持有场景锁时执行（否则渲染线程被卡住，
/// 编辑结果要等音频初始化完才可见）。
pub(crate) fn after_edit(s: &mut RenderScene) -> Option<std::sync::Arc<yinhe_core::YinModel>> {
    let model = s.document.as_ref().map(|d| d.data.model.clone());
    scene::refresh_snapshot(s);
    model
}
