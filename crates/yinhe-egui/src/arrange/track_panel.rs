use std::collections::HashSet;

use eframe::egui;
use egui_material_icons::icons::ICON_EDIT;

use yinhe_core::TrackInfo;

use yinhe_editor_core::document::TrackOverride;

/// Actions requested by the track panel that need Document access.
#[derive(Clone, Debug)]
pub(crate) enum TrackAction {
    /// Add a new track after the given index (or at end if None)
    AddTrack { after_idx: Option<usize> },
    /// Remove the track at the given index
    RemoveTrack { idx: usize },
    /// Move a track up (swap with previous)
    MoveUp { idx: usize },
    /// Move a track down (swap with next)
    MoveDown { idx: usize },
}

/// Render the track list using a painter (unified component for both
/// pianoroll and transport contexts).
///
/// Returns `(audio_dirty, actions)` where `audio_dirty` is `true` if the user
/// toggled a Mute or Solo button this frame, and `actions` is a list of
/// track-management actions (add/remove/move) for the caller to apply.
#[must_use]
pub(crate) fn show(
    ui: &mut egui::Ui,
    track_info: &[TrackInfo],
    track_visible: &[bool],
    track_overrides: &mut [TrackOverride],
    track_selected: &mut HashSet<u16>,
    selection_anchor: &mut Option<u16>,
    conductor_track_idx: Option<u16>,
    track_colors: &[[f32; 3]],
    row_height: &mut f32,
    scroll_y: &mut f32,
    request_pianoroll: &mut bool,
    editing_track: &mut Option<u16>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
) -> (bool, Vec<TrackAction>) {
    let panel_rect = ui.max_rect();
    let panel_w = panel_rect.width();
    let panel_h = panel_rect.height();
    let num_tracks = track_info.len();

    if num_tracks == 0 || panel_w < 1.0 || panel_h < 1.0 {
        return (false, Vec::new());
    }

    let mut actions = Vec::new();

    let show_details = *row_height >= 30.0;

    // Clamp scroll_y
    let max_scroll = (num_tracks as f32 * *row_height - panel_h).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll);

    // Visible track range
    let first = (*scroll_y / *row_height).floor() as usize;
    let visible_count = (panel_h / *row_height).ceil() as usize + 2;
    let last = (first + visible_count).min(num_tracks);

    let painter = ui.painter().clone();
    let mut audio_dirty = false;

    let interact_id = egui::Id::new("track_panel_area");
    let resp = ui.interact(panel_rect, interact_id, egui::Sense::click_and_drag());

    let btn_size = egui::vec2(18.0, 18.0);

    for idx in first..last {
        if !track_visible.get(idx).copied().unwrap_or(true) {
            continue;
        }
        let ti = &track_info[idx];
        let y = panel_rect.min.y + idx as f32 * *row_height - *scroll_y;
        if y > panel_rect.max.y || y + *row_height < panel_rect.min.y {
            continue;
        }

        let row_rect = egui::Rect::from_min_size(
            egui::pos2(panel_rect.min.x, y),
            egui::vec2(panel_w, *row_height),
        );

        let is_conductor = Some(ti.index) == conductor_track_idx;
        let selected = track_selected.contains(&ti.index);
        if selected {
            painter.rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill);
        } else if row_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
            painter.rect_filled(row_rect, 0.0, egui::Color32::WHITE.gamma_multiply(0.03));
        }

        let color = track_colors.get(idx).copied().unwrap_or([0.5, 0.5, 0.5]);
        let color32 = egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );

        let badge_w = 8.0_f32;
        let badge_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(badge_w, *row_height));
        painter.rect_filled(badge_rect, 0.0, color32);

        let text_x = badge_rect.max.x + 6.0;
        let track_num_text = format!("{:03}", ti.index);

        if show_details {
            let font = egui::FontId::proportional((*row_height * 0.25).clamp(8.0, 13.0));

            painter.text(
                egui::pos2(text_x, badge_rect.min.y + *row_height * 0.30),
                egui::Align2::LEFT_CENTER,
                &track_num_text,
                font.clone(),
                egui::Color32::WHITE.gamma_multiply(0.85),
            );
            let badge_text = if is_conductor {
                "Master".to_string()
            } else {
                let port_letter = match ti.port {
                    0 => 'A',
                    1 => 'B',
                    2 => 'C',
                    3 => 'D',
                    4 => 'E',
                    5 => 'F',
                    6 => 'G',
                    7 => 'H',
                    _ => '?',
                };
                format!("{}{:02}", port_letter, ti.channel + 1)
            };
            painter.text(
                egui::pos2(text_x + 32.0, badge_rect.min.y + *row_height * 0.30),
                egui::Align2::LEFT_CENTER,
                &badge_text,
                font.clone(),
                egui::Color32::WHITE.gamma_multiply(0.85),
            );

            let name = &ti.name;
            let name_font = egui::FontId::proportional((*row_height * 0.25).clamp(9.0, 13.0));
            painter.text(
                egui::pos2(text_x, badge_rect.min.y + *row_height * 0.70),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                egui::Color32::WHITE.gamma_multiply(0.85),
            );

            if !is_conductor {
                let muted = track_overrides.get(idx).map(|o| o.muted).unwrap_or(false);
                let soloed = track_overrides.get(idx).map(|o| o.soloed).unwrap_or(false);

                let gap = 2.0;
                let total_btn_w = 2.0 * btn_size.x + gap;
                let btn_x_start = row_rect.max.x - total_btn_w - 6.0;
                let btn_y = badge_rect.center().y - btn_size.y * 0.5;

                let m_rect = egui::Rect::from_min_size(egui::pos2(btn_x_start, btn_y), btn_size);
                let s_rect = egui::Rect::from_min_size(
                    egui::pos2(btn_x_start + btn_size.x + gap, btn_y),
                    btn_size,
                );

                let m_resp = draw_inline_button(
                    ui,
                    &painter,
                    m_rect,
                    "M",
                    muted,
                    crate::theme::MUTE_ACTIVE,
                    egui::Id::new(("track_btn_m", idx)),
                );
                let s_resp = draw_inline_button(
                    ui,
                    &painter,
                    s_rect,
                    "S",
                    soloed,
                    crate::theme::SOLO_ACTIVE,
                    egui::Id::new(("track_btn_s", idx)),
                );

                if m_resp.clicked() {
                    if let Some(ov) = track_overrides.get_mut(idx) {
                        ov.muted = !ov.muted;
                        audio_dirty = true;
                    }
                }
                if s_resp.clicked() {
                    if let Some(ov) = track_overrides.get_mut(idx) {
                        ov.soloed = !ov.soloed;
                        audio_dirty = true;
                    }
                }
            }

            // 铅笔 ICON：双击 track 后显示，表示该 track 是 pencil/automation 的编辑目标。
            // 非 conductor：在 M/S 按钮左侧；conductor：在行右侧（无 M/S 按钮）。
            if *editing_track == Some(ti.index) {
                let gap = 2.0;
                let total_btn_w = 2.0 * btn_size.x + gap;
                let icon_x = if !is_conductor {
                    row_rect.max.x - total_btn_w - 6.0 - gap - btn_size.x
                } else {
                    row_rect.max.x - 6.0 - btn_size.x
                };
                let icon_y = badge_rect.center().y - btn_size.y * 0.5;
                let icon_rect = egui::Rect::from_min_size(
                    egui::pos2(icon_x, icon_y),
                    btn_size,
                );
                painter.text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    ICON_EDIT.codepoint,
                    egui::FontId::new(14.0, ICON_EDIT.font_family()),
                    egui::Color32::WHITE,
                );
            }
        } else {
            let font = egui::FontId::proportional((*row_height * 0.45).clamp(8.0, 14.0));
            painter.text(
                egui::pos2(text_x, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &track_num_text,
                font,
                egui::Color32::WHITE.gamma_multiply(0.85),
            );

            let name = &ti.name;
            let name_font = egui::FontId::proportional((*row_height * 0.45).clamp(8.0, 14.0));
            painter.text(
                egui::pos2(text_x + 40.0, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                egui::Color32::WHITE.gamma_multiply(0.85),
            );
        }
    }

    // ── Click handling ──
    let hit = |pos: egui::Pos2| -> Option<usize> {
        let rel_y = pos.y - panel_rect.min.y + *scroll_y;
        let idx = (rel_y / *row_height).floor() as usize;
        if idx >= num_tracks { None } else { Some(idx) }
    };

    if resp.double_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if let Some(idx) = hit(pos) {
                // 双击：打开 PR 并把此 track 设为 pencil/automation 的编辑目标。
                // 单击切换选择不会改变 editing_track（ICON 只在双击时切换）。
                *editing_track = Some(track_info[idx].index);
                *request_pianoroll = true;
            }
        }
    } else if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if let Some(idx) = hit(pos) {
                let track_idx = track_info[idx].index;
                let shift = ui.input(|i| i.modifiers.shift);
                let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

                if shift {
                    // Range-select from anchor to this track.
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
                    // Toggle this track.
                    if track_selected.contains(&track_idx) {
                        track_selected.remove(&track_idx);
                    } else {
                        track_selected.insert(track_idx);
                    }
                    *selection_anchor = Some(track_idx);
                } else {
                    // Plain click: replace selection.
                    track_selected.clear();
                    track_selected.insert(track_idx);
                    *selection_anchor = Some(track_idx);
                }
                *info_content = Some(crate::right_panel::InfoContent::Track);
            }
        }
    }

    // ── Right-click context menu ──
    // On secondary click, select the track under the cursor and record its
    // index in egui temp data so the context_menu closure (which may run on
    // subsequent frames while the menu stays open) can recover it.
    let ctx_menu_idx_id = egui::Id::new("track_ctx_menu_idx");
    if resp.secondary_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if let Some(idx) = hit(pos) {
                let track_idx = track_info[idx].index;
                if !track_selected.contains(&track_idx) {
                    track_selected.clear();
                    track_selected.insert(track_idx);
                    *selection_anchor = Some(track_idx);
                }
                *info_content = Some(crate::right_panel::InfoContent::Track);
                ui.ctx().data_mut(|d| d.insert_temp(ctx_menu_idx_id, idx));
            }
        }
    }

    resp.context_menu(|ui| {
        let idx = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(ctx_menu_idx_id))
            .unwrap_or(0);
        let track_idx = track_info.get(idx).map(|t| t.index).unwrap_or(0);
        let is_conductor = conductor_track_idx == Some(track_idx);

        if !is_conductor {
            if ui.button("在此轨道下方添加").clicked() {
                actions.push(TrackAction::AddTrack { after_idx: Some(idx) });
                ui.close();
            }
            if ui.button("在此轨道上方添加").clicked() {
                actions.push(TrackAction::AddTrack { after_idx: Some(idx.saturating_sub(1)) });
                ui.close();
            }
            ui.separator();
            if idx > 0 && conductor_track_idx != Some((idx - 1) as u16) {
                if ui.button("上移").clicked() {
                    actions.push(TrackAction::MoveUp { idx });
                    ui.close();
                }
            }
            if idx < num_tracks - 1 {
                if ui.button("下移").clicked() {
                    actions.push(TrackAction::MoveDown { idx });
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("删除轨道").clicked() {
                actions.push(TrackAction::RemoveTrack { idx });
                ui.close();
            }
        } else {
            // Conductor track: only allow adding after
            if ui.button("在此轨道下方添加").clicked() {
                actions.push(TrackAction::AddTrack { after_idx: Some(idx) });
                ui.close();
            }
        }
    });

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

    if resp.hovered() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y.abs() > 0.5 {
            *scroll_y = (*scroll_y - scroll_delta.y).max(0.0);
        }
    }

    (audio_dirty, actions)
}

/// Paint an 18x18 inline button with a one-letter label and click handling.
fn draw_inline_button(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    active: bool,
    active_color: egui::Color32,
    id: egui::Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, egui::Sense::click());
    let hovered = resp.hovered();

    let (fill, text_col) = if active {
        let f = if hovered {
            active_color.gamma_multiply(1.15)
        } else {
            active_color
        };
        (f, egui::Color32::BLACK)
    } else {
        let f = if hovered {
            egui::Color32::from_gray(70)
        } else {
            egui::Color32::from_gray(45)
        };
        (f, egui::Color32::from_gray(180))
    };

    painter.rect_filled(rect, 3.0, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        text_col,
    );

    resp
}
