//! 图层面板：紧凑列出每个音轨，提供 选中 / 可见 / 锁定。
//!
//! - 选中：点击行 = 单选该轨（写 `track_selected`），行用强调色高亮。
//! - 可见：切换 `track_pianoroll_visible`（PR 显示）。
//! - 锁定：切换 `track_locked`（锁定的轨道音符不可选择/编辑，仍播放）。

use eframe::egui;
use egui_material_icons::icons::{ICON_LOCK, ICON_LOCK_OPEN, ICON_VISIBILITY, ICON_VISIBILITY_OFF};

use yinhe_editor_core::document::Document;

/// 行高（与其他属性行一致）。
const ROW_H: f32 = 22.0;
const ICON_SIZE: f32 = 16.0;

/// 显示图层列表。
pub(crate) fn show(ui: &mut egui::Ui, doc: &mut Document) {
    let num_tracks = doc.data.model.tracks.len();
    if num_tracks == 0 {
        crate::widgets::hint::empty_hint(ui, rust_i18n::t!("track.no_tracks").as_ref());
        return;
    }

    // 兜底补齐可见/锁定缓存长度（结构变化后可能短暂不足）。
    while doc.edit.track_pianoroll_visible.len() < num_tracks {
        doc.edit.track_pianoroll_visible.push(true);
    }
    while doc.edit.track_locked.len() < num_tracks {
        doc.edit.track_locked.push(false);
    }

    let conductor_idx = doc.edit.track_cache.conductor_idx;
    egui::ScrollArea::vertical()
        .id_salt("layers_scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            for i in 0..num_tracks {
                let selected = doc.edit.track_selected.contains(&(i as u16));
                let visible = doc
                    .edit
                    .track_pianoroll_visible
                    .get(i)
                    .copied()
                    .unwrap_or(true);
                let locked = doc.edit.track_locked.get(i).copied().unwrap_or(false);
                let name = doc.data.model.tracks[i].name.clone();
                let color = doc
                    .edit
                    .track_cache
                    .colors
                    .get(i)
                    .copied()
                    .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);

                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), ROW_H),
                    egui::Sense::click(),
                );

                // 选中底色。
                if selected {
                    ui.painter()
                        .rect_filled(rect, 4.0, crate::theme::selected_bg());
                } else if resp.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        4.0,
                        crate::theme::hover_color(crate::theme::app_bg()),
                    );
                }

                // 左侧色块（行高一致）。
                let swatch = egui::Rect::from_min_size(
                    egui::pos2(rect.min.x + 2.0, rect.min.y + 3.0),
                    egui::vec2(6.0, ROW_H - 6.0),
                );
                ui.painter().rect_filled(
                    swatch,
                    2.0,
                    egui::Color32::from_rgba_unmultiplied(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                        (color[3] * 255.0) as u8,
                    ),
                );

                // 轨名。
                let text_color = if selected {
                    crate::theme::text_bright()
                } else {
                    crate::theme::text_secondary()
                };
                ui.painter().text(
                    egui::pos2(swatch.max.x + 6.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{:03} {}", i, name),
                    egui::FontId::proportional(crate::theme::SMALL_FONT),
                    text_color,
                );

                // 右侧 可见 / 锁定 图标（Conductor 轨不提供）。
                if Some(i as u16) != conductor_idx {
                    let icon_font = egui::FontId::new(ICON_SIZE, ICON_LOCK.font_family());

                    // 锁定。
                    let lock_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.max.x - 14.0, rect.center().y),
                        egui::vec2(ROW_H, ROW_H),
                    );
                    let lock_hover =
                        ui.input(|i| i.pointer.hover_pos().is_some_and(|p| lock_rect.contains(p)));
                    let lock_icon = if locked { ICON_LOCK } else { ICON_LOCK_OPEN };
                    let lock_color = if locked {
                        crate::theme::accent_active()
                    } else if lock_hover {
                        crate::theme::text_bright()
                    } else {
                        crate::theme::text_label()
                    };
                    ui.painter().text(
                        lock_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        lock_icon.codepoint,
                        icon_font.clone(),
                        lock_color,
                    );
                    if lock_hover {
                        let lock_resp = ui.interact(
                            lock_rect,
                            ui.id().with(("layer_lock", i)),
                            egui::Sense::click(),
                        );
                        if lock_resp.clicked() {
                            doc.edit.track_locked[i] = !locked;
                        }
                    }

                    // 可见。
                    let vis_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.max.x - 34.0, rect.center().y),
                        egui::vec2(ROW_H, ROW_H),
                    );
                    let vis_hover =
                        ui.input(|i| i.pointer.hover_pos().is_some_and(|p| vis_rect.contains(p)));
                    let vis_icon = if visible {
                        ICON_VISIBILITY
                    } else {
                        ICON_VISIBILITY_OFF
                    };
                    let vis_color = if visible {
                        crate::theme::text_secondary()
                    } else if vis_hover {
                        crate::theme::text_bright()
                    } else {
                        crate::theme::text_label()
                    };
                    ui.painter().text(
                        vis_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        vis_icon.codepoint,
                        icon_font,
                        vis_color,
                    );
                    if vis_hover {
                        let vis_resp = ui.interact(
                            vis_rect,
                            ui.id().with(("layer_vis", i)),
                            egui::Sense::click(),
                        );
                        if vis_resp.clicked() {
                            doc.edit.track_pianoroll_visible[i] = !visible;
                            doc.edit.pianoroll_view.base.dirty = true;
                        }
                    }

                    // 行点击（避开图标区）→ 单选该轨。
                    if resp.clicked() && !lock_hover && !vis_hover {
                        doc.edit.track_selected.clear();
                        doc.edit.track_selected.insert(i as u16);
                    }
                } else if resp.clicked() {
                    doc.edit.track_selected.clear();
                    doc.edit.track_selected.insert(i as u16);
                }
            }
        });
}
