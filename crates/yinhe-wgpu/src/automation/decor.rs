use yinhe_types::AutomationPanelView;
use crate::grid;
use yinhe_types::TimeSigEvent;
use crate::vertex::DrawInstance;

/// Build grid line instances (layer 0).
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
            Some(theme.pr_tick_line),
            scroll_x_pixel,
        );
    }
}
