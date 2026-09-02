use eframe::egui;
use rust_i18n::t;
use yinhe_theme::base::{BaseColors, FIXED_WARNING, Rgba};
use yinhe_theme::egui_colors::derive_theme;

use crate::audio_settings::AudioSettings;
use yinhe_editor_core::audio_settings::CustomTheme;

/// 编辑一个标准色（保留供潜在复用，当前顶部单行直接内联实现）。
#[allow(dead_code)]
fn edit_std_color(ui: &mut egui::Ui, label: &str, rgba: &mut Rgba) -> bool {
    ui.label(label);
    let mut c = rgba.to_color32();
    let changed = crate::widgets::color_picker::color_edit_button(ui, &mut c).changed();
    if changed {
        *rgba = Rgba::from_color32(c);
    }
    changed
}

#[allow(dead_code)]
// ── 单卡渲染（保留，当前网格内联实现以支持星标与右键菜单） ──
fn theme_card(
    ui: &mut egui::Ui,
    base: BaseColors,
    display_name: &str,
    is_selected: bool,
    is_favorited: bool,
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
        // 标题预留星标区：宽度 20，避免重叠
        let mut title = display_name.to_string();
        // 简单截断：过长时用 .. 省略（egui text 不会自动截断，靠 painter 截断可能溢出星标）
        // 这里不做复杂测量，依赖卡片宽度 158，星标占右 18，标题区 130 足够大多数
        let _ = &mut title;
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
    // 星标按钮（右上角 18x18，固定警告金=已收藏）
    let star_rect = egui::Rect::from_min_max(
        egui::pos2(card_rect.max.x - 24.0, card_rect.min.y + 5.0),
        egui::pos2(card_rect.max.x - 6.0, card_rect.min.y + 23.0),
    );
    // 用随机 id 避免与卡片 id 冲突，基于 display_name+base
    let star_id = ui.id().with(format!("star_{display_name}_{}", base.bg.r));
    let star_resp = ui.interact(star_rect, star_id, egui::Sense::click());
    if ui.is_rect_visible(star_rect) {
        let painter = ui.painter_at(star_rect);
        let bg = if star_resp.hovered() {
            derive_theme(base).hovered(derive_theme(base).control_bg)
        } else {
            egui::Color32::TRANSPARENT
        };
        if star_resp.hovered() {
            painter.rect_filled(star_rect, egui::CornerRadius::same(4), bg);
        }
        let (icon, color) = if is_favorited {
            (
                egui_material_icons::icons::ICON_STAR,
                FIXED_WARNING.to_color32(),
            )
        } else {
            (
                egui_material_icons::icons::ICON_STAR_BORDER,
                derive_theme(base).text_secondary.gamma_multiply(0.55),
            )
        };
        painter.text(
            star_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint.to_string(),
            egui::FontId::new(14.0, icon.font_family()),
            color,
        );
    }
    let card_clicked = card_resp.clicked() && !star_resp.clicked();
    let star_clicked = star_resp.clicked();
    // 右键菜单由调用方通过 card_resp.context_menu 处理，这里仅返回点击状态
    // 为了让外层能调用 context_menu，需要把 card_resp 传出？此处直接在内部注册空菜单占位
    // 实际菜单在 show_theme_tab 中通过 card_rect 的 Response 统一处理
    // 这里保留 card_resp 的 context_menu 能力：调用方需自行持有；简化起见返回 card_rect 供外层使用
    // 但我们已消耗 card_resp，调用方无法再访问；改为在调用方直接处理菜单
    (card_clicked, star_clicked)
}

// ── 各分类内容 ──

