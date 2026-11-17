//! AR（编排视图）展开自动化 lane 的交互与渲染数据收集。
//!
//! 复用 PR 自动化面板的 handle_automation_interaction 与 wgpu 的
//! prepare_arr_automation：每条可见的 AM 行（展开音轨的子 lane +
//! Conductor 主行的 Tempo 直显）独立跑一遍交互，结果汇总给 view_ui。
//!
//! 坐标约定：
//! - 交互层用 lane 局部 y（lview.y_offset = 0，panel_rect = lane 屏幕矩形）；
//! - 渲染层用 AR 纹理坐标（y_offset = 子行顶部 y），ghost lane 按 y_offset 重建。

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui;

use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{ArRow, ArRowLayout, AutomationLane, AutomationPanelView, AutomationTarget};
use yinhe_types::{ArrangementView, AutomationEdit};
use yinhe_wgpu::AutomationGhost;

use crate::piano_view::automation_panel::AutomationEditCtx;
use crate::piano_view::automation_panel::interaction::{self, SelOp};
use crate::right_panel::{InfoContent, RightTab};

/// 一条需要交互/渲染的 AM 行。
#[derive(Clone, Copy, Debug)]
pub(crate) struct AmRowRef {
    /// 所属音轨索引（Bus lane 时是 Conductor 索引，仅用于视图状态）。
    pub track: usize,
    /// 该轨 automation_lanes 的下标；None = Conductor 主行（Tempo 直显）。
    pub sub: Option<usize>,
    /// Some = 这条 lane 属于 MixerNode 的 automation。
    pub node: Option<yinhe_types::MixerNodeId>,
    /// 行顶部 y（音乐坐标，未减 scroll_y）。通道 AR 行高不均，必须显式携带。
    pub y_top: f32,
    /// 行高（px）。
    pub height: f32,
}

/// 收集可视范围内的 AM 行（展开 lane + Conductor 主行）。
/// 轨道 AR 行高均匀：y_top = row * lane_height、height = lane_height。
#[allow(clippy::too_many_arguments)] // 行布局上下文透传
pub(crate) fn visible_am_rows(
    layout: &ArRowLayout,
    first_row: usize,
    last_row: usize,
    conductor: Option<u16>,
    tracks: &[Arc<yinhe_core::TrackData>],
    bus_ids: &[yinhe_types::BusId],
    bus_automation: &[Vec<AutomationLane>],
    track_mixer_lanes: &[Vec<(yinhe_types::MixerNodeId, Vec<AutomationLane>)>],
    lane_height: f32,
) -> Vec<AmRowRef> {
    let mut out = Vec::new();
    for row in first_row..last_row.min(layout.total_rows()) {
        let y_top = row as f32 * lane_height;
        match layout.row_hit(row) {
            Some(ArRow::Automation(track, sub)) => {
                let track_lane_count = tracks
                    .get(track)
                    .map(|t| t.automation_lanes.len())
                    .unwrap_or(0);
                if sub < track_lane_count {
                    out.push(AmRowRef {
                        track,
                        sub: Some(sub),
                        node: None,
                        y_top,
                        height: lane_height,
                    });
                } else if let Some((node, lane_idx)) = track_mixer_lane_at(
                    track_mixer_lanes
                        .get(track)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    track_lane_count,
                    sub,
                ) {
                    out.push(AmRowRef {
                        track,
                        sub: Some(lane_idx),
                        node: Some(node),
                        y_top,
                        height: lane_height,
                    });
                }
            }
            Some(ArRow::Track(track)) if conductor == Some(track as u16) => out.push(AmRowRef {
                track,
                sub: None,
                node: None,
                y_top,
                height: lane_height,
            }),
            Some(ArRow::BusAutomation(bus, lane_idx)) => {
                let Some(master) = conductor.map(usize::from) else {
                    continue;
                };
                let Some(bus_id) = bus_ids.get(bus).copied() else {
                    continue;
                };
                let Some(_) = bus_automation.get(bus).and_then(|l| l.get(lane_idx)) else {
                    continue;
                };
                out.push(AmRowRef {
                    track: master,
                    sub: Some(lane_idx),
                    node: Some(yinhe_types::MixerNodeId::Bus(bus_id)),
                    y_top,
                    height: lane_height,
                });
            }
            _ => {}
        }
    }
    out
}

