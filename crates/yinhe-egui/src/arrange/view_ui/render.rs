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
