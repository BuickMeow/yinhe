use std::sync::Arc;

use eframe::egui;

use yinhe_editor_core::document::Document;

/// Show the Info panel for the selected track.
///
/// Displays track name, port, channel, mute/solo controls, and summary
/// metadata.  When no document or no track is selected, shows a placeholder.
///
/// Returns `true` if the port or channel was changed (caller should tear
/// down the audio engine so it gets rebuilt with the new channel map).
pub fn show(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
) -> bool {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（未打开文档）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return false;
    };

    let num_tracks = doc.data.midi().track_ports.len();
    if num_tracks == 0 {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（无音轨）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return false;
    }

    // ── Track selector ──
    let track_names: Vec<String> = doc
        .data
        .track_names
        .iter()
        .enumerate()
        .map(|(i, name)| format!("{:03} – {}", i + 1, name))
        .collect();

    let sel_idx = doc
        .edit
        .track_selected
        .iter()
        .next()
        .copied()
        .map(|i| (i as usize).min(num_tracks - 1))
        .unwrap_or(0);

    egui::ComboBox::from_id_salt("info_track_sel")
        .selected_text(&track_names[sel_idx])
        .show_ui(ui, |ui| {
            for (i, tn) in track_names.iter().enumerate() {
                if ui.selectable_label(i == sel_idx, tn).clicked() {
                    doc.edit.track_selected.clear();
                    doc.edit.track_selected.insert(i as u16);
                }
            }
        });

    ui.add_space(6.0);

    // ── Multi-select summary ──
    if doc.edit.track_selected.len() > 1 {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("已选 {} 个音轨", doc.edit.track_selected.len()))
                .strong()
                .size(14.0)
                .color(egui::Color32::from_gray(220)),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("（多选模式：卷帘将显示所有选中音轨的音符）")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        return false;
    }

    let Some(&track_idx) = doc.edit.track_selected.iter().next() else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（未选中音轨）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return false;
    };
    let track_idx = track_idx as usize;
    let track_idx = track_idx.min(num_tracks - 1);

    // ── Conductor track: show a simplified read-only view ──
    if Some(track_idx as u16) == doc.edit.conductor_track_idx {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Conductor")
                .strong()
                .size(14.0)
                .color(egui::Color32::from_gray(220)),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("（指挥轨：tempo / time-sig 等全局元事件）")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        ui.add_space(8.0);

        if !doc.data.project_name.is_empty() {
            ui.horizontal(|ui| {
                ui.label("歌曲标题:");
                ui.label(
                    egui::RichText::new(&doc.data.project_name)
                        .color(egui::Color32::from_gray(200))
                        .size(13.0),
                );
            });
            ui.add_space(2.0);
        }

        ui.horizontal(|ui| {
            ui.label("Tempo 数:");
            ui.label(
                egui::RichText::new(format!("{}", doc.data.midi().tempo_segments.len()))
                    .color(egui::Color32::from_gray(180))
                    .size(13.0),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Time-sig 数:");
            ui.label(
                egui::RichText::new(format!("{}", doc.data.midi().time_sig_events.len()))
                    .color(egui::Color32::from_gray(180))
                    .size(13.0),
            );
        });
        return false;
    }

    // ── Track name ──
    let mut name_change: Option<String> = None;
    let mut name_resp_id: Option<egui::Id> = None;
    let mut name_gained_focus = false;
    let mut name_lost_focus = false;
    ui.horizontal(|ui| {
        ui.label("音轨名称:");
        let mut name = doc.data.track_names[track_idx].clone();
        let resp = ui.add_sized(
            egui::vec2(ui.available_width().max(60.0), 18.0),
            egui::TextEdit::singleline(&mut name).id_salt(("track_name", track_idx)),
        );
        if resp.changed() {
            name_change = Some(name);
        }
        name_resp_id = Some(resp.id);
        name_gained_focus = resp.gained_focus();
        name_lost_focus = resp.lost_focus();
    });
    if let Some(id) = name_resp_id {
        if name_gained_focus {
            yinhe_editor_core::history::begin_edit(&doc.data, &mut doc.edit.pending_edits, id.value(), "Edit track name");
        }
        if let Some(new_name) = name_change {
            doc.data.track_names[track_idx] = new_name.clone();
            if let Some(ti_mut) = doc.edit.track_info_cache.get_mut(track_idx) {
                ti_mut.name = new_name;
            }
        }
        if name_lost_focus {
            yinhe_editor_core::history::commit_edit(&doc.data, &mut doc.history, &mut doc.edit.pending_edits, id.value());
        }
    }
    let ti = &doc.edit.track_info_cache[track_idx];

    ui.add_space(4.0);

    // ── Port / Channel (editable) ──
    let mut port_changed = false;
    let mut new_port = ti.port;
    let mut new_ch = ti.channel;

    ui.horizontal(|ui| {
        ui.label("端口/通道:");

        // Port combo: A(0) .. P(15)
        let port_options: Vec<String> = (0..16)
            .map(|p| format!("Port {}", (b'A' + p) as char))
            .collect();
        let port_sel = egui::ComboBox::from_id_salt("track_port")
            .selected_text(format!("Port {}", (b'A' + ti.port) as char))
            .width(70.0)
            .show_ui(ui, |ui| {
                for (i, label) in port_options.iter().enumerate() {
                    if ui.selectable_label(i == ti.port as usize, label).clicked() {
                        new_port = i as u8;
                        port_changed = true;
                    }
                }
            });

        ui.add_space(4.0);

        // Channel combo: 01 .. 16
        let ch_options: Vec<String> = (1..=16).map(|c| format!("{:02}", c)).collect();
        let _ch_sel = egui::ComboBox::from_id_salt("track_channel")
            .selected_text(format!("{:02}", ti.channel))
            .width(50.0)
            .show_ui(ui, |ui| {
                for (i, label) in ch_options.iter().enumerate() {
                    if ui.selectable_label(i + 1 == ti.channel as usize, label).clicked() {
                        new_ch = (i + 1) as u8;
                        port_changed = true;
                    }
                }
            });
    });

    // Apply port/channel change
    if port_changed {
        // Push pre-change snapshot for undo.
        let snap = doc.data.snapshot("Change port/channel");
        doc.history.push(snap);
        {
            let model = Arc::make_mut(&mut doc.data.model);
            if track_idx < model.tracks.len() {
                let td = Arc::make_mut(&mut model.tracks[track_idx]);
                td.port = new_port;
                td.channel = new_ch.saturating_sub(1);
            }
        }
        // Rebuild metadata and caches
        doc.data.rebuild_model();
        doc.edit.track_info_cache = doc.data.track_info();
        doc.edit.pc_map_cache = doc.data.pc_map_cache();
        doc.data.bump_version();
        // Signal caller to tear down audio
        return true;
    }

    ui.add_space(6.0);

    // ── Mute / Solo ──
    // Ensure track_overrides is long enough
    while doc.edit.track_overrides.len() <= track_idx {
        doc.edit.track_overrides
            .push(yinhe_editor_core::document::TrackOverride::default());
    }

    let muted = doc.edit.track_overrides[track_idx].muted;
    let soloed = doc.edit.track_overrides[track_idx].soloed;

    let mut mute_clicked = false;
    let mut solo_clicked = false;

    ui.horizontal(|ui| {
        // Mute button (yellow)
        let mute_label = if muted { "🔇 静音" } else { "🔊 静音" };
        let mute_color = if muted {
            crate::theme::MUTE_ACTIVE
        } else {
            egui::Color32::from_gray(140)
        };
        let r1 = ui.add(
            egui::Button::new(egui::RichText::new(mute_label).color(mute_color).size(12.0))
                .min_size(egui::vec2(60.0, 22.0)),
        );

        ui.add_space(4.0);

        // Solo button (red)
        let solo_label = if soloed { "🔊 独奏" } else { "🔈 独奏" };
        let solo_color = if soloed {
            crate::theme::SOLO_ACTIVE
        } else {
            egui::Color32::from_gray(140)
        };
        let r2 = ui.add(
            egui::Button::new(egui::RichText::new(solo_label).color(solo_color).size(12.0))
                .min_size(egui::vec2(60.0, 22.0)),
        );

        mute_clicked = r1.clicked();
        solo_clicked = r2.clicked();
    });

    // Apply changes after the closure so we can borrow doc mutably again
    if mute_clicked || solo_clicked {
        // Ensure track_overrides is long enough (re-check since closure is done)
        while doc.edit.track_overrides.len() <= track_idx {
            doc.edit.track_overrides
                .push(yinhe_editor_core::document::TrackOverride::default());
        }
        if mute_clicked {
            doc.edit.track_overrides[track_idx].muted = !muted;
        }
        if solo_clicked {
            doc.edit.track_overrides[track_idx].soloed = !soloed;
        }
        send_skip_tracks(doc, audio);
    }

    ui.add_space(8.0);

    // ── Summary ──
    ui.separator();
    ui.add_space(4.0);
    ui.label(egui::RichText::new("属性摘要").size(11.0).strong());
    ui.add_space(2.0);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("音符数:")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        ui.label(egui::RichText::new(format!("{}", ti.note_count)).size(11.0));
    });

    // Show program change if available
    let global_ch = ti.port as u32 * 16 + (ti.channel as u32 - 1);
    if let Some(pc) = doc.edit.pc_map_cache.get(&(global_ch as u8)) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("音色:")
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );
            ui.label(egui::RichText::new(format!("PC {}", pc)).size(11.0));
        });
    }

    false
}

/// Compute the per-track skip mask and send it to the audio engine.
pub(crate) fn send_skip_tracks(doc: &Document, audio: Option<&yinhe_audio::CpalAudioHandle>) {
    let has_solo = doc.edit.track_overrides.iter().any(|t| t.soloed);
    let skip: Vec<bool> = doc
        .edit
        .track_overrides
        .iter()
        .map(|ov| if has_solo { !ov.soloed } else { ov.muted })
        .collect();

    if let Some(audio) = audio {
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SkipTracks { skip });
    }
}
