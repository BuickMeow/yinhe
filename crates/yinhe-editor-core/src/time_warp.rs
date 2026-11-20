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

/// 满档的 log2 指数：2^±6 = 64 / 1/64。
///
/// 幂曲线 `x^γ` 在 γ→0 时让最前面的音符铺满选框、后面趋近 0；γ→∞
/// 时反过来。要真正达到「前面极短、最后一个音符几乎整框」，指数必须
/// 远大于 2；64 能让选框前 10% 的音符在上拉满档时覆盖约 96% 的跨度。
pub const FULL_GAMMA_LOG2: f32 = 6.0;

/// 屏幕 dy（向下为正）→ 时间渐变指数。
///
/// 用二次响应：中心附近 1px 仍然细腻，越接近满档变化越快。
/// `t = dy / 120`，`γ = 2^(6 · t · |t|)`：
/// - 0px → γ=1；
/// - 上拉 120px → γ=1/64（最前面的音符铺满选框，后面极短）；
/// - 下拉 120px → γ=64（前面极短，最后一个音符几乎铺满选框）。
pub fn gamma_from_dy(dy: f32) -> f32 {
    let t = (dy / FULL_DRAG_PX).clamp(-1.0, 1.0);
    let k = FULL_GAMMA_LOG2 * t * t.abs();
    2.0_f32.powf(k)
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
        assert!((gamma_from_dy(-FULL_DRAG_PX) - 1.0 / 64.0).abs() < 1e-6);
        assert!((gamma_from_dy(FULL_DRAG_PX) - 64.0).abs() < 1e-3);
        // 中心附近保持细腻：半程不满档。
        assert!((gamma_from_dy(-FULL_DRAG_PX * 0.5) - 2.0_f32.powf(-1.5)).abs() < 1e-5);
        // 超出 120px 夹住。
        assert!((gamma_from_dy(-500.0) - 1.0 / 64.0).abs() < 1e-6);
        assert!((gamma_from_dy(500.0) - 64.0).abs() < 1e-3);
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
    fn full_drag_collapses_one_side() {
        // 下拉满档：前 9% 的音符被压到几乎 0，最后 10% 的音符铺满 96%+。
        let (front_start, front_end) = warp_note(100.0, 900.0, 64.0, 10, 20);
        assert!(
            (front_start, front_end) == (0, 0) || front_end - front_start <= 1,
            "前面音符应几乎为 0 gate: {front_start}..{front_end}"
        );
        let (last_start, last_end) = warp_note(0.0, 1000.0, 64.0, 900, 1000);
        assert!(
            last_end - last_start >= 960,
            "最后一个音符应几乎铺满选框: {last_start}..{last_end}"
        );

        // 上拉满档：最前面的音符铺满选框，后面趋近 0。
        let (first_start, first_end) = warp_note(0.0, 1000.0, 1.0 / 64.0, 0, 100);
        assert!(
            first_end - first_start >= 960,
            "最前面的音符应几乎铺满选框: {first_start}..{first_end}"
        );
        let (tail_start, tail_end) = warp_note(0.0, 1000.0, 1.0 / 64.0, 900, 1000);
        assert!(
            tail_end - tail_start <= 2,
            "最后面的音符应几乎为 0 gate: {tail_start}..{tail_end}"
        );
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
