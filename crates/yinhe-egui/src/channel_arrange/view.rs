//! 通道 AR GPU 视图：一帧的渲染与输入组装。
//!
//! 与轨道 AR（`arrange::view_ui`）同构：纹理全宽、左侧面板列由 egui 层绘制、
//! 音符/自动化走 GPU 层。差异全部体现在行布局（`CaRowLayout`）与
//! 「行 id 直通 GPU」上。

use eframe::egui;

use std::collections::HashSet;

use yinhe_types::ArrangementView;
use yinhe_wgpu::{InstanceRenderer, MAX_TRACKS, Uniforms};

use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

use super::groups::CaGroups;
use super::rows::CaRowLayout;
use super::types::{layout_hash, row_colors, sel_view_y};

/// 通道 AR 视图/播放/渲染配置。
pub(crate) struct CaViewCfg<'a> {
    pub is_playing: bool,
    pub follow_mode: &'a mut crate::view_interaction::FollowMode,
    pub min_border_width: f32,
    pub revision: u64,
    /// Conductor 行颜色（随主题明暗派生）。
    pub conductor_color: [f32; 4],
}

/// 行布局上的 scroll clamp（内容总高 ≠ 行数 × 行高）。
fn clamp_scroll(
    view: &mut ArrangementView,
    width: f32,
    height: f32,
    total_ticks: f64,
    total_height: f32,
) {
    view.base.clamp_scroll_x(width, total_ticks);
    let max_scroll_y = (total_height - height).max(0.0);
    view.base.scroll_y = view.base.scroll_y.clamp(0.0, max_scroll_y);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut InstanceRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    layout: &CaRowLayout,
    groups: &CaGroups,
    data: &crate::arrange::ArrangeData<'_>,
    edit: &mut crate::arrange::ArrangeEdit<'_>,
    chan_ar_sel: &mut Vec<yinhe_editor_core::edit_state::ChanSelRect>,
    cfg: &mut CaViewCfg<'_>,
) {
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
    clamp_scroll(
        view,
        w as f32,
        h as f32,
        data.total_ticks,
        layout.total_height(),
    );

    // ── 播放跟随（水平）：与轨道 AR 同一策略 ──
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
                lp,
                *cfg.follow_mode,
                0.0,
                view.base.scroll_x,
            ) {
                view.base.scroll_x = t;
                clamp_scroll(
                    view,
                    w as f32,
                    h as f32,
                    data.total_ticks,
                    layout.total_height(),
                );
                view.base.follow_target = None;
                view.base.follow_anim_elapsed = 0.0;
            }
        } else if *cfg.follow_mode == FollowMode::Page {
            if let Some(t) = crate::view_interaction::compute_follow_scroll(
                ct,
                view.base.pixels_per_tick,
                w as f32,
                lp,
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
                clamp_scroll(
                    view,
                    w as f32,
                    h as f32,
                    data.total_ticks,
                    layout.total_height(),
                );
                let done = view.base.follow_anim_elapsed
                    >= crate::view_interaction::FOLLOW_PAGE_DURATION
                    || (target - view.base.scroll_x).abs() <= 1.0;
                if done {
                    view.base.scroll_x = target;
                    clamp_scroll(
                        view,
                        w as f32,
                        h as f32,
                        data.total_ticks,
                        layout.total_height(),
                    );
                    view.base.follow_target = None;
                    view.base.follow_anim_elapsed = 0.0;
                }
            }
        }
    }

    // ── 可见行与 GPU 表 ──
    let scroll_y = view.base.scroll_y;
    let visible = layout.visible_rows(scroll_y, h as f32);
    let track_rows = super::render::build_track_rows(layout, groups, visible.clone());
    let colors = row_colors(layout, groups, data.track_colors, cfg.conductor_color);
    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: view.base.scroll_x,
        scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0,
        keyboard_width: view.base.left_panel_width,
        mode: 2,
        min_border_width: cfg.min_border_width,
        track_count: colors.len().min(MAX_TRACKS) as u32,
        sel_rect_count: 0,
        // 通道 AR 音符不描边：填充即行原色（同轨道 AR）。
        note_outline: 0,
        lane_height: view.lane_height(),
        value_zoom: 0.0,
        value_scroll: 0.0,
        orientation: 0,
        dark_theme: u32::from(yinhe_theme::current_gpu_theme().dark_mode),
        ..Default::default()
    };
    view.base.dirty = false;
    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(&colors);
    renderer.upload_track_offsets(layout.row_offsets());
    renderer.upload_row_heights(layout.row_heights());
    renderer.ensure_layers(2);

    // ── 音频片段交互优先（消费指针则不开始框选）──
    let audio_consumed =
        crate::arrange::view_ui::audio_edit::frame(ui, rect, music_rect, view, layout, data, edit);

    // ── 选框/拖动/橡皮擦 ──
    let mut ghost_notes = Vec::new();
    let mut hidden_notes = HashSet::new();
    let mut drag_rect: Option<egui::Rect> = None;
    if !audio_consumed {
        let out = super::interaction::sel_drag_frame(
            ui,
            rect,
            music_rect,
            view,
            layout,
            groups,
            &track_rows,
            data,
            edit,
            chan_ar_sel,
        );
        ghost_notes = out.ghost_notes;
        hidden_notes = out.hidden_notes;
        drag_rect = out.drag_rect;
    }

    // ── 音符层 ──
    let vh = view.render_hash();
    let wh = {
        let mut hash: u64 = 0;
        hash = hash
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(w as u64);
        hash = hash
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(h as u64);
        hash
    };
    let tv_hash = yinhe_wgpu::hash_bools(data.track_visible);
    super::render::upload_note_layers(
        renderer,
        view,
        layout,
        &track_rows,
        data,
        &hidden_notes,
        &mut ghost_notes,
        w as f32,
        vh,
        wh,
        tv_hash,
        cfg.revision,
    );

    // ── 自动化交互（展开的自动化行 + Conductor Tempo）──
    let am_rows = super::render::collect_am_rows(layout, visible.clone(), data.conductor_track_idx);
    let mut am_ghost: Option<(yinhe_wgpu::AutomationGhost, f32, f32, f32)> = None;
    let mut am_marquee: Option<egui::Rect> = None;
    if !am_rows.is_empty() {
        let am_ctx = crate::piano_view::automation_panel::AutomationEditCtx {
            active_tool: Tool::Select,
            active_track: None,
            quantize: data.quantize,
            ppq: data.ppq,
            bar_line_data: data.bar_line_data,
        };
        let mut io = crate::arrange::am_lanes::AmLanesIo {
            tracks: data.tracks,
            tempo_lane: data.tempo_lane,
            track_colors: data.track_colors,
            selected: edit.selected,
            info_content: edit.info_content,
            right_tab: edit.right_tab,
            am_views: edit.arr_am_views,
            edits: edit.am_edits,
        };
        let out = crate::arrange::am_lanes::interact_all(
            ui, &am_rows, view, rect, music_rect, &am_ctx, &mut io,
        );
        am_ghost = out.ghost;
        am_marquee = out.marquee;
        if let Some(tick) = out.cursor_tick {
            *edit.cursor_tick = Some(tick as f64);
            view.base.dirty = true;
        }
    }

    // ── 自动化层（展开行 + 折叠通道预览）──
    let folded = super::render::collect_folded_am(layout, groups, visible.clone(), data);
    let offsets_hash = layout_hash(layout);
    crate::arrange::view_ui::render::prepare_automation(
        renderer,
        view,
        data,
        edit,
        &am_rows,
        &folded,
        am_ghost,
        w as f32,
        h as f32,
        vh,
        wh,
        tv_hash,
        offsets_hash,
        cfg.revision,
        Tool::Select,
    );

    // ── 绘制 ──
    super::render::draw_lane_stripes(&painter, rect, layout, visible.clone(), scroll_y, lp);
    super::render::draw_grid(&painter, rect, view, data);
    render_ctx.paint(
        renderer,
        pw,
        ph,
        "channel_arrangement_frame",
        &painter,
        rect,
        true,
    );
    super::render::draw_audio_clips(
        &painter,
        rect,
        view,
        layout,
        data,
        edit.selected_audio_clips,
    );

    // 播放光标。
    if let Some(ct) = *edit.cursor_tick {
        let cx_local = view.tick_to_x(ct);
        if cx_local >= lp && cx_local <= w as f32 {
            let cx = rect.min.x + cx_local;
            painter.line_segment(
                [egui::pos2(cx, rect.min.y), egui::pos2(cx, rect.max.y)],
                egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::accent_active()),
            );
        }
    }

    // 框选预览。
    if let Some(dr) = drag_rect {
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            dr,
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
        );
    }

    // 已提交选框绘制。
    for s in chan_ar_sel.iter() {
        let Some((top, bottom)) = sel_view_y(layout, &s.tracks, scroll_y) else {
            continue;
        };
        let view_sx = view.tick_to_x(s.t_start);
        let view_ex = view.tick_to_x(s.t_end);
        let snapped = egui::Rect::from_min_max(
            egui::pos2(view_sx.min(view_ex), top.min(bottom)),
            egui::pos2(view_sx.max(view_ex), top.max(bottom)),
        );
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            snapped,
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
        );
    }

    // 自动化框选 marquee。
    if let Some(mr) = am_marquee {
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            mr.translate(-rect.min.to_vec2()),
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
        );
    }

    // ── 输入：滚轮/缩放/光标 ──
    ui.push_id("ca_content_interaction", |ui| {
        crate::view_interaction::handle_input(
            ui,
            rect,
            view,
            edit.cursor_tick,
            0.0,
            Some((data.quantize, data.ppq)),
            data.bar_line_data,
            Some(music_rect),
            cfg.is_playing,
            cfg.follow_mode,
            &Tool::Select,
        );
    });

    clamp_scroll(
        view,
        w as f32,
        h as f32,
        data.total_ticks,
        layout.total_height(),
    );
}
