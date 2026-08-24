mod interaction;
mod render;
mod types;

use std::collections::HashSet;

use eframe::egui;

use yinhe_types::{ArRowLayout, ArrangementView};
use yinhe_wgpu::{InstanceRenderer, MAX_TRACKS, Uniforms};

use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut InstanceRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    row_layout: &ArRowLayout,
    data: super::ArrangeData<'_>,
    edit: &mut super::ArrangeEdit<'_>,
    cfg: &mut super::ArrangeViewCfg<'_>,
) {
    let _arrange_total_start = if yinhe_memtrace::perf_probe::enabled() {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let full_rect = ui.max_rect();
    let lp = view.base.left_panel_width;
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(full_rect.min.x + lp, full_rect.min.y),
        full_rect.max,
    );
    ui.set_clip_rect(ui.clip_rect().intersect(music_rect));
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;
    let ppp = ui.ctx().pixels_per_point();
    let w = rect.width() as u32;
    let h = rect.height() as u32;
    let pw = (w as f32 * ppp) as u32;
    let ph = (h as f32 * ppp) as u32;
    if w == 0 || h == 0 {
        return;
    }
    render_ctx.ensure_size(pw, ph);
    view.clamp_scroll(
        w as f32,
        h as f32,
        data.total_ticks,
        row_layout.total_rows(),
    );
    let follow_active =
        cfg.is_playing && *cfg.follow_mode != crate::view_interaction::FollowMode::None;
    if !follow_active {
        view.base.follow_target = None;
        view.base.follow_anim_elapsed = 0.0;
    } else if let Some(ct) = *edit.cursor_tick {
        let dt = ui.input(|i| i.stable_dt).max(1e-4);
        use crate::view_interaction::FollowMode;
        if *cfg.follow_mode == FollowMode::Centered || *cfg.follow_mode == FollowMode::Continuous {
            if let Some(t) = crate::view_interaction::compute_follow_scroll(
                ct,
                view.base.pixels_per_tick,
                w as f32,
                0.0,
                *cfg.follow_mode,
                0.0,
                view.base.scroll_x,
            ) {
                view.base.scroll_x = t;
                view.clamp_scroll(
                    w as f32,
                    h as f32,
                    data.total_ticks,
                    row_layout.total_rows(),
                );
                view.base.follow_target = None;
                view.base.follow_anim_elapsed = 0.0;
            }
        } else if *cfg.follow_mode == FollowMode::Page {
            if let Some(t) = crate::view_interaction::compute_follow_scroll(
                ct,
                view.base.pixels_per_tick,
                w as f32,
                0.0,
                *cfg.follow_mode,
                0.0,
                view.base.scroll_x,
            ) {
                let need_restart = view.base.follow_target != Some(t);
                if need_restart {
                    view.base.follow_anim_start = view.base.scroll_x;
                    view.base.follow_anim_elapsed = 0.0;
                    view.base.follow_target = Some(t);
                }
            }
            if let Some(target) = view.base.follow_target {
                view.base.follow_anim_elapsed += dt;
                view.base.scroll_x = crate::view_interaction::follow_page_lerp(
                    view.base.follow_anim_start,
                    target,
                    view.base.follow_anim_elapsed,
                    crate::view_interaction::FOLLOW_PAGE_DURATION,
                );
                view.clamp_scroll(
                    w as f32,
                    h as f32,
                    data.total_ticks,
                    row_layout.total_rows(),
                );
                let done = view.base.follow_anim_elapsed
                    >= crate::view_interaction::FOLLOW_PAGE_DURATION
                    || (target - view.base.scroll_x).abs() <= 1.0;
                if done {
                    view.base.scroll_x = target;
                    view.clamp_scroll(
                        w as f32,
                        h as f32,
                        data.total_ticks,
                        row_layout.total_rows(),
                    );
                    view.base.follow_target = None;
                    view.base.follow_anim_elapsed = 0.0;
                }
            }
        }
    }
    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = match cfg.scroll_mode {
        0 => (scroll_x, 0.0),
        _ => {
            let f = scroll_x.floor();
            (f, scroll_x - f)
        }
    };
    let track_count = data.track_colors.len().min(MAX_TRACKS) as u32;
    let tc_colors: Vec<[f32; 4]> = data.track_colors.iter().take(MAX_TRACKS).copied().collect();
    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: scroll_x_pos,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0,
        keyboard_width: view.base.left_panel_width,
        mode: 2,
        scroll_frac,
        scroll_mode: cfg.scroll_mode,
        min_border_width: cfg.min_border_width,
        track_count,
        sel_rect_count: 0,
        note_outline: 1,
        lane_height: view.lane_height(),
        value_zoom: 0.0,
        value_scroll: 0.0,
        orientation: 0,
    };
    view.base.dirty = false;
    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(&tc_colors);
    renderer.ensure_layers(2);
    let (mut ghost_notes, hidden_notes, drag_rect) =
        if *cfg.active_tool == Tool::Select || *cfg.active_tool == Tool::SelectVertical {
            let vertical = *cfg.active_tool == Tool::SelectVertical;
            interaction::sel_drag_frame_arrange(
                ui, rect, music_rect, view, row_layout, &data, edit, vertical,
            )
        } else {
            (Vec::new(), HashSet::new(), None)
        };
    let vh = view.render_hash();
    let wh = {
        let mut hash: u64 = 0;
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(w as u64);
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(h as u64);
        hash
    };
    let tv_hash = {
        let mut h = 0u64;
        for &v in data.track_visible {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
        }
        h
    };
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;
    let first_row = ((scroll_y / lh).floor().max(0.0) as usize).min(row_layout.total_rows());
    let last_row =
        (((scroll_y + h as f32) / lh).ceil().max(0.0) as usize).min(row_layout.total_rows());
    let track_range = row_layout.visible_track_range(scroll_y, h as f32, lh);
    renderer.upload_track_offsets(&row_layout.track_offsets(lh));
    let offsets_hash = row_layout.track_offsets(lh).iter().fold(0u64, |acc, &o| {
        acc.wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add(o.to_bits() as u64)
    });
    render::upload_note_layers(
        renderer,
        view,
        row_layout,
        &data,
        &hidden_notes,
        &mut ghost_notes,
        w as f32,
        h as f32,
        vh,
        wh,
        tv_hash,
        offsets_hash,
        cfg.revision,
        track_range,
    );
    let am_rows =
        super::am_lanes::visible_am_rows(row_layout, first_row, last_row, data.conductor_track_idx);
    let mut am_ghost: Option<(yinhe_wgpu::AutomationGhost, f32, f32, f32)> = None;
    let mut am_marquee: Option<egui::Rect> = None;
    if !am_rows.is_empty() {
        let am_ctx = crate::piano_view::automation_panel::AutomationEditCtx {
            active_tool: *cfg.active_tool,
            active_track: None,
            quantize: data.quantize,
            ppq: data.ppq,
            bar_line_data: data.bar_line_data,
        };
        let mut io = super::am_lanes::AmLanesIo {
            tracks: data.tracks,
            tempo_lane: data.tempo_lane,
            track_colors: data.track_colors,
            selected: &mut *edit.selected,
            info_content: &mut *edit.info_content,
            right_tab: &mut *edit.right_tab,
            am_views: &mut *edit.arr_am_views,
            edits: &mut *edit.am_edits,
        };
        let out =
            super::am_lanes::interact_all(ui, &am_rows, view, rect, music_rect, &am_ctx, &mut io);
        am_ghost = out.ghost;
        am_marquee = out.marquee;
    }
    render::prepare_automation(
        renderer,
        view,
        row_layout,
        &data,
        edit,
        &am_rows,
        am_ghost,
        w as f32,
        h as f32,
        vh,
        wh,
        tv_hash,
        offsets_hash,
        cfg.revision,
        *cfg.active_tool,
    );
    let content_changed = true;
    render::draw_track_lanes(
        &painter,
        rect,
        view,
        row_layout,
        data.track_visible,
        first_row,
        last_row,
        lh,
        scroll_y,
    );
    render::draw_grid(&painter, rect, view, row_layout, &data);
    render_ctx.paint(
        renderer,
        pw,
        ph,
        "arrangement_frame",
        &painter,
        rect,
        content_changed,
    );
    if let Some(ct) = *edit.cursor_tick {
        let lb_w = view.base.left_panel_width;
        let cx_local = view.tick_to_x(ct);
        if cx_local >= lb_w && cx_local <= w as f32 {
            let cx = rect.min.x + cx_local;
            painter.line_segment(
                [egui::pos2(cx, rect.min.y), egui::pos2(cx, rect.max.y)],
                egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::contrast_fg()),
            );
        }
    }
    if let Some(dr) = drag_rect {
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            dr,
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
        );
    }
    if *cfg.active_tool == Tool::Eraser {
        interaction::eraser_drag_frame_arrange(ui, rect, music_rect, view, row_layout, &data, edit);
    }
    for &(t_start, t_end, track_lo, track_hi) in edit.arr_sel_rect.iter() {
        let view_sy = row_layout.track_y(track_lo, lh) - scroll_y;
        let view_ey =
            row_layout.track_y(track_hi, lh) + row_layout.track_height(track_hi, lh) - scroll_y;
        let view_sx = view.tick_to_x(t_start);
        let view_ex = view.tick_to_x(t_end);
        let snapped = egui::Rect::from_min_max(
            egui::pos2(view_sx.min(view_ex), view_sy.min(view_ey)),
            egui::pos2(view_sx.max(view_ex), view_sy.max(view_ey)),
        );
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            snapped,
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
        );
    }
    if *cfg.active_tool == Tool::Eraser {
        let drag_id = ui.id().with("eraser_drag_arr");
        let drag: Option<((f64, f32), egui::Pos2)> =
            ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);
        if let Some((start_music, end)) = drag {
            let start_pixel = egui::pos2(
                view.tick_to_x(start_music.0),
                start_music.1 * view.lane_height() - view.base.scroll_y,
            );
            if (end - start_pixel).length() >= 3.0
                && let Some(b) =
                    types::arrange_snapped_bounds(start_pixel, end, view, row_layout, &data, false)
            {
                let snapped = egui::Rect::from_min_max(
                    egui::pos2(b.view_sx.min(b.view_ex), b.view_sy.min(b.view_ey)),
                    egui::pos2(b.view_sx.max(b.view_ex), b.view_sy.max(b.view_ey)),
                );
                crate::selection::draw::draw(
                    ui.painter(),
                    rect,
                    snapped,
                    crate::theme::danger_text_bright(),
                    crate::theme::danger_text_bright(),
                );
            }
        }
    }
    if let Some(mr) = am_marquee {
        let col = if *cfg.active_tool == Tool::Eraser {
            crate::theme::danger_text_bright()
        } else {
            crate::theme::contrast_fg()
        };
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            mr.translate(-rect.min.to_vec2()),
            col,
            col,
        );
    }
    crate::view_interaction::handle_input(
        ui,
        rect,
        view,
        edit.cursor_tick,
        0.0,
        Some((data.quantize, data.ppq)),
        data.bar_line_data,
        None,
        Some(music_rect),
        cfg.is_playing,
        cfg.follow_mode,
        cfg.active_tool,
    );
    view.clamp_scroll(
        w as f32,
        h as f32,
        data.total_ticks,
        row_layout.total_rows(),
    );
    if let Some(t0) = _arrange_total_start {
        yinhe_memtrace::perf_probe::record_arrange_total(t0.elapsed());
    }
}
