//! 钢笔工具（Illustrator 风格）：多锚点贝塞尔路径的绘制与编辑。
//!
//! 交互：
//! - 点击空白：追加角点；未在绘制时则以该点为起点新建路径。
//! - 点击并拖拽空白：追加平滑点，拖出对称手柄。
//! - 拖拽锚点：移动锚点。
//! - 拖拽手柄：调整曲线（平滑点两侧对称；按住 Alt 只动一侧）。
//! - 单击锚点：删除（首个锚点除外）；绘制中单击首个锚点：闭合路径。
//! - Alt + 单击锚点：角点 / 平滑点互转。
//! - 单击线段：在线上插入锚点（保持形状）。
//! - Enter / Esc：结束绘制（Esc 在路径不足两点时清除）。

use eframe::egui;

use yinhe_editor_core::pen::{PenAnchor, PenPath, Pt};
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{PianoRollView, TimeSigEvent};

use super::drag;

/// 锚点 / 手柄命中半径（px）。
const HIT_PX: f32 = 7.0;
/// 单击判定：释放时鼠标位移小于此值视为单击（而非拖拽）。
const CLICK_PX: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HandleSide {
    In,
    Out,
}

#[derive(Clone, Copy, Debug)]
enum PenDrag {
    /// 拖动锚点（记录起始鼠标，用于区分单击删除）。
    Anchor { index: usize, start_mouse: (f32, f32) },
    /// 拖动某锚点的方向手柄。
    Handle { index: usize, side: HandleSide },
    /// 新按下的锚点（尚未确定是否拖出手柄）。
    NewAnchor { index: usize },
}

#[derive(Clone, Copy, Default, Debug)]
struct PenMemory {
    drag: Option<PenDrag>,
    drawing: bool,
}

enum PenHit {
    Handle(usize, HandleSide),
    Anchor(usize),
    /// 命中线段：`(段索引, 线上精确逻辑坐标)`，用于插入锚点。
    Segment(usize, Pt),
}

// ── 坐标转换（content-local） ──

/// key（f64）→ 副轴像素（行中心）。
fn key_px(view: &PianoRollView, key: f64) -> f32 {
    let k = key as f32 + 0.5;
    if view.is_vertical() {
        k * view.key_height - view.base.scroll_x
    } else {
        view.total_key_height() - view.base.scroll_y - k * view.key_height
    }
}

/// `(tick, key)` → content-local 像素。
pub(crate) fn point_px(view: &PianoRollView, tick: f64, key: f64) -> egui::Pos2 {
    let main_px = drag::tick_to_main_px_dir(view, tick);
    let cross_px = key_px(view, key);
    if view.is_vertical() {
        egui::pos2(cross_px, main_px)
    } else {
        egui::pos2(main_px, cross_px)
    }
}

fn screen(content_rect: egui::Rect, p: egui::Pos2) -> egui::Pos2 {
    egui::pos2(content_rect.min.x + p.x, content_rect.min.y + p.y)
}

fn local_of(content_rect: egui::Rect, pos: egui::Pos2) -> egui::Pos2 {
    egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y)
}

/// 副轴像素 → key（f64，连续，行中心）。
fn key_at_px(view: &PianoRollView, cross_px: f32) -> f64 {
    if view.is_vertical() {
        (((cross_px + view.base.scroll_x) / view.key_height) - 0.5).clamp(0.0, 127.0) as f64
    } else {
        let bottom = view.total_key_height() - view.base.scroll_y;
        (((bottom - cross_px) / view.key_height) - 0.5).clamp(0.0, 127.0) as f64
    }
}

/// 吸附后的 `(tick, key)`（锚点用：tick 走量化，key 取整数行）。
fn snapped_point(
    view: &PianoRollView,
    content_rect: egui::Rect,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    pos: egui::Pos2,
) -> Pt {
    let local = local_of(content_rect, pos);
    let (main_px, cross_px) = drag::main_cross_x_y(view, (local.x, local.y));
    let raw_tick = drag::main_px_to_tick_dir(view, main_px);
    let tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data).max(0.0);
    (tick, view.cross_px_to_key(cross_px) as f64)
}

