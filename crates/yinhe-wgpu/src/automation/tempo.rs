use yinhe_types::AutomationPanelView;
use yinhe_theme::GpuTheme;
use crate::vertex::DrawInstance;

/// Build stepped-line instances for tempo curve (layer 2).
///
/// Renders each tempo event as a staircase: horizontal line (bpm held) +
/// vertical line (bpm change).  Range is [0, max_bpm] where max_bpm is the
/// highest BPM across all tempo events.
pub fn build_tempo_lines(
    out: &mut Vec<DrawInstance>,
    w: f32,
    _h: f32,
    view: &AutomationPanelView,
    tempo_events: &[(u32, f64)],
    _theme: &GpuTheme,
) {
    if tempo_events.is_empty() {
        return;
    }

    let max_bpm = tempo_events
        .iter()
        .map(|(_, bpm)| *bpm)
        .fold(0.0f64, f64::max) as f32;
    if max_bpm <= 0.0 {
        return;
    }

    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;
    let x_offset = view.base.left_panel_width - view.base.scroll_x;
    let grid_left_x = view.base.left_panel_width;

    // Find first visible event index
    let vis_start = tempo_events.partition_point(|e| e.0 < pad_start);
    let vis_end = tempo_events.partition_point(|e| e.0 < pad_end);

    if vis_start >= vis_end {
        // No events in visible range: draw full-width line at chase value
        let chase_idx = if vis_start > 0 { vis_start - 1 } else { 0 };
        let val = tempo_events[chase_idx].1 as f32;
        let y = view.value_to_y(val, max_bpm);
        if w > grid_left_x {
            out.push(DrawInstance::solid_rect(
                grid_left_x, y, w - grid_left_x, 1.0,
                [0.80, 0.30, 0.30, 0.85],
            ));
        }
        return;
    }

    // Value before first visible event (chase)
    let prev_idx = if vis_start > 0 { vis_start - 1 } else { 0 };
    let mut prev_val = tempo_events[prev_idx].1 as f32;
    let mut prev_tick = tempo_events[prev_idx].0;

    // Horizontal line from grid left edge to the first visible event
    let first_tick = tempo_events[vis_start].0;
    let first_x = x_offset + first_tick as f32 * ppu;
    let first_y = view.value_to_y(prev_val, max_bpm);
    if first_x > grid_left_x {
        out.push(DrawInstance::solid_rect(
            grid_left_x, first_y, first_x - grid_left_x, 1.0,
            [0.80, 0.30, 0.30, 0.85],
        ));
    }

    for i in vis_start..vis_end {
        let (tick, bpm) = tempo_events[i];
        let val = bpm as f32;
        let x1 = x_offset + prev_tick as f32 * ppu;
        let x2 = x_offset + tick as f32 * ppu;
        let y1 = view.value_to_y(prev_val, max_bpm);
        let y2 = view.value_to_y(val, max_bpm);

        // Horizontal line: value held from prev_tick to tick
        if x2 > x1 {
            out.push(DrawInstance::solid_rect(
                x1, y1, x2 - x1, 1.0,
                [0.80, 0.30, 0.30, 0.85],
            ));
        }

        // Vertical line: value change at tick
        let dy = y2 - y1;
        if dy.abs() > 0.0 {
            out.push(DrawInstance::solid_rect(
                x2 - 0.5, y1.min(y2), 1.0, dy.abs(),
                [0.80, 0.30, 0.30, 0.85],
            ));
        }

        prev_val = val;
        prev_tick = tick;
    }

    // Horizontal line from last visible event to right edge
    let last_x = x_offset + prev_tick as f32 * ppu;
    let last_y = view.value_to_y(prev_val, max_bpm);
    if w > last_x {
        out.push(DrawInstance::solid_rect(
            last_x, last_y, w - last_x, 1.0,
            [0.80, 0.30, 0.30, 0.85],
        ));
    }
}
