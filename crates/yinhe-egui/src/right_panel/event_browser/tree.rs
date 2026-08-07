//! 左侧树状导航：project/mapping → Conductor → Port/Channel/Track。
//!
//! Track 展开后显示 Notes / 各 Automation lane（CC/PB/RPN/NRPN）/ Program Change。

use eframe::egui;
use egui_material_icons::icons::*;

use rust_i18n::t;
use yinhe_core::YinModel;
use yinhe_types::AutomationTarget;

use super::state::{ArchiveKey, EventBrowserState, SelectedItem};
use super::{group_tracks_by_port_channel, port_letter};

/// 渲染整个树状导航。
pub(super) fn render_tree(
    ui: &mut egui::Ui,
    doc: &yinhe_editor_core::document::Document,
    state: &mut EventBrowserState,
) {
    let model = &doc.data.model;
    let conductor_idx = doc.edit.conductor_track_idx;
    let groups = group_tracks_by_port_channel(model, conductor_idx);

    render_leaf_item(
        ui,
        "project.json",
        ICON_DESCRIPTION,
        0,
        SelectedItem::ProjectJson,
        state,
    );
    render_leaf_item(
        ui,
        "mapping.json",
        ICON_DESCRIPTION,
        0,
        SelectedItem::MappingJson,
        state,
    );

    // Conductor 级事件始终显示（即使为 0），方便用户新建第一个事件。
    let tempo_count = model.conductor.tempo.events.len();
    let ts_count = model.conductor.time_sig.len();
    let key_sig_count = model.conductor.key_sig.len();
    let markers_count = model.conductor.markers.len();
    let cond_lyrics_count = model.conductor.lyrics.len();
    let cond_chord_count = model.conductor.chord.len();
    let cond_expanded = state.expanded_keys.contains(&ArchiveKey::Conductor);
    let child_count = 6; // 始终 6 个子节点
    if render_dir_row(ui, "Conductor", 0, cond_expanded, child_count) {
        toggle_key(state, ArchiveKey::Conductor);
    }
    if cond_expanded {
        render_leaf_item(
            ui,
            &format!("Tempo ({})", tempo_count),
            ICON_SPEED,
            1,
            SelectedItem::Automation {
                track: 0,
                target: AutomationTarget::Tempo,
            },
            state,
        );
        render_leaf_item(
            ui,
            &format!("TimeSig ({})", ts_count),
            ICON_SCHEDULE,
            1,
            SelectedItem::TimeSig,
            state,
        );
        render_leaf_item(
            ui,
            &format!("KeySig ({})", key_sig_count),
            ICON_MUSIC_OFF,
            1,
            SelectedItem::KeySig,
            state,
        );
        render_leaf_item(
            ui,
            &format!("Markers ({})", markers_count),
            ICON_BOOKMARK,
            1,
            SelectedItem::Markers,
            state,
        );
        render_leaf_item(
            ui,
            &format!("Lyrics ({})", cond_lyrics_count),
            ICON_SUBTITLES,
            1,
            SelectedItem::ConductorLyrics,
            state,
        );
        render_leaf_item(
            ui,
            &format!("Chord ({})", cond_chord_count),
            ICON_LIBRARY_MUSIC,
            1,
            SelectedItem::ConductorChord,
            state,
        );
    }

    for (&port, channels) in &groups {
        let port_key = ArchiveKey::Port(port);
        let port_expanded = state.expanded_keys.contains(&port_key);
        let port_track_count: usize = channels.values().map(|v| v.len()).sum();
        let port_label = t!(
            "event_browser.port_tracks",
            port = port_letter(port),
            n = port_track_count
        )
        .to_string();
        if render_dir_row(ui, &port_label, 0, port_expanded, channels.len()) {
            toggle_key(state, port_key);
        }
        if !port_expanded {
            continue;
        }

        for (&channel, track_indices) in channels {
            let ch_key = ArchiveKey::Channel(port, channel);
            let ch_expanded = state.expanded_keys.contains(&ch_key);
            let ch_label = t!(
                "event_browser.channel_tracks",
                ch = channel + 1,
                n = track_indices.len()
            )
            .to_string();
            if render_dir_row(ui, &ch_label, 1, ch_expanded, track_indices.len()) {
                toggle_key(state, ch_key);
            }
            if !ch_expanded {
                continue;
            }

            for &track_idx in track_indices {
                render_track_row(ui, model, track_idx, state);
            }
        }
    }
}

