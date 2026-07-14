//! Automation panel mouse interaction logic (pencil/curve tools, right-click).

use eframe::egui;

use yinhe_types::{AutomationLane, AutomationTarget, SegmentShape};
use yinhe_types::AutomationPanelView;
use yinhe_automation::{AutomationGhost, build_lane_override};

use crate::right_panel::{InfoContent, RightTab};
use crate::widgets::tools_panel::Tool;
use super::{AutomationEditCtx, ANCHOR_HIT_PX};

/// 拖拽状态（ghost）。存在 egui data 中，跨帧保持。
#[derive(Clone, Copy, Debug)]
pub(crate) enum AutoDrag {
    /// Pencil 拖拽锚点：`old_tick` 是原始位置，`start_tick/start_value` 是按下时的锚点原始值
    /// （用于判断是否实际移动过，避免单击时产生空 Move）
    MoveAnchor { old_tick: u32, start_tick: u32, start_value: u16 },
    /// Curve 拖拽：起点已固定
    CurveDraw { start_tick: u32, start_value: u16 },
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
    let interp_value = left.value as f32 + interp * (right.value as f32 - left.value as f32);

    // 转换为像素坐标并检查距离
    let interp_y = panel.value_to_y(interp_value, max_val);
    let mouse_y = panel.value_to_y(value, max_val);
    (interp_y - mouse_y).abs() <= 8.0
}

