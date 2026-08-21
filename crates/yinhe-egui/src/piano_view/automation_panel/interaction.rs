//! Automation panel mouse interaction logic (pencil/curve tools, right-click).

use eframe::egui;

use yinhe_types::{AnchorSelRect, AutomationPanelView};
use yinhe_types::{AutomationLane, AutomationTarget, SegmentShape};
use yinhe_wgpu::{
    AutomationGhost, build_lane_multi_copy, build_lane_multi_move, build_lane_override,
    build_lane_shape_override,
};

use super::constants::{ANCHOR_HIT_PX, HOVER_DELAY, MARQUEE_THRESHOLD};
use super::types::AutomationEditCtx;
use super::value::panel_max_val;
use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;

/// 计算两个 sel_rect 的并集（用于 Shift/Cmd+点击或框选扩展选区）。
/// - tick 范围：取 min/max
/// - value 范围：若任一为 None（垂直全选），结果为 None；否则取 min/max
fn union_anchor_sel_rect(a: AnchorSelRect, b: AnchorSelRect) -> AnchorSelRect {
    let ts = a
        .tick_start
        .min(a.tick_end)
        .min(b.tick_start)
        .min(b.tick_end);
    let te = a
        .tick_start
        .max(a.tick_end)
        .max(b.tick_start)
        .max(b.tick_end);
    let value_range = match (a.value_range, b.value_range) {
        (None, _) | (_, None) => None,
        (Some((va1, va2)), Some((vb1, vb2))) => {
            let vmin = va1.min(va2).min(vb1).min(vb2);
            let vmax = va1.max(va2).max(vb1).max(vb2);
            Some((vmin, vmax))
        }
    };
    AnchorSelRect {
        tick_start: ts,
        tick_end: te,
        value_range,
    }
}

/// Hover/drag tooltip 数据。锚点和控制点用不同的显示内容。
#[derive(Clone, Copy, Debug)]
pub(crate) enum HoverTooltip {
    /// 锚点（或拖拽锚点）：显示 tick（小节:拍:tick）+ automation value
    Anchor {
        tick: u32,
        value: f32,
        pos: egui::Pos2,
    },
    /// 贝塞尔控制点（或拖拽控制点）：显示 CSS 风格 4 值 (x1, y1, x2, y2)
    ControlPoint {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        pos: egui::Pos2,
    },
}

/// 控制点端别：cubic Bézier 有两个控制点，分别是 P1（起点出）和 P2（终点入）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CtrlEnd {
    /// 第一控制点 P1 = P0 + (P3-P0) * (x1, y1)，对应 CSS `cubic-bezier(x1, y1, _, _)`
    Out,
    /// 第二控制点 P2 = P0 + (P3-P0) * (x2, y2)，对应 CSS `cubic-bezier(_, _, x2, y2)`
    In,
}

/// 命中曲线控制点的结果（`dist_sq` 仅用于选最近命中，调用方不消费）。
#[derive(Clone, Copy)]
pub(crate) struct ControlPointHit {
    prev_tick: u32,
    which: CtrlEnd,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    pos: egui::Pos2,
    dist_sq: f32,
}

/// 拖拽状态（ghost）。存在 egui data 中，跨帧保持。
#[derive(Clone, Copy, Debug)]
pub(crate) enum AutoDrag {
    /// Pencil 拖拽锚点：`old_tick` 是原始位置，`start_tick/start_value` 是按下时的锚点原始值
    /// （用于判断是否实际移动过，避免单击时产生空 Move）
    MoveAnchor {
        old_tick: u32,
        start_tick: u32,
        start_value: f32,
    },
    /// Curve 拖拽：起点已固定
    CurveDraw { start_tick: u32, start_value: f32 },
    /// 拖拽 Curve 段的某个控制点。
    /// `prev_tick`：被拖段的前驱事件 tick（段的起点，shape 存于此事件）。
    /// `which`：拖的是 P1（Out）还是 P2（In）。
    /// `start`：按下时该控制点的归一化 (x, y) 位置，用于判断是否实际移动过。
    DragControlPoint {
        prev_tick: u32,
        which: CtrlEnd,
        start_x: f32,
        start_y: f32,
    },
    /// Select 工具拖拽多个选中锚点。`start_tick/start_value` 是按下时鼠标的 snapped 位置，
    /// 用于计算 delta。`alt` = Option 拖拽（复制而非移动）。
    MoveAnchors {
        start_tick: u32,
        start_value: f32,
        alt: bool,
    },
    /// Select 工具框选锚点。
    /// `start_pos`：按下时的屏幕位置，用于 3px 阈值判断 + 框选矩形计算。
    MarqueeSelect { start_pos: egui::Pos2 },
    /// Eraser 工具框选删除锚点（矩形内锚点在释放时删除）。
    EraserMarquee { start_pos: egui::Pos2 },
}

/// 持续化选框变更操作。
#[derive(Clone, Debug)]
pub(crate) enum SelRectOp {
    /// 替换所有选框为单个新选框（非 shift 框选完成 / 点击锚点设置单点选框）
    Set(AnchorSelRect),
    /// 追加一个新选框（shift+框选完成时累加）
    Append(AnchorSelRect),
    /// 替换所有选框为一组新选框（如多选框整体偏移后回写）
    ReplaceAll(Vec<AnchorSelRect>),
    /// 保持现有选框
    Keep,
}

/// Select 工具的选区变更操作（由 interaction 返回，caller 应用到 `panel`）。
#[derive(Clone, Debug)]
pub(crate) enum SelOp {
    /// 设置选框（替换或新建）
    Set(SelRectOp),
    /// 清空选框（点击空白处 < 3px）
    Clear,
    /// 开始新的框选（非加选模式 press）：清空共享音符选区（doc.edit.selected），
    /// 触发 App 层三视图选框互斥，使其他视图的选框立即消失。
    ClearNoteSelection,
}

/// 右键点击锚点时记录的编辑信息。
#[derive(Clone, Debug)]
pub(crate) struct RightClickAnchor {
    pub track_idx: u16,
    pub lane_idx: usize,
    pub old_tick: u32,
    pub target: AutomationTarget,
}

/// 检测鼠标是否悬停在两个锚点之间的线段上。
///
/// 如果鼠标位置在插值线附近（阈值 8 像素），返回 `true`。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn hit_line_on_lane(
    lane: &AutomationLane,
    tick: u32,
    value: f32,
    _ppu: f32,
    _scroll_x: f32,
    _grid_min_x: f32,
    _panel_min_y: f32,
    panel: &AutomationPanelView,
    max_val: f32,
) -> bool {
    // 找 bracket tick 的两个事件
    let idx = lane.events.partition_point(|e| e.tick <= tick);
    if idx == 0 || idx >= lane.events.len() {
        return false; // 左侧无事件或右侧无事件
    }
    let left = &lane.events[idx - 1];
    let right = &lane.events[idx];

    // 计算插值值
    let t = if right.tick == left.tick {
        0.0
    } else {
        (tick - left.tick) as f32 / (right.tick - left.tick) as f32
    };
    let interp = left.shape.interpolate(t);
    let interp_value = left.value + interp * (right.value - left.value);

    // 转换为像素坐标并检查距离
    let interp_y = panel.value_to_y(interp_value, max_val);
    let mouse_y = panel.value_to_y(value, max_val);
    (interp_y - mouse_y).abs() <= 8.0
}

