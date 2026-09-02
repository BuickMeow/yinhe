use yinhe_types::{AutomationLane, AutomationTarget, NoteSource, SegmentShape, TimelineViewBase};

use super::data_lines;
use super::ghost;
use super::velocity_bars;
use crate::layer::layer_cache_key;
use crate::renderer::InstanceRenderer;
use crate::vertex::Uniforms;
use yinhe_types::AutomationPanelView;

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
    Curve {
        start_x: f32,
        start_y: f32,
        cur_x: f32,
        cur_y: f32,
        color: [f32; 3],
    },
}

fn target_hash(target: &AutomationTarget) -> u64 {
    match target {
        AutomationTarget::CC { controller } => *controller as u64,
        AutomationTarget::PitchBend => 1,
        AutomationTarget::Rpn { parameter } => 2 + *parameter as u64,
        AutomationTarget::Nrpn { parameter } => 2 + 0x10000 + *parameter as u64,
        AutomationTarget::Tempo => u64::MAX,
    }
}

/// Hash automation lane 事件内容（tick + value + shape）。
/// 用于 ghost_lane_hash：拖拽过程中 ghost lane 不通过 Document 编辑，
/// revision 不会 bump，所以需要单独 hash ghost 自身内容来触发 Layer 1 重建。
fn hash_lane(lane: &AutomationLane) -> u64 {
    let mut h: u64 = 0;
    h = h
        .wrapping_mul(0x9e3779b97f4a7c15)
        .wrapping_add(lane.events.len() as u64);
    for e in &lane.events {
        h = h
            .wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add(e.tick as u64);
        h = h
            .wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add(e.value.to_bits() as u64);
        let shape_bits = match e.shape {
            SegmentShape::Step => 0u64,
            SegmentShape::Curve { x1, y1, x2, y2 } => {
                1 + (x1.to_bits() as u64)
                    .wrapping_mul(0x9e3779b97f4a7c15)
                    .wrapping_add(y1.to_bits() as u64)
                    .wrapping_mul(0x9e3779b97f4a7c15)
                    .wrapping_add(x2.to_bits() as u64)
                    .wrapping_mul(0x9e3779b97f4a7c15)
                    .wrapping_add(y2.to_bits() as u64)
            }
        };
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(shape_bits);
    }
    h
}

