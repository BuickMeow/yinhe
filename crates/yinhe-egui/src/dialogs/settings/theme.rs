use eframe::egui;
use rust_i18n::t;
use yinhe_theme::base::{BaseColors, Rgba};
use yinhe_theme::egui_colors::derive_theme;

use crate::audio_settings::AudioSettings;
use yinhe_editor_core::audio_settings::CustomTheme;

/// 编辑一个标准色（调色板弹窗：RGBA/HSV 数值可切换 ↔ 主题 Rgba）。
fn edit_std_color(ui: &mut egui::Ui, label: &str, rgba: &mut Rgba) -> bool {
    ui.label(label);
    let mut c = rgba.to_color32();
    let changed = crate::widgets::color_picker::color_edit_button(ui, &mut c).changed();
    if changed {
        *rgba = Rgba::from_color32(c);
    }
    changed
}

fn theme_preview_card(
    ui: &mut egui::Ui,
    base: BaseColors,
    display_name: &str,
    is_selected: bool,
) -> bool {
    let card_size = egui::vec2(158.0, 96.0);
    let (card_rect, card_resp) = ui.allocate_exact_size(card_size, egui::Sense::click());
    let preview = derive_theme(base);
    let cur_accent = crate::theme::accent_active();
    let cur_line = crate::theme::line_fg();
    let stroke = if is_selected {
        egui::Stroke::new(2.0, cur_accent)
    } else if card_resp.hovered() {
        egui::Stroke::new(1.5, cur_line.gamma_multiply(0.9))
    } else {
        egui::Stroke::new(1.0, cur_line.gamma_multiply(0.45))
    };
    if ui.is_rect_visible(card_rect) {
        let painter = ui.painter_at(card_rect);
        painter.rect_filled(card_rect, egui::CornerRadius::same(8), preview.app_bg);
        painter.rect_stroke(
            card_rect,
            egui::CornerRadius::same(8),
            stroke,
            egui::StrokeKind::Inside,
        );
        let title_pos = card_rect.min + egui::vec2(8.0, 14.0);
        painter.text(
            title_pos,
            egui::Align2::LEFT_CENTER,
            display_name,
            egui::FontId::proportional(11.0),
            preview.text_primary,
        );
        let mock_rect = egui::Rect::from_min_max(
            card_rect.min + egui::vec2(8.0, 28.0),
            card_rect.max - egui::vec2(8.0, 8.0),
        );
        painter.rect_filled(mock_rect, egui::CornerRadius::same(4), preview.control_bg);
        let r1 = egui::Rect::from_min_max(
            mock_rect.min + egui::vec2(6.0, 6.0),
            egui::pos2(mock_rect.min.x + 42.0, mock_rect.min.y + 10.0),
        );
        painter.rect_filled(r1, egui::CornerRadius::same(2), preview.text_primary);
        let r2 = egui::Rect::from_min_max(
            egui::pos2(mock_rect.min.x + 6.0, mock_rect.min.y + 14.0),
            egui::pos2(mock_rect.min.x + 68.0, mock_rect.min.y + 18.0),
        );
        painter.rect_filled(r2, egui::CornerRadius::same(2), preview.text_secondary);
        let r3 = egui::Rect::from_min_max(
            egui::pos2(mock_rect.min.x + 6.0, mock_rect.min.y + 22.0),
            egui::pos2(mock_rect.min.x + 26.0, mock_rect.min.y + 26.0),
        );
        painter.rect_filled(r3, egui::CornerRadius::same(2), preview.accent_active);
        let line_y = mock_rect.min.y + 34.0;
        painter.hline(
            mock_rect.min.x + 6.0..=mock_rect.max.x - 6.0,
            line_y,
            egui::Stroke::new(1.0, preview.line_fg.gamma_multiply(0.7)),
        );
        let line_y2 = mock_rect.min.y + 40.0;
        painter.hline(
            mock_rect.min.x + 6.0..=mock_rect.max.x - 6.0,
            line_y2,
            egui::Stroke::new(1.0, preview.grid_sub_beat),
        );
    }
    card_resp.clicked()
}

