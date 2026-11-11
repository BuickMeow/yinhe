//! 图层面板：紧凑列出每个音轨，提供 选中 / 可见 / 锁定。
//!
//! - 选中：点击行 = 单选该轨（写 `track_selected`），行用强调色高亮。
//! - 可见：切换 `track_pianoroll_visible`（PR 显示）。
//! - 锁定：切换 `track_locked`（锁定的轨道音符不可选择/编辑，仍播放）。

use eframe::egui;
use egui_extras::{Column, TableBuilder};
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
    let drag_id = egui::Id::unique("layers_drag");
    let mut drag: Option<crate::widgets::reorder::DragReorder> = ui
        .ctx()
        .data_mut(|d| d.get_temp(drag_id))
        .unwrap_or_default();
    let mut item_rects: Vec<egui::Rect> = Vec::with_capacity(num_tracks);
    let mut structural_changed = false;

    let table = TableBuilder::new(ui)
        .id_salt("layers_grid")
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::remainder().at_least(120.0))
        .column(Column::exact(row_h))
        .column(Column::exact(row_h))
        .auto_shrink([false, false]);
    table.body(|body| {
        body.rows(row_h, num_tracks, |mut row| {
            let i = row.index();
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

            // ── 第 1 列：色块 + 轨名（整格可点选/拖动排序/右键重命名）──
            row.col(|ui| {
                let cell = ui.max_rect();
                let resp = ui.interact(
                    cell,
                    ui.scope_id().with(("layer_row", i)),
                    egui::Sense::click_and_drag(),
                );
                paint_cell_bg(ui, cell, selected, resp.hovered());
                // 左侧色块。
                let swatch = egui::Rect::from_min_size(
                    egui::pos2(cell.min.x + pad_x, cell.min.y + 3.0),
                    egui::vec2(6.0, row_h - 6.0),
                );
                ui.painter().rect_filled(
                    swatch,
                    2.0,
                    theme::rgba_to_color32((color[0], color[1], color[2], color[3])),
                );
                let text_color = if selected {
                    theme::text_bright()
                } else {
                    theme::text_secondary()
                };
                let name_font =
                    egui::FontId::proportional(scaled_font(ui.ctx(), theme::SMALL_FONT));
                let name_x = swatch.max.x + icon_gap;
                if crate::widgets::track_rename::active(ui, i) {
                    let name_rect = egui::Rect::from_min_size(
                        egui::pos2(name_x, cell.center().y - name_font.size * 0.75),
                        egui::vec2(
                            (cell.max.x - name_x - scaled_font(ui.ctx(), 8.0)).max(80.0),
                            name_font.size * 1.5,
                        ),
                    );
                    if let Some(new_name) =
                        crate::widgets::track_rename::show(ui, i, name_rect, name_font.size)
                    {
                        let before = doc.capture_snapshot();
                        let info = doc.edit.track_cache.info[i].clone();
                        let inst = doc
                            .data
                            .model
                            .channel_instruments
                            .get(info.channel as usize)
                            .copied()
                            .flatten();
                        if let Some(action) =
                            doc.set_track_info(i, new_name, info.port, info.channel, inst)
                        {
                            doc.push_undo(
                                action,
                                rust_i18n::t!("undo.edit_track_info").as_ref(),
                                before,
                            );
                        }
                    }
                } else {
                    ui.painter().text(
                        egui::pos2(name_x, cell.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("{:03} {}", i, name),
                        name_font,
                        text_color,
                    );
                }

                // 右键：重命名。
                resp.context_menu(|ui| {
                    if ui
                        .add(crate::widgets::menu::menu_item_button(
                            ui,
                            false,
                            rust_i18n::t!("arrange.rename_track").as_ref(),
                        ))
                        .clicked()
                    {
                        crate::widgets::track_rename::begin(ui, i, &name);
                        ui.close();
                    }
                });

                // 行点击（不在拖拽中）→ 单选该轨。
                if resp.clicked() && drag.is_none() {
                    doc.edit.track_selected.clear();
                    doc.edit.track_selected.insert(i as u16);
                }
                // 拖拽排序：从行体开始拖动。
                if resp.drag_started() && drag.is_none() {
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
                    indices.retain(|&j| Some(j as u16) != conductor_idx);
                    if !indices.is_empty() {
                        drag = Some(crate::widgets::reorder::DragReorder {
                            indices,
                            insert_idx: i,
                        });
                    }
                }
            });

            // ── 第 2 列：可见（右键独奏）──
            row.col(|ui| {
                ui.set_min_width(row_h);
                let cell = ui.max_rect();
                paint_cell_bg(ui, cell, selected, ui.rect_contains_pointer(cell));
                if Some(i as u16) == conductor_idx {
                    return;
                }
                let soloed = doc.edit.layer_solo.contains(&(i as u16));
                let vis_icon = if visible {
                    ICON_VISIBILITY
                } else {
                    ICON_VISIBILITY_OFF
                };
                let vis_color = if soloed {
                    theme::accent_active()
                } else if visible {
                    theme::text_secondary()
                } else {
                    theme::text_label()
                };
                let resp = icon_cell(ui, vis_icon, vis_color, row_h, ("layer_vis", i));
                crate::widgets::hint::hover(
                    ui.ctx(),
                    &resp,
                    rust_i18n::t!("hint.info.layer_visible"),
                );
                if resp.clicked() {
                    doc.edit.track_pianoroll_visible[i] = !visible;
                    doc.edit.layer_solo.clear();
                    doc.edit.layer_solo_prev.clear();
                    doc.edit.pianoroll_view.base.dirty = true;
                }
                if resp.secondary_clicked() {
                    toggle_solo(doc, i);
                    doc.edit.pianoroll_view.base.dirty = true;
                }
            });

            // ── 第 3 列：锁定 ──
            row.col(|ui| {
                ui.set_min_width(row_h);
                let cell = ui.max_rect();
                paint_cell_bg(ui, cell, selected, ui.rect_contains_pointer(cell));
                if Some(i as u16) == conductor_idx {
                    return;
                }
                let lock_icon = if locked { ICON_LOCK } else { ICON_LOCK_OPEN };
                let lock_color = if locked {
                    theme::accent_active()
                } else {
                    theme::text_label()
                };
                let resp = icon_cell(ui, lock_icon, lock_color, row_h, ("layer_lock", i));
                crate::widgets::hint::hover(ui.ctx(), &resp, rust_i18n::t!("hint.info.layer_lock"));
                if resp.clicked() {
                    doc.edit.track_locked[i] = !locked;
                }
            });

            item_rects.push(row.response().rect);
        });
    });

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

    ui.ctx().data_mut(|d| d.insert_temp(drag_id, drag));
    structural_changed
}

