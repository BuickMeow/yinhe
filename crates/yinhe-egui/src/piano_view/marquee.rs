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
        let (main_px, cross_px) = super::drag::main_cross_x_y(view, (local.x, local.y));
        let raw_tick = super::drag::main_px_to_tick_dir(view, main_px);
        let start_tick = crate::view_interaction::snap_tick(raw_tick, quantize, ppq, bar_line_data);
        // 副轴起始位置存「未滚动」坐标：横向 = content_y + scroll_y；纵向 = content_x + scroll_x。
        let start_cross = cross_px + view.cross_scroll_val();
        drag = Some(((start_tick, start_cross), local, local));
    }

    // Recompute start pixel from music coords each frame (immune to scroll/zoom).
    // 用于绘制选框时对齐量化网格。
    let start_pixel = drag.map(|((tick, cross_unscrolled), _, _)| {
        if view.is_vertical() {
            egui::pos2(
                cross_unscrolled - view.base.scroll_x,
                view.tick_to_main_px(tick),
            )
        } else {
            egui::pos2(view.tick_to_x(tick), cross_unscrolled - view.base.scroll_y)
        }
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
            crate::selection::drag::auto_scroll_on_drag_dir(
                ui,
                view,
                music_rect,
                pos,
                |view, w, h| {
                    view.clamp_scroll(w, h, total_ticks);
                },
            );
            view.clamp_scroll(content_rect.width(), content_rect.height(), total_ticks);

            // ── Tooltip：显示 ±tick / ±key（tick 按量化 snap）──
            let (s_tick, s_cross) = start_music;
            let (main_px, cross_px) = super::drag::main_cross_x_y(view, (local.x, local.y));
            let raw_cur = super::drag::main_px_to_tick_dir(view, main_px);
            let snapped_cur =
                crate::view_interaction::snap_tick(raw_cur, quantize, ppq, bar_line_data);
            let dt = (snapped_cur - s_tick).round() as i64;
            let s_key = view.cross_px_to_key(s_cross - view.cross_scroll_val());
            let cur_key = view.cross_px_to_key(cross_px);
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
                    let (x0, x1, y0, y1, t_start, t_end, key_lo, key_hi) =
                        piano_snapped_bounds(start_px, end, view, quantize, ppq, bar_line_data);
                    let kb_w = music_rect.min.x - content_rect.min.x;
                    // 横向时 x 是含 kb_w 的 content 坐标，需减 kb_w 转 music 区；
                    // 纵向时 music == content（kb_w == 0），x 即 key 像素。
                    let snapped_view_rect = egui::Rect::from_min_max(
                        egui::pos2(x0 - kb_w, y0),
                        egui::pos2(x1 - kb_w, y1),
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
        let start = if view.is_vertical() {
            egui::pos2(
                start_music.1 - view.base.scroll_x,
                view.tick_to_main_px(start_music.0),
            )
        } else {
            egui::pos2(
                view.tick_to_x(start_music.0),
                start_music.1 - view.base.scroll_y,
            )
        };
        let (x0, x1, y0, y1, _, _, _, _) =
            piano_snapped_bounds(start, end, view, quantize, ppq, bar_line_data);
        let kb_w = music_rect.min.x - content_rect.min.x;
        // 垂直全选模式：y 范围用 music_rect 全高，x 范围不变
        let snapped = if vertical {
            egui::Rect::from_min_max(
                egui::pos2(x0 - kb_w, 0.0),
                egui::pos2(x1 - kb_w, music_rect.height()),
            )
        } else {
            egui::Rect::from_min_max(egui::pos2(x0 - kb_w, y0), egui::pos2(x1 - kb_w, y1))
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
///
/// 返回 `(x0, x1, y0, y1, t_start, t_end, key_lo, key_hi)`：
/// `(x0≤x1, y0≤y1)` 是选中框在 content-local 像素里的屏幕范围——
/// 横向：x = 时间轴（tick）、y = 音高；纵向：x = 音高、y = 时间轴。
/// 时间轴两端按量化网格对齐；音高范围对齐到整键单元（横向 ≈ 旧行为）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn piano_snapped_bounds(
    start: egui::Pos2,
    end: egui::Pos2,
    view: &yinhe_types::PianoRollView,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f32, f32, f32, f32, f64, f64, u8, u8) {
    // 主轴 = 时间轴（横向 X / 纵向 Y），副轴 = 音高（横向 Y / 纵向 X）。
    let (main0, main1) = if view.is_vertical() {
        (start.y.min(end.y), start.y.max(end.y))
    } else {
        (start.x.min(end.x), start.x.max(end.x))
    };
    let (cross0, cross1) = if view.is_vertical() {
        (start.x.min(end.x), start.x.max(end.x))
    } else {
        (start.y.min(end.y), start.y.max(end.y))
    };

    let tick_s = super::drag::main_px_to_tick_dir(view, main0);
    let tick_e = super::drag::main_px_to_tick_dir(view, main1);
    let snapped_s = crate::view_interaction::snap_tick(tick_s, quantize, ppq, bar_line_data);
    let snapped_e = crate::view_interaction::snap_tick(tick_e, quantize, ppq, bar_line_data);
    let t_start = snapped_s.min(snapped_e);
    let mut t_end = snapped_s.max(snapped_e);

    // Ensure minimum width of one quantise grid interval
    let interval = quantize.tick_interval(ppq) as f64;
    if t_end <= t_start {
        t_end = t_start + interval.max(1.0);
    }

    // 副轴 key 范围 + 对齐整键单元的屏幕边界。横向保持旧数学（key127 在顶）；
    // 纵向 key0 在左：左 = key_lo 左缘，右 = key_hi 右缘。
    let (key_lo, key_hi, cross_snap0, cross_snap1) = if view.is_vertical() {
        let k0 = view.cross_px_to_key(cross0);
        let k1 = view.cross_px_to_key(cross1);
        let (lo, hi) = (k0.min(k1), k0.max(k1));
        (
            lo,
            hi,
            view.key_to_cross_px(lo),
            view.key_to_cross_px(hi) + view.key_height,
        )
    } else {
        let kh = view.key_height;
        let scroll_y = view.base.scroll_y;
        let lo = (127.0 - ((scroll_y + cross1) / kh))
            .ceil()
            .clamp(0.0, 127.0) as u8;
        let hi = (127.0 - ((scroll_y + cross0) / kh))
            .ceil()
            .clamp(0.0, 127.0) as u8;
        let top = (127.0 - hi as f32) * kh - scroll_y;
        let bottom = (127.0 - lo as f32 + 1.0) * kh - scroll_y;
        (lo, hi, top, bottom)
    };

    // 主轴对齐量化网格后的像素：横向 x / 纵向 y
    let main_snap0 = super::drag::tick_to_main_px_dir(view, t_start);
    let main_snap1 = super::drag::tick_to_main_px_dir(view, t_end);
    let (x0, x1, y0, y1) = if view.is_vertical() {
        (
            cross_snap0.min(cross_snap1),
            cross_snap0.max(cross_snap1),
            main_snap0.min(main_snap1),
            main_snap0.max(main_snap1),
        )
    } else {
        (
            main_snap0.min(main_snap1),
            main_snap0.max(main_snap1),
            cross_snap0.min(cross_snap1),
            cross_snap0.max(cross_snap1),
        )
    };

    (x0, x1, y0, y1, t_start, t_end, key_lo, key_hi)
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
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            key_height: 10.0,
            viewport_h: 0.0,
            orientation: yinhe_types::Orientation::Horizontal,
        }
    }

    fn content() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    /// 浮动工具条上的一点（用 compute_bar_rect 计算得到，避免硬编码坐标）。
    fn bar_point() -> egui::Pos2 {
        let eff = [(0.0f64, 100.0f64, 60u8, 70u8)];
        let view = test_view();
        let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
            &view, eff[0].0, eff[0].1, eff[0].2, eff[0].3,
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
