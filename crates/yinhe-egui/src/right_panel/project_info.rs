use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{begin_edit, commit_project_name, commit_artist, commit_description, commit_ppq, commit_compression_level};

/// 弹框确认 ID：PPQ 修改后是否 rescale 音符。
const PPQ_RESCALE_DIALOG_ID: &str = "ppq_rescale_dialog";
/// 暂存待确认的 PPQ 修改（old, new, dragvalue_id）。
const PPQ_RESCALE_PENDING_ID: &str = "ppq_rescale_pending";

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
    let mut name = doc.data.model.meta.name.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut name).id_salt("proj_name"),
    );
    if resp.gained_focus() {
        begin_edit(&mut doc.edit.pending_edits, resp.id.value(), &doc.data.model.meta.name);
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.name = name;
    }
    if resp.lost_focus() {
        commit_project_name(
            &mut doc.history,
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.name,
            doc.edit.selected.clone(),
            doc.edit.track_selected.clone(),
            doc.edit.sel_rect.clone(),
        );
    }

    ui.add_space(6.0);

    // ── Artist ──
    ui.label(
        egui::RichText::new("艺术家")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut artist = doc.data.model.meta.artist.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 20.0),
        egui::TextEdit::singleline(&mut artist).id_salt("proj_artist"),
    );
    if resp.gained_focus() {
        begin_edit(&mut doc.edit.pending_edits, resp.id.value(), &doc.data.model.meta.artist);
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.artist = artist;
    }
    if resp.lost_focus() {
        commit_artist(
            &mut doc.history,
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.artist,
            doc.edit.selected.clone(),
            doc.edit.track_selected.clone(),
            doc.edit.sel_rect.clone(),
        );
    }

    ui.add_space(6.0);

    // ── PPQ ──
    // 修改 PPQ 后，若 old != new 会弹框问用户是否 rescale 已有音符以保留时值。
    // - 选"是"：调用 model.rescale_ppq() 批量缩放所有 tick，undo 用反向 rescale。
    // - 选"否"：只改 meta.ppq + rebuild_tempo_map，音符 tick 不变。
    ui.label(
        egui::RichText::new("PPQ (每拍节拍数)")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut ppq = doc.data.model.meta.ppq as i32;
    let resp = ui.add_sized(
        egui::vec2(80.0, 20.0),
        egui::DragValue::new(&mut ppq).range(1..=32767),
    );
    if resp.gained_focus() || (resp.drag_started() && !doc.edit.pending_edits.has(resp.id.value())) {
        begin_edit(&mut doc.edit.pending_edits, resp.id.value(), &doc.data.model.meta.ppq.to_string());
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.ppq = ppq.max(1) as u32;
    }
    if resp.lost_focus() || resp.drag_stopped() {
        // 取出 pending old 值判断是否需要弹框
        let old_str = doc.edit.pending_edits.get(resp.id.value()).map(|s| s.to_string());
        let new_val = doc.data.model.meta.ppq;
        let old_val: u32 = old_str.as_deref().and_then(|s| s.parse().ok()).unwrap_or(new_val);
        let has_notes = doc.data.model.note_count > 0;
        if old_val != new_val && has_notes {
            // 暂存待确认信息，弹框询问是否 rescale
            ui.ctx().data_mut(|d| d.insert_temp(
                egui::Id::new(PPQ_RESCALE_PENDING_ID),
                (old_val, new_val, resp.id.value()),
            ));
            // 不立即 commit，等弹框确认后再 commit
        } else {
            // 无音符或未变化：直接 commit（rescale=false）
            commit_ppq(
                &mut doc.history,
                &mut doc.edit.pending_edits,
                resp.id.value(),
                new_val,
                false,
                doc.edit.selected.clone(),
                doc.edit.track_selected.clone(),
                doc.edit.sel_rect.clone(),
            );
        }
    }

    ui.add_space(6.0);

    // ── zstd compression level ──
    ui.label(
        egui::RichText::new("压缩级别 (zstd)")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut zstd_level = doc.data.model.meta.compression_level as i32;
    let resp = ui.add_sized(
        egui::vec2(60.0, 20.0),
        egui::DragValue::new(&mut zstd_level).range(0..=22),
    );
    if resp.gained_focus() || (resp.drag_started() && !doc.edit.pending_edits.has(resp.id.value())) {
        begin_edit(&mut doc.edit.pending_edits, resp.id.value(), &doc.data.model.meta.compression_level.to_string());
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.compression_level = zstd_level;
    }
    if resp.lost_focus() || resp.drag_stopped() {
        commit_compression_level(
            &mut doc.history,
            &mut doc.edit.pending_edits,
            resp.id.value(),
            doc.data.model.meta.compression_level,
            doc.edit.selected.clone(),
            doc.edit.track_selected.clone(),
            doc.edit.sel_rect.clone(),
        );
    }

    ui.add_space(6.0);

    // ── Description ──
    ui.label(
        egui::RichText::new("简介")
            .color(egui::Color32::from_gray(160))
            .size(11.0),
    );
    let mut desc = doc.data.model.meta.description.clone();
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 60.0),
        egui::TextEdit::multiline(&mut desc).id_salt("proj_desc"),
    );
    if resp.gained_focus() {
        begin_edit(&mut doc.edit.pending_edits, resp.id.value(), &doc.data.model.meta.description);
    }
    if resp.changed() {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        model.meta.description = desc;
    }
    if resp.lost_focus() {
        commit_description(
            &mut doc.history,
            &mut doc.edit.pending_edits,
            resp.id.value(),
            &doc.data.model.meta.description,
            doc.edit.selected.clone(),
            doc.edit.track_selected.clone(),
            doc.edit.sel_rect.clone(),
        );
    }

    // ── PPQ rescale 确认弹框 ──
    let pending: Option<(u32, u32, u64)> = ui
        .ctx()
        .data(|d| d.get_temp(egui::Id::new(PPQ_RESCALE_PENDING_ID)));
    if let Some((old_val, new_val, dragvalue_id)) = pending {
        let dialog_id = egui::Id::new(PPQ_RESCALE_DIALOG_ID);
        let mut should_close = false;
        let mut user_choice: Option<bool> = None; // Some(true)=rescale, Some(false)=不rescale

        egui::Window::new("PPQ 变更")
            .id(dialog_id)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.set_width(360.0);
                ui.add_space(6.0);
                ui.label(format!(
                    "PPQ 将从 {} 变为 {}。",
                    old_val, new_val
                ));
                ui.add_space(4.0);
                ui.label("是否同时缩放已有音符与自动化事件，以保留绝对时值？");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("• 是：所有 tick 按比例缩放（推荐）\n• 否：仅改 PPQ，音符位置不变（时值会改变）")
                        .color(egui::Color32::from_gray(140))
                        .size(11.0),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
                    if ui.button("是（缩放音符）").clicked() {
                        user_choice = Some(true);
                        should_close = true;
                    }
                    if ui.button("否（保持音符）").clicked() {
                        user_choice = Some(false);
                        should_close = true;
                    }
                    if ui.button("取消").clicked() {
                        // 取消：把 PPQ 还原为 old_val
                        let model = std::sync::Arc::make_mut(&mut doc.data.model);
                        model.meta.ppq = old_val;
                        // 清掉 pending edit（不推 undo）
                        doc.edit.pending_edits.take(dragvalue_id);
                        should_close = true;
                    }
                });
            });

        if should_close {
            if let Some(rescale) = user_choice {
                if rescale {
                    // 执行 rescale（已经在 DragValue.changed() 中改了 meta.ppq，
                    // 这里调用 rescale_ppq 会用当前 meta.ppq 作为 new_ppq）
                    let model = std::sync::Arc::make_mut(&mut doc.data.model);
                    // rescale_ppq 用 old_ppq = 当前 meta.ppq 之前的值？
                    // 不对：DragValue 已经把 meta.ppq 改成 new_val 了，rescale_ppq 会用 new_val 作为 old_ppq。
                    // 需要先把 meta.ppq 还原为 old_val，再调 rescale_ppq(new_val)。
                    model.meta.ppq = old_val;
                    model.rescale_ppq(new_val);
                    doc.data.bump_revision();
                } else {
                    // 不 rescale，但要 rebuild_tempo_map（meta.ppq 已是 new_val）
                    let model = std::sync::Arc::make_mut(&mut doc.data.model);
                    model.rebuild_tempo_map();
                }
                // commit undo（带 rescale 标志）
                commit_ppq(
                    &mut doc.history,
                    &mut doc.edit.pending_edits,
                    dragvalue_id,
                    new_val,
                    rescale,
                    doc.edit.selected.clone(),
                    doc.edit.track_selected.clone(),
                    doc.edit.sel_rect.clone(),
                );
            }
            ui.ctx().data_mut(|d| d.remove::<(u32, u32, u64)>(
                egui::Id::new(PPQ_RESCALE_PENDING_ID),
            ));
        }
    }
}
