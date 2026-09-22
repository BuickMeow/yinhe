//! 锚点线工具（直线 / 剪刀）：两个 `(tick, key)` 锚点，可拖动调整。
//!
//! 两者共用同一个交互：拖出新线 → 线保留、锚点可再拖 → 点浮动条 ✓ 由
//! App 层执行（直线生成音符 / 剪刀切割）。单击（未拖动）时剪刀清空线、
//! 直线保留（单点线可生成一个音符）。

use eframe::egui;

use yinhe_editor_core::edit_state::AnchorLine;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{PianoRollView, TimeSigEvent};

/// 锚点命中半径（px）。
const ANCHOR_HIT_PX: f32 = 7.0;

/// 命中的是哪端锚点。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AnchorEnd {
    Start,
    End,
}

/// 锚点线拖拽模式（持久化到 egui memory）。
#[derive(Clone, Copy)]
enum AnchorDrag {
    /// 划新线（press 定起点，拖终点）。
    Draw,
    /// 拖动起点锚点。
    AnchorStart,
    /// 拖动终点锚点。
    AnchorEnd,
}

/// content-local 像素 → 屏幕坐标。
fn screen_pos(content_rect: egui::Rect, p: egui::Pos2) -> egui::Pos2 {
    egui::pos2(content_rect.min.x + p.x, content_rect.min.y + p.y)
}

/// 锚点 → content-relative 像素（行中心）。
pub(crate) fn point_px(view: &PianoRollView, tick: f64, key: u8) -> egui::Pos2 {
    let main_px = super::drag::tick_to_main_px_dir(view, tick);
    let cross_px = view.key_to_cross_px(key) + view.key_height * 0.5;
    if view.is_vertical() {
        egui::pos2(cross_px, main_px)
    } else {
        egui::pos2(main_px, cross_px)
    }
}

/// 线的像素包围盒（含锚点半径），用于浮动条定位（content-local）。
pub(crate) fn pixel_bbox(view: &PianoRollView, line: &AnchorLine) -> egui::Rect {
    let a = point_px(view, line.start.0, line.start.1);
    let b = point_px(view, line.end.0, line.end.1);
    egui::Rect::from_two_pos(a, b).expand(ANCHOR_HIT_PX)
}

/// 指针（content-local）是否命中锚点。
pub(crate) fn anchor_at(
    view: &PianoRollView,
    line: &AnchorLine,
    content_local: egui::Pos2,
) -> Option<AnchorEnd> {
    let ds = content_local.distance(point_px(view, line.start.0, line.start.1));
    let de = content_local.distance(point_px(view, line.end.0, line.end.1));
    if ds <= ANCHOR_HIT_PX && ds <= de {
        Some(AnchorEnd::Start)
    } else if de <= ANCHOR_HIT_PX {
        Some(AnchorEnd::End)
    } else {
        None
    }
}

/// 屏幕位置 → 吸附后的 `(tick, key)`。
fn point_at(
    view: &PianoRollView,
    content_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    pos: egui::Pos2,
) -> (f64, u8) {
    let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
    let (main_px, cross_px) = super::drag::main_cross_x_y(view, (local.x, local.y));
    let raw_tick = super::drag::main_px_to_tick_dir(view, main_px);
    let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data).max(0.0);
    (tick, view.cross_px_to_key(cross_px))
}

