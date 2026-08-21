use eframe::egui;
use yinhe_types::{AutomationLane, AutomationPanelView, AutomationTarget};
use yinhe_wgpu::{AutomationGhost, build_lane_override, build_lane_shape_override};

use super::hit_test::{compute_ctrl_from_mouse, merge_ctrl_shape};
use super::hover::CtrlEnd;

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

/// Pencil 与 Select 工具共用的拖拽释放提交：单锚点移动（MoveAnchor）+ 控制点拖拽（DragControlPoint）。
/// 返回本帧需要显示的 ghost（防止松手瞬间旧曲线闪现）；无提交时返回 None。
/// Bug 10：Select 工具也要能像铅笔一样直接编辑锚点/控制点，提交逻辑与 Pencil 完全一致。
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_anchor_or_ctrl_release(
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
