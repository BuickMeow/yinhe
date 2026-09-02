use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use rust_i18n::t;
use yinhe_core::TrackInfo;
use yinhe_types::{ArRow, ArRowLayout, AutomationTarget};

use super::types::TrackAction;

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
    chevron_rects: &[egui::Rect],
    item_rects: &[egui::Rect],
    drag: &mut Option<crate::widgets::reorder::DragReorder>,
    drag_id: egui::Id,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    request_pianoroll: &mut bool,
    tracks: &[Arc<yinhe_core::TrackData>],
    actions: &mut Vec<TrackAction>,
    max_scroll: f32,
) {
    let dragging = drag.is_some();

    // 行命中 → 音轨（AM 子行归到所属音轨；双击/单击子行等效于主行）。
    let hit = |pos: egui::Pos2| -> Option<usize> {
        let rel_y = pos.y - panel_rect.min.y + *scroll_y;
        row_layout.hit_at_music_y(rel_y, lh).map(|h| h.track())
    };

    if resp.double_clicked() && !dragging {
        if let Some(pos) = resp.interact_pointer_pos()
            && let Some(idx) = hit(pos)
        {
            // 双击：选中该行（track_selected = {该行}，即成为主音轨）并打开 PR。
            let track_idx = track_info[idx].index;
            track_selected.clear();
            track_selected.insert(track_idx);
            *selection_anchor = Some(track_idx);
            *request_pianoroll = true;
            am_lane_selected.clear();
        }
    } else if resp.clicked()
        && !dragging
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(row_hit) = row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh)
    {
        let shift = ui.input(|i| i.modifiers.shift);
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

        if let ArRow::Automation(t, s) = row_hit {
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
        } else {
            let idx = row_hit.track();
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
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
    }

    // 右键：选中并记录菜单索引
    let ctx_menu_idx_id = egui::Id::new("track_ctx_menu_idx");
    if resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(row_hit) = row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh)
    {
        let (idx, sub) = match row_hit {
            ArRow::Track(t) => (t, None),
            ArRow::Automation(t, s) => (t, Some(s)),
        };
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
        ui.ctx()
            .data_mut(|d| d.insert_temp(ctx_menu_idx_id, (idx, sub)));
    }

    resp.context_menu(|ui| {
        ui.set_min_width(160.0);
        ui.set_max_width(160.0);
        let (idx, sub) = ui
            .ctx()
            .data(|d| d.get_temp::<(usize, Option<usize>)>(ctx_menu_idx_id))
            .unwrap_or((0, None));
        let track_idx = track_info.get(idx).map(|t| t.index).unwrap_or(0);
        let is_conductor = conductor_track_idx == Some(track_idx);

        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("arrange.track_properties").as_ref(),
            ))
            .clicked()
        {
            actions.push(TrackAction::ShowProperties { idx });
            ui.close();
        }
        ui.separator();

        if let Some(lane_idx) = sub {
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.delete_automation"),
                ))
                .clicked()
            {
                actions.push(TrackAction::DeleteAutomation { idx, lane_idx });
                ui.close();
            }
            return;
        }

        if !is_conductor {
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.add_below"),
                ))
                .clicked()
            {
                actions.push(TrackAction::AddTrack {
                    after_idx: Some(idx),
                });
                ui.close();
            }
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.add_above"),
                ))
                .clicked()
            {
                actions.push(TrackAction::AddTrack {
                    after_idx: Some(idx.saturating_sub(1)),
                });
                ui.close();
            }
            ui.separator();
            if idx > 0
                && conductor_track_idx != Some((idx - 1) as u16)
                && ui
                    .add(crate::widgets::menu::menu_item_button(
                        ui,
                        false,
                        t!("arrange.move_up"),
                    ))
                    .clicked()
            {
                actions.push(TrackAction::MoveUp { idx });
                ui.close();
            }
            if idx < num_tracks - 1
                && ui
                    .add(crate::widgets::menu::menu_item_button(
                        ui,
                        false,
                        t!("arrange.move_down"),
                    ))
                    .clicked()
            {
                actions.push(TrackAction::MoveDown { idx });
                ui.close();
            }
            ui.separator();
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.delete_track"),
                ))
                .clicked()
            {
                actions.push(TrackAction::RemoveTrack { idx });
                ui.close();
            }
            ui.separator();
            super::menu::create_automation_menu(ui, idx, tracks, actions);
        } else if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("arrange.add_below"),
            ))
            .clicked()
        {
            actions.push(TrackAction::AddTrack {
                after_idx: Some(idx),
            });
            ui.close();
        }
    });

    // ── 拖拽排序 ──
    if resp.drag_started()
        && !dragging
        && let Some(pos) = resp.interact_pointer_pos()
        && !chevron_rects.iter().any(|r| r.contains(pos))
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
