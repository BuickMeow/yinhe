use eframe::egui;

use crate::document::Document;

/// Show the XSynth channel-mapping panel.
///
/// Displays how many XSynth channels were created and which MIDI
/// (port, channel) each one maps to.  Helps confirm that multi-port
/// MIDI files are wired up correctly.
pub fn show(ui: &mut egui::Ui, doc: Option<&mut Document>) {
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
    for notes in &doc.midi.key_notes {
        for note in notes {
            if note.velocity > 1 {
                active[note.channel as usize] = true;
            }
        }
    }
    for ev in &doc.midi.control_events {
        let ch = match ev {
            yinhe_midi::MidiControlEvent::ControlChange { channel, .. }
            | yinhe_midi::MidiControlEvent::ProgramChange { channel, .. }
            | yinhe_midi::MidiControlEvent::PitchBend { channel, .. } => *channel,
        };
        if (ch as usize) < 256 {
            active[ch as usize] = true;
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

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("channel_map_grid")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("XSynth").strong());
                ui.label(egui::RichText::new("源通道").strong());
                ui.label(egui::RichText::new("活跃").strong());
                ui.end_row();

                for d in 0..compacted_channels {
                    let sources = &reverse[d as usize];
                    let source_label = if sources.is_empty() {
                        "—".to_string()
                    } else {
                        sources
                            .iter()
                            .map(|&src| {
                                let port = (src >> 4) as u8;
                                let ch = (src & 0x0F) + 1; // 1-based for display
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
                        ui.visuals().text_color()
                    } else {
                        egui::Color32::from_gray(120)
                    };

                    ui.label(
                        egui::RichText::new(format!("{:3}", d))
                            .monospace()
                            .color(color),
                    );
                    ui.label(
                        egui::RichText::new(source_label)
                            .monospace()
                            .color(color),
                    );
                    ui.label(
                        egui::RichText::new(if is_active { "●" } else { "○" })
                            .color(if is_active {
                                egui::Color32::from_rgb(80, 200, 80)
                            } else {
                                egui::Color32::from_gray(120)
                            }),
                    );
                    ui.end_row();
                }
            });
    });
}
