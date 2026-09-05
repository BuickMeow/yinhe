use std::time::Instant;

use super::model::Toast;

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

pub(crate) fn fly_anim(toast: &Toast) -> f32 {
    let now = Instant::now();
    if let Some(since) = toast.leaving_since {
        return exit_x(now.duration_since(since).as_secs_f32());
    }
    enter_x(now.duration_since(toast.created).as_secs_f32())
}
