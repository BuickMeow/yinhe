//! 音轨信息面板。
//!
//! 显示选中音轨的名称 / 端口 / 通道 / Mute / Solo / 属性摘要，
//! 以及 Conductor 轨和多多选汇总。

use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::{ICON_FORMAT_COLOR_RESET, ICON_HEADPHONES, ICON_VOLUME_OFF};

use yinhe_editor_core::document::Document;

use rust_i18n::t;

use super::InfoContent;

/// 显示音轨信息编辑器。返回 `true` 表示端口/通道改变（需重建音频引擎）。
/// `info_content` 在“无选中音轨”等分支被写为 None（侧栏会回落显示工程设置；
/// 浮窗调用方传局部占位即可，不接入全局选择状态）。
pub(crate) fn show_track_info(
    ui: &mut egui::Ui,
    doc: &mut Document,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    info_content: &mut Option<InfoContent>,
) -> bool {
    let num_tracks = doc.data.model.tracks.len();
    if num_tracks == 0 {
        crate::widgets::hint::empty_hint(ui, t!("track.no_tracks").as_ref());
        return false;
    }

    // ── Track selector ──
    // 轨道号 0-based：Conductor = 000（与 AR 面板一致）。
    let track_names: Vec<String> = doc
        .data
        .model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{:03} – {}", i, t.name))
        .collect();

    let sel_idx = doc
        .edit
        .track_selected
        .iter()
        .next()
        .copied()
        .map(|i| (i as usize).min(num_tracks - 1))
        .unwrap_or(0);

    egui::ComboBox::from_id_salt("info_track_sel")
        .selected_text(&track_names[sel_idx])
        .show_ui(ui, |ui| {
            for (i, tn) in track_names.iter().enumerate() {
                if ui.selectable_label(i == sel_idx, tn).clicked() {
                    doc.edit.track_selected.clear();
                    doc.edit.track_selected.insert(i as u16);
                }
            }
        });

    ui.add_space(6.0);

    let Some(&track_idx) = doc.edit.track_selected.iter().next() else {
        // 未选中音轨 → 回退到项目设置（由父级 None 分支处理）。
        *info_content = None;
        return false;
    };
    let track_idx = track_idx as usize;
    let track_idx = track_idx.min(num_tracks - 1);

    // ── Conductor track ──
    if Some(track_idx as u16) == doc.edit.conductor_track_idx {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t!("track.conductor").as_ref())
                .strong()
                .size(crate::theme::PANEL_TITLE_FONT)
                .color(crate::theme::text_primary()),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(t!("track.conductor_hint").as_ref())
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_label()),
        );
        ui.add_space(8.0);

        if !doc.data.model.meta.name.is_empty() {
            ui.horizontal(|ui| {
                ui.label(t!("track.song_title").as_ref());
                ui.label(
                    egui::RichText::new(&doc.data.model.meta.name)
                        .color(crate::theme::text_bright())
                        .size(crate::theme::SUB_TITLE_FONT),
                );
            });
            ui.add_space(2.0);
        }

        ui.horizontal(|ui| {
            ui.label(t!("track.tempo_count").as_ref());
            ui.label(
                egui::RichText::new(format!("{}", doc.data.model.conductor.tempo.events.len()))
                    .color(crate::theme::text_secondary())
                    .size(crate::theme::SUB_TITLE_FONT),
            );
        });
        ui.horizontal(|ui| {
            ui.label(t!("track.timesig_count").as_ref());
            ui.label(
                egui::RichText::new(format!("{}", doc.data.model.conductor.time_sig.len()))
                    .color(crate::theme::text_secondary())
                    .size(crate::theme::SUB_TITLE_FONT),
            );
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        if ui
            .add(egui::Button::new(
                egui::RichText::new(t!("common.clear_selection").as_ref())
                    .size(crate::theme::BODY_FONT),
            ))
            .clicked()
        {
            *info_content = None;
        }
        return false;
    }

    // ── Track name ──
    let mut name_change: Option<String> = None;
    let mut name_resp_id: Option<egui::Id> = None;
    let mut name_gained_focus = false;
    let mut name_lost_focus = false;
    ui.horizontal(|ui| {
        ui.label(t!("track.name").as_ref());
        let mut name = doc.data.model.tracks[track_idx].name.clone();
        let resp = ui.add_sized(
            egui::vec2(ui.available_width().max(60.0), 18.0),
            egui::TextEdit::singleline(&mut name).id_salt(("track_name", track_idx)),
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
            // 唯一权威源是 model.tracks[].name（保存时 sync_mapping_file 读它）；
            // track_info_cache 是显示缓存，同步更新（undo 由 apply.rs 反向同步）。
            if let Some(td) = Arc::make_mut(&mut doc.data.model).tracks.get_mut(track_idx) {
                Arc::make_mut(td).name = new_name.clone();
            }
            if let Some(ti_mut) = doc.edit.track_info_cache.get_mut(track_idx) {
                ti_mut.name = new_name;
            }
        }
        if name_lost_focus {
            let name = doc.data.model.tracks[track_idx].name.clone();
            yinhe_editor_core::history::commit_track_name(doc, id.value(), track_idx, &name);
        }
    }
    // 快照一份 track_info（避免借用与后续颜色 undo 的 &mut doc 冲突）。
    let ti = doc.edit.track_info_cache[track_idx].clone();

    ui.add_space(4.0);

    // ── Port / Channel ──
    let mut port_changed = false;
    let mut new_port = ti.port;
    let mut new_ch = ti.channel;

    ui.horizontal(|ui| {
        ui.label("端口/通道:");

        let port_options: Vec<String> = (0..16)
            .map(|p| format!("Port {}", (b'A' + p) as char))
            .collect();
        let _port_sel = egui::ComboBox::from_id_salt("track_port")
            .selected_text(format!("Port {}", (b'A' + ti.port) as char))
            .width(70.0)
            .show_ui(ui, |ui| {
                for (i, label) in port_options.iter().enumerate() {
                    if ui.selectable_label(i == ti.port as usize, label).clicked() {
                        new_port = i as u8;
                        port_changed = true;
                    }
                }
            });

        ui.add_space(4.0);

        let ch_options: Vec<String> = (0..16).map(|c| format!("{:02}", c + 1)).collect();
        let _ch_sel = egui::ComboBox::from_id_salt("track_channel")
            .selected_text(format!("{:02}", ti.channel + 1))
            .width(50.0)
            .show_ui(ui, |ui| {
                for (i, label) in ch_options.iter().enumerate() {
                    if ui
                        .selectable_label(i == ti.channel as usize, label)
                        .clicked()
                    {
                        new_ch = i as u8;
                        port_changed = true;
                    }
                }
            });
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
        doc.edit.track_info_cache = doc.data.track_info();
        doc.edit.pc_map_cache = doc.data.pc_map_cache();
        doc.data.bump_revision();
        return true;
    }

    ui.add_space(6.0);

    // ── 音轨颜色（ImageToMidi 颜色事件兼容）──
    // 显示当前实际颜色（缓存：显式颜色优先，否则调色板），
    // 编辑后写入 TrackData.color 并刷新缓存（无需重建音频引擎）。
    // undo：颜色编辑会话（滑块拖动/弹窗连续变化）开始记录旧色，
    // 会话结束（连续两帧无变化）时若有变化提交一条 undo。
    let mut undo_color: Option<([f32; 4], [f32; 4])> = None; // (old, new)
    let edit_id = ui.id().with("track_color_edit");
    let was_editing = ui.data(|d| d.get_temp::<bool>(edit_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.label("颜色:");
        let cur = doc
            .edit
            .track_colors_cache
            .get(track_idx)
            .copied()
            .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
        let mut srgba = crate::theme::rgba_to_color32((cur[0], cur[1], cur[2], cur[3]));
        let mut changed = false;
        changed |= crate::widgets::color_picker::color_edit_button(ui, &mut srgba).changed();
        // 重置为默认颜色：清除显式颜色事件（写入占位色），
        // 显示回落到调色板；轨道已是默认色时禁用。
        let stored_color = doc.data.model.tracks[track_idx].color;
        let reset_btn = ui.add_enabled(
            stored_color != yinhe_core::DEFAULT_TRACK_COLOR,
            egui::Button::new(crate::widgets::icon_text::icon_text(
                ICON_FORMAT_COLOR_RESET,
                t!("track.reset_color").as_ref(),
                12.0,
                crate::theme::text_label(),
            ))
            .min_size(egui::vec2(68.0, 22.0)),
        );
        let editing = changed;
        if editing && !was_editing {
            // 会话开始：记录编辑前颜色
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
            if let Some(c) = doc.edit.track_colors_cache.get_mut(track_idx) {
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
            if let Some(c) = doc.edit.track_colors_cache.get_mut(track_idx) {
                *c = yinhe_editor_core::document::track_color(
                    &doc.data.model.tracks[track_idx],
                    track_idx,
                    doc.edit.conductor_track_idx,
                );
            }
            doc.data.bump_revision();
            undo_color = Some((old, yinhe_core::DEFAULT_TRACK_COLOR));
        }
        if !editing && was_editing {
            // 会话结束：颜色有变则提交一条 undo
            let old = ui
                .data(|d| d.get_temp::<[f32; 4]>(edit_id.with("old")))
                .unwrap_or(cur);
            let new = doc
                .edit
                .track_colors_cache
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

    ui.add_space(6.0);

    // ── Mute / Solo ──
    while doc.edit.track_overrides.len() <= track_idx {
        doc.edit
            .track_overrides
            .push(yinhe_editor_core::document::TrackOverride::default());
    }

    let muted = doc.edit.track_overrides[track_idx].muted;
    let soloed = doc.edit.track_overrides[track_idx].soloed;

    let mut mute_clicked = false;
    let mut solo_clicked = false;

    ui.horizontal(|ui| {
        // 静音：始终显示 ICON_VOLUME_OFF + 文字，颜色区分激活状态
        // 图标走 material-icons 家族（否则 PUA 码点被 Pretendard/MiSans 抢占）
        let mute_color = if muted {
            crate::theme::mute_active()
        } else {
            crate::theme::text_label()
        };
        let r1 = ui.add(
            egui::Button::new(crate::widgets::icon_text::icon_text(
                ICON_VOLUME_OFF,
                t!("track.mute").as_ref(),
                12.0,
                mute_color,
            ))
            .min_size(egui::vec2(60.0, 22.0)),
        );

        ui.add_space(4.0);

        // 独奏：始终显示 ICON_HEADPHONES + 文字，颜色区分激活状态
        let solo_color = if soloed {
            crate::theme::solo_active()
        } else {
            crate::theme::text_label()
        };
        let r2 = ui.add(
            egui::Button::new(crate::widgets::icon_text::icon_text(
                ICON_HEADPHONES,
                t!("track.solo").as_ref(),
                12.0,
                solo_color,
            ))
            .min_size(egui::vec2(60.0, 22.0)),
        );

        mute_clicked = r1.clicked();
        solo_clicked = r2.clicked();
    });

    if mute_clicked || solo_clicked {
        if mute_clicked {
            doc.edit.track_overrides[track_idx].muted = !muted;
        }
        if solo_clicked {
            doc.edit.track_overrides[track_idx].soloed = !soloed;
        }
        send_skip_tracks(doc, audio);
    }

    ui.add_space(8.0);

    // ── 摘要 ──
    ui.separator();
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t!("track.properties").as_ref())
            .size(crate::theme::SMALL_FONT)
            .strong(),
    );
    ui.add_space(2.0);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("track.note_count").as_ref())
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_label()),
        );
        ui.label(egui::RichText::new(format!("{}", ti.note_count)).size(crate::theme::SMALL_FONT));
    });
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("track.event_count").as_ref())
                .size(crate::theme::SMALL_FONT)
                .color(crate::theme::text_label()),
        );
        ui.label(egui::RichText::new(format!("{}", ti.event_count)).size(crate::theme::SMALL_FONT));
    });

    // Program Change
    let global_ch = ti.port as u32 * 16 + (ti.channel as u32 - 1);
    if let Some(pc) = doc.edit.pc_map_cache.get(&(global_ch as u8)) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("track.program").as_ref())
                    .size(crate::theme::SMALL_FONT)
                    .color(crate::theme::text_label()),
            );
            ui.label(egui::RichText::new(format!("PC {}", pc)).size(crate::theme::SMALL_FONT));
        });
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    if ui
        .add(egui::Button::new(
            egui::RichText::new(t!("track.clear_selection").as_ref()).size(crate::theme::BODY_FONT),
        ))
        .clicked()
    {
        *info_content = None;
    }

    false
}

/// 计算每轨 skip mask 并发给音频引擎。
pub(crate) fn send_skip_tracks(doc: &Document, audio: Option<&yinhe_audio::CpalAudioHandle>) {
    let skip = doc.compute_skip_mask();
    if let Some(audio) = audio {
        audio.handle.set_skip_tracks(skip);
    }
}
