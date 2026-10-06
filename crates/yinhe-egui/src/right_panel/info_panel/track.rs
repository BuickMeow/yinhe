//! 音轨信息面板。
//!
//! 「音轨」选项卡：当前选中轨的名称 / 端口 / 通道 / 颜色。
//!
//! 图层 / 历史记录 / 属性概要已拆为独立顶层选项卡（见 [`super::layers`] /
//! [`super::history`] / [`super::summary`]），由右栏多栏布局统一停靠。

use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::ICON_FORMAT_COLOR_RESET;

use yinhe_editor_core::document::Document;

use rust_i18n::t;

/// 固定标签列 + 只读值（`BODY_FONT` + `text_bright`）。
fn field_value(ui: &mut egui::Ui, label: &str, value: impl Into<String>) {
    crate::widgets::rows::form_row(ui, label, |ui| {
        ui.label(
            egui::RichText::new(value.into())
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::BODY_FONT,
                ))
                .color(crate::theme::text_bright()),
        );
    });
}

/// 显示音轨属性（「音轨」选项卡）。返回 `true` 表示端口/通道改变。
pub(crate) fn show_track_info(ui: &mut egui::Ui, doc: &mut Document) -> bool {
    let num_tracks = doc.data.model.tracks.len();
    if num_tracks == 0 {
        crate::widgets::hint::empty_hint(ui, t!("track.no_tracks").as_ref());
        return false;
    }

    let track_idx = doc
        .edit
        .track_selected
        .iter()
        .next()
        .copied()
        .map(|i| (i as usize).min(num_tracks - 1));
    if let Some(track_idx) = track_idx {
        show_track_fields(ui, doc, track_idx)
    } else {
        crate::widgets::hint::empty_hint(ui, t!("panel.select_track_hint").as_ref());
        false
    }
}

/// 音轨字段（名称 / 端口 / 通道 / 颜色）。返回端口/通道是否改变。
fn show_track_fields(ui: &mut egui::Ui, doc: &mut Document, track_idx: usize) -> bool {
    let num_tracks = doc.data.model.tracks.len();
    let track_idx = track_idx.min(num_tracks - 1);

    // ── Conductor 轨 ──
    if Some(track_idx as u16) == doc.edit.track_cache.conductor_idx {
        crate::widgets::rows::section_header(ui, t!("track.conductor").as_ref());
        if !doc.data.model.meta.name.is_empty() {
            field_value(
                ui,
                t!("track.song_title").as_ref(),
                doc.data.model.meta.name.clone(),
            );
        }
        field_value(
            ui,
            t!("track.tempo_count").as_ref(),
            format!("{}", doc.data.model.conductor.tempo.events.len()),
        );
        field_value(
            ui,
            t!("track.timesig_count").as_ref(),
            format!("{}", doc.data.model.conductor.time_sig.len()),
        );
        return false;
    }

    // ── 名称 ──
    let mut name_change: Option<String> = None;
    let mut name_resp_id: Option<egui::Id> = None;
    let mut name_gained_focus = false;
    let mut name_lost_focus = false;
    crate::widgets::rows::panel_row(ui, t!("track.name").as_ref(), |ui| {
        let mut name = doc.data.model.tracks[track_idx].name.clone();
        let resp = crate::widgets::text_input::control_text_input(
            ui,
            &mut name,
            ui.available_width().max(60.0),
            ("track_name", track_idx),
            None,
        );
        if resp.changed() {
            name_change = Some(name);
        }
        name_resp_id = Some(resp.id);
        name_gained_focus = resp.gained_focus();
        name_lost_focus = resp.lost_focus();
    });
    if let Some(id) = name_resp_id {
        if name_gained_focus {
            yinhe_editor_core::history::begin_edit(
                &mut doc.edit.pending_edits,
                id.value(),
                &doc.data.model.tracks[track_idx].name,
            );
        }
        if let Some(new_name) = name_change {
            if let Some(td) = Arc::make_mut(&mut doc.data.model).tracks.get_mut(track_idx) {
                Arc::make_mut(td).name = new_name.clone();
            }
            if let Some(ti_mut) = doc.edit.track_cache.info.get_mut(track_idx) {
                ti_mut.name = new_name;
            }
        }
        if name_lost_focus {
            let name = doc.data.model.tracks[track_idx].name.clone();
            yinhe_editor_core::history::commit_track_name(doc, id.value(), track_idx, &name);
        }
    }
    let ti = doc.edit.track_cache.info[track_idx].clone();

    ui.add_space(4.0);

    // ── 端口 / 通道 ──
    let mut port_changed = false;
    let mut new_port = ti.port;
    let mut new_ch = ti.channel;
    crate::widgets::rows::panel_row(ui, t!("track.port_channel").as_ref(), |ui| {
        // right_to_left：先放右侧的通道，再放左侧的端口。
        let ch_options: Vec<(usize, String)> =
            (0..16).map(|c| (c, format!("{:02}", c + 1))).collect();
        let mut ch_sel = ti.channel as usize;
        if crate::widgets::combo::combo_select(ui, "track_channel", &mut ch_sel, 60.0, &ch_options)
        {
            new_ch = ch_sel as u8;
            port_changed = true;
        }
        ui.add_space(4.0);
        let port_options: Vec<(usize, String)> = (0..16)
            .map(|p| (p, format!("Port {}", (b'A' + p as u8) as char)))
            .collect();
        let mut port_sel = ti.port as usize;
        if crate::widgets::combo::combo_select(
            ui,
            "track_port",
            &mut port_sel,
            120.0,
            &port_options,
        ) {
            new_port = port_sel as u8;
            port_changed = true;
        }
    });
    if port_changed {
        {
            let model = Arc::make_mut(&mut doc.data.model);
            if track_idx < model.tracks.len() {
                let td = Arc::make_mut(&mut model.tracks[track_idx]);
                td.port = new_port;
                td.channel = new_ch;
            }
        }
        doc.data.rebuild_model();
        doc.edit.track_cache.rebuild_info(&doc.data);
        doc.edit.track_cache.rebuild_pc_map(&doc.data);
        doc.data.bump_revision();
    }

    ui.add_space(4.0);

    // ── 颜色（行高一致的小色块 + 重置）──
    show_color_row(ui, doc, track_idx);

    port_changed
}

