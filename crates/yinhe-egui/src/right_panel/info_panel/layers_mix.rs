//! MIX 图层面板：以**通道**为单位（MIDI-A01… / Audio-01…）。
//!
//! 与轨道版（`layers.rs`）的差异：行 = 通道（按通道号排序，不支持拖拽排序）；
//! - 选中：点击行 = 选中该通道的全部轨道（`track_selected`）；
//! - 可见：切换 `track_visible`（通道 AR / 轨道 AR 的显隐），作用于通道全部轨道；
//! - 锁定：切换 `track_locked`，作用于通道全部轨道。

use eframe::egui;
use egui_material_icons::icons::{ICON_LOCK, ICON_LOCK_OPEN, ICON_VISIBILITY, ICON_VISIBILITY_OFF};
use rust_i18n::t;

use yinhe_editor_core::document::Document;

use crate::channel_arrange::{CaChannel, CaGroups};
use crate::scaling::scaled_font;
use crate::theme;

const ICON_SIZE: f32 = 16.0;

/// 显示 MIX 图层列表（通道级）。返回恒 `false`（通道顺序由数据决定，无结构变化）。
pub(crate) fn show(ui: &mut egui::Ui, doc: &mut Document) -> bool {
    crate::right_panel::region_hint(ui, t!("hint.panel.layers"));
    let conductor_idx = doc.edit.track_cache.conductor_idx;
    let groups = CaGroups::from_model(&doc.data.model, conductor_idx);
    let total = groups.midi.len() + groups.audio.len();
    if total == 0 {
        crate::widgets::hint::empty_hint(ui, t!("track.no_tracks").as_ref());
        return false;
    }

    // 兜底补齐可见/锁定缓存长度（结构变化后可能短暂不足）。
    let num_tracks = doc.data.model.tracks.len();
    while doc.edit.track_visible.len() < num_tracks {
        doc.edit.track_visible.push(true);
    }
    while doc.edit.track_locked.len() < num_tracks {
        doc.edit.track_locked.push(false);
    }

    let row_h = crate::widgets::control::h(ui.ctx());
    let pad_x = scaled_font(ui.ctx(), theme::PAD_X);
    let icon_gap = scaled_font(ui.ctx(), 6.0);

    crate::widgets::scroll::rows_scroll_full(ui, "layers_mix_scroll", row_h, None, |ui| {
        for group in groups.iter_all() {
            let tracks = &group.tracks;
            if tracks.is_empty() {
                continue;
            }
            let all_visible = tracks.iter().all(|&ti| {
                doc.edit
                    .track_visible
                    .get(ti as usize)
                    .copied()
                    .unwrap_or(true)
            });
            let any_locked = tracks.iter().any(|&ti| {
                doc.edit
                    .track_locked
                    .get(ti as usize)
                    .copied()
                    .unwrap_or(false)
            });
            let selected = tracks
                .iter()
                .all(|&ti| doc.edit.track_selected.contains(&ti));
            let color = group.color(&doc.edit.track_cache.colors);
            let name = match group.channel {
                CaChannel::Midi(c) => crate::mix::channel_label(c),
                CaChannel::Audio(c) => crate::mix::audio_label(c),
            };

            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), row_h),
                egui::Sense::click(),
            );

            // 选中底色 / 悬停。
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

            // 左侧色块（通道色带 = 首轨色）。
            let swatch = egui::Rect::from_min_size(
                egui::pos2(rect.min.x + pad_x, rect.min.y + 3.0),
                egui::vec2(6.0, row_h - 6.0),
            );
            ui.painter().rect_filled(
                swatch,
                2.0,
                theme::rgba_to_color32((color[0], color[1], color[2], color[3])),
            );

            // 通道名 + 轨道数。
            let text_color = if selected {
                theme::text_bright()
            } else {
                theme::text_secondary()
            };
            ui.painter().text(
                egui::pos2(swatch.max.x + icon_gap, rect.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{}  {}", name, t!("ca.tracks", n = tracks.len())),
                egui::FontId::proportional(scaled_font(ui.ctx(), theme::SMALL_FONT)),
                text_color,
            );

            // 右侧 可见 / 锁定 图标（作用于通道全部轨道）。
            let icon_font =
                egui::FontId::new(scaled_font(ui.ctx(), ICON_SIZE), ICON_LOCK.font_family());

            // 锁定。
            let lock_rect = egui::Rect::from_center_size(
                egui::pos2(rect.max.x - scaled_font(ui.ctx(), 14.0), rect.center().y),
                egui::vec2(row_h, row_h),
            );
            let lock_hover =
                ui.input(|i| i.pointer.hover_pos().is_some_and(|p| lock_rect.contains(p)));
            let lock_icon = if any_locked {
                ICON_LOCK
            } else {
                ICON_LOCK_OPEN
            };
            let lock_color = if any_locked {
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
                    ui.scope_id().with(("layer_mix_lock", group.channel)),
                    egui::Sense::click(),
                );
                crate::widgets::hint::hover(ui.ctx(), &lock_resp, t!("hint.info.layer_lock"));
                if lock_resp.clicked() {
                    let target = !any_locked;
                    for &ti in tracks {
                        if let Some(v) = doc.edit.track_locked.get_mut(ti as usize) {
                            *v = target;
                        }
                    }
                }
            }

            // 可见。
            let vis_rect = egui::Rect::from_center_size(
                egui::pos2(rect.max.x - scaled_font(ui.ctx(), 34.0), rect.center().y),
                egui::vec2(row_h, row_h),
            );
            let vis_hover =
                ui.input(|i| i.pointer.hover_pos().is_some_and(|p| vis_rect.contains(p)));
            let vis_icon = if all_visible {
                ICON_VISIBILITY
            } else {
                ICON_VISIBILITY_OFF
            };
            let vis_color = if all_visible {
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
                    ui.scope_id().with(("layer_mix_vis", group.channel)),
                    egui::Sense::click(),
                );
                crate::widgets::hint::hover(ui.ctx(), &vis_resp, t!("hint.info.layer_visible"));
                if vis_resp.clicked() {
                    let target = !all_visible;
                    for &ti in tracks {
                        if let Some(v) = doc.edit.track_visible.get_mut(ti as usize) {
                            *v = target;
                        }
                    }
                    // 手动切换可见性时退出图层独奏，避免残留还原状态。
                    doc.edit.layer_solo.clear();
                    doc.edit.layer_solo_prev.clear();
                    doc.edit.pianoroll_view.base.dirty = true;
                }
            }

            // 行点击（避开图标区）→ 选中该通道全部轨道。
            if resp.clicked() && !lock_hover && !vis_hover {
                doc.edit.track_selected.clear();
                for &ti in tracks {
                    doc.edit.track_selected.insert(ti);
                }
            }
        }
    });

    false
}
