//! automation 面板 velocity 模式的铅笔笔划交互。
//!
//! 笔迹扫过的力度条（按 noteon / start_tick 判定）被设为笔迹在该处的插值高度。
//! 与钢琴卷帘铅笔工具一致：只作用于 `active_track`（editing_track 且可见）。
//! 拖拽中只画 egui 预览，松开时一次性提交（一笔 = 一个 undo entry）。

use std::collections::HashMap;

use eframe::egui;

use yinhe_types::{AutomationPanelView, NoteSource, VelocityEdit};

/// 笔划命中的像素容差（pt）：单击时也会命中 start 在 ±HIT_PX 内的力度条。
const HIT_PX: f32 = 2.0;

/// 笔划中被触及的力度条（预览 + 提交用）。
#[derive(Clone, Copy, Debug)]
struct TouchedBar {
    key: u8,
    start_tick: u32,
    length: u32,
    new_velocity: u8,
}

/// 笔划状态：存在 egui data 中，跨帧保持。
#[derive(Clone)]
struct VelocityStroke {
    /// 锁定的音轨：按下瞬间的 active_track，整笔不随外部切换。
    track: u16,
    /// 上一个采样点 (tick, value)，与当前鼠标位置构成线段。
    last: (f64, f32),
    /// 已触及的力度条：同一条反复经过时后经过的值覆盖前值。
    touched: HashMap<(u8, u32), TouchedBar>,
}

/// 预览几何（屏幕坐标），由 show_panels 在 wgpu 纹理之后绘制。
pub(crate) struct VelocityPreview {
    pub bars: Vec<egui::Rect>,
    pub color: egui::Color32,
}

/// 把线段 (t0,v0)→(t1,v1) 扫过的、属于 track 的力度条写入 `touched`。
///
/// 命中只看 noteon：start_tick 落在线段 tick 窗口（±`hit_ticks`）内即被笔迹经过；
/// 新 velocity 取线段在该 start_tick 处的插值，clamp 到 1..=127。
/// 复杂度 O(log n + 命中数)：按 start_tick 二分定位窗口，与 1 亿音符场景兼容。
fn collect_segment(
    midi: &dyn NoteSource,
    track: u16,
    seg: ((f64, f32), (f64, f32)),
    hit_ticks: f64,
    touched: &mut HashMap<(u8, u32), TouchedBar>,
) {
    let ((t0, v0), (t1, v1)) = seg;
    let lo = (t0.min(t1) - hit_ticks).max(0.0);
    let hi = t0.max(t1) + hit_ticks;
    for key in 0u8..128 {
        // u32 边界保守外扩（floor/ceil），range 是 [lo, hi) 右开，hi+1 保持闭区间语义。
        let lo_u = lo.floor() as u32;
        let hi_u = hi.ceil() as u32;
        for note in midi.key_notes(key).range(lo_u, hi_u.saturating_add(1)) {
            if note.track != track {
                continue;
            }
            let t = if (t1 - t0).abs() < f64::EPSILON {
                0.0
            } else {
                ((note.start_tick as f64 - t0) / (t1 - t0)) as f32
            };
            let new_velocity = (v0 + (v1 - v0) * t).round().clamp(1.0, 127.0) as u8;
            touched.insert(
                (key, note.start_tick),
                TouchedBar {
                    key,
                    start_tick: note.start_tick,
                    length: note.end_tick - note.start_tick,
                    new_velocity,
                },
            );
        }
    }
}

