//! Automation panel mouse interaction logic (pencil/curve tools, right-click).

use eframe::egui;

use yinhe_types::{AutomationLane, AutomationTarget, SegmentShape};
use yinhe_types::AutomationPanelView;
use yinhe_wgpu::{AutomationGhost, build_lane_override, build_lane_shape_override};

use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;
use super::{AutomationEditCtx, ANCHOR_HIT_PX};

/// 悬停在锚点上多久后显示 tooltip（秒）。
const HOVER_DELAY: f64 = 0.6;

/// Hover/drag tooltip 数据。锚点和控制点用不同的显示内容。
#[derive(Clone, Copy, Debug)]
pub(crate) enum HoverTooltip {
    /// 锚点（或拖拽锚点）：显示 tick（小节:拍:tick）+ automation value
    Anchor { tick: u32, value: f32, pos: egui::Pos2 },
    /// 贝塞尔控制点（或拖拽控制点）：显示 ctrl_x / ctrl_y（归一化 [0,1]）
    ControlPoint { ctrl_x: f32, ctrl_y: f32, pos: egui::Pos2 },
}

/// 拖拽状态（ghost）。存在 egui data 中，跨帧保持。
#[derive(Clone, Copy, Debug)]
pub(crate) enum AutoDrag {
    /// Pencil 拖拽锚点：`old_tick` 是原始位置，`start_tick/start_value` 是按下时的锚点原始值
    /// （用于判断是否实际移动过，避免单击时产生空 Move）
    MoveAnchor { old_tick: u32, start_tick: u32, start_value: f32 },
    /// Curve 拖拽：起点已固定
    CurveDraw { start_tick: u32, start_value: f32 },
    /// 拖拽 Curve 段中间的空心圆控制点。
    /// `prev_tick`：被拖段的前驱事件 tick（段的起点，shape 存于此事件）。
    /// `start_ctrl_x/y`：按下时的控制点位置，用于判断是否实际移动过。
    DragControlPoint { prev_tick: u32, start_ctrl_x: f32, start_ctrl_y: f32 },
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

/// 检测鼠标是否悬停在 Curve 段中间的空心圆控制点上。
///
/// 遍历所有 Curve 段（非直线），计算控制点屏幕位置，
/// 返回最近控制点所属段的前驱事件 tick + 原始 ctrl_x/ctrl_y + 控制点像素位置。
pub(crate) fn hit_control_point_on_lane(
    lane: &AutomationLane,
    mouse: egui::Pos2,
    ppu: f32,
    scroll_x: f32,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    max_val: f32,
) -> Option<(u32, f32, f32, egui::Pos2)> {
    let x_offset = grid_area.min.x - scroll_x;
    let hit_sq = ANCHOR_HIT_PX * ANCHOR_HIT_PX;
    let mut best: Option<(u32, f32, f32, egui::Pos2, f32)> = None; // (prev_tick, ctrl_x, ctrl_y, pos, dist_sq)
    for i in 1..lane.events.len() {
        let prev = &lane.events[i - 1];
        let cur = &lane.events[i];
        let SegmentShape::Curve { ctrl_x, ctrl_y } = prev.shape else { continue; };
        if prev.shape.is_linear() { continue; }

        let x0 = x_offset + prev.tick as f32 * ppu;
        let y0 = panel_rect.min.y + panel.value_to_y(prev.value, max_val);
        let x1 = x_offset + cur.tick as f32 * ppu;
        let y1 = panel_rect.min.y + panel.value_to_y(cur.value, max_val);

        let cx = x0 + (x1 - x0) * ctrl_x;
        let cy = y0 + (y1 - y0) * ctrl_y;

        let dist_sq = (cx - mouse.x).powi(2) + (cy - mouse.y).powi(2);
        if dist_sq <= hit_sq
            && best.as_ref().map(|(_, _, _, _, d)| dist_sq < *d).unwrap_or(true)
        {
            best = Some((prev.tick, ctrl_x, ctrl_y, egui::pos2(cx, cy), dist_sq));
        }
    }
    best.map(|(t, cx, cy, p, _)| (t, cx, cy, p))
}

/// 从鼠标屏幕位置反推 Curve 段的 `(ctrl_x, ctrl_y)`（归一化 [0,1]）。
///
/// 段由 `prev_tick` 定位（前驱事件），下一个事件为段的终点。
/// 段水平或竖直时（dx/dy ≈ 0），对应分量为 0.5（中点，无意义）。
/// 返回 `None` 表示 `prev_tick` 不存在或没有下一个事件。
fn compute_ctrl_from_mouse(
    lane: &AutomationLane,
    prev_tick: u32,
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
    let x0 = x_offset + prev.tick as f32 * ppu;
    let y0 = panel_rect.min.y + panel.value_to_y(prev.value, max_val);
    let x1 = x_offset + next.tick as f32 * ppu;
    let y1 = panel_rect.min.y + panel.value_to_y(next.value, max_val);
    let dx = x1 - x0;
    let dy = y1 - y0;
    let new_ctrl_x = if dx.abs() < 1e-3 {
        0.5
    } else {
        ((mouse.x - x0) / dx).clamp(0.0, 1.0)
    };
    let new_ctrl_y = if dy.abs() < 1e-3 {
        0.5
    } else {
        ((mouse.y - y0) / dy).clamp(0.0, 1.0)
    };
    Some((new_ctrl_x, new_ctrl_y))
}

/// 处理 automation 面板上的鼠标交互。
///
/// **Ghost 模式**：拖拽中不写模型，只返回 ghost 几何（由 wgpu Layer 3 绘制），
/// 释放时才提交编辑。
///
/// `tempo_lane`：`conductor.tempo`。当 `selected_target == Tempo` 时用作编辑目标。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_automation_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    automation_lanes: &[AutomationLane],
    tempo_lane: &AutomationLane,
    track_idx: u16,
    ctx: &AutomationEditCtx<'_>,
    panel_index: usize,
    track_colors: &[[f32; 3]],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
) -> (Vec<yinhe_types::AutomationEdit>, Option<AutomationGhost>, Option<HoverTooltip>, Option<HoverTooltip>) {
    let mut edits = Vec::new();
    // target 直接来自 selected_target（Tempo 也是 selected_target 的一种）。
    let target = panel.selected_target.clone();
    // Tempo 的 max_val 由实际事件动态计算；其他用 target.max_value()。
    let max_val = if target == yinhe_types::AutomationTarget::Tempo {
        tempo_lane.events.iter().map(|e| e.value).fold(0.0_f32, f32::max).max(1.0)
    } else {
        target.max_value()
    };
    if max_val == 0.0 {
        return (edits, None, None, None);
    }

    let ppu = panel.base.pixels_per_tick;
    let scroll_x = panel.base.scroll_x;
    let drag_id = ui.id().with("auto_drag").with(panel_index);
    // ghost 用 track color 而非黄色
    let track_color = track_colors
        .get(track_idx as usize)
        .copied()
        .unwrap_or([0.8, 0.8, 0.8]);

    // 读取当前拖拽状态
    let drag_state = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));

    // 不用 ui.interact——piano_view 的 handle_input 已用 Sense::click_and_drag()
    // 占用了整个 music_rect，自动化面板的 grid_area 是子区域，事件已被父级消费。
    // 改用 ui.input() 直接检测指针状态。
    let pointer_hover_pos = ui.input(|i| i.pointer.hover_pos());
    let pointer_pressed = ui.input(|i| i.pointer.primary_pressed());
    let pointer_released = ui.input(|i| i.pointer.primary_released());
    let pointer_clicked = ui.input(|i| i.pointer.primary_clicked());
    let pointer_double_clicked = ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
    let pointer_secondary_clicked = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));

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
        ).max(0.0) as u32;
        let y_in_panel = (p.y - panel_rect.min.y).clamp(0.0, panel_rect.height());
        let value = panel.y_to_value(y_in_panel, max_val).clamp(0.0, max_val);
        (p, snapped_tick, value)
    });

    // 鼠标是否在 grid 区域内
    let in_grid = pos.is_some_and(|p| grid_area.contains(p));

    // 找当前 lane：Tempo 模式直接用 tempo_lane；其他模式从 automation_lanes 查。
    let (lane_idx, lane): (Option<usize>, Option<&AutomationLane>) = if target == yinhe_types::AutomationTarget::Tempo {
        (Some(0), Some(tempo_lane))
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

    // 命中检测：Curve 段中间的空心圆控制点（仅 Pencil 工具下，未拖拽时）
    let hit_ctrl = if ctx.active_tool == Tool::Pencil
        && drag_state.is_none()
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

    // 拖拽中：鼠标变捏合抓手
    if drag_state.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if (hit_anchor.is_some() || hit_ctrl.is_some()) && in_grid {
        // 悬停在锚点或控制点上时，鼠标变抓手
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

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
                } else if hit_ctrl.is_none() && let Some((_, tick, value)) = mouse_info {
                    // 双击空白处：新建锚点（控制点上双击不新建）
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None, None, None);
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
                        d.insert_temp(drag_id, AutoDrag::MoveAnchor { old_tick: tick, start_tick: tick, start_value: anchor_value });
                    });
                } else if let Some((prev_tick, ctrl_x, ctrl_y, _)) = hit_ctrl {
                    // 命中控制点：开始拖拽控制点
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, AutoDrag::DragControlPoint {
                            prev_tick,
                            start_ctrl_x: ctrl_x,
                            start_ctrl_y: ctrl_y,
                        });
                    });
                } else if drag_state.is_none() {
                    // 不在锚点/控制点上：检查是否在线段上，是则添加锚点并开始拖拽
                    if let Some(l) = lane {
                        if let Some((_, tick, value)) = mouse_info {
                            if hit_line_on_lane(l, tick, value, ppu, scroll_x, grid_area.min.x, panel_rect.min.y, panel, max_val) {
                                edits.push(yinhe_types::AutomationEdit::Add {
                                    track_idx,
                                    target: target.clone(),
                                    tick,
                                    value,
                                    shape: SegmentShape::Step,
                                });
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(drag_id, AutoDrag::MoveAnchor { old_tick: tick, start_tick: tick, start_value: value });
                                });
                            }
                        }
                    }
                }
            }
            if pointer_released {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                match drag {
                    Some(AutoDrag::MoveAnchor { old_tick, start_tick, start_value }) => {
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
                                    return (edits, Some(AutomationGhost::Move { lane: override_lane, color: track_color }), None, None);
                                }
                            }
                        }
                    }
                    Some(AutoDrag::DragControlPoint { prev_tick, start_ctrl_x, start_ctrl_y }) => {
                        // 提交控制点拖拽：从鼠标位置反推新 ctrl_x/ctrl_y
                        if let Some(l) = lane
                            && let Some((p, _, _)) = mouse_info
                            && let Some(lidx) = lane_idx
                        {
                            if let Some(new_ctrl) = compute_ctrl_from_mouse(
                                l, prev_tick, p, ppu, scroll_x, grid_area, panel_rect, panel, max_val,
                            ) {
                                if new_ctrl.0 != start_ctrl_x || new_ctrl.1 != start_ctrl_y {
                                    edits.push(yinhe_types::AutomationEdit::SetShape {
                                        track_idx,
                                        lane_idx: lidx,
                                        target: target.clone(),
                                        tick: prev_tick,
                                        shape: SegmentShape::Curve { ctrl_x: new_ctrl.0, ctrl_y: new_ctrl.1 },
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
                return (edits, None, None, None);
            }

            // 点击空白（非拖拽，非控制点）：添加新锚点（shape = Step）
            if pointer_clicked && in_grid && hit_anchor.is_none() && hit_ctrl.is_none() && drag_state.is_none() {
                if let Some((_, tick, value)) = mouse_info {
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None, None, None);
            }
        }
        Tool::Curve => {
            // 拖拽起点 → 终点：press 记录起点，release 提交 2 个锚点
            // release 不检查 in_grid（同 Pencil 理由）。
            if pointer_pressed && in_grid {
                if let Some((_, tick, value)) = mouse_info {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, AutoDrag::CurveDraw { start_tick: tick, start_value: value });
                    });
                }
            }
            if pointer_released {
                let drag = ui.ctx().data(|d| d.get_temp::<AutoDrag>(drag_id));
                ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                if let Some(AutoDrag::CurveDraw { start_tick: t1, start_value: v1 }) = drag {
                    if let Some((_, t2, v2)) = mouse_info {
                        if t1 != t2 {
                            // 两个锚点：起点 Curve 直线，终点 Step
                            edits.push(yinhe_types::AutomationEdit::Add {
                                track_idx,
                                target: target.clone(),
                                tick: t1.min(t2),
                                value: v1,
                                shape: SegmentShape::Curve {
                                    ctrl_x: SegmentShape::LINEAR_CTRL_X,
                                    ctrl_y: SegmentShape::LINEAR_CTRL_Y,
                                },
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
                                shape: SegmentShape::Curve {
                                    ctrl_x: SegmentShape::LINEAR_CTRL_X,
                                    ctrl_y: SegmentShape::LINEAR_CTRL_Y,
                                },
                            });
                        }
                    }
                }
                return (edits, None, None, None);
            }
        }
        _ => {}
    }

    // 右键点击锚点 → 记录编辑信息，供 show_panels 弹窗
    let right_click_id = ui.id().with("auto_right_click").with(panel_index);
    if pointer_secondary_clicked && in_grid {
        if let Some((_, tick)) = hit_anchor {
            if let Some(lidx) = lane_idx {
                if let Some(l) = lane {
                    if let Some(_evt) = l.events.iter().find(|e| e.tick == tick) {
                        // 清除旧编辑值，确保新锚点使用自己的初始值
                        let edit_tick_id = ui.id().with("auto_right_tick").with(panel_index);
                        let edit_value_id = ui.id().with("auto_right_value").with(panel_index);
                        let was_open_id = ui.id().with("auto_right_was_open").with(panel_index);
                        ui.ctx().data_mut(|d| {
                            d.remove::<f64>(edit_tick_id);
                            d.remove::<f64>(edit_value_id);
                            d.remove::<bool>(was_open_id);
                            d.insert_temp(right_click_id, RightClickAnchor {
                                track_idx,
                                lane_idx: lidx,
                                old_tick: tick,
                                target: target.clone(),
                            });
                        });
                    }
                }
            }
        }
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
            AutoDrag::MoveAnchor { old_tick, start_tick: _, start_value: _ } => {
                // 用 build_lane_override 生成覆盖后的完整 lane，ghost 层整 lane 绘制。
                // 这样无论锚点如何跨越、插入、拖到末尾，都只需要正常画线逻辑。
                lane.map(|l| {
                    let override_lane = build_lane_override(l, old_tick, cur_tick, cur_value);
                    AutomationGhost::Move { lane: override_lane, color: track_color }
                })
            }
            AutoDrag::CurveDraw { start_tick, start_value } => {
                let start_x = x_offset + start_tick as f32 * ppu;
                let start_y = panel.value_to_y(start_value, max_val);
                Some(AutomationGhost::Curve { start_x, start_y, cur_x, cur_y, color: track_color })
            }
            AutoDrag::DragControlPoint { prev_tick, .. } => {
                // 用原始鼠标位置（不 snap）反推 ctrl_x/ctrl_y，
                // 生成覆盖后的 lane（前驱事件 shape 已更新）。
                lane.and_then(|l| {
                    let new_ctrl = compute_ctrl_from_mouse(
                        l, prev_tick, p, ppu, scroll_x, grid_area, panel_rect, panel, max_val,
                    )?;
                    let new_shape = SegmentShape::Curve { ctrl_x: new_ctrl.0, ctrl_y: new_ctrl.1 };
                    let override_lane = build_lane_shape_override(l, prev_tick, new_shape);
                    Some(AutomationGhost::Move { lane: override_lane, color: track_color })
                })
            }
        }
    } else {
        None
    };

    // 拖拽中返回拖拽信息用于 tooltip
    let drag_info: Option<HoverTooltip> = if ghost.is_some() {
        match drag_now {
            Some(AutoDrag::DragControlPoint { prev_tick, .. }) => {
                // 拖控制点：从鼠标位置反推 ctrl_x/ctrl_y
                lane.and_then(|l| {
                    let (p, _, _) = mouse_info?;
                    let new_ctrl = compute_ctrl_from_mouse(
                        l, prev_tick, p, ppu, scroll_x, grid_area, panel_rect, panel, max_val,
                    )?;
                    Some(HoverTooltip::ControlPoint {
                        ctrl_x: new_ctrl.0, ctrl_y: new_ctrl.1, pos: p,
                    })
                })
            }
            _ => {
                // 拖锚点 / CurveDraw：显示 (tick, value)，位置跟随鼠标
                mouse_info.map(|(p, tick, value)| HoverTooltip::Anchor { tick, value, pos: p })
            }
        }
    } else {
        None
    };

    // ── Hover tooltip：悬停在锚点/控制点上 HOVER_DELAY 秒后显示 tooltip ──
    // 仅在非拖拽时触发（拖拽时 drag_info 已覆盖）。
    let hover_info: Option<HoverTooltip> = if drag_info.is_none() && in_grid {
        let hover_anchor_id = ui.id().with("auto_hover_anchor").with(panel_index);
        let hover_ctrl_id = ui.id().with("auto_hover_ctrl").with(panel_index);
        let now = ui.input(|i| i.time);
        if let Some((_, anchor_tick)) = hit_anchor {
            // 锚点 hover：清除控制点计时
            ui.ctx().data_mut(|d| d.remove::<(u32, f64)>(hover_ctrl_id));
            let prev: Option<(u32, f64)> = ui.ctx().data(|d| d.get_temp::<(u32, f64)>(hover_anchor_id));
            let entry = match prev {
                Some(e) if e.0 == anchor_tick => e,
                _ => {
                    let new_entry = (anchor_tick, now);
                    ui.ctx().data_mut(|d| d.insert_temp(hover_anchor_id, new_entry));
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
                    Some(HoverTooltip::Anchor { tick: anchor_tick, value: v, pos: egui::pos2(ax, ay) })
                } else {
                    None
                }
            } else {
                ui.ctx().request_repaint();
                None
            }
        } else if let Some((prev_tick, ctrl_x, ctrl_y, ctrl_pos)) = hit_ctrl {
            // 控制点 hover：清除锚点计时
            ui.ctx().data_mut(|d| d.remove::<(u32, f64)>(hover_anchor_id));
            let prev: Option<(u32, f64)> = ui.ctx().data(|d| d.get_temp::<(u32, f64)>(hover_ctrl_id));
            let entry = match prev {
                Some(e) if e.0 == prev_tick => e,
                _ => {
                    let new_entry = (prev_tick, now);
                    ui.ctx().data_mut(|d| d.insert_temp(hover_ctrl_id, new_entry));
                    new_entry
                }
            };
            if now - entry.1 >= HOVER_DELAY {
                Some(HoverTooltip::ControlPoint { ctrl_x, ctrl_y, pos: ctrl_pos })
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

    (edits, ghost, drag_info, hover_info)
}