fn paint_cell_bg(ui: &egui::Ui, cell: egui::Rect, selected: bool, hovered: bool) {
    if selected {
        ui.painter()
            .rect_filled(cell, theme::ROW_RADIUS, theme::selected_bg());
    } else if hovered {
        ui.painter()
            .rect_filled(cell, theme::ROW_RADIUS, theme::hover_color(theme::app_bg()));
    }
}

fn icon_cell(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    color: egui::Color32,
    row_h: f32,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
) -> egui::Response {
    ui.push_id(id_salt, |ui| {
        ui.add_space((ui.available_width() - row_h).max(0.0) * 0.5);
        crate::widgets::flat::flat_button_filled_ghost(
            ui,
            icon.rich_text()
                .size(scaled_font(ui.ctx(), ICON_SIZE))
                .color(color),
            egui::vec2(row_h, row_h),
            None,
            true,
        )
    })
    .inner
}

fn toggle_solo(doc: &mut Document, i: usize) {
    let key = i as u16;
    if doc.edit.layer_solo.remove(&key) {
        if doc.edit.layer_solo.is_empty() {
            if doc.edit.layer_solo_prev.len() == doc.edit.track_pianoroll_visible.len() {
                doc.edit.track_pianoroll_visible = doc.edit.layer_solo_prev.clone();
            } else {
                doc.edit.track_pianoroll_visible.fill(true);
            }
            doc.edit.layer_solo_prev.clear();
        } else {
            doc.edit.track_pianoroll_visible[i] = false;
        }
    } else {
        if doc.edit.layer_solo.is_empty() {
            doc.edit.layer_solo_prev = doc.edit.track_pianoroll_visible.clone();
        }
        doc.edit.layer_solo.insert(key);
        let solo = doc.edit.layer_solo.clone();
        for (t, v) in doc.edit.track_pianoroll_visible.iter_mut().enumerate() {
            *v = solo.contains(&(t as u16));
        }
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(key);
    }
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
