use eframe::egui;

use rust_i18n::t;
use yinhe_core::SongInfo;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{begin_edit, commit_compression_level, commit_ppq};

use crate::dialogs::settings::setting_row;

/// 工程设置草稿：编辑期缓存，点「保存」才写回 doc（支持「取消」）。
#[derive(Clone)]
pub struct ProjectSettingsDraft {
    pub info: SongInfo,
    pub ppq: i32,
    pub compression_level: i32,
}

impl ProjectSettingsDraft {
    pub fn from_doc(doc: &Document) -> Self {
        let meta = &doc.data.model.meta;
        Self {
            info: SongInfo::from_meta(meta),
            ppq: meta.ppq as i32,
            compression_level: meta.compression_level,
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
                &mut draft.info.name,
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
                &mut draft.info.artist,
                220.0,
                "proj_artist",
                None,
            );
        },
    );

    // ── Album / album artist / genre / year / track / disc / copyright ──
    setting_row(
        ui,
        t!("project.album").as_ref(),
        t!("project.album_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input(
                ui,
                &mut draft.info.album,
                220.0,
                "proj_album",
                None,
            );
        },
    );
    setting_row(
        ui,
        t!("project.album_artist").as_ref(),
        t!("project.album_artist_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input(
                ui,
                &mut draft.info.album_artist,
                220.0,
                "proj_album_artist",
                None,
            );
        },
    );
    setting_row(
        ui,
        t!("project.genre").as_ref(),
        t!("project.genre_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input(
                ui,
                &mut draft.info.genre,
                220.0,
                "proj_genre",
                None,
            );
        },
    );
    setting_row(
        ui,
        t!("project.year").as_ref(),
        t!("project.year_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input(
                ui,
                &mut draft.info.year,
                160.0,
                "proj_year",
                None,
            );
        },
    );
    setting_row(
        ui,
        t!("project.track_no").as_ref(),
        t!("project.track_no_desc").as_ref(),
        |ui| {
            let mut v = i32::from(draft.info.track_no);
            crate::widgets::stepper::stepper(&mut v)
                .range(0..=999)
                .step(1.0)
                .width(110.0)
                .show(ui);
            draft.info.track_no = v.max(0) as u16;
        },
    );
    setting_row(
        ui,
        t!("project.disc_no").as_ref(),
        t!("project.disc_no_desc").as_ref(),
        |ui| {
            let mut v = i32::from(draft.info.disc_no);
            crate::widgets::stepper::stepper(&mut v)
                .range(0..=99)
                .step(1.0)
                .width(110.0)
                .show(ui);
            draft.info.disc_no = v.max(0) as u16;
        },
    );
    setting_row(
        ui,
        t!("project.copyright").as_ref(),
        t!("project.copyright_desc").as_ref(),
        |ui| {
            crate::widgets::text_input::control_text_input(
                ui,
                &mut draft.info.copyright,
                220.0,
                "proj_copyright",
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
                &mut draft.info.description,
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

    // ── 歌曲元数据：整组比较，差异合并为一条 undo ──
    let old = SongInfo::from_meta(&doc.data.model.meta);
    if old != draft.info {
        let before = doc.capture_snapshot();
        draft
            .info
            .apply_to(&mut Arc::make_mut(&mut doc.data.model).meta);
        doc.push_undo(
            yinhe_editor_core::history::UndoAction::SongInfo {
                old: Box::new(old),
                new: Box::new(draft.info.clone()),
            },
            "Edit song info",
            before,
        );
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
        if doc.data.model.note_count > 0 {
            // 异步 rescale：meta.ppq 保持 old 作为子线程基准，
            // 完成后 poll 里 commit_ppq(rescale=true)。
            ctx.data_mut(|d| {
                d.insert_temp(
                    egui::Id::unique(crate::app::rescale_state::RESCALE_REQUEST_ID),
                    crate::app::rescale_state::RescaleRequest {
                        old_ppq: old,
                        new_ppq,
                        dragvalue_id: save_id,
                    },
                )
            });
        } else {
            let model = Arc::make_mut(&mut doc.data.model);
            model.meta.ppq = new_ppq;
            model.rebuild_tempo_map();
            commit_ppq(doc, save_id, new_ppq, false);
        }
    }
}
