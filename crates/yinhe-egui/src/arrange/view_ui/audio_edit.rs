//! AR 音频片段交互：选择、拖动移动、边缘裁剪、淡入淡出手柄、右键菜单。
//!
//! 本模块只产出 [`AudioEditCmd`]（out-param），由 arrange.rs 在 GPU scope
//! 之外用 `Document` 统一应用并生成 undo——与 AM/音符交互同一条路径。
//! 拖拽过程只画 ghost，不改模型；松手才提交命令。
//!
//! 坐标约定：所有 `tick_to_x`/`x_to_tick` 使用视图本地 x（`pos.x - rect.min.x`），
//! 与 view_ui 其它交互一致。

use eframe::egui;
use yinhe_core::{AudioClip, TrackKind};
use yinhe_types::{ArRowLayout, ArrangementView};

use super::super::{ArrangeData, ArrangeEdit};

/// 片段边缘命中区宽度（px）。
const EDGE_W: f32 = 6.0;
/// 淡入淡出手柄尺寸（px）。
const FADE_HANDLE: f32 = 9.0;
/// 拖动 ghost 填充色 alpha。
const GHOST_ALPHA: u8 = 70;

/// 音频片段编辑命令（arrange.rs 应用 + undo）。
#[derive(Clone, Debug)]
pub(crate) enum AudioEditCmd {
    /// 时间平移（秒）。
    Move {
        track: usize,
        ids: Vec<u32>,
        delta_seconds: f64,
    },
    /// 裁剪左边缘到指定时刻（秒）。
    TrimStart {
        track: usize,
        id: u32,
        new_start_seconds: f64,
    },
    /// 裁剪右边缘到指定时刻（秒）。
    TrimEnd {
        track: usize,
        id: u32,
        new_end_seconds: f64,
    },
    /// 设置淡入/淡出（秒）。
    SetFades {
        track: usize,
        id: u32,
        fade_in_seconds: f64,
        fade_out_seconds: f64,
    },
    /// 在指定时刻（秒）分割。
    Split {
        track: usize,
        id: u32,
        at_seconds: f64,
    },
    /// 复制并平移（秒）。
    Duplicate {
        track: usize,
        ids: Vec<u32>,
        delta_seconds: f64,
    },
    /// 删除。
    Delete { track: usize, ids: Vec<u32> },
    /// 反向。
    Reverse { track: usize, ids: Vec<u32> },
    /// 按素材峰值归一化到 0 dB。
    Normalize { track: usize, id: u32 },
    /// 增益增减（dB）。
    GainDelta {
        track: usize,
        id: u32,
        delta_db: f32,
    },
    /// 重置增益为 1.0。
    GainReset { track: usize, id: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
    FadeIn,
    FadeOut,
}

#[derive(Clone)]
struct DragState {
    track: usize,
    id: u32,
    mode: DragMode,
    start_clip: AudioClip,
    alt_copy: bool,
}

/// 命中区域。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HitZone {
    Body,
    LeftEdge,
    RightEdge,
    FadeInHandle,
    FadeOutHandle,
}

fn drag_id() -> egui::Id {
    egui::Id::new("arr_audio_clip_drag")
}

