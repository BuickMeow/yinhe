//! AR MIDI Clip 交互：标题栏拖动移动、左右边缘拖动 loop。
//!
//! 与音频片段交互一致：拖拽过程只画 ghost，不改模型；松手才产出
//! [`ClipEditCmd`]，由 arrange.rs 在 GPU scope 之外用 `Document` 统一应用
//! 并生成 undo。Clip 主体不消费指针，留给音符框选/选择。

use eframe::egui;
use yinhe_core::{Clip, TrackKind};
use yinhe_types::ArrangementView;

use super::super::{ArrangeData, ArrangeEdit};
use super::audio_edit::TrackRowGeom;
use super::render::{MIDI_CLIP_HEADER_H, rgb};

/// 边缘命中区宽度（px）。
const EDGE_W: f32 = 6.0;
/// 拖动 ghost 填充色 alpha。
const GHOST_ALPHA: u8 = 70;

/// MIDI Clip 编辑命令（arrange.rs 应用 + undo）。
#[derive(Clone, Debug)]
pub(crate) enum ClipEditCmd {
    /// 整体水平平移（tick）。
    Move {
        track: u16,
        id: u32,
        delta_ticks: i64,
    },
    /// 左边缘拖动：向左循环复制内容到更早范围。
    LoopLeft {
        track: u16,
        id: u32,
        new_start_tick: u32,
    },
    /// 右边缘拖动：向右循环复制内容到更晚范围。
    LoopRight {
        track: u16,
        id: u32,
        new_end_tick: u32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragMode {
    Move,
    LoopLeft,
    LoopRight,
}

#[derive(Clone)]
struct DragState {
    track: u16,
    clip: Clip,
    mode: DragMode,
    /// 按下点相对 Clip 起点的 tick 偏移（保证标题栏拖动不跳位）。
    grab_offset_ticks: f64,
}

/// 命中区域。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HitZone {
    Header,
    Body,
    LeftEdge,
    RightEdge,
}

fn drag_id() -> egui::Id {
    egui::Id::unique("arr_midi_clip_drag")
}

/// 处理一帧 MIDI Clip 交互。返回 true = 本帧消费了指针事件
/// （调用方应跳过 AR 框选处理）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame(
    ui: &egui::Ui,
    rect: egui::Rect,
    music_rect: egui::Rect,
    view: &ArrangementView,
    geom: &dyn TrackRowGeom,
    data: &ArrangeData<'_>,
    edit: &mut ArrangeEdit<'_>,
) -> bool {
    // ── 拖动中：画 ghost / 松手提交 ──
    if let Some(state) = ui.data(|d| d.get_temp::<DragState>(drag_id())) {
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let down = ui.input(|i| i.pointer.primary_down());
        if down {
            if let Some(pos) = pointer {
                set_drag_cursor(ui, state.mode);
                draw_drag_ghost(ui, rect, view, geom, data, &state, pos);
            }
            return true;
        }
        if let Some(pos) = pointer
            && let Some(cmd) = finish_drag(&state, pos, rect, view, data)
        {
            edit.clip_commands.push(cmd);
        }
        ui.data_mut(|d| d.remove::<DragState>(drag_id()));
        return true;
    }

    // ── 悬停：手型 / 水平缩放指针 ──
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && music_rect.contains(pos)
        && let Some((track, clip)) = hit_test(view, geom, data, pos, rect)
    {
        match hit_zone(view, geom, data, pos, rect, track, &clip) {
            HitZone::Header => ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand),
            HitZone::LeftEdge | HitZone::RightEdge => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            HitZone::Body => {}
        }
    }

    // ── 左键按下：标题栏开始移动 / 边缘开始 loop ──
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    if !pressed
        || ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        })
    {
        return false;
    }
    let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
        return false;
    };
    if !music_rect.contains(pos) {
        return false;
    }
    let Some((track, clip)) = hit_test(view, geom, data, pos, rect) else {
        return false;
    };
    let mode = match hit_zone(view, geom, data, pos, rect, track, &clip) {
        HitZone::Header => DragMode::Move,
        HitZone::LeftEdge => DragMode::LoopLeft,
        HitZone::RightEdge => DragMode::LoopRight,
        // 主体留给音符选择/框选，不抢指针。
        HitZone::Body => return false,
    };
    let grab_offset_ticks = clip.start_tick as f64 - view.x_to_tick(pos.x - rect.min.x);
    ui.data_mut(|d| {
        d.insert_temp(
            drag_id(),
            DragState {
                track,
                clip,
                mode,
                grab_offset_ticks,
            },
        )
    });
    set_drag_cursor(ui, mode);
    true
}

