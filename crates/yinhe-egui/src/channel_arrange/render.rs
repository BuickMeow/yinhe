//! 通道 AR 的 GPU 数据准备与 egui 层绘制。

use eframe::egui;

use std::collections::HashSet;

use yinhe_types::ArrangementView;
use yinhe_wgpu::{InstanceRenderer, layer_cache_key};

use super::groups::CaGroups;
use super::rows::{CaRow, CaRowLayout, note_rows_by_track};
use super::types::layout_hash;
/// 每轨音符的「可见行」映射：对全量映射按可视行范围过滤（行 id 全局不变）。
pub(crate) fn build_track_rows(
    layout: &CaRowLayout,
    groups: &CaGroups,
    visible: std::ops::Range<usize>,
) -> Vec<Vec<u16>> {
    let from = visible.start as u16;
    let to = visible.end as u16;
    note_rows_by_track(layout, groups)
        .into_iter()
        .map(|rows| rows.into_iter().filter(|&r| r >= from && r < to).collect())
        .collect()
}

/// 绘制行条纹背景（偶数行着色，与轨道 AR 同风格）。
pub(crate) fn draw_lane_stripes(
    painter: &egui::Painter,
    rect: egui::Rect,
    layout: &CaRowLayout,
    visible: std::ops::Range<usize>,
    scroll_y: f32,
    left_w: f32,
) {
    painter.rect_filled(rect, 0.0, crate::theme::app_bg());
    let stripe = crate::theme::stripe_bg();
    for row in visible {
        // 全局行号奇偶（与原 AR 一致：偶数行条纹）。
        if row % 2 != 0 {
            continue;
        }
        let y = rect.min.y + layout.y_top(row) - scroll_y;
        let h = layout.height(row);
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.min.x + left_w, y),
                egui::vec2(rect.width() - left_w, h),
            ),
            0.0,
            stripe,
        );
    }
}

/// 绘制网格线（egui 层，复用轨道 AR 的 arrangement 配色）。
pub(crate) fn draw_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &ArrangementView,
    data: &crate::arrange::ArrangeData<'_>,
) {
    let Some(midi) = data.midi else {
        return;
    };
    let Some(tpb) = midi.ticks_per_beat() else {
        return;
    };
    let (def_num, def_den) = midi.time_sig_default();
    let sig_events = midi.time_sig_events();
    let grid_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + view.base.left_panel_width, rect.min.y),
        rect.max,
    );
    crate::widgets::grid_lines::paint_grid_lines(
        painter,
        grid_rect,
        &view.base,
        tpb,
        def_num,
        def_den,
        sig_events,
        &crate::widgets::grid_lines::GridColors::arrangement(),
        yinhe_types::Orientation::Horizontal,
    );
}

/// 上传音符层与 ghost 层。
#[allow(clippy::too_many_arguments)]
pub(crate) fn upload_note_layers(
    renderer: &mut InstanceRenderer,
    view: &ArrangementView,
    layout: &CaRowLayout,
    track_rows: &[Vec<u16>],
    data: &crate::arrange::ArrangeData<'_>,
    hidden_notes: &HashSet<(u16, u32, u8)>,
    ghost_notes: &mut [super::types::CaGhostNote],
    w: f32,
    vh: u64,
    wh: u64,
    tv_hash: u64,
    revision: u64,
) {
    let notes_key = layer_cache_key(&[
        vh,
        wh,
        tv_hash,
        layout_hash(layout),
        revision,
        hidden_notes.len() as u64,
    ]);
    renderer.upload_note_layer(0, notes_key, |out| {
        if let Some(midi) = data.midi {
            yinhe_wgpu::build_channel_notes(
                out,
                w,
                midi,
                view,
                track_rows,
                data.track_visible,
                hidden_notes,
            );
        }
    });
    renderer.upload_note_layer(1, 0, |out| {
        yinhe_wgpu::build_channel_ghost_notes(out, ghost_notes, w, view);
    });
}