/// 每轨可显示的 MixerNode automation（per-track LC / 音频 LC）。
pub(crate) fn track_mixer_lanes(
    track: &yinhe_core::TrackData,
    mixer: &yinhe_mixer::MixerParams,
) -> Vec<(yinhe_types::MixerNodeId, Vec<AutomationLane>)> {
    let node_id = if let Some(lc) = track.lc {
        Some(yinhe_types::MixerNodeId::LogicalChannel(lc))
    } else if track.kind == yinhe_core::TrackKind::Audio {
        track.audio_channel.map(|ch| {
            yinhe_types::MixerNodeId::LogicalChannel(yinhe_types::LogicalChannelId(
                yinhe_types::MIDI_CHANNEL_COUNT as u32 + u32::from(ch),
            ))
        })
    } else {
        None
    };
    node_id
        .and_then(|id| mixer.node(id))
        .filter(|node| !node.automation.is_empty())
        .and_then(|node| {
            let id = match node.id {
                yinhe_types::ChannelId::Lc(id) => yinhe_types::MixerNodeId::LogicalChannel(id),
                yinhe_types::ChannelId::Bus(bus) => yinhe_types::MixerNodeId::Bus(bus),
                yinhe_types::ChannelId::Master => yinhe_types::MixerNodeId::Master,
                yinhe_types::ChannelId::Ap(_) => return None,
            };
            Some(vec![(id, node.automation.clone())])
        })
        .unwrap_or_default()
}

/// `sub` = 该轨 automation 行索引；先轨道 lane，再 MixerNode lane。
pub(crate) fn track_mixer_lane_at(
    lanes: &[(yinhe_types::MixerNodeId, Vec<AutomationLane>)],
    track_lane_count: usize,
    sub: usize,
) -> Option<(yinhe_types::MixerNodeId, usize)> {
    let mut remain = sub.checked_sub(track_lane_count)?;
    for (node, lanes) in lanes {
        if remain < lanes.len() {
            return Some((*node, remain));
        }
        remain -= lanes.len();
    }
    None
}

/// 把 track 形态的编辑转换成 MixerNode owner 形态。
fn to_node_edit(node: yinhe_types::MixerNodeId, edit: AutomationEdit) -> AutomationEdit {
    match edit {
        AutomationEdit::Add {
            target,
            tick,
            value,
            shape,
            ..
        } => AutomationEdit::NodeAdd {
            node,
            target,
            tick,
            value,
            shape,
        },
        AutomationEdit::Move {
            lane_idx,
            target,
            old_tick,
            new_tick,
            new_value,
            ..
        } => AutomationEdit::NodeMove {
            node,
            lane_idx,
            target,
            old_tick,
            new_tick,
            new_value,
        },
        AutomationEdit::MoveBatch {
            lane_idx,
            target,
            moves,
            ..
        } => AutomationEdit::NodeMoveBatch {
            node,
            lane_idx,
            target,
            moves,
        },
        AutomationEdit::CycleShape {
            lane_idx,
            target,
            tick,
            ..
        } => AutomationEdit::NodeCycleShape {
            node,
            lane_idx,
            target,
            tick,
        },
        AutomationEdit::SetShape {
            lane_idx,
            target,
            tick,
            shape,
            ..
        } => AutomationEdit::NodeSetShape {
            node,
            lane_idx,
            target,
            tick,
            shape,
        },
        AutomationEdit::Delete {
            lane_idx,
            target,
            tick,
            ..
        } => AutomationEdit::NodeDelete {
            node,
            lane_idx,
            target,
            tick,
        },
        other => other,
    }
}

/// AM lane 的值域上限：Tempo 由实际事件动态计算（与 PR 的 panel_max_val 一致），
/// 其他 target 统一归一化 1.0。
pub(crate) fn lane_max_val(lane: &AutomationLane) -> f32 {
    if lane.target == AutomationTarget::Tempo {
        lane.events
            .iter()
            .map(|e| e.value)
            .fold(0.0_f32, f32::max)
            .max(1.0)
    } else {
        1.0
    }
}