fn set_drag_cursor(ui: &egui::Ui, mode: DragMode) {
    let icon = match mode {
        DragMode::Move => egui::CursorIcon::Grabbing,
        DragMode::LoopLeft | DragMode::LoopRight => egui::CursorIcon::ResizeHorizontal,
    };
    ui.ctx().set_cursor_icon(icon);
}

/// 拖动松手：把 ghost 位置换算成命令。
fn finish_drag(
    state: &DragState,
    pos: egui::Pos2,
    rect: egui::Rect,
    view: &ArrangementView,
    data: &ArrangeData<'_>,
) -> Option<ClipEditCmd> {
    let local_x = pos.x - rect.min.x;
    match state.mode {
        DragMode::Move => {
            let start = snapped_tick(view.x_to_tick(local_x) + state.grab_offset_ticks, data);
            let delta = start as i64 - state.clip.start_tick as i64;
            (delta != 0).then_some(ClipEditCmd::Move {
                track: state.track,
                id: state.clip.id,
                delta_ticks: delta,
            })
        }
        DragMode::LoopLeft => {
            let (prev_end, _) = neighbor_bounds(data, state.track, &state.clip);
            let new_start = snapped_tick(view.x_to_tick(local_x), data)
                .max(prev_end)
                .min(state.clip.end_tick.saturating_sub(1));
            (new_start != state.clip.start_tick).then_some(ClipEditCmd::LoopLeft {
                track: state.track,
                id: state.clip.id,
                new_start_tick: new_start,
            })
        }
        DragMode::LoopRight => {
            let (_, next_start) = neighbor_bounds(data, state.track, &state.clip);
            let new_end = snapped_tick(view.x_to_tick(local_x), data)
                .min(next_start)
                .max(state.clip.start_tick.saturating_add(1));
            (new_end != state.clip.end_tick).then_some(ClipEditCmd::LoopRight {
                track: state.track,
                id: state.clip.id,
                new_end_tick: new_end,
            })
        }
    }
}

/// 视图本地 x → 吸附后的 tick。
fn snapped_tick(tick: f64, data: &ArrangeData<'_>) -> u32 {
    let snapped = crate::view_interaction::snap_tick(
        tick.max(0.0),
        data.quantize,
        data.ppq,
        data.bar_line_data,
    );
    clamped_u32(snapped.max(0.0))
}

fn clamped_u32(v: f64) -> u32 {
    v.min(u32::MAX as f64) as u32
}

/// 相邻 Clip 边界：(prev_end, next_start)；没有相邻 Clip 时分别为 0 / u32::MAX。
fn neighbor_bounds(data: &ArrangeData<'_>, track: u16, clip: &Clip) -> (u32, u32) {
    let Some(track_data) = data.tracks.get(track as usize) else {
        return (0, u32::MAX);
    };
    let Some(idx) = track_data.clips.iter().position(|c| c.id == clip.id) else {
        return (0, u32::MAX);
    };
    let prev_end = idx
        .checked_sub(1)
        .map_or(0, |i| track_data.clips[i].end_tick);
    let next_start = track_data
        .clips
        .get(idx + 1)
        .map_or(u32::MAX, |c| c.start_tick);
    (prev_end, next_start)
}

