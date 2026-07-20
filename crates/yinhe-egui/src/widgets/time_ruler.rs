use eframe::egui;
use yinhe_types::{build_time_sig_segments, measure_ticks, TimeSigEvent};

// ── Constants ──

use crate::theme;
const MIN_LABEL_SPACING: f32 = 38.0;
const SUB_BEAT_DIV: u32 = 4;

// ── TimeRulerView trait ──

/// View information needed by the time ruler.
/// Both `PianoRollView` and `ArrangementView` implement this.
pub(crate) trait TimeRulerView {
    fn tick_to_x(&self, tick: f64) -> f32;
    fn x_to_tick(&self, x: f32) -> f64;
    fn pixels_per_tick(&self) -> f32;
    /// Minimum x where content (and ruler labels) should appear.
    fn content_left(&self) -> f32;
    /// 水平缩放（围绕指定 x，x 已转换为 view 局部坐标）。
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32);
    /// 标记 view 为 dirty，触发重绘。
    fn mark_dirty(&mut self);
}

impl TimeRulerView for yinhe_types::PianoRollView {
    fn tick_to_x(&self, tick: f64) -> f32 {
        self.tick_to_x(tick)
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.x_to_tick(x)
    }
    fn pixels_per_tick(&self) -> f32 {
        self.base.pixels_per_tick
    }
    fn content_left(&self) -> f32 {
        self.base.left_panel_width
    }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) {
        self.zoom_around_x(pointer_x, factor);
    }
    fn mark_dirty(&mut self) {
        self.base.dirty = true;
    }
}

impl TimeRulerView for yinhe_types::ArrangementView {
    fn tick_to_x(&self, tick: f64) -> f32 {
        self.tick_to_x(tick)
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.x_to_tick(x)
    }
    fn pixels_per_tick(&self) -> f32 {
        self.base.pixels_per_tick
    }
    fn content_left(&self) -> f32 {
        self.base.left_panel_width
    }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) {
        self.zoom_around_x(pointer_x, factor);
    }
    fn mark_dirty(&mut self) {
        self.base.dirty = true;
    }
}

// ── Public API ──

/// Paint a horizontal time ruler into the given rect.
///
/// Labels are aligned with the measure/beat/sub-beat grid lines rendered by wgpu.
/// Density adapts to `pixels_per_tick`:
/// - sparse → measure numbers only
/// - medium → `bar.beat`
/// - dense  → `bar.beat.sub_beat`
/// - very dense → `bar.beat.tick` (e.g. `1.1.234`)
/// Paint the ruler background and bottom divider.
fn paint_background(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 0.0, theme::RULER_BG);

    let stroke = egui::Stroke::new(1.0, theme::RULER_DIVIDER);
    painter.line_segment(
        [
            egui::pos2(rect.min.x, rect.max.y),
            egui::pos2(rect.max.x, rect.max.y),
        ],
        stroke,
    );
}

/// Paint an interactive time ruler that also jumps the cursor when clicked or dragged.
///
/// `snap` receives the raw tick under the pointer and should return the snapped tick.
/// `id_salt` must be unique for each ruler in the same UI scope (e.g. "piano_ruler"
/// vs "arrange_ruler").
///
/// Returns `true` if the ruler was clicked or dragged this frame (the caller
/// typically uses this to clear any active selection box).
pub(crate) fn interactive_ruler(
    ui: &mut egui::Ui,
    ruler_rect: egui::Rect,
    view: &mut impl TimeRulerView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    snap: impl Fn(f64) -> f64,
    id_salt: &str,
    cursor_tick: &mut Option<f64>,
) -> bool {
    let painter = ui.painter_at(ruler_rect);
    paint_background(&painter, ruler_rect);
    paint_labels(
        &painter,
        ruler_rect,
        view,
        tpb,
        default_num,
        default_den,
        time_sig_events,
    );

    let ruler_resp = ui.interact(
        ruler_rect,
        ui.id().with(id_salt),
        egui::Sense::click_and_drag(),
    );
    let mut jumped = false;
    if (ruler_resp.clicked() || ruler_resp.dragged())
        && let Some(pos) = ruler_resp.interact_pointer_pos()
    {
        let view_x = pos.x - (ruler_rect.min.x - view.content_left());
        let tick = view.x_to_tick(view_x);
        *cursor_tick = Some(snap(tick).max(0.0));
        ui.ctx().request_repaint();
        jumped = true;
    }

    // ── 滚轮 / 触摸板上下滑动 → 水平缩放 ──
    // 时间标尺专属：纯滚轮即可触发水平缩放（无需 Cmd 修饰键），
    // 与内容区的 Cmd+滚轮 缩放语义分离，避免冲突。
    // pinch（zoom_delta）也联动水平缩放。
    let pointer_in_ruler = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| ruler_rect.contains(p));
    if pointer_in_ruler {
        let pointer_x_view = ui.input(|i| i.pointer.hover_pos().unwrap_or_default()).x
            - (ruler_rect.min.x - view.content_left());

        // pinch → 水平缩放
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 {
            view.zoom_around_x(pointer_x_view, zoom_delta);
            view.mark_dirty();
            ui.ctx().request_repaint();
        }

        // 滚轮 / 触摸板上下滑动 → 水平缩放（无需 Cmd）
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        if scroll.y.abs() > 0.5 {
            let factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
            view.zoom_around_x(pointer_x_view, factor);
            view.mark_dirty();
            ui.ctx().request_repaint();
        }
    }

    jumped
}