fn render_track_row(ui: &mut egui::Ui, model: &YinModel, idx: u16, state: &mut EventBrowserState) {
    let track = &model.tracks[idx as usize];
    let track_key = ArchiveKey::Track(idx);
    let expanded = state.expanded_keys.contains(&track_key);
    let is_selected = state.selected_track == Some(idx);

    let note_count = *model.track_note_count.get(idx as usize).unwrap_or(&0) as usize;
    let pc_count = track.program_change.len();

    let label_text = if track.name.is_empty() {
        format!("(track #{})", idx)
    } else {
        track.name.clone()
    };
    let summary = format!(
        "{} notes \u{00b7} {} auto \u{00b7} {} PC",
        note_count,
        track.automation_lanes.len(),
        pc_count
    );

    let row_bg = if is_selected {
        crate::theme::ROW_SELECTED_BG
    } else {
        egui::Color32::TRANSPARENT
    };

    let mut toggled = false;
    let mut selected = false;

    egui::Frame::NONE
        .fill(row_bg)
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(2.0 * 14.0);

                let chev = if expanded {
                    ICON_EXPAND_MORE
                } else {
                    ICON_CHEVRON_RIGHT
                };
                if ui
                    .add(
                        egui::Label::new(
                            chev.rich_text()
                                .size(crate::theme::SUB_TITLE_FONT)
                                .color(crate::theme::TEXT_MEDIUM),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    toggled = true;
                }

                ui.add(
                    egui::Label::new(
                        ICON_AUDIOTRACK
                            .rich_text()
                            .size(crate::theme::BODY_FONT)
                            .color(if is_selected {
                                egui::Color32::WHITE
                            } else {
                                crate::theme::TEXT_MUTED
                            }),
                    )
                    .selectable(false),
                );

                let name_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&label_text)
                            .size(crate::theme::SMALL_FONT)
                            .monospace()
                            .color(if is_selected {
                                egui::Color32::WHITE
                            } else {
                                crate::theme::TEXT_PRIMARY
                            }),
                    )
                    .selectable(false)
                    .sense(egui::Sense::click()),
                );
                if name_resp.clicked() {
                    selected = true;
                }

                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("[{}]", summary))
                            .size(crate::theme::SMALL_LABEL_FONT)
                            .color(crate::theme::TEXT_DIMMER),
                    )
                    .selectable(false),
                );
            });
        });

    if toggled {
        toggle_key(state, track_key);
    }
    if selected {
        state.selected_track = Some(idx);
        state.selected_item = None;
    }

    if expanded {
        // per-track 事件始终显示（即使为 0），方便用户新建第一个事件。
        render_leaf_item(
            ui,
            &format!("Notes ({})", note_count),
            ICON_MUSIC_NOTE,
            3,
            SelectedItem::Notes { track: idx },
            state,
        );
        // 所有 automation lane 都作为叶子显示（CC/PB/RPN/NRPN）
        for lane in &track.automation_lanes {
            let icon = automation_icon(&lane.target);
            render_leaf_item(
                ui,
                &format!("{} ({})", lane.target.display_name(), lane.events.len()),
                icon,
                3,
                SelectedItem::Automation {
                    track: idx,
                    target: lane.target.clone(),
                },
                state,
            );
        }
        render_leaf_item(
            ui,
            &format!("Program Change ({})", pc_count),
            ICON_PALETTE,
            3,
            SelectedItem::ProgramChange { track: idx },
            state,
        );
        render_leaf_item(
            ui,
            &format!("Lyrics ({})", track.lyrics.len()),
            ICON_SUBTITLES,
            3,
            SelectedItem::Lyrics { track: idx },
            state,
        );
        render_leaf_item(
            ui,
            &format!("Chord ({})", track.chord.len()),
            ICON_LIBRARY_MUSIC,
            3,
            SelectedItem::Chord { track: idx },
            state,
        );
    }
}