/// 锚点线编辑帧：press 命中锚点 → 拖该端点；否则从按下点重新划线。
///
/// `clear_on_click`：未拖动的单击是否清空线（剪刀 true / 直线 false）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut PianoRollView,
    line: &mut Option<AnchorLine>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    id_suffix: &'static str,
    clear_on_click: bool,
) {
    let state_id = ui.id().with(id_suffix);
    let mut drag: Option<AnchorDrag> = ui.data_mut(|d| d.get_persisted(state_id)).unwrap_or(None);

    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return;
    }
    let pointer = ui.input(|i| i.pointer.clone());

    // 上一帧的状态若已无按键（失焦等）：清除。
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
        ui.data_mut(|d| d.insert_persisted(state_id, Option::<AnchorDrag>::None));
    }

    // ✓ 浮动条上按下不启动拖拽。
    let press_on_bar = line.is_some_and(|l| {
        let bbox = pixel_bbox(view, &l);
        crate::widgets::selection_actions::bar_rect(music_rect, bbox, 1)
            .is_some_and(|bar| pointer.hover_pos().is_some_and(|p| bar.contains(p)))
    });

    if pointer.primary_pressed()
        && !press_on_bar
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let p = point_at(view, content_rect, quantize, ppq, bar_line_data, pos);
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        drag = Some(
            match line.as_ref().and_then(|l| anchor_at(view, l, local)) {
                Some(AnchorEnd::Start) => AnchorDrag::AnchorStart,
                Some(AnchorEnd::End) => AnchorDrag::AnchorEnd,
                None => {
                    *line = Some(AnchorLine { start: p, end: p });
                    AnchorDrag::Draw
                }
            },
        );
        ui.data_mut(|d| d.insert_persisted(state_id, drag));
    }

    if let Some(mode) = drag {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            super::drag::drag_scroll_and_clamp(
                ui,
                view,
                content_rect,
                music_rect,
                total_ticks,
                pos,
            );
            let clamped = pos.clamp(music_rect.min, music_rect.max);
            let p = point_at(view, content_rect, quantize, ppq, bar_line_data, clamped);
            if let Some(l) = line.as_mut() {
                match mode {
                    AnchorDrag::Draw | AnchorDrag::AnchorEnd => l.end = p,
                    AnchorDrag::AnchorStart => l.start = p,
                }
            }
        }
        if pointer.primary_released() {
            if clear_on_click
                && matches!(mode, AnchorDrag::Draw)
                && line.is_some_and(|l| l.start == l.end)
            {
                *line = None;
            }
            ui.data_mut(|d| d.insert_persisted(state_id, Option::<AnchorDrag>::None));
        }
    }
}

/// 画锚点线：线段 + 两端圆点（content-local）。
pub(crate) fn paint_line(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    view: &PianoRollView,
    line: &AnchorLine,
    color: egui::Color32,
) {
    let a = screen_pos(content_rect, point_px(view, line.start.0, line.start.1));
    let b = screen_pos(content_rect, point_px(view, line.end.0, line.end.1));
    painter.line_segment([a, b], egui::Stroke::new(1.5, color));
    for p in [a, b] {
        painter.circle_filled(p, 4.0, color);
    }
}

/// 直线工具预览：每行实际生成位置（吸附后的行中心小圆点）。
pub(crate) fn paint_snap_marks(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    view: &PianoRollView,
    line: &AnchorLine,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) {
    if quantize.tick_interval(ppq) == 0 {
        return;
    }
    let color = crate::theme::accent_active();
    let (_, k1) = line.start;
    let (_, k2) = line.end;
    let (lo, hi) = (k1.min(k2), k1.max(k2));
    for key in lo..=hi {
        let raw = yinhe_editor_core::quantize::line_tick_at_key(line.start, line.end, key);
        let tick =
            yinhe_editor_core::quantize::snap_tick(raw, quantize, ppq, bar_line_data).max(0.0);
        let p = screen_pos(content_rect, point_px(view, tick, key));
        painter.circle_filled(p, 2.5, color);
    }
}

/// 剪刀预览：逐行切点台阶折线（实际切割位置）。
pub(crate) fn paint_scissors_cuts(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    view: &PianoRollView,
    cuts: &[(u8, u32)],
) {
    let stroke = egui::Stroke::new(1.5, crate::theme::accent_active().gamma_multiply(0.8));
    let points: Vec<egui::Pos2> = cuts
        .iter()
        .map(|&(key, cut)| screen_pos(content_rect, point_px(view, cut as f64, key)))
        .collect();
    match points.as_slice() {
        [] => {}
        [p] => {
            let kh = view.key_height;
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