// ── Label painting ──

fn paint_labels(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &impl TimeRulerView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
) {
    let ppu = view.pixels_per_tick();
    if ppu <= 0.001 {
        return;
    }

    // `rect` lives in the painter's coordinate system.  For pianoroll the
    // painter is from allocate_painter (origin = content left), for
    // arrangement the painter is ui.painter() (origin = window top-left).
    //
    // `tick_to_x` returns coordinates relative to `view.content_left()`.
    // We bridge the two via an offset so labels always land on screen
    // right where the wgpu grid lines appear.
    let offset_x = rect.min.x - view.content_left();
    let left = rect.min.x;
    let right = rect.max.x;

    // Convert painter-space edges back to view-space ticks
    let tick_start = view.x_to_tick((left - offset_x).max(0.0)).max(0.0);
    let tick_end = view.x_to_tick(right - offset_x);

    let ticks_per_sub = (tpb / SUB_BEAT_DIV).max(1);

    let segments = build_time_sig_segments(time_sig_events, default_num, default_den);

    let bar_offsets = cumulative_bar_offsets(tpb, &segments);

    let pixels_per_beat = tpb as f32 * ppu;
    let pixels_per_sub = ticks_per_sub as f32 * ppu;
    let pixels_per_tick = ppu;

    let show_beat = pixels_per_beat >= MIN_LABEL_SPACING;
    let show_sub = pixels_per_sub >= MIN_LABEL_SPACING;
    let show_tick = pixels_per_tick >= MIN_LABEL_SPACING;

    let tick_step = if show_tick {
        (MIN_LABEL_SPACING / ppu).ceil() as u32
    } else {
        0
    };

    let font_id = egui::FontId::new(10.0, egui::FontFamily::Monospace);
    let text_y_center = rect.min.y + rect.height() / 2.0;

    let sub_f = ticks_per_sub as f64;

    for i in 0..segments.len() {
        let (seg_start, num, den) = segments[i];
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
        let seg_start_f = seg_start as f64;
        if seg_start_f > tick_end {
            break;
        }

        let ticks_per_measure = measure_ticks(tpb, num, den);
        let ticks_per_beat = ticks_per_measure / num as u32;
        let bar_offset = bar_offsets[i];

        // ── Sub-beat granularity loop ──
        let first_tick_f = seg_start_f.max(tick_start);
        let first_sub = ((first_tick_f / sub_f).floor() as u32)
            .saturating_mul(ticks_per_sub)
            .max(seg_start);

        let mut tick = first_sub;
        while (tick as f64) <= tick_end && tick < seg_end {
            let local = tick - seg_start;
            let x = offset_x + view.tick_to_x(tick as f64);

            if x >= left && x <= right {
                let is_measure = local % ticks_per_measure == 0;
                let is_beat = if !is_measure {
                    (local % ticks_per_measure).is_multiple_of(ticks_per_beat)
                } else {
                    false
                };

                let (label, color) = if is_measure {
                    let bar = bar_offset + (local / ticks_per_measure) + 1;
                    (format!("{}", bar), theme::MEASURE_LABEL)
                } else if is_beat && show_beat {
                    let bar = bar_offset + (local / ticks_per_measure) + 1;
                    let beat = (local % ticks_per_measure) / ticks_per_beat + 1;
                    (format!("{}.{}", bar, beat), theme::BEAT_LABEL)
                } else if show_sub {
                    let bar = bar_offset + (local / ticks_per_measure) + 1;
                    let beat = (local % ticks_per_measure) / ticks_per_beat + 1;
                    if show_tick {
                        let tick_in_beat = (tick as f64 % tpb as f64) as u32;
                        (
                            format!("{}.{}.{:03}", bar, beat, tick_in_beat),
                            theme::TICK_LABEL,
                        )
                    } else {
                        let sub = (local % ticks_per_beat) / ticks_per_sub;
                        (format!("{}.{}.{}", bar, beat, sub), theme::SUB_BEAT_LABEL)
                    }
                } else {
                    tick += ticks_per_sub;
                    continue;
                };

                draw_label(painter, &font_id, x, text_y_center, &label, color);
            }

            tick += ticks_per_sub;
        }

        // ── Fine-tick loop: label individual ticks between sub-beat lines ──
        if tick_step > 0 && tick_step < ticks_per_sub {
            let first_tick_u = seg_start.max(tick_start as u32);
            let first_aligned = first_tick_u.div_ceil(tick_step) * tick_step;

            let mut ft = first_aligned;
            while (ft as f64) <= tick_end && ft < seg_end {
                let local = ft - seg_start;

                let is_measure = local % ticks_per_measure == 0;
                let is_beat_line = if !is_measure {
                    (local % ticks_per_measure).is_multiple_of(ticks_per_beat)
                } else {
                    false
                };
                let is_sub_line = local % ticks_per_sub == 0;

                if !is_measure && !is_beat_line && !is_sub_line {
                    let x = offset_x + view.tick_to_x(ft as f64);
                    if x >= left && x <= right {
                        let bar = bar_offset + (local / ticks_per_measure) + 1;
                        let beat = (local % ticks_per_measure) / ticks_per_beat + 1;
                        let tick_in_beat = (ft as f64 % tpb as f64) as u32;
                        let label = format!("{}.{}.{:03}", bar, beat, tick_in_beat);
                        draw_label(
                            painter,
                            &font_id,
                            x,
                            text_y_center,
                            &label,
                            theme::TICK_LABEL,
                        );
                    }
                }

                ft += tick_step;
            }
        }
    }
}

