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
            let cursor_x = cursor_tick as f32 * pixels_per_tick;
            let right_edge = current_scroll_x + viewport_width;
            let left_edge = current_scroll_x + left_boundary;
            if cursor_x > right_edge {
                Some(current_scroll_x + viewport_width)
            } else if cursor_x < left_edge {
                Some(current_scroll_x - viewport_width)
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
        // 左边界 100（钢琴键盘宽）：光标 50 越过左缘 → 向左翻一页
        let result = compute_follow_scroll(50.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 500.0);
        assert_eq!(result, Some(-300.0));
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
    fn centered_mode_with_nonzero_left_boundary() {
        // 左边界 60，光标 300 在视口中部：不滚动
        let result = compute_follow_scroll(300.0, 1.0, 800.0, 60.0, FollowMode::Centered, 1.0, 0.0);
        assert_eq!(result, None);
    }
}
