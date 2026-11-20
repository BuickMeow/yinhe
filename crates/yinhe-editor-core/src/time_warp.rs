//! 选框时间渐变（Cubase 式上拉/下拉）：保持两端固定，用幂曲线重排内部音符。
//!
//! 归一化位置 `x = (t - t0) / (t1 - t0)`，映射 `f(x) = x^γ`：
//! - γ < 1：前疏后密（上拉）
//! - γ > 1：前密后疏（下拉）

/// 垂直拖动到该像素数即拉满；再远夹住。
pub const FULL_DRAG_PX: f32 = 120.0;
/// 垂直拖动死区。
pub const FULL_DRAG_DEADZONE_PX: f32 = 4.0;
/// 力度拖动满档的力度范围（1..127 的跨度）。
pub const VELOCITY_FULL_RANGE: f32 = 126.0;

/// 屏幕 dy（向下为正）→ 时间渐变指数。
///
/// 上拉 dy 为负：γ = 2^(dy / 120)，从 1 降到 0.5；
/// 下拉 dy 为正：从 1 升到 2。
pub fn gamma_from_dy(dy: f32) -> f32 {
    let a = (dy / FULL_DRAG_PX).clamp(-1.0, 1.0);
    2.0_f32.powf(a)
}

/// 力度拖动 dy → 力度增量；上拉（dy 负）为正向。
pub fn velocity_delta_from_dy(dy: f32) -> i16 {
    ((-dy / FULL_DRAG_PX) * VELOCITY_FULL_RANGE)
        .round()
        .clamp(-VELOCITY_FULL_RANGE, VELOCITY_FULL_RANGE) as i16
}

/// 把单个 tick 映射到渐变后的时间；两端固定，超出范围夹住。
pub fn warp_tick(t0: f64, t1: f64, gamma: f32, tick: u32) -> u32 {
    if t1 <= t0 || !t0.is_finite() || !t1.is_finite() || !gamma.is_finite() {
        return tick;
    }
    let x = ((f64::from(tick) - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let y = x.powf(f64::from(gamma));
    let out = (t0 + y * (t1 - t0)).round();
    if out <= t0 {
        t0.round().max(0.0) as u32
    } else if out >= t1 {
        t1.round().max(0.0) as u32
    } else {
        out as u32
    }
}

/// 映射一个音符的 `(start, end)`：两端固定、start 不越过 `t1 - 1`、
/// gate 至少 1 tick。核心编辑与 UI ghost 预览共用，避免两套边界规则漂移。
pub fn warp_note(t0: f64, t1: f64, gamma: f32, start: u32, end: u32) -> (u32, u32) {
    if t1 <= t0 || !t0.is_finite() || !t1.is_finite() || !gamma.is_finite() {
        return (start, end);
    }
    let t0u = t0.round().max(0.0) as u32;
    let t1u = t1.round().max(t0u as f64 + 1.0) as u32;
    let mut new_start = warp_tick(t0, t1, gamma, start).clamp(t0u, t1u - 1);
    let new_end = warp_tick(t0, t1, gamma, end).max(new_start + 1).min(t1u);
    if new_end <= new_start {
        new_start = new_end.saturating_sub(1);
    }
    (new_start, new_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamma_directions_and_full_range() {
        assert!((gamma_from_dy(0.0) - 1.0).abs() < 1e-6);
        assert!((gamma_from_dy(-FULL_DRAG_PX) - 0.5).abs() < 1e-6);
        assert!((gamma_from_dy(FULL_DRAG_PX) - 2.0).abs() < 1e-6);
        // 超出 120px 夹住。
        assert!((gamma_from_dy(-500.0) - 0.5).abs() < 1e-6);
        assert!((gamma_from_dy(500.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn endpoints_stay_fixed_and_midpoint_moves() {
        let (t0, t1) = (100.0, 500.0);
        assert_eq!(warp_tick(t0, t1, 0.5, 100), 100);
        assert_eq!(warp_tick(t0, t1, 0.5, 500), 500);
        // 上拉：中点右移；下拉：中点左移。
        let mid = warp_tick(t0, t1, 0.5, 300);
        let mid_down = warp_tick(t0, t1, 2.0, 300);
        assert!(mid > 300, "上拉后中点应右移: {mid}");
        assert!(mid_down < 300, "下拉后中点应左移: {mid_down}");
    }

    #[test]
    fn velocity_delta_directions() {
        assert_eq!(velocity_delta_from_dy(0.0), 0);
        assert_eq!(velocity_delta_from_dy(-10.0), 11); // 上拉变响
        assert_eq!(velocity_delta_from_dy(10.0), -11);
        assert_eq!(velocity_delta_from_dy(-FULL_DRAG_PX), 126);
        assert_eq!(velocity_delta_from_dy(FULL_DRAG_PX), -126);
    }
}