/// 自由的 `(tick, key)`（手柄用：不吸附）。
fn free_point(view: &PianoRollView, content_rect: egui::Rect, pos: egui::Pos2) -> Pt {
    let local = local_of(content_rect, pos);
    let (main_px, cross_px) = drag::main_cross_x_y(view, (local.x, local.y));
    let tick = drag::main_px_to_tick_dir(view, main_px).max(0.0);
    (tick, key_at_px(view, cross_px))
}

/// 点到线段的距离。
fn dist_seg(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// 命中检测：手柄 > 锚点 > 线段。
fn hit_test(view: &PianoRollView, path: &PenPath, local: egui::Pos2) -> Option<PenHit> {
    for (i, a) in path.anchors.iter().enumerate() {
        for (side, h) in [(HandleSide::Out, a.out_handle), (HandleSide::In, a.in_handle)] {
            if let Some(h) = h {
                let p = point_px(view, a.tick + h.0, a.key + h.1);
                if local.distance(p) <= HIT_PX {
                    return Some(PenHit::Handle(i, side));
                }
            }
        }
    }
    for (i, a) in path.anchors.iter().enumerate() {
        if local.distance(point_px(view, a.tick, a.key)) <= HIT_PX {
            return Some(PenHit::Anchor(i));
        }
    }
    // 线段：逐段采样折线，找最近点。
    let mut best: Option<(f32, usize, Pt)> = None;
    for i in 0..path.segment_count() {
        let (p0, c0, c1, p1) = path.segment(i).expect("segment in range");
        let n = 24;
        let mut prev = point_px(view, p0.0, p0.1);
        for j in 1..=n {
            let t = j as f64 / n as f64;
            let q = sample_cubic(p0, c0, c1, p1, t);
            let cur = point_px(view, q.0, q.1);
            let d = dist_seg(local, prev, cur);
            if best.as_ref().is_none_or(|(bd, _, _)| d < *bd) {
                best = Some((d, i, q));
            }
            prev = cur;
        }
    }
    if let Some((d, seg, point)) = best
        && d <= HIT_PX
    {
        return Some(PenHit::Segment(seg, point));
    }
    None
}

fn sample_cubic(p0: Pt, c0: Pt, c1: Pt, p1: Pt, t: f64) -> Pt {
    let u = 1.0 - t;
    let (uu, tt) = (u * u, t * t);
    (
        uu * u * p0.0 + 3.0 * uu * t * c0.0 + 3.0 * u * tt * c1.0 + tt * t * p1.0,
        uu * u * p0.1 + 3.0 * uu * t * c0.1 + 3.0 * u * tt * c1.1 + tt * t * p1.1,
    )
}

/// 角点 ↔ 平滑点互转（平滑点取相邻锚点连线的 1/3 作为手柄）。
fn convert_anchor(anchors: &mut [PenAnchor], i: usize, closed: bool) {
    if anchors[i].is_smooth() {
        anchors[i].in_handle = None;
        anchors[i].out_handle = None;
        return;
    }
    let n = anchors.len();
    let cur = anchors[i].pos();
    let prev = if i > 0 {
        anchors[i - 1].pos()
    } else if closed {
        anchors[n - 1].pos()
    } else {
        cur
    };
    let next = if i + 1 < n {
        anchors[i + 1].pos()
    } else if closed {
        anchors[0].pos()
    } else {
        cur
    };
    anchors[i].in_handle = Some(((prev.0 - cur.0) / 3.0, (prev.1 - cur.1) / 3.0));
    anchors[i].out_handle = Some(((next.0 - cur.0) / 3.0, (next.1 - cur.1) / 3.0));
}

/// 钢笔路径的像素包围盒（含锚点/手柄），用于浮动条定位（content-local）。
pub(crate) fn pixel_bbox(view: &PianoRollView, path: &PenPath) -> egui::Rect {
    let mut rect: Option<egui::Rect> = None;
    let mut extend = |p: egui::Pos2| {
        let r = egui::Rect::from_center_size(p, egui::Vec2::ZERO).expand(HIT_PX);
        rect = Some(match rect {
            Some(r0) => r0.union(r),
            None => r,
        });
    };
    for a in &path.anchors {
        extend(point_px(view, a.tick, a.key));
        for h in [a.in_handle, a.out_handle].into_iter().flatten() {
            extend(point_px(view, a.tick + h.0, a.key + h.1));
        }
    }
    rect.unwrap_or(egui::Rect::NOTHING)
}

/// 当前是否处于连续绘制状态（供渲染层画橡皮筋）。
pub(crate) fn is_drawing(ui: &egui::Ui) -> bool {
    let mem_id = ui.id().with("pen_tool");
    ui.data_mut(|d| d.get_persisted::<PenMemory>(mem_id))
        .map(|m| m.drawing)
        .unwrap_or(false)
}

/// 每帧交互：编辑 `path`（None 表示无路径）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut PianoRollView,
    path: &mut Option<PenPath>,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
) {
    let mem_id = ui.id().with("pen_tool");
    let mut mem: PenMemory = ui.data_mut(|d| d.get_persisted(mem_id)).unwrap_or_default();

    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return;
    }
    let pointer = ui.input(|i| i.pointer.clone());
    let alt = ui.input(|i| i.modifiers.alt);

    if mem.drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        mem.drag = None;
    }

    // 结束绘制。
    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        mem.drawing = false;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        mem.drawing = false;
        if path.as_ref().is_some_and(|p| p.len() < 2) {
            *path = None;
        }
    }

    // 浮动条上按下不启动拖拽。
    let press_on_bar = path.as_ref().is_some_and(|p| {
        let bbox = pixel_bbox(view, p);
        crate::widgets::selection_actions::bar_rect(music_rect, bbox, 1)
            .is_some_and(|bar| pointer.hover_pos().is_some_and(|q| bar.contains(q)))
    });

    if pointer.primary_pressed()
        && !press_on_bar
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
    {
        let local = local_of(content_rect, pos);
        let hit = path.as_ref().and_then(|p| hit_test(view, p, local));
        match hit {
            Some(PenHit::Handle(index, side)) => {
                mem.drag = Some(PenDrag::Handle { index, side });
            }
            Some(PenHit::Anchor(index)) => {
                if index == 0 && mem.drawing {
                    if let Some(p) = path.as_mut() {
                        p.closed = true;
                    }
                    mem.drawing = false;
                } else if alt {
                    if let Some(p) = path.as_mut() {
                        convert_anchor(&mut p.anchors, index, p.closed);
                    }
                } else {
                    mem.drag = Some(PenDrag::Anchor {
                        index,
                        start_mouse: (pos.x, pos.y),
                    });
                }
            }
            Some(PenHit::Segment(seg, point)) => {
                if let Some(p) = path.as_mut() {
                    let at = seg + 1;
                    p.anchors.insert(at, PenAnchor::new(point.0, point.1));
                    mem.drag = Some(PenDrag::Anchor {
                        index: at,
                        start_mouse: (pos.x, pos.y),
                    });
                }
            }
            None => {
                let p = snapped_point(view, content_rect, quantize, ppq, bar_line_data, pos);
                let index = match path.as_mut() {
                    Some(existing) if mem.drawing => {
                        existing.anchors.push(PenAnchor::new(p.0, p.1));
                        existing.anchors.len() - 1
                    }
                    _ => {
                        *path = Some(PenPath {
                            anchors: vec![PenAnchor::new(p.0, p.1)],
                            closed: false,
                        });
                        mem.drawing = true;
                        0
                    }
                };
                mem.drag = Some(PenDrag::NewAnchor { index });
            }
        }
        // 空路径（删空）清理。
        if path.as_ref().is_some_and(|p| p.is_empty()) {
            *path = None;
            mem.drawing = false;
        }
        ui.data_mut(|d| d.insert_persisted(mem_id, mem));
    }

    if let Some(drag_kind) = mem.drag {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            drag::drag_scroll_and_clamp(ui, view, content_rect, music_rect, total_ticks, pos);
            let clamped = pos.clamp(music_rect.min, music_rect.max);
            match drag_kind {
                PenDrag::Anchor { index, .. } => {
                    let p = snapped_point(view, content_rect, quantize, ppq, bar_line_data, clamped);
                    if let Some(a) = path.as_mut().and_then(|p| p.anchors.get_mut(index)) {
                        a.tick = p.0;
                        a.key = p.1;
                    }
                }
                PenDrag::Handle { index, side } => {
                    let f = free_point(view, content_rect, clamped);
                    if let Some(a) = path.as_mut().and_then(|p| p.anchors.get_mut(index)) {
                        let off = (f.0 - a.tick, f.1 - a.key);
                        match side {
                            HandleSide::Out => {
                                a.out_handle = Some(off);
                                if !alt && a.in_handle.is_some() {
                                    a.in_handle = Some((-off.0, -off.1));
                                }
                            }
                            HandleSide::In => {
                                a.in_handle = Some(off);
                                if !alt && a.out_handle.is_some() {
                                    a.out_handle = Some((-off.0, -off.1));
                                }
                            }
                        }
                    }
                }
                PenDrag::NewAnchor { index } => {
                    let f = free_point(view, content_rect, clamped);
                    if let Some(a) = path.as_mut().and_then(|p| p.anchors.get_mut(index)) {
                        let off = (f.0 - a.tick, f.1 - a.key);
                        a.out_handle = Some(off);
                        a.in_handle = Some((-off.0, -off.1));
                    }
                }
            }
        }
        if pointer.primary_released() {
            if let PenDrag::Anchor { index, start_mouse } = drag_kind {
                let moved = pointer.hover_pos().is_none_or(|pos| {
                    ((pos.x - start_mouse.0).powi(2) + (pos.y - start_mouse.1).powi(2)).sqrt()
                        > CLICK_PX
                });
                if !moved && index != 0 {
                    if let Some(p) = path.as_mut() {
                        p.anchors.remove(index);
                    }
                    if path.as_ref().is_some_and(|p| p.is_empty()) {
                        *path = None;
                        mem.drawing = false;
                    }
                }
            }
            mem.drag = None;
        }
    }
    ui.data_mut(|d| d.insert_persisted(mem_id, mem));
}

