use super::*;

/// 选框右侧三个方块控制点：
/// - 右下角 `4`：重复（Riff）；
/// - 右缘中点 `3`：整体变速；
/// - 右缘内缩一个量化格 `5`：每个音符批量改 gate（仅 PR）。
///
/// 几何与命中判定统一走 `yinhe_editor_core::selection_handle`，与桌面端同源。
/// `interval_px` 为 `None` 时（AR）不生成 gate 控制点。
pub(crate) fn selection_handle_hit(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x: f32,
    y: f32,
    interval_px: Option<f32>,
) -> Option<u8> {
    let kind = yinhe_editor_core::selection_handle::hit_test(
        (x0, y0, x1, y1),
        interval_px.unwrap_or(0.0),
        false,
        interval_px.is_some(),
        x,
        y,
        SEL_HANDLE_R,
    )?;
    Some(kind.code())
}

pub(crate) fn pr_selection_handle(
    s: &RenderScene,
    geom: &PrGeom,
    x: f32,
    y: f32,
) -> Option<SelHandle> {
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
    let interval_px = quantize_of(s).tick_interval(s.ppq).max(1) as f32 * geom.ppu;
    let edge = selection_handle_hit(x0, y0, x1, y1, x, y, Some(interval_px))?;
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

pub(crate) fn ar_selection_handle(
    s: &RenderScene,
    geom: &ArGeom,
    x: f32,
    y: f32,
) -> Option<SelHandle> {
    let doc = s.document.as_ref()?;
    if doc.edit.arr_sel_rect.len() != 1 {
        return None;
    }
    let (t0, t1, r0, r1) = doc.edit.arr_sel_rect[0];
    let x0 = geom.tick_to_x(t0);
    let x1 = geom.tick_to_x(t1);
    let y0 = geom.row_to_y(r0);
    let y1 = geom.row_to_y(r1 + 1);
    let edge = selection_handle_hit(x0, y0, x1, y1, x, y, None)?;
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
pub(crate) fn write_track(s: &RenderScene) -> Option<usize> {
    let n = s.num_tracks;
    if n == 0 {
        return None;
    }
    // PR 只显示 Conductor 时没有可编辑音符。以前这里会落到第 1 轨，
    // 导致框选/落笔改到看不见的轨道上（还会画出别的轨的 ghost）。
    if s.display_track == Some(0) {
        return None;
    }
    // Conductor 作为当前编辑轨时同样没有可写音符（即使「全部轨道」打开，
    // 也不能把编辑落到第 1 轨或拖动其他轨的音符）。
    let t = match s.display_track.or(s.active_track) {
        Some(0) => return None,
        Some(t) => t.min(n - 1),
        None if n > 1 => 1,
        None => return None,
    };
    if t == 0 { None } else { Some(t) }
}

/// 当前视图量化档位的 tick 间隔（AM 选框按量化吸附用）。
pub fn quantize_tick_interval() -> u32 {
    let Ok(s) = scene::scene().lock() else {
        return 1;
    };
    quantize_of(&s).tick_interval(s.ppq).max(1)
}

/// 当前量化档位（Auto 按视图缩放解析）。
pub(crate) fn quantize_of(s: &RenderScene) -> QuantizePreset {
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

pub(crate) fn bump(s: &mut RenderScene) {
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
}

// ── 拖拽生命周期 ──