/// Prepare an automation panel for rendering using the layered cache API.
///
/// Layers:
///   0 = data lines (or velocity bars when target is Velocity, or tempo curve)
///   1 = ghost (拖拽预览)
///
/// Grid lines 不再由 automation 面板绘制：automation 共享 pianoroll 顶部的时间标尺，
/// 标尺已经提供了"线 + 标签"的视觉锚点，面板内不再补 grid。
/// Background + center line 由 egui 在 wgpu 纹理前绘制。
///
/// When `lanes` is empty and the panel target is Velocity, velocity bars are
/// rendered directly from `midi` instead of from an automation lane.
///
/// `show_anchors`: 在每个事件位置画圆形锚点（铅笔工具下显示）。
/// `ghost`: 拖拽预览（Layer 1，每帧重建，无缓存）。
/// `highlight_ticks`: 这些 tick 位置的锚点渲染为白色高亮（选中锚点，可多选）。
///
/// `max_val`: 当前 panel 的值域上界。Tempo 由调用方按实际事件动态计算，
///            其他 target 直接传 `target.max_value()`。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn prepare(
    renderer: &mut InstanceRenderer,
    width: u32,
    height: u32,
    view: &AutomationPanelView,
    lanes: &[&AutomationLane],
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    min_border_width: f32,
    show_anchors: bool,
    max_val: f32,
    ghost: Option<AutomationGhost>,
    revision: u64,
    highlight_ticks: &[u32],
) -> bool {
    let w = width as f32;
    let h = height as f32;
    let scroll_x = view.base.scroll_x;

    // Build track colors in GPU format (vec4) — needed for velocity pipeline
    // which fetches color via `tc[track]` in the shader.
    let tc_colors: Vec<[f32; 4]> = track_colors.to_vec();
    let track_count = tc_colors.len() as u32;

    let uniforms = Uniforms {
        width: w,
        height: h,
        scroll_x,
        scroll_y: 0.0,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0,
        keyboard_width: view.base.left_panel_width,
        mode: 0, // pixel mode (automation uses rgba_packed directly)
        min_border_width,
        track_count,       // used by velocity pipeline for tc[track] bounds check
        sel_rect_count: 0, // unused in pixel mode
        note_outline: 1,   // unused in pixel mode
        lane_height: 0.0,  // unused in pixel mode
        value_zoom: view.value_zoom,
        value_scroll: view.value_scroll,
        orientation: 0, // automation 面板恒为横向像素模式
    };

    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(&tc_colors);
    // Grid 已迁移到 egui（automation 面板不补 grid，共享 pianoroll 顶部 ruler），
    // wgpu 只剩 data + ghost 两层。
    renderer.ensure_layers(2);

    let vh = view.render_hash();
    let wh = crate::hash_f32s(&[w, h]);

    // Layer 0: data lines (curve pipeline) — or velocity bars (velocity pipeline)
    // Tempo 走和 CC/PB/RPN 一样的 curve pipeline，由调用方在 `lanes` 中
    // 传入 `conductor.tempo` lane。
    let is_velocity = view.show_velocity;
    let tv_hash = crate::hash_bools(track_visible);
    // Conductor 颜色跟随主题主文字（text_primary）变化时，track_colors 会改变
    // （曲线颜色烘焙在 CurveInstance 里，不在 tc 缓冲），必须让 bars_key 失效重建
    let tc_hash = {
        let mut h: u64 = 0;
        for c in track_colors {
            h = h
                .wrapping_mul(0x9e3779b97f4a7c15)
                .wrapping_add(crate::hash_f32s(c));
        }
        h
    };
    // ghost_lane_hash：被 ghost 覆盖的 lane 内容变化时触发 Layer 0 重建。
    // 拖拽过程中 ghost 不通过 Document 编辑，revision 不会 bump，所以需要单独 hash。
    let ghost_lane_hash = ghost
        .as_ref()
        .map(|g| match g {
            AutomationGhost::Move { lane, .. } => hash_lane(lane),
            AutomationGhost::Curve { .. } => 1,
        })
        .unwrap_or(0);
    // 固定层 lane 内容变化由 revision 检测：所有 lane 编辑路径
    // (add/move/delete/set_shape/arrange_move/apply_automation_delta) 都 bump revision。
    // 拖拽 ghost 时 revision 不变，固定层 cache 复用——正是想要的行为。
    // 之前这里有 O(全事件数) 的 fixed_lanes_hash，与 revision 双重检测，纯冗余，已删除。
    let bars_key = layer_cache_key(&[
        vh,
        wh,
        tv_hash,
        tc_hash,
        target_hash(&view.selected_target),
        show_anchors as u64,
        view.show_velocity as u64,
        ghost_lane_hash,
        revision,
        highlight_ticks
            .iter()
            .fold(0u64, |acc, &t| acc.wrapping_mul(31).wrapping_add(t as u64)),
    ]);
    let ghost_for_layer0 = ghost.clone();
    let highlight_ticks_for_layer0 = highlight_ticks;
    let theme = renderer.theme();

    if is_velocity {
        // Velocity bars via velocity pipeline (VelocityBarInstance, 16B)
        if let Some(midi) = midi {
            renderer.upload_velocity_layer(0, bars_key, |out| {
                velocity_bars::build_velocity_bars(out, w, midi, view, track_visible);
            });
        }
    } else {
        // Data lines + anchors via curve pipeline (CurveInstance)
        // Tempo 与 CC/PB/RPN 共用此路径；max_val 由调用方传入。
        renderer.upload_curve_layer(0, bars_key, |out| {
            let skip_lane = match ghost_for_layer0 {
                Some(AutomationGhost::Move { ref lane, .. }) => Some(lane),
                _ => None,
            };
            data_lines::build_data_lines(
                out,
                w,
                h,
                view,
                lanes,
                max_val,
                track_visible,
                track_colors,
                show_anchors,
                skip_lane,
                highlight_ticks_for_layer0,
                &theme,
            );
        });
    }

    // Layer 1: ghost (拖拽预览，无缓存，每帧重建) — curve pipeline
    renderer.upload_curve_layer(1, 0, |out| {
        if let Some(g) = ghost {
            ghost::build_ghost(out, g, w, view, max_val, show_anchors, &theme);
        }
    });

    true
}

/// AR 共享纹理中一条可见的自动化 lane。
pub struct ArrAutomationLane<'a> {
    pub lane: &'a AutomationLane,
    /// 子行顶部在 AR 纹理中的 y（像素）。
    pub y_top: f32,
    /// 子行高（= 音轨行高）。
    pub height: f32,
    /// 值域上限（Tempo 由调用方按事件动态算，其他 target.max_value()）。
    pub max_val: f32,
    /// 需要白色高亮的锚点 tick（选中的锚点）。
    pub highlight_ticks: &'a [u32],
}