pub fn show_theme_tab(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;

    ui.heading(t!("settings.theme.heading").as_ref());
    ui.add_space(8.0);

    // ── 顶部单行：日/月切换（最前） + 背景 / 主文字 / 强调色 ──
    // 日/月切换：翻转当前主题并保留选中态，所有面板按全局明暗统一显示为深/浅（不额外持久化自定义翻转，仅显示层 inverted）
    // 只要预设被改 → 自动新建自定义；若当前已是自定义 → 直接改该自定义
    {
        let mut new_base = settings.theme_base;
        let mut color_changed = false;
        let mut toggle_clicked = false;
        ui.horizontal(|ui| {
            // 日/月在最前
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
                toggle_clicked = true;
            }
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label(t!("settings.theme.bg").as_ref());
            let mut c = new_base.bg.to_color32();
            if crate::widgets::color_picker::color_edit_button(ui, &mut c).changed() {
                new_base.bg = Rgba::from_color32(c);
                color_changed = true;
            }
            ui.add_space(12.0);
            ui.label(t!("settings.theme.text").as_ref());
            let mut c2 = new_base.text.to_color32();
            if crate::widgets::color_picker::color_edit_button(ui, &mut c2).changed() {
                new_base.text = Rgba::from_color32(c2);
                color_changed = true;
            }
            ui.add_space(12.0);
            ui.label(t!("settings.theme.accent").as_ref());
            let mut c3 = new_base.accent.to_color32();
            if crate::widgets::color_picker::color_edit_button(ui, &mut c3).changed() {
                new_base.accent = Rgba::from_color32(c3);
                color_changed = true;
            }
        });
        if toggle_clicked {
            // 根治保留选中：以选中项的原始基色为锚点，按新全局明暗单次 inverted，避免二次漂移
            let old_is_dark = settings.theme_base.is_dark();
            let new_is_dark = !old_is_dark;
            let orig_base: Option<BaseColors> = if let Ok(id) = settings.theme_preset.parse::<u64>()
            {
                settings
                    .custom_themes
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.base)
            } else {
                BaseColors::preset_by_name(&settings.theme_preset)
            };
            let new_base = if let Some(orig) = orig_base {
                if orig.is_dark() == new_is_dark {
                    orig
                } else {
                    orig.inverted()
                }
            } else {
                // 兜底：未找到选中项（legacy custom 字符串等），直接翻转当前
                settings.theme_base.inverted()
            };
            settings.theme_base = new_base;
            crate::theme::set_theme(new_base);
            changed = true;
        }
        if color_changed {
            // 判断当前是否为已存在的自定义（通过 preset 解析为 id）
            let is_custom = settings
                .theme_preset
                .parse::<u64>()
                .ok()
                .and_then(|id| settings.custom_themes.iter().find(|c| c.id == id))
                .is_some();
            if is_custom {
                // 直接更新该自定义
                if let Some(ct) = settings
                    .custom_themes
                    .iter_mut()
                    .find(|c| c.id.to_string() == settings.theme_preset)
                {
                    ct.base = new_base;
                }
                settings.theme_base = new_base;
                crate::theme::set_theme(new_base);
                changed = true;
            } else {
                // 预设被改 → 新建自定义（自动命名）
                let next_id = settings
                    .custom_themes
                    .iter()
                    .map(|c| c.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let name = {
                    let locale = settings.locale.clone();
                    if locale.starts_with("zh") {
                        format!("自定义 {}", next_id)
                    } else if locale.starts_with("ja") {
                        format!("カスタム {}", next_id)
                    } else if locale.starts_with("ko") {
                        format!("커스텀 {}", next_id)
                    } else {
                        format!("Custom {}", next_id)
                    }
                };
                settings.custom_themes.push(CustomTheme {
                    id: next_id,
                    name,
                    base: new_base,
                });
                settings.theme_base = new_base;
                settings.theme_preset = next_id.to_string();
                crate::theme::set_theme(new_base);
                changed = true;
            }
        }
    }
    ui.add_space(8.0);

    // ── 构建统一主题列表（收藏置顶）──
    // id_str: 预设为 kebab 名，自定义为 id 字符串
    struct Item {
        id_str: String,
        base: BaseColors,
        display: String,
        is_custom: bool,
        custom_id: Option<u64>,
    }
    let mut all: Vec<Item> =
        Vec::with_capacity(settings.custom_themes.len() + BaseColors::PRESETS.len());
    for ct in &settings.custom_themes {
        all.push(Item {
            id_str: ct.id.to_string(),
            base: ct.base,
            display: ct.name.clone(),
            is_custom: true,
            custom_id: Some(ct.id),
        });
    }
    for (name, base) in BaseColors::PRESETS.iter() {
        let key = format!("settings.theme.{}", name.replace('-', "_"));
        let display = t!(key.as_str()).to_string();
        all.push(Item {
            id_str: (*name).to_string(),
            base: *base,
            display,
            is_custom: false,
            custom_id: None,
        });
    }
    // 按收藏分区：收藏的排前面，保持原相对顺序（稳定分区）
    let mut fav = Vec::new();
    let mut rest = Vec::new();
    for it in all {
        if settings.favorite_themes.contains(&it.id_str) {
            fav.push(it);
        } else {
            rest.push(it);
        }
    }
    let mut ordered = Vec::with_capacity(fav.len() + rest.len());
    ordered.extend(fav);
    ordered.extend(rest);
    let global_is_dark = settings.theme_base.is_dark();

    // ── 网格渲染（3 列）+ 星标 + 右键菜单（所有卡片按全局明暗统一显示为深/浅）──
    let mut to_apply: Option<(BaseColors, String)> = None;
    let mut to_toggle_fav: Option<String> = None;
    let mut to_copy: Option<(BaseColors, String)> = None;
    let mut to_delete: Option<u64> = None;
    let mut to_rename: Option<u64> = None;

    egui::Grid::new("theme_cards_grid")
        .num_columns(3)
        .spacing([12.0, 12.0])
        .show(ui, |ui| {
            let mut col = 0u32;
            for item in &ordered {
                let eff_base = if item.base.is_dark() == global_is_dark {
                    item.base
                } else {
                    item.base.inverted()
                };
                let is_selected =
                    settings.theme_preset == item.id_str && settings.theme_base == eff_base;
                let is_fav = settings.favorite_themes.contains(&item.id_str);
                // 渲染卡片（带星标）——按全局明暗统一显示
                let card_size = egui::vec2(158.0, 96.0);
                let (card_rect, card_resp) =
                    ui.allocate_exact_size(card_size, egui::Sense::click());
                let preview = derive_theme(eff_base);
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
                        &item.display,
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
                // 星标
                let star_rect = egui::Rect::from_min_max(
                    egui::pos2(card_rect.max.x - 24.0, card_rect.min.y + 5.0),
                    egui::pos2(card_rect.max.x - 6.0, card_rect.min.y + 23.0),
                );
                let star_id = ui.id().with(format!("star_{}", item.id_str));
                let star_resp = ui.interact(star_rect, star_id, egui::Sense::click());
                if ui.is_rect_visible(star_rect) {
                    let painter = ui.painter_at(star_rect);
                    if star_resp.hovered() {
                        painter.rect_filled(
                            star_rect,
                            egui::CornerRadius::same(4),
                            preview.control_bg.gamma_multiply(0.9),
                        );
                    }
                    let (icon, color) = if is_fav {
                        (
                            egui_material_icons::icons::ICON_STAR,
                            FIXED_WARNING.to_color32(),
                        )
                    } else {
                        (
                            egui_material_icons::icons::ICON_STAR_BORDER,
                            preview.text_secondary.gamma_multiply(0.55),
                        )
                    };
                    painter.text(
                        star_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icon.codepoint.to_string(),
                        egui::FontId::new(14.0, icon.font_family()),
                        color,
                    );
                }
                if star_resp.clicked() {
                    to_toggle_fav = Some(item.id_str.clone());
                }
                let card_clicked = card_resp.clicked() && !star_resp.clicked();
                if card_clicked {
                    to_apply = Some((eff_base, item.id_str.clone()));
                }
                // 右键菜单
                card_resp.context_menu(|ui| {
                    let is_fav_inner = settings.favorite_themes.contains(&item.id_str);
                    let fav_label = if is_fav_inner {
                        t!("settings.theme.unfavorite").to_string()
                    } else {
                        t!("settings.theme.favorite").to_string()
                    };
                    if ui.button(fav_label).clicked() {
                        to_toggle_fav = Some(item.id_str.clone());
                        ui.close();
                    }
                    if ui.button(t!("settings.theme.copy").to_string()).clicked() {
                        to_copy = Some((eff_base, item.display.clone()));
                        ui.close();
                    }
                    if item.is_custom {
                        if ui.button(t!("settings.theme.rename").to_string()).clicked() {
                            if let Some(cid) = item.custom_id {
                                to_rename = Some(cid);
                            }
                            ui.close();
                        }
                        if ui.button(t!("settings.theme.delete").to_string()).clicked() {
                            if let Some(cid) = item.custom_id {
                                to_delete = Some(cid);
                            }
                            ui.close();
                        }
                    }
                });
                // star 的右键也可触发同样菜单（复用）
                star_resp.context_menu(|ui| {
                    let is_fav_inner = settings.favorite_themes.contains(&item.id_str);
                    let fav_label = if is_fav_inner {
                        t!("settings.theme.unfavorite").to_string()
                    } else {
                        t!("settings.theme.favorite").to_string()
                    };
                    if ui.button(fav_label).clicked() {
                        to_toggle_fav = Some(item.id_str.clone());
                        ui.close();
                    }
                    if ui.button(t!("settings.theme.copy").to_string()).clicked() {
                        to_copy = Some((eff_base, item.display.clone()));
                        ui.close();
                    }
                    if item.is_custom {
                        if ui.button(t!("settings.theme.rename").to_string()).clicked() {
                            if let Some(cid) = item.custom_id {
                                to_rename = Some(cid);
                            }
                            ui.close();
                        }
                        if ui.button(t!("settings.theme.delete").to_string()).clicked() {
                            if let Some(cid) = item.custom_id {
                                to_delete = Some(cid);
                            }
                            ui.close();
                        }
                    }
                });

                col += 1;
                if col.is_multiple_of(3) {
                    ui.end_row();
                }
            }
            if !col.is_multiple_of(3) {
                ui.end_row();
            }
        });

    if let Some(id_str) = to_toggle_fav {
        if settings.favorite_themes.contains(&id_str) {
            settings.favorite_themes.retain(|s| s != &id_str);
        } else {
            settings.favorite_themes.push(id_str);
        }
        changed = true;
    }
    if let Some((base, id_str)) = to_apply {
        settings.theme_base = base;
        settings.theme_preset = id_str;
        crate::theme::set_theme(base);
        changed = true;
    }
    if let Some((base, display)) = to_copy {
        let next_id = settings
            .custom_themes
            .iter()
            .map(|c| c.id)
            .max()
            .unwrap_or(0)
            + 1;
        let suffix = if settings.locale.starts_with("zh") {
            " 副本"
        } else if settings.locale.starts_with("ja") {
            " コピー"
        } else if settings.locale.starts_with("ko") {
            " 복사"
        } else {
            " Copy"
        };
        let name = format!("{display}{suffix}");
        settings.custom_themes.push(CustomTheme {
            id: next_id,
            name: name.clone(),
            base,
        });
        // 复制后不自动切换，仅收藏排序会将其前置？保持当前选中不变
        changed = true;
    }
    if let Some(id) = to_delete {
        settings.custom_themes.retain(|c| c.id != id);
        settings.favorite_themes.retain(|s| s != &id.to_string());
        // 若删除的是当前选中，退回默认预设
        if settings.theme_preset == id.to_string() {
            settings.theme_base = BaseColors::DARK;
            settings.theme_preset = "ink-wash".to_string();
            crate::theme::set_theme(BaseColors::DARK);
        }
        changed = true;
    }
    if let Some(id) = to_rename
        && let Some(ct) = settings.custom_themes.iter().find(|c| c.id == id)
    {
        settings.rename_custom_id = Some(id);
        settings.rename_buffer = ct.name.clone();
    }

    // 重命名弹窗（仅自定义）
    if let Some(rid) = settings.rename_custom_id {
        let mut open = true;
        egui::Window::new(t!("settings.theme.rename_title").as_ref())
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.label(t!("settings.theme.custom_name").as_ref());
                let mut buf = settings.rename_buffer.clone();
                let resp = ui.text_edit_singleline(&mut buf);
                // 回写缓冲
                settings.rename_buffer = buf;
                ui.add_space(6.0);
                let mut do_ok = false;
                let mut do_cancel = false;
                ui.horizontal(|ui| {
                    if ui.button(t!("common.ok").as_ref()).clicked() {
                        do_ok = true;
                    }
                    if ui.button(t!("common.cancel").as_ref()).clicked() {
                        do_cancel = true;
                    }
                });
                // 回车直接确认
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_ok = true;
                }
                if do_ok {
                    let name = settings.rename_buffer.trim().to_string();
                    if !name.is_empty() {
                        if let Some(ct) = settings.custom_themes.iter_mut().find(|c| c.id == rid) {
                            ct.name = name.clone();
                            // 若当前正在使用该自定义，同步刷新显示
                            if settings.theme_preset == rid.to_string() {
                                // display 会自动更新
                            }
                        }
                        changed = true;
                    }
                    settings.rename_custom_id = None;
                    settings.rename_buffer.clear();
                } else if do_cancel {
                    settings.rename_custom_id = None;
                    settings.rename_buffer.clear();
                }
            });
        if !open && settings.rename_custom_id.is_some() {
            settings.rename_custom_id = None;
            settings.rename_buffer.clear();
        }
    }

    // 界面缩放：拖动中不缩放（缩放会让滑条自身位置来回跑），松手才应用
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
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