/// 按 AutomationTarget 类型选图标。
fn automation_icon(target: &AutomationTarget) -> egui_material_icons::MaterialIcon {
    match target {
        AutomationTarget::CC { .. } => ICON_SETTINGS,
        AutomationTarget::PitchBend => ICON_EDIT_AUDIO,
        AutomationTarget::Rpn { .. } | AutomationTarget::Nrpn { .. } => ICON_TUNE,
        AutomationTarget::Tempo => ICON_SPEED,
    }
}

// ── Tree row renderers ──

fn render_dir_row(
    ui: &mut egui::Ui,
    name: &str,
    depth: usize,
    expanded: bool,
    child_count: usize,
) -> bool {
    let mut toggled = false;
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(depth as f32 * 14.0);
                let chev = if expanded {
                    ICON_EXPAND_MORE
                } else {
                    ICON_CHEVRON_RIGHT
                };
                if ui
                    .add(
                        egui::Label::new(
                            chev.rich_text()
                                .size(crate::theme::SUB_TITLE_FONT)
                                .color(crate::theme::TEXT_MEDIUM),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    toggled = true;
                }
                let folder = if expanded {
                    ICON_FOLDER_OPEN
                } else {
                    ICON_FOLDER
                };
                if ui
                    .add(
                        egui::Label::new(
                            folder
                                .rich_text()
                                .size(crate::theme::SUB_TITLE_FONT)
                                .color(crate::theme::WARNING_GOLD),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    toggled = true;
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(name)
                            .size(crate::theme::SMALL_FONT)
                            .color(crate::theme::TEXT_PRIMARY),
                    )
                    .selectable(false),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("({})", child_count))
                            .size(crate::theme::SMALL_LABEL_FONT)
                            .color(crate::theme::TEXT_DIMMER),
                    )
                    .selectable(false),
                );
            });
        });
    toggled
}

fn render_leaf_item(
    ui: &mut egui::Ui,
    name: &str,
    icon: egui_material_icons::MaterialIcon,
    depth: usize,
    item: SelectedItem,
    state: &mut EventBrowserState,
) {
    let is_selected = state.selected_item.as_ref() == Some(&item);
    let bg = if is_selected {
        crate::theme::ROW_SELECTED_BG
    } else {
        egui::Color32::TRANSPARENT
    };
    let frame_r = egui::Frame::NONE
        .fill(bg)
        .inner_margin(egui::Margin::symmetric(2, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(depth as f32 * 14.0);
                ui.add_space(14.0);
                ui.add(
                    egui::Label::new(icon.rich_text().size(crate::theme::BODY_FONT).color(
                        if is_selected {
                            egui::Color32::WHITE
                        } else {
                            crate::theme::TEXT_MUTED
                        },
                    ))
                    .selectable(false),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(name)
                            .size(crate::theme::SMALL_FONT)
                            .monospace()
                            .color(if is_selected {
                                egui::Color32::WHITE
                            } else {
                                crate::theme::TEXT_BRIGHT
                            }),
                    )
                    .selectable(false),
                );
            });
        });
    if frame_r.response.interact(egui::Sense::click()).clicked() {
        state.selected_item = Some(item);
        state.selected_track = None;
        state.event_page = 0;
        // 切换选中条目时清空行多选状态
        state.selected_ticks.clear();
        state.last_clicked_tick = None;
    }
}

fn toggle_key(state: &mut EventBrowserState, key: ArchiveKey) {
    if state.expanded_keys.contains(&key) {
        state.expanded_keys.remove(&key);
    } else {
        state.expanded_keys.insert(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_key_inserts_then_removes() {
        let mut state = EventBrowserState::default();
        assert!(!state.expanded_keys.contains(&ArchiveKey::Conductor));
        toggle_key(&mut state, ArchiveKey::Conductor);
        assert!(state.expanded_keys.contains(&ArchiveKey::Conductor));
        toggle_key(&mut state, ArchiveKey::Conductor);
        assert!(!state.expanded_keys.contains(&ArchiveKey::Conductor));
    }
}
