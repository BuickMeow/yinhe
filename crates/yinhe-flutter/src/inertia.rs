//! 视口惯性滚动（Flutter 手势结束后由 Dart 启动）。
//!
//! 手势识别、slop、焦点和缩放倍率全部在 Flutter；这里只负责把松手速度
//! 变成指数衰减的视口位移。这样 PR/AR 仍共用同一套视口 clamp 和渲染状态。

use std::sync::Mutex;
use std::time::Instant;

use crate::scene::{self, ViewMode};

struct Inertia {
    vx: f32,
    vy: f32,
    at: Option<Instant>,
}

static INERTIA: Mutex<Inertia> = Mutex::new(Inertia {
    vx: 0.0,
    vy: 0.0,
    at: None,
});

/// 启动惯性所需的最小速度（逻辑像素/秒），避免慢速拖拽松手后漂移。
const FLING_MIN_SPEED: f32 = 250.0;
/// 停止阈值（低于此速度直接归零）。
const FLING_STOP_SPEED: f32 = 20.0;
/// 惯性衰减系数（每秒）：约 0.25s 衰减到 1/e。
const FLING_DECAY: f32 = 4.0;

/// 停止惯性滚动。
pub fn stop() {
    if let Ok(mut st) = INERTIA.lock() {
        st.vx = 0.0;
        st.vy = 0.0;
        st.at = None;
    }
}

/// 以给定速度启动惯性（逻辑像素/秒）。速度过小则忽略。
pub fn start(vx: f32, vy: f32) {
    let speed = (vx * vx + vy * vy).sqrt();
    if !speed.is_finite() || speed < FLING_MIN_SPEED {
        stop();
        return;
    }
    if let Ok(mut st) = INERTIA.lock() {
        st.vx = vx;
        st.vy = vy;
        st.at = Some(Instant::now());
    }
}

/// 每帧惯性滚动（Flutter 帧驱动调用）。
///
/// 返回 1 = 本帧有滚动；0 = 无惯性。
pub fn tick() -> i32 {
    let Ok(mut st) = INERTIA.lock() else {
        return 0;
    };
    let Some(at) = st.at else {
        return 0;
    };
    let now = Instant::now();
    let dt = now.duration_since(at).as_secs_f32().clamp(0.001, 0.05);
    st.at = Some(now);
    let decay = (-FLING_DECAY * dt).exp();
    st.vx *= decay;
    st.vy *= decay;
    let speed = (st.vx * st.vx + st.vy * st.vy).sqrt();
    if speed < FLING_STOP_SPEED {
        st.vx = 0.0;
        st.vy = 0.0;
        st.at = None;
        return 0;
    }
    let (dx, dy) = (st.vx * dt, st.vy * dt);

    let Ok(mut s) = scene::scene().lock() else {
        return 0;
    };
    let (lw, lh) = crate::logical_size();
    let total_ticks = s.total_ticks;
    let num_tracks = s.num_tracks;
    match s.view_mode {
        ViewMode::Pianoroll => {
            *s.pr_view.main_scroll() -= dx;
            *s.pr_view.cross_scroll() -= dy;
            s.pr_view.clamp_scroll(lw, lh, total_ticks);
        }
        ViewMode::Arrangement => {
            s.arr_view.base.scroll_x -= dx;
            s.arr_view.base.scroll_y -= dy;
            s.arr_view.clamp_scroll(lw, lh, total_ticks, num_tracks + 1);
        }
    }
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    1
}
