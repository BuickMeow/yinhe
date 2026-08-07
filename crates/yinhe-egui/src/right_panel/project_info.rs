use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{
    begin_edit, commit_artist, commit_compression_level, commit_description, commit_ppq,
    commit_project_name,
};

/// ctx memory 中暂存待确认的 PPQ 修改（old, new, dragvalue_id）。
///
/// `project_info.rs` 在 DragValue 失焦且 old != new && has_notes 时写入，
/// `dialog_dispatch.rs` 每帧检测此 Id 并弹出独立 viewport 确认框。
/// 用户选择后 `dialog_dispatch.rs` 清除此 Id。
pub(crate) const PPQ_RESCALE_PENDING_ID: &str = "ppq_rescale_pending";

/// Show the Project Info panel for editing project metadata.
pub fn show(ui: &mut egui::Ui, doc: Option<&mut Document>) {
    let Some(doc) = doc else {
        crate::widgets::hint::empty_hint(ui, t!("common.no_document").as_ref());
        return;
    };

    ui.add_space(8.0);

    // ── Project name ──
    ui.label(
        egui::RichText::new(t!("project.name").as_ref())
            .color(crate::theme::TEXT_MUTED)
            .size(crate::theme::SMALL_FONT),
    );
    let mut name = doc.data.model.meta.name.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut name).id_salt("proj_name"),
    );
    if resp.gained_focus() {
        begin_edit(
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.name,
        );
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.name = name.clone();
        // SMF 标准：track 0 name = song title，同步更新
        if let Some(track) = model.tracks.get_mut(0) {
            std::sync::Arc::make_mut(track).name = name;
        }
    }
    if resp.lost_focus() {
        let name = doc.data.model.meta.name.clone();
        commit_project_name(doc, resp.id.value(), &name);
    }

    ui.add_space(6.0);

    // ── Artist ──
    ui.label(
        egui::RichText::new(t!("project.artist").as_ref())
            .color(crate::theme::TEXT_MUTED)
            .size(crate::theme::SMALL_FONT),
    );
    let mut artist = doc.data.model.meta.artist.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut artist).id_salt("proj_artist"),
    );
    if resp.gained_focus() {
        begin_edit(
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.artist,
        );
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.artist = artist;
    }
    if resp.lost_focus() {
        let artist = doc.data.model.meta.artist.clone();
        commit_artist(doc, resp.id.value(), &artist);
    }

    ui.add_space(6.0);

    // ── PPQ ──
    // 修改 PPQ 后，若有音符且 old != new，写入 ctx memory 让 dialog_dispatch
    // 弹出独立 viewport 确认框（标准 dialog 形式，不受面板开关影响）。
    // - 选"是"：异步 rescale（main_loop 检测 RESCALE_REQUEST_ID 启动子线程）。
    // - 选"否"：rebuild_tempo_map + commit_ppq(rescale=false)。
    // - 选"取消"：还原 meta.ppq = old。
    ui.label(
        egui::RichText::new(t!("project.ppq").as_ref())
            .color(crate::theme::TEXT_MUTED)
            .size(crate::theme::SMALL_FONT),
    );
    let mut ppq = doc.data.model.meta.ppq as i32;
    let resp = ui.add_sized(
        egui::vec2(80.0, 20.0),
        crate::widgets::numeric_input::decimal_drag_value(&mut ppq).range(1..=32767),
    );
    if resp.gained_focus() || (resp.drag_started() && !doc.edit.pending_edits.has(resp.id.value()))
    {
        begin_edit(
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.ppq.to_string(),
        );
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.ppq = ppq.max(1) as u32;
    }
    if resp.lost_focus() || resp.drag_stopped() {
        // 取出 pending old 值判断是否需要弹框
        let old_str = doc
            .edit
            .pending_edits
            .get(resp.id.value())
            .map(|s| s.to_string());
        let new_val = doc.data.model.meta.ppq;
        let old_val: u32 = old_str
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(new_val);
        let has_notes = doc.data.model.note_count > 0;
        if old_val != new_val && has_notes {
            // 暂存待确认信息，由 dialog_dispatch 弹出独立 viewport 确认框。
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new(PPQ_RESCALE_PENDING_ID),
                    (old_val, new_val, resp.id.value()),
                )
            });
            // 不立即 commit，等弹框确认后再 commit
        } else {
            // 无音符或未变化：直接 commit（rescale=false）
            commit_ppq(doc, resp.id.value(), new_val, false);
        }
    }

    ui.add_space(6.0);

    // ── zstd compression level ──
    ui.label(
        egui::RichText::new(t!("project.compression").as_ref())
            .color(crate::theme::TEXT_MUTED)
            .size(crate::theme::SMALL_FONT),
    );
    let mut zstd_level = doc.data.model.meta.compression_level;
    let resp = ui.add_sized(
        egui::vec2(60.0, 20.0),
        crate::widgets::numeric_input::decimal_drag_value(&mut zstd_level).range(0..=22),
    );
    if resp.gained_focus() || (resp.drag_started() && !doc.edit.pending_edits.has(resp.id.value()))
    {
        begin_edit(
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.compression_level.to_string(),
        );
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.compression_level = zstd_level;
    }
    if resp.lost_focus() || resp.drag_stopped() {
        commit_compression_level(doc, resp.id.value(), doc.data.model.meta.compression_level);
    }

    ui.add_space(6.0);

    // ── Description ──
    ui.label(
        egui::RichText::new(t!("project.description").as_ref())
            .color(crate::theme::TEXT_MUTED)
            .size(crate::theme::SMALL_FONT),
    );
    let mut desc = doc.data.model.meta.description.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 60.0),
        egui::TextEdit::multiline(&mut desc).id_salt("proj_desc"),
    );
    if resp.gained_focus() {
        begin_edit(
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.description,
        );
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.description = desc;
    }
    if resp.lost_focus() {
        let desc = doc.data.model.meta.description.clone();
        commit_description(doc, resp.id.value(), &desc);
    }
}
