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
        view.base.follow_anim_elapsed = 0.0;
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

    // 持续跟随（居中 / 左侧）直接贴合，无固定时长动画
    if *follow_mode == FollowMode::Centered || *follow_mode == FollowMode::Continuous {
        // 左侧跟随紧贴左缘（inset 0），居中跟随由 compute 内部处理
        let inset = 0.0;
        if let Some(t) = crate::view_interaction::compute_follow_scroll(
            ct,
            view.base.pixels_per_tick,
            main_len,
            left_boundary,
            *follow_mode,
            inset,
            cur_main,
        ) {
            // 直接赋值确保紧贴，clamp 保证不越界
            *view.main_scroll() = t;
            view.clamp_scroll(layout.w as f32, layout.h as f32, layout.total_ticks);
            // 保持 follow_target 为 None，避免与 Page 动画状态混淆
            view.base.follow_target = None;
            view.base.follow_anim_elapsed = 0.0;
        }
        return;
    }

    // Page 翻页：固定时长缓动
    if let Some(t) = crate::view_interaction::compute_follow_scroll(
        ct,
        view.base.pixels_per_tick,
        main_len,
        left_boundary,
        *follow_mode,
        0.0,
        cur_main,
    ) {
        // 新目标与旧目标不同时重启动画（处理大跳转）
        let need_restart = view.base.follow_target != Some(t);
        if need_restart {
            view.base.follow_anim_start = *view.main_scroll();
            view.base.follow_anim_elapsed = 0.0;
            view.base.follow_target = Some(t);
        }
    }
    if let Some(target) = view.base.follow_target {
        view.base.follow_anim_elapsed += dt;
        let new_scroll = crate::view_interaction::follow_page_lerp(
            view.base.follow_anim_start,
            target,
            view.base.follow_anim_elapsed,
            crate::view_interaction::FOLLOW_PAGE_DURATION,
        );
        *view.main_scroll() = new_scroll;
        view.clamp_scroll(layout.w as f32, layout.h as f32, layout.total_ticks);
        let done = view.base.follow_anim_elapsed >= crate::view_interaction::FOLLOW_PAGE_DURATION
            || (target - *view.main_scroll()).abs() <= 1.0;
        if done {
            *view.main_scroll() = target;
            view.clamp_scroll(layout.w as f32, layout.h as f32, layout.total_ticks);
            view.base.follow_target = None;
            view.base.follow_anim_elapsed = 0.0;
        }
    }
}