/// 由笔划已触及的力度条构建预览矩形（屏幕坐标）。
/// 宽度与 GPU 力度条一致（音符长度，最短 2pt），高度为新 velocity。
fn build_preview(
    stroke: &VelocityStroke,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    color: egui::Color32,
) -> VelocityPreview {
    let ppu = panel.base.pixels_per_tick;
    let scroll_x = panel.base.scroll_x;
    let bars = stroke
        .touched
        .values()
        .map(|b| {
            let x = grid_area.min.x + b.start_tick as f32 * ppu - scroll_x;
            let w = (b.length as f32 * ppu).max(2.0);
            // 126 级映射（与 shader 一致）：vel 2..=127 → 高度 1..=126 单位。
            let top = panel_rect.min.y + panel.value_to_y((b.new_velocity - 1) as f32, 126.0);
            egui::Rect::from_min_max(egui::pos2(x, top), egui::pos2(x + w, panel_rect.max.y))
        })
        .collect();
    VelocityPreview { bars, color }
}

/// 拖拽中跟随鼠标显示的 (tick, value, 屏幕位置)。
pub(crate) type VelocityHover = (u32, f32, egui::Pos2);

/// 处理 velocity 面板上的铅笔笔划。
///
/// 返回 `(edits, preview, tooltip)`：
/// - `edits`：松开时一次性提交的批量修改（拖拽中为空）；
/// - `preview`：拖拽中及松开当帧的预览几何（松开当帧仍画，防止模型重渲染前旧条闪现）；
/// - `tooltip`：拖拽中跟随鼠标显示 (tick, value)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_velocity_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    midi: &dyn NoteSource,
    track: u16,
    track_color: [f32; 4],
    panel_index: usize,
) -> (
    Vec<VelocityEdit>,
    Option<VelocityPreview>,
    Option<VelocityHover>,
) {
    let stroke_id = ui.id().with("velocity_stroke").with(panel_index);

    let pos = ui.input(|i| i.pointer.hover_pos());
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    let down = ui.input(|i| i.pointer.primary_down());
    let released = ui.input(|i| i.pointer.primary_released());

    // 鼠标位置 → (tick, velocity)。y clamp 在面板内（与 automation 铅笔一致）。
    let ppu = panel.base.pixels_per_tick;
    let mouse = pos.map(|p| {
        let tick = (((p.x - grid_area.min.x + panel.base.scroll_x) / ppu) as f64).max(0.0);
        let y = (p.y - panel_rect.min.y).clamp(0.0, panel_rect.height());
        // 126 级映射（与 shader 一致）：y → vel = y_to_value(y, 126) + 1。
        let value = (panel.y_to_value(y, 126.0) + 1.0).clamp(1.0, 127.0);
        (p, tick, value)
    });
    let in_grid = pos.is_some_and(|p| grid_area.contains(p));

    let mut stroke: Option<VelocityStroke> = ui.ctx().data(|d| d.get_temp(stroke_id));
    let hit_ticks = (HIT_PX / ppu) as f64;

    if pressed && in_grid {
        if let Some((_, tick, value)) = mouse {
            // 单击也要命中：零长线段 + 容差收集一次
            let mut touched = HashMap::new();
            collect_segment(
                midi,
                track,
                ((tick, value), (tick, value)),
                hit_ticks,
                &mut touched,
            );
            stroke = Some(VelocityStroke {
                track,
                last: (tick, value),
                touched,
            });
        }
    } else if down && let (Some(s), Some((_, tick, value))) = (stroke.as_mut(), mouse) {
        let last = s.last;
        collect_segment(
            midi,
            s.track,
            (last, (tick, value)),
            hit_ticks,
            &mut s.touched,
        );
        s.last = (tick, value);
    }

    let color = crate::theme::rgba_to_color32((
        track_color[0],
        track_color[1],
        track_color[2],
        track_color[3],
    ));

    let mut edits = Vec::new();
    let mut preview = None;
    let mut tooltip = None;
    if released {
        if let Some(s) = stroke.take() {
            edits = s
                .touched
                .values()
                .map(|b| VelocityEdit {
                    track: s.track,
                    start_tick: b.start_tick,
                    key: b.key,
                    velocity: b.new_velocity,
                })
                .collect();
            preview = Some(build_preview(&s, grid_area, panel_rect, panel, color));
        }
        ui.ctx().data_mut(|d| d.remove::<VelocityStroke>(stroke_id));
    } else {
        if let Some(s) = &stroke {
            preview = Some(build_preview(s, grid_area, panel_rect, panel, color));
            tooltip = mouse.map(|(p, tick, value)| (tick as u32, value, p));
        }
        match stroke {
            Some(s) => {
                ui.ctx().data_mut(|d| d.insert_temp(stroke_id, s));
            }
            None => ui.ctx().data_mut(|d| d.remove::<VelocityStroke>(stroke_id)),
        }
    }

    (edits, preview, tooltip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::Note;
    use yinhe_types::NoteBucket;

    struct MockSource {
        notes: NoteBucket,
    }

    impl NoteSource for MockSource {
        fn key_notes(&self, key: u8) -> &NoteBucket {
            // 测试音符都放在 key 60
            if key == 60 {
                &self.notes
            } else {
                static EMPTY: std::sync::LazyLock<NoteBucket> =
                    std::sync::LazyLock::new(NoteBucket::default);
                &EMPTY
            }
        }

        fn duration(&self) -> f64 {
            0.0
        }
    }

    fn note(track: u16, start: u32, end: u32, velocity: u8) -> Note {
        Note {
            id: 0,
            start_tick: start,
            end_tick: end,
            velocity,
            track,
        }
    }

    #[test]
    fn segment_hits_only_active_track_and_interpolates() {
        let src = MockSource {
            notes: NoteBucket::from_sorted(vec![
                note(0, 100, 200, 64),
                note(1, 300, 400, 64), // 其他音轨：不命中
                note(0, 500, 600, 64),
            ]),
        };
        let mut touched = HashMap::new();
        // 线段 tick 0 → 1000，value 0 → 100
        collect_segment(&src, 0, ((0.0, 0.0), (1000.0, 100.0)), 0.0, &mut touched);
        assert_eq!(touched.len(), 2);
        assert_eq!(touched[&(60, 100)].new_velocity, 10);
        assert_eq!(touched[&(60, 500)].new_velocity, 50);
    }

    #[test]
    fn zero_length_segment_uses_tolerance_and_start_value() {
        let src = MockSource {
            notes: NoteBucket::from_sorted(vec![note(0, 100, 200, 64), note(0, 110, 200, 64)]),
        };
        let mut touched = HashMap::new();
        // 单击 tick 105，容差 ±5：两条都命中，值都取单击值 80
        collect_segment(&src, 0, ((105.0, 80.0), (105.0, 80.0)), 5.0, &mut touched);
        assert_eq!(touched.len(), 2);
        assert_eq!(touched[&(60, 100)].new_velocity, 80);
        assert_eq!(touched[&(60, 110)].new_velocity, 80);
    }

    #[test]
    fn later_segment_overwrites_earlier() {
        let src = MockSource {
            notes: NoteBucket::from_sorted(vec![note(0, 100, 200, 64)]),
        };
        let mut touched = HashMap::new();
        collect_segment(&src, 0, ((0.0, 30.0), (200.0, 30.0)), 0.0, &mut touched);
        collect_segment(&src, 0, ((200.0, 90.0), (0.0, 90.0)), 0.0, &mut touched);
        assert_eq!(
            touched[&(60, 100)].new_velocity,
            90,
            "回扫时后经过的值应覆盖前值"
        );
    }

    #[test]
    fn velocity_clamped_to_valid_range() {
        let src = MockSource {
            notes: NoteBucket::from_sorted(vec![note(0, 0, 100, 64), note(0, 1000, 1100, 64)]),
        };
        let mut touched = HashMap::new();
        collect_segment(&src, 0, ((0.0, -50.0), (1000.0, 500.0)), 0.0, &mut touched);
        assert_eq!(touched[&(60, 0)].new_velocity, 1);
        assert_eq!(touched[&(60, 1000)].new_velocity, 127);
    }
}