/// 检测鼠标是否悬停在 Curve 段的两个空心圆控制点之一上。
///
/// 遍历所有 Curve 段（非直线），计算两个控制点屏幕位置（偏移量 *4 放大，
/// P1 相对 P0、P2 相对 P3），返回最近控制点所属段的前驱事件 tick + 端别 +
/// 该段的 4 个 ctrl 值 + 控制点像素位置。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn hit_control_point_on_lane(
    lane: &AutomationLane,
    mouse: egui::Pos2,
    ppu: f32,
    scroll_x: f32,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    max_val: f32,
) -> Option<ControlPointHit> {
    let x_offset = grid_area.min.x - scroll_x;
    let hit_sq = ANCHOR_HIT_PX * ANCHOR_HIT_PX;
    let mut best: Option<ControlPointHit> = None;
    for i in 1..lane.events.len() {
        let prev = &lane.events[i - 1];
        let cur = &lane.events[i];
        let SegmentShape::Curve { x1, y1, x2, y2 } = prev.shape else {
            continue;
        };
        if prev.shape.is_linear() {
            continue;
        }

        let px0 = x_offset + prev.tick as f32 * ppu;
        let py0 = panel_rect.min.y + panel.value_to_y(prev.value, max_val);
        let px3 = x_offset + cur.tick as f32 * ppu;
        let py3 = panel_rect.min.y + panel.value_to_y(cur.value, max_val);

        // 两个控制点屏幕坐标（偏移量 *4：P1 相对 P0，P2 相对 P3）
        let c1x = px0 + (px3 - px0) * x1 * 4.0;
        let c1y = py0 + (py3 - py0) * y1 * 4.0;
        let c2x = px3 + (px3 - px0) * x2 * 4.0;
        let c2y = py3 + (py3 - py0) * y2 * 4.0;

        // 分别检测两个控制点
        let d1 = (c1x - mouse.x).powi(2) + (c1y - mouse.y).powi(2);
        let d2 = (c2x - mouse.x).powi(2) + (c2y - mouse.y).powi(2);
        if d1 <= hit_sq && best.as_ref().map(|b| d1 < b.dist_sq).unwrap_or(true) {
            best = Some(ControlPointHit {
                prev_tick: prev.tick,
                which: CtrlEnd::Out,
                x1,
                y1,
                x2,
                y2,
                pos: egui::pos2(c1x, c1y),
                dist_sq: d1,
            });
        }
        if d2 <= hit_sq && best.as_ref().map(|b| d2 < b.dist_sq).unwrap_or(true) {
            best = Some(ControlPointHit {
                prev_tick: prev.tick,
                which: CtrlEnd::In,
                x1,
                y1,
                x2,
                y2,
                pos: egui::pos2(c2x, c2y),
                dist_sq: d2,
            });
        }
    }
    best
}

/// 从鼠标屏幕位置反推 Curve 段某一端控制点的偏移量 `(x, y) ∈ [-0.5, 0.5]`。
///
/// 偏移量参数化（内部 *4 放大）：
/// - `Out`（P1）：相对 P0，`offset = (mouse - P0) / (P3 - P0) / 4`
/// - `In`（P2）：相对 P3，`offset = (mouse - P3) / (P3 - P0) / 4`
///
/// 段水平或竖直时（dx/dy ≈ 0），对应分量为 0（与直线控制点对齐）。
/// 返回 `None` 表示 `prev_tick` 不存在或没有下一个事件。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn compute_ctrl_from_mouse(
    lane: &AutomationLane,
    prev_tick: u32,
    which: CtrlEnd,
    mouse: egui::Pos2,
    ppu: f32,
    scroll_x: f32,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    max_val: f32,
) -> Option<(f32, f32)> {
    let prev_idx = lane.events.iter().position(|e| e.tick == prev_tick)?;
    let prev = &lane.events[prev_idx];
    let next = lane.events.get(prev_idx + 1)?;
    let x_offset = grid_area.min.x - scroll_x;
    let px0 = x_offset + prev.tick as f32 * ppu;
    let py0 = panel_rect.min.y + panel.value_to_y(prev.value, max_val);
    let px3 = x_offset + next.tick as f32 * ppu;
    let py3 = panel_rect.min.y + panel.value_to_y(next.value, max_val);
    let dx = px3 - px0;
    let dy = py3 - py0;
    // 参考点：Out 用 P0，In 用 P3
    let (rx, ry) = match which {
        CtrlEnd::Out => (px0, py0),
        CtrlEnd::In => (px3, py3),
    };
    // offset = (mouse - ref) / seg / 4。
    // x 方向 clamp 到 CSS 单调区间：P1.x ∈ [0,1] → x1 ∈ [0, 0.25]；
    // P2.x ∈ [0,1] → x2 ∈ [-0.25, 0]。保证 x(u) 单调，渲染=音频=命中三者一致。
    let x_range = match which {
        CtrlEnd::Out => (0.0, 0.25),
        CtrlEnd::In => (-0.25, 0.0),
    };
    let new_x = if dx.abs() < 1e-3 {
        0.0
    } else {
        ((mouse.x - rx) / dx / 4.0).clamp(x_range.0, x_range.1)
    };
    // y 方向不限单调（automation 值可任意变化），clamp 到 [-0.5, 0.5]
    let new_y = if dy.abs() < 1e-3 {
        0.0
    } else {
        ((mouse.y - ry) / dy / 4.0).clamp(-0.5, 0.5)
    };
    Some((new_x, new_y))
}

/// Pencil 与 Select 工具共用的拖拽释放提交：单锚点移动（MoveAnchor）+ 控制点拖拽（DragControlPoint）。
/// 返回本帧需要显示的 ghost（防止松手瞬间旧曲线闪现）；无提交时返回 None。
/// Bug 10：Select 工具也要能像铅笔一样直接编辑锚点/控制点，提交逻辑与 Pencil 完全一致。
#[allow(clippy::too_many_arguments)]
fn commit_anchor_or_ctrl_release(
    drag: Option<AutoDrag>,
    lane: Option<&AutomationLane>,
    lane_idx: Option<usize>,
    track_idx: u16,
    target: &AutomationTarget,
    mouse_info: Option<(egui::Pos2, u32, f32)>,
    ppu: f32,
    scroll_x: f32,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    max_val: f32,
    edits: &mut Vec<yinhe_types::AutomationEdit>,
    track_color: [f32; 3],
) -> Option<AutomationGhost> {
    match drag {
        Some(AutoDrag::MoveAnchor {
            old_tick,
            start_tick,
            start_value,
        }) => {
            if let Some((_, new_tick, new_value)) = mouse_info {
                // 只有实际移动过才提交 Move（避免单击时锚点偏移到鼠标位置）
                if new_tick != start_tick || new_value != start_value {
                    if let Some(lidx) = lane_idx {
                        edits.push(yinhe_types::AutomationEdit::Move {
                            track_idx,
                            lane_idx: lidx,
                            target: target.clone(),
                            old_tick,
                            new_tick,
                            new_value,
                        });
                    }
                    // 构造 ghost 用于本帧渲染（防止松手瞬间旧线段闪现）
                    if let Some(l) = lane {
                        let override_lane = build_lane_override(l, old_tick, new_tick, new_value);
                        return Some(AutomationGhost::Move {
                            lane: override_lane,
                            color: track_color,
                        });
                    }
                }
            }
        }
        Some(AutoDrag::DragControlPoint {
            prev_tick,
            which,
            start_x,
            start_y,
        }) => {
            // 提交控制点拖拽：从鼠标位置反推新 (x, y)，并按端别合并到 shape
            if let Some(l) = lane
                && let Some((p, _, _)) = mouse_info
                && let Some(lidx) = lane_idx
                && let Some(new_ctrl) = compute_ctrl_from_mouse(
                    l, prev_tick, which, p, ppu, scroll_x, grid_area, panel_rect, panel, max_val,
                )
                && (new_ctrl.0 != start_x || new_ctrl.1 != start_y)
            {
                // 读取当前 shape，按端别更新对应分量
                let new_shape = merge_ctrl_shape(l, prev_tick, which, new_ctrl);
                edits.push(yinhe_types::AutomationEdit::SetShape {
                    track_idx,
                    lane_idx: lidx,
                    target: target.clone(),
                    tick: prev_tick,
                    shape: new_shape,
                });
                let override_lane = build_lane_shape_override(l, prev_tick, new_shape);
                return Some(AutomationGhost::Move {
                    lane: override_lane,
                    color: track_color,
                });
            }
        }
        _ => {}
    }
    None
}