/// 处理一帧音频片段交互。返回 true = 本帧音频交互消费了指针事件
/// （调用方应跳过 AR 框选处理）。
#[allow(clippy::too_many_arguments)]
pub(super) fn frame(
    ui: &egui::Ui,
    rect: egui::Rect,
    music_rect: egui::Rect,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &ArrangeData<'_>,
    edit: &mut ArrangeEdit<'_>,
) -> bool {
    // ── 拖动中：画 ghost / 松手提交 ──
    if let Some(state) = ui.data(|d| d.get_temp::<DragState>(drag_id())) {
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let down = ui.input(|i| i.pointer.primary_down());
        if down {
            if let Some(pos) = pointer {
                draw_drag_ghost(ui, rect, view, row_layout, data, &state, pos);
            }
            return true;
        }
        // 松手：按模式生成命令（需要指针最终位置）。
        if let Some(pos) = pointer
            && let Some(cmd) = finish_drag(&state, pos, rect, view, data)
        {
            edit.audio_commands.push(cmd);
        }
        ui.data_mut(|d| d.remove::<DragState>(drag_id()));
        return true;
    }

    // ── 右键菜单（不消费左键）──
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && music_rect.contains(pos)
        && let Some((track, clip)) = hit_test(view, row_layout, data, pos, rect)
    {
        let id = egui::Id::new(("arr_audio_ctx", track, clip.id));
        let resp = ui.interact(rect, id, egui::Sense::click());
        let selected_ids = selected_ids_for(edit, track, clip.id);
        resp.context_menu(|ui| {
            if crate::widgets::flat::flat_button(ui, rust_i18n::t!("arrange.audio_split")).clicked()
            {
                let at =
                    pointer_seconds(pos.x - rect.min.x, view, data).max(clip.start_seconds + 0.001);
                if at < clip.end_seconds() - 0.001 {
                    edit.audio_commands.push(AudioEditCmd::Split {
                        track,
                        id: clip.id,
                        at_seconds: at,
                    });
                }
                ui.close();
            }
            if ui
                .button(rust_i18n::t!("arrange.audio_duplicate"))
                .clicked()
            {
                // 复制到播放光标处（光标不可用则平移一个片段长度）。
                let target = edit
                    .cursor_tick
                    .map(|t| data.tempo_map.tick_to_seconds(t.max(0.0) as u64))
                    .unwrap_or(clip.start_seconds + clip.duration_seconds);
                let delta = target - clip.start_seconds;
                edit.audio_commands.push(AudioEditCmd::Duplicate {
                    track,
                    ids: selected_ids.clone(),
                    delta_seconds: delta,
                });
                ui.close();
            }
            if crate::widgets::flat::flat_button(ui, rust_i18n::t!("arrange.audio_reverse"))
                .clicked()
            {
                edit.audio_commands.push(AudioEditCmd::Reverse {
                    track,
                    ids: selected_ids.clone(),
                });
                ui.close();
            }
            if ui
                .button(rust_i18n::t!("arrange.audio_normalize"))
                .clicked()
            {
                edit.audio_commands
                    .push(AudioEditCmd::Normalize { track, id: clip.id });
                ui.close();
            }
            ui.separator();
            if crate::widgets::flat::flat_button(ui, rust_i18n::t!("arrange.audio_gain_up"))
                .clicked()
            {
                edit.audio_commands.push(AudioEditCmd::GainDelta {
                    track,
                    id: clip.id,
                    delta_db: 3.0,
                });
                ui.close();
            }
            if ui
                .button(rust_i18n::t!("arrange.audio_gain_down"))
                .clicked()
            {
                edit.audio_commands.push(AudioEditCmd::GainDelta {
                    track,
                    id: clip.id,
                    delta_db: -3.0,
                });
                ui.close();
            }
            if ui
                .button(rust_i18n::t!("arrange.audio_gain_reset"))
                .clicked()
            {
                edit.audio_commands
                    .push(AudioEditCmd::GainReset { track, id: clip.id });
                ui.close();
            }
            ui.separator();
            if crate::widgets::flat::flat_button(ui, rust_i18n::t!("arrange.audio_delete"))
                .clicked()
            {
                edit.audio_commands.push(AudioEditCmd::Delete {
                    track,
                    ids: selected_ids.clone(),
                });
                ui.close();
            }
        });
    }

    // ── 左键按下：命中开始拖动 / 空白清除选择 ──
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    if !pressed {
        return false;
    }
    let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
        return false;
    };
    if !music_rect.contains(pos) {
        return false;
    }
    let alt_copy = ui.input(|i| i.modifiers.alt);

    if let Some((track, clip)) = hit_test(view, row_layout, data, pos, rect) {
        let zone = hit_zone(view, row_layout, data, pos, rect, track, &clip);
        // 选择：命中未选中片段时单选；已选中则保持多选（便于整体拖动）。
        let key = (track as u16, clip.id);
        if !edit.selected_audio_clips.contains(&key) {
            let add = ui.input(|i| i.modifiers.shift);
            if !add {
                edit.selected_audio_clips.clear();
            }
            edit.selected_audio_clips.insert(key);
        }
        let mode = match zone {
            HitZone::Body => DragMode::Move,
            HitZone::LeftEdge => DragMode::TrimStart,
            HitZone::RightEdge => DragMode::TrimEnd,
            HitZone::FadeInHandle => DragMode::FadeIn,
            HitZone::FadeOutHandle => DragMode::FadeOut,
        };
        ui.data_mut(|d| {
            d.insert_temp(
                drag_id(),
                DragState {
                    track,
                    id: clip.id,
                    mode,
                    start_clip: clip,
                    alt_copy,
                },
            )
        });
        true
    } else {
        // 点空白：清除音频选择（不消费，让框选正常开始）。
        edit.selected_audio_clips.clear();
        false
    }
}

/// 已选片段 id 列表（命中片段不在选中集时只返回它自己）。
fn selected_ids_for(edit: &ArrangeEdit<'_>, track: usize, hit_id: u32) -> Vec<u32> {
    let ids: Vec<u32> = edit
        .selected_audio_clips
        .iter()
        .filter(|(t, _)| *t as usize == track)
        .map(|(_, id)| *id)
        .collect();
    if ids.is_empty() { vec![hit_id] } else { ids }
}

