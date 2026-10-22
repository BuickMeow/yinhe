use eframe::egui;
use egui_material_icons::icons::ICON_CHECK;

use yinhe_types::{KeySigEvent, PianoRollView, TimeSigEvent};

use super::types::RULER_H;
use crate::widgets::selection_actions::{BarButton, SelectionAction};

/// 覆盖层绘制：背景、音阶背景、网格线、wgpu 纹理、键盘、游标、选框、标尺。
///
/// 抽取自 `piano_view.rs` 570-842 行（现 276-548 段），保持原绘制顺序与坐标系：
/// content_rect 含键盘列、music_rect 为纯音乐区（横向不含 kb_w）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_overlays(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    rect: egui::Rect,
    view: &mut PianoRollView,
    theme: &yinhe_theme::GpuTheme,
    kh: f32,
    kb_w: f32,
    key_sig_events: &[KeySigEvent],
    content_opacity: f32,
    midi: Option<&dyn yinhe_types::NoteSource>,
    tpb: Option<u32>,
    grid_rect: egui::Rect,
    cull_ready: bool,
    render_ctx: &mut crate::render_context::RenderContext,
    pianoroll: &mut yinhe_wgpu::InstanceRenderer,
    pw: u32,
    ph: u32,
    keyboard_rect: egui::Rect,
    cursor_tick: &mut Option<f64>,
    effective_tool: crate::widgets::tools_panel::Tool,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    quantize: yinhe_editor_core::quantize::QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    selected: &mut yinhe_core::Selection,
    pen_path: Option<&yinhe_editor_core::pen::PenPath>,
    scissors_line: Option<&yinhe_editor_core::edit_state::AnchorLine>,
    markers: &[yinhe_types::MarkerEvent],
) -> (
    Option<crate::widgets::selection_actions::SelectionAction>,
    Option<crate::widgets::time_ruler::MarkerEdit>,
) {
    // 兼容任务要求的形参（部分由 midi 派生，此处透传占位，避免未使用警告）
    let _ = tpb;
    let _ = grid_rect;
    let _ = keyboard_rect;

    // ── Background（app_bg 一层，不透明不叠加；条纹/色块自行叠上）──
    painter.rect_filled(content_rect, 0.0, crate::theme::app_bg());

    // ── Scale background + 八度横线（调号驱动的调内/调外/根音条带）──
    // bg::paint
    super::bg::paint(
        painter,
        content_rect,
        kb_w,
        kh,
        view,
        key_sig_events,
        content_opacity,
    );

    // ── Grid lines (drawn by egui before wgpu texture) ──
    // 代替原 wgpu grid layer，与 time_ruler 共用 MIN_SPACING 阈值。
    // grid_lines::paint_grid_lines
    if let Some(midi) = midi
        && let Some(tpb_val) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let grid_rect_computed = if view.is_vertical() {
            content_rect
        } else {
            egui::Rect::from_min_max(
                egui::pos2(
                    content_rect.min.x + view.keyboard_width(),
                    content_rect.min.y,
                ),
                content_rect.max,
            )
        };
        crate::widgets::grid_lines::paint_grid_lines(
            painter,
            grid_rect_computed,
            &view.base,
            tpb_val,
            def_num,
            def_den,
            sig_events,
            &crate::widgets::grid_lines::GridColors::pianoroll(),
            view.orientation(),
        );
    }

    // Paint wgpu content into the content_rect (notes only — grid moved to egui)
    // render_ctx::paint
    if cull_ready {
        render_ctx.paint(
            pianoroll,
            pw,
            ph,
            "pianoroll_frame",
            painter,
            content_rect,
            true,
        );
    } else {
        render_ctx.paint_texture_only(pw, ph, painter, content_rect);
    }

    // ── Keyboard (drawn by egui on top of the wgpu texture) ──
    // 横向 = 左侧键盘列；纵向 = 底部横键盘条（高 = kb_w）。
    // keyboard::paint
    let keyboard_rect_computed = if view.is_vertical() {
        let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;
        let kb_bottom = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H;
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, kb_bottom - kb_w),
            egui::pos2(content_right_x, kb_bottom),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, content_rect.min.y),
            egui::pos2(content_rect.min.x + kb_w, content_rect.max.y),
        )
    };
    super::keyboard::paint(painter, keyboard_rect_computed, kb_w, kh, view, theme);

    // 纵向：底部键盘条上滚轮/触控板缩放音高轴（横向由 handle_input 的左区处理）。
    if view.is_vertical() {
        keyboard_zoom(ui, view, keyboard_rect_computed, content_rect);
    }

    // ── Playback cursor (drawn by egui on top of the wgpu texture) ──
    // line_segment
    if let Some(ct) = *cursor_tick {
        let kb_w_cur = view.keyboard_width();
        let w = content_rect.width();
        let h = content_rect.height();
        if view.is_vertical() {
            let cy_local = view.tick_to_main_px(ct);
            if (0.0..=h).contains(&cy_local) {
                let cy = content_rect.min.y + cy_local;
                painter.line_segment(
                    [
                        egui::pos2(content_rect.min.x, cy),
                        egui::pos2(content_rect.max.x, cy),
                    ],
                    egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::accent_active()),
                );
            }
        } else if kb_w_cur <= w {
            let cx_local = view.tick_to_x(ct);
            if (kb_w_cur..=w).contains(&cx_local) {
                let cx = content_rect.min.x + cx_local;
                painter.line_segment(
                    [
                        egui::pos2(cx, content_rect.min.y),
                        egui::pos2(cx, content_rect.max.y),
                    ],
                    egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::accent_active()),
                );
            }
        }
    }

    // ── Draw selection box on TOP of GPU content ──
    let mut sel_action = None;
    let mut marker_edit: Option<crate::widgets::time_ruler::MarkerEdit> = None;
    // marquee::draw_marquee_box
    if effective_tool == crate::widgets::tools_panel::Tool::Select
        || effective_tool == crate::widgets::tools_panel::Tool::SelectVertical
    {
        let vertical = effective_tool == crate::widgets::tools_panel::Tool::SelectVertical;
        super::marquee::draw_marquee_box(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            "sel_drag",
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
            vertical,
        );
    } else if effective_tool == crate::widgets::tools_panel::Tool::Eraser {
        super::marquee::draw_marquee_box(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            "eraser_drag",
            crate::theme::danger_text_bright(),
            crate::theme::danger_text_bright(),
            false,
        );
    }

    // ── 锚点线工具（直线/剪刀）：线、锚点与生成/切割位置预览 ──
    match effective_tool {
        crate::widgets::tools_panel::Tool::Line => {
            if let Some(path) = pen_path {
                super::pen_tool::paint(
                    painter,
                    content_rect,
                    view,
                    path,
                    super::pen_tool::is_drawing(ui),
                    ui.input(|i| i.pointer.hover_pos()),
                );
            }
        }
        crate::widgets::tools_panel::Tool::Scissors => {
            if let Some(line) = scissors_line {
                super::anchor_line::paint_line(
                    painter,
                    content_rect,
                    view,
                    line,
                    crate::theme::accent_active(),
                );
                let cuts = yinhe_editor_core::quantize::line_cuts(
                    line.start,
                    line.end,
                    quantize,
                    ppq,
                    bar_line_data,
                );
                super::anchor_line::paint_scissors_cuts(painter, content_rect, view, &cuts);
            }
        }
        _ => {}
    }
    // 已提交的持久选框：任意工具下均保持可见。
    // 悬停 transport bar 网格按钮时切换为「网格预览」：强调色选框 + 量化网格线，
    // 预览即将按哪个网格切割（与切割用的已解析量化同源；Off 无网格不预览）。
    let grid_interval = quantize.tick_interval(ppq);
    let grid_preview = grid_interval > 0
        && ui.data_mut(|d| {
            d.get_temp::<bool>(egui::Id::unique(
                crate::widgets::selection_actions::GRID_PREVIEW_HOVER_ID,
            ))
            .unwrap_or(false)
        });
    {
        let eff_rects = sel_rect.effective_rects();
        if !eff_rects.is_empty() {
            let persisted_pixel_rects: Vec<egui::Rect> = eff_rects
                .iter()
                .map(|&(t_start, t_end, key_lo, key_hi)| {
                    crate::selection::drag::music_sel_to_pixel_rect(
                        view, t_start, t_end, key_lo, key_hi,
                    )
                })
                .collect();
            let (fill_color, stroke_color) = if grid_preview {
                (crate::theme::accent_active(), crate::theme::accent_active())
            } else {
                (crate::theme::contrast_fg(), crate::theme::contrast_fg())
            };
            {
                let kb_w_shift = if view.is_vertical() {
                    0.0
                } else {
                    music_rect.min.x - content_rect.min.x
                };
                let music_rect_local = egui::Rect::from_min_max(
                    egui::pos2(0.0, 0.0),
                    egui::pos2(music_rect.width(), music_rect.height()),
                );
                for &r in &persisted_pixel_rects {
                    let shifted = egui::Rect::from_min_max(
                        egui::pos2(r.min.x - kb_w_shift, r.min.y),
                        egui::pos2(r.max.x - kb_w_shift, r.max.y),
                    );
                    if shifted.intersects(music_rect_local) {
                        crate::selection::draw::draw(
                            ui.painter(),
                            music_rect,
                            shifted,
                            fill_color,
                            stroke_color,
                        );
                    }
                }
            }
            if grid_preview {
                let main_len = if view.is_vertical() {
                    music_rect.height()
                } else {
                    music_rect.width()
                };
                for &(t0, t1, kl, kh) in &eff_rects {
                    paint_rect_grid(
                        painter,
                        content_rect,
                        view,
                        t0,
                        t1,
                        kl,
                        kh,
                        grid_interval,
                        main_len,
                    );
                }
            }
        }
    }

    // ── 浮动工具条：直线/剪刀用锚点线 bbox 定位 ──
    let line_buttons = [(ICON_CHECK, SelectionAction::LineConfirm)];
    let scissors_buttons = [(ICON_CHECK, SelectionAction::ScissorsConfirm)];
    let bar_spec: Option<(&[BarButton], Option<egui::Rect>)> = match effective_tool {
        // 选择工具的动作按钮已迁到 transport bar（见 selection_actions::SELECT_BAR_BUTTONS），
        // 此处不再绘制浮动条。
        crate::widgets::tools_panel::Tool::Select
        | crate::widgets::tools_panel::Tool::SelectVertical => None,
        crate::widgets::tools_panel::Tool::Line => pen_path.map(|p| {
            (
                &line_buttons[..],
                Some(super::pen_tool::pixel_bbox(view, p)),
            )
        }),
        crate::widgets::tools_panel::Tool::Scissors => scissors_line.map(|l| {
            (
                &scissors_buttons[..],
                Some(super::anchor_line::pixel_bbox(view, l)),
            )
        }),
        _ => None,
    };
    if let Some((buttons, rect)) = bar_spec
        && let Some(action) = crate::widgets::selection_actions::show(ui, music_rect, rect, buttons)
    {
        sel_action = Some(action);
    }

    // ── Time ruler ──（ruler 贴内容顶部，便于查看/跳转）
    // time_ruler::interactive_ruler
    if let Some(midi) = midi
        && let Some(tpb_val) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let content_y = content_rect.min.y;
        let content_bottom = content_rect.max.y;
        let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;
        let ruler_rect = if view.is_vertical() {
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, content_y),
                egui::pos2(rect.min.x + RULER_H, content_bottom),
            )
        } else {
            // 横向：ruler 从 PR 顶部开始；键盘与右上角空白同带。
            let ruler_y0 = rect.min.y;
            let ruler_y1 = ruler_y0 + RULER_H;
            let left_corner = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, ruler_y0),
                egui::pos2(rect.min.x + view.keyboard_width(), ruler_y1),
            );
            ui.painter()
                .rect_filled(left_corner, 0.0, crate::theme::track_bg());
            let corner_rect = egui::Rect::from_min_max(
                egui::pos2(content_right_x, ruler_y0),
                egui::pos2(rect.max.x, ruler_y1),
            );
            ui.painter()
                .rect_filled(corner_rect, 0.0, crate::theme::track_bg());
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x + view.keyboard_width(), ruler_y0),
                egui::pos2(content_right_x, ruler_y1),
            )
        };
        let outcome = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            view,
            tpb_val,
            def_num,
            def_den,
            sig_events,
            markers,
            |tick| crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data),
            "piano_ruler",
            cursor_tick,
        );
        if outcome.jumped {
            selected.clear();
            sel_rect.clear();
        }
        marker_edit = outcome.marker_edit;
        let _ = tpb_val;
    }

    (sel_action, marker_edit)
}