// ── Label drawing ──

fn draw_label(
    painter: &egui::Painter,
    font_id: &egui::FontId,
    x: f32,
    y_center: f32,
    text: &str,
    color: egui::Color32,
) {
    painter.text(
        egui::pos2(x + 2.0, y_center),
        egui::Align2::LEFT_CENTER,
        text,
        font_id.clone(),
        color,
    );
}

// ── Bar offset computation ──

/// Compute cumulative bar counts before each segment starts.
///
/// `offsets[i]` = total number of complete bars in segments 0..i.
/// Segment 0 always starts at offset 0.
fn cumulative_bar_offsets(tpb: u32, segments: &[(u32, u8, u8)]) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(segments.len());
    let mut acc = 0u32;
    for i in 0..segments.len() {
        offsets.push(acc);
        if i + 1 < segments.len() {
            let (start, num, den) = segments[i];
            let end = segments[i + 1].0;
            let tm = measure_ticks(tpb, num, den);
            if tm > 0 && end > start {
                acc += (end - start) / tm;
            }
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_bar_offsets_single_segment() {
        // 4/4 at 480tpb, one segment from tick 0
        let segs = vec![(0u32, 4u8, 2u8)];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert_eq!(offsets, vec![0]);
    }

    #[test]
    fn cumulative_bar_offsets_two_segments() {
        // 4/4 from tick 0, then 3/4 from tick 1920
        let segs = vec![(0, 4, 2), (1920, 3, 2)];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert_eq!(offsets.len(), 2);
        assert_eq!(offsets[0], 0);
        // 1920 ticks / (480*4=1920 ticks/bar) = 1 bar
        assert_eq!(offsets[1], 1);
    }

    #[test]
    fn cumulative_bar_offsets_empty() {
        let segs: Vec<(u32, u8, u8)> = vec![];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert!(offsets.is_empty());
    }

    #[test]
    fn cumulative_bar_offsets_starts_at_zero() {
        let segs = vec![(0, 4, 2), (960, 4, 2), (1920, 3, 2)];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert_eq!(offsets[0], 0);
    }
}