/// 画拖动 ghost：移动时画目标块，loop 时画目标边界与新增区域。
fn draw_drag_ghost(
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &ArrangementView,
    geom: &dyn TrackRowGeom,
    data: &ArrangeData<'_>,
    state: &DragState,
    pos: egui::Pos2,
) {
    let Some((row_y, row_h)) = geom.track_row_yh(state.track) else {
        return;
    };
    let painter = ui.painter();
    let scroll_y = view.base.scroll_y;
    let y_top = rect.min.y + row_y - scroll_y;
    let color = data
        .track_colors
        .get(state.track as usize)
        .copied()
        .map(rgb)
        .map(|c| egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), GHOST_ALPHA))
        .unwrap_or(egui::Color32::TRANSPARENT);
    let stroke = egui::Stroke::new(1.5, crate::theme::contrast_fg());
    let local_x = pos.x - rect.min.x;
    let clip = &state.clip;
    let x0 = view.tick_to_x(clip.start_tick as f64);
    let x1 = view.tick_to_x(clip.end_tick as f64);
    let header_h = MIDI_CLIP_HEADER_H.min(row_h - 2.0).max(0.0);

    match state.mode {
        DragMode::Move => {
            let start = snapped_tick(view.x_to_tick(local_x) + state.grab_offset_ticks, data);
            let len = clip.end_tick.saturating_sub(clip.start_tick).max(1);
            let nx0 = view.tick_to_x(start as f64);
            let nx1 = view.tick_to_x(start.saturating_add(len) as f64);
            let r = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + nx0, y_top),
                egui::pos2(rect.min.x + nx1.max(nx0 + 2.0), y_top + row_h),
            );
            painter.rect_filled(r, 2.0, color);
            painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(r.min.x, r.min.y + header_h),
                    egui::pos2(r.max.x, r.min.y + header_h),
                ],
                egui::Stroke::new(1.0, crate::theme::contrast_fg().gamma_multiply(0.5)),
            );
        }
        DragMode::LoopLeft => {
            let (prev_end, _) = neighbor_bounds(data, state.track, clip);
            let new_start = snapped_tick(view.x_to_tick(local_x), data)
                .max(prev_end)
                .min(clip.end_tick.saturating_sub(1));
            let nx0 = view.tick_to_x(new_start as f64);
            let fill = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + nx0, y_top),
                egui::pos2(rect.min.x + x0, y_top + row_h),
            );
            painter.rect_filled(fill, 2.0, color);
            let x = rect.min.x + nx0;
            painter.line_segment([egui::pos2(x, y_top), egui::pos2(x, y_top + row_h)], stroke);
        }
        DragMode::LoopRight => {
            let (_, next_start) = neighbor_bounds(data, state.track, clip);
            let new_end = snapped_tick(view.x_to_tick(local_x), data)
                .min(next_start)
                .max(clip.start_tick.saturating_add(1));
            let nx1 = view.tick_to_x(new_end as f64);
            let fill = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + x1, y_top),
                egui::pos2(rect.min.x + nx1, y_top + row_h),
            );
            painter.rect_filled(fill, 2.0, color);
            let x = rect.min.x + nx1;
            painter.line_segment([egui::pos2(x, y_top), egui::pos2(x, y_top + row_h)], stroke);
        }
    }
}

/// 命中测试：返回 (轨道, Clip 克隆)。
fn hit_test(
    view: &ArrangementView,
    geom: &dyn TrackRowGeom,
    data: &ArrangeData<'_>,
    pos: egui::Pos2,
    rect: egui::Rect,
) -> Option<(u16, Clip)> {
    let scroll_y = view.base.scroll_y;
    let my = pos.y - rect.min.y + scroll_y;
    let track = geom.track_at_music_y(my)?;
    let track_data = data.tracks.get(track as usize)?;
    if track_data.kind != TrackKind::Midi {
        return None;
    }
    let (gy, gh) = geom.track_row_yh(track)?;
    let y_top = rect.min.y + gy - scroll_y;
    if pos.y < y_top || pos.y > y_top + gh {
        return None;
    }
    let local_x = pos.x - rect.min.x;
    track_data
        .clips
        .iter()
        .find(|c| {
            let x0 = view.tick_to_x(c.start_tick as f64);
            let x1 = view.tick_to_x(c.end_tick as f64);
            local_x >= x0 - EDGE_W && local_x <= x1 + EDGE_W
        })
        .cloned()
        .map(|c| (track, c))
}

/// 细分命中区（边缘优先，其次标题栏，最后主体）。
fn hit_zone(
    view: &ArrangementView,
    geom: &dyn TrackRowGeom,
    _data: &ArrangeData<'_>,
    pos: egui::Pos2,
    rect: egui::Rect,
    track: u16,
    clip: &Clip,
) -> HitZone {
    let local_x = pos.x - rect.min.x;
    let x0 = view.tick_to_x(clip.start_tick as f64);
    let x1 = view.tick_to_x(clip.end_tick as f64);
    if (local_x - x0).abs() <= EDGE_W {
        return HitZone::LeftEdge;
    }
    if (local_x - x1).abs() <= EDGE_W {
        return HitZone::RightEdge;
    }
    let header_h = geom
        .track_row_yh(track)
        .map(|(_, h)| MIDI_CLIP_HEADER_H.min(h - 2.0).max(0.0))
        .unwrap_or(0.0);
    let y_top = geom
        .track_row_yh(track)
        .map(|(gy, _)| rect.min.y + gy - view.base.scroll_y)
        .unwrap_or(0.0);
    if pos.y <= y_top + header_h {
        HitZone::Header
    } else {
        HitZone::Body
    }
}