/// 纵向瀑布流：指针在底部键盘条上时，滚轮/触控板沿音高轴（屏幕 X）缩放。
fn keyboard_zoom(
    ui: &egui::Ui,
    view: &mut PianoRollView,
    kb_rect: egui::Rect,
    content_rect: egui::Rect,
) {
    if !crate::view_interaction::pointer_hits(ui, kb_rect) {
        return;
    }
    crate::widgets::hint::set(ui.ctx(), rust_i18n::t!("hint.pr.keyboard_zoom"));
    let pos = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
    let anchor = pos.x - content_rect.min.x;
    let mut changed = false;

    let pinch = ui.input(|i| i.zoom_delta());
    if (pinch - 1.0).abs() > 0.001 {
        view.zoom_around_x(anchor, pinch);
        changed = true;
    }
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll.abs() > 0.5 {
        let factor = if scroll > 0.0 { 1.0 / 1.1 } else { 1.1 };
        view.zoom_around_x(anchor, factor);
        changed = true;
    }

    if changed {
        view.base.dirty = true;
        ui.ctx().request_repaint();
    }
}

/// 在选框范围内画量化网格线（仅主轴方向：横向画竖线、纵向画横线），
/// 按可见范围裁剪。网格间距不足 1px 时跳过（画出来是一片纯色）。
///
/// `main_len` = 主轴视口长度（横向 = 音乐区宽，纵向 = 内容高）。
/// 供 transport bar 网格按钮的悬停预览使用（与切割用的已解析量化同源）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn paint_rect_grid(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    view: &PianoRollView,
    t_start: f64,
    t_end: f64,
    key_lo: u8,
    key_hi: u8,
    interval: u32,
    main_len: f32,
) {
    if interval == 0 {
        return;
    }
    // 网格间距不足 1px 时画出来是一片纯色，跳过内部线。
    if (interval as f32) * view.base.pixels_per_tick < 1.0 {
        return;
    }
    let (vis0, vis1) = view.visible_main_range(main_len);
    let lo = t_start.max(vis0).max(0.0);
    let hi = t_end.min(vis1);
    if hi <= lo {
        return;
    }
    // 副轴覆盖范围：横向 key_hi 在上（小 y），纵向 key_lo 在左。
    let (cross0, cross1) = if view.is_vertical() {
        (
            view.key_to_cross_px(key_lo),
            view.key_to_cross_px(key_hi) + view.key_height,
        )
    } else {
        (
            view.key_to_cross_px(key_hi),
            view.key_to_cross_px(key_lo) + view.key_height,
        )
    };
    let stroke = egui::Stroke::new(1.0, crate::theme::accent_active().gamma_multiply(0.55));
    let interval_f = interval as f64;
    let mut t = (lo / interval_f).ceil() * interval_f;
    while t <= hi {
        let main_px = crate::selection::drag::tick_to_main_px_dir(view, t);
        let (a, b) = if view.is_vertical() {
            (
                egui::pos2(content_rect.min.x + cross0, content_rect.min.y + main_px),
                egui::pos2(content_rect.min.x + cross1, content_rect.min.y + main_px),
            )
        } else {
            (
                egui::pos2(content_rect.min.x + main_px, content_rect.min.y + cross0),
                egui::pos2(content_rect.min.x + main_px, content_rect.min.y + cross1),
            )
        };
        painter.line_segment([a, b], stroke);
        t += interval_f;
    }
}
