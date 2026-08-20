/// Cursor-follow mode for auto-scrolling during playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowMode {
    /// Never auto-scroll — user has full manual control.
    None,
    /// 居中跟随：光标接近视口边缘（20% 余量）时滚动到光标居中。
    Centered,
    /// 真正的翻页跟随：光标越过视口边缘时整页滚动，光标落在页首附近。
    Page,
    /// Cursor stays glued to the leftmost edge of the content area.
    Continuous,
}

impl FollowMode {
    pub fn next(self) -> Self {
        match self {
            FollowMode::None => FollowMode::Centered,
            FollowMode::Centered => FollowMode::Page,
            FollowMode::Page => FollowMode::Continuous,
            FollowMode::Continuous => FollowMode::None,
        }
    }
}

/// Total timeline length in ticks with 64 bars of padding after the last
/// note (or after position 0 if there are no notes).
///
/// Assumes 4/4 time (ticks_per_bar = ppq * 4) for the padding calculation.
pub fn total_ticks_padded(tick_length: u64, ppq: u32) -> f64 {
    (tick_length + 64 * ppq as u64 * 4) as f64
}

/// Apply cursor-follow scrolling during playback.
///
/// Returns the new `scroll_x` if the view should scroll, or `None` if no
/// scroll is needed (mode is `None` or cursor is within the safe margin).
///
/// - `left_boundary`: left content edge in pixels (keyboard_width for piano, 0.0 for arrangement)
/// - `continuous_inset`: pixels to inset the cursor in Continuous mode (1.0 for piano, 0.01 for arrangement)
/// - `current_scroll_x`: 当前视口滚动位置（翻页/居中边界判断的基准）
pub fn compute_follow_scroll(
    cursor_tick: f64,
    pixels_per_tick: f32,
    viewport_width: f32,
    left_boundary: f32,
    follow_mode: FollowMode,
    continuous_inset: f32,
    current_scroll_x: f32,
) -> Option<f32> {
    match follow_mode {
        FollowMode::None => None,
        FollowMode::Centered => {
            let cursor_x = cursor_tick as f32 * pixels_per_tick;
            let content_width = viewport_width - left_boundary;
            let margin = content_width * 0.2;
            let right_edge = current_scroll_x + viewport_width;
            let left_edge = current_scroll_x + left_boundary;
            if cursor_x > right_edge - margin || cursor_x < left_edge + margin {
                Some(cursor_x - content_width * 0.5)
            } else {
                None
            }
        }
        FollowMode::Page => {
            // 真正的翻页：光标越过视口右缘（左缘）时整页滚动，
            // 翻页后光标落在页首（页尾）附近，持续播放会逐页前进。
            // 页宽 = 视口宽 - 左边界（PR 键盘宽）：翻页后光标落在新页左缘右侧，
            // 不会立刻命中向左翻页判定。若按完整视口宽翻页，光标会卡在
            // 新页的键盘遮挡区（cursor_x < 新左缘），下一帧又往回翻，
            // 再下一帧光标仍越右缘又往前翻——每帧来回振荡（翻页抖动）。
            let cursor_x = cursor_tick as f32 * pixels_per_tick;
            let page = (viewport_width - left_boundary).max(1.0);
            let right_edge = current_scroll_x + viewport_width;
            let left_edge = current_scroll_x + left_boundary;
            if cursor_x > right_edge {
                Some(current_scroll_x + page)
            } else if cursor_x < left_edge {
                Some(current_scroll_x - page)
            } else {
                None
            }
        }
        FollowMode::Continuous => {
            let target = cursor_tick as f32 * pixels_per_tick;
            Some(target - continuous_inset)
        }
    }
}

/// 跟随平滑时间常数（秒）：指数插值的收敛速度。越小越跟手（硬）、越大越柔和（滞后）。
pub const FOLLOW_TAU: f32 = 0.1;

