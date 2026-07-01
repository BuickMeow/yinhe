use yinhe_types::{AutomationLane, AutomationTarget, NoteSource, TimeSigEvent};

use crate::PianorollRenderer;
use crate::automation_instances;
use crate::AutomationPanelView;
use crate::Uniforms;
use yinhe_wgpu::layer_cache_key;

fn target_hash(target: &AutomationTarget) -> u64 {
    match target {
        AutomationTarget::CC { controller } => *controller as u64,
        AutomationTarget::PitchBend => 1,
        AutomationTarget::Rpn { parameter } => 2 + *parameter as u64,
        AutomationTarget::Nrpn { parameter } => 2 + 0x10000 + *parameter as u64,
    }
}

/// Prepare an automation panel for rendering using the layered cache API.
///
/// Layers:
///   0 = decor (background + center line)
///   1 = grid lines
///   2 = data bars (or velocity bars when target is Velocity)
///
/// When `lanes` is empty and the panel target is Velocity, velocity bars are
/// rendered directly from `midi` instead of from an automation lane.
pub fn prepare(
    renderer: &mut PianorollRenderer,
    width: u32,
    height: u32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    midi: Option<&dyn NoteSource>,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    _force_rebuild: bool,
    scroll_mode: u32,
    min_border_width: f32,
    velocity_display_mode: u32,
    automation_display_mode: u32,
    automation_show_dots: bool,
) -> bool {
    let w = width as f32;
    let h = height as f32;
    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = match scroll_mode {
        0 => (scroll_x, 0.0),
        _ => {
            let f = scroll_x.floor();
            (f, scroll_x - f)
        },
    };

    let uniforms = Uniforms {
        width: w,
        height: h,
        scroll_x: scroll_x_pos,
        scroll_y: 0.0,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0,
        keyboard_width: view.base.left_panel_width,
        mode: 0, // pixel mode
        scroll_frac,
        scroll_mode,
        min_border_width,
    };

    renderer.upload_uniforms(uniforms);
    renderer.ensure_layers(3);

    let vh = view.render_hash();
    let wh = {
        let mut hash: u64 = 0;
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(w.to_bits() as u64);
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(h.to_bits() as u64);
        hash
    };

    // Layer 0: decor (background + center line)
    let center_line_hash = lanes
        .first()
        .map(|l| {
            if l.target.max_value() > 0 && l.target.has_center_line() {
                l.target.default_value() as u64
            } else {
                0
            }
        })
        .unwrap_or(0);
    let decor_key = layer_cache_key(&[vh, wh, center_line_hash, target_hash(&view.selected_target)]);
    renderer.upload_layer(0, decor_key, |out| {
        automation_instances::build_decor(out, w, h, lanes);
    });

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[vh, wh]);
    let mut sig_hash = 0u64;
    for ev in time_sig_events {
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.tick as u64);
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.numerator as u64);
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.denominator as u64);
    }
    grid_key = layer_cache_key(&[grid_key, sig_hash]);
    renderer.upload_layer(1, grid_key, |out| {
        automation_instances::build_grid(out, w, h, view, tpb, default_num, default_den, time_sig_events, scroll_x_pos);
    });

    // Layer 2: data bars (or velocity bars when show_velocity is true)
    let is_velocity = view.show_velocity;
    let tv_hash = {
        let mut h = 0u64;
        for &v in track_visible {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
        }
        h
    };
    let bars_key = layer_cache_key(&[
        vh, wh, tv_hash,
        velocity_display_mode as u64,
        target_hash(&view.selected_target),
        automation_display_mode as u64,
        automation_show_dots as u64,
    ]);
    renderer.upload_layer(2, bars_key, |out| {
        if is_velocity {
            if let Some(midi) = midi {
                automation_instances::build_velocity_bars(
                    out, w, h, midi, view, track_visible, track_colors, velocity_display_mode,
                );
            }
        } else if automation_display_mode == 1 {
            automation_instances::build_data_lines(
                out, w, h, view, lanes, track_visible, track_colors, automation_show_dots,
            );
        } else {
            automation_instances::build_data_bars(out, w, h, view, lanes, track_visible, track_colors);
        }
    });

    true
}