/// 某 lane 在编辑/播放光标处的归一化值（0..1；Tempo 按动态上限归一化）。
/// `None` = 无光标或首个事件之前（面板自动化表显示空表）。
pub(crate) fn lane_cursor_value(lane: &AutomationLane, cursor_tick: Option<f64>) -> Option<f32> {
    let ct = cursor_tick?;
    let max_val = lane_max_val(lane);
    if max_val <= 0.0 {
        return None;
    }
    let (v, _) = lane.value_at(ct.max(0.0) as u32)?;
    Some((v / max_val).clamp(0.0, 1.0))
}

/// 高亮锚点 tick 集合（Select 框选的锚点 + Pencil 点选的 info_content 锚点）。
/// 与 PR automation_panel.rs 的 highlight 计算逻辑一致。
pub(crate) fn lane_highlight_ticks(
    lane: &AutomationLane,
    track: u16,
    sel_rects: &[yinhe_types::AnchorSelRect],
    info_content: &Option<InfoContent>,
) -> Box<[u32]> {
    let mut out: Vec<u32> = lane
        .events
        .iter()
        .filter(|e| sel_rects.iter().any(|r| r.contains(e.tick, e.value)))
        .map(|e| e.tick)
        .collect();
    if let Some(InfoContent::Anchor {
        target,
        track_idx,
        event_idx,
        ..
    }) = info_content
        && *target == lane.target
        && (lane.target == AutomationTarget::Tempo || *track_idx == track)
        && let Some(tick) = lane.events.get(*event_idx).map(|e| e.tick)
        && !out.contains(&tick)
    {
        out.push(tick);
    }
    out.into_boxed_slice()
}

/// 交互所需的外部状态（view_ui 每帧组装）。
pub(crate) struct AmLanesIo<'a> {
    pub tracks: &'a [Arc<yinhe_core::TrackData>],
    pub bus_ids: &'a [yinhe_types::BusId],
    pub bus_automation: &'a [Vec<AutomationLane>],
    /// 每轨 LC / 音频 LC 节点的 automation（与 Track 索引对齐）。
    pub track_mixer_lanes: &'a [Vec<(yinhe_types::MixerNodeId, Vec<AutomationLane>)>],
    pub tempo_lane: &'a AutomationLane,
    pub track_colors: &'a [[f32; 4]],
    pub selected: &'a mut yinhe_core::Selection,
    pub info_content: &'a mut Option<InfoContent>,
    pub right_tab: &'a mut Option<RightTab>,
    /// 每 lane 持久视图状态（锚点选框等），key = (音轨, target)。
    pub am_views: &'a mut HashMap<(u16, AutomationTarget), AutomationPanelView>,
    /// 收集到的编辑（由 arrange.rs 应用到 Document）。
    pub edits: &'a mut Vec<AutomationEdit>,
}

/// interact_all 的返回：拖拽 ghost（纹理坐标）+ Select/Eraser 框选矩形。
#[derive(Default)]
pub(crate) struct AmInteractionOut {
    /// (ghost, 所在行顶部纹理 y, 行高, max_val)。
    pub ghost: Option<(AutomationGhost, f32, f32, f32)>,
    pub marquee: Option<egui::Rect>,
    /// 空白单击的吸附 tick（未命中锚点），view_ui 用它移动播放光标。
    pub cursor_tick: Option<u32>,
}

