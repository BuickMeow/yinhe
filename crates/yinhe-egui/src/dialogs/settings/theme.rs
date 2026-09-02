use eframe::egui;
use rust_i18n::t;
use yinhe_theme::base::{BaseColors, Rgba};
use yinhe_theme::egui_colors::derive_theme;

use crate::audio_settings::AudioSettings;

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
    id: &str,
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
    // 背景
    if ui.is_rect_visible(card_rect) {
        let painter = ui.painter_at(card_rect);
        painter.rect_filled(card_rect, egui::CornerRadius::same(8), preview.app_bg);
        painter.rect_stroke(
            card_rect,
            egui::CornerRadius::same(8),
            stroke,
            egui::StrokeKind::Inside,
        );
        // 标题
        let title_pos = card_rect.min + egui::vec2(8.0, 14.0);
        painter.text(
            title_pos,
            egui::Align2::LEFT_CENTER,
            display_name,
            egui::FontId::proportional(11.0),
            preview.text_primary,
        );
        // 预览 mock 区域
        let mock_rect = egui::Rect::from_min_max(
            card_rect.min + egui::vec2(8.0, 28.0),
            card_rect.max - egui::vec2(8.0, 8.0),
        );
        // mock 背景（control 层级）
        painter.rect_filled(mock_rect, egui::CornerRadius::same(4), preview.control_bg);
        // 三档文字条 + 强调色块 + 网格线
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
        // 网格线示意
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
    // 日/月切换按钮（右上角 18x18）
    let btn_rect = egui::Rect::from_min_max(
        egui::pos2(card_rect.max.x - 24.0, card_rect.min.y + 5.0),
        egui::pos2(card_rect.max.x - 6.0, card_rect.min.y + 23.0),
    );
    let btn_id = ui.id().with(format!("theme_toggle_{id}"));
    let btn_resp = ui.interact(btn_rect, btn_id, egui::Sense::click());
    let is_dark = base.is_dark();
    let icon = if is_dark {
        egui_material_icons::icons::ICON_SUNNY
    } else {
        egui_material_icons::icons::ICON_DARK_MODE
    };
    if ui.is_rect_visible(btn_rect) {
        let painter = ui.painter_at(btn_rect);
        let bg = if btn_resp.hovered() {
            preview.hovered(preview.control_bg)
        } else if btn_resp.is_pointer_button_down_on() {
            preview.pressed(preview.control_bg)
        } else {
            preview.control_bg
        };
        painter.rect_filled(btn_rect, egui::CornerRadius::same(4), bg);
        painter.rect_stroke(
            btn_rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, preview.line_fg.gamma_multiply(0.35)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint.to_string(),
            egui::FontId::new(13.0, icon.font_family()),
            preview.text_primary,
        );
    }
    // 悬停提示
    if btn_resp.hovered() {
        let tip = if is_dark {
            t!("settings.theme.to_light").to_string()
        } else {
            t!("settings.theme.to_dark").to_string()
        };
        btn_resp.clone().on_hover_text(tip);
    }
    let card_clicked = card_resp.clicked() && !btn_resp.clicked();
    let toggle_clicked = btn_resp.clicked();
    (card_clicked, toggle_clicked)
}

// ── 各分类内容 ──

pub fn show_theme_tab(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;

    ui.heading(t!("settings.theme.heading").as_ref());
    ui.label(t!("settings.theme.hint").as_ref());
    ui.add_space(8.0);

    // ── 预设卡片网格（替代 ComboBox） ──
    egui::Grid::new("theme_cards_grid")
        .num_columns(3)
        .spacing([12.0, 12.0])
        .show(ui, |ui| {
            for (idx, (name, base)) in BaseColors::PRESETS.iter().enumerate() {
                let key = format!("settings.theme.{}", name.replace('-', "_"));
                let display = t!(key.as_str()).to_string();
                let inverted = base.inverted();
                let is_selected = settings.theme_base == *base || settings.theme_base == inverted;
                let (card_clicked, toggle_clicked) =
                    theme_preview_card(ui, name, *base, &display, is_selected);
                if card_clicked {
                    settings.theme_base = *base;
                    settings.theme_preset = (*name).to_string();
                    crate::theme::set_theme(*base);
                    changed = true;
                }
                if toggle_clicked {
                    let inv = base.inverted();
                    settings.theme_base = inv;
                    settings.theme_preset = "custom".to_string();
                    crate::theme::set_theme(inv);
                    changed = true;
                }
                if (idx + 1) % 3 == 0 {
                    ui.end_row();
                }
            }
        });

    // 自定义提示：当前为自定义配色时显示
    let is_custom = BaseColors::PRESETS
        .iter()
        .all(|(_, b)| settings.theme_base != *b && settings.theme_base != b.inverted());
    if is_custom {
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
