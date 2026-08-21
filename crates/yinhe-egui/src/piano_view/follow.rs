//! 自动跟随（播放时按 FollowMode 滚动）。

use eframe::egui;

use yinhe_types::PianoRollView;

use crate::view_interaction::FollowMode;

/// 更新跟随滚动（仅播放时生效）。
///
/// - 非播放或 `FollowMode::None` 时清空 `follow_target`
/// - 否则根据 `cursor_tick` 计算目标滚动位置并向目标指数插值
/// - 到达目标或被 clamp 卡住时结束插值
pub(crate) fn update_follow(
    view: &mut PianoRollView,
    cursor_tick: Option<f64>,
    is_playing: bool,
    follow_mode: &FollowMode,
    ui: &egui::Ui,
    layout: &super::layout::Layout,
) {
    let follow_active = is_playing && *follow_mode != FollowMode::None;
    if !follow_active {
        view.base.follow_target = None;
        return;
    }
    let Some(ct) = cursor_tick else {
        return;
    };
    let dt = ui.input(|i| i.stable_dt).max(1e-4);
    // 沿主轴跟随：横向滚动目标 = scroll_x（视口宽 w）；纵向 = scroll_y（视口高 h，
    // 时间轴起点在顶部、无键盘列偏移）。compute_follow_scroll 数学单轴通用。
    let (main_len, left_boundary, cur_main) = if view.is_vertical() {
        (layout.h as f32, 0.0, view.base.scroll_y)
    } else {
        (layout.w as f32, view.keyboard_width(), view.base.scroll_x)
    };
    if let Some(t) = crate::view_interaction::compute_follow_scroll(
        ct,
        view.base.pixels_per_tick,
        main_len,
        left_boundary,
        *follow_mode,
        1.0,
        cur_main,
    ) {
        view.base.follow_target = Some(t);
    }
    if let Some(t) = view.base.follow_target {
        let before = *view.main_scroll();
        *view.main_scroll() = crate::view_interaction::follow_interpolate(
            before,
            t,
            dt,
            crate::view_interaction::FOLLOW_TAU,
        );
        view.clamp_scroll(layout.w as f32, layout.h as f32, layout.total_ticks);
        // 已到达目标（1px 数值容差）或滚动被 clamp 卡在边界：结束插值。
        if (t - *view.main_scroll()).abs() <= 1.0 || *view.main_scroll() == before {
            view.base.follow_target = None;
        }
    }
}
