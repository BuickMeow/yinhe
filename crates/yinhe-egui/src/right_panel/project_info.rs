use eframe::egui;

use crate::document::Document;
use crate::history::{begin_edit, commit_edit};

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
    let mut name = doc.data.project_name.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut name).id_salt("proj_name"),
    );
    if resp.gained_focus() {
        begin_edit(&doc.data, &mut doc.edit.pending_edits, resp.id, "Edit project name");
    }
    if resp.changed() {
        doc.data.project_name = name;
    }
    if resp.lost_focus() {
        commit_edit(&doc.data, &mut doc.history, &mut doc.edit.pending_edits, resp.id);
    }

    ui.add_space(6.0);

    // ── Artist ──
    ui.label(
        egui::RichText::new("艺术家")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut artist = doc.data.project_artist.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut artist).id_salt("proj_artist"),
    );
    if resp.gained_focus() {
        begin_edit(&doc.data, &mut doc.edit.pending_edits, resp.id, "Edit artist");
    }
    if resp.changed() {
        doc.data.project_artist = artist;
    }
    if resp.lost_focus() {
        commit_edit(&doc.data, &mut doc.history, &mut doc.edit.pending_edits, resp.id);
    }

    ui.add_space(6.0);

    // ── PPQ ──
    ui.label(
        egui::RichText::new("PPQ (每拍节拍数)")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut ppq = doc.data.project_ppq as i32;
    let resp = ui.add_sized(
        egui::vec2(80.0, 20.0),
        egui::DragValue::new(&mut ppq).range(1..=32767),
    );
    if resp.gained_focus() || (resp.drag_started() && !doc.edit.pending_edits.has(resp.id)) {
        begin_edit(&doc.data, &mut doc.edit.pending_edits, resp.id, "Edit PPQ");
    }
    if resp.changed() {
        doc.data.project_ppq = ppq.max(1) as u32;
    }
    if resp.lost_focus() || resp.drag_stopped() {
        commit_edit(&doc.data, &mut doc.history, &mut doc.edit.pending_edits, resp.id);
    }

    ui.add_space(6.0);

    // ── zstd compression level ──
    ui.label(
        egui::RichText::new("压缩级别 (zstd)")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut zstd_level = doc.data.compression_level as i32;
    let resp = ui.add_sized(
        egui::vec2(60.0, 20.0),
        egui::DragValue::new(&mut zstd_level).range(0..=22),
    );
    if resp.gained_focus() || (resp.drag_started() && !doc.edit.pending_edits.has(resp.id)) {
        begin_edit(&doc.data, &mut doc.edit.pending_edits, resp.id, "Edit zstd level");
    }
    if resp.changed() {
        doc.data.compression_level = zstd_level;
    }
    if resp.lost_focus() || resp.drag_stopped() {
        commit_edit(&doc.data, &mut doc.history, &mut doc.edit.pending_edits, resp.id);
    }

    ui.add_space(6.0);

    // ── Description ──
    ui.label(
        egui::RichText::new("简介")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut desc = doc.data.project_description.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 60.0),
        egui::TextEdit::multiline(&mut desc).id_salt("proj_desc"),
    );
    if resp.gained_focus() {
        begin_edit(&doc.data, &mut doc.edit.pending_edits, resp.id, "Edit description");
    }
    if resp.changed() {
        doc.data.project_description = desc;
    }
    if resp.lost_focus() {
        commit_edit(&doc.data, &mut doc.history, &mut doc.edit.pending_edits, resp.id);
    }
}