/// 收集可视范围内的 AM 行（自动化子行 + Conductor 主行）。
pub(crate) fn collect_am_rows(
    layout: &CaRowLayout,
    visible: std::ops::Range<usize>,
    conductor_track_idx: Option<u16>,
) -> Vec<crate::arrange::am_lanes::AmRowRef> {
    let mut out = Vec::new();
    for row in visible {
        let y_top = layout.y_top(row);
        let height = layout.height(row);
        match layout.row(row) {
            Some(CaRow::Automation { track, sub }) => {
                out.push(crate::arrange::am_lanes::AmRowRef {
                    track: track as usize,
                    sub: Some(sub),
                    y_top,
                    height,
                })
            }
            Some(CaRow::Conductor) => {
                if let Some(c) = conductor_track_idx {
                    out.push(crate::arrange::am_lanes::AmRowRef {
                        track: c as usize,
                        sub: None,
                        y_top,
                        height,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// 通道 AR 折叠通道的自动化预览：未展开的通道把其全部轨道的全部 lane 以
/// 该通道行的 y 顶、半透明叠加显示（与轨道 AR 的折叠预览同语义）。
pub(crate) fn collect_folded_am<'a>(
    layout: &CaRowLayout,
    groups: &CaGroups,
    visible: std::ops::Range<usize>,
    data: &crate::arrange::ArrangeData<'a>,
) -> Vec<yinhe_wgpu::ArrAutomationLane<'a>> {
    let mut folded = Vec::new();
    for row in visible {
        let Some(CaRow::Channel(ch)) = layout.row(row) else {
            continue;
        };
        let Some(group) = groups.group(ch) else {
            continue;
        };
        let y_top = layout.y_top(row);
        let height = layout.height(row);
        for &t in &group.tracks {
            if !data.track_visible.get(t as usize).copied().unwrap_or(true) {
                continue;
            }
            let Some(tr) = data.tracks.get(t as usize) else {
                continue;
            };
            for lane in &tr.automation_lanes {
                folded.push(yinhe_wgpu::ArrAutomationLane {
                    lane,
                    y_top,
                    height,
                    max_val: crate::arrange::am_lanes::lane_max_val(lane),
                    highlight_ticks: &[],
                    show_anchors: false,
                    translucent: true,
                });
            }
        }
    }
    folded
}

/// 绘制音频通道行上的片段（背景 + 波形 + 淡入淡出 + 素材名 + 选中高亮）。
///
/// 仅音频轨的「轨道行」显示片段；通道合并行不重复绘制片段
/// （音频通道一般只含一条轨道，轨道行在展开后必然存在）。
pub(crate) fn draw_audio_clips(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &ArrangementView,
    layout: &CaRowLayout,
    data: &crate::arrange::ArrangeData<'_>,
    selected: &std::collections::HashSet<(u16, u32)>,
) {
    let scroll_y = view.base.scroll_y;
    let map = layout.track_row_map();
    for (track_idx, track) in data.tracks.iter().enumerate() {
        if track.kind != yinhe_core::TrackKind::Audio || track.audio_clips.is_empty() {
            continue;
        }
        let ti = track_idx as u16;
        if !data.track_visible.get(track_idx).copied().unwrap_or(true) {
            continue;
        }
        let Some(&row) = map.get(&ti) else {
            continue;
        };
        let y_top = rect.min.y + layout.y_top(row) - scroll_y;
        let h = layout.height(row);
        if y_top >= rect.max.y || y_top + h <= rect.min.y {
            continue;
        }
        let tc = data
            .track_colors
            .get(track_idx)
            .copied()
            .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
        let track_color = crate::arrange::view_ui::render::rgb(tc);
        for (ci, clip) in track.audio_clips.iter().enumerate() {
            if clip.duration_seconds <= 0.0 {
                continue;
            }
            let start_tick = data.tempo_map.tick_at_time(clip.start_seconds.max(0.0));
            let end_tick = data.tempo_map.tick_at_time(clip.end_seconds().max(0.0));
            let x0 = rect.min.x + view.tick_to_x(start_tick);
            let x1 = rect.min.x + view.tick_to_x(end_tick);
            if x1 < rect.min.x || x0 > rect.max.x {
                continue;
            }
            let r = egui::Rect::from_min_max(
                egui::pos2(x0, y_top + 1.0),
                egui::pos2(x1.max(x0 + 2.0), y_top + h - 1.0),
            );
            let selected_this = selected.contains(&(ti, clip.id));
            let fill = egui::Color32::from_rgba_unmultiplied(
                track_color.r(),
                track_color.g(),
                track_color.b(),
                if selected_this { 150 } else { 96 },
            );
            painter.rect_filled(r, 2.0, fill);
            let stroke = if selected_this {
                egui::Stroke::new(2.0, crate::theme::contrast_fg())
            } else {
                egui::Stroke::new(1.0, track_color.gamma_multiply(1.2))
            };
            painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Inside);

            crate::arrange::view_ui::render::draw_clip_waveform(
                painter,
                rect,
                r,
                x0,
                x1,
                clip,
                data.audio_library,
                track_color,
            );

            let (fade_in, fade_out) = yinhe_audio::effective_fades(&track.audio_clips, ci);
            let fade_stroke = egui::Stroke::new(1.0, track_color.gamma_multiply(1.6));
            if fade_in > 0.0 {
                let fx = r.min.x + (fade_in / clip.duration_seconds) as f32 * r.width();
                painter.line_segment(
                    [egui::pos2(r.min.x, r.max.y), egui::pos2(fx, r.min.y)],
                    fade_stroke,
                );
            }
            if fade_out > 0.0 {
                let fx = r.max.x - (fade_out / clip.duration_seconds) as f32 * r.width();
                painter.line_segment(
                    [egui::pos2(fx, r.min.y), egui::pos2(r.max.x, r.max.y)],
                    fade_stroke,
                );
            }

            if r.width() > 40.0 && r.height() > 14.0 {
                let name = data
                    .audio_sources
                    .iter()
                    .find(|s| s.uuid == clip.source)
                    .map(|s| s.name.as_str())
                    .unwrap_or("");
                let text_rect = r.shrink2(egui::vec2(4.0, 2.0));
                painter.text(
                    text_rect.min,
                    egui::Align2::LEFT_TOP,
                    name,
                    egui::FontId::new(
                        crate::theme::SMALL_FONT.min(r.height() - 4.0),
                        egui::FontFamily::Proportional,
                    ),
                    crate::theme::contrast_fg(),
                );
            }
            if data.audio_library.get(&clip.source).is_none()
                && r.width() > 40.0
                && data.audio_library.is_pending(&clip.source)
            {
                painter.text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    rust_i18n::t!("arrange.audio_decoding"),
                    egui::FontId::new(
                        crate::theme::SMALL_FONT.min(r.height() - 4.0),
                        egui::FontFamily::Proportional,
                    ),
                    crate::theme::text_muted(),
                );
            }
        }
    }
}
