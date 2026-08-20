//! Marquee drag (框选) + eraser tool logic.
//!
//! 提供共享的框选状态机 `marquee_drag_frame`，供选框工具和橡皮擦工具复用。
//! 绘制函数 `draw_marquee_box` 在 GPU 内容之上绘制选框。

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::TimeSigEvent;

use super::PianoViewEvent;

/// Result of a completed marquee drag (distance >= 3px).
pub(crate) struct MarqueeDragResult {
    pub t_start: f64,
    pub t_end: f64,
    pub key_lo: u8,
    pub key_hi: u8,
    /// view-local pixel rect of the snapped marquee (for drawing).
    #[allow(dead_code)]
    pub snapped_view_rect: egui::Rect,
}

/// Shared marquee drag lifecycle: press → move (with auto-scroll) → release.
///
/// Returns `Some(MarqueeDragResult)` on a valid drag release (>= 3px), `None` otherwise.
/// A click that stays within 3px returns `None` — the caller can treat it as a
/// plain cursor-position click (set cursor_tick etc.).
///
/// `on_bar`: 按下时指针是否在选框浮动工具条上。为 true 时不启动框选——
/// 否则从工具条上按下拖拽会穿透到选框逻辑（不按 ctrl 也能拉出第二个选框）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn marquee_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    id_suffix: &'static str,
    on_bar: bool,
) -> Option<MarqueeDragResult> {
    let sel_id = ui.id().with(id_suffix);
    // drag = (start_music, press_pos, current_pos)
    // - start_music: (snapped_tick, content_y) — 量化后的起始音乐坐标，用于计算选区 bounds
    // - press_pos: 按下时的原始像素位置 — 用于 3px 阈值检查（不受量化偏移影响）
    // - current_pos: 当前鼠标位置 — 用于绘制选框 + auto-scroll
    let mut drag: Option<((f64, f32), egui::Pos2, egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    // Clear stale drag state
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return None;
    }

    // Press → start drag
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && !on_bar
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let raw_tick = view.x_to_tick(local.x);
        let start_tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
        let start_content_y = local.y + view.base.scroll_y;
        drag = Some(((start_tick, start_content_y), local, local));
    }

    // Recompute start pixel from music coords each frame (immune to scroll/zoom).
    // 用于绘制选框时对齐量化网格。
    let start_pixel = drag.map(|((tick, content_y), _, _)| {
        egui::pos2(view.tick_to_x(tick), content_y - view.base.scroll_y)
    });

    // Move -> update with auto-scroll
    if let (Some(start_px), Some((start_music, press_pos, _))) = (start_pixel, drag) {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            let clamped = pos.clamp(music_rect.min, music_rect.max);
            let local = egui::pos2(
                clamped.x - content_rect.min.x,
                clamped.y - content_rect.min.y,
            );

            drag = Some((start_music, press_pos, local));

            // ── Auto-scroll when dragging near the edge ──
            // No scroll compensation needed: start is in music coords, so it
            // automatically follows the content.
            crate::selection::drag::auto_scroll_on_drag(
                ui,
                &mut view.base,
                music_rect,
                pos,
                |base, w, _h| {
                    base.clamp_scroll_x(w, total_ticks);
                    base.scroll_y = base.scroll_y.max(0.0);
                },
            );
            view.clamp_scroll(content_rect.width(), content_rect.height(), total_ticks);

            // ── Tooltip：显示 ±tick / ±key（tick 按量化 snap）──
            let (s_tick, s_content_y) = start_music;
            let raw_cur = view.x_to_tick(local.x);
            let snapped_cur =
                crate::view_interaction::snap_tick(raw_cur, quantize, ppq, bar_line_data);
            let dt = (snapped_cur - s_tick).round() as i64;
            let s_key = view.y_to_key(s_content_y - view.base.scroll_y);
            let cur_key = view.y_to_key(local.y);
            let dk = cur_key as i32 - s_key as i32;
            let lines = vec![
                crate::view_interaction::format_signed("tick", dt),
                crate::view_interaction::format_signed("key", dk as i64),
            ];
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, pos.x, pos.y);
        }

        // Release → compute snapped bounds
        if pointer.primary_released() {
            let result = drag.and_then(|(_, press_pos, end)| {
                // 3px 阈值用 press_pos（按下时的原始像素位置）
                if (end - press_pos).length() >= 3.0 {
                    // 选区 bounds 用 start_px（量化后）和 end（当前鼠标）计算
                    let (sx, ex, sy, ey, t_start, t_end, key_lo, key_hi) =
                        piano_snapped_bounds(start_px, end, view, quantize, ppq, bar_line_data);
                    let kb_w = music_rect.min.x - content_rect.min.x;
                    let snapped_view_rect = egui::Rect::from_min_max(
                        egui::pos2(sx.min(ex) - kb_w, sy.min(ey)),
                        egui::pos2(sx.max(ex) - kb_w, sy.max(ey)),
                    );
                    Some(MarqueeDragResult {
                        t_start,
                        t_end,
                        key_lo,
                        key_hi,
                        snapped_view_rect,
                    })
                } else {
                    None
                }
            });
            ui.data_mut(|d| {
                d.insert_persisted(sel_id, Option::<((f64, f32), egui::Pos2, egui::Pos2)>::None)
            });
            view.base.dirty = true;
            return result;
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    None
}