/// 对所有可见 AM 行跑一遍编辑交互（Pencil/Select/SelectVertical/Eraser）。
///
/// 必须在 AR 的音符选框/橡皮擦处理之外按行区域互斥（view_ui 负责 gating）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn interact_all(
    ui: &mut egui::Ui,
    rows: &[AmRowRef],
    view: &ArrangementView,
    rect: egui::Rect,
    music_rect: egui::Rect,
    am_ctx: &AutomationEditCtx<'_>,
    io: &mut AmLanesIo<'_>,
) -> AmInteractionOut {
    let mut out = AmInteractionOut::default();
    let scroll_y = view.base.scroll_y;

    for r in rows {
        let (lanes, tempo_lane, target): (
            &[AutomationLane],
            Option<&AutomationLane>,
            AutomationTarget,
        ) = match (r.node, r.sub) {
            (Some(node), Some(sub)) => {
                let lanes = if let yinhe_types::MixerNodeId::Bus(bus) = node {
                    io.bus_ids
                        .iter()
                        .position(|id| *id == bus)
                        .and_then(|i| io.bus_automation.get(i))
                        .map(Vec::as_slice)
                } else {
                    io.track_mixer_lanes
                        .iter()
                        .flat_map(|lanes| lanes.iter())
                        .find(|(id, _)| *id == node)
                        .map(|(_, lanes)| lanes.as_slice())
                };
                let Some(lanes) = lanes else {
                    continue;
                };
                let Some(lane) = lanes.get(sub) else {
                    continue;
                };
                (lanes, None, lane.target.clone())
            }
            (None, Some(sub)) => {
                let Some(track) = io.tracks.get(r.track) else {
                    continue;
                };
                let Some(lane) = track.automation_lanes.get(sub) else {
                    continue;
                };
                (track.automation_lanes.as_slice(), None, lane.target.clone())
            }
            (None, None) => (
                &[] as &[AutomationLane],
                Some(io.tempo_lane),
                AutomationTarget::Tempo,
            ),
            (Some(_), None) => continue,
        };

        let y_top = r.y_top - scroll_y; // 纹理坐标
        let sy = rect.min.y + y_top; // 屏幕坐标
        let lane_rect = egui::Rect::from_min_max(
            egui::pos2(music_rect.min.x, sy),
            egui::pos2(music_rect.max.x, sy + r.height),
        );

        // 每 lane 持久视图状态：anchor_sel_rects 跨帧保留，其余字段每帧同步。
        let key = (r.track as u16, target.clone());
        let lview = io.am_views.entry(key.clone()).or_default();
        lview.base.scroll_x = view.base.scroll_x;
        lview.base.pixels_per_tick = view.base.pixels_per_tick;
        lview.base.left_panel_width = view.base.left_panel_width;
        lview.panel_height = r.height;
        lview.y_offset = 0.0; // 交互用 panel 局部坐标
        lview.value_zoom = 1.0;
        lview.value_scroll = 0.0;
        lview.selected_target = target.clone();
        lview.show_velocity = false;

        let id_base = ui.scope_id().with(("arr_am_lane", r.track, r.sub));
        // AR conductor 主行 Tempo 空白处不产生锚点框选（交给 sel_drag 选中/跳转）
        let suppress_blank = r.sub.is_none();
        let (edits, ghost, drag_info, hover_info, marquee, sel_op, blank_click_tick) =
            interaction::handle_automation_interaction(
                ui,
                lane_rect,
                lane_rect,
                lview,
                lanes,
                tempo_lane,
                r.track as u16,
                am_ctx,
                id_base,
                io.track_colors,
                io.info_content,
                io.right_tab,
                suppress_blank,
            );
        if let Some(node) = r.node {
            match io.info_content {
                Some(InfoContent::Anchor {
                    target: anchor_target,
                    node: anchor_node,
                    ..
                }) if *anchor_target == target => {
                    *anchor_node = Some(node);
                }
                Some(InfoContent::Anchor { .. }) => *io.info_content = None,
                _ => {}
            }
            io.edits
                .extend(edits.into_iter().map(|edit| to_node_edit(node, edit)));
        } else {
            io.edits.extend(edits);
        }
        if let Some(tick) = blank_click_tick {
            out.cursor_tick = Some(tick);
        }

        // 应用 Select 工具的选区变更（与 PR show_panels 一致）
        if let Some(op) = sel_op {
            let lview = io.am_views.entry(key).or_default();
            match op {
                SelOp::Set(rect_op) => {
                    let lane = interaction::panel_lane(lview, lanes, tempo_lane);
                    interaction::apply_sel_rect_op(lview, rect_op, lane);
                }
                SelOp::Clear => {
                    lview.clear_anchor_selection();
                    lview.dirty = true;
                }
                SelOp::ClearNoteSelection => {
                    // 三视图选框互斥：开始新框选时清空共享音符选区
                    io.selected.clear_rects();
                }
            }
        }

        if let Some(mr) = marquee {
            out.marquee = Some(mr);
        }

        if let Some(g) = ghost {
            // Move ghost 携带整条 lane，渲染时按 y_offset 重建，无需平移。
            let max_val = match (r.node, r.sub) {
                (Some(node), Some(sub)) => {
                    let lanes = if let yinhe_types::MixerNodeId::Bus(bus) = node {
                        io.bus_ids
                            .iter()
                            .position(|id| *id == bus)
                            .and_then(|i| io.bus_automation.get(i))
                            .map(Vec::as_slice)
                    } else {
                        io.track_mixer_lanes
                            .iter()
                            .flat_map(|lanes| lanes.iter())
                            .find(|(id, _)| *id == node)
                            .map(|(_, lanes)| lanes.as_slice())
                    };
                    lanes
                        .and_then(|lanes| lanes.get(sub))
                        .map(lane_max_val)
                        .unwrap_or(1.0)
                }
                (None, Some(sub)) => io
                    .tracks
                    .get(r.track)
                    .and_then(|t| t.automation_lanes.get(sub))
                    .map(lane_max_val)
                    .unwrap_or(1.0),
                _ => lane_max_val(io.tempo_lane),
            };
            out.ghost = Some((g, y_top, r.height, max_val));
        }

        // tooltip：拖拽中显示 drag_info，否则 hover 超时显示 hover_info（与 PR 一致）。
        if let Some(tip) = drag_info.or(hover_info) {
            let (lines, x, y): (Vec<String>, f32, f32) = match tip {
                interaction::HoverTooltip::Anchor { tick, value, pos } => {
                    let pos_str = if let Some((ppq, num, den, ts_events)) = am_ctx.bar_line_data {
                        format_tick_bar_beat_with_time_sig(tick as f64, ppq, ts_events, num, den)
                    } else {
                        format!("{}", tick)
                    };
                    let val_str = if target == AutomationTarget::Tempo {
                        format!("{:.2} BPM", value)
                    } else {
                        crate::piano_view::automation_panel::format_display_value(&target, value)
                    };
                    (vec![pos_str, val_str], pos.x, pos.y)
                }
                interaction::HoverTooltip::ControlPoint {
                    x1,
                    y1,
                    x2,
                    y2,
                    pos,
                } => (
                    vec![
                        format!("X1: {:.2}", x1),
                        format!("Y1: {:.2}", y1),
                        format!("X2: {:.2}", x2),
                        format!("Y2: {:.2}", y2),
                    ],
                    pos.x,
                    pos.y,
                ),
            };
            crate::view_interaction::draw_hover_tooltip(ui.ctx(), &lines, x, y);
        }
    }

    out
}