fn add_custom_card(ui: &mut egui::Ui) -> bool {
    let card_size = egui::vec2(158.0, 96.0);
    let (card_rect, card_resp) = ui.allocate_exact_size(card_size, egui::Sense::click());
    let cur_bg = crate::theme::control_bg();
    let cur_accent = crate::theme::accent_active();
    let cur_line = crate::theme::line_fg();
    let cur_text = crate::theme::text_primary();
    let stroke = if card_resp.hovered() {
        egui::Stroke::new(1.5, cur_accent)
    } else {
        egui::Stroke::new(1.0, cur_line.gamma_multiply(0.45))
    };
    if ui.is_rect_visible(card_rect) {
        let painter = ui.painter_at(card_rect);
        painter.rect_filled(card_rect, egui::CornerRadius::same(8), cur_bg);
        painter.rect_stroke(
            card_rect,
            egui::CornerRadius::same(8),
            stroke,
            egui::StrokeKind::Inside,
        );
        // Material + 号居中
        let icon = egui_material_icons::icons::ICON_ADD;
        painter.text(
            card_rect.center() - egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            icon.codepoint.to_string(),
            egui::FontId::new(28.0, icon.font_family()),
            cur_text,
        );
        painter.text(
            card_rect.center() + egui::vec2(0.0, 18.0),
            egui::Align2::CENTER_CENTER,
            t!("settings.theme.add_custom").to_string(),
            egui::FontId::proportional(11.0),
            cur_text,
        );
    }
    card_resp.clicked()
}