/// Draw the active marquee box on top of GPU content.
/// Must be called AFTER `render_ctx.paint` so the box is not covered by the texture.
/// `id_suffix` — persisted drag state key (e.g. "sel_drag" or "eraser_drag").
/// `fill_color` / `stroke_color` — base colors for the marquee.
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn draw_marquee_box(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    id_suffix: &'static str,
    fill_color: egui::Color32,
    stroke_color: egui::Color32,
    vertical: bool,
) {
    let drag_id = ui.id().with(id_suffix);
    let drag: Option<((f64, f32), egui::Pos2, egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);

    if let Some((start_music, press_pos, end)) = drag {
        // 3px 阈值检查用 press_pos（按下时的原始像素位置）
        if (end - press_pos).length() < 3.0 {
            return;
        }
        // 绘制用 start_music（量化后）和 end（当前鼠标）计算选区 bounds
        let start = egui::pos2(
            view.tick_to_x(start_music.0),
            start_music.1 - view.base.scroll_y,
        );
        let (vx, vy, vw, vh, _, _, _, _) =
            piano_snapped_bounds(start, end, view, quantize, ppq, bar_line_data);
        let kb_w = music_rect.min.x - content_rect.min.x;
        // 垂直全选模式：y 范围用 music_rect 全高，x 范围不变
        let snapped = if vertical {
            egui::Rect::from_min_max(
                egui::pos2(vx.min(vy) - kb_w, 0.0),
                egui::pos2(vx.max(vy) - kb_w, music_rect.height()),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(vx.min(vy) - kb_w, vw.min(vh)),
                egui::pos2(vx.max(vy) - kb_w, vw.max(vh)),
            )
        };
        crate::selection::draw::draw(ui.painter(), music_rect, snapped, fill_color, stroke_color);
    }
}

// ── Eraser tool ──

/// Eraser-tool input: uses the shared marquee drag, then returns an
/// `EraserDelete` event on release. No selection persistence.
#[allow(clippy::too_many_arguments)]
pub(crate) fn eraser_drag_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    total_ticks: f64,
    track_selected: &std::collections::HashSet<u16>,
) -> Option<PianoViewEvent> {
    let result = marquee_drag_frame(
        ui,
        content_rect,
        music_rect,
        view,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        "eraser_drag",
        // 橡皮擦工具没有选区浮动工具条，无需防穿透
        false,
    )?;
    // 轨道作用域：track_selected（空 = 全部轨道）。
    let (track_lo, track_hi) = crate::selection::drag::pr_track_range(track_selected);
    Some(PianoViewEvent::EraserDelete {
        t_start: result.t_start as u32,
        t_end: result.t_end as u32,
        key_lo: result.key_lo,
        key_hi: result.key_hi,
        track_lo,
        track_hi,
    })
}

