use eframe::egui;

use crate::document::Document;

/// Show the Project Info panel for editing project metadata.
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

    ui.add_space(8.0);

    // ── Project name ──
    ui.label(
        egui::RichText::new("项目名称")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut name = doc.project_name.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut name),
    );
    if resp.changed() {
        doc.project_name = name;
    }

    ui.add_space(6.0);

    // ── Artist ──
    ui.label(
        egui::RichText::new("艺术家")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut artist = doc.project_artist.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut artist),
    );
    if resp.changed() {
        doc.project_artist = artist;
    }

    ui.add_space(6.0);

    // ── PPQ (read-only for now) ──
    ui.label(
        egui::RichText::new(format!("PPQ: {}", doc.midi.ticks_per_beat))
            .color(egui::Color32::from_gray(140))
            .size(11.0),
    );

    ui.add_space(6.0);

    // ── zstd compression level ──
    ui.label(
        egui::RichText::new("压缩级别 (zstd)")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut zstd_level = doc
        .archive
        .as_ref()
        .map(|a| a.compression_level)
        .unwrap_or(0) as i32;
    let resp = ui.add_sized(
        egui::vec2(60.0, 20.0),
        egui::DragValue::new(&mut zstd_level).range(0..=22),
    );
    if resp.changed() {
        if let Some(archive) = &mut doc.archive {
            archive.compression_level = zstd_level;
        }
    }

    ui.add_space(6.0);

    // ── Description ──
    ui.label(
        egui::RichText::new("简介")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut desc = doc.project_description.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 60.0),
        egui::TextEdit::multiline(&mut desc),
    );
    if resp.changed() {
        doc.project_description = desc;
    }
}