fn custom_preview_card(
    ui: &mut egui::Ui,
    id: u64,
    base: BaseColors,
    display_name: &str,
    is_selected: bool,
) -> (bool, bool) {
    let card_size = egui::vec2(158.0, 96.0);
    let (card_rect, card_resp) = ui.allocate_exact_size(card_size, egui::Sense::click());
    let preview = derive_theme(base);
    let cur_accent = crate::theme::accent_active();
    let cur_line = crate::theme::line_fg();
    let stroke = if is_selected {
        egui::Stroke::new(2.0, cur_accent)
    } else if card_resp.hovered() {
        egui::Stroke::new(1.5, cur_line.gamma_multiply(0.9))
    } else {
        egui::Stroke::new(1.0, cur_line.gamma_multiply(0.45))
    };
    if ui.is_rect_visible(card_rect) {
        let painter = ui.painter_at(card_rect);
        painter.rect_filled(card_rect, egui::CornerRadius::same(8), preview.app_bg);
        painter.rect_stroke(
            card_rect,
            egui::CornerRadius::same(8),
            stroke,
            egui::StrokeKind::Inside,
        );
        let title_pos = card_rect.min + egui::vec2(8.0, 14.0);
        painter.text(
            title_pos,
            egui::Align2::LEFT_CENTER,
            display_name,
            egui::FontId::proportional(11.0),
            preview.text_primary,
        );
        let mock_rect = egui::Rect::from_min_max(
            card_rect.min + egui::vec2(8.0, 28.0),
            card_rect.max - egui::vec2(8.0, 8.0),
        );
        painter.rect_filled(mock_rect, egui::CornerRadius::same(4), preview.control_bg);
        let r1 = egui::Rect::from_min_max(
            mock_rect.min + egui::vec2(6.0, 6.0),
            egui::pos2(mock_rect.min.x + 42.0, mock_rect.min.y + 10.0),
        );
        painter.rect_filled(r1, egui::CornerRadius::same(2), preview.text_primary);
        let r2 = egui::Rect::from_min_max(
            egui::pos2(mock_rect.min.x + 6.0, mock_rect.min.y + 14.0),
            egui::pos2(mock_rect.min.x + 68.0, mock_rect.min.y + 18.0),
        );
        painter.rect_filled(r2, egui::CornerRadius::same(2), preview.text_secondary);
        let r3 = egui::Rect::from_min_max(
            egui::pos2(mock_rect.min.x + 6.0, mock_rect.min.y + 22.0),
            egui::pos2(mock_rect.min.x + 26.0, mock_rect.min.y + 26.0),
        );
        painter.rect_filled(r3, egui::CornerRadius::same(2), preview.accent_active);
        let line_y = mock_rect.min.y + 34.0;
        painter.hline(
            mock_rect.min.x + 6.0..=mock_rect.max.x - 6.0,
            line_y,
            egui::Stroke::new(1.0, preview.line_fg.gamma_multiply(0.7)),
        );
        let line_y2 = mock_rect.min.y + 40.0;
        painter.hline(
            mock_rect.min.x + 6.0..=mock_rect.max.x - 6.0,
            line_y2,
            egui::Stroke::new(1.0, preview.grid_sub_beat),
        );
    }
    // 删除按钮（右上角 18x18，×）
    let del_rect = egui::Rect::from_min_max(
        egui::pos2(card_rect.max.x - 24.0, card_rect.min.y + 5.0),
        egui::pos2(card_rect.max.x - 6.0, card_rect.min.y + 23.0),
    );
    let del_id = ui.id().with(format!("del_{id}"));
    let del_resp = ui.interact(del_rect, del_id, egui::Sense::click());
    let preview = derive_theme(base);
    if ui.is_rect_visible(del_rect) {
        let painter = ui.painter_at(del_rect);
        let bg = if del_resp.hovered() {
            preview.hovered(preview.control_bg)
        } else {
            preview.control_bg
        };
        painter.rect_filled(del_rect, egui::CornerRadius::same(4), bg);
        painter.rect_stroke(
            del_rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, preview.line_fg.gamma_multiply(0.35)),
            egui::StrokeKind::Inside,
        );
        let icon = egui_material_icons::icons::ICON_CLOSE;
        painter.text(
            del_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint.to_string(),
            egui::FontId::new(13.0, icon.font_family()),
            preview.text_primary,
        );
    }
    let card_clicked = card_resp.clicked() && !del_resp.clicked();
    let del_clicked = del_resp.clicked();
    (card_clicked, del_clicked)
}

// ── 各分类内容 ──