/// 帧间插值：向目标滚动位置平滑收敛（帧率无关的指数平滑）。
/// `dt` 为帧间隔（秒，<= 0 时原样返回 current），`tau` 为时间常数（秒）。
/// 每帧调用一次，把居中/翻页触发产生的硬跳变变成可见的减速滑动。
pub fn follow_interpolate(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    if dt <= 0.0 {
        return current;
    }
    let k = 1.0 - (-dt / tau).exp();
    current + (target - current) * k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_mode_returns_none() {
        assert_eq!(
            compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::None, 1.0, 0.0),
            None
        );
    }

    #[test]
    fn continuous_mode_returns_cursor_minus_inset() {
        assert_eq!(
            compute_follow_scroll(100.0, 2.0, 800.0, 0.0, FollowMode::Continuous, 50.0, 0.0),
            Some(150.0)
        );
    }

    #[test]
    fn centered_mode_centers_when_cursor_near_right_edge() {
        // 光标离右缘不足 20% 余量：滚动到光标居中
        let result = compute_follow_scroll(700.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 0.0);
        assert_eq!(result, Some(300.0));
    }

    #[test]
    fn centered_mode_centers_when_cursor_near_left_boundary() {
        let result = compute_follow_scroll(20.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 0.0);
        assert_eq!(result, Some(-380.0));
    }

    #[test]
    fn centered_mode_stays_when_cursor_in_center() {
        assert_eq!(
            compute_follow_scroll(400.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 0.0),
            None
        );
    }

    #[test]
    fn centered_mode_respects_current_scroll() {
        // 已滚动到 500：右缘在 1300，光标 900 距右缘 400（>20% 余量）→ 不滚动
        assert_eq!(
            compute_follow_scroll(900.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 500.0),
            None
        );
        // 光标 1200 距右缘 100（<20% 余量 160）→ 滚动到居中
        assert_eq!(
            compute_follow_scroll(1200.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 500.0),
            Some(800.0)
        );
    }

    #[test]
    fn page_mode_turns_page_when_cursor_passes_right_edge() {
        // 光标 900 越过右缘 800：整页滚动到 800
        let result = compute_follow_scroll(900.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 0.0);
        assert_eq!(result, Some(800.0));
    }

    #[test]
    fn page_mode_turns_back_when_cursor_passes_left_edge() {
        // 已滚动到 800，光标 100 越过左缘 0：向左翻一页
        let result = compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 800.0);
        assert_eq!(result, Some(0.0));
    }

    #[test]
    fn page_mode_stays_while_cursor_inside_viewport() {
        // 光标在视口内：不滚动
        assert_eq!(
            compute_follow_scroll(700.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 0.0),
            None
        );
        // 光标在视口内但接近右缘（居中模式会滚，翻页模式不滚）
        assert_eq!(
            compute_follow_scroll(750.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 0.0),
            None
        );
    }

    #[test]
    fn page_mode_with_nonzero_left_boundary() {
        // 左边界 100（钢琴键盘宽）：光标 50 越过左缘 → 向左翻一页（页宽 700）
        let result = compute_follow_scroll(50.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 500.0);
        assert_eq!(result, Some(-200.0));
    }

    #[test]
    fn page_mode_forward_turn_does_not_oscillate() {
        // 回归：键盘宽 100、视口 800。光标刚过右缘（805）→ 向前翻一页，
        // 页宽 700 → scroll = 700。新页左缘 = 700 + 100 = 800，光标 805 在其右侧，
        // 下一帧不得再触发向左翻页（旧实现按整视口宽翻页，scroll = 800，
        // 光标 805 < 新左缘 900 → 每帧来回翻页 = 翻页抖动）。
        let turned = compute_follow_scroll(805.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 0.0);
        assert_eq!(turned, Some(700.0));
        let next = compute_follow_scroll(805.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 700.0);
        assert_eq!(next, None, "翻页后光标在新页内，不得来回振荡");
    }

    #[test]
    fn page_mode_backward_turn_does_not_oscillate() {
        // 回归：反向同理。scroll = 700、光标 795 越过左缘 800 → 向左翻一页（页宽 700）
        // → scroll = 0。新右缘 = 800 > 光标 795，下一帧不得再触发向前翻页。
        let turned = compute_follow_scroll(795.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 700.0);
        assert_eq!(turned, Some(0.0));
        let next = compute_follow_scroll(795.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 0.0);
        assert_eq!(next, None, "回翻后光标在新页内，不得来回振荡");
    }

    #[test]
    fn follow_mode_next_cycles() {
        assert_eq!(FollowMode::None.next(), FollowMode::Centered);
        assert_eq!(FollowMode::Centered.next(), FollowMode::Page);
        assert_eq!(FollowMode::Page.next(), FollowMode::Continuous);
        assert_eq!(FollowMode::Continuous.next(), FollowMode::None);
    }

    #[test]
    fn total_ticks_padded_positive() {
        let ppq = 480;
        let bars = 64 * ppq as u64 * 4;
        assert_eq!(total_ticks_padded(1000, ppq), (1000 + bars) as f64);
        assert_eq!(total_ticks_padded(480, ppq), (480 + bars) as f64);
    }

    #[test]
    fn total_ticks_padded_zero() {
        let ppq = 480;
        let bars = 64 * ppq as u64 * 4;
        assert_eq!(total_ticks_padded(0, ppq), bars as f64);
    }

    #[test]
    fn continuous_mode_with_left_boundary() {
        let result =
            compute_follow_scroll(100.0, 1.0, 800.0, 60.0, FollowMode::Continuous, 60.0, 0.0);
        assert_eq!(result, Some(40.0));
    }

    #[test]
    fn interpolate_moves_toward_target() {
        let v = follow_interpolate(0.0, 100.0, 1.0 / 60.0, FOLLOW_TAU);
        assert!(v > 0.0 && v < 100.0);
    }

    #[test]
    fn interpolate_reaches_target_over_time() {
        // 指数平滑的收敛性：以 60fps 插值 100 帧（约 1.7s）应基本到达。
        let mut v = 0.0;
        for _ in 0..100 {
            v = follow_interpolate(v, 100.0, 1.0 / 60.0, FOLLOW_TAU);
        }
        assert!((100.0 - v).abs() < 0.01);
    }

    #[test]
    fn interpolate_is_framerate_independent() {
        // 相同累计时间下，30fps 与 60fps 的收敛程度应一致（帧率无关）。
        let mut a = 0.0;
        let mut b = 0.0;
        for _ in 0..60 {
            a = follow_interpolate(a, 100.0, 1.0 / 60.0, FOLLOW_TAU);
        }
        for _ in 0..30 {
            b = follow_interpolate(b, 100.0, 1.0 / 30.0, FOLLOW_TAU);
        }
        assert!((a - b).abs() < 0.01);
    }

    #[test]
    fn centered_mode_with_nonzero_left_boundary() {
        // 左边界 60，光标 300 在视口中部：不滚动
        let result = compute_follow_scroll(300.0, 1.0, 800.0, 60.0, FollowMode::Centered, 1.0, 0.0);
        assert_eq!(result, None);
    }
}
