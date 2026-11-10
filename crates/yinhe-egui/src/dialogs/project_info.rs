use eframe::egui;

use rust_i18n::t;
use yinhe_core::{CoverArt, SongInfo};
use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{begin_edit, commit_compression_level, commit_ppq};

use crate::dialogs::settings::setting_row;

/// 单个艺术家的 egui 编辑形态。
#[derive(Clone, Default)]
pub struct ArtistDraft {
    pub id: String,
    pub name: String,
    pub role: String,
    /// 每行一条网站。
    pub websites_text: String,
    pub bio: String,
}

impl ArtistDraft {
    fn from_ref(a: &yinhe_core::ArtistRef) -> Self {
        Self {
            id: a.id.clone(),
            name: a.name.clone(),
            role: a.role.clone(),
            websites_text: a.websites.join("\n"),
            bio: a.bio.clone(),
        }
    }

    fn to_ref(&self) -> yinhe_core::ArtistRef {
        yinhe_core::ArtistRef {
            id: self.id.clone(),
            name: self.name.trim().to_string(),
            role: self.role.trim().to_string(),
            websites: self
                .websites_text
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            bio: self.bio.trim().to_string(),
        }
    }
}

/// 工程设置草稿：编辑期缓存，点「保存」才写回 doc（支持「取消」）。
#[derive(Clone)]
pub struct ProjectSettingsDraft {
    pub info: SongInfo,
    /// 动态艺术家列表；第一项为主。
    pub artists: Vec<ArtistDraft>,
    pub ppq: i32,
    pub compression_level: i32,
    /// 当前累计前台工作时间（秒），只读展示。
    pub work_time_secs: u64,
    /// 最终封面（None = 未设置）。
    pub cover: Option<CoverArt>,
    /// 选图失败的临时提示（对话框内显示）。
    pub cover_error: Option<String>,
}

impl ProjectSettingsDraft {
    pub fn from_doc(doc: &Document) -> Self {
        let meta = &doc.data.model.meta;
        let info = SongInfo::from_meta(meta);
        let artists = info.artists.iter().map(ArtistDraft::from_ref).collect();
        Self {
            info,
            artists,
            ppq: meta.ppq as i32,
            compression_level: meta.compression_level,
            work_time_secs: doc.data.work_time_secs,
            cover: doc.data.cover.clone(),
            cover_error: None,
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

    // ── Artists（动态列表，第一项为主）──
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("project.artists").as_ref())
                .strong()
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SUB_TITLE_FONT,
                )),
        );
        if ui
            .small_button(format!("+ {}", t!("project.artist").as_ref()))
            .clicked()
        {
            draft.artists.push(ArtistDraft::default());
        }
    });
    let mut remove_artist: Option<usize> = None;
    for i in 0..draft.artists.len() {
        let label = format!("{} {}", t!("project.artist").as_ref(), i + 1);
        setting_row(ui, &label, t!("project.artist_desc").as_ref(), |ui| {
            ui.horizontal(|ui| {
                crate::widgets::text_input::control_text_input(
                    ui,
                    &mut draft.artists[i].name,
                    190.0,
                    ("proj_artist_name", i),
                    None,
                );
                if ui.small_button("−").clicked() {
                    remove_artist = Some(i);
                }
            });
        });
        setting_row(
            ui,
            t!("project.artist_websites").as_ref(),
            t!("project.artist_websites_desc").as_ref(),
            |ui| {
                crate::widgets::text_input::control_text_input_multiline(
                    ui,
                    &mut draft.artists[i].websites_text,
                    220.0,
                    44.0,
                    ("proj_artist_websites", i),
                );
            },
        );
        setting_row(
            ui,
            t!("project.artist_bio").as_ref(),
            t!("project.artist_bio_desc").as_ref(),
            |ui| {
                crate::widgets::text_input::control_text_input_multiline(
                    ui,
                    &mut draft.artists[i].bio,
                    220.0,
                    52.0,
                    ("proj_artist_bio", i),
                );
            },
        );
    }
    if let Some(i) = remove_artist {
        draft.artists.remove(i);
    }

    // ── PPQ（第三个主要字段）──
    setting_row(
        ui,
        t!("project.ppq").as_ref(),
        t!("project.ppq_desc").as_ref(),
        |ui| {
            crate::widgets::stepper::stepper(&mut draft.ppq)
                .range(1..=65535)
                .step(1.0)
                .width(110.0)
                .show(ui);
        },
    );

    // ── 工作时间（只读）──
    setting_row(
        ui,
        t!("project.work_time").as_ref(),
        t!("project.work_time_desc").as_ref(),
        |ui| {
            let secs = draft.work_time_secs;
            ui.label(format!(
                "{:02}:{:02}:{:02}",
                secs / 3600,
                (secs % 3600) / 60,
                secs % 60
            ));
        },
    );

    // ── Album / album artist / genre / year / track / disc ──
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
    // ── Cover ──
    setting_row(
        ui,
        t!("project.cover").as_ref(),
        t!("project.cover_desc").as_ref(),
        |ui| {
            ui.horizontal(|ui| {
                if ui.button(t!("project.cover_pick")).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Image", &["jpg", "jpeg", "png", "webp"])
                        .pick_file()
                {
                    match yinhe_editor_core::cover::load_cover(&path) {
                        Ok(c) => {
                            draft.cover = Some(c);
                            draft.cover_error = None;
                        }
                        Err(e) => draft.cover_error = Some(e),
                    }
                }
                if draft.cover.is_some() && ui.button(t!("project.cover_clear")).clicked() {
                    draft.cover = None;
                    draft.cover_error = None;
                }
                match &draft.cover {
                    Some(c) => {
                        ui.weak(format!("{} KB", c.data.len().div_ceil(1024)));
                    }
                    None => {
                        ui.weak(t!("project.cover_none"));
                    }
                }
            });
            if let Some(e) = &draft.cover_error {
                ui.colored_label(crate::theme::danger(), e);
            }
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
    let mut info = draft.info.clone();
    info.artists = draft
        .artists
        .iter()
        .filter(|a| !a.name.trim().is_empty())
        .map(ArtistDraft::to_ref)
        .collect();
    info.artist = info
        .artists
        .first()
        .map(|a| a.name.clone())
        .unwrap_or_default();
    let old = SongInfo::from_meta(&doc.data.model.meta);
    if old != info {
        let before = doc.capture_snapshot();
        info.apply_to(&mut Arc::make_mut(&mut doc.data.model).meta);
        doc.push_undo(
            yinhe_editor_core::history::UndoAction::SongInfo {
                old: Box::new(old),
                new: Box::new(info),
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

    // ── Cover ──
    if draft.cover != doc.data.cover {
        let before = doc.capture_snapshot();
        let old = doc.data.cover.clone();
        doc.data.cover = draft.cover.clone();
        doc.data.bump_revision();
        doc.push_undo(
            yinhe_editor_core::history::UndoAction::Cover {
                old,
                new: draft.cover.clone(),
            },
            "Edit cover",
            before,
        );
    }
}
