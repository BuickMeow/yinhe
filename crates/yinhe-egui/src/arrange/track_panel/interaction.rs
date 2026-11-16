use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use rust_i18n::t;
use yinhe_core::TrackInfo;
use yinhe_types::{ArRow, ArRowLayout, AutomationTarget, BusId};

use super::types::TrackAction;
use yinhe_editor_core::shortcuts;

/// 右键菜单命中的对象：普通音轨（含 AM 子行）或固定总线。
#[derive(Clone, Copy)]
enum RowMenuTarget {
    Track(usize, Option<usize>),
    Bus(BusId),
    BusAutomation(BusId, usize),
}

impl Default for RowMenuTarget {
    fn default() -> Self {
        Self::Bus(BusId(0))
    }
}

/// 右键普通轨：更新选择态与信息面板焦点。
#[allow(clippy::too_many_arguments)]
fn prepare_track_menu(
    idx: usize,
    sub: Option<usize>,
    track_info: &[TrackInfo],
    track_selected: &mut HashSet<u16>,
    selection_anchor: &mut Option<u16>,
    am_lane_selected: &mut HashSet<(u16, AutomationTarget)>,
    tracks: &[Arc<yinhe_core::TrackData>],
    info_content: &mut Option<crate::right_panel::InfoContent>,
) {
    let track_idx = track_info[idx].index;
    if let Some(s) = sub {
        am_lane_selected.clear();
        if let Some(lane) = tracks.get(idx).and_then(|tr| tr.automation_lanes.get(s)) {
            let key = (track_idx, lane.target.clone());
            if !am_lane_selected.contains(&key) {
                am_lane_selected.insert(key);
            }
        }
        track_selected.clear();
        *selection_anchor = None;
    } else if !track_selected.contains(&track_idx) {
        am_lane_selected.clear();
        track_selected.clear();
        track_selected.insert(track_idx);
        *selection_anchor = Some(track_idx);
    }
    *info_content = Some(crate::right_panel::InfoContent::Track);
}