/// 绘制路径、锚点、手柄、生成位置预览与橡皮筋。
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    view: &PianoRollView,
    path: &PenPath,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    drawing: bool,
    mouse_pos: Option<egui::Pos2>,
) {
    let color = crate::theme::accent_active();
    let s = |p: Pt| screen(content_rect, point_px(view, p.0, p.1));

    // 路径（逐段贝塞尔）。
    for i in 0..path.segment_count() {
        let (p0, c0, c1, p1) = path.segment(i).expect("segment in range");
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [s(p0), s(c0), s(c1), s(p1)],
            false,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(1.5, color),
        ));
    }

    // 手柄。
    for a in &path.anchors {
        let ap = screen(content_rect, point_px(view, a.tick, a.key));
        for h in [a.in_handle, a.out_handle].into_iter().flatten() {
            let hp = screen(content_rect, point_px(view, a.tick + h.0, a.key + h.1));
            painter.line_segment([ap, hp], egui::Stroke::new(1.0, color.gamma_multiply(0.6)));
            painter.circle_filled(hp, 3.5, color);
        }
    }

    // 锚点：平滑点画圆、角点画方。
    for a in &path.anchors {
        let p = screen(content_rect, point_px(view, a.tick, a.key));
        if a.is_smooth() {
            painter.circle_filled(p, 4.0, color);
        } else {
            painter.rect_filled(egui::Rect::from_center_size(p, egui::vec2(7.0, 7.0)), 0.0, color);
        }
    }

    // 生成位置预览（吸附后的行中心小圆点）。
    let interval = quantize.tick_interval(ppq);
    if interval > 0 {
        for (key, tick) in path.note_points(interval as f64) {
            let snapped =
                crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data).max(0.0);
            let p = screen(content_rect, point_px(view, snapped, key as f64));
            painter.circle_filled(p, 2.5, color.gamma_multiply(0.85));
        }
    }

    // 橡皮筋：绘制中从末锚点到鼠标。
    if drawing
        && let Some(mouse) = mouse_pos
        && let Some(last) = path.anchors.last()
    {
        let a = screen(content_rect, point_px(view, last.tick, last.key));
        let f = free_point(view, content_rect, mouse);
        let b = screen(content_rect, point_px(view, f.0, f.1));
        painter.line_segment([a, b], egui::Stroke::new(1.0, color.gamma_multiply(0.5)));
    }
}