/// 处理 automation 面板上的鼠标交互。
///
/// **Ghost 模式**：拖拽中不写模型，只返回 ghost 几何（由 wgpu Layer 3 绘制），
/// 释放时才提交编辑。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_automation_interaction(
    ui: &mut egui::Ui,
    grid_area: egui::Rect,
    panel_rect: egui::Rect,
    panel: &AutomationPanelView,
    automation_lanes: &[AutomationLane],
    track_idx: u16,
    ctx: &AutomationEditCtx<'_>,
    panel_index: usize,
    track_colors: &[[f32; 3]],
    info_content: &mut Option<InfoContent>,
    right_tab: &mut Option<RightTab>,
) -> (Vec<yinhe_types::AutomationEdit>, Option<AutomationGhost>, Option<(u32, u16)>) {
    let mut edits = Vec::new();
    let target = &panel.selected_target;
    let max_val = target.max_value();
    if max_val == 0 {
        return (edits, None, None);
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
        let value = panel.y_to_value(y_in_panel, max_val as f32)
            .round()
            .clamp(0.0, max_val as f32) as u16;
        (p, snapped_tick, value)
    });

    // 鼠标是否在 grid 区域内
    let in_grid = pos.is_some_and(|p| grid_area.contains(p));

    // 找当前 lane
    let lane_idx = automation_lanes.iter().position(|l| l.target == *target);
    let lane = lane_idx.and_then(|i| automation_lanes.get(i));

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
                let ey = panel_rect.min.y + panel.value_to_y(e.value as f32, max_val as f32);
                let dist = ((ex - p.x).powi(2) + (ey - p.y).powi(2)).sqrt();
                if dist <= ANCHOR_HIT_PX {
                    Some((i, e.tick))
                } else {
                    None
                }
            })
    });

    // 拖拽中：鼠标变捏合抓手
    if drag_state.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if hit_anchor.is_some() && in_grid {
        // 悬停在锚点上时，鼠标变抓手
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
                            tick,
                        });
                    }
                    // 清除可能残留的 drag_state（双击时 pointer_pressed 也会触发）
                    ui.ctx().data_mut(|d| d.remove::<AutoDrag>(drag_id));
                } else if let Some((_, tick, value)) = mouse_info {
                    // 双击空白处：新建锚点
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None, None);
            }

            // 拖拽锚点：press 记录，release 提交
            // release 不检查 in_grid——用户可能拖到边缘（值=127/0）时鼠标移出 grid，
            // 但 mouse_info 仍有效（y_in_panel 已 clamp），不应丢失这次编辑。
            if pointer_pressed && in_grid {
                if let Some((_hit_idx, tick)) = hit_anchor {
                    // 左键点击锚点 → 选中它（信息面板显示该锚点）
                    if let Some(lidx) = lane_idx {
                        *info_content = Some(InfoContent::Anchor {
                            track_idx,
                            lane_idx: lidx,
                            tick,
                            target: target.clone(),
                        });
                        *right_tab = Some(RightTab::Info);
                    }
                    // 记录锚点原始位置，用于判断是否实际拖动过
                    let anchor_value = lane
                        .and_then(|l| l.events.iter().find(|e| e.tick == tick))
                        .map(|e| e.value)
                        .unwrap_or(0);
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, AutoDrag::MoveAnchor { old_tick: tick, start_tick: tick, start_value: anchor_value });
                    });
                } else if drag_state.is_none() {
                    // 不在锚点上：检查是否在线段上，是则添加锚点并开始拖拽
                    if let Some(l) = lane {
                        if let Some((_, tick, value)) = mouse_info {
                            if hit_line_on_lane(l, tick, value as f32, ppu, scroll_x, grid_area.min.x, panel_rect.min.y, panel, max_val as f32) {
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
                if let Some(AutoDrag::MoveAnchor { old_tick, start_tick, start_value }) = drag {
                    if let Some((_, new_tick, new_value)) = mouse_info {
                        // 只有实际移动过才提交 Move（避免单击时锚点偏移到鼠标位置）
                        if new_tick != start_tick || new_value != start_value {
                            if let Some(lidx) = lane_idx {
                                edits.push(yinhe_types::AutomationEdit::Move {
                                    track_idx,
                                    lane_idx: lidx,
                                    old_tick,
                                    new_tick,
                                    new_value,
                                });
                            }
                            // 构造 ghost 用于本帧渲染（防止松手瞬间旧线段闪现）
                            if let Some(l) = lane {
                                let override_lane = build_lane_override(l, old_tick, new_tick, new_value);
                                return (edits, Some(AutomationGhost::Move { lane: override_lane, color: track_color }), None);
                            }
                        }
                    }
                }
                return (edits, None, None);
            }

            // 点击空白（非拖拽）：添加新锚点（shape = Step）
            if pointer_clicked && in_grid && hit_anchor.is_none() && drag_state.is_none() {
                if let Some((_, tick, value)) = mouse_info {
                    edits.push(yinhe_types::AutomationEdit::Add {
                        track_idx,
                        target: target.clone(),
                        tick,
                        value,
                        shape: SegmentShape::Step,
                    });
                }
                return (edits, None, None);
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
                            // 两个锚点：起点 Curve{tension:0}，终点 Step
                            edits.push(yinhe_types::AutomationEdit::Add {
                                track_idx,
                                target: target.clone(),
                                tick: t1.min(t2),
                                value: v1,
                                shape: SegmentShape::Curve { tension: 0 },
                            });
                            edits.push(yinhe_types::AutomationEdit::Add {
                                track_idx,
                                target: target.clone(),
                                tick: t1.max(t2),
                                value: v2,
                                shape: SegmentShape::Step,
                            });
                        } else {
                            // 单击：只加一个 Curve{tension:0} 锚点
                            edits.push(yinhe_types::AutomationEdit::Add {
                                track_idx,
                                target: target.clone(),
                                tick: t2,
                                value: v2,
                                shape: SegmentShape::Curve { tension: 0 },
                            });
                        }
                    }
                }
                return (edits, None, None);
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
                    if let Some(evt) = l.events.iter().find(|e| e.tick == tick) {
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
        && let Some((_, cur_tick, cur_value)) = mouse_info
    {
        // panel 局部坐标，与 build_data_lines 一致：x = x_offset + tick*ppu
        let x_offset = panel.base.left_panel_width - scroll_x;
        let cur_x = x_offset + cur_tick as f32 * ppu;
        let cur_y = panel.value_to_y(cur_value as f32, max_val as f32);
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
                let start_y = panel.value_to_y(start_value as f32, max_val as f32);
                Some(AutomationGhost::Curve { start_x, start_y, cur_x, cur_y, color: track_color })
            }
        }
    } else {
        None
    };

    // 拖拽中返回拖拽信息用于 tooltip
    let drag_info = if ghost.is_some() {
        mouse_info.map(|(_, tick, value)| (tick, value))
    } else {
        None
    };

    (edits, ghost, drag_info)
}