/// 普通音轨右键菜单（原逻辑抽出）。
#[allow(clippy::too_many_arguments)]
fn track_context_menu(
    ui: &mut egui::Ui,
    idx: usize,
    sub: Option<usize>,
    track_info: &[TrackInfo],
    tracks: &[Arc<yinhe_core::TrackData>],
    track_selected: &mut HashSet<u16>,
    conductor_track_idx: Option<u16>,
    num_tracks: usize,
    actions: &mut Vec<TrackAction>,
) {
    let track_idx = track_info.get(idx).map(|t| t.index).unwrap_or(0);
    let is_conductor = conductor_track_idx == Some(track_idx);

    if !is_conductor
        && ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.rename_track").as_ref(),
                shortcuts::ACTION_TRACK_RENAME,
            ))
            .clicked()
    {
        if let Some(info) = track_info.get(idx) {
            crate::widgets::track_rename::begin(ui, idx, &info.name);
        }
        ui.close();
    }
    if ui
        .add(crate::widgets::menu::menu_item_action(
            ui,
            false,
            t!("arrange.track_properties").as_ref(),
            shortcuts::ACTION_TRACK_PROPERTIES,
        ))
        .clicked()
    {
        actions.push(TrackAction::ShowProperties { idx });
        ui.close();
    }
    if is_conductor
        && ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.open_device_chain").as_ref(),
                "",
            ))
            .clicked()
    {
        actions.push(TrackAction::OpenMasterDock);
        ui.close();
    }
    let track_kind = tracks.get(idx).map(|t| t.kind);
    let is_folder = track_kind == Some(yinhe_core::TrackKind::Folder);
    if is_folder
        && ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.ungroup_folder").as_ref(),
                "",
            ))
            .clicked()
    {
        actions.push(TrackAction::UngroupTrack { idx });
        ui.close();
    } else if !is_folder && !is_conductor && !track_selected.is_empty() {
        let mut indices: Vec<usize> = track_selected.iter().map(|&t| t as usize).collect();
        if !indices.contains(&idx) {
            indices.push(idx);
        }
        indices.sort_unstable();
        indices.dedup();
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.group_tracks").as_ref(),
                shortcuts::ACTION_TRACK_GROUP,
            ))
            .clicked()
        {
            actions.push(TrackAction::GroupTracks { indices });
            ui.close();
        }
    }
    ui.separator();

    if let Some(lane_idx) = sub {
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.delete_automation"),
                shortcuts::ACTION_DELETE_AUTOMATION,
            ))
            .clicked()
        {
            actions.push(TrackAction::DeleteAutomation { idx, lane_idx });
            ui.close();
        }
        return;
    }

    if !is_conductor && !is_folder {
        let folder_blocks_reorder = tracks
            .iter()
            .any(|t| t.kind == yinhe_core::TrackKind::Folder);
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.add_below"),
                shortcuts::ACTION_TRACK_ADD_BELOW,
            ))
            .clicked()
        {
            actions.push(TrackAction::AddTrack {
                after_idx: Some(idx),
            });
            ui.close();
        }
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.add_above"),
                shortcuts::ACTION_TRACK_ADD_ABOVE,
            ))
            .clicked()
        {
            actions.push(TrackAction::AddTrack {
                after_idx: Some(idx.saturating_sub(1)),
            });
            ui.close();
        }
        ui.separator();
        if !folder_blocks_reorder
            && idx > 0
            && conductor_track_idx != Some((idx - 1) as u16)
            && ui
                .add(crate::widgets::menu::menu_item_action(
                    ui,
                    false,
                    t!("arrange.move_up"),
                    shortcuts::ACTION_TRACK_MOVE_UP,
                ))
                .clicked()
        {
            actions.push(TrackAction::MoveUp { idx });
            ui.close();
        }
        if !folder_blocks_reorder
            && idx < num_tracks - 1
            && ui
                .add(crate::widgets::menu::menu_item_action(
                    ui,
                    false,
                    t!("arrange.move_down"),
                    shortcuts::ACTION_TRACK_MOVE_DOWN,
                ))
                .clicked()
        {
            actions.push(TrackAction::MoveDown { idx });
            ui.close();
        }
        ui.separator();
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.delete_track"),
                shortcuts::ACTION_TRACK_DELETE,
            ))
            .clicked()
        {
            actions.push(TrackAction::RemoveTrack { idx });
            ui.close();
        }
        ui.separator();
        // 「添加自动化」：自动化属于设备（XSynth / 插件），不属于音轨。
        if ui
            .add(crate::widgets::menu::menu_item_action(
                ui,
                false,
                t!("arrange.add_automation"),
                shortcuts::ACTION_ADD_AUTOMATION,
            ))
            .clicked()
        {
            actions.push(TrackAction::OpenAutomationPicker { idx });
            ui.close();
        }
    } else if ui
        .add(crate::widgets::menu::menu_item_action(
            ui,
            false,
            t!("arrange.add_below"),
            shortcuts::ACTION_TRACK_ADD_BELOW,
        ))
        .clicked()
    {
        actions.push(TrackAction::AddTrack {
            after_idx: Some(idx),
        });
        ui.close();
    }
}

