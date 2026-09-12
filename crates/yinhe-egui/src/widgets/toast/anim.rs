use std::time::Instant;

use super::model::Notification;

// 纯位移飞行动画：右入左出，无停顿、无透明度。
// 起步 x=DIST（卡片左侧贴窗口右沿之外，阴影也完全在屏外），终点 x=0。
pub(crate) const FLY_DIST: f32 = 420.0;
const FLY_DUR: f32 = 0.32;

/// 入场：DIST → 0，ease-out（快起慢收），无 stagger，创建即走。
pub(crate) fn enter_x(elapsed_secs: f32) -> f32 {
    if elapsed_secs <= 0.0 {
        return FLY_DIST;
    }
    let t = (elapsed_secs / FLY_DUR).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - t).powi(3);
    (1.0 - e) * FLY_DIST
}

/// 退场：入场的严格时间反向（0 → DIST），即 enter(DUR-t)。
/// enter(s)=(1-s)^3*DIST，故 exit(t)=t^3*DIST，时长一致。
pub(crate) fn exit_x(elapsed_secs: f32) -> f32 {
    let t = (elapsed_secs / FLY_DUR).clamp(0.0, 1.0);
    t.powi(3) * FLY_DIST
}

pub(crate) fn fly_anim(toast: &Notification) -> f32 {
    let now = Instant::now();
    if let Some(since) = toast.leaving_since {
        return exit_x(now.duration_since(since).as_secs_f32());
    }
    enter_x(now.duration_since(toast.created).as_secs_f32())
}

// ── Y 轴 ease-out 动画（与 anim.rs 里 x 飞行动画同族曲线 1-(1-t)^3，时长 0.35s）──
#[derive(Clone, Copy, Debug)]
pub(super) struct YAnim {
    pub(super) from: f32,
    pub(super) to: f32,
    pub(super) t0: Instant,
}

pub(super) const Y_ANIM_DUR_SECS: f32 = 0.35;

pub(super) fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub(super) fn y_anim_value(anim: &YAnim, now: Instant) -> f32 {
    let elapsed = now.saturating_duration_since(anim.t0).as_secs_f32();
    let t = (elapsed / Y_ANIM_DUR_SECS).clamp(0.0, 1.0);
    anim.from + (anim.to - anim.from) * ease_out_cubic(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_cubic_values_and_monotonic() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < 1e-6);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-6);
        // t=0.5 时 1-(0.5)^3=0.875，快起慢收
        assert!((ease_out_cubic(0.5) - 0.875).abs() < 1e-6);
        // 单调递增
        let mut prev = ease_out_cubic(0.0);
        let mut t: f32 = 0.1;
        while t <= 1.0001 {
            let cur = ease_out_cubic(t.min(1.0));
            assert!(cur >= prev, "not monotonic at t={t}");
            prev = cur;
            t += 0.1;
        }
    }
}