/// 把拖拽出的控制点 (x, y) 按端别合并进 `prev_tick` 事件的 shape。
/// 释放提交与 ghost 预览共用，保证两者生成的 shape 完全一致。
fn merge_ctrl_shape(
    lane: &AutomationLane,
    prev_tick: u32,
    which: CtrlEnd,
    new_ctrl: (f32, f32),
) -> SegmentShape {
    lane.events
        .iter()
        .find(|e| e.tick == prev_tick)
        .map(|e| match e.shape {
            SegmentShape::Curve { x1, y1, x2, y2 } => match which {
                CtrlEnd::Out => SegmentShape::Curve {
                    x1: new_ctrl.0,
                    y1: new_ctrl.1,
                    x2,
                    y2,
                },
                CtrlEnd::In => SegmentShape::Curve {
                    x1,
                    y1,
                    x2: new_ctrl.0,
                    y2: new_ctrl.1,
                },
            },
            SegmentShape::Step => SegmentShape::Step,
        })
        .unwrap_or(SegmentShape::Step)
}

/// 处理 automation 面板上的鼠标交互。
///
/// **Ghost 模式**：拖拽中不写模型，只返回 ghost 几何（由 wgpu Layer 3 绘制），
/// 释放时才提交编辑。
///
/// `tempo_lane`：`conductor.tempo`。当 `selected_target == Tempo` 时用作编辑目标；
/// 非 Tempo target 可传 None。Tempo target 且 None 时不产生任何编辑（防御）。
///
/// `id_base`：egui 跨帧状态（拖拽/hover 计时/右键锚点）的 id 前缀。
/// PR 传面板 id（ui.id().with(panel_index)），AR 传 per-lane id。
///
/// 返回值：
/// - `edits`：提交到 Document 的 AutomationEdit 列表。
/// - `ghost`：拖拽预览（wgpu Layer 1）。
/// - `drag_info` / `hover_info`：tooltip 数据。
/// - `marquee_rect`：Select 工具框选矩形（用于 egui painter 绘制 + 渲染层高亮预览）。
/// - `sel_op`：选区变更操作（caller 应用到 `panel.anchor_sel_rects`）。
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn handle_automation_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    automation_lanes: &[AutomationLane],
    tempo_lane: Option<&AutomationLane>,
    track_idx: u16,
    ctx: &AutomationEditCtx<'_>,
    id_base: egui::Id,
    track_colors: &[[f32; 4]],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
) -> (
    Vec<yinhe_types::AutomationEdit>,
    Option<AutomationGhost>,
    Option<HoverTooltip>,
    Option<HoverTooltip>,
    Option<egui::Rect>,
    Option<SelOp>,
) {
    let mut edits = Vec::new();
    // target 直接来自 selected_target（Tempo 也是 selected_target 的一种）。
    let target = panel.selected_target.clone();
    // Tempo 没有 tempo_lane 时无法编辑（防御：正常调用方必传）。
    if target == AutomationTarget::Tempo && tempo_lane.is_none() {
        return (edits, None, None, None, None, None);
    }
    // max_val 与 show_panels 共用同一计算（Tempo 由实际事件动态计算）。
    let max_val = match tempo_lane {
        Some(tl) => panel_max_val(panel, tl),
        None => panel.selected_target.max_value(),
    };
    if max_val == 0.0 {
        return (edits, None, None, None, None, None);
    }
    // value 的绝对上限：Tempo 允许拖拽突破当前显示上限（当前最大事件值），
    // 直达 `max_value()` 的 60_000_000 BPM；其他 target 上限就是 max_val（不变）。
    let value_cap = if target == yinhe_types::AutomationTarget::Tempo {
        yinhe_types::AutomationTarget::Tempo.max_value()
    } else {
        max_val
    };

    let ppu = panel.base.pixels_per_tick;
    let scroll_x = panel.base.scroll_x;
    let drag_id = ui.id().with("auto_drag").with(id_base);
    // MoveAnchors 拖拽偏移量写入此 id，供 automation_panel.rs 偏移持续化选框
    let move_offset_id = ui.id().with("auto_move_offset").with(id_base);
    // ghost 用 track color 而非黄色（ghost 自身有固定透明度）
    let track_color4 = track_colors
        .get(track_idx as usize)
        .copied()
        .unwrap_or([0.8, 0.8, 0.8, 1.0]);
    let track_color = [track_color4[0], track_color4[1], track_color4[2]];

    // 读取当前拖拽状态
    let drag_state = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));

    // 不用 ui.interact——piano_view 的 handle_input 已用 Sense::click_and_drag()
    // 占用了整个 music_rect，自动化面板的 grid_area 是子区域，事件已被父级消费。
    // 改用 ui.input() 直接检测指针状态。
    let pointer_hover_pos = ui.input(|i| i.pointer.hover_pos());
    let pointer_pressed = ui.input(|i| i.pointer.primary_pressed());
    let pointer_released = ui.input(|i| i.pointer.primary_released());
    let pointer_clicked = ui.input(|i| i.pointer.primary_clicked());
    let pointer_double_clicked = ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    });
    let pointer_secondary_clicked =
        ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));

    // 鼠标位置 → tick/value。tick clamp 到 >= 0 防止 as u32 溢出。
    let pos = pointer_hover_pos;
    let mouse_info = pos.map(|p| {
        let x_in_grid = p.x - grid_area.min.x;
        let raw_tick = ((x_in_grid + scroll_x) / ppu).max(0.0);
        let snapped_tick = crate::view_interaction::snap_tick(
            raw_tick as f64,
            ctx.quantize,
            ctx.ppq,
            ctx.bar_line_data,
        )
        .max(0.0) as u32;
        // y 不 clamp：允许鼠标拖到面板上方（y < 0），value 线性外推
        // 突破当前显示上限——Tempo 锚点由此可拖出 120 以上的 BPM。
        // value 的下限 0 由下方 clamp 兜底。
        let y_in_panel = p.y - panel_rect.min.y;
        let value = panel.y_to_value(y_in_panel, max_val).clamp(0.0, value_cap);
        (p, snapped_tick, value)
    });

    // 鼠标是否在 grid 区域内
    let in_grid = pos.is_some_and(|p| grid_area.contains(p));

    // 找当前 lane：Tempo 模式直接用 tempo_lane；其他模式从 automation_lanes 查。
    let (lane_idx, lane): (Option<usize>, Option<&AutomationLane>) =
        if target == yinhe_types::AutomationTarget::Tempo {
            // 上方已防御 None，这里 tempo_lane 必为 Some
            (Some(0), tempo_lane)
        } else {
            let idx = automation_lanes.iter().position(|l| l.target == target);
            (idx, idx.and_then(|i| automation_lanes.get(i)))
        };

    // 命中检测：找距离鼠标最近的锚点
    let hit_anchor = lane.and_then(|l| {
        let (_, snapped_tick, _) = mouse_info?;
        l.events
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| (e.tick as i64 - snapped_tick as i64).unsigned_abs())
            .and_then(|(i, e)| {
                let (p, _, _) = mouse_info?;
                let ex = grid_area.min.x + (e.tick as f32) * ppu - scroll_x;
                let ey = panel_rect.min.y + panel.value_to_y(e.value, max_val);
                let dist = ((ex - p.x).powi(2) + (ey - p.y).powi(2)).sqrt();
                if dist <= ANCHOR_HIT_PX {
                    Some((i, e.tick))
                } else {
                    None
                }
            })
    });

    // 命中检测：Curve 段中间的空心圆控制点（Pencil / Select 工具下，未拖拽时）。
    // Bug 10：Select 也能编辑控制点，命中门控放开到 Select 系。
    let hit_ctrl = if matches!(
        ctx.active_tool,
        Tool::Pencil | Tool::Select | Tool::SelectVertical
    ) && drag_state.is_none()
        && hit_anchor.is_none()
        && in_grid
    {
        lane.and_then(|l| {
            let (p, _, _) = mouse_info?;
            hit_control_point_on_lane(l, p, ppu, scroll_x, grid_area, panel_rect, panel, max_val)
        })
    } else {
        None
    };

    // 拖拽中：根据拖拽类型设置光标
    if let Some(drag) = drag_state {
        match drag {
            AutoDrag::MoveAnchors { .. } => {
                // 多锚点拖拽：显示移动光标（上下左右箭头）
                ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
            }
            AutoDrag::MarqueeSelect { .. } | AutoDrag::EraserMarquee { .. } => {
                // 框选：保持默认光标
            }
            _ => {
                // 单锚点拖拽/控制点拖拽：捏合抓手
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        }
    } else if (hit_anchor.is_some() || hit_ctrl.is_some()) && in_grid {
        // 悬停在锚点或控制点上时，鼠标变抓手
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    } else if matches!(ctx.active_tool, Tool::Select | Tool::SelectVertical)
        && hit_anchor.is_none()
        && in_grid
        && !panel.anchor_sel_rects.is_empty()
    {
        // Select 工具下，悬停在持续化选框内（未命中锚点）：显示移动光标
        let in_sel_rect = panel.anchor_sel_rects.iter().any(|r| {
            let Some((_, tick, value)) = mouse_info else {
                return false;
            };
            r.contains(tick, value)
        });
        if in_sel_rect {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
    }

    // Select 工具的框选矩形（拖拽中每帧更新，用于 egui painter 绘制 + 渲染层高亮预览）。
    let mut marquee_rect: Option<egui::Rect> = None;
    // Select 工具的选区变更操作（caller 应用到 `panel.anchor_sel_rects`）。
    let mut sel_op: Option<SelOp> = None;

    match ctx.active_tool {
        Tool::Pencil => {
            // 双击：删除锚点（在锚点上）或新建锚点（空白处）
            if pointer_double_clicked && in_grid {
                if let Some((_, tick)) = hit_anchor {
                    if let Some(lidx) = lane_idx {
                        edits.push(yinhe_types::AutomationEdit::Delete {
                            track_idx,
                            lane_idx: lidx,
                            target: target.clone(),
                            tick,
                        });
                    }
                    // 清除可能残留的 drag_state（双击时 pointer_pressed 也会触发）
                    ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                } else if hit_ctrl.is_none()
                    && let Some((_, tick, value)) = mouse_info
                {
                    // 双击空白处：新建锚点（控制点上双击不新建）
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None, None, None, None, None);
            }

            // 拖拽锚点：press 记录，release 提交
            // release 不检查 in_grid——用户可能拖到边缘（值=127/0）时鼠标移出 grid，
            // 但 mouse_info 仍有效（y_in_panel 已 clamp），不应丢失这次编辑。
            if pointer_pressed && in_grid {
                if let Some((event_idx, tick)) = hit_anchor {
                    // 左键点击锚点 → 选中它（信息面板显示该锚点）
                    if let Some(lidx) = lane_idx {
                        *info_content = Some(InfoContent::Anchor {
                            track_idx,
                            lane_idx: lidx,
                            event_idx,
                            target: target.clone(),
                        });
                        *right_tab = Some(RightTab::Info);
                    }
                    // 记录锚点原始位置，用于判断是否实际拖动过
                    let anchor_value = lane
                        .and_then(|l| l.events.iter().find(|e| e.tick == tick))
                        .map(|e| e.value)
                        .unwrap_or(0.0);
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            drag_id,
                            AutoDrag::MoveAnchor {
                                old_tick: tick,
                                start_tick: tick,
                                start_value: anchor_value,
                            },
                        );
                    });
                } else if let Some(hit) = hit_ctrl {
                    // 命中控制点：开始拖拽该端控制点
                    let (start_x, start_y) = match hit.which {
                        CtrlEnd::Out => (hit.x1, hit.y1),
                        CtrlEnd::In => (hit.x2, hit.y2),
                    };
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            drag_id,
                            AutoDrag::DragControlPoint {
                                prev_tick: hit.prev_tick,
                                which: hit.which,
                                start_x,
                                start_y,
                            },
                        );
                    });
                } else if drag_state.is_none() {
                    // 不在锚点/控制点上：检查是否在线段上，是则添加锚点并开始拖拽
                    if let Some(l) = lane
                        && let Some((_, tick, value)) = mouse_info
                        && hit_line_on_lane(
                            l,
                            tick,
                            value,
                            ppu,
                            scroll_x,
                            grid_area.min.x,
                            panel_rect.min.y,
                            panel,
                            max_val,
                        )
                    {
                        edits.push(yinhe_types::AutomationEdit::Add {
                            track_idx,
                            target: target.clone(),
                            tick,
                            value,
                            shape: SegmentShape::Step,
                        });
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(
                                drag_id,
                                AutoDrag::MoveAnchor {
                                    old_tick: tick,
                                    start_tick: tick,
                                    start_value: value,
                                },
                            );
                        });
                    }
                }
            }
            if pointer_released {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                let ghost = commit_anchor_or_ctrl_release(
                    drag,
                    lane,
                    lane_idx,
                    track_idx,
                    &target,
                    mouse_info,
                    ppu,
                    scroll_x,
                    grid_area,
                    panel_rect,
                    panel,
                    max_val,
                    &mut edits,
                    track_color,
                );
                return (edits, ghost, None, None, None, None);
            }

            // 点击空白（非拖拽，非控制点）：添加新锚点（shape = Step）
            if pointer_clicked
                && in_grid
                && hit_anchor.is_none()
                && hit_ctrl.is_none()
                && drag_state.is_none()
            {
                if let Some((_, tick, value)) = mouse_info {
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None, None, None, None, None);
            }
        }
        Tool::Curve => {
            // 拖拽起点 → 终点：press 记录起点，release 提交 2 个锚点
            // release 不检查 in_grid（同 Pencil 理由）。
            if pointer_pressed
                && in_grid
                && let Some((_, tick, value)) = mouse_info
            {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        drag_id,
                        AutoDrag::CurveDraw {
                            start_tick: tick,
                            start_value: value,
                        },
                    );
                });
            }
            if pointer_released {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                if let Some(AutoDrag::CurveDraw {
                    start_tick: t1,
                    start_value: v1,
                }) = drag
                    && let Some((_, t2, v2)) = mouse_info
                {
                    if t1 != t2 {
                        // 两个锚点：起点 Curve 直线，终点 Step
                        edits.push(yinhe_types::AutomationEdit::Add {
                            track_idx,
                            target: target.clone(),
                            tick: t1.min(t2),
                            value: v1,
                            shape: SegmentShape::linear_curve(),
                        });
                        edits.push(yinhe_types::AutomationEdit::Add {
                            track_idx,
                            target: target.clone(),
                            tick: t1.max(t2),
                            value: v2,
                            shape: SegmentShape::Step,
                        });
                    } else {
                        // 单击：只加一个 Curve 直线锚点
                        edits.push(yinhe_types::AutomationEdit::Add {
                            track_idx,
                            target: target.clone(),
                            tick: t2,
                            value: v2,
                            shape: SegmentShape::linear_curve(),
                        });
                    }
                }
                return (edits, None, None, None, None, None);
            }
        }
        Tool::Select | Tool::SelectVertical => {
            let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
            let alt = ui.input(|i| i.modifiers.alt);
            let shift = ui.input(|i| i.modifiers.shift);
            let vertical = matches!(ctx.active_tool, Tool::SelectVertical);

            // 检测鼠标是否在持续化选框内（音乐坐标判断）
            let in_sel_rect = panel.anchor_sel_rects.iter().any(|r| {
                let Some((_, tick, value)) = mouse_info else {
                    return false;
                };
                r.contains(tick, value)
            });

            // Bug 10：Select 具备铅笔的锚点编辑能力——双击锚点删除、双击空白新建。
            if pointer_double_clicked && in_grid {
                if let Some((_, tick)) = hit_anchor {
                    if let Some(lidx) = lane_idx {
                        edits.push(yinhe_types::AutomationEdit::Delete {
                            track_idx,
                            lane_idx: lidx,
                            target: target.clone(),
                            tick,
                        });
                    }
                    // 清除可能残留的 drag_state（双击时 pointer_pressed 也会触发）
                    ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                } else if hit_ctrl.is_none()
                    && let Some((_, tick, value)) = mouse_info
                {
                    // 双击空白处：新建锚点（控制点上双击不新建）
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None, None, None, None, None);
            }

            // ── 按下：点击锚点 / 点击选框内拖拽 / 开始框选 ──
            if pointer_pressed && in_grid {
                if let Some((event_idx, tick)) = hit_anchor {
                    // 读取锚点实际 value（用于 sel_rect 单点设置）
                    let anchor_value = lane
                        .and_then(|l| l.events.get(event_idx))
                        .map(|e| e.value)
                        .unwrap_or(0.0);
                    let anchor_in_sel = panel
                        .anchor_sel_rects
                        .iter()
                        .any(|r| r.contains(tick, anchor_value));
                    if cmd || shift {
                        // Shift/Cmd+点击锚点：保持单选框语义，union 到最新选框（或新点 rect）
                        // 用 Set 替换所有为 unioned rect（与原行为一致）
                        let point_rect = AnchorSelRect {
                            tick_start: tick as f64,
                            tick_end: tick as f64,
                            value_range: if vertical {
                                None
                            } else {
                                Some((anchor_value, anchor_value))
                            },
                        };
                        let final_rect = match panel.anchor_sel_rects.last().copied() {
                            Some(existing) => union_anchor_sel_rect(existing, point_rect),
                            None => point_rect,
                        };
                        sel_op = Some(SelOp::Set(SelRectOp::Set(final_rect)));
                    } else {
                        // 普通点击：若锚点不在 sel_rect 内，设置 sel_rect 为单点
                        if !anchor_in_sel {
                            let point_rect = AnchorSelRect {
                                tick_start: tick as f64,
                                tick_end: tick as f64,
                                value_range: if vertical {
                                    None
                                } else {
                                    Some((anchor_value, anchor_value))
                                },
                            };
                            sel_op = Some(SelOp::Set(SelRectOp::Set(point_rect)));
                        }
                        // 开始 MoveAnchors 拖拽（用鼠标位置作为 start，用于 delta 计算）
                        if let Some((_, _, value)) = mouse_info {
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(
                                    drag_id,
                                    AutoDrag::MoveAnchors {
                                        start_tick: tick,
                                        start_value: value,
                                        alt,
                                    },
                                );
                            });
                        }
                    }
                } else if in_sel_rect && !panel.anchor_sel_rects.is_empty() {
                    // 点击持续化选框内（未命中锚点）→ 拖拽选中的锚点
                    if let Some((_, tick, value)) = mouse_info {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(
                                drag_id,
                                AutoDrag::MoveAnchors {
                                    start_tick: tick,
                                    start_value: value,
                                    alt,
                                },
                            );
                        });
                    }
                } else if let Some(hit) = hit_ctrl {
                    // Bug 10：控制点上按下 → 拖拽该端控制点（与铅笔一致）
                    let (start_x, start_y) = match hit.which {
                        CtrlEnd::Out => (hit.x1, hit.y1),
                        CtrlEnd::In => (hit.x2, hit.y2),
                    };
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            drag_id,
                            AutoDrag::DragControlPoint {
                                prev_tick: hit.prev_tick,
                                which: hit.which,
                                start_x,
                                start_y,
                            },
                        );
                    });
                } else if !(cmd || shift)
                    && let Some(l) = lane
                    && let Some((_, tick, value)) = mouse_info
                    && hit_line_on_lane(
                        l,
                        tick,
                        value,
                        ppu,
                        scroll_x,
                        grid_area.min.x,
                        panel_rect.min.y,
                        panel,
                        max_val,
                    )
                {
                    // Bug 10：线段上按下 → 添加锚点并直接拖拽（与铅笔一致）
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            drag_id,
                            AutoDrag::MoveAnchor {
                                old_tick: tick,
                                start_tick: tick,
                                start_value: value,
                            },
                        );
                    });
                } else if let Some((p, _tick, _value)) = mouse_info {
                    // 不在选框内 → 开始框选（3px 阈值在拖拽中判断）
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, AutoDrag::MarqueeSelect { start_pos: p });
                    });
                    // 非加选模式：清空共享音符选区，触发三视图选框互斥
                    // （与 AR/PR 在 press 时清空 selected 的行为一致）。
                    if !cmd && !shift {
                        sel_op = Some(SelOp::ClearNoteSelection);
                    }
                }
            }

            // ── 拖拽中：更新 marquee_rect ──
            if let Some(AutoDrag::MarqueeSelect { start_pos, .. }) = drag_state
                && let Some(p) = pos
            {
                let dist = (p - start_pos).length();
                if dist >= MARQUEE_THRESHOLD {
                    // Bug 10：AM 选择工具的选框 = 垂直全选（y 范围扩展到整个 grid_area，
                    // x 范围按鼠标），Select 与 SelectVertical 行为一致。
                    let rect = egui::Rect::from_min_max(
                        egui::pos2(start_pos.x.min(p.x), grid_area.min.y),
                        egui::pos2(start_pos.x.max(p.x), grid_area.max.y),
                    )
                    .intersect(grid_area);
                    marquee_rect = Some(rect);
                    // 无修饰键时清空选区（让用户看到选区被清空）
                    if !cmd && !shift {
                        sel_op = Some(SelOp::Set(SelRectOp::Keep));
                    }
                }
            }

            // ── 释放：提交选区或拖拽 ──
            if pointer_released {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                // 清除 move_offset（拖拽结束）
                ui.ctx()
                    .data_mut(|d| d.remove::<(i64, f32)>(move_offset_id));
                // 释放时的 ghost（MoveAnchors 释放当帧仍需显示 ghost，避免闪烁）
                let mut release_ghost: Option<AutomationGhost> = None;
                match drag {
                    Some(AutoDrag::MarqueeSelect { start_pos, .. }) => {
                        let dist = pos.map(|p| (p - start_pos).length()).unwrap_or(0.0);
                        if dist >= MARQUEE_THRESHOLD {
                            // 框选完成：计算持续化选框（音乐坐标）。
                            // Bug 10：AM 选择工具的选框 = 垂直全选（value_range = None），
                            // Select 与 SelectVertical 行为一致，Select 框选不再按 y 取值。
                            if let Some((p, _, _)) = mouse_info {
                                let min_x = start_pos.x.min(p.x);
                                let max_x = start_pos.x.max(p.x);
                                let tick_from_x = |x: f32| -> f64 {
                                    ((x - grid_area.min.x + scroll_x) / ppu).max(0.0) as f64
                                };
                                let new_rect = AnchorSelRect {
                                    tick_start: tick_from_x(min_x),
                                    tick_end: tick_from_x(max_x),
                                    value_range: None,
                                };
                                // Shift/Cmd+框选：追加新选框（多选框）；否则替换所有
                                if cmd || shift {
                                    sel_op = Some(SelOp::Set(SelRectOp::Append(new_rect)));
                                } else {
                                    sel_op = Some(SelOp::Set(SelRectOp::Set(new_rect)));
                                }
                            }
                        } else {
                            // dist < 3px：视为点击，清空选框
                            if !cmd && !shift {
                                sel_op = Some(SelOp::Clear);
                            }
                        }
                    }
                    Some(AutoDrag::MoveAnchors {
                        start_tick,
                        start_value,
                        alt,
                        ..
                    }) => {
                        // 提交移动或复制
                        if let Some((_, cur_tick, cur_value)) = mouse_info
                            && let Some(l) = lane
                            && let Some(lidx) = lane_idx
                            && !panel.anchor_sel_rects.is_empty()
                        {
                            let d_tick = cur_tick as i64 - start_tick as i64;
                            // 垂直工具或垂直全选框（value_range=None）：只能水平移动，d_value 强制为 0
                            let d_value = if vertical
                                || panel
                                    .anchor_sel_rects
                                    .iter()
                                    .any(|r| r.value_range.is_none())
                            {
                                0.0
                            } else {
                                cur_value - start_value
                            };
                            if d_tick != 0 || d_value.abs() > 1e-6 {
                                // 收集 moves：从 lane.events 筛选落在任一 sel_rect 内的锚点
                                // moves = (original_tick, new_tick, new_value)
                                let moves: Vec<(u32, u32, f32)> = l
                                    .events
                                    .iter()
                                    .filter_map(|e| {
                                        if !panel
                                            .anchor_sel_rects
                                            .iter()
                                            .any(|r| r.contains(e.tick, e.value))
                                        {
                                            return None;
                                        }
                                        let new_tick = (e.tick as i64 + d_tick).max(0) as u32;
                                        let new_value = (e.value + d_value).clamp(0.0, value_cap);
                                        Some((e.tick, new_tick, new_value))
                                    })
                                    .collect();
                                if !moves.is_empty() {
                                    // 释放当帧仍显示 ghost（基于最终位置），
                                    // 避免 edits 在 layout.rs apply 前显示旧曲线一帧
                                    let ghost_lane = if alt {
                                        build_lane_multi_copy(l, &moves)
                                    } else {
                                        build_lane_multi_move(l, &moves)
                                    };
                                    release_ghost = Some(AutomationGhost::Move {
                                        lane: ghost_lane,
                                        color: track_color,
                                    });
                                    if alt {
                                        // Alt = 复制：为每个选中锚点生成 Add（shape 从原始事件读取）
                                        for &(tick, new_tick, new_value) in &moves {
                                            let shape = l
                                                .events
                                                .iter()
                                                .find(|e| e.tick == tick)
                                                .map(|e| e.shape)
                                                .unwrap_or(SegmentShape::Step);
                                            edits.push(yinhe_types::AutomationEdit::Add {
                                                track_idx,
                                                target: target.clone(),
                                                tick: new_tick,
                                                value: new_value,
                                                shape,
                                            });
                                        }
                                    } else {
                                        // 移动：用 MoveBatch 一次性提交所有锚点移动，
                                        // 避免逐个 Move 导致链式覆盖
                                        // （如 1→2, 2→3 时 1→2 会先删掉原 2）
                                        edits.push(yinhe_types::AutomationEdit::MoveBatch {
                                            track_idx,
                                            lane_idx: lidx,
                                            target: target.clone(),
                                            moves,
                                        });
                                    }

                                    // 所有选框一起偏移：tick += d_tick，value += d_value
                                    // 垂直工具 value_range 为 None 保持 None
                                    let new_rects: Vec<AnchorSelRect> = panel
                                        .anchor_sel_rects
                                        .iter()
                                        .map(|sel_rect| AnchorSelRect {
                                            tick_start: (sel_rect.tick_start + d_tick as f64)
                                                .max(0.0),
                                            tick_end: (sel_rect.tick_end + d_tick as f64).max(0.0),
                                            value_range: sel_rect.value_range.map(
                                                |(vmin, vmax)| {
                                                    (
                                                        (vmin + d_value).clamp(0.0, value_cap),
                                                        (vmax + d_value).clamp(0.0, value_cap),
                                                    )
                                                },
                                            ),
                                        })
                                        .collect();
                                    sel_op = Some(SelOp::Set(SelRectOp::ReplaceAll(new_rects)));
                                }
                            }
                            // delta == 0：视为点击，不提交编辑
                        }
                    }
                    // Bug 10：Select 工具直接拖拽单锚点/控制点（与铅笔一致），
                    // 走与 Pencil 相同的提交逻辑（MoveAnchor / DragControlPoint）。
                    drag @ (Some(AutoDrag::MoveAnchor { .. })
                    | Some(AutoDrag::DragControlPoint { .. })) => {
                        release_ghost = commit_anchor_or_ctrl_release(
                            drag,
                            lane,
                            lane_idx,
                            track_idx,
                            &target,
                            mouse_info,
                            ppu,
                            scroll_x,
                            grid_area,
                            panel_rect,
                            panel,
                            max_val,
                            &mut edits,
                            track_color,
                        );
                    }
                    _ => {}
                }
                return (edits, release_ghost, None, None, marquee_rect, sel_op);
            }
        }
        Tool::Eraser => {
            // 按下命中锚点：立即删除（橡皮擦语义）；空白按下：开始框选删除。
            if pointer_pressed && in_grid {
                if let Some((_, tick)) = hit_anchor {
                    if let Some(lidx) = lane_idx {
                        edits.push(yinhe_types::AutomationEdit::Delete {
                            track_idx,
                            lane_idx: lidx,
                            target: target.clone(),
                            tick,
                        });
                    }
                } else if let Some((p, _, _)) = mouse_info {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, AutoDrag::EraserMarquee { start_pos: p });
                    });
                }
            }

            // 拖拽中：更新 marquee_rect（红色删除框由 caller 绘制）。
            if let Some(AutoDrag::EraserMarquee { start_pos, .. }) = drag_state
                && let Some(p) = pos
                && (p - start_pos).length() >= MARQUEE_THRESHOLD
            {
                marquee_rect = Some(egui::Rect::from_two_pos(start_pos, p).intersect(grid_area));
            }

            // 释放：删除矩形内（tick + value 双范围）的所有锚点。
            if pointer_released {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                if let Some(AutoDrag::EraserMarquee { start_pos }) = drag
                    && let Some(l) = lane
                    && let Some(lidx) = lane_idx
                    && let Some(p) = pos
                    && (p - start_pos).length() >= MARQUEE_THRESHOLD
                {
                    let rect = egui::Rect::from_two_pos(start_pos, p);
                    let tick_from_x = |x: f32| -> f64 {
                        ((x - grid_area.min.x + scroll_x) / ppu).max(0.0) as f64
                    };
                    let t0 = tick_from_x(rect.min.x);
                    let t1 = tick_from_x(rect.max.x);
                    // rect 顶部 = 高 value，底部 = 低 value
                    let v_hi = panel.y_to_value(rect.min.y - panel_rect.min.y, max_val);
                    let v_lo = panel.y_to_value(rect.max.y - panel_rect.min.y, max_val);
                    for e in &l.events {
                        if (e.tick as f64) >= t0
                            && (e.tick as f64) <= t1
                            && e.value >= v_lo
                            && e.value <= v_hi
                        {
                            edits.push(yinhe_types::AutomationEdit::Delete {
                                track_idx,
                                lane_idx: lidx,
                                target: target.clone(),
                                tick: e.tick,
                            });
                        }
                    }
                }
                return (edits, None, None, None, marquee_rect, None);
            }
        }
        _ => {}
    }

    // 右键点击锚点 → 记录编辑信息，供 show_panels 弹窗
    let right_click_id = ui.id().with("auto_right_click").with(id_base);
    if pointer_secondary_clicked
        && in_grid
        && let Some((_, tick)) = hit_anchor
        && let Some(lidx) = lane_idx
        && let Some(l) = lane
        && let Some(_evt) = l.events.iter().find(|e| e.tick == tick)
    {
        // 清除旧编辑值，确保新锚点使用自己的初始值
        let edit_tick_id = ui.id().with("auto_right_tick").with(id_base);
        let edit_value_id = ui.id().with("auto_right_value").with(id_base);
        let was_open_id = ui.id().with("auto_right_was_open").with(id_base);
        ui.ctx().data_mut(|d| {
            d.remove::<f64>(edit_tick_id);
            d.remove::<f64>(edit_value_id);
            d.remove::<bool>(was_open_id);
            d.insert_temp(
                right_click_id,
                RightClickAnchor {
                    track_idx,
                    lane_idx: lidx,
                    old_tick: tick,
                    target: target.clone(),
                },
            );
        });
    }

    // ── Ghost 计算（panel 局部坐标，传给 wgpu Layer 3 绘制）──
    // 重新读取 drag_state：press 分支可能刚设置过，release 分支已 return。
    let drag_now = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
    let ghost = if let Some(drag) = drag_now
        && let Some((p, cur_tick, cur_value)) = mouse_info
    {
        // panel 局部坐标，与 build_data_lines 一致：x = x_offset + tick*ppu
        let x_offset = panel.base.left_panel_width - scroll_x;
        let cur_x = x_offset + cur_tick as f32 * ppu;
        let cur_y = panel.value_to_y(cur_value, max_val);
        match drag {
            AutoDrag::MoveAnchor {
                old_tick,
                start_tick: _,
                start_value: _,
            } => {
                // 用 build_lane_override 生成覆盖后的完整 lane，ghost 层整 lane 绘制。
                // 这样无论锚点如何跨越、插入、拖到末尾，都只需要正常画线逻辑。
                lane.map(|l| {
                    let override_lane = build_lane_override(l, old_tick, cur_tick, cur_value);
                    AutomationGhost::Move {
                        lane: override_lane,
                        color: track_color,
                    }
                })
            }
            AutoDrag::CurveDraw {
                start_tick,
                start_value,
            } => {
                let start_x = x_offset + start_tick as f32 * ppu;
                let start_y = panel.value_to_y(start_value, max_val);
                Some(AutomationGhost::Curve {
                    start_x,
                    start_y,
                    cur_x,
                    cur_y,
                    color: track_color,
                })
            }
            AutoDrag::DragControlPoint {
                prev_tick, which, ..
            } => {
                // 用原始鼠标位置（不 snap）反推该端控制点的 (x, y)，
                // 合并到现有 shape 后生成覆盖 lane。
                lane.and_then(|l| {
                    let new_ctrl = compute_ctrl_from_mouse(
                        l, prev_tick, which, p, ppu, scroll_x, grid_area, panel_rect, panel,
                        max_val,
                    )?;
                    // 读现有 shape，按端别替换对应分量
                    let new_shape = merge_ctrl_shape(l, prev_tick, which, new_ctrl);
                    let override_lane = build_lane_shape_override(l, prev_tick, new_shape);
                    Some(AutomationGhost::Move {
                        lane: override_lane,
                        color: track_color,
                    })
                })
            }
            AutoDrag::MoveAnchors {
                start_tick,
                start_value,
                alt,
                ..
            } => {
                // Select 工具拖拽多个选中锚点：构建 multi-move 或 multi-copy ghost lane。
                // 选中锚点的原始 (tick, value) 从 lane.events 读取（拖拽中模型不变）。
                let d_tick = cur_tick as i64 - start_tick as i64;
                // 垂直工具或垂直全选框（value_range=None）：只能水平移动，d_value 强制为 0
                let vertical_now = matches!(ctx.active_tool, Tool::SelectVertical)
                    || panel
                        .anchor_sel_rects
                        .iter()
                        .any(|r| r.value_range.is_none());
                let d_value = if vertical_now {
                    0.0
                } else {
                    cur_value - start_value
                };
                // 写入 move_offset，供 automation_panel.rs 偏移持续化选框
                ui.ctx()
                    .data_mut(|d| d.insert_temp(move_offset_id, (d_tick, d_value)));
                // 未实际移动时不产生 ghost
                if d_tick == 0 && d_value.abs() <= 1e-6 {
                    None
                } else {
                    lane.and_then(|l| {
                        if panel.anchor_sel_rects.is_empty() {
                            return None;
                        }
                        // 从 lane.events 筛选落在任一 sel_rect 内的锚点
                        let moves: Vec<(u32, u32, f32)> = l
                            .events
                            .iter()
                            .filter_map(|e| {
                                if !panel
                                    .anchor_sel_rects
                                    .iter()
                                    .any(|r| r.contains(e.tick, e.value))
                                {
                                    return None;
                                }
                                let new_tick = (e.tick as i64 + d_tick).max(0) as u32;
                                let new_value = (e.value + d_value).clamp(0.0, value_cap);
                                Some((e.tick, new_tick, new_value))
                            })
                            .collect();
                        if moves.is_empty() {
                            return None;
                        }
                        let override_lane = if alt {
                            // Alt = 复制：原事件保留 + 副本
                            build_lane_multi_copy(l, &moves)
                        } else {
                            // 移动：原事件移到新位置
                            build_lane_multi_move(l, &moves)
                        };
                        Some(AutomationGhost::Move {
                            lane: override_lane,
                            color: track_color,
                        })
                    })
                }
            }
            AutoDrag::MarqueeSelect { .. } | AutoDrag::EraserMarquee { .. } => {
                // 框选不产生 ghost（marquee_rect 由 egui painter 绘制）
                None
            }
        }
    } else {
        None
    };

    // 拖拽中返回拖拽信息用于 tooltip
    let drag_info: Option<HoverTooltip> = if ghost.is_some() {
        match drag_now {
            Some(AutoDrag::DragControlPoint {
                prev_tick, which, ..
            }) => {
                // 拖控制点：从鼠标位置反推 (x, y)，与现有 shape 合并后显示完整 4 值
                lane.and_then(|l| {
                    let (p, _, _) = mouse_info?;
                    let new_ctrl = compute_ctrl_from_mouse(
                        l, prev_tick, which, p, ppu, scroll_x, grid_area, panel_rect, panel,
                        max_val,
                    )?;
                    let (x1, y1, x2, y2) =
                        l.events
                            .iter()
                            .find(|e| e.tick == prev_tick)
                            .map(|e| match e.shape {
                                SegmentShape::Curve { x1, y1, x2, y2 } => (x1, y1, x2, y2),
                                SegmentShape::Step => (0.0, 0.0, 0.0, 0.0),
                            })?;
                    let (x1, y1, x2, y2) = match which {
                        CtrlEnd::Out => (new_ctrl.0, new_ctrl.1, x2, y2),
                        CtrlEnd::In => (x1, y1, new_ctrl.0, new_ctrl.1),
                    };
                    Some(HoverTooltip::ControlPoint {
                        x1,
                        y1,
                        x2,
                        y2,
                        pos: p,
                    })
                })
            }
            _ => {
                // 拖锚点 / CurveDraw：显示 (tick, value)，位置跟随鼠标
                mouse_info.map(|(p, tick, value)| HoverTooltip::Anchor {
                    tick,
                    value,
                    pos: p,
                })
            }
        }
    } else {
        None
    };

    // ── Hover tooltip：悬停在锚点/控制点上 HOVER_DELAY 秒后显示 tooltip ──
    // 仅在非拖拽时触发（拖拽时 drag_info 已覆盖）。
    let hover_info: Option<HoverTooltip> = if drag_info.is_none() && in_grid {
        let hover_anchor_id = ui.id().with("auto_hover_anchor").with(id_base);
        let hover_ctrl_id = ui.id().with("auto_hover_ctrl").with(id_base);
        let now = ui.input(|i| i.time);
        if let Some((_, anchor_tick)) = hit_anchor {
            // 锚点 hover：清除控制点计时
            ui.ctx().data_mut(|d| d.remove::<(u32, f64)>(hover_ctrl_id));
            let prev: Option<(u32, f64)> =
                ui.ctx().data(|d| d.get_temp::<(u32, f64)>(hover_anchor_id));
            let entry = match prev {
                Some(e) if e.0 == anchor_tick => e,
                _ => {
                    let new_entry = (anchor_tick, now);
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(hover_anchor_id, new_entry));
                    new_entry
                }
            };
            if now - entry.1 >= HOVER_DELAY {
                // 从 tick + value 算锚点像素位置
                let anchor_value = lane
                    .and_then(|l| l.events.iter().find(|e| e.tick == anchor_tick))
                    .map(|e| e.value);
                if let Some(v) = anchor_value {
                    let ax = grid_area.min.x + anchor_tick as f32 * ppu - scroll_x;
                    let ay = panel_rect.min.y + panel.value_to_y(v, max_val);
                    Some(HoverTooltip::Anchor {
                        tick: anchor_tick,
                        value: v,
                        pos: egui::pos2(ax, ay),
                    })
                } else {
                    None
                }
            } else {
                ui.ctx().request_repaint();
                None
            }
        } else if let Some(hit) = hit_ctrl {
            // 控制点 hover：清除锚点计时
            ui.ctx()
                .data_mut(|d| d.remove::<(u32, f64)>(hover_anchor_id));
            let prev: Option<(u32, f64)> =
                ui.ctx().data(|d| d.get_temp::<(u32, f64)>(hover_ctrl_id));
            let entry = match prev {
                Some(e) if e.0 == hit.prev_tick => e,
                _ => {
                    let new_entry = (hit.prev_tick, now);
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(hover_ctrl_id, new_entry));
                    new_entry
                }
            };
            if now - entry.1 >= HOVER_DELAY {
                Some(HoverTooltip::ControlPoint {
                    x1: hit.x1,
                    y1: hit.y1,
                    x2: hit.x2,
                    y2: hit.y2,
                    pos: hit.pos,
                })
            } else {
                ui.ctx().request_repaint();
                None
            }
        } else {
            ui.ctx().data_mut(|d| {
                d.remove::<(u32, f64)>(hover_anchor_id);
                d.remove::<(u32, f64)>(hover_ctrl_id);
            });
            None
        }
    } else {
        None
    };

    (edits, ghost, drag_info, hover_info, marquee_rect, sel_op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::tools_panel::Tool;
    use yinhe_editor_core::quantize::QuantizePreset;
    use yinhe_types::{AutomationEdit, AutomationEvent, AutomationTarget, SegmentShape};

    /// 测试用面板：Tempo target、1px/tick、面板高 80px、无滚动、无缩放。
    fn tempo_panel() -> AutomationPanelView {
        AutomationPanelView {
            selected_target: AutomationTarget::Tempo,
            show_velocity: false,
            panel_height: 80.0,
            value_zoom: 1.0,
            value_scroll: 0.0,
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: 1.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 0.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
            ..Default::default()
        }
    }

    /// 构造带 tempo 事件的 lane。
    fn tempo_lane(events: Vec<(u32, f32)>) -> AutomationLane {
        AutomationLane {
            target: AutomationTarget::Tempo,
            track: 0,
            events: events
                .into_iter()
                .map(|(tick, value)| AutomationEvent {
                    tick,
                    value,
                    shape: SegmentShape::Step,
                })
                .collect(),
        }
    }

    fn edit_ctx() -> AutomationEditCtx<'static> {
        AutomationEditCtx {
            active_tool: Tool::Pencil,
            active_track: Some(0),
            // 1/16 音符网格：interval = 480*4/16 = 120 tick，与测试拖拽位置对齐。
            quantize: QuantizePreset::Fraction(1, 16),
            ppq: 480,
            bar_line_data: None,
        }
    }

    /// 面板矩形：800x80，grid 与 panel 同宽（combo_width=0）。
    fn panel_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 80.0))
    }

    /// 跑一帧 handle_automation_interaction，返回 edits。
    fn run_frame(
        ctx: &egui::Context,
        raw: egui::RawInput,
        panel: &AutomationPanelView,
        lane: &AutomationLane,
    ) -> Vec<AutomationEdit> {
        let mut edits = Vec::new();
        ctx.run_ui(raw, |ui| {
            let mut info: Option<InfoContent> = None;
            let mut right_tab: Option<RightTab> = None;
            edits = handle_automation_interaction(
                ui,
                panel_rect(),
                panel_rect(),
                panel,
                &[],
                Some(lane),
                0,
                &edit_ctx(),
                ui.id().with(0),
                &[[0.8, 0.8, 0.8, 1.0]],
                &mut info,
                &mut right_tab,
            )
            .0;
        })
        .textures_delta
        .clear();
        edits
    }

    fn press_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    /// 指定工具的 edit ctx（默认 edit_ctx() 是 Pencil）。
    fn edit_ctx_tool(tool: Tool) -> AutomationEditCtx<'static> {
        AutomationEditCtx {
            active_tool: tool,
            active_track: Some(0),
            quantize: QuantizePreset::Fraction(1, 16),
            ppq: 480,
            bar_line_data: None,
        }
    }

    /// 跑一帧并返回完整输出（edits + marquee_rect + sel_op），供 Select 工具测试断言。
    fn run_frame_full(
        ctx: &egui::Context,
        raw: egui::RawInput,
        panel: &AutomationPanelView,
        lane: &AutomationLane,
        tool: Tool,
    ) -> (Vec<AutomationEdit>, Option<egui::Rect>, Option<SelOp>) {
        let (mut edits, mut marquee, mut sel_op) = (Vec::new(), None, None);
        ctx.run_ui(raw, |ui| {
            let mut info: Option<InfoContent> = None;
            let mut right_tab: Option<RightTab> = None;
            let (e, _g, _di, _hi, m, so) = handle_automation_interaction(
                ui,
                panel_rect(),
                panel_rect(),
                panel,
                &[],
                Some(lane),
                0,
                &edit_ctx_tool(tool),
                ui.id().with(0),
                &[[0.8, 0.8, 0.8, 1.0]],
                &mut info,
                &mut right_tab,
            );
            edits = e;
            marquee = m;
            sel_op = so;
        })
        .textures_delta
        .clear();
        (edits, marquee, sel_op)
    }

    fn drag_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw
    }

    fn release_event(pos: egui::Pos2) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        raw
    }

    /// 回归测试：Tempo 锚点拖到面板上方可突破当前显示上限（如 120 BPM）。
    /// 曾因 `value.clamp(0.0, max_val)` 把值钳死在显示上限，无法插入更高 BPM。
    #[test]
    fn tempo_anchor_drag_above_panel_exceeds_display_max() {
        let ctx = egui::Context::default();
        let panel = tempo_panel();
        let lane = tempo_lane(vec![(0, 120.0)]);
        // 锚点 (tick=0, value=120) 位于面板顶部：y = value_to_y(120, 120) = 0。
        let anchor = egui::pos2(0.0, 0.0);
        // 拖到面板上方 20px、tick 120（1/16 音符量化点）。
        let above = egui::pos2(120.0, -20.0);

        let _ = run_frame(&ctx, press_event(anchor), &panel, &lane);
        let _ = run_frame(&ctx, drag_event(above), &panel, &lane);
        let edits = run_frame(&ctx, release_event(above), &panel, &lane);

        let move_edit = edits
            .iter()
            .find(|e| matches!(e, AutomationEdit::Move { .. }))
            .expect("拖拽应提交 Move");
        match move_edit {
            AutomationEdit::Move {
                old_tick,
                new_tick,
                new_value,
                ..
            } => {
                assert_eq!(*old_tick, 0);
                assert_eq!(*new_tick, 120);
                assert!(
                    *new_value > 120.0,
                    "BPM 应突破显示上限 120，实际 {new_value}"
                );
                assert_eq!(*new_value, 150.0);
            }
            _ => unreachable!(),
        }
    }

    /// Bug 10 回归：AM 选择工具的选框 = 垂直全选（value_range = None），
    /// 与 SelectVertical 一致，不再按鼠标 y 限制值范围。
    #[test]
    fn select_tool_marquee_is_vertical() {
        let ctx = egui::Context::default();
        let panel = tempo_panel();
        let lane = tempo_lane(vec![(0, 120.0)]);
        // 普通 Select 工具框选 (100,10) → (300,70)。
        let start = egui::pos2(100.0, 10.0);
        let end = egui::pos2(300.0, 70.0);
        let _ = run_frame_full(&ctx, press_event(start), &panel, &lane, Tool::Select);
        let (_, marquee, _) = run_frame_full(&ctx, drag_event(end), &panel, &lane, Tool::Select);
        let (_, _, sel_op) = run_frame_full(&ctx, release_event(end), &panel, &lane, Tool::Select);

        // 拖拽中的临时选框矩形应满高（垂直全选）
        let rect = marquee.expect("拖拽中应产生 marquee_rect");
        assert_eq!(rect.min.y, 0.0, "选框顶部 = grid 顶部");
        assert_eq!(rect.max.y, 80.0, "选框底部 = grid 底部（面板高 80）");
        assert_eq!(rect.min.x, 100.0);
        assert_eq!(rect.max.x, 300.0);

        // 释放后提交的持续化选框：value_range = None（垂直全选）
        match sel_op {
            Some(SelOp::Set(SelRectOp::Set(r))) => {
                assert_eq!(r.tick_start, 100.0, "tick 起点按鼠标 x");
                assert_eq!(r.tick_end, 300.0, "tick 终点按鼠标 x");
                assert!(r.value_range.is_none(), "普通 Select 框选应为垂直全选");
            }
            other => panic!("期望 Set(Set(vertical rect))，实际 {other:?}"),
        }
    }

    /// Bug 10 回归：选择工具双击锚点 = 删除（与铅笔一致）。
    #[test]
    fn select_tool_double_click_anchor_deletes() {
        let ctx = egui::Context::default();
        let panel = tempo_panel();
        // 锚点 tick 120、value 120（面板顶 y=0）。
        let lane = tempo_lane(vec![(120, 120.0)]);
        let pos = egui::pos2(120.0, 0.0);
        let _ = run_frame_full(&ctx, press_event(pos), &panel, &lane, Tool::Select);
        let _ = run_frame_full(&ctx, release_event(pos), &panel, &lane, Tool::Select);
        let _ = run_frame_full(&ctx, press_event(pos), &panel, &lane, Tool::Select);
        let (edits, _, _) = run_frame_full(&ctx, release_event(pos), &panel, &lane, Tool::Select);

        assert!(
            edits
                .iter()
                .any(|e| matches!(e, AutomationEdit::Delete { tick: 120, .. })),
            "选择工具双击锚点应删除，实际 {edits:?}"
        );
    }
}
