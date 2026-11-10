//! AR MIDI Clip 交互：标题栏拖动移动、左右边缘拖动 loop。
//!
//! 拖拽过程把目标区间写入临时预览（渲染层按真实 Clip 样式实时绘制），
//! 不改模型；松手才产出 [`ClipEditCmd`]，由 arrange.rs 在 GPU scope 之外
//! 用 `Document` 统一应用并生成 undo。Clip 主体不消费指针，留给音符框选。

use eframe::egui;
use yinhe_core::{Clip, TrackKind};
use yinhe_types::ArrangementView;

use super::super::{ArrangeData, ArrangeEdit};
use super::audio_edit::TrackRowGeom;
use super::render::MIDI_CLIP_HEADER_H;

/// 边缘命中区宽度（px）。
const EDGE_W: f32 = 6.0;

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

/// 拖动中的实时预览区间（渲染层据此按真实 Clip 样式绘制）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct DragPreview {
    pub track: u16,
    pub id: u32,
    pub start_tick: u32,
    pub end_tick: u32,
}

fn drag_id() -> egui::Id {
    egui::Id::unique("arr_midi_clip_drag")
}

fn preview_id() -> egui::Id {
    egui::Id::unique("arr_midi_clip_preview")
}

/// 当前帧的 Clip 拖动预览（无拖动时 None）。
pub(crate) fn preview(ui: &egui::Ui) -> Option<DragPreview> {
    ui.data(|d| d.get_temp::<DragPreview>(preview_id()))
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
    if !data.clip_enabled {
        ui.data_mut(|d| {
            d.remove::<DragState>(drag_id());
            d.remove::<DragPreview>(preview_id());
        });
        return false;
    }

    // ── 拖动中：写实时预览 / 松手提交 ──
    let drag_state = ui.data(|d| d.get_temp::<DragState>(drag_id()));
    if let Some(state) = drag_state {
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let down = ui.input(|i| i.pointer.primary_down());
        if down {
            if let Some(pos) = pointer {
                set_drag_cursor(ui, state.mode);
                let (start_tick, end_tick) = target_range(&state, pos, rect, view, data);
                ui.data_mut(|d| {
                    d.insert_temp(
                        preview_id(),
                        DragPreview {
                            track: state.track,
                            id: state.clip.id,
                            start_tick,
                            end_tick,
                        },
                    )
                });
            }
            return true;
        }
        if let Some(pos) = pointer {
            // 松手帧也刷新一次预览：命令要等本帧绘制之后才落地，
            // 若这里立刻清掉预览，会先闪回模型旧位置、下一帧才跳到新位置。
            let (start_tick, end_tick) = target_range(&state, pos, rect, view, data);
            ui.data_mut(|d| {
                d.insert_temp(
                    preview_id(),
                    DragPreview {
                        track: state.track,
                        id: state.clip.id,
                        start_tick,
                        end_tick,
                    },
                )
            });
            if let Some(cmd) = finish_drag(&state, pos, rect, view, data) {
                edit.clip_commands.push(cmd);
            }
        }
        // 只清拖动状态，保留预览到下一帧：下一帧模型已更新，
        // 开场处会清掉预览，视觉上不会出现「跳回原点」。
        ui.data_mut(|d| d.remove::<DragState>(drag_id()));
        return true;
    }
    ui.data_mut(|d| d.remove::<DragPreview>(preview_id()));

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

/// 拖动目标区间（已吸附/夹紧到相邻 Clip 之间）。
fn target_range(
    state: &DragState,
    pos: egui::Pos2,
    rect: egui::Rect,
    view: &ArrangementView,
    data: &ArrangeData<'_>,
) -> (u32, u32) {
    let local_x = pos.x - rect.min.x;
    let clip = &state.clip;
    match state.mode {
        DragMode::Move => {
            let start = snapped_tick(view.x_to_tick(local_x) + state.grab_offset_ticks, data);
            let len = clip.end_tick.saturating_sub(clip.start_tick).max(1);
            (start, start.saturating_add(len))
        }
        DragMode::LoopLeft => {
            let (prev_end, _) = neighbor_bounds(data, state.track, clip);
            let new_start = snapped_tick(view.x_to_tick(local_x), data)
                .max(prev_end)
                .min(clip.end_tick.saturating_sub(1));
            (new_start, clip.end_tick)
        }
        DragMode::LoopRight => {
            let (_, next_start) = neighbor_bounds(data, state.track, clip);
            let new_end = snapped_tick(view.x_to_tick(local_x), data)
                .min(next_start)
                .max(clip.start_tick.saturating_add(1));
            (clip.start_tick, new_end)
        }
    }
}

/// 拖动松手：把预览区间换算成命令。
fn finish_drag(
    state: &DragState,
    pos: egui::Pos2,
    rect: egui::Rect,
    view: &ArrangementView,
    data: &ArrangeData<'_>,
) -> Option<ClipEditCmd> {
    let (start, end) = target_range(state, pos, rect, view, data);
    match state.mode {
        DragMode::Move => {
            let delta = start as i64 - state.clip.start_tick as i64;
            (delta != 0).then_some(ClipEditCmd::Move {
                track: state.track,
                id: state.clip.id,
                delta_ticks: delta,
            })
        }
        DragMode::LoopLeft => (start != state.clip.start_tick).then_some(ClipEditCmd::LoopLeft {
            track: state.track,
            id: state.clip.id,
            new_start_tick: start,
        }),
        DragMode::LoopRight => (end != state.clip.end_tick).then_some(ClipEditCmd::LoopRight {
            track: state.track,
            id: state.clip.id,
            new_end_tick: end,
        }),
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
