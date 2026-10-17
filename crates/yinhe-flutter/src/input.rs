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

/// 触摸事件：`kind` 0=down 1=move 2=up/cancel，`x`/`y` 为画布逻辑像素。
pub fn on_touch(kind: i32, x: f32, y: f32, pointer: i32) {
    let Ok(mut g) = GESTURE.lock() else { return };
    match kind {
        0 => {
            // 同一 pointer id 重复 down：以新位置为准。
            g.pointers.retain(|p| p.id != pointer);
            g.pointers.push(Pointer { id: pointer, x, y });
            reset_baseline(&mut g);
        }
        1 => {
            let Some(p) = g.pointers.iter_mut().find(|p| p.id == pointer) else {
                return;
            };
            p.x = x;
            p.y = y;
            apply(&mut g);
        }
        _ => {
            g.pointers.retain(|p| p.id != pointer);
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
    match s.view_mode {
        ViewMode::Pianoroll => apply_pianoroll(&mut s, g, base, lw, lh),
        ViewMode::Arrangement => apply_arrangement(&mut s, g, base),
    }
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
