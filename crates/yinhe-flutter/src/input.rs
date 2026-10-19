//! 触摸手势 → 视口更新（PR/AR 双视图）。
//!
//! - 单指拖动：平移（抓手语义，内容跟随手指）。
//! - 双指捏合：
//!   - PR：内容区缩放时间轴（`zoom_main_around`），键盘列缩放音高轴；
//!   - AR：内容区缩放时间轴（`zoom_around_x`），轨道面板区缩放行高档位。
//! - 双指中点移动即平移。
//!
//! 坐标为画布逻辑像素（全宽坐标系）。clamp 由渲染线程每帧统一收口，这里只写原始位移。

use std::sync::Mutex;
use std::time::Instant;

use crate::scene::{self, RenderScene, ViewMode};

/// 活跃触点。
#[derive(Clone, Copy)]
struct Pointer {
    id: i32,
    x: f32,
    y: f32,
}

/// 上次采样的基准（单指=位置；双指=距离+中点）。
#[derive(Clone, Copy)]
struct Baseline {
    dist: f32,
    mid: (f32, f32),
}

struct GestureState {
    pointers: Vec<Pointer>,
    baseline: Option<Baseline>,
    /// 平移速度（逻辑像素/秒，指数平滑），松手时用于惯性。
    vel: (f32, f32),
    /// 上一次移动采样时间。
    last_move: Option<Instant>,
}

static GESTURE: Mutex<GestureState> = Mutex::new(GestureState {
    pointers: Vec::new(),
    baseline: None,
    vel: (0.0, 0.0),
    last_move: None,
});

/// 松手惯性状态（逻辑像素/秒；每帧指数衰减滚动视图）。
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

/// 惯性触发阈值（逻辑像素/秒）。
const FLING_MIN_SPEED: f32 = 250.0;
/// 停止阈值（低于此速度直接归零）。
const FLING_STOP_SPEED: f32 = 20.0;
/// 惯性衰减系数（每秒）：约 0.25s 衰减到 1/e。
const FLING_DECAY: f32 = 4.0;

/// 停止惯性滚动（重新按下/取消时调用）。
pub fn stop_inertia() {
    if let Ok(mut st) = INERTIA.lock() {
        st.vx = 0.0;
        st.vy = 0.0;
        st.at = None;
    }
}

/// 触摸/鼠标事件：0=down 1=move 2=up/cancel，3/4/5=鼠标右键按下/移动/松开。
///
/// 右键走独立的强制平移通道：不触发当前工具，取消进行中的编辑拖拽，
/// 语义等同于临时切换到手形工具。
pub fn on_touch(kind: i32, x: f32, y: f32, pointer: i32) {
    let Ok(mut g) = GESTURE.lock() else { return };
    match kind {
        3 => {
            // 右键按下：取消编辑拖拽后按单指平移处理。
            stop_inertia();
            g.pointers.retain(|p| p.id != pointer);
            g.pointers.push(Pointer { id: pointer, x, y });
            g.vel = (0.0, 0.0);
            g.last_move = Some(Instant::now());
            crate::edit::cancel();
            reset_baseline(&mut g);
        }
        4 => {
            let Some(p) = g.pointers.iter_mut().find(|p| p.id == pointer) else {
                return;
            };
            p.x = x;
            p.y = y;
            apply(&mut g);
        }
        5 => {
            // 右键松开：只移除触点，不提交编辑。
            g.pointers.retain(|p| p.id != pointer);
            reset_baseline(&mut g);
        }
        0 => {
            // 同一 pointer id 重复 down：以新位置为准。
            stop_inertia();
            g.pointers.retain(|p| p.id != pointer);
            g.pointers.push(Pointer { id: pointer, x, y });
            g.vel = (0.0, 0.0);
            g.last_move = Some(Instant::now());
            if g.pointers.len() == 1 {
                // 单指：按当前工具尝试开始编辑拖拽（手形/未命中则退回平移）。
                crate::edit::begin(x, y);
            } else {
                // 多指：取消编辑拖拽，进入缩放/平移。
                crate::edit::cancel();
            }
            reset_baseline(&mut g);
        }
        1 => {
            let Some(p) = g.pointers.iter_mut().find(|p| p.id == pointer) else {
                return;
            };
            p.x = x;
            p.y = y;
            if g.pointers.len() == 1 && crate::edit::is_active() {
                crate::edit::update(x, y);
                reset_baseline(&mut g);
                return;
            }
            apply(&mut g);
        }
        _ => {
            // 单指平移松手：按最近速度启动惯性（编辑拖拽/多指不触发）。
            let single = g.pointers.len() == 1;
            let editing = crate::edit::is_active();
            g.pointers.retain(|p| p.id != pointer);
            if g.pointers.is_empty() {
                crate::edit::end(x, y);
            } else if crate::edit::is_active() {
                crate::edit::cancel();
            }
            let idle = g
                .last_move
                .map(|t| t.elapsed().as_secs_f32())
                .unwrap_or(f32::MAX);
            let speed = (g.vel.0 * g.vel.0 + g.vel.1 * g.vel.1).sqrt();
            if single
                && !editing
                && idle < 0.1
                && speed >= FLING_MIN_SPEED
                && let Ok(mut st) = INERTIA.lock()
            {
                st.vx = g.vel.0;
                st.vy = g.vel.1;
                st.at = Some(Instant::now());
            }
            g.vel = (0.0, 0.0);
            reset_baseline(&mut g);
        }
    }
}

