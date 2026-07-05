use yinhe_types::{AutomationLane, AutomationTarget, NoteSource, SegmentShape, TimeSigEvent};

use crate::InstanceRenderer;
use crate::automation_instances;
use crate::AutomationPanelView;
use crate::Uniforms;
use yinhe_wgpu::layer_cache_key;

/// 拖拽预览（ghost）。由交互层每帧计算，传给 wgpu 在 ghost 层绘制。
///
/// 坐标为 panel 局部像素坐标（原点在 panel 左上角）。
#[derive(Clone, Copy, Debug)]
pub enum AutomationGhost {
    /// Pencil 拖拽锚点：从 `old` 位置移到 `cur` 位置
    Move { old_x: f32, old_y: f32, cur_x: f32, cur_y: f32 },
    /// Curve 拖拽：从 `start` 到 `cur` 画预览线
    Curve { start_x: f32, start_y: f32, cur_x: f32, cur_y: f32 },
}

fn target_hash(target: &AutomationTarget) -> u64 {
    match target {
        AutomationTarget::CC { controller } => *controller as u64,
        AutomationTarget::PitchBend => 1,
        AutomationTarget::Rpn { parameter } => 2 + *parameter as u64,
        AutomationTarget::Nrpn { parameter } => 2 + 0x10000 + *parameter as u64,
    }
}

fn tempo_hash(tempo_events: &[(u32, f64)]) -> u64 {
    let mut h: u64 = 0;
    for (tick, bpm) in tempo_events {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(*tick as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(bpm.to_bits());
    }
    h
}

/// Hash automation lane 事件内容（tick + value + shape）。
/// 用于 Layer 2 cache key：任何 Add/Move/Delete/CycleShape 后 key 变化 → 重建。
fn hash_lanes(lanes: &[&AutomationLane]) -> u64 {
    let mut h: u64 = 0;
    for lane in lanes {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(lane.events.len() as u64);
        for e in &lane.events {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(e.tick as u64);
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(e.value as u64);
            let shape_bits = match e.shape {
                SegmentShape::Step => 0u64,
                SegmentShape::Curve { tension } => 1 + (tension as u8 as u64),
            };
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(shape_bits);
        }
    }
    h
}

/// Prepare an automation panel for rendering using the layered cache API.
///
/// Layers:
///   0 = decor (background + center line)
///   1 = grid lines
///   2 = data lines (or velocity bars when target is Velocity, or tempo curve)
///
/// When `lanes` is empty and the panel target is Velocity, velocity bars are
/// rendered directly from `midi` instead of from an automation lane.
///
/// `show_anchors`: 在每个事件位置画圆形锚点（铅笔工具下显示）。
/// `ghost`: 拖拽预览（Layer 3，每帧重建，无缓存）。
pub fn prepare(
    renderer: &mut InstanceRenderer,
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
    scroll_mode: u32,
    min_border_width: f32,
    velocity_display_mode: u32,
    show_anchors: bool,
    tempo_events: &[(u32, f64)],
    ghost: Option<AutomationGhost>,
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
        mode: 0, // pixel mode (automation uses rgba_packed directly)
        scroll_frac,
        scroll_mode,
        min_border_width,
        track_count: 0, // unused in pixel mode
        sel_rect_count: 0, // unused in pixel mode
        note_selection_highlight: 0, // automation: no note selection highlight
        lane_height: 0.0, // unused in pixel mode
        note_alpha: 1.0, // unused in pixel mode (decor uses packed rgba)
    };

    renderer.upload_uniforms(uniforms);
    renderer.ensure_layers(4);

    let vh = view.render_hash();
    let wh = yinhe_wgpu::hash_f32s(&[w, h]);

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
    let theme = renderer.theme.clone();
    renderer.upload_layer(0, decor_key, |out| {
        automation_instances::build_decor(out, w, h, lanes, &theme);
    });

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[vh, wh]);
    let mut sig_hash = 0u64;
    for ev in time_sig_events {
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.tick as u64);
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.numerator as u64);
        sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.denominator as u64);
    }
    grid_key = layer_cache_key(&[vh, wh, sig_hash]);
    renderer.upload_layer(1, grid_key, |out| {
        automation_instances::build_grid(
            out, w, h, view, tpb, default_num, default_den, time_sig_events, scroll_x_pos, &theme,
        );
    });

    // Layer 2: data lines (or velocity bars when show_velocity is true, or tempo curve)
    let is_velocity = view.show_velocity;
    let is_tempo = view.show_tempo;
    let tv_hash = yinhe_wgpu::hash_bools(track_visible);
    // lanes 事件内容 hash：任何 Add/Move/Delete/CycleShape 后 key 变化 → Layer 2 重建
    let lanes_hash = hash_lanes(lanes);
    let bars_key = layer_cache_key(&[
        vh, wh, tv_hash,
        velocity_display_mode as u64,
        target_hash(&view.selected_target),
        show_anchors as u64,
        view.show_velocity as u64,
        view.show_tempo as u64,
        tempo_hash(tempo_events),
        lanes_hash,
    ]);
    renderer.upload_layer(2, bars_key, |out| {
        if is_tempo {
            automation_instances::build_tempo_lines(out, w, h, view, tempo_events, &theme);
        } else if is_velocity {
            if let Some(midi) = midi {
                automation_instances::build_velocity_bars(
                    out, w, h, midi, view, track_visible, track_colors, velocity_display_mode, &theme,
                );
            }
        } else {
            automation_instances::build_data_lines(
                out, w, h, view, lanes, track_visible, track_colors, show_anchors, &theme,
            );
        }
    });

    // Layer 3: ghost (拖拽预览，无缓存，每帧重建)
    renderer.upload_layer_force(3, |out| {
        if let Some(g) = ghost {
            automation_instances::build_ghost(out, g, &theme);
        }
    });

    true
}