/// 处理行点击、右键菜单、拖拽排序与键盘导航
/// 从 `track_panel::show` 尾部抽取，保持 `show` 高内聚为布局+渲染
#[allow(clippy::too_many_arguments)]
pub fn handle_interactions(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    panel_rect: egui::Rect,
    row_layout: &ArRowLayout,
    lh: f32,
    scroll_y: &mut f32,
    track_info: &[TrackInfo],
    track_visible: &[bool],
    track_selected: &mut HashSet<u16>,
    selection_anchor: &mut Option<u16>,
    conductor_track_idx: Option<u16>,
    num_tracks: usize,
    am_lane_selected: &mut HashSet<(u16, AutomationTarget)>,
    resp: &egui::Response,
    no_drag_rects: &[egui::Rect],
    item_rects: &[egui::Rect],
    drag: &mut Option<crate::widgets::reorder::DragReorder>,
    drag_id: egui::Id,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    request_pianoroll: &mut bool,
    tracks: &[Arc<yinhe_core::TrackData>],
    bus_ids: &[BusId],
    actions: &mut Vec<TrackAction>,
    max_scroll: f32,
) {
    let dragging = drag.is_some();
    if dragging {
        crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.drag_reorder"));
    }

    // 行命中 → 音轨（AM 子行归到所属音轨；双击/单击子行等效于主行）。
    let hit = |pos: egui::Pos2| -> Option<usize> {
        let rel_y = pos.y - panel_rect.min.y + *scroll_y;
        row_layout
            .hit_at_music_y(rel_y, lh)
            .and_then(|h| h.track_opt())
    };

    if resp.double_clicked() && !dragging {
        if let Some(pos) = resp.interact_pointer_pos()
            && let Some(row_hit) =
                row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh)
        {
            if let ArRow::Bus(bus) = row_hit
                && let Some(&bus_id) = bus_ids.get(bus)
            {
                actions.push(TrackAction::OpenBusDock { bus: bus_id });
            } else if let Some(idx) = row_hit.track_opt() {
                // 双击文件夹 = 折叠/展开，不打开 PR（文件夹没有音符）。
                if tracks
                    .get(idx)
                    .is_some_and(|t| t.kind == yinhe_core::TrackKind::Folder)
                {
                    actions.push(TrackAction::ToggleFolder { idx });
                } else {
                    // 双击：选中该行（track_selected = {该行}，即成为主音轨）并打开 PR。
                    let track_idx = track_info[idx].index;
                    track_selected.clear();
                    track_selected.insert(track_idx);
                    *selection_anchor = Some(track_idx);
                    *request_pianoroll = true;
                    am_lane_selected.clear();
                }
            }
        }
    } else if resp.clicked()
        && !dragging
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(row_hit) = row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh)
    {
        let shift = ui.input(|i| i.modifiers.shift);
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

        let lane_hit = match row_hit {
            ArRow::Automation(t, s) => Some((t, s)),
            ArRow::BusAutomation(_, s) => conductor_track_idx.map(|m| (m as usize, s)),
            _ => None,
        };
        if let Some((t, s)) = lane_hit {
            if let Some(lane) = tracks.get(t).and_then(|tr| tr.automation_lanes.get(s)) {
                let key = (track_info[t].index, lane.target.clone());
                if cmd {
                    if !am_lane_selected.remove(&key) {
                        am_lane_selected.insert(key);
                    }
                } else {
                    am_lane_selected.clear();
                    am_lane_selected.insert(key);
                }
            }
            track_selected.clear();
            track_selected.insert(track_info[t].index);
            *selection_anchor = None;
            *info_content = Some(crate::right_panel::InfoContent::Track);
        } else if let Some(idx) = row_hit.track_opt() {
            let track_idx = track_info[idx].index;
            am_lane_selected.clear();
            if shift {
                if let Some(anchor) = *selection_anchor {
                    let a = anchor as usize;
                    let b = track_idx as usize;
                    let lo = a.min(b);
                    let hi = a.max(b);
                    for i in lo..=hi {
                        track_selected.insert(i as u16);
                    }
                } else {
                    track_selected.clear();
                    track_selected.insert(track_idx);
                    *selection_anchor = Some(track_idx);
                }
            } else if cmd {
                if track_selected.contains(&track_idx) {
                    track_selected.remove(&track_idx);
                } else {
                    track_selected.insert(track_idx);
                }
                *selection_anchor = Some(track_idx);
            } else {
                if track_selected.len() == 1 && track_selected.contains(&track_idx) {
                    track_selected.clear();
                } else {
                    track_selected.clear();
                    track_selected.insert(track_idx);
                }
                *selection_anchor = Some(track_idx);
            }
            *info_content = Some(crate::right_panel::InfoContent::Track);
        }
    }

    // 右键：记录命中的行（普通轨 / 固定总线行）。
    let ctx_target_id = egui::Id::unique("track_ctx_menu_target");
    if resp.secondary_clicked() {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<RowMenuTarget>(ctx_target_id));
        if let Some(pos) = resp.interact_pointer_pos()
            && let Some(row_hit) =
                row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh)
        {
            match row_hit {
                ArRow::Bus(bus) => {
                    if let Some(&bus_id) = bus_ids.get(bus) {
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(ctx_target_id, RowMenuTarget::Bus(bus_id)));
                    }
                }
                ArRow::BusAutomation(bus, lane) => {
                    if let Some(&bus_id) = bus_ids.get(bus) {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(ctx_target_id, RowMenuTarget::BusAutomation(bus_id, lane))
                        });
                    }
                }
                ArRow::Track(idx) => {
                    prepare_track_menu(
                        idx,
                        None,
                        track_info,
                        track_selected,
                        selection_anchor,
                        am_lane_selected,
                        tracks,
                        info_content,
                    );
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(ctx_target_id, RowMenuTarget::Track(idx, None))
                    });
                }
                ArRow::Automation(idx, sub) => {
                    prepare_track_menu(
                        idx,
                        Some(sub),
                        track_info,
                        track_selected,
                        selection_anchor,
                        am_lane_selected,
                        tracks,
                        info_content,
                    );
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(ctx_target_id, RowMenuTarget::Track(idx, Some(sub)))
                    });
                }
            }
        }
    }

    if ui
        .ctx()
        .data(|d| d.get_temp::<RowMenuTarget>(ctx_target_id))
        .is_some()
    {
        resp.context_menu(|ui| {
            ui.set_min_width(160.0);
            ui.set_max_width(160.0);
            let target = ui
                .ctx()
                .data(|d| d.get_temp::<RowMenuTarget>(ctx_target_id));
            match target {
                Some(RowMenuTarget::Bus(bus)) => {
                    if ui
                        .add(crate::widgets::menu::menu_item_action(
                            ui,
                            false,
                            t!("arrange.open_device_chain").as_ref(),
                            "",
                        ))
                        .clicked()
                    {
                        actions.push(TrackAction::OpenBusDock { bus });
                        ui.close();
                    }
                    if ui
                        .add(crate::widgets::menu::menu_item_action(
                            ui,
                            false,
                            t!("arrange.add_bus_automation").as_ref(),
                            "",
                        ))
                        .clicked()
                    {
                        actions.push(TrackAction::AddBusAutomation { bus });
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add(crate::widgets::menu::menu_item_action(
                            ui,
                            false,
                            t!("arrange.remove_bus").as_ref(),
                            "",
                        ))
                        .clicked()
                    {
                        actions.push(TrackAction::RemoveBus { bus });
                        ui.close();
                    }
                }
                Some(RowMenuTarget::BusAutomation(bus, lane_idx)) => {
                    if ui
                        .add(crate::widgets::menu::menu_item_action(
                            ui,
                            false,
                            t!("arrange.delete_automation").as_ref(),
                            shortcuts::ACTION_DELETE_AUTOMATION,
                        ))
                        .clicked()
                    {
                        if let Some(master) = conductor_track_idx {
                            actions.push(TrackAction::DeleteAutomation {
                                idx: master as usize,
                                lane_idx,
                            });
                        }
                        ui.close();
                    }
                    let _ = bus;
                }
                Some(RowMenuTarget::Track(idx, sub)) => {
                    track_context_menu(
                        ui,
                        idx,
                        sub,
                        track_info,
                        tracks,
                        track_selected,
                        conductor_track_idx,
                        num_tracks,
                        actions,
                    );
                }
                None => {}
            }
        });
    }

    // ── 拖拽排序 ──
    // 起点落在 M/S、chevron、加号等按钮上时不排序，只有行内空白区可拖拽。
    if resp.drag_started()
        && !dragging
        && !tracks
            .iter()
            .any(|t| t.kind == yinhe_core::TrackKind::Folder)
        && let Some(pos) = resp.interact_pointer_pos()
        && !no_drag_rects.iter().any(|r| r.contains(pos))
        && matches!(
            row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh),
            Some(ArRow::Track(_))
        )
        && let Some(idx) = hit(pos)
        && Some(track_info[idx].index) != conductor_track_idx
    {
        let track_idx = track_info[idx].index;
        if !track_selected.contains(&track_idx) {
            track_selected.clear();
            track_selected.insert(track_idx);
            *selection_anchor = Some(track_idx);
        }
        let mut indices: Vec<usize> = track_selected.iter().map(|&t| t as usize).collect();
        indices.sort_unstable();
        indices.retain(|&i| track_info.get(i).map(|t| Some(t.index)) != Some(conductor_track_idx));
        if !indices.is_empty() {
            *drag = Some(crate::widgets::reorder::DragReorder {
                indices,
                insert_idx: idx,
            });
        }
    }

    if let Some(drag_state) = drag {
        if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
            drag_state.update_insert_idx(p.y, item_rects);
            drag_state.insert_idx = drag_state.insert_idx.max(1);

            const AUTO_SCROLL_MARGIN: f32 = 20.0;
            const AUTO_SCROLL_SPEED: f32 = 32.0;
            if p.y < panel_rect.top() + AUTO_SCROLL_MARGIN {
                *scroll_y = (*scroll_y - AUTO_SCROLL_SPEED).max(0.0);
            } else if p.y > panel_rect.bottom() - AUTO_SCROLL_MARGIN {
                *scroll_y = (*scroll_y + AUTO_SCROLL_SPEED).min(max_scroll);
            }
        }

        if let Some(y) = drag_state.insert_line_y(item_rects) {
            let x1 = panel_rect.min.x + 4.0;
            let x2 = panel_rect.max.x - 4.0;
            painter.line_segment(
                [egui::pos2(x1, y), egui::pos2(x2, y)],
                egui::Stroke::new(3.0, crate::theme::accent_active()),
            );
        }

        if ui.input(|i| i.pointer.any_released()) {
            actions.push(TrackAction::MoveTracks {
                indices: drag_state.indices.clone(),
                insert_at: drag_state.insert_idx,
            });
            *drag = None;
        }
    }

    // ── Up/Down arrow key navigation ──
    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
        if let Some(&current) = track_selected.iter().next() {
            let new_idx = current.saturating_sub(1);
            let mut found = None;
            for i in (0..=new_idx as usize).rev() {
                if track_visible.get(i).copied().unwrap_or(true) {
                    found = Some(i as u16);
                    break;
                }
            }
            if let Some(target) = found {
                track_selected.clear();
                track_selected.insert(target);
                *selection_anchor = Some(target);
            }
        } else if !track_info.is_empty() {
            let last = track_info.len() - 1;
            track_selected.clear();
            track_selected.insert(last as u16);
            *selection_anchor = Some(last as u16);
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
    }
    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        if let Some(&current) = track_selected.iter().next() {
            let new_idx = (current as usize + 1).min(num_tracks - 1);
            let mut found = None;
            for i in new_idx..num_tracks {
                if track_visible.get(i).copied().unwrap_or(true) {
                    found = Some(i as u16);
                    break;
                }
            }
            if let Some(target) = found {
                track_selected.clear();
                track_selected.insert(target);
                *selection_anchor = Some(target);
            }
        } else if !track_info.is_empty() {
            track_selected.clear();
            track_selected.insert(0);
            *selection_anchor = Some(0);
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
    }

    ui.data_mut(|d| d.insert_temp(drag_id, drag.clone()));
}
