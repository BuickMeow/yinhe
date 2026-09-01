/// Cursor-follow mode for auto-scrolling during playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowMode {
    /// Never auto-scroll — user has full manual control.
    None,
    /// 居中跟随：光标始终保持在视口中央（持续跟随）。
    Centered,
    /// 翻页跟随：光标接近/越过视口边缘时整页翻页，带固定时长动画。
    Page,
    /// 左侧跟随：光标紧贴内容区左缘（持续跟随）。
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
/// - `continuous_inset`: pixels to inset the cursor in Continuous mode (0 for tight left)
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
            // 持续居中：每帧都把光标拉到内容区中央，类似 Continuous 的中央版。
            // 无需边缘判定，避免“从右半区跳到中间”的突兀感。
            let cursor_x = cursor_tick as f32 * pixels_per_tick;
            let content_width = (viewport_width - left_boundary).max(1.0);
            Some(cursor_x - content_width * 0.5)
        }
        FollowMode::Page => {
            // 翻页：提前触发 + 大跳转直达 + 固定时长动画（时长由调用方控制）。
            // - 提前量：视口边缘内 0.06% 或 1px 取大者，避免指示线完全消失才翻。
            // - 落点：光标翻后落在新页左侧 inset 处（0.1% 页宽），几乎不留左空白；
            //   仍保持 inset > margin 以避免贴边振荡（下一帧不在对侧 margin 内）。
            // - 远距离跳转（seek）：若光标远离视口一页以上，直接跳到含光标的页，
            //   保持同样动画时长，避免逐页翻叠加多秒。
            let cursor_x = cursor_tick as f32 * pixels_per_tick;
            let page = (viewport_width - left_boundary).max(1.0);
            // 可见 tick 区间为 [scroll, scroll+page]（page 已扣除 left_boundary 的钢琴/面板宽），
            // cursor_x = tick*ppu 不含 left，避免 left 重复计入（钢琴距离只在 page 中扣一次）。
            let right_edge = current_scroll_x + page;
            let left_edge = current_scroll_x;
            let margin = (page * 0.0006).max(1.0_f32);
            let inset = (page * 0.001).max(2.0_f32);

            let far_forward = cursor_x > right_edge + page;
            let far_backward = cursor_x < left_edge - page;

            if far_forward || far_backward {
                // 远距离：直接以光标为锚点计算目标，保证同样时长一次到位。
                // 前向远跳落左侧 inset，后向远跳落右侧附近（保留上下文）。
                if cursor_x > right_edge {
                    Some((cursor_x - inset).max(0.0))
                } else {
                    // 后向：让光标落在新页右侧附近（距右缘 inset），避免紧贴左缘后立即再触发
                    let target = cursor_x - page + inset;
                    Some(target.max(0.0))
                }
            } else if cursor_x > right_edge - margin {
                // 提前翻页：光标尚未完全越界即翻
                Some((cursor_x - inset).max(0.0))
            } else if cursor_x < left_edge + margin {
                // 向左翻：光标落在新页右侧 inset 处
                let target = cursor_x - page + inset;
                Some(target.max(0.0))
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

/// 跟随平滑时间常数（秒）：指数插值的收敛速度（仅左侧/居中持续跟随使用）。
pub const FOLLOW_TAU: f32 = 0.02;

/// 翻页固定动画时长（秒）：无论距离多少像素，动画时间相同。
pub const FOLLOW_PAGE_DURATION: f32 = 0.30;

/// 帧间插值：向目标滚动位置平滑收敛（帧率无关的指数平滑）。
/// `dt` 为帧间隔（秒，<= 0 时原样返回 current），`tau` 为时间常数（秒）。
/// 用于 Continuous / Centered 的持续跟随（轻微平滑，紧贴）。
pub fn follow_interpolate(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    if dt <= 0.0 {
        return current;
    }
    let k = 1.0 - (-dt / tau).exp();
    current + (target - current) * k
}

/// 翻页固定时长缓动（ease-out cubic），与距离无关。
/// `elapsed` 为已流逝时间，`duration` 为总时长。
pub fn follow_page_lerp(start: f32, target: f32, elapsed: f32, duration: f32) -> f32 {
    if duration <= 1e-6 {
        return target;
    }
    let t = (elapsed / duration).clamp(0.0, 1.0);
    // ease-out cubic: 1 - (1-t)^3
    let e = 1.0 - (1.0 - t).powi(3);
    start + (target - start) * e
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
    fn centered_mode_always_centers() {
        // 居中模式为持续居中：无论光标在何处都返回居中位置
        let result = compute_follow_scroll(400.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 0.0);
        assert_eq!(result, Some(0.0)); // 400 - 400
        let result2 = compute_follow_scroll(700.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 0.0);
        assert_eq!(result2, Some(300.0)); // 700-400
        let result3 =
            compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::Centered, 1.0, 500.0);
        // 中心位置与 current_scroll 无关（持续跟随）
        assert_eq!(result3, Some(-300.0));
    }

    #[test]
    fn page_mode_turns_page_when_cursor_passes_right_edge() {
        // 光标 900 越过右缘 800：翻页到 inset 位置
        let result = compute_follow_scroll(900.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 0.0);
        // page 800, inset 2 => 900-0-2=898
        assert_eq!(result, Some(898.0));
    }

    #[test]
    fn page_mode_turns_back_when_cursor_passes_left_edge() {
        // 已滚动到 800，光标 100 越过左缘附近：向左翻，落在右侧附近
        let result = compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 800.0);
        // 后向：100-0-800+80 = -620 => clamp 0
        assert_eq!(result, Some(0.0));
    }

    #[test]
    fn page_mode_early_trigger() {
        // margin 1px 时仅近边缘 1px 内提前触发
        let result = compute_follow_scroll(799.5, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 0.0);
        assert!(result.is_some(), "应提前翻页");
        // 非提前区域内不触发
        let inside = compute_follow_scroll(400.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 0.0);
        assert_eq!(inside, None);
    }

    #[test]
    fn page_mode_far_jump_direct() {
        // 远距离跳转：光标 5000，视口 800，当前 0，应直接跳到光标附近而非逐页
        let result = compute_follow_scroll(5000.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 0.0);
        // 应该一次到 4998 附近 (5000-2)
        assert_eq!(result, Some(4998.0));
        // 从末尾跳回开头
        let back = compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0, 5000.0);
        assert_eq!(back, Some(0.0));
    }

    #[test]
    fn page_mode_with_nonzero_left_boundary() {
        // 左边界 100，后向翻页
        let result = compute_follow_scroll(50.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 500.0);
        // 50-100-700+70 ≈ -680 =>0
        assert_eq!(result, Some(0.0));
    }

    #[test]
    fn page_mode_forward_turn_does_not_oscillate() {
        // 键盘宽 100、视口 800。光标 805 刚过右缘/提前区，翻后应在新页内不振荡
        let turned = compute_follow_scroll(805.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 0.0);
        assert!(turned.is_some());
        let target = turned.unwrap();
        // 翻页后光标相对新视口位置应在 inset 附近（> margin），不触发回翻
        // 新逻辑 left 不重复计入：offset = cursor - scroll（左缘即 scroll）
        let page: f32 = 700.0;
        let margin = (page * 0.0006).max(1.0_f32);
        let offset = 805.0 - target;
        assert!(
            offset > margin,
            "翻页后光标距新左缘 {offset} 应大于 margin {margin}"
        );
        let next = compute_follow_scroll(805.0, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, target);
        assert_eq!(next, None, "翻页后光标在新页内，不得来回振荡");
    }

    #[test]
    fn page_mode_backward_turn_does_not_oscillate() {
        let turned = compute_follow_scroll(700.5, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, 700.0);
        assert!(turned.is_some());
        let target = turned.unwrap();
        let next = compute_follow_scroll(700.5, 1.0, 800.0, 100.0, FollowMode::Page, 1.0, target);
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
            compute_follow_scroll(100.0, 1.0, 800.0, 60.0, FollowMode::Continuous, 0.0, 0.0);
        assert_eq!(result, Some(100.0));
    }

    #[test]
    fn interpolate_moves_toward_target() {
        let v = follow_interpolate(0.0, 100.0, 1.0 / 60.0, FOLLOW_TAU);
        assert!(v > 0.0 && v < 100.0);
    }

    #[test]
    fn page_lerp_fixed_duration() {
        // 无论距离，0.3s 内到达
        let d1 = follow_page_lerp(0.0, 100.0, 0.15, 0.3);
        let d2 = follow_page_lerp(0.0, 1000.0, 0.15, 0.3);
        // 半程 ease-out cubic ≈ 0.875
        assert!((d1 - 87.5).abs() < 1.0);
        assert!((d2 - 875.0).abs() < 10.0);
        assert_eq!(follow_page_lerp(0.0, 100.0, 0.3, 0.3), 100.0);
        assert_eq!(follow_page_lerp(0.0, 100.0, 0.5, 0.3), 100.0);
    }

    #[test]
    fn centered_mode_with_nonzero_left_boundary() {
        let result = compute_follow_scroll(300.0, 1.0, 800.0, 60.0, FollowMode::Centered, 1.0, 0.0);
        // 300 - (740*0.5)=300-370=-70
        assert_eq!(result, Some(-70.0));
    }
}
