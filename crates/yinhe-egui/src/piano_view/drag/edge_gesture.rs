//! 选框边缘手势：左右缘纵向拖动做时间渐变，下缘纵向拖动批量调力度。
//!
//! 边缘同时承担原有的时间轴方向拖动（Gate/Scale/Riff），因此第一次超过
//! 死区后按主轴/副轴锁定本次手势，不再中途切换：
//! - 时间终点边 + 主轴 → 原有 Gate/Scale；
//! - 时间边 + 副轴 → `x^γ` 时间渐变；
//! - 力度边 + 副轴 → 相对力度增量。
//!
//! Riff 模式在 press 时直接进入原有状态机，不参与轴锁定：Riff 的纵向
//! 拖动是平铺功能，必须保持两轴都可用的老行为。
//!
//! 其他组合（时间起点边的主轴拖动、力度边的主轴拖动）不做事。

use eframe::egui;

use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::time_warp::{
    FULL_DRAG_DEADZONE_PX, gamma_from_dy, velocity_delta_from_dy, warp_note,
};

use super::state::{EdgeGestureState, GestureAxis, SelDragFrameState};
use crate::selection::drag::SelectionEdge;

/// 屏幕坐标增量 → (沿时间轴, 垂直时间轴)。
///
/// 横向 PR：主轴 = X、副轴 = Y；纵向 PR：主轴 = Y、副轴 = X。
fn axis_deltas(delta: egui::Vec2, vertical: bool) -> (f32, f32) {
    if vertical {
        (delta.y, delta.x)
    } else {
        (delta.x, delta.y)
    }
}

/// 第一次超过死区后锁定手势方向。
fn lock_axis(delta: egui::Vec2, vertical: bool) -> Option<GestureAxis> {
    if delta.x.abs().max(delta.y.abs()) <= FULL_DRAG_DEADZONE_PX {
        return None;
    }
    let (main, cross) = axis_deltas(delta, vertical);
    // 平局归主轴：宁可保留 Gate/Scale，也不要横向拖时误触渐变。
    Some(if main.abs() >= cross.abs() {
        GestureAxis::Main
    } else {
        GestureAxis::Cross
    })
}

/// 时间终点边 + 主轴拖动：切回原有的 Gate/Scale 状态机。
///
/// Riff 在 press 时已直接进入 `sel_riff_state`，这里保留分支只作防御。
fn activate_main_mode(
    state: &mut SelDragFrameState,
    gesture: EdgeGestureState,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
) {
    use crate::widgets::selection_actions::SelectionDragMode;
    match gesture.mode {
        SelectionDragMode::Gate => {
            state.sel_resize_state = Some((ResizeSide::Right, gesture.t1, gesture.t0));
            sel_rect.start_resize(ResizeSide::Right);
        }
        SelectionDragMode::Scale => {
            state.sel_scale_state = Some((gesture.t0, gesture.t1));
            sel_rect.start_resize(ResizeSide::Right);
        }
        SelectionDragMode::Riff => {
            state.sel_riff_state = Some((gesture.t0, gesture.t1, gesture.k0, gesture.k1));
        }
    }
    state.edge_gesture = None;
}

/// 时间渐变 ghost：两端固定，内部音符按 `x^γ` 重排。
fn rebuild_warp_ghost(
    state: &mut SelDragFrameState,
    gesture: EdgeGestureState,
    view: &yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    gamma: f32,
) {
    state.ghost_notes.clear();
    state.hidden_notes.clear();
    let Some(notes) = state.drag_notes.as_ref() else {
        return;
    };
    let (tick_lo, tick_hi) = view.visible_tick_range(music_rect.width());
    let (key_lo, key_hi) =
        view.visible_cross_range(view.cross_axis_len(content_rect.width(), content_rect.height()));
    for info in notes.iter() {
        if info.key < key_lo || info.key > key_hi {
            continue;
        }
        let (new_start, new_end) = warp_note(
            gesture.t0,
            gesture.t1,
            gamma,
            info.start_tick,
            info.end_tick,
        );
        if f64::from(new_start) >= tick_hi || f64::from(new_end) <= tick_lo {
            continue;
        }
        state
            .ghost_notes
            .push((new_start, new_end, info.key, info.track));
        if f64::from(info.start_tick) < tick_hi && f64::from(info.end_tick) > tick_lo {
            state
                .hidden_notes
                .push((info.track, info.start_tick, info.key));
        }
    }
}