/// Prepare AR 展开自动化 lane 的曲线渲染（画在 AR 共享走带纹理上）。
///
/// Layers:
///   2 = 数据层（各展开 lane 的线段 + 锚点，按 cache_key 缓存）
///   3 = ghost（拖拽预览，无缓存，每帧重建）
///
/// 不碰 uniforms / track_colors：AR 的 view_ui 已上传；curve shader 只用
/// width/height/scroll，与 AR 的 uniforms 兼容。
///
/// cache_key：调用方算（含 render 相关 hash + revision——任何编辑都 bump
/// revision，展开/布局变化进布局 hash）。ghost 覆盖 lane 的内容 hash 在本函数
/// 内部额外混入数据层 key（拖拽不 bump revision，见 prepare 的 ghost_lane_hash）。
///
/// ghost：(ghost, y_top, height, max_val)——ghost lane 画在哪个子行。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn prepare_arr_automation(
    renderer: &mut InstanceRenderer,
    width: f32,
    height: f32,
    base: &TimelineViewBase,
    lanes: &[ArrAutomationLane],
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    show_anchors: bool,
    ghost: Option<(AutomationGhost, f32, f32, f32)>,
    cache_key: u64,
) {
    // ghost_lane_hash：拖拽 ghost 不经 Document 编辑、revision 不 bump，
    // 需单独 hash ghost lane 内容以触发数据层重建（逻辑同 prepare）。
    let ghost_lane_hash = ghost
        .as_ref()
        .map(|(g, ..)| match g {
            AutomationGhost::Move { lane, .. } => hash_lane(lane),
            AutomationGhost::Curve { .. } => 1,
        })
        .unwrap_or(0);
    // Conductor 颜色跟随主题主文字变化时，曲线烘焙色需重建（同 prepare 的 tc_hash）
    let tc_hash = {
        let mut h: u64 = 0;
        for c in track_colors {
            h = h
                .wrapping_mul(0x9e3779b97f4a7c15)
                .wrapping_add(crate::hash_f32s(c));
        }
        h
    };
    let data_key = layer_cache_key(&[cache_key, ghost_lane_hash, tc_hash]);

    // 数据层 skip_lane：ghost 为 Move 时被覆盖的 lane 由 ghost 层完整重画。
    let skip_lane = ghost.as_ref().and_then(|(g, ..)| match g {
        AutomationGhost::Move { lane, .. } => Some(lane),
        AutomationGhost::Curve { .. } => None,
    });

    let theme = renderer.theme();

    // Layer 2: 数据层——每条可见 lane 构造一个临时 panel view，
    // y_offset 把曲线平移到所属子行顶部（upload_curve_layer 内部已
    // ensure_layer(2, Curve)，无需显式调用）。
    renderer.upload_curve_layer(2, data_key, |out| {
        for l in lanes {
            let view = AutomationPanelView {
                base: base.clone(),
                panel_height: l.height,
                y_offset: l.y_top,
                selected_target: l.lane.target.clone(),
                show_velocity: false,
                value_zoom: 1.0,
                value_scroll: 0.0,
                ..Default::default()
            };
            data_lines::build_data_lines(
                out,
                width,
                height,
                &view,
                &[l.lane],
                l.max_val,
                track_visible,
                track_colors,
                show_anchors,
                skip_lane,
                l.highlight_ticks,
                &theme,
            );
        }
    });

    // Layer 3: ghost（拖拽预览，无缓存，每帧重建）。
    renderer.upload_curve_layer(3, 0, |out| {
        if let Some((g, y_top, h, max_val)) = ghost {
            let view = AutomationPanelView {
                base: base.clone(),
                panel_height: h,
                y_offset: y_top,
                show_velocity: false,
                value_zoom: 1.0,
                value_scroll: 0.0,
                ..Default::default()
            };
            ghost::build_ghost(out, g, width, &view, max_val, show_anchors, &theme);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：切换主题后 Conductor 曲线颜色变化必须使 layer cache 失效
    /// （track_colors 烘焙在 CurveInstance 里，bars_key 必须包含 tc_hash）
    #[test]
    fn automation_bars_key_changes_with_conductor_color() {
        let wh = crate::hash_f32s(&[800.0, 80.0]);
        let tv_hash = crate::hash_bools(&[true, true, true]);
        let target = target_hash(&yinhe_types::AutomationTarget::Tempo);
        let dark = [220.0 / 255.0, 220.0 / 255.0, 220.0 / 255.0, 1.0];
        let light = [30.0 / 255.0, 30.0 / 255.0, 34.0 / 255.0, 1.0];
        let dark_hash = {
            let mut h = 0u64;
            for c in [dark] {
                h = h
                    .wrapping_mul(0x9e3779b97f4a7c15)
                    .wrapping_add(crate::hash_f32s(&c));
            }
            h
        };
        let light_hash = {
            let mut h = 0u64;
            for c in [light] {
                h = h
                    .wrapping_mul(0x9e3779b97f4a7c15)
                    .wrapping_add(crate::hash_f32s(&c));
            }
            h
        };
        assert_ne!(
            dark_hash, light_hash,
            "不同 Conductor 颜色 tc_hash 必须不同"
        );
        let vh = 12345u64;
        let bars_dark = layer_cache_key(&[vh, wh, tv_hash, dark_hash, target, 1, 0, 0, 0, 0]);
        let bars_light = layer_cache_key(&[vh, wh, tv_hash, light_hash, target, 1, 0, 0, 0, 0]);
        assert_ne!(
            bars_dark, bars_light,
            "bars_key 必须包含 tc_hash，否则主题切换后曲线沿用旧纹理"
        );
        // AR 数据层同理
        let data_dark = layer_cache_key(&[vh, 0, dark_hash]);
        let data_light = layer_cache_key(&[vh, 0, light_hash]);
        assert_ne!(data_dark, data_light);
    }
}
