use yinhe_types::AutomationPanelView;
use crate::grid;
use yinhe_theme::GpuTheme;
use yinhe_types::{AutomationLane, TimeSigEvent};
use crate::vertex::DrawInstance;

/// Build background + center line instances (layer 0).
/// Dependencies: none (background is static), lane target (center line)
pub fn build_decor(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    theme: &GpuTheme,
) {
    out.push(DrawInstance::solid_rect(
        0.0, 0.0, w, h,
        [theme.pr_bg.0, theme.pr_bg.1, theme.pr_bg.2, 1.0],
    ));

    if let Some(lane) = lanes.first() {
        let target = &lane.target;
        let max_val = target.max_value() as f32;
        if max_val > 0.0 && target.has_center_line() {
            let center_val = target.default_value() as f32;
            let y_center = view.value_to_y(center_val, max_val);
            out.push(DrawInstance::solid_rect(
                0.0, y_center - 0.5, w, 1.0,
                [theme.center_line.0, theme.center_line.1, theme.center_line.2, theme.center_line.3],
            ));
        }
    }
}

/// Build grid line instances (layer 1).
/// Dependencies: scroll_x, pixels_per_tick, time_sig
pub fn build_grid(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &AutomationPanelView,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    scroll_x_pixel: f32,
    theme: &yinhe_theme::GpuTheme,
) {
    if let Some(tpb) = tpb {
        grid::build_timeline_grid(
            out,
            w,
            h,
            &view.base,
            tpb,
            default_num,
            default_den,
            time_sig_events,
            theme.pr_measure_line,
            theme.pr_beat_line,
            Some(theme.pr_sub_beat_line),
            scroll_x_pixel,
        );
    }
}