/// 边缘手势每帧推进；松手时通过 `selection_warp` / `selection_velocity_delta`
/// 提交一次操作。
#[allow(clippy::too_many_arguments)]
pub(crate) fn edge_gesture_frame(
    ui: &mut egui::Ui,
    state: &mut SelDragFrameState,
    view: &yinhe_types::PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    selection_warp: &mut Option<(f64, f64, f32)>,
    selection_velocity_delta: &mut Option<i16>,
    pointer: &egui::PointerState,
) {
    let Some(mut gesture) = state.edge_gesture else {
        return;
    };
    let vertical = view.is_vertical();

    if pointer.primary_down()
        && !pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
    {
        let delta = pos - gesture.press;
        if gesture.lock.is_none() {
            let Some(axis) = lock_axis(delta, vertical) else {
                // 死区内：不动作，也不显示 tooltip。
                return;
            };
            gesture.lock = Some(axis);
            state.edge_gesture = Some(gesture);
            if axis == GestureAxis::Main {
                if gesture.edge == SelectionEdge::TimeEnd {
                    activate_main_mode(state, gesture, sel_rect);
                } else {
                    state.edge_gesture = None;
                    state.drag_notes = None;
                }
                return;
            }
        }

        if gesture.lock == Some(GestureAxis::Cross) {
            let (_, cross) = axis_deltas(delta, vertical);
            match gesture.edge {
                SelectionEdge::TimeStart | SelectionEdge::TimeEnd => {
                    let gamma = gamma_from_dy(cross);
                    rebuild_warp_ghost(state, gesture, view, content_rect, music_rect, gamma);
                    // 中点归一化位置 = 0.5^γ；直接告诉用户疏密偏向。
                    let mid = (0.5_f32.powf(gamma) * 100.0).round() as i32;
                    crate::widgets::tooltip::draw_hover_tooltip(
                        ui.ctx(),
                        &[format!("slope ×{gamma:.2}"), format!("mid {mid}%")],
                        pos.x,
                        pos.y,
                    );
                }
                SelectionEdge::Velocity => {
                    let delta_v = velocity_delta_from_dy(cross);
                    crate::widgets::tooltip::draw_hover_tooltip(
                        ui.ctx(),
                        &[format!("vel {delta_v:+}")],
                        pos.x,
                        pos.y,
                    );
                }
            }
            ui.ctx().request_repaint();
        }
        return;
    }

    if pointer.primary_released() {
        let released = pointer.hover_pos();
        // 极快的手势可能没有中间的 drag 帧：释放帧补一次锁定判定。
        if gesture.lock.is_none()
            && let Some(pos) = released
        {
            gesture.lock = lock_axis(pos - gesture.press, vertical);
        }
        match gesture.lock {
            Some(GestureAxis::Cross) => {
                if let Some(pos) = released {
                    let (_, cross) = axis_deltas(pos - gesture.press, vertical);
                    match gesture.edge {
                        SelectionEdge::TimeStart | SelectionEdge::TimeEnd => {
                            let gamma = gamma_from_dy(cross);
                            if (gamma - 1.0).abs() > 1e-4 {
                                *selection_warp = Some((gesture.t0, gesture.t1, gamma));
                            }
                        }
                        SelectionEdge::Velocity => {
                            let delta_v = velocity_delta_from_dy(cross);
                            if delta_v != 0 {
                                *selection_velocity_delta = Some(delta_v);
                            }
                        }
                    }
                }
                state.edge_gesture = None;
                state.drag_notes = None;
            }
            Some(GestureAxis::Main) => {
                if gesture.edge == SelectionEdge::TimeEnd {
                    activate_main_mode(state, gesture, sel_rect);
                } else {
                    state.edge_gesture = None;
                    state.drag_notes = None;
                }
            }
            None => {
                // 死区内松手 = 单击边缘，不产生编辑。
                state.edge_gesture = None;
                state.drag_notes = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_lock_uses_time_axis_not_screen_axis() {
        // 横向 PR：X 是主轴，Y 是副轴。
        assert_eq!(
            lock_axis(egui::vec2(10.0, 2.0), false),
            Some(GestureAxis::Main)
        );
        assert_eq!(
            lock_axis(egui::vec2(2.0, 10.0), false),
            Some(GestureAxis::Cross)
        );
        // 纵向 PR：Y 是主轴，X 是副轴。
        assert_eq!(
            lock_axis(egui::vec2(2.0, 10.0), true),
            Some(GestureAxis::Main)
        );
        assert_eq!(
            lock_axis(egui::vec2(10.0, 2.0), true),
            Some(GestureAxis::Cross)
        );
    }

    #[test]
    fn axis_lock_respects_deadzone_and_keeps_ties_on_main() {
        assert_eq!(
            lock_axis(egui::vec2(2.0, 3.0), false),
            None,
            "4px 死区内不锁轴"
        );
        // 45° 平局给主轴，避免横向 Gate/Scale 时误触渐变。
        assert_eq!(
            lock_axis(egui::vec2(10.0, 10.0), false),
            Some(GestureAxis::Main)
        );
    }
}
