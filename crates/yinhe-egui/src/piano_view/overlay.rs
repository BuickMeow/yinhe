use eframe::egui;

use yinhe_types::{KeySigEvent, PianoRollView, TimeSigEvent};

use super::control_bar;
use super::types::{PianoViewFeedback, RULER_H};

/// 覆盖层绘制：背景、音阶背景、网格线、wgpu 纹理、键盘、游标、选框、标尺、控制栏。
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
    bar: &control_bar::PrBarData<'_>,
    feedback: &mut PianoViewFeedback<'_>,
    selected: &mut yinhe_core::Selection,
) -> Option<crate::widgets::selection_actions::SelectionAction> {
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
    // 已提交的持久选框：任意工具下均保持可见
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
                            crate::theme::contrast_fg(),
                            crate::theme::contrast_fg(),
                        );
                    }
                }
            }
            if (effective_tool == crate::widgets::tools_panel::Tool::Select
                || effective_tool == crate::widgets::tools_panel::Tool::SelectVertical)
                && let Some(action) = crate::widgets::selection_actions::show(
                    ui,
                    music_rect,
                    persisted_pixel_rects.last().copied(),
                )
            {
                sel_action = Some(action);
            }
        }
    }

    // ── Time ruler ──（横向：control_bar 在最上，ruler 在其下贴内容，更贴近音符便于查看/跳转）
    // time_ruler::interactive_ruler
    if let Some(midi) = midi
        && let Some(tpb_val) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let content_y = content_rect.min.y;
        let content_bottom = content_rect.max.y;
        let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;
        let ruler_band_y = rect.min.y;
        let ruler_rect = if view.is_vertical() {
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, content_y),
                egui::pos2(rect.min.x + RULER_H, content_bottom),
            )
        } else {
            // 横向 ruler 下移 PR_BAR_H，紧贴内容；键盘与右上角空白相应下移
            let ruler_y0 = ruler_band_y + crate::theme::PR_BAR_H;
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
        let ruler_jumped = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            view,
            tpb_val,
            def_num,
            def_den,
            sig_events,
            |tick| crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data),
            "piano_ruler",
            cursor_tick,
        );
        if ruler_jumped {
            selected.clear();
            sel_rect.clear();
        }
        let _ = tpb_val;
    }

    // ── PR 控制栏（最顶部：量化/音轨名称/和弦指示器；横向时在标尺上方）──
    // control_bar::show
    {
        let bar_y0 = rect.min.y;
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, bar_y0),
            egui::pos2(rect.max.x, bar_y0 + crate::theme::PR_BAR_H),
        );
        super::control_bar::show(ui, bar_rect, bar, feedback.bar_events);
    }

    sel_action
}