/// 拖动松手：把 ghost 位置换算成命令。
fn finish_drag(
    state: &DragState,
    pos: egui::Pos2,
    rect: egui::Rect,
    view: &ArrangementView,
    data: &ArrangeData<'_>,
) -> Option<AudioEditCmd> {
    let c = &state.start_clip;
    match state.mode {
        DragMode::Move => {
            let delta = pointer_seconds(pos.x - rect.min.x, view, data) - c.start_seconds;
            if delta.abs() < 1e-9 {
                return None;
            }
            if state.alt_copy {
                Some(AudioEditCmd::Duplicate {
                    track: state.track,
                    ids: vec![state.id],
                    delta_seconds: delta,
                })
            } else {
                Some(AudioEditCmd::Move {
                    track: state.track,
                    ids: vec![state.id],
                    delta_seconds: delta,
                })
            }
        }
        DragMode::TrimStart => Some(AudioEditCmd::TrimStart {
            track: state.track,
            id: state.id,
            new_start_seconds: pointer_seconds(pos.x - rect.min.x, view, data),
        }),
        DragMode::TrimEnd => Some(AudioEditCmd::TrimEnd {
            track: state.track,
            id: state.id,
            new_end_seconds: pointer_seconds(pos.x - rect.min.x, view, data),
        }),
        DragMode::FadeIn | DragMode::FadeOut => {
            let (fi, fo) = fade_from_pointer(state, pos, rect, view, data)?;
            Some(AudioEditCmd::SetFades {
                track: state.track,
                id: state.id,
                fade_in_seconds: fi,
                fade_out_seconds: fo,
            })
        }
    }
}

/// 视图本地 x → 时间线秒（吸附网格）。
fn pointer_seconds(local_x: f32, view: &ArrangementView, data: &ArrangeData<'_>) -> f64 {
    let tick = view.x_to_tick(local_x).max(0.0);
    let snapped =
        crate::view_interaction::snap_tick(tick, data.quantize, data.ppq, data.bar_line_data);
    data.tempo_map.tick_to_seconds(snapped.max(0.0) as u64)
}

/// 淡入/淡出拖动：返回新的 (fade_in, fade_out)。
fn fade_from_pointer(
    state: &DragState,
    pos: egui::Pos2,
    rect: egui::Rect,
    view: &ArrangementView,
    data: &ArrangeData<'_>,
) -> Option<(f64, f64)> {
    let c = &state.start_clip;
    let local_x = pos.x - rect.min.x;
    let x0 = view.tick_to_x(data.tempo_map.tick_at_time(c.start_seconds.max(0.0)));
    let x1 = view.tick_to_x(data.tempo_map.tick_at_time(c.end_seconds().max(0.0)));
    let width = (x1 - x0).max(1.0);
    match state.mode {
        DragMode::FadeIn => {
            let frac = ((local_x - x0) / width).clamp(0.0, 1.0) as f64;
            Some((frac * c.duration_seconds, c.fade_out_seconds))
        }
        DragMode::FadeOut => {
            let frac = ((x1 - local_x) / width).clamp(0.0, 1.0) as f64;
            Some((c.fade_in_seconds, frac * c.duration_seconds))
        }
        _ => None,
    }
}

