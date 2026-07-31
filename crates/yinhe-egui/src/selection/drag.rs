use eframe::egui;
use yinhe_types::view_base::TimelineViewBase;
use yinhe_types::{key_notes_in_range, NoteSource, TimeSigEvent};
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_editor_core::ResizeSide;

/// Hit-test 边缘的像素阈值（与铅笔工具一致）。
const EDGE_THRESHOLD_PX: f32 = 6.0;

/// 一次拖拽中收集的选中音符信息（move / resize 共用）。
#[derive(Clone)]
pub struct CollectedNote {
    pub track: u16,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
}

/// Auto-scroll the view when the pointer is near the edges of `content_rect`.
/// Returns the actual (dx, dy) scroll delta applied, so callers can compensate
/// drag anchors.
///
/// `clamp_fn` is called after modifying scroll to enforce bounds.
/// It receives `(content_width, content_height)` and should call
/// `view.base.clamp_scroll_x(...)` etc.
pub fn auto_scroll_on_drag(
    ui: &egui::Ui,
    base: &mut TimelineViewBase,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut TimelineViewBase, f32, f32),
) -> (f32, f32) {
    const MARGIN: f32 = 20.0;
    const BASE_SPEED: f32 = 15.0;
    let dt = ui.input(|i| i.unstable_dt);
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;

    if pos.x < content_rect.min.x + MARGIN {
        dx = -(content_rect.min.x + MARGIN - pos.x) * BASE_SPEED * dt;
    } else if pos.x > content_rect.max.x - MARGIN {
        dx = (pos.x - (content_rect.max.x - MARGIN)) * BASE_SPEED * dt;
    }

    if pos.y < content_rect.min.y + MARGIN {
        dy = -(content_rect.min.y + MARGIN - pos.y) * BASE_SPEED * dt;
    } else if pos.y > content_rect.max.y - MARGIN {
        dy = (pos.y - (content_rect.max.y - MARGIN)) * BASE_SPEED * dt;
    }

    if dx != 0.0 || dy != 0.0 {
        let old_x = base.scroll_x;
        let old_y = base.scroll_y;
        base.scroll_x += dx;
        base.scroll_y += dy;
        clamp_fn(base, content_rect.width(), content_rect.height());
        let actual_dx = base.scroll_x - old_x;
        let actual_dy = base.scroll_y - old_y;
        if actual_dx != 0.0 || actual_dy != 0.0 {
            base.dirty = true;
            ui.ctx().request_repaint();
            return (actual_dx, actual_dy);
        }
    }
    (0.0, 0.0)
}

/// Convert a persisted music selection `(t_start, t_end, key_lo, key_hi)` to
/// a pixel-space `Rect` in the pianoroll view.
pub fn music_sel_to_pixel_rect(
    base: &TimelineViewBase,
    key_height: f32,
    t_start: f64,
    t_end: f64,
    key_lo: u8,
    key_hi: u8,
) -> egui::Rect {
    let kh = key_height;
    let scroll_y = base.scroll_y;
    let sy = (127.0 - key_hi as f32) * kh - scroll_y;
    let ey = (127.0 - key_lo as f32 + 1.0) * kh - scroll_y;
    let sx = base.tick_to_x(t_start);
    let ex = base.tick_to_x(t_end);
    egui::Rect::from_min_max(
        egui::pos2(sx.min(ex), sy.min(ey)),
        egui::pos2(sx.max(ex), sy.max(ey)),
    )
}

/// 预计算选中音符信息（move 和 resize 共用）。
///
/// 使用 `Selection::contains` 做精确的半开 tick 过滤，并叠加 track_selected
/// 和 track_visible 过滤，与 note layer 的 build_notes 对齐。
/// `track_selected` 传空集合表示不过滤轨道（如 AR 视图）。
pub fn collect_selected_notes(
    selected: &yinhe_core::Selection,
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
) -> Vec<CollectedNote> {
    selected.rects.iter().flat_map(|&(ts, te, kl, kh, _tl, _th)| {
        (kl..=kh).flat_map(move |key| {
            midi.map(|m| {
                key_notes_in_range(m.key_notes(key), ts, te).iter()
                    .filter(|n| selected.contains(n.track, n.start_tick, key))
                    .filter(|n| track_selected.is_empty() || track_selected.contains(&n.track))
                    .filter(|n| track_visible.get(n.track as usize).copied().unwrap_or(true))
                    .map(|n| CollectedNote {
                        track: n.track,
                        start_tick: n.start_tick,
                        end_tick: n.end_tick,
                        key,
                    })
                    .collect::<Vec<_>>()
            }).unwrap_or_default()
        })
    }).collect()
}

