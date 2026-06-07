use yinhe_types::{
    AutomationLane, TimeSigEvent, TRACK_PALETTE,
};

use crate::automation_view::AutomationPanelView;
use crate::grid;
use crate::vertex::{NoteInstance, pack_props, pack_rgba};

/// Bar width in pixels for automation events.
const BAR_WIDTH: f32 = 2.0;
/// Corner radius for automation bars (flat).
const BAR_ROUNDING: f32 = 0.0;
/// Border width for automation bars.
const BAR_BORDER: f32 = 0.0;
/// Color for the center/default reference line.
const CENTER_LINE_COLOR: (f32, f32, f32) = (0.30, 0.30, 0.35);
/// Center line alpha.
const CENTER_LINE_ALPHA: f32 = 0.6;

/// Build all instances for one automation panel: background, grid, data bars, reference line.
pub fn build_automation_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    view: &AutomationPanelView,
    lane: Option<&AutomationLane>,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
) {
    let w = width as f32;
    let h = height as f32;
    let kb_w = view.left_panel_width();
    let ppu = view.base.pixels_per_tick;

    // 1. Background
    instances.push(NoteInstance {
        x: kb_w,
        y: 0.0,
        w: w - kb_w,
        h,
        rgba_packed: pack_rgba(
            grid::PR_BG_COLOR.0,
            grid::PR_BG_COLOR.1,
            grid::PR_BG_COLOR.2,
            1.0,
        ),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        tag: 0,
    });

    // 2. Grid lines (vertical only — reuse the shared timeline grid builder)
    if let Some(tpb) = tpb {
        grid::build_timeline_grid(
            instances,
            w,
            h,
            &view.base,
            tpb,
            default_num,
            default_den,
            time_sig_events,
            grid::PR_MEASURE_LINE_COLOR,
            grid::PR_BEAT_LINE_COLOR,
            Some(grid::PR_SUB_BEAT_LINE_COLOR),
        );
    }

    // 3. Reference / center line
    if let Some(lane) = lane {
        let target = &lane.target;
        let max_val = target.max_value() as f32;
        if max_val > 0.0 && target.has_center_line() {
            let center_val = target.default_value() as f32;
            let y_center = h - (center_val / max_val) * h;
            // Draw a thin horizontal line across the panel
            instances.push(NoteInstance {
                x: kb_w,
                y: y_center - 0.5,
                w: w - kb_w,
                h: 1.0,
                rgba_packed: pack_rgba(
                    CENTER_LINE_COLOR.0,
                    CENTER_LINE_COLOR.1,
                    CENTER_LINE_COLOR.2,
                    CENTER_LINE_ALPHA,
                ),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }

        // 4. Data bars
        let (tick_start, tick_end) = view.base.visible_tick_range(w);
        // Add some padding for bars that extend past the viewport edge
        let tick_pad = (w - kb_w) / ppu;
        let pad_start = (tick_start - tick_pad as f64).max(0.0) as u32;
        let pad_end = (tick_end + tick_pad as f64) as u32;

        let events = lane.events_in_range(pad_start, pad_end);
        let x_offset = kb_w - view.base.scroll_x;

        for evt in events {
            let val = evt.value as f32;
            let bar_h = (val / max_val) * h;
            let bar_x = x_offset + evt.tick as f32 * ppu;

            // Skip if completely off-screen
            if bar_x + BAR_WIDTH < kb_w || bar_x > w {
                continue;
            }

            let trk = evt.track as usize % TRACK_PALETTE.len();
            let color = TRACK_PALETTE[trk];

            instances.push(NoteInstance {
                x: bar_x,
                y: h - bar_h,
                w: BAR_WIDTH,
                h: bar_h,
                rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                props_packed: pack_props(BAR_ROUNDING, BAR_BORDER),
                velocity: evt.value as u32,
                tag: 0,
            });
        }
    }
}
