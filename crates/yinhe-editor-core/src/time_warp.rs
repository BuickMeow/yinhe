//! 选框时间渐变（Cubase 式上拉/下拉）：保持两端固定，用指数曲线重排内部音符。
//!
//! 归一化位置 `x = (t - t0) / (t1 - t0)`，映射：
//!
//! ```text
//! f(x) = (e^(k·x) - 1) / (e^k - 1)
//! ```
//!
//! - `k > 0`：前密后疏（下拉）
//! - `k < 0`：前疏后密（上拉）
//!
//! 与幂函数 `x^γ` 相比，指数映射满足 `f_{-k}(1-x) = 1 - f_k(x)`，
//! 所以上拉和下拉在左右方向上是严格镜像的。

/// 垂直拖动到该像素数即拉满；再远夹住。
pub const FULL_DRAG_PX: f32 = 120.0;
/// 垂直拖动死区。
pub const FULL_DRAG_DEADZONE_PX: f32 = 4.0;
/// 力度拖动满档的力度范围（1..127 的跨度）。
pub const VELOCITY_FULL_RANGE: f32 = 126.0;

/// 满档弯曲量：`|k| = 40`。
///
/// `k=40` 时选框最后一个 10% 会展开到约 98%，前面 10% 被压到 0；
/// `k=-40` 完全镜像。需要更极端时调大该常量即可。
pub const FULL_BEND: f32 = 40.0;

/// 屏幕 dy（向下为正）→ 指数曲线弯曲量 `k`。
///
/// 二次响应：中心附近仍然细腻，越接近满档变化越快。
/// `t = dy / 120`，`k = 40 · t · |t|`。
pub fn bend_from_dy(dy: f32) -> f32 {
    let t = (dy / FULL_DRAG_PX).clamp(-1.0, 1.0);
    FULL_BEND * t * t.abs()
}

/// 把归一化位置 `x` 映射到渐变后的位置；两端固定，超出 0..1 夹住。
///
/// 核心编辑、UI ghost 和 tooltip 共用同一公式，避免三处漂移。
pub fn warp_fraction(bend: f32, x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    if !bend.is_finite() {
        return x;
    }
    let k = f64::from(bend);
    if k.abs() < 1e-6 {
        x
    } else {
        ((k * x).exp_m1() / k.exp_m1()).clamp(0.0, 1.0)
    }
}

/// 把单个 tick 映射到渐变后的时间；两端固定，超出范围夹住。
pub fn warp_tick(t0: f64, t1: f64, bend: f32, tick: u32) -> u32 {
    if t1 <= t0 || !t0.is_finite() || !t1.is_finite() || !bend.is_finite() {
        return tick;
    }
    let x = ((f64::from(tick) - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let y = warp_fraction(bend, x);
    let out = (t0 + y * (t1 - t0)).round();
    if out <= t0 {
        t0.round().max(0.0) as u32
    } else if out >= t1 {
        t1.round().max(0.0) as u32
    } else {
        out as u32
    }
}

/// 力度拖动 dy → 力度增量；上拉（dy 负）为正向。
pub fn velocity_delta_from_dy(dy: f32) -> i16 {
    ((-dy / FULL_DRAG_PX) * VELOCITY_FULL_RANGE)
        .round()
        .clamp(-VELOCITY_FULL_RANGE, VELOCITY_FULL_RANGE) as i16
}

/// 映射一个音符的 `(start, end)`：两端固定、start 不越过 `t1 - 1`、
/// gate 至少 1 tick。核心编辑与 UI ghost 预览共用，避免两套边界规则漂移。
pub fn warp_note(t0: f64, t1: f64, bend: f32, start: u32, end: u32) -> (u32, u32) {
    if t1 <= t0 || !t0.is_finite() || !t1.is_finite() || !bend.is_finite() {
        return (start, end);
    }
    let t0u = t0.round().max(0.0) as u32;
    let t1u = t1.round().max(t0u as f64 + 1.0) as u32;
    let mut new_start = warp_tick(t0, t1, bend, start).clamp(t0u, t1u - 1);
    let new_end = warp_tick(t0, t1, bend, end).max(new_start + 1).min(t1u);
    if new_end <= new_start {
        new_start = new_end.saturating_sub(1);
    }
    (new_start, new_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bend_directions_and_full_range() {
        assert!((bend_from_dy(0.0)).abs() < 1e-6);
        assert!((bend_from_dy(-FULL_DRAG_PX) + FULL_BEND).abs() < 1e-4);
        assert!((bend_from_dy(FULL_DRAG_PX) - FULL_BEND).abs() < 1e-4);
        // 二次响应：半程是 1/4 弯曲量。
        assert!((bend_from_dy(-FULL_DRAG_PX * 0.5) + FULL_BEND * 0.25).abs() < 1e-4);
        // 超出 120px 夹住。
        assert!((bend_from_dy(-500.0) + FULL_BEND).abs() < 1e-4);
        assert!((bend_from_dy(500.0) - FULL_BEND).abs() < 1e-4);
    }

    #[test]
    fn endpoints_stay_fixed_and_midpoint_moves() {
        let (t0, t1) = (100.0, 500.0);
        assert_eq!(warp_tick(t0, t1, FULL_BEND, 100), 100);
        assert_eq!(warp_tick(t0, t1, FULL_BEND, 500), 500);
        // 下拉：中点前移；上拉：中点后移。
        let mid_down = warp_tick(t0, t1, FULL_BEND, 300);
        let mid_up = warp_tick(t0, t1, -FULL_BEND, 300);
        assert!(mid_down < 300, "下拉后中点应左移: {mid_down}");
        assert!(mid_up > 300, "上拉后中点应右移: {mid_up}");
    }

    /// 指数曲线必须左右镜像：下拉 k 与上拉 -k 对调两端后位置互补。
    #[test]
    fn curve_is_left_right_symmetric() {
        for x in [0.0, 0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95, 1.0] {
            let down = warp_fraction(FULL_BEND, x);
            let up_mirrored = warp_fraction(-FULL_BEND, 1.0 - x);
            assert!(
                (down + up_mirrored - 1.0).abs() < 1e-9,
                "x={x}: down={down}, up_mirrored={up_mirrored}"
            );
        }
    }

    #[test]
    fn full_drag_collapses_one_side() {
        // 下拉满档：前 10% 的音符 gate 接近 0，最后 10% 的音符铺满 98%+。
        let (front_start, front_end) = warp_note(100.0, 900.0, FULL_BEND, 10, 20);
        assert!(
            (front_start, front_end) == (0, 0) || front_end - front_start <= 1,
            "前面音符应几乎为 0 gate: {front_start}..{front_end}"
        );
        let (last_start, last_end) = warp_note(0.0, 1000.0, FULL_BEND, 900, 1000);
        assert!(
            last_end - last_start >= 980,
            "最后一个音符应几乎铺满选框: {last_start}..{last_end}"
        );

        // 上拉满档：最前面的音符铺满选框，后面趋近 0。
        let (first_start, first_end) = warp_note(0.0, 1000.0, -FULL_BEND, 0, 100);
        assert!(
            first_end - first_start >= 980,
            "最前面的音符应几乎铺满选框: {first_start}..{first_end}"
        );
        let (tail_start, tail_end) = warp_note(0.0, 1000.0, -FULL_BEND, 900, 1000);
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