/// 颜色行：色块（点击直接弹出取色器）+ 重置按钮。
fn show_color_row(ui: &mut egui::Ui, doc: &mut Document, track_idx: usize) {
    let mut undo_color: Option<([f32; 4], [f32; 4])> = None;
    let edit_id = ui.id().with("track_color_edit");
    let was_editing = ui.data(|d| d.get_temp::<bool>(edit_id)).unwrap_or(false);

    crate::widgets::rows::panel_row(ui, t!("track.color").as_ref(), |ui| {
        let cur = doc
            .edit
            .track_cache
            .colors
            .get(track_idx)
            .copied()
            .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
        let stored_color = doc.data.model.tracks[track_idx].color;
        // right_to_left：先放右侧的重置按钮，再放左侧的色块。
        let reset_btn = crate::widgets::flat::flat_button_filled_auto(
            ui,
            crate::widgets::icon_text::icon_text(
                ICON_FORMAT_COLOR_RESET,
                t!("track.reset_color").as_ref(),
                12.0,
                crate::theme::text_label(),
            ),
            stored_color != yinhe_core::DEFAULT_TRACK_COLOR,
        );
        ui.add_space(crate::theme::GAP_SM);
        let mut srgba = crate::theme::rgba_to_color32((cur[0], cur[1], cur[2], cur[3]));
        // 色块即取色器按钮：点击一次直接展开调色板。
        let resp = crate::widgets::color_picker::color_edit_button(
            ui,
            &mut srgba,
            egui::vec2(
                36.0,
                crate::scaling::scaled_font(ui.ctx(), crate::theme::ROW_H_COMPACT),
            ),
        );
        let changed = resp.changed();

        let editing = changed;
        if editing && !was_editing {
            ui.data_mut(|d| d.insert_temp(edit_id.with("old"), cur));
        }
        if changed {
            let new = [
                srgba.r() as f32 / 255.0,
                srgba.g() as f32 / 255.0,
                srgba.b() as f32 / 255.0,
                srgba.a() as f32 / 255.0,
            ];
            {
                let model = Arc::make_mut(&mut doc.data.model);
                if track_idx < model.tracks.len() {
                    let td = Arc::make_mut(&mut model.tracks[track_idx]);
                    td.color = new;
                }
            }
            if let Some(c) = doc.edit.track_cache.colors.get_mut(track_idx) {
                *c = new;
            }
            doc.data.bump_revision();
        }
        if reset_btn.clicked() {
            let old = cur;
            {
                let model = Arc::make_mut(&mut doc.data.model);
                if track_idx < model.tracks.len() {
                    let td = Arc::make_mut(&mut model.tracks[track_idx]);
                    td.color = yinhe_core::DEFAULT_TRACK_COLOR;
                }
            }
            if let Some(c) = doc.edit.track_cache.colors.get_mut(track_idx) {
                *c = yinhe_editor_core::document::track_color(
                    &doc.data.model.tracks[track_idx],
                    track_idx,
                    doc.edit.track_cache.conductor_idx,
                );
            }
            doc.data.bump_revision();
            undo_color = Some((old, yinhe_core::DEFAULT_TRACK_COLOR));
        }
        if !editing && was_editing {
            let old = ui
                .data(|d| d.get_temp::<[f32; 4]>(edit_id.with("old")))
                .unwrap_or(cur);
            let new = doc
                .edit
                .track_cache
                .colors
                .get(track_idx)
                .copied()
                .unwrap_or(cur);
            if old != new {
                undo_color = Some((old, new));
            }
        }
        ui.data_mut(|d| d.insert_temp(edit_id, editing));
    });

    if let Some((old, new)) = undo_color {
        let snapshot = doc.capture_snapshot();
        doc.push_undo(
            yinhe_editor_core::history::UndoAction::TrackColor {
                track_idx,
                old,
                new,
            },
            "Edit track color",
            snapshot,
        );
    }
}

/// 计算每轨 skip mask 并发给音频引擎。
pub(crate) fn send_skip_tracks(doc: &Document, audio: Option<&yinhe_audio::CpalAudioHandle>) {
    let skip = doc.compute_skip_mask();
    if let Some(audio) = audio {
        audio.handle.set_skip_tracks(skip);
    }
}
