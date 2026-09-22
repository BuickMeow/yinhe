//! 网格工具：在钢琴卷帘上拉出一个框，框内显示量化网格。
//!
//! 松手后框停留在 `SelRectState`，可整体拖动 / 边缘缩放；点浮动条的 ✓
//! 由 `Document::split_selection_by_grid` 把框内（含边界）音符按网格切开。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{PianoRollView, TimeSigEvent};

use crate::selection::drag::{
    compute_resize_dt, hit_test_sel_edge, main_cross_x_y, main_px_to_tick_dir,
    music_sel_to_pixel_rect,
};

/// 网格工具下的选框拖拽模式（持久化到 egui memory）。
#[derive(Clone, Copy)]
enum GridDrag {
    /// 整体移动选框：按下时的 `(snap_tick, key)`。
    Moving { origin_tick: f64, origin_key: i32 },
    /// 边缘缩放选框：`(side, 被拖动边缘原 tick, 另一边缘原 tick)`。
    Resizing {
        side: ResizeSide,
        origin_boundary: f64,
        other_boundary: f64,
    },
}

/// 网格工具帧处理：press 分发（缩放/移动/新框选）、拖拽更新、release 提交选框。
#[allow(clippy::too_many_arguments)]
pub(crate) fn grid_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut PianoRollView,
    selected: &mut yinhe_core::Selection,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    track_selected: &std::collections::HashSet<u16>,
) {
    let state_id = ui.id().with("grid_drag_state");
    let mut state: Option<GridDrag> = ui.data_mut(|d| d.get_persisted(state_id)).unwrap_or(None);

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return;
    }

    let pointer = ui.input(|i| i.pointer.clone());

    // 上一帧的状态若已无按键（失焦等）：清除。
    if state.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        state = None;
        ui.data_mut(|d| d.insert_persisted(state_id, Option::<GridDrag>::None));
    }

    let eff_rects = sel_rect.effective_rects();
    let clamped_local = |pos: egui::Pos2| -> egui::Pos2 {
        let clamped = pos.clamp(music_rect.min, music_rect.max);
        egui::pos2(
            clamped.x - content_rect.min.x,
            clamped.y - content_rect.min.y,
        )
    };

    // 浮动条（✓ 按钮）上按下：不启动任何拖拽。
    let press_on_bar = pointer
        .hover_pos()
        .is_some_and(|pos| super::drag::on_action_bar(pos, music_rect, view, &eff_rects, 1));

    // ── Press 分发：边缘 → 缩放；框内 → 整体移动；空白 → 清空并交给 marquee ──
    if pointer.primary_pressed()
        && !press_on_bar
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        if let Some((side, origin_boundary, other_boundary)) =
            hit_test_sel_edge(&eff_rects, view, local)
        {
            state = Some(GridDrag::Resizing {
                side,
                origin_boundary,
                other_boundary,
            });
            sel_rect.start_resize(side);
        } else if eff_rects
            .iter()
            .any(|&(t0, t1, kl, kh)| music_sel_to_pixel_rect(view, t0, t1, kl, kh).contains(local))
        {
            let (tick, key) = pointer_music(view, content_rect, quantize, ppq, bar_line_data, pos);
            state = Some(GridDrag::Moving {
                origin_tick: tick,
                origin_key: key,
            });
            sel_rect.start_drag();
        } else {
            selected.clear();
            sel_rect.clear();
        }
        ui.data_mut(|d| d.insert_persisted(state_id, state));
    }

    // ── 拖拽 / 松手：移动或缩放选框 ──
    match state {
        Some(GridDrag::Moving {
            origin_tick,
            origin_key,
        }) => {
            if pointer.primary_down()
                && !pointer.primary_pressed()
                && let Some(pos) = pointer.hover_pos()
            {
                super::drag::drag_scroll_and_clamp(
                    ui,
                    view,
                    content_rect,
                    music_rect,
                    total_ticks,
                    pos,
                );
                let (tick, key) = pointer_music(
                    view,
                    content_rect,
                    quantize,
                    ppq,
                    bar_line_data,
                    pos.clamp(music_rect.min, music_rect.max),
                );
                sel_rect.update_drag((tick - origin_tick).round() as i64, key - origin_key);
            }
            if pointer.primary_released() {
                sel_rect.end_drag();
                ui.data_mut(|d| d.insert_persisted(state_id, Option::<GridDrag>::None));
            }
        }
        Some(GridDrag::Resizing {
            side,
            origin_boundary,
            other_boundary,
        }) => {
            if pointer.primary_down()
                && !pointer.primary_pressed()
                && let Some(pos) = pointer.hover_pos()
            {
                super::drag::drag_scroll_and_clamp(
                    ui,
                    view,
                    content_rect,
                    music_rect,
                    total_ticks,
                    pos,
                );
                let local = clamped_local(pos);
                let (main_px, _) = main_cross_x_y(view, (local.x, local.y));
                let raw_tick = main_px_to_tick_dir(view, main_px);
                let (_, dt) = compute_resize_dt(
                    raw_tick,
                    side,
                    origin_boundary,
                    other_boundary,
                    quantize,
                    ppq,
                    bar_line_data,
                );
                sel_rect.update_resize(dt);
            }
            if pointer.primary_released() {
                sel_rect.end_resize();
                ui.data_mut(|d| d.insert_persisted(state_id, Option::<GridDrag>::None));
            }
        }
        None => {
            // 新框选：共享 marquee 状态机；release 后替换为唯一选框。
            if let Some(result) = super::marquee::marquee_drag_frame(
                ui,
                content_rect,
                music_rect,
                view,
                quantize,
                ppq,
                bar_line_data,
                total_ticks,
                "grid_drag",
                press_on_bar,
            ) {
                selected.clear();
                sel_rect.clear();
                crate::selection::drag::add_pr_selection_rect(
                    selected,
                    result.t_start as u32,
                    result.t_end as u32,
                    result.key_lo,
                    result.key_hi,
                    track_selected,
                    // 网格工具的选择只服务 split（用 sel_rect 几何），不物化成员。
                    None,
                );
                sel_rect.push_rect(
                    (result.t_start, result.t_end, result.key_lo, result.key_hi),
                    false,
                );
            }
        }
    }
}