/// Compute snapped selection bounds for piano roll.
#[allow(clippy::too_many_arguments)]
pub(crate) fn piano_snapped_bounds(
    start: egui::Pos2,
    end: egui::Pos2,
    view: &yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f32, f32, f32, f32, f64, f64, u8, u8) {
    let sx = start.x.min(end.x);
    let ex = start.x.max(end.x);
    let sy = start.y.min(end.y);
    let ey = start.y.max(end.y);

    let tick_s = view.x_to_tick(sx);
    let tick_e = view.x_to_tick(ex);
    let snapped_s = crate::view_interaction::snap_tick(tick_s, quantize, ppq, bar_line_data);
    let snapped_e = crate::view_interaction::snap_tick(tick_e, quantize, ppq, bar_line_data);
    let t_start = snapped_s.min(snapped_e);
    let mut t_end = snapped_s.max(snapped_e);

    // Ensure minimum width of one quantise grid interval
    let interval = quantize.tick_interval(ppq) as f64;
    if t_end <= t_start {
        t_end = t_start + interval.max(1.0);
    }

    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;

    let key_lo = (127.0 - ((scroll_y + ey) / kh)).ceil().clamp(0.0, 127.0) as u8;
    let key_hi = (127.0 - ((scroll_y + sy) / kh)).ceil().clamp(0.0, 127.0) as u8;
    let screen_sy = (127.0 - key_hi as f32) * kh - scroll_y;
    let screen_ey = (127.0 - key_lo as f32 + 1.0) * kh - scroll_y;

    let screen_sx = view.tick_to_x(t_start);
    let screen_ex = view.tick_to_x(t_end);

    (
        screen_sx, screen_ex, screen_sy, screen_ey, t_start, t_end, key_lo, key_hi,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_editor_core::quantize::QuantizePreset;

    /// 构造测试用的钢琴卷帘视图：1px/tick、无滚动、key 高 10px。
    fn test_view() -> yinhe_types::PianoRollView {
        yinhe_types::PianoRollView {
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: 1.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 0.0,
                dirty: false,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
            key_height: 10.0,
            viewport_h: 0.0,
        }
    }

    fn content() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    /// 浮动工具条上的一点（用 compute_bar_rect 计算得到，避免硬编码坐标）。
    fn bar_point() -> egui::Pos2 {
        let eff = [(0.0f64, 100.0f64, 60u8, 70u8)];
        let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
            &test_view().base,
            test_view().key_height,
            eff[0].0,
            eff[0].1,
            eff[0].2,
            eff[0].3,
        );
        let bar = crate::widgets::selection_actions::compute_bar_rect(content(), pixel_rect)
            .expect("bar 应显示");
        bar.center()
    }

    /// 跑一帧 marquee_drag_frame，返回框选结果。
    fn run_frame(
        ctx: &egui::Context,
        raw: egui::RawInput,
        view: &mut yinhe_types::PianoRollView,
        on_bar: bool,
    ) -> Option<MarqueeDragResult> {
        let mut out = None;
        ctx.run_ui(raw, |ui| {
            out = marquee_drag_frame(
                ui,
                content(),
                content(),
                view,
                QuantizePreset::Fraction(1, 4),
                480,
                None,
                1000.0,
                "sel_drag",
                on_bar,
            );
        })
        .textures_delta
        .clear();
        out
    }

    fn press_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    fn drag_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw
    }

    fn release_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    /// 回归测试：从浮动工具条上按下拖拽不得启动框选。
    /// （曾导致不按 ctrl 也能拉出第二个选框 —— 事件穿透。）
    #[test]
    fn marquee_from_action_bar_does_not_start() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        let pos = bar_point();

        let _ = run_frame(&ctx, press_event(pos), &mut view, true);
        let _ = run_frame(
            &ctx,
            drag_event(pos + egui::vec2(30.0, 10.0)),
            &mut view,
            true,
        );
        let result = run_frame(
            &ctx,
            release_event(pos + egui::vec2(30.0, 10.0)),
            &mut view,
            true,
        );
        assert!(result.is_none(), "从工具条按下拖拽不得产生选框（事件穿透）");
    }

    /// 保障测试：音乐区正常框选不受防穿透影响。
    #[test]
    fn marquee_normal_drag_still_works() {
        let ctx = egui::Context::default();
        let mut view = test_view();
        let start = egui::pos2(200.0, 300.0);
        let end = egui::pos2(240.0, 330.0);

        let _ = run_frame(&ctx, press_event(start), &mut view, false);
        let _ = run_frame(&ctx, drag_event(end), &mut view, false);
        let result = run_frame(&ctx, release_event(end), &mut view, false);
        let result = result.expect("正常框选应产生结果");
        assert!(result.t_start < result.t_end, "t 范围应有效");
        assert!(result.key_lo <= result.key_hi, "key 范围应有效");
    }
}