pub fn show_theme_tab(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;

    // 标题行 + 全局日/月切换
    ui.horizontal(|ui| {
        ui.heading(t!("settings.theme.heading").as_ref());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let is_dark = settings.theme_base.is_dark();
            let icon = if is_dark {
                egui_material_icons::icons::ICON_SUNNY
            } else {
                egui_material_icons::icons::ICON_DARK_MODE
            };
            let btn = egui::Button::new(
                egui::RichText::new(icon.codepoint.to_string())
                    .family(icon.font_family())
                    .size(16.0),
            )
            .min_size(egui::vec2(32.0, 24.0));
            if ui
                .add(btn)
                .on_hover_text(if is_dark {
                    t!("settings.theme.to_light").to_string()
                } else {
                    t!("settings.theme.to_dark").to_string()
                })
                .clicked()
            {
                let inv = settings.theme_base.inverted();
                settings.theme_base = inv;
                settings.theme_preset = "custom".to_string();
                crate::theme::set_theme(inv);
                changed = true;
            }
        });
    });
    ui.label(t!("settings.theme.hint").as_ref());
    ui.add_space(8.0);

    // ── 预设卡片网格（首位 + 号 + 自定义 + 100 预设） ──
    let mut to_delete: Option<u64> = None;
    let mut custom_to_apply: Option<BaseColors> = None;
    let mut custom_preset_id: Option<u64> = None;
    egui::Grid::new("theme_cards_grid")
        .num_columns(3)
        .spacing([12.0, 12.0])
        .show(ui, |ui| {
            let mut col = 0u32;
            // 1) + 号卡片
            if add_custom_card(ui) {
                settings.custom_theme_draft = Some(CustomTheme {
                    id: 0,
                    name: String::new(),
                    base: BaseColors::DARK,
                });
                settings.show_custom_theme_editor = true;
            }
            col += 1;
            if col.is_multiple_of(3) {
                ui.end_row();
            }
            // 2) 自定义主题卡片
            for ct in settings.custom_themes.clone() {
                let is_selected = settings.theme_base == ct.base;
                let (card_clicked, del_clicked) =
                    custom_preview_card(ui, ct.id, ct.base, &ct.name, is_selected);
                if card_clicked {
                    custom_to_apply = Some(ct.base);
                    custom_preset_id = Some(ct.id);
                }
                if del_clicked {
                    to_delete = Some(ct.id);
                }
                col += 1;
                if col.is_multiple_of(3) {
                    ui.end_row();
                }
            }
            // 3) 内置 100 预设
            for (idx, (name, base)) in BaseColors::PRESETS.iter().enumerate() {
                let _ = idx;
                let key = format!("settings.theme.{}", name.replace('-', "_"));
                let display = t!(key.as_str()).to_string();
                let is_selected = settings.theme_base == *base;
                if theme_preview_card(ui, *base, &display, is_selected) {
                    settings.theme_base = *base;
                    settings.theme_preset = (*name).to_string();
                    crate::theme::set_theme(*base);
                    changed = true;
                }
                col += 1;
                if col.is_multiple_of(3) {
                    ui.end_row();
                }
            }
            // 补齐最后一行
            if !col.is_multiple_of(3) {
                ui.end_row();
            }
        });
    if let Some(id) = to_delete {
        settings.custom_themes.retain(|c| c.id != id);
        changed = true;
    }
    if let Some(base) = custom_to_apply {
        settings.theme_base = base;
        if let Some(pid) = custom_preset_id {
            settings.theme_preset = pid.to_string();
        }
        crate::theme::set_theme(base);
        changed = true;
    }

    // 自定义主题编辑弹窗
    if settings.show_custom_theme_editor {
        let mut open = true;
        egui::Window::new(t!("settings.theme.custom_editor_title").as_ref())
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                let mut draft = settings.custom_theme_draft.take().unwrap_or(CustomTheme {
                    id: 0,
                    name: String::new(),
                    base: BaseColors::DARK,
                });
                ui.label(t!("settings.theme.custom_name").as_ref());
                ui.text_edit_singleline(&mut draft.name);
                ui.add_space(6.0);
                egui::Grid::new("custom_theme_colors")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        let mut changed_inner = false;
                        changed_inner |= edit_std_color(
                            ui,
                            t!("settings.theme.bg").as_ref(),
                            &mut draft.base.bg,
                        );
                        ui.end_row();
                        changed_inner |= edit_std_color(
                            ui,
                            t!("settings.theme.text").as_ref(),
                            &mut draft.base.text,
                        );
                        ui.end_row();
                        changed_inner |= edit_std_color(
                            ui,
                            t!("settings.theme.accent").as_ref(),
                            &mut draft.base.accent,
                        );
                        ui.end_row();
                        let _ = changed_inner;
                    });
                // 预览
                ui.add_space(8.0);
                let preview = derive_theme(draft.base);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(280.0, 56.0), egui::Sense::hover());
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, egui::CornerRadius::same(6), preview.app_bg);
                    painter.rect_stroke(
                        rect,
                        egui::CornerRadius::same(6),
                        egui::Stroke::new(1.0, preview.line_fg.gamma_multiply(0.4)),
                        egui::StrokeKind::Inside,
                    );
                    let inner = rect.shrink(8.0);
                    painter.rect_filled(inner, egui::CornerRadius::same(4), preview.control_bg);
                    painter.text(
                        inner.min + egui::vec2(8.0, 12.0),
                        egui::Align2::LEFT_CENTER,
                        if draft.name.is_empty() {
                            "Preview"
                        } else {
                            &draft.name
                        },
                        egui::FontId::proportional(12.0),
                        preview.text_primary,
                    );
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            inner.min + egui::vec2(8.0, 24.0),
                            egui::pos2(inner.min.x + 28.0, inner.min.y + 32.0),
                        ),
                        egui::CornerRadius::same(2),
                        preview.accent_active,
                    );
                }
                ui.add_space(8.0);
                let mut save = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    if ui.button(t!("common.ok").as_ref()).clicked() {
                        save = true;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        cancel = true;
                    }
                });
                if save {
                    let name = draft.name.trim().to_string();
                    if !name.is_empty() {
                        let next_id = settings
                            .custom_themes
                            .iter()
                            .map(|c| c.id)
                            .max()
                            .unwrap_or(0)
                            + 1;
                        let base = draft.base;
                        settings.custom_themes.push(CustomTheme {
                            id: next_id,
                            name: name.clone(),
                            base,
                        });
                        settings.theme_base = base;
                        settings.theme_preset = next_id.to_string();
                        crate::theme::set_theme(base);
                        changed = true;
                    }
                    settings.show_custom_theme_editor = false;
                    settings.custom_theme_draft = None;
                } else if cancel {
                    settings.show_custom_theme_editor = false;
                    settings.custom_theme_draft = None;
                } else {
                    // 未关闭，放回草稿供下一帧继续编辑
                    settings.custom_theme_draft = Some(draft);
                }
            });
        if !open {
            settings.show_custom_theme_editor = false;
            settings.custom_theme_draft = None;
        }
    }

    // 自定义提示：当前为自定义配色时显示
    let is_custom = {
        let mut matched = false;
        for (_, base) in BaseColors::PRESETS.iter() {
            if settings.theme_base == *base {
                matched = true;
                break;
            }
        }
        if !matched {
            for ct in &settings.custom_themes {
                if settings.theme_base == ct.base {
                    matched = true;
                    break;
                }
            }
        }
        !matched
    };
    if is_custom && !settings.show_custom_theme_editor {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t!("settings.theme.custom_active").as_ref())
                .size(11.0)
                .color(crate::theme::text_muted()),
        );
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    let mut base = settings.theme_base;
    let mut base_changed = false;
    egui::Grid::new("theme_colors_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            base_changed |= edit_std_color(ui, t!("settings.theme.bg").as_ref(), &mut base.bg);
            ui.end_row();
            base_changed |= edit_std_color(ui, t!("settings.theme.text").as_ref(), &mut base.text);
            ui.end_row();
            base_changed |=
                edit_std_color(ui, t!("settings.theme.accent").as_ref(), &mut base.accent);
            ui.end_row();
        });
    if base_changed {
        settings.theme_base = base;
        settings.theme_preset = "custom".to_string();
        crate::theme::set_theme(base);
        changed = true;
    }

    // 界面缩放：拖动中不缩放（缩放会让滑条自身位置来回跑），松手才应用
    egui::Grid::new("theme_scale_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label(t!("settings.theme.ui_scale").as_ref());
            ui.horizontal(|ui| {
                let mut scale = settings.ui_scale;
                let resp = ui.add(
                    egui::Slider::new(&mut scale, 0.75..=2.0)
                        .step_by(0.05)
                        .show_value(true),
                );
                if resp.changed() {
                    settings.ui_scale = scale;
                    changed = true;
                }
                if resp.drag_stopped() {
                    main_ctx.set_zoom_factor(settings.ui_scale);
                }
                if ui
                    .button(t!("settings.theme.reset_scale").as_ref())
                    .clicked()
                {
                    settings.ui_scale = 1.0;
                    main_ctx.set_zoom_factor(1.0);
                    changed = true;
                }
            });
            ui.end_row();
        });
    changed
}