/// lane 的菜单/标签名（子行绘制与右键菜单共用）。统一走 `display_name`
/// （CC/RPN 带零填充编号 + 名称，如 "CC 010 Pan"、"RPN 000 PB Sensitivity"）。
pub(crate) fn lane_label(target: &AutomationTarget) -> String {
    target.display_name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_mixer::MixerParams;

    #[test]
    fn track_mixer_lane_offsets_after_track_lanes() {
        let lc = yinhe_types::LogicalChannelId(yinhe_types::TRACK_LC_BASE);
        let mut track = yinhe_core::TrackData::new(0, 0);
        track.lc = Some(lc);
        let mut mixer = MixerParams::default();
        assert!(mixer.add_track_lc_with_id(lc));
        mixer
            .track_node_mut(lc)
            .expect("track lc")
            .automation
            .push(yinhe_types::AutomationLane {
                target: yinhe_types::AutomationTarget::MixerGain {
                    channel: yinhe_types::MixerChannelId::TrackLc(lc),
                },
                track: 0,
                events: Vec::new(),
            });

        let lanes = track_mixer_lanes(&track, &mixer);
        assert_eq!(
            track_mixer_lane_at(&lanes, 0, 0),
            Some((yinhe_types::MixerNodeId::LogicalChannel(lc), 0))
        );
        assert_eq!(track_mixer_lane_at(&lanes, 1, 0), None);
    }
}
