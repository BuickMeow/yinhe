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
}

static GESTURE: Mutex<GestureState> = Mutex::new(GestureState {
    pointers: Vec::new(),
    baseline: None,
});

/// 触摸/鼠标事件：0=down 1=move 2=up/cancel，3/4/5=鼠标右键按下/移动/松开。
///
/// 右键走独立的强制平移通道：不触发当前工具，取消进行中的编辑拖拽，
/// 语义等同于临时切换到手形工具。
pub fn on_touch(kind: i32, x: f32, y: f32, pointer: i32) {
    let Ok(mut g) = GESTURE.lock() else { return };
    match kind {
        3 => {
            // 右键按下：取消编辑拖拽后按单指平移处理。
            g.pointers.retain(|p| p.id != pointer);
            g.pointers.push(Pointer { id: pointer, x, y });
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
            g.pointers.retain(|p| p.id != pointer);
            g.pointers.push(Pointer { id: pointer, x, y });
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
            g.pointers.retain(|p| p.id != pointer);
            if g.pointers.is_empty() {
                crate::edit::end(x, y);
            } else if crate::edit::is_active() {
                crate::edit::cancel();
            }
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
