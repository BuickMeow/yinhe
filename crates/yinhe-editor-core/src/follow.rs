/// Cursor-follow mode for auto-scrolling during playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowMode {
    /// Never auto-scroll — user has full manual control.
    None,
    /// Scroll when cursor reaches the viewport edge (current behavior).
    Page,
    /// Cursor stays glued to the leftmost edge of the content area.
    Continuous,
}

impl FollowMode {
    pub fn next(self) -> Self {
        match self {
            FollowMode::None => FollowMode::Page,
            FollowMode::Page => FollowMode::Continuous,
            FollowMode::Continuous => FollowMode::None,
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            FollowMode::None => "不滚动",
            FollowMode::Page => "翻页跟随",
            FollowMode::Continuous => "实时跟随",
        }
    }
}

/// Total timeline length in ticks with 20% padding, or a sensible default
/// when the source has no notes.
pub fn total_ticks_padded(tick_length: u64) -> f64 {
    if tick_length > 0 {
        tick_length as f64 * 1.2
    } else {
        0.0
    }
}

/// Apply cursor-follow scrolling during playback.
///
/// Returns the new `scroll_x` if the view should scroll, or `None` if no
/// scroll is needed (mode is `None` or cursor is within the safe margin).
///
/// - `left_boundary`: left content edge in pixels (keyboard_width for piano, 0.0 for arrangement)
/// - `continuous_inset`: pixels to inset the cursor in Continuous mode (1.0 for piano, 0.01 for arrangement)
pub fn compute_follow_scroll(
    cursor_tick: f64,
    pixels_per_tick: f32,
    viewport_width: f32,
    left_boundary: f32,
    follow_mode: FollowMode,
    continuous_inset: f32,
) -> Option<f32> {
    match follow_mode {
        FollowMode::None => None,
        FollowMode::Page => {
            let cursor_x = cursor_tick as f32 * pixels_per_tick;
            let content_width = viewport_width - left_boundary;
            let margin = content_width * 0.2;
            if cursor_x > viewport_width - margin || cursor_x < left_boundary + margin {
                Some((cursor_tick as f32 * pixels_per_tick) - content_width * 0.5)
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
            compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::None, 1.0),
            None
        );
    }

    #[test]
    fn continuous_mode_returns_cursor_minus_inset() {
        assert_eq!(
            compute_follow_scroll(100.0, 2.0, 800.0, 0.0, FollowMode::Continuous, 50.0),
            Some(150.0)
        );
    }

    #[test]
    fn page_mode_scrolls_when_cursor_near_right_edge() {
        let result = compute_follow_scroll(900.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0);
        assert!(result.is_some());
    }

    #[test]
    fn page_mode_scrolls_when_cursor_near_left_boundary() {
        let result = compute_follow_scroll(20.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0);
        assert!(result.is_some());
    }

    #[test]
    fn page_mode_stays_when_cursor_in_center() {
        assert_eq!(
            compute_follow_scroll(400.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0),
            None
        );
    }

    #[test]
    fn page_mode_with_nonzero_left_boundary() {
        let result = compute_follow_scroll(50.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0);
        assert!(result.is_some());
    }

    #[test]
    fn follow_mode_next_cycles() {
        assert_eq!(FollowMode::None.next(), FollowMode::Page);
        assert_eq!(FollowMode::Page.next(), FollowMode::Continuous);
        assert_eq!(FollowMode::Continuous.next(), FollowMode::None);
    }

    #[test]
    fn follow_mode_tooltip_not_empty() {
        assert!(!FollowMode::None.tooltip().is_empty());
        assert!(!FollowMode::Page.tooltip().is_empty());
        assert!(!FollowMode::Continuous.tooltip().is_empty());
    }

    #[test]
    fn total_ticks_padded_positive() {
        assert!((total_ticks_padded(1000) - 1200.0).abs() < 0.01);
        assert!((total_ticks_padded(480) - 576.0).abs() < 0.01);
    }

    #[test]
    fn total_ticks_padded_zero() {
        assert_eq!(total_ticks_padded(0), 0.0);
    }

    #[test]
    fn page_mode_scroll_target() {
        let result = compute_follow_scroll(900.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0);
        assert_eq!(result, Some(500.0));
    }

    #[test]
    fn continuous_mode_with_left_boundary() {
        let result = compute_follow_scroll(100.0, 1.0, 800.0, 60.0, FollowMode::Continuous, 60.0);
        assert_eq!(result, Some(40.0));
    }

    #[test]
    fn page_mode_no_scroll_when_cursor_in_margin() {
        let result = compute_follow_scroll(300.0, 1.0, 800.0, 60.0, FollowMode::Page, 1.0);
        assert_eq!(result, None);
    }
}