/// 丢弃全部手势状态（如 surface 重建时）。
pub fn reset() {
    if let Ok(mut g) = GESTURE.lock() {
        g.pointers.clear();
        g.baseline = None;
    }
}

/// 丢弃手势并取消编辑：模态窗口打开/App 切后台时调用。
///
/// Flutter 在路由切换或生命周期变化时可能收不到某个触点的 up，
/// 残留触点会让下一次单指被误判为双指（平移/缩放）。
pub fn reset_all() {
    reset();
    stop_inertia();
    crate::edit::cancel();
}

fn reset_baseline(g: &mut GestureState) {
    g.baseline = match g.pointers.len() {
        1 => Some(Baseline {
            dist: 0.0,
            mid: (g.pointers[0].x, g.pointers[0].y),
        }),
        2 => Some(two_finger_baseline(&g.pointers)),
        _ => None,
    };
}

fn two_finger_baseline(pts: &[Pointer]) -> Baseline {
    let (a, b) = (&pts[0], &pts[1]);
    Baseline {
        dist: ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt(),
        mid: ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5),
    }
}

/// 依据当前触点与基线的差值更新当前视图。
fn apply(g: &mut GestureState) {
    let Some(base) = g.baseline else {
        reset_baseline(g);
        return;
    };

    // 单指平移速度（指数平滑）：内容跟随手指，视图速度 = 手指速度。
    if g.pointers.len() == 1 {
        let p = g.pointers[0];
        let (dx, dy) = (p.x - base.mid.0, p.y - base.mid.1);
        let now = Instant::now();
        if let Some(t0) = g.last_move {
            let dt = now.duration_since(t0).as_secs_f32();
            if dt > 0.12 {
                // 手指停顿后再动：不继承旧速度（避免突然弹飞）。
                g.vel = (0.0, 0.0);
            } else if dt > 0.0 {
                let alpha = (dt / 0.08).clamp(0.2, 1.0);
                g.vel.0 = g.vel.0 * (1.0 - alpha) + (dx / dt) * alpha;
                g.vel.1 = g.vel.1 * (1.0 - alpha) + (dy / dt) * alpha;
            }
        }
        g.last_move = Some(now);
    }

    let Ok(mut s) = scene::scene().lock() else {
        return;
    };
    let (lw, lh) = crate::logical_size();
    let total_ticks = s.total_ticks;
    let num_tracks = s.num_tracks;
    match s.view_mode {
        ViewMode::Pianoroll => {
            apply_pianoroll(&mut s, g, base, lw, lh);
            // 写入即收口：否则未 clamp 的中间位移会被快照读取，浮层闪一帧再归位。
            s.pr_view.clamp_scroll(lw, lh, total_ticks);
        }
        ViewMode::Arrangement => {
            apply_arrangement(&mut s, g, base);
            s.arr_view.clamp_scroll(lw, lh, total_ticks, num_tracks + 1);
        }
    }
    // 视口变了：覆盖层里的选框是按内容坐标换算的屏幕矩形，必须失效重算，
    // 否则 Flutter 会继续画旧矩形，选框看起来钉在屏幕上（桌面每帧重算）。
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    drop(s);

    reset_baseline(g);
}

