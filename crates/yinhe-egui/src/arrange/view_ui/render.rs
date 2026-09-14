use eframe::egui;

use std::collections::HashSet;

use yinhe_types::{ArRowLayout, ArrangementView};
use yinhe_wgpu::{InstanceRenderer, layer_cache_key};
use yinhe_wgpu::{build_arr_notes, build_ghost_notes};

use crate::piano_view::drag::{GhostNote, HiddenNote};

/// 绘制轨道条纹背景。
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_track_lanes(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    track_visible: &[bool],
    first_row: usize,
    last_row: usize,
    lh: f32,
    scroll_y: f32,
) {
    painter.rect_filled(rect, 0.0, crate::theme::app_bg());
    let lb_w = view.base.left_panel_width;
    let w = rect.width();
    for row in first_row..last_row {
        let Some(hit) = row_layout.row_hit(row) else {
            continue;
        };
        let track = hit.track();
        if !track_visible.get(track).copied().unwrap_or(true) {
            continue;
        }
        if row % 2 != 0 {
            continue;
        }
        let y = rect.min.y + row as f32 * lh - scroll_y;
        let col = crate::theme::stripe_bg();
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(rect.min.x + lb_w, y), egui::vec2(w - lb_w, lh)),
            0.0,
            col,
        );
    }
}

/// 绘制网格线（egui 层，替代原 wgpu grid layer）。
pub(crate) fn draw_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
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
    // 避免未使用警告：row_layout 仅用于保持签名一致，实际网格与行无关
    let _ = row_layout;
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
    row_layout: &ArRowLayout,
    data: &crate::arrange::ArrangeData<'_>,
    hidden_notes: &HashSet<HiddenNote>,
    ghost_notes: &mut [GhostNote],
    w: f32,
    h: f32,
    vh: u64,
    wh: u64,
    tv_hash: u64,
    offsets_hash: u64,
    revision: u64,
    track_range: (usize, usize),
) {
    let _ = h;
    let _ = row_layout;
    let notes_key = layer_cache_key(&[
        vh,
        wh,
        tv_hash,
        offsets_hash,
        revision,
        hidden_notes.len() as u64,
    ]);
    renderer.upload_note_layer(0, notes_key, |out| {
        if let Some(midi) = data.midi {
            build_arr_notes(
                out,
                w,
                midi,
                view,
                track_range,
                data.track_visible,
                hidden_notes,
            );
        }
    });
    renderer.upload_note_layer(1, 0, |out| {
        build_ghost_notes(out, ghost_notes, w, view, track_range, data.track_visible);
    });
}

