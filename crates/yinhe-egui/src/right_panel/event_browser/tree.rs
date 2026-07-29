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
pub(super) fn render_tree(ui: &mut egui::Ui, doc: &yinhe_editor_core::document::Document, state: &mut EventBrowserState) {
    let model = &doc.data.model;
    let conductor_idx = doc.edit.conductor_track_idx;
    let groups = group_tracks_by_port_channel(model, conductor_idx);

    render_leaf_item(ui, "project.json", ICON_DESCRIPTION, 0, SelectedItem::ProjectJson, state);
    render_leaf_item(ui, "mapping.json", ICON_DESCRIPTION, 0, SelectedItem::MappingJson, state);

    let has_tempo = !model.conductor.tempo.events.is_empty();
    let has_ts = !model.conductor.time_sig.is_empty();
    let has_key_sig = !model.conductor.key_sig.is_empty();
    let has_markers = !model.conductor.markers.is_empty();
    let has_cond_lyrics = !model.conductor.lyrics.is_empty();
    let has_cond_chord = !model.conductor.chord.is_empty();
    if has_tempo || has_ts || has_key_sig || has_markers || has_cond_lyrics || has_cond_chord {
        let cond_expanded = state.expanded_keys.contains(&ArchiveKey::Conductor);
        let child_count = has_tempo as usize
            + has_ts as usize
            + has_key_sig as usize
            + has_markers as usize
            + has_cond_lyrics as usize
            + has_cond_chord as usize;
        if render_dir_row(ui, "Conductor", 0, cond_expanded, child_count) {
            toggle_key(state, ArchiveKey::Conductor);
        }
        if cond_expanded {
            if has_tempo {
                render_leaf_item(
                    ui,
                    &format!("Tempo ({})", model.conductor.tempo.events.len()),
                    ICON_SPEED,
                    1,
                    SelectedItem::Automation { track: 0, target: AutomationTarget::Tempo },
                    state,
                );
            }
            if has_ts {
                render_leaf_item(
                    ui,
                    &format!("TimeSig ({})", model.conductor.time_sig.len()),
                    ICON_SCHEDULE,
                    1,
                    SelectedItem::TimeSig,
                    state,
                );
            }
            if has_key_sig {
                render_leaf_item(
                    ui,
                    &format!("KeySig ({})", model.conductor.key_sig.len()),
                    ICON_MUSIC_OFF,
                    1,
                    SelectedItem::KeySig,
                    state,
                );
            }
            if has_markers {
                render_leaf_item(
                    ui,
                    &format!("Markers ({})", model.conductor.markers.len()),
                    ICON_BOOKMARK,
                    1,
                    SelectedItem::Markers,
                    state,
                );
            }
            if has_cond_lyrics {
                render_leaf_item(
                    ui,
                    &format!("Lyrics ({})", model.conductor.lyrics.len()),
                    ICON_SUBTITLES,
                    1,
                    SelectedItem::ConductorLyrics,
                    state,
                );
            }
            if has_cond_chord {
                render_leaf_item(
                    ui,
                    &format!("Chord ({})", model.conductor.chord.len()),
                    ICON_LIBRARY_MUSIC,
                    1,
                    SelectedItem::ConductorChord,
                    state,
                );
            }
        }
    }

    for (&port, channels) in &groups {
        let port_key = ArchiveKey::Port(port);
        let port_expanded = state.expanded_keys.contains(&port_key);
        let port_track_count: usize = channels.values().map(|v| v.len()).sum();
        let port_label = t!("event_browser.port_tracks", port = port_letter(port), n = port_track_count).to_string();
        if render_dir_row(ui, &port_label, 0, port_expanded, channels.len()) {
            toggle_key(state, port_key);
        }
        if !port_expanded {
            continue;
        }

        for (&channel, track_indices) in channels {
            let ch_key = ArchiveKey::Channel(port, channel);
            let ch_expanded = state.expanded_keys.contains(&ch_key);
            let ch_label = t!("event_browser.channel_tracks", ch = channel + 1, n = track_indices.len()).to_string();
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
    let summary = format!("{} notes \u{00b7} {} auto \u{00b7} {} PC", note_count, track.automation_lanes.len(), pc_count);

    let row_bg = if is_selected {
        egui::Color32::from_rgb(40, 50, 70)
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

                let chev = if expanded { ICON_EXPAND_MORE } else { ICON_CHEVRON_RIGHT };
                if ui.add(egui::Label::new(chev.rich_text().size(13.0).color(egui::Color32::from_gray(190))).sense(egui::Sense::click())).clicked() {
                    toggled = true;
                }

                ui.label(
                    ICON_AUDIOTRACK
                        .rich_text()
                        .size(12.0)
                        .color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(160) }),
                );

                let name_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&label_text)
                            .size(11.0)
                            .monospace()
                            .color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(220) }),
                    )
                    .sense(egui::Sense::click()),
                );
                if name_resp.clicked() {
                    selected = true;
                }

                ui.label(
                    egui::RichText::new(format!("[{}]", summary))
                        .size(10.0)
                        .color(egui::Color32::from_gray(110)),
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
        if note_count > 0 {
            render_leaf_item(ui, &format!("Notes ({})", note_count), ICON_MUSIC_NOTE, 3, SelectedItem::Notes { track: idx }, state);
        }
        // 所有 automation lane 都作为叶子显示（CC/PB/RPN/NRPN）
        for lane in &track.automation_lanes {
            let icon = automation_icon(&lane.target);
            render_leaf_item(
                ui,
                &format!("{} ({})", lane.target.display_name(), lane.events.len()),
                icon,
                3,
                SelectedItem::Automation { track: idx, target: lane.target.clone() },
                state,
            );
        }
        if pc_count > 0 {
            render_leaf_item(ui, &format!("Program Change ({})", pc_count), ICON_PALETTE, 3, SelectedItem::ProgramChange { track: idx }, state);
        }
        if !track.lyrics.is_empty() {
            render_leaf_item(
                ui,
                &format!("Lyrics ({})", track.lyrics.len()),
                ICON_SUBTITLES,
                3,
                SelectedItem::Lyrics { track: idx },
                state,
            );
        }
        if !track.chord.is_empty() {
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

fn render_dir_row(ui: &mut egui::Ui, name: &str, depth: usize, expanded: bool, child_count: usize) -> bool {
    let mut toggled = false;
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(2, 1)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(depth as f32 * 14.0);
            let chev = if expanded { ICON_EXPAND_MORE } else { ICON_CHEVRON_RIGHT };
            if ui.add(egui::Label::new(chev.rich_text().size(13.0).color(egui::Color32::from_gray(190))).sense(egui::Sense::click())).clicked() { toggled = true; }
            let folder = if expanded { ICON_FOLDER_OPEN } else { ICON_FOLDER };
            if ui.add(egui::Label::new(folder.rich_text().size(13.0).color(egui::Color32::from_rgb(220, 180, 90))).sense(egui::Sense::click())).clicked() { toggled = true; }
            ui.label(egui::RichText::new(name).size(11.0).color(egui::Color32::from_gray(220)));
            ui.label(egui::RichText::new(format!("({})", child_count)).size(10.0).color(egui::Color32::from_gray(110)));
        });
    });
    toggled
}

fn render_leaf_item(ui: &mut egui::Ui, name: &str, icon: egui_material_icons::MaterialIcon, depth: usize, item: SelectedItem, state: &mut EventBrowserState) {
    let is_selected = state.selected_item.as_ref() == Some(&item);
    let bg = if is_selected { egui::Color32::from_rgb(40, 50, 70) } else { egui::Color32::TRANSPARENT };
    let frame_r = egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(2, 1)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(depth as f32 * 14.0);
            ui.add_space(14.0);
            ui.label(icon.rich_text().size(12.0).color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(160) }));
            ui.label(egui::RichText::new(name).size(11.0).monospace().color(if is_selected { egui::Color32::WHITE } else { egui::Color32::from_gray(200) }));
        });
    });
    if frame_r.response.interact(egui::Sense::click()).clicked() {
        state.selected_item = Some(item);
        state.selected_track = None;
        state.event_page = 0;
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
