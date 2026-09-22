//! Scissors 工具：在钢琴卷帘上拖一条（可斜的）切割线。
//!
//! 起终点吸附到量化刻度（带小节感知）；线穿过每个音高行时在该行取一个切点
//! （线性插值后再吸附）。release 时把 `(key, cut_tick)` 列表交给
//! `Document::split_notes_at` 执行。单击 / 水平拖拽没有音高跨度：在该 tick
//! 全列切一刀（所有音高行）。

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{PianoRollView, TimeSigEvent};

use super::types::PianoViewEvent;

/// 拖拽中持久化到 egui memory 的剪刀状态（吸附后的 `(tick, key)`）。
#[derive(Clone, Copy)]
struct ScissorsDrag {
    start: (f64, u8),
    current: (f64, u8),
}

/// 逐行切点：`(key, cut_tick)`，按 key 升序。
pub(crate) type ScissorsCuts = Vec<(u8, u32)>;

/// 由起终点计算逐行切点。
///
/// 同一行（单击 / 水平拖拽）→ 起点 tick 处全列一刀；跨行 → 每行按线穿过
/// 该行的 tick 线性插值，再吸附到量化网格。
pub(crate) fn line_cuts(
    start: (f64, u8),
    end: (f64, u8),
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> ScissorsCuts {
    let snap = |t: f64| -> u32 {
        crate::view_interaction::snap_tick(t, quantize, ppq, bar_line_data).max(0.0) as u32
    };
    let (t1, k1) = start;
    let (t2, k2) = end;
    if k1 == k2 {
        let cut = snap(t1);
        return (0..=yinhe_types::MAX_KEY).map(|k| (k, cut)).collect();
    }
    let (lo, hi) = (k1.min(k2), k1.max(k2));
    let span = k2 as f64 - k1 as f64;
    (lo..=hi)
        .map(|k| {
            let t = t1 + (t2 - t1) * (k as f64 - k1 as f64) / span;
            (k, snap(t))
        })
        .collect()
}

/// 剪刀工具的帧处理：press 建立起点、drag 更新终点、release 输出切割事件。
///
/// 返回 `(release 事件, 拖拽中的预览切点)`。
pub(crate) fn scissors_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (Option<PianoViewEvent>, Option<ScissorsCuts>) {
    let state_id = ui.id().with("scissors_drag");
    let mut drag_state: Option<ScissorsDrag> =
        ui.data_mut(|d| d.get_persisted(state_id)).unwrap_or(None);

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (None, None);
    }

    let pointer = ui.input(|i| i.pointer.clone());

    // 上一帧的拖拽状态若已无按键（失焦等）：清除。
    if drag_state.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        ui.data_mut(|d| d.insert_persisted(state_id, Option::<ScissorsDrag>::None));
        drag_state = None;
    }

    let to_point = |pos: egui::Pos2| -> (f64, u8) {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let (main_px, cross_px) = super::drag::main_cross_x_y(view, (local.x, local.y));
        let raw_tick = super::drag::main_px_to_tick_dir(view, main_px);
        let tick =
            crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data).max(0.0);
        (tick, view.cross_px_to_key(cross_px))
    };

    // 按下：起点吸附后建立拖拽状态（必须落在音乐区内）。
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let start = to_point(pos);
        drag_state = Some(ScissorsDrag {
            start,
            current: start,
        });
        ui.data_mut(|d| d.insert_persisted(state_id, drag_state));
    }

    // 拖动：更新终点（允许越出音乐区，clamp 到边界）。
    if pointer.primary_down()
        && let Some(state) = drag_state.as_mut()
        && let Some(pos) = pointer.hover_pos()
    {
        state.current = to_point(pos.clamp(music_rect.min, music_rect.max));
        let updated = *state;
        ui.data_mut(|d| d.insert_persisted(state_id, Some(updated)));
    }

    // 松开：输出逐行切点（单击 = 全列一刀）。
    if pointer.primary_released()
        && let Some(state) = drag_state
    {
        ui.data_mut(|d| d.insert_persisted(state_id, Option::<ScissorsDrag>::None));
        let cuts = line_cuts(state.start, state.current, quantize, ppq, bar_line_data);
        return (Some(PianoViewEvent::ScissorsSplit { cuts }), None);
    }

    // 拖拽中：逐帧重算预览切点（逐行台阶线）。
    let preview = drag_state.map(|s| line_cuts(s.start, s.current, quantize, ppq, bar_line_data));
    (None, preview)
}

/// 预览绘制：把逐行切点连成台阶折线（吸附后的实际切割位置）。
pub(crate) fn paint_preview(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    view: &PianoRollView,
    cuts: &[(u8, u32)],
) {
    let stroke = egui::Stroke::new(1.5, crate::theme::accent_active());
    let kh = view.key_height;
    let points: Vec<egui::Pos2> = cuts
        .iter()
        .map(|&(key, cut)| {
            let main_px = super::drag::tick_to_main_px_dir(view, cut as f64);
            let cross_px = view.key_to_cross_px(key) + kh * 0.5;
            let (x, y) = if view.is_vertical() {
                (cross_px, main_px)
            } else {
                (main_px, cross_px)
            };
            egui::pos2(content_rect.min.x + x, content_rect.min.y + y)
        })
        .collect();
    match points.as_slice() {
        [] => {}
        [p] => {
            let (a, b) = if view.is_vertical() {
                (
                    *p - egui::vec2(kh * 0.5, 0.0),
                    *p + egui::vec2(kh * 0.5, 0.0),
                )
            } else {
                (
                    *p - egui::vec2(0.0, kh * 0.5),
                    *p + egui::vec2(0.0, kh * 0.5),
                )
            };
            painter.line_segment([a, b], stroke);
        }
        _ => {
            painter.add(egui::Shape::line(points, stroke));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cuts(start: (f64, u8), end: (f64, u8)) -> ScissorsCuts {
        line_cuts(start, end, QuantizePreset::Fraction(1, 16), 480, None)
    }

    #[test]
    fn vertical_drag_same_tick_across_rows() {
        // 480ppq、1/16 → 间隔 120 tick；垂直划：每行同一 tick。
        let c = cuts((125.0, 60), (125.0, 63));
        assert_eq!(c, vec![(60, 120), (61, 120), (62, 120), (63, 120)]);
    }

    #[test]
    fn slanted_drag_interpolates_per_row() {
        // 从 (0,60) 到 (480,63)：每行 +160 tick，吸附到 120 倍数。
        let c = cuts((0.0, 60), (480.0, 63));
        assert_eq!(c, vec![(60, 0), (61, 120), (62, 360), (63, 480)]);
    }

    #[test]
    fn click_cuts_whole_column_at_snapped_tick() {
        let c = cuts((100.0, 42), (100.0, 42));
        assert_eq!(c.len(), yinhe_types::KEY_COUNT, "单击 = 全列所有键");
        assert!(c.iter().all(|&(_, t)| t == 120));
        assert_eq!(c[0], (0, 120));
        assert_eq!(c[127], (127, 120));
    }

    #[test]
    fn horizontal_drag_uses_start_tick() {
        // 同一行水平拖拽：取起点 tick，全列一刀。
        let c = cuts((100.0, 42), (700.0, 42));
        assert_eq!(c.len(), yinhe_types::KEY_COUNT);
        assert!(c.iter().all(|&(_, t)| t == 120));
    }

    #[test]
    fn reversed_drag_is_mirrored() {
        let a = cuts((0.0, 60), (480.0, 63));
        let b = cuts((480.0, 63), (0.0, 60));
        assert_eq!(a, b, "反向拖拽结果相同（切点表按 key 升序）");
    }
}