/// 屏幕位置 → 吸附后的 `(tick, key)`。
fn pointer_music(
    view: &PianoRollView,
    content_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    pos: egui::Pos2,
) -> (f64, i32) {
    let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
    let (main_px, cross_px) = main_cross_x_y(view, (local.x, local.y));
    let raw_tick = main_px_to_tick_dir(view, main_px);
    let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
    (tick, view.cross_px_to_key(cross_px) as i32)
}

/// 在选框范围内画量化网格线（仅时间轴方向），按可见范围裁剪。
///
/// `main_len` = 主轴视口长度（横向 = 音乐区宽，纵向 = 内容高）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_rect_grid(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    view: &PianoRollView,
    t_start: f64,
    t_end: f64,
    key_lo: u8,
    key_hi: u8,
    interval: u32,
    main_len: f32,
) {
    if interval == 0 {
        return;
    }
    // 网格间距不足 1px 时画出来是一片纯色，跳过内部线。
    if (interval as f32) * view.base.pixels_per_tick < 1.0 {
        return;
    }
    let (vis0, vis1) = view.visible_main_range(main_len);
    let lo = t_start.max(vis0).max(0.0);
    let hi = t_end.min(vis1);
    if hi <= lo {
        return;
    }
    // 副轴覆盖范围：横向 key_hi 在上（小 y），纵向 key_lo 在左。
    let (cross0, cross1) = if view.is_vertical() {
        (
            view.key_to_cross_px(key_lo),
            view.key_to_cross_px(key_hi) + view.key_height,
        )
    } else {
        (
            view.key_to_cross_px(key_hi),
            view.key_to_cross_px(key_lo) + view.key_height,
        )
    };
    let stroke = egui::Stroke::new(1.0, crate::theme::accent_active().gamma_multiply(0.55));
    let interval_f = interval as f64;
    let mut t = (lo / interval_f).ceil() * interval_f;
    while t <= hi {
        let main_px = super::drag::tick_to_main_px_dir(view, t);
        let (a, b) = if view.is_vertical() {
            (
                egui::pos2(content_rect.min.x + cross0, content_rect.min.y + main_px),
                egui::pos2(content_rect.min.x + cross1, content_rect.min.y + main_px),
            )
        } else {
            (
                egui::pos2(content_rect.min.x + main_px, content_rect.min.y + cross0),
                egui::pos2(content_rect.min.x + main_px, content_rect.min.y + cross1),
            )
        };
        painter.line_segment([a, b], stroke);
        t += interval_f;
    }
}
