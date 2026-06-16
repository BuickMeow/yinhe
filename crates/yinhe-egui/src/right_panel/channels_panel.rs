use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::dialogs::settings::AudioSettings;
use crate::document::Document;

/// Show the XSynth channel-mapping panel.
///
/// Displays how many XSynth channels were created and which MIDI
/// (port, channel) each one maps to, plus whether each port has a
/// SoundFont loaded.
pub fn show(ui: &mut egui::Ui, doc: Option<&mut Document>, settings: &AudioSettings) {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（未打开文档）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return;
    };

    ui.add_space(4.0);
    ui.label(egui::RichText::new("XSynth 通道映射").size(14.0).strong());
    ui.add_space(4.0);

    // Build active mask from the MIDI file (same logic as
    // yinhe_audio::channels_for_midi).
    let mut active = [false; 256];
    for notes in &doc.midi().key_notes {
        for note in notes {
            if note.velocity > 1 {
                let ch = doc.midi()
                    .track_channels
                    .get(note.track as usize)
                    .copied()
                    .unwrap_or(0) as usize;
                active[ch] = true;
            }
        }
    }
    for ev in &doc.midi().control_events {
        let track = match ev {
            yinhe_midi::MidiControlEvent::ControlChange { track, .. }
            | yinhe_midi::MidiControlEvent::ProgramChange { track, .. }
            | yinhe_midi::MidiControlEvent::PitchBend { track, .. } => *track,
        };
        let ch = doc.midi()
            .track_channels
            .get(track as usize)
            .copied()
            .unwrap_or(0) as usize;
        if ch < 256 {
            active[ch] = true;
        }
    }

    let max_active_ch = active.iter().rposition(|&a| a).unwrap_or(0);
    let num_ports = ((max_active_ch / 16) + 1).max(1);
    let num_channels = num_ports * 16;

    // Compute dense mapping exactly as the audio engine does.
    let mut dense = [u32::MAX; 256];
    let mut next_dense: u32 = 0;
    for src in 0..num_channels {
        if active[src] {
            dense[src] = next_dense;
            next_dense += 1;
        }
    }
    let compacted_channels = next_dense.max(16);

    ui.horizontal(|ui| {
        ui.label(format!("MIDI 端口数: {}", num_ports));
        ui.label(format!("XSynth 通道数: {}", compacted_channels));
    });
    ui.add_space(4.0);

    // Build reverse map: dense -> list of source channels
    let mut reverse: Vec<Vec<u8>> = vec![Vec::new(); compacted_channels as usize];
    for src in 0..num_channels {
        let d = dense[src];
        if d != u32::MAX {
            reverse[d as usize].push(src as u8);
        }
    }

    TableBuilder::new(ui)
        .id_salt("channel_map_table")
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(40.0).at_least(30.0).clip(true))
        .column(Column::initial(130.0).at_least(60.0).clip(true))
        .column(Column::initial(30.0).at_least(24.0))
        .column(Column::initial(140.0).at_least(60.0).clip(true))
        .header(20.0, |mut h| {
            h.col(|ui| { ui.label(egui::RichText::new("XSynth").strong().size(11.0)); });
            h.col(|ui| { ui.label(egui::RichText::new("源通道").strong().size(11.0)); });
            h.col(|ui| { ui.label(egui::RichText::new("活跃").strong().size(11.0)); });
            h.col(|ui| { ui.label(egui::RichText::new("音色库").strong().size(11.0)); });
        })
        .body(|body| {
            body.rows(18.0, compacted_channels as usize, |mut row| {
                let d = row.index();
                let sources = &reverse[d as usize];
                let source_label = if sources.is_empty() {
                    "—".to_string()
                } else {
                    sources
                        .iter()
                        .map(|&src| {
                            let port = (src >> 4) as u8;
                            let ch = (src & 0x0F) + 1;
                            format!(
                                "{}{:02}",
                                (b'A' + port) as char,
                                ch
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };

                let is_active = !sources.is_empty();
                let color = if is_active {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_gray(120)
                };

                let sf_status = if sources.is_empty() {
                    None
                } else {
                    let port = (sources[0] >> 4) as u8;
                    let has_sf = if settings.global_sf_config.global_enabled {
                        settings.global_sf_config.ports[0]
                            .iter()
                            .any(|e| e.enabled)
                    } else {
                        doc.edit.project_sf
                            .overrides
                            .iter()
                            .any(|(p, entries)| *p == port && entries.iter().any(|e| e.enabled))
                    };
                    Some(has_sf)
                };

                row.col(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:3}", d))
                            .monospace()
                            .color(color),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        egui::RichText::new(source_label)
                            .monospace()
                            .color(color),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        egui::RichText::new(if is_active { "●" } else { "○" })
                            .color(if is_active {
                                egui::Color32::from_rgb(80, 200, 80)
                            } else {
                                egui::Color32::from_gray(120)
                            }),
                    );
                });
                match sf_status {
                    None => {
                        row.col(|_| {});
                    }
                    Some(true) => {
                        row.col(|_| {});
                    }
                    Some(false) => {
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new("● 未加载音色库")
                                    .color(egui::Color32::from_rgb(230, 160, 40))
                                    .size(11.0),
                            );
                        });
                    }
                }
            });
        });
}
