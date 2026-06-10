use eframe::egui;

use crate::document::Document;

/// Show the Info panel for the selected track.
///
/// Displays track name, port, channel, mute/solo controls, and summary
/// metadata.  When no document or no track is selected, shows a placeholder.
pub fn show(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
) {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（未打开文档）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return;
    };

    let num_tracks = doc.midi.track_ports.len();
    if num_tracks == 0 {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（无音轨）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return;
    }

    // ── Track selector ──
    let track_names: Vec<String> = doc
        .track_names
        .iter()
        .enumerate()
        .map(|(i, name)| format!("{:03} – {}", i + 1, name))
        .collect();

    let sel = doc.track_selected.unwrap_or(0) as usize;
    let sel_idx = sel.min(num_tracks - 1);

    egui::ComboBox::from_id_salt("info_track_sel")
        .selected_text(&track_names[sel_idx])
        .show_ui(ui, |ui| {
            for (i, tn) in track_names.iter().enumerate() {
                if ui.selectable_label(i == sel_idx, tn).clicked() {
                    doc.track_selected = Some(i as u16);
                }
            }
        });

    ui.add_space(6.0);

    let track_idx = doc.track_selected.unwrap_or(0) as usize;
    let track_idx = track_idx.min(num_tracks - 1);

    // ── Track name ──
    let mut name_change: Option<String> = None;
    ui.horizontal(|ui| {
        ui.label("音轨名称:");
        let mut name = doc.track_names[track_idx].clone();
        let resp = ui.add_sized(
            egui::vec2(ui.available_width().max(60.0), 18.0),
            egui::TextEdit::singleline(&mut name),
        );
        if resp.changed() {
            name_change = Some(name);
        }
    });
    if let Some(new_name) = name_change {
        doc.track_names[track_idx] = new_name.clone();
        if let Some(ti_mut) = doc.track_info_cache.get_mut(track_idx) {
            ti_mut.name = new_name;
        }
    }
    let ti = &doc.track_info_cache[track_idx];

    ui.add_space(4.0);

    // ── Port (read for now, editable in future) ──
    let port_letter = match ti.port {
        0 => 'A',
        1 => 'B',
        2 => 'C',
        3 => 'D',
        4 => 'E',
        5 => 'F',
        6 => 'G',
        7 => 'H',
        8 => 'I',
        9 => 'J',
        10 => 'K',
        11 => 'L',
        12 => 'M',
        13 => 'N',
        14 => 'O',
        15 => 'P',
        _ => '?',
    };
    ui.horizontal(|ui| {
        ui.label("端口:");
        ui.label(
            egui::RichText::new(format!("Port {}", port_letter))
                .color(egui::Color32::from_gray(180))
                .size(13.0),
        );
    });

    ui.add_space(2.0);

    // ── Channel ──
    ui.horizontal(|ui| {
        ui.label("通道:");
        ui.label(
            egui::RichText::new(format!("{:02}", ti.channel))
                .color(egui::Color32::from_gray(180))
                .size(13.0),
        );
    });

    ui.add_space(6.0);

    // ── Mute / Solo ──
    // Ensure track_overrides is long enough
    while doc.track_overrides.len() <= track_idx {
        doc.track_overrides
            .push(crate::document::TrackOverride::default());
    }

    let muted = doc.track_overrides[track_idx].muted;
    let soloed = doc.track_overrides[track_idx].soloed;

    let mut mute_clicked = false;
    let mut solo_clicked = false;

    ui.horizontal(|ui| {
        // Mute button
        let mute_label = if muted { "🔇 静音" } else { "🔊 静音" };
        let mute_color = if muted {
            egui::Color32::from_rgb(220, 80, 80)
        } else {
            egui::Color32::from_gray(140)
        };
        let r1 = ui.add(
            egui::Button::new(egui::RichText::new(mute_label).color(mute_color).size(12.0))
                .min_size(egui::vec2(60.0, 22.0)),
        );

        ui.add_space(4.0);

        // Solo button
        let solo_label = if soloed { "🔊 独奏" } else { "🔈 独奏" };
        let solo_color = if soloed {
            egui::Color32::from_rgb(240, 200, 60)
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
        while doc.track_overrides.len() <= track_idx {
            doc.track_overrides
                .push(crate::document::TrackOverride::default());
        }
        if mute_clicked {
            doc.track_overrides[track_idx].muted = !muted;
        }
        if solo_clicked {
            doc.track_overrides[track_idx].soloed = !soloed;
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
    if let Some(pc) = doc.pc_map_cache.get(&(global_ch as u8)) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("音色:")
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );
            ui.label(egui::RichText::new(format!("PC {}", pc)).size(11.0));
        });
    }
}

/// Compute the per-track skip mask and send it to the audio engine.
fn send_skip_tracks(doc: &Document, audio: Option<&yinhe_audio::CpalAudioHandle>) {
    let has_solo = doc.track_overrides.iter().any(|t| t.soloed);
    let skip: Vec<bool> = doc
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