/// 绘制音频轨道片段：块背景 + 波形 + 淡入淡出斜线 + 素材名 + 选中高亮。
///
/// 在 GPU 音符层贴图之后调用（音频轨没有音符，不与音符层冲突）。
pub(crate) fn draw_audio_clips(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &crate::arrange::ArrangeData<'_>,
    selected: &std::collections::HashSet<(u16, u32)>,
) {
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;
    for (track_idx, track) in data.tracks.iter().enumerate() {
        if track.kind != yinhe_core::TrackKind::Audio || track.audio_clips.is_empty() {
            continue;
        }
        if !data.track_visible.get(track_idx).copied().unwrap_or(true) {
            continue;
        }
        let y_top = rect.min.y + row_layout.track_y(track_idx, lh) - scroll_y;
        let h = row_layout.track_height(track_idx, lh);
        if y_top >= rect.max.y || y_top + h <= rect.min.y {
            continue;
        }
        let tc = data
            .track_colors
            .get(track_idx)
            .copied()
            .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
        let track_color = rgb(tc);
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
            // 最小 2px 可见宽度（极窄片段仍可点选）。
            let r = egui::Rect::from_min_max(
                egui::pos2(x0, y_top + 1.0),
                egui::pos2(x1.max(x0 + 2.0), y_top + h - 1.0),
            );
            let selected_this = selected.contains(&(track_idx as u16, clip.id));
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

            // 波形（可见部分）。
            draw_clip_waveform(
                painter,
                rect,
                r,
                x0,
                x1,
                clip,
                data.audio_library,
                track_color,
            );

            // 淡入淡出斜线（自身 + 自动交叉淡化，与引擎渲染一致）。
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

            // 素材名（宽度足够时）。
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
            // 解码中占位。
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

/// 片段波形：按可见像素逐列取峰值金字塔的 (min, max) 画竖线。
#[allow(clippy::too_many_arguments)] // 绘制上下文透传
fn draw_clip_waveform(
    painter: &egui::Painter,
    rect: egui::Rect,
    r: egui::Rect,
    x0: f32,
    x1: f32,
    clip: &yinhe_core::AudioClip,
    library: &crate::app::audio_library::AudioLibrary,
    color: egui::Color32,
) {
    let Some(decoded) = library.get(&clip.source) else {
        return;
    };
    if decoded.frames == 0 || x1 <= x0 {
        return;
    }
    let sr = decoded.sample_rate as f64;
    if sr <= 0.0 {
        return;
    }
    let vis_x0 = r.min.x.max(rect.min.x);
    let vis_x1 = r.max.x.min(rect.max.x);
    if vis_x1 <= vis_x0 {
        return;
    }
    let mid = r.center().y;
    let amp = (r.height() * 0.5 - 2.0).max(1.0);
    let frames_per_px = (clip.duration_seconds * sr) / (x1 - x0) as f64;
    let level = decoded.peaks.level_for(frames_per_px);
    let bucket_frames = decoded.peaks.bucket_frames(level).max(1) as f64;
    let stroke = egui::Stroke::new(1.0, color.gamma_multiply(1.8));
    let mut shapes: Vec<egui::Shape> =
        Vec::with_capacity(((vis_x1 - vis_x0) as usize).saturating_add(2));
    let mut px = vis_x0.floor();
    while px <= vis_x1 {
        // 该像素列在片段内的归一化位置 → 素材内秒（考虑 offset/反向）。
        let frac = ((px - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64;
        let t_in_clip = frac * clip.duration_seconds;
        let src_t = if clip.reversed {
            clip.offset_seconds + (clip.duration_seconds - t_in_clip)
        } else {
            clip.offset_seconds + t_in_clip
        };
        let frame = (src_t * sr).max(0.0);
        let bucket = (frame / bucket_frames) as usize;
        let (mn, mx) = decoded.peaks.bucket(level, bucket);
        let y0 = mid - mx.clamp(-1.0, 1.0) * amp;
        let y1 = mid - mn.clamp(-1.0, 1.0) * amp;
        shapes.push(egui::Shape::line_segment(
            [egui::pos2(px, y0), egui::pos2(px, y1)],
            stroke,
        ));
        px += 1.0;
    }
    painter.extend(shapes);
}

/// RGBA f32（0..1）→ Color32（不透明）。
fn rgb(c: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// 准备自动化曲线渲染层（layer 2 数据 + layer 3 ghost）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_automation(
    renderer: &mut InstanceRenderer,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &crate::arrange::ArrangeData<'_>,
    edit: &crate::arrange::ArrangeEdit<'_>,
    am_rows: &[crate::arrange::am_lanes::AmRowRef],
    am_ghost: Option<(yinhe_wgpu::AutomationGhost, f32, f32, f32)>,
    w: f32,
    h: f32,
    vh: u64,
    wh: u64,
    tv_hash: u64,
    offsets_hash: u64,
    revision: u64,
    active_tool: crate::widgets::tools_panel::Tool,
) {
    use crate::widgets::tools_panel::Tool;
    let show_anchors = matches!(
        active_tool,
        Tool::Pencil | Tool::Curve | Tool::Select | Tool::SelectVertical
    );
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;
    let mut am_render: Vec<yinhe_wgpu::ArrAutomationLane> = Vec::new();
    let mut am_highlights: Vec<Box<[u32]>> = Vec::new();
    for r in am_rows {
        let (lane, track) = match r.sub {
            Some(sub) => match data
                .tracks
                .get(r.track)
                .and_then(|t| t.automation_lanes.get(sub))
            {
                Some(l) => (l, r.track as u16),
                None => continue,
            },
            None => (data.tempo_lane, r.track as u16),
        };
        let key = (track, lane.target.clone());
        let sel_rects = edit
            .arr_am_views
            .get(&key)
            .map(|v| v.anchor_sel_rects.as_slice())
            .unwrap_or(&[]);
        am_highlights.push(crate::arrange::am_lanes::lane_highlight_ticks(
            lane,
            track,
            sel_rects,
            edit.info_content,
        ));
        am_render.push(yinhe_wgpu::ArrAutomationLane {
            lane,
            y_top: r.row as f32 * lh - scroll_y,
            height: lh,
            max_val: crate::arrange::am_lanes::lane_max_val(lane),
            highlight_ticks: &[],
        });
    }
    for (i, l) in am_render.iter_mut().enumerate() {
        l.highlight_ticks = &am_highlights[i];
    }
    let hl_hash = am_highlights.iter().fold(0u64, |acc, hl| {
        hl.iter()
            .fold(acc, |a, &tk| a.wrapping_mul(31).wrapping_add(tk as u64))
    });
    let am_key = layer_cache_key(&[
        vh,
        wh,
        tv_hash,
        offsets_hash,
        show_anchors as u64,
        revision,
        hl_hash,
    ]);
    // 避免未使用
    let _ = row_layout;
    yinhe_wgpu::prepare_arr_automation(
        renderer,
        w,
        h,
        &view.base,
        &am_render,
        data.track_visible,
        data.track_colors,
        show_anchors,
        am_ghost,
        am_key,
    );
}
