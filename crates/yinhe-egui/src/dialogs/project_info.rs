use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{
    begin_edit, commit_artist, commit_compression_level, commit_description, commit_ppq,
    commit_project_name,
};

use crate::dialogs::settings::setting_row;

/// 工程设置草稿：编辑期缓存，点「保存」才写回 doc（支持「取消」）。
#[derive(Clone)]
pub struct ProjectSettingsDraft {
    pub name: String,
    pub artist: String,
    pub ppq: i32,
    pub compression_level: i32,
    pub description: String,
}

impl ProjectSettingsDraft {
    pub fn from_doc(doc: &Document) -> Self {
        let meta = &doc.data.model.meta;
        Self {
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            ppq: meta.ppq as i32,
            compression_level: meta.compression_level,
            description: meta.description.clone(),
        }
    }
}

/// 工程设置内容（编辑 [`ProjectSettingsDraft`]，不直接改 doc）。
///
/// 排版对齐设置窗口：标题 + 描述在左、控件靠右、行间分割线。
/// 顶部不加 heading（窗口标题栏已有「工程设置」）。
pub fn show(ui: &mut egui::Ui, draft: &mut ProjectSettingsDraft) {
    ui.add_space(8.0);

    // ── Project name ──
    setting_row(
        ui,
        t!("project.name").as_ref(),
        t!("project.name_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input(
                ui,
                &mut draft.name,
                220.0,
                "proj_name",
                None,
            );
        },
    );

    // ── Artist ──
    setting_row(
        ui,
        t!("project.artist").as_ref(),
        t!("project.artist_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input(
                ui,
                &mut draft.artist,
                220.0,
                "proj_artist",
                None,
            );
        },
    );

    // ── PPQ ──
    setting_row(
        ui,
        t!("project.ppq").as_ref(),
        t!("project.ppq_desc").as_ref(),
        |ui| {
            crate::widgets::stepper::stepper(&mut draft.ppq)
                .range(1..=32767)
                .step(1.0)
                .width(110.0)
                .show(ui);
        },
    );

    // ── zstd compression level ──
    setting_row(
        ui,
        t!("project.compression").as_ref(),
        t!("project.compression_desc").as_ref(),
        |ui| {
            crate::widgets::stepper::stepper(&mut draft.compression_level)
                .range(0..=22)
                .step(1.0)
                .width(110.0)
                .show(ui);
        },
    );

    // ── Description ──
    setting_row(
        ui,
        t!("project.description").as_ref(),
        t!("project.description_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input_multiline(
                ui,
                &mut draft.description,
                220.0,
                60.0,
                "proj_desc",
            );
        },
    );
}

/// 把草稿写回 doc（点「保存」时调用）：
/// - name/artist/description/compression 直接提交；
/// - PPQ 变化时：有音符走异步 rescale（保持绝对时间），无音符直接提交。
pub(crate) fn commit_draft(
    ctx: &egui::Context,
    doc: &mut Document,
    draft: &ProjectSettingsDraft,
    save_id: u64,
) {
    use std::sync::Arc;

    let has_notes = doc.data.model.note_count > 0;

    // ── Project name（同步写 track 0 name，SMF 语义）──
    if draft.name != doc.data.model.meta.name {
        let old = doc.data.model.meta.name.clone();
        begin_edit(&mut doc.edit.pending_edits, save_id, &old);
        commit_project_name(doc, save_id, &draft.name);
        let new_name = draft.name.clone();
        let model = Arc::make_mut(&mut doc.data.model);
        model.meta.name = new_name.clone();
        if let Some(track) = model.tracks.get_mut(0) {
            Arc::make_mut(track).name = new_name;
        }
    }

    // ── Artist ──
    if draft.artist != doc.data.model.meta.artist {
        let old = doc.data.model.meta.artist.clone();
        begin_edit(&mut doc.edit.pending_edits, save_id, &old);
        commit_artist(doc, save_id, &draft.artist);
        Arc::make_mut(&mut doc.data.model).meta.artist = draft.artist.clone();
    }

    // ── Description ──
    if draft.description != doc.data.model.meta.description {
        let old = doc.data.model.meta.description.clone();
        begin_edit(&mut doc.edit.pending_edits, save_id, &old);
        commit_description(doc, save_id, &draft.description);
        Arc::make_mut(&mut doc.data.model).meta.description = draft.description.clone();
    }

    // ── zstd compression level ──
    if draft.compression_level != doc.data.model.meta.compression_level {
        let old = doc.data.model.meta.compression_level;
        begin_edit(&mut doc.edit.pending_edits, save_id, &old.to_string());
        commit_compression_level(doc, save_id, draft.compression_level);
        Arc::make_mut(&mut doc.data.model).meta.compression_level = draft.compression_level;
    }

    // ── PPQ ──
    let new_ppq = draft.ppq.max(1) as u32;
    if new_ppq != doc.data.model.meta.ppq {
        let old = doc.data.model.meta.ppq;
        begin_edit(&mut doc.edit.pending_edits, save_id, &old.to_string());
        if has_notes {
            // 异步 rescale：meta.ppq 保持 old 作为子线程基准，
            // 完成后 poll 里 commit_ppq(rescale=true)。
            ctx.data_mut(|d| {
                d.insert_temp(
                    egui::Id::new(crate::app::rescale_state::RESCALE_REQUEST_ID),
                    crate::app::rescale_state::RescaleRequest {
                        old_ppq: old,
                        new_ppq,
                        dragvalue_id: save_id,
                    },
                )
            });
        } else {
            let model = std::sync::Arc::make_mut(&mut doc.data.model);
            model.meta.ppq = new_ppq;
            model.rebuild_tempo_map();
            commit_ppq(doc, save_id, new_ppq, false);
        }
    }
}