fn apply_pianoroll(s: &mut RenderScene, g: &GestureState, base: Baseline, lw: f32, lh: f32) {
    let view = &mut s.pr_view;
    match g.pointers.len() {
        1 => {
            let p = g.pointers[0];
            let (dx, dy) = (p.x - base.mid.0, p.y - base.mid.1);
            if dx != 0.0 || dy != 0.0 {
                *view.main_scroll() -= dx;
                *view.cross_scroll() -= dy;
            }
        }
        2 => {
            let now = two_finger_baseline(&g.pointers);
            if now.dist > 1.0 && base.dist > 1.0 {
                let factor = now.dist / base.dist;
                if (factor - 1.0).abs() > 0.001 {
                    let kb_w = view.keyboard_width();
                    if now.mid.0 < kb_w {
                        // 键盘列：音高缩放（锚点 y）。
                        let cross = view.cross_axis_len(lw, lh);
                        view.zoom_cross_around(now.mid.1, factor, cross);
                    } else {
                        // 内容区：时间缩放（坐标系原点在键盘列右缘）。
                        view.zoom_main_around(now.mid.0 - kb_w, factor);
                    }
                }
            }
            let (mdx, mdy) = (now.mid.0 - base.mid.0, now.mid.1 - base.mid.1);
            if mdx != 0.0 || mdy != 0.0 {
                *view.main_scroll() -= mdx;
                *view.cross_scroll() -= mdy;
            }
        }
        _ => {}
    }
}

/// 每帧惯性滚动（Flutter 帧驱动调用）：速度指数衰减并 scroll 视图。
///
/// 返回 1 = 本帧有滚动；0 = 无惯性。
pub fn tick_inertia() -> i32 {
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

fn apply_arrangement(s: &mut RenderScene, g: &GestureState, base: Baseline) {
    let view = &mut s.arr_view;
    match g.pointers.len() {
        1 => {
            let p = g.pointers[0];
            let (dx, dy) = (p.x - base.mid.0, p.y - base.mid.1);
            if dx != 0.0 || dy != 0.0 {
                view.base.scroll_x -= dx;
                view.base.scroll_y -= dy;
            }
        }
        2 => {
            let now = two_finger_baseline(&g.pointers);
            if now.dist > 1.0 && base.dist > 1.0 {
                let factor = now.dist / base.dist;
                if (factor - 1.0).abs() > 0.001 {
                    if now.mid.0 < view.base.left_panel_width {
                        // 轨道面板区：行高离散缩放（锚点 y）。
                        view.zoom_lane_height(now.mid.1, factor);
                    } else {
                        // 内容区：时间缩放。
                        view.base.zoom_around_x(now.mid.0, factor);
                    }
                }
            }
            let (mdx, mdy) = (now.mid.0 - base.mid.0, now.mid.1 - base.mid.1);
            if mdx != 0.0 || mdy != 0.0 {
                view.base.scroll_x -= mdx;
                view.base.scroll_y -= mdy;
            }
        }
        _ => {}
    }
}
