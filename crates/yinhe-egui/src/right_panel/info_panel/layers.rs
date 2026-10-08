//! 图层面板：紧凑列出每个音轨，提供 选中 / 可见 / 锁定。
//!
//! - 选中：点击行 = 单选该轨（写 `track_selected`），行用强调色高亮。
//! - 可见：切换 `track_pianoroll_visible`（PR 显示）。
//! - 锁定：切换 `track_locked`（锁定的轨道音符不可选择/编辑，仍播放）。

use eframe::egui;
use egui_material_icons::icons::{ICON_LOCK, ICON_LOCK_OPEN, ICON_VISIBILITY, ICON_VISIBILITY_OFF};

use yinhe_editor_core::document::Document;

use crate::scaling::scaled_font;
use crate::theme;

const ICON_SIZE: f32 = 16.0;

/// 显示图层列表。返回是否发生「音轨结构变化」（拖动排序），
/// 调用方据此 drop 音频引擎以便下帧重建。
pub(crate) fn show(ui: &mut egui::Ui, doc: &mut Document) -> bool {
    crate::right_panel::region_hint(ui, rust_i18n::t!("hint.panel.layers"));
    let num_tracks = doc.data.model.tracks.len();
    if num_tracks == 0 {
        crate::widgets::hint::empty_hint(ui, rust_i18n::t!("track.no_tracks").as_ref());
        return false;
    }

    // 兜底补齐可见/锁定缓存长度（结构变化后可能短暂不足）。
    while doc.edit.track_pianoroll_visible.len() < num_tracks {
        doc.edit.track_pianoroll_visible.push(true);
    }
    while doc.edit.track_locked.len() < num_tracks {
        doc.edit.track_locked.push(false);
    }

    let conductor_idx = doc.edit.track_cache.conductor_idx;
    let row_h = crate::widgets::control::h(ui.ctx());
    let pad_x = scaled_font(ui.ctx(), theme::PAD_X);
    let icon_gap = scaled_font(ui.ctx(), 6.0);

    // 拖拽排序跨帧状态（算法见 widgets::reorder，与 AR 音轨面板同源）。
    let drag_id = egui::Id::new("layers_drag");
    let mut drag: Option<crate::widgets::reorder::DragReorder> = ui
        .ctx()
        .data_mut(|d| d.get_temp(drag_id))
        .unwrap_or_default();
    let mut item_rects: Vec<egui::Rect> = Vec::with_capacity(num_tracks);
    let mut structural_changed = false;

    crate::widgets::scroll::rows_scroll_full(ui, "layers_scroll", row_h, None, |ui| {
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
                egui::vec2(ui.available_width(), row_h),
                egui::Sense::click_and_drag(),
            );
            item_rects.push(rect);

            // 选中底色。
            if selected {
                ui.painter()
                    .rect_filled(rect, theme::ROW_RADIUS, theme::selected_bg());
            } else if resp.hovered() {
                ui.painter().rect_filled(
                    rect,
                    theme::ROW_RADIUS,
                    theme::hover_color(theme::app_bg()),
                );
            }

            // 左侧色块（行高一致）。
            let swatch = egui::Rect::from_min_size(
                egui::pos2(rect.min.x + pad_x, rect.min.y + 3.0),
                egui::vec2(6.0, row_h - 6.0),
            );
            ui.painter().rect_filled(
                swatch,
                2.0,
                theme::rgba_to_color32((color[0], color[1], color[2], color[3])),
            );

            // 轨名。
            let text_color = if selected {
                theme::text_bright()
            } else {
                theme::text_secondary()
            };
            ui.painter().text(
                egui::pos2(swatch.max.x + icon_gap, rect.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{:03} {}", i, name),
                egui::FontId::proportional(scaled_font(ui.ctx(), theme::SMALL_FONT)),
                text_color,
            );

            // 右侧 可见 / 锁定 图标（Conductor 轨不提供）。
            if Some(i as u16) != conductor_idx {
                let icon_font =
                    egui::FontId::new(scaled_font(ui.ctx(), ICON_SIZE), ICON_LOCK.font_family());

                // 锁定。
                let lock_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.max.x - scaled_font(ui.ctx(), 14.0), rect.center().y),
                    egui::vec2(row_h, row_h),
                );
                let lock_hover =
                    ui.input(|i| i.pointer.hover_pos().is_some_and(|p| lock_rect.contains(p)));
                let lock_icon = if locked { ICON_LOCK } else { ICON_LOCK_OPEN };
                let lock_color = if locked {
                    theme::accent_active()
                } else if lock_hover {
                    theme::text_bright()
                } else {
                    theme::text_label()
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
                    crate::widgets::hint::hover(
                        ui.ctx(),
                        &lock_resp,
                        rust_i18n::t!("hint.info.layer_lock"),
                    );
                    if lock_resp.clicked() {
                        doc.edit.track_locked[i] = !locked;
                    }
                }

                // 可见。
                let vis_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.max.x - scaled_font(ui.ctx(), 34.0), rect.center().y),
                    egui::vec2(row_h, row_h),
                );
                let vis_hover =
                    ui.input(|i| i.pointer.hover_pos().is_some_and(|p| vis_rect.contains(p)));
                let vis_icon = if visible {
                    ICON_VISIBILITY
                } else {
                    ICON_VISIBILITY_OFF
                };
                let soloed = doc.edit.layer_solo.contains(&(i as u16));
                let vis_color = if soloed {
                    theme::accent_active()
                } else if visible {
                    theme::text_secondary()
                } else if vis_hover {
                    theme::text_bright()
                } else {
                    theme::text_label()
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
                    crate::widgets::hint::hover(
                        ui.ctx(),
                        &vis_resp,
                        rust_i18n::t!("hint.info.layer_visible"),
                    );
                    if vis_resp.clicked() {
                        doc.edit.track_pianoroll_visible[i] = !visible;
                        // 手动切换可见性时退出图层独奏，避免残留还原状态。
                        doc.edit.layer_solo.clear();
                        doc.edit.layer_solo_prev.clear();
                        doc.edit.pianoroll_view.base.dirty = true;
                    }
                    // 右击眼睛图标 = 图层独奏（可叠加，多轨一起看）。
                    if vis_resp.secondary_clicked() {
                        let key = i as u16;
                        if doc.edit.layer_solo.remove(&key) {
                            // 取消本轨独奏：全部取消则还原快照，否则本轨隐藏。
                            if doc.edit.layer_solo.is_empty() {
                                if doc.edit.layer_solo_prev.len()
                                    == doc.edit.track_pianoroll_visible.len()
                                {
                                    doc.edit.track_pianoroll_visible =
                                        doc.edit.layer_solo_prev.clone();
                                } else {
                                    doc.edit.track_pianoroll_visible.fill(true);
                                }
                                doc.edit.layer_solo_prev.clear();
                            } else {
                                doc.edit.track_pianoroll_visible[i] = false;
                            }
                        } else {
                            // 加入独奏：首次进入保存快照；只显示独奏集合内的轨。
                            if doc.edit.layer_solo.is_empty() {
                                doc.edit.layer_solo_prev = doc.edit.track_pianoroll_visible.clone();
                            }
                            doc.edit.layer_solo.insert(key);
                            let solo = doc.edit.layer_solo.clone();
                            for (t, v) in doc.edit.track_pianoroll_visible.iter_mut().enumerate() {
                                *v = solo.contains(&(t as u16));
                            }
                            // 独奏即自动选中该轨。
                            doc.edit.track_selected.clear();
                            doc.edit.track_selected.insert(key);
                        }
                        doc.edit.pianoroll_view.base.dirty = true;
                    }
                }

                // 拖拽排序：从行体（避开图标区）开始拖动。
                if resp.drag_started() && drag.is_none() && !lock_hover && !vis_hover {
                    if !doc.edit.track_selected.contains(&(i as u16)) {
                        doc.edit.track_selected.clear();
                        doc.edit.track_selected.insert(i as u16);
                    }
                    let mut indices: Vec<usize> = doc
                        .edit
                        .track_selected
                        .iter()
                        .map(|&t| t as usize)
                        .collect();
                    indices.sort_unstable();
                    // Conductor 不参与排序。
                    indices.retain(|&j| Some(j as u16) != conductor_idx);
                    if !indices.is_empty() {
                        drag = Some(crate::widgets::reorder::DragReorder {
                            indices,
                            insert_idx: i,
                        });
                    }
                }

                // 行点击（避开图标区、且不在拖拽中）→ 单选该轨。
                if resp.clicked() && !lock_hover && !vis_hover && drag.is_none() {
                    doc.edit.track_selected.clear();
                    doc.edit.track_selected.insert(i as u16);
                }
            } else if resp.clicked() && drag.is_none() {
                doc.edit.track_selected.clear();
                doc.edit.track_selected.insert(i as u16);
            }
        }

        // 拖拽排序：更新插入位置、画插入线、松手落地。
        if let Some(d) = drag.as_mut() {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                d.update_insert_idx(p.y, &item_rects);
                // 第 0 行是 Conductor：不能把音轨插到它之前/上。
                d.insert_idx = d.insert_idx.max(1);
            }
            if let Some(y) = d.insert_line_y(&item_rects)
                && let (Some(first), Some(last)) = (item_rects.first(), item_rects.last())
            {
                ui.painter().line_segment(
                    [
                        egui::pos2(first.min.x + 4.0, y),
                        egui::pos2(last.max.x - 4.0, y),
                    ],
                    egui::Stroke::new(3.0, theme::accent_active()),
                );
            }
            if ui.input(|i| i.pointer.any_released()) {
                let indices = d.indices.clone();
                let insert_at = d.insert_idx;
                drag = None;
                structural_changed = apply_reorder(doc, &indices, insert_at);
            }
        }
    });

    ui.ctx().data_mut(|d| d.insert_temp(drag_id, drag));
    structural_changed
}

/// 把一次拖拽排序（`indices` → `insert_at`）拆成逐个 `move_track`，
/// 合并为一个 Composite undo。返回是否真的发生改动。
fn apply_reorder(doc: &mut Document, indices: &[usize], insert_at: usize) -> bool {
    let moves =
        crate::widgets::reorder::plan_moves(doc.data.model.tracks.len(), indices, insert_at);
    if moves.is_empty() {
        return false;
    }
    let before = doc.capture_snapshot();
    let mut subs = Vec::new();
    for (from, to) in moves {
        if let Some(action) = doc.move_track(from, to) {
            subs.push(action);
        }
    }
    if subs.is_empty() {
        return false;
    }
    doc.push_undo(
        yinhe_editor_core::history::UndoAction::Composite(subs),
        rust_i18n::t!("undo.move_track").as_ref(),
        before,
    );
    true
}
