use rayon::prelude::*;
use yinhe_types::{AutomationLane, AutomationTarget, NoteSource, SegmentShape, TimeSigEvent};

use super::data_lines;
use super::decor;
use super::ghost;
use super::tempo;
use super::velocity_bars;
use crate::renderer::InstanceRenderer;
use yinhe_types::AutomationPanelView;
use crate::vertex::Uniforms;
use crate::layer::layer_cache_key;

/// 拖拽预览（ghost）。由交互层每帧计算，传给 wgpu 在 ghost 层绘制。
///
/// 坐标为 panel 局部像素坐标（原点在 panel 左上角）。
#[derive(Clone, Debug)]
pub enum AutomationGhost {
    /// Pencil 拖拽锚点：整条 lane 用被拖事件的临时位置重新生成。
    /// 固定层完全跳过该 lane，由 ghost 层画完整覆盖后的 lane。
    Move {
        /// 覆盖后的完整 lane（已将被拖事件移动到新位置）。
        lane: AutomationLane,
        /// 音轨颜色（ghost 用 track color 而非黄色）。
        color: [f32; 3],
    },
    /// Curve 拖拽：从 `start` 到 `cur` 画预览线
    Curve { start_x: f32, start_y: f32, cur_x: f32, cur_y: f32, color: [f32; 3] },
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
fn hash_lane(lane: &AutomationLane) -> u64 {
    let mut h: u64 = 0;
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
    h
}

/// 计算 lanes 的 hash，但排除被 ghost 覆盖的 lane。
/// 这样拖拽过程中固定层 key 不变，只在开始/结束拖拽时变化。
fn hash_lanes_excluding(lanes: &[&AutomationLane], ghost: Option<&AutomationGhost>) -> u64 {
    let ghost_lane_key = match ghost {
        Some(AutomationGhost::Move { lane, .. }) => Some((lane.track, lane.target.clone())),
        _ => None,
    };
    lanes
        .par_iter()
        .fold(
            || 0u64,
            |h, lane| {
                if ghost_lane_key.as_ref() == Some(&(lane.track, lane.target.clone())) {
                    h
                } else {
                    h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(hash_lane(lane))
                }
            },
        )
        .reduce(|| 0u64, |a, b| a ^ b)
}

/// Prepare an automation panel for rendering using the layered cache API.
///
/// Layers:
///   0 = grid lines
///   1 = data lines (or velocity bars when target is Velocity, or tempo curve)
///
/// Background + center line are now drawn by egui before the wgpu texture.
///
/// When `lanes` is empty and the panel target is Velocity, velocity bars are
/// rendered directly from `midi` instead of from an automation lane.
///
/// `show_anchors`: 在每个事件位置画圆形锚点（铅笔工具下显示）。
/// `ghost`: 拖拽预览（Layer 2，每帧重建，无缓存）。
/// `highlight_tick`: 如果非 None，该 tick 位置的锚点渲染为白色高亮（选中锚点）。
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
    revision: u64,
    highlight_tick: Option<u32>,
) -> bool {
    let w = width as f32;
    let h = height as f32;
    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = crate::compute_scroll_frac(scroll_x, scroll_mode);

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
        note_outline: 1, // unused in pixel mode
        lane_height: 0.0, // unused in pixel mode
        note_alpha: 1.0, // unused in pixel mode (decor uses packed rgba)
    };

    renderer.upload_uniforms(uniforms);
    renderer.ensure_layers(3);

    let vh = view.render_hash();
    let wh = crate::hash_f32s(&[w, h]);

    // Layer 0: grid lines (background + center line now drawn by egui)
    let sig_hash = crate::hash_time_sigs(time_sig_events);
    let grid_key = layer_cache_key(&[vh, wh, sig_hash]);
    let theme = renderer.theme.clone();
    renderer.upload_layer(0, grid_key, |out| {
        decor::build_grid(
            out, w, h, view, tpb, default_num, default_den, time_sig_events, scroll_x_pos, &theme,
        );
    });

    // Layer 1: data lines (or velocity bars when show_velocity is true, or tempo curve)
    let is_velocity = view.show_velocity;
    let is_tempo = view.show_tempo;
    let tv_hash = crate::hash_bools(track_visible);
    // ghost_lane_hash：被 ghost 覆盖的 lane 内容变化时触发 Layer 1 重建。
    // 这样开始/结束拖拽时固定层会隐藏/恢复该 lane。
    let ghost_lane_hash = ghost.as_ref().map(|g| match g {
        AutomationGhost::Move { lane, .. } => hash_lane(lane),
        AutomationGhost::Curve { .. } => 1,
    }).unwrap_or(0);
    // 固定层排除 ghost lane 后计算 lanes_hash（避免拖拽时 lane 原数据未变但 key 变化）
    let fixed_lanes_hash = hash_lanes_excluding(lanes, ghost.as_ref());
    let bars_key = layer_cache_key(&[
        vh, wh, tv_hash,
        velocity_display_mode as u64,
        target_hash(&view.selected_target),
        show_anchors as u64,
        view.show_velocity as u64,
        view.show_tempo as u64,
        tempo_hash(tempo_events),
        fixed_lanes_hash,
        ghost_lane_hash,
        revision,
        highlight_tick.unwrap_or(u32::MAX) as u64,
    ]);
    let ghost_for_layer1 = ghost.clone();
    let highlight_tick_for_layer1 = highlight_tick;
    renderer.upload_layer(1, bars_key, |out| {
        if is_tempo {
            tempo::build_tempo_lines(out, w, h, view, tempo_events, &theme);
        } else if is_velocity {
            if let Some(midi) = midi {
                velocity_bars::build_velocity_bars(
                    out, w, h, midi, view, track_visible, track_colors, velocity_display_mode, &theme,
                );
            }
        } else {
            // 固定层跳过被 ghost 覆盖的 lane
            let skip_lane = match ghost_for_layer1 {
                Some(AutomationGhost::Move { ref lane, .. }) => Some(lane),
                _ => None,
            };
            data_lines::build_data_lines(
                out, w, h, view, lanes, track_visible, track_colors, show_anchors, skip_lane, highlight_tick_for_layer1, &theme,
            );
        }
    });

    // Layer 2: ghost (拖拽预览，无缓存，每帧重建)
    renderer.upload_layer(2, 0, |out| {
        if let Some(g) = ghost {
            ghost::build_ghost(out, g, w, view, show_anchors, &theme);
        }
    });

    true
}
