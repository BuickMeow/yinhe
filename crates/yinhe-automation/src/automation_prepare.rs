use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::PianorollRenderer;
use crate::automation_instances;
use crate::AutomationPanelView;
use crate::Uniforms;
use yinhe_wgpu::layer_cache_key;

/// Prepare an automation panel for rendering using the layered cache API.
///
/// Layers:
///   0 = decor (background + center line)
///   1 = grid lines
///   2 = data bars
pub fn prepare(
    renderer: &mut PianorollRenderer,
    width: u32,
    height: u32,
    view: &AutomationPanelView,
    lane: Option<&AutomationLane>,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    _force_rebuild: bool,
) -> bool {
    let w = width as f32;
    let h = height as f32;

    let uniforms = Uniforms {
        width: w,
        height: h,
        scroll_x: view.base.scroll_x,
        scroll_y: 0.0,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0,
        keyboard_width: view.base.left_panel_width,
        _pad: 0.0,
    };

    renderer.upload_uniforms(uniforms);
    renderer.ensure_layers(3);

    // Layer 0: decor (background + center line)
    let center_line_hash = lane
        .map(|l| {
            if l.target.max_value() > 0 && l.target.has_center_line() {
                l.target.default_value() as u64
            } else {
                0
            }
        })
        .unwrap_or(0);
    let decor_key = layer_cache_key(&[
        w.to_bits() as u64,
        h.to_bits() as u64,
        center_line_hash,
    ]);
    renderer.upload_layer(0, decor_key, |out| {
        automation_instances::build_decor(out, w, h, lane);
    });

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[
        view.base.scroll_x.to_bits() as u64,
        view.base.pixels_per_tick.to_bits() as u64,
        w.to_bits() as u64,
        h.to_bits() as u64,
        view.base.left_panel_width.to_bits() as u64,
    ]);
    let mut sig_hash = 0u64;
    for ev in time_sig_events {
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.tick as u64);
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.numerator as u64);
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.denominator as u64);
    }
    grid_key = layer_cache_key(&[grid_key, sig_hash]);
    renderer.upload_layer(1, grid_key, |out| {
        automation_instances::build_grid(out, w, h, view, tpb, default_num, default_den, time_sig_events);
    });

    // Layer 2: data bars
    let tv_hash = {
        let mut h = 0u64;
        for &v in track_visible {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
        }
        h
    };
    let bars_key = layer_cache_key(&[
        view.base.scroll_x.to_bits() as u64,
        view.base.pixels_per_tick.to_bits() as u64,
        w.to_bits() as u64,
        h.to_bits() as u64,
        view.base.left_panel_width.to_bits() as u64,
        tv_hash,
    ]);
    renderer.upload_layer(2, bars_key, |out| {
        automation_instances::build_data_bars(out, w, h, view, lane, track_visible, track_colors);
    });

    true
}