/// Hit-test 鼠标是否在某个选框的左右边缘 `EDGE_THRESHOLD_PX` 内。
///
/// 返回 `(side, origin_boundary_tick, other_boundary_tick)`：
/// - `origin_boundary_tick`：被拖动边缘的原 tick
/// - `other_boundary_tick`：另一个边缘的原 tick（用于计算最小宽度约束）
///
/// 窄选框（宽度 <= 2×阈值）跳过边缘检测，避免与 move 冲突。
pub fn hit_test_sel_edge(
    eff_rects: &[(f64, f64, u8, u8)],
    base: &TimelineViewBase,
    key_height: f32,
    local: egui::Pos2,
) -> Option<(ResizeSide, f64, f64)> {
    for &(t_start, t_end, key_lo, key_hi) in eff_rects {
        let pixel_rect = music_sel_to_pixel_rect(base, key_height, t_start, t_end, key_lo, key_hi);
        // y 不在选框纵向范围（含阈值）内 → 跳过
        if local.y < pixel_rect.min.y - EDGE_THRESHOLD_PX
            || local.y > pixel_rect.max.y + EDGE_THRESHOLD_PX
        {
            continue;
        }
        // 窄选框跳过边缘检测，避免与 move 冲突
        if pixel_rect.width() <= 2.0 * EDGE_THRESHOLD_PX {
            continue;
        }
        // 左边缘
        if (local.x - pixel_rect.min.x).abs() <= EDGE_THRESHOLD_PX {
            return Some((ResizeSide::Left, t_start, t_end));
        }
        // 右边缘
        if (local.x - pixel_rect.max.x).abs() <= EDGE_THRESHOLD_PX {
            return Some((ResizeSide::Right, t_end, t_start));
        }
    }
    None
}

/// 计算 resize 的 dt（按量化 snap），并约束最小宽度为一个量化间隔。
///
/// - `origin_boundary_tick`：被拖动边缘的原 tick
/// - `other_boundary_tick`：另一个边缘的原 tick
/// - 返回 `(snapped_boundary, dt)`：dt 已 clamp 使得 new_width >= interval
#[allow(clippy::too_many_arguments)]
pub fn compute_resize_dt(
    raw_tick: f64,
    side: ResizeSide,
    origin_boundary_tick: f64,
    other_boundary_tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f64, i64) {
    let interval = quantize.tick_interval(ppq) as f64;
    let original_width = (origin_boundary_tick - other_boundary_tick).abs();
    // 最小允许宽度：如果原选框已经 < interval，不允许再压缩（min = original_width）
    let min_width = original_width.min(interval).max(1.0);

    match side {
        ResizeSide::Right => {
            let snapped = crate::view_interaction::snap_tick_ceil(raw_tick, quantize, ppq, bar_line_data);
            let mut dt = (snapped - origin_boundary_tick).round() as i64;
            // new_width = original_width + dt >= min_width
            let dt_min = (min_width - original_width).ceil() as i64;
            dt = dt.max(dt_min);
            (origin_boundary_tick + dt as f64, dt)
        }
        ResizeSide::Left => {
            let snapped = crate::view_interaction::snap_tick_floor(raw_tick, quantize, ppq, bar_line_data);
            let mut dt = (snapped - origin_boundary_tick).round() as i64;
            // new_width = original_width - dt >= min_width → dt <= original_width - min_width
            let dt_max = (original_width - min_width).floor() as i64;
            dt = dt.min(dt_max);
            (origin_boundary_tick + dt as f64, dt)
        }
    }
}