/// 画拖动 ghost（目标位置轮廓 + 淡入淡出示意）。
fn draw_drag_ghost(
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &ArrangeData<'_>,
    state: &DragState,
    pos: egui::Pos2,
) {
    let painter = ui.painter();
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;
    let c = &state.start_clip;
    let y_top = rect.min.y + row_layout.track_y(state.track, lh) - scroll_y;
    let h = row_layout.track_height(state.track, lh);
    let track_color = data
        .track_colors
        .get(state.track)
        .copied()
        .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
    let color = egui::Color32::from_rgba_unmultiplied(
        (track_color[0].clamp(0.0, 1.0) * 255.0) as u8,
        (track_color[1].clamp(0.0, 1.0) * 255.0) as u8,
        (track_color[2].clamp(0.0, 1.0) * 255.0) as u8,
        GHOST_ALPHA,
    );
    let stroke = egui::Stroke::new(1.5, crate::theme::contrast_fg());
    let local_x = pos.x - rect.min.x;
    let x0 = view.tick_to_x(data.tempo_map.tick_at_time(c.start_seconds.max(0.0)));
    let x1 = view.tick_to_x(data.tempo_map.tick_at_time(c.end_seconds().max(0.0)));

    match state.mode {
        DragMode::Move => {
            let delta = pointer_seconds(local_x, view, data) - c.start_seconds;
            let new_start = (c.start_seconds + delta).max(0.0);
            let new_end = new_start + c.duration_seconds;
            let nx0 = view.tick_to_x(data.tempo_map.tick_at_time(new_start));
            let nx1 = view.tick_to_x(data.tempo_map.tick_at_time(new_end));
            let r = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + nx0, y_top + 1.0),
                egui::pos2(rect.min.x + nx1.max(nx0 + 2.0), y_top + h - 1.0),
            );
            painter.rect_filled(r, 2.0, color);
            painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Inside);
        }
        DragMode::TrimStart => {
            let new_start = pointer_seconds(local_x, view, data)
                .clamp(c.start_seconds - c.offset_seconds, c.end_seconds() - 0.001);
            let x = rect.min.x + view.tick_to_x(data.tempo_map.tick_at_time(new_start.max(0.0)));
            painter.line_segment([egui::pos2(x, y_top), egui::pos2(x, y_top + h)], stroke);
        }
        DragMode::TrimEnd => {
            let new_end = pointer_seconds(local_x, view, data).max(c.start_seconds + 0.001);
            let x = rect.min.x + view.tick_to_x(data.tempo_map.tick_at_time(new_end.max(0.0)));
            painter.line_segment([egui::pos2(x, y_top), egui::pos2(x, y_top + h)], stroke);
        }
        DragMode::FadeIn | DragMode::FadeOut => {
            if let Some((fi, fo)) = fade_from_pointer(state, pos, rect, view, data) {
                let w = (x1 - x0).max(1.0);
                if fi > 0.0 {
                    let fx = rect.min.x + x0 + (fi / c.duration_seconds) as f32 * w;
                    painter.line_segment(
                        [
                            egui::pos2(rect.min.x + x0, y_top + h),
                            egui::pos2(fx, y_top),
                        ],
                        stroke,
                    );
                }
                if fo > 0.0 {
                    let fx = rect.min.x + x1 - (fo / c.duration_seconds) as f32 * w;
                    painter.line_segment(
                        [
                            egui::pos2(fx, y_top),
                            egui::pos2(rect.min.x + x1, y_top + h),
                        ],
                        stroke,
                    );
                }
            }
        }
    }
}

/// 命中测试：返回 (轨道索引, 片段克隆)。
fn hit_test(
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &ArrangeData<'_>,
    pos: egui::Pos2,
    rect: egui::Rect,
) -> Option<(usize, AudioClip)> {
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;
    let my = pos.y - rect.min.y + scroll_y;
    let track = match row_layout.hit_at_music_y(my, lh) {
        Some(yinhe_types::ArRow::Track(t)) => t,
        _ => return None,
    };
    let track_data = data.tracks.get(track)?;
    if track_data.kind != TrackKind::Audio {
        return None;
    }
    let y_top = rect.min.y + row_layout.track_y(track, lh) - scroll_y;
    let h = row_layout.track_height(track, lh);
    if pos.y < y_top || pos.y > y_top + h {
        return None;
    }
    let local_x = pos.x - rect.min.x;
    for clip in &track_data.audio_clips {
        let x0 = view.tick_to_x(data.tempo_map.tick_at_time(clip.start_seconds.max(0.0)));
        let x1 = view.tick_to_x(data.tempo_map.tick_at_time(clip.end_seconds().max(0.0)));
        if local_x >= x0 - EDGE_W && local_x <= x1 + EDGE_W {
            return Some((track, clip.clone()));
        }
    }
    None
}

/// 细分命中区（边缘/淡入淡出手柄/主体）。
fn hit_zone(
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &ArrangeData<'_>,
    pos: egui::Pos2,
    rect: egui::Rect,
    track: usize,
    clip: &AudioClip,
) -> HitZone {
    let local_x = pos.x - rect.min.x;
    let x0 = view.tick_to_x(data.tempo_map.tick_at_time(clip.start_seconds.max(0.0)));
    let x1 = view.tick_to_x(data.tempo_map.tick_at_time(clip.end_seconds().max(0.0)));
    // 上角手柄优先（淡入/淡出）。
    let lh = view.lane_height();
    let y_top = rect.min.y + row_layout.track_y(track, lh) - view.base.scroll_y;
    if pos.y < y_top + FADE_HANDLE + 2.0 {
        if (local_x - x0).abs() <= FADE_HANDLE {
            return HitZone::FadeInHandle;
        }
        if (local_x - x1).abs() <= FADE_HANDLE {
            return HitZone::FadeOutHandle;
        }
    }
    if (local_x - x0).abs() <= EDGE_W {
        HitZone::LeftEdge
    } else if (local_x - x1).abs() <= EDGE_W {
        HitZone::RightEdge
    } else {
        HitZone::Body
    }
}
