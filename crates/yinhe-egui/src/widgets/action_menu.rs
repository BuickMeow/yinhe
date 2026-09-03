use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

/// 菜单 popup 行的统一接口
pub trait PopupRow: Copy {
    fn pinned_index(self) -> usize;
    fn action_id(self) -> &'static str;
    fn icon(self) -> egui_material_icons::MaterialIcon;
    fn label_key(self) -> &'static str;
    fn is_enabled(self, has_active: bool, loading: bool) -> bool;
    fn has_pin(self) -> bool {
        true
    }
    fn icon_accent(self) -> Option<egui::Color32> {
        None
    }
    fn is_selected(self) -> bool {
        false
    }
}

pub struct PopupRowSpec<'a> {
    pub icon: egui_material_icons::MaterialIcon,
    pub label: &'a str,
    pub shortcut: Option<&'a str>,
    pub enabled: bool,
    pub selected: bool,
    pub accent: Option<egui::Color32>,
    pub pin: Option<bool>,
    pub pin_index: Option<usize>,
    pub chevron: bool,
}

pub const PIN_W: f32 = 26.0;
pub const MAIN_PIN_GAP: f32 = 2.0;

pub fn popup_menu_row(
    ui: &mut egui::Ui,
    spec: PopupRowSpec<'_>,
) -> (egui::Response, Option<egui::Response>) {
    let row_h = 24.0;
    let row_w = ui.available_width();
    let (row_rect, _) = ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::hover());

    let has_pin = spec.pin.is_some();
    let main_w = if has_pin {
        row_w - PIN_W - MAIN_PIN_GAP
    } else {
        row_w
    };
    let main_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(main_w, row_h));
    let icon_color = spec.accent.unwrap_or_else(|| {
        if spec.enabled {
            crate::theme::text_bright()
        } else {
            crate::theme::text_disabled()
        }
    });
    let main_btn = crate::widgets::menu::menu_item_button(
        ui,
        spec.selected,
        crate::widgets::icon_text::icon_text(
            spec.icon,
            spec.label,
            crate::theme::FILE_MENU_FONT,
            icon_color,
        ),
    )
    .min_size(egui::vec2(main_w, 0.0))
    .wrap_mode(egui::TextWrapMode::Truncate)
    .shortcut_text(if spec.chevron {
        ICON_CHEVRON_RIGHT
            .rich_text()
            .size(crate::theme::FILE_MENU_FONT)
    } else {
        egui::RichText::new(spec.shortcut.unwrap_or(""))
    });
    let main_resp = ui.put(main_rect, main_btn);

    let mut pin_resp = None;
    if let Some(is_pinned) = spec.pin {
        let pin_rect = egui::Rect::from_min_size(
            egui::pos2(row_rect.max.x - PIN_W, row_rect.min.y),
            egui::vec2(PIN_W, row_h),
        );
        // 无背景，仅前景三态（与 mode_bar hover_button 一致），用 pin_index 固定 Id 避免错位
        let pin_id = if let Some(idx) = spec.pin_index {
            ui.id().with(("pin", idx))
        } else {
            ui.id().with("pin")
        };
        let resp = ui.interact(pin_rect, pin_id, egui::Sense::click());
        let pin_color = if is_pinned {
            crate::theme::accent_active()
        } else if resp.hovered() {
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_disabled()
        };
        ui.painter().text(
            pin_rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_KEEP.codepoint.to_string(),
            egui::FontId::new(crate::theme::FILE_MENU_FONT, ICON_KEEP.font_family()),
            pin_color,
        );
        pin_resp = Some(resp);
    }
    (main_resp, pin_resp)
}

pub fn measure_menu_width<T: PopupRow>(
    ctx: &egui::Context,
    groups: &[&[T]],
    keybindings: &yinhe_editor_core::shortcuts::Keybindings,
) -> f32 {
    let spacing = &ctx.style_of(ctx.theme()).spacing;
    let pad_x = spacing.button_padding.x * 2.0;
    let shortcut_gap = spacing.item_spacing.x;
    let mut max_content = 0.0f32;
    for group in groups {
        for &action in *group {
            let label = t!(action.label_key());
            let shortcut = keybindings
                .get(action.action_id())
                .first()
                .map(crate::shortcuts::display_combo)
                .unwrap_or_default();
            let job = crate::widgets::icon_text::icon_text(
                action.icon(),
                label.as_ref(),
                crate::theme::FILE_MENU_FONT,
                egui::Color32::WHITE,
            );
            let content_w = ctx.fonts_mut(|f| {
                let icon_label_w = f.layout_job(job).size().x;
                let shortcut_w = if shortcut.is_empty() {
                    0.0
                } else {
                    f.layout_no_wrap(
                        shortcut.clone(),
                        egui::FontId::proportional(crate::theme::FILE_MENU_FONT),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
                };
                icon_label_w + shortcut_w
            });
            max_content = max_content.max(content_w + pad_x + shortcut_gap);
        }
    }
    let has_pin = groups.iter().copied().flatten().any(|a| a.has_pin());
    max_content + if has_pin { PIN_W + MAIN_PIN_GAP } else { 0.0 }
}

pub struct ActionMenuOutcome {
    pub pinned_changed: bool,
    pub popup_open: bool,
}

pub struct ActionMenuExtra<'a> {
    pub after_group: usize,
    pub min_width: f32,
    pub render: &'a mut dyn FnMut(&mut egui::Ui, bool),
}

#[allow(clippy::too_many_arguments)]
pub fn show_action_menu<T: PopupRow>(
    button: &egui::Response,
    groups: &[&[T]],
    has_active: bool,
    loading: bool,
    keybindings: &yinhe_editor_core::shortcuts::Keybindings,
    pinned: Option<&mut [bool]>,
    pending_action: &mut Option<T>,
    extra: Option<ActionMenuExtra<'_>>,
) -> ActionMenuOutcome {
    let extra_min_width = extra.as_ref().map(|e| e.min_width).unwrap_or(0.0);
    let menu_w = measure_menu_width(&button.ctx, groups, keybindings).max(extra_min_width);
    let mut pinned_changed = false;
    let mut pin_toggled: Option<usize> = None;
    let popup_response = egui::Popup::from_toggle_button_response(button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(menu_w)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_min_width(menu_w);
            ui.set_max_width(menu_w);
            let hover_id = button.id.with("menu_extra_hover");
            let prev_row_hovered: bool =
                ui.ctx().data_mut(|d| d.get_temp(hover_id)).unwrap_or(false);
            let mut any_row_hovered = false;
            let mut extra = extra;
            for (gi, group) in groups.iter().enumerate() {
                if gi > 0 {
                    ui.separator();
                }
                for &action in *group {
                    let enabled = action.is_enabled(has_active, loading);
                    let is_pinned = pinned
                        .as_ref()
                        .is_some_and(|p| p.get(action.pinned_index()).copied().unwrap_or(false));
                    let shortcut = keybindings
                        .get(action.action_id())
                        .first()
                        .map(crate::shortcuts::display_combo);
                    let (main_resp, pin_resp) = popup_menu_row(
                        ui,
                        PopupRowSpec {
                            icon: action.icon(),
                            label: &t!(action.label_key()),
                            shortcut: shortcut.as_deref(),
                            enabled,
                            selected: action.is_selected(),
                            accent: action.icon_accent(),
                            pin: if action.has_pin() {
                                Some(is_pinned)
                            } else {
                                None
                            },
                            pin_index: action.has_pin().then_some(action.pinned_index()),
                            chevron: false,
                        },
                    );
                    if main_resp.hovered() || pin_resp.as_ref().is_some_and(|r| r.hovered()) {
                        any_row_hovered = true;
                    }
                    if enabled && main_resp.clicked() {
                        *pending_action = Some(action);
                        ui.close();
                    }
                    if pin_resp.is_some_and(|r| r.clicked()) {
                        pin_toggled = Some(action.pinned_index());
                    }
                }
                if let Some(extra) = extra.take()
                    && gi == extra.after_group
                {
                    (extra.render)(ui, prev_row_hovered);
                }
            }
            ui.ctx()
                .data_mut(|d| d.insert_temp(hover_id, any_row_hovered));
        });
    if let Some(idx) = pin_toggled
        && let Some(p) = pinned
    {
        if let Some(v) = p.get_mut(idx) {
            *v = !*v;
        }
        pinned_changed = true;
    }
    ActionMenuOutcome {
        pinned_changed,
        popup_open: popup_response.is_some(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn pinned_action_buttons<T: PopupRow>(
    ui: &mut egui::Ui,
    id_prefix: &str,
    actions: &[T],
    pinned: &[bool],
    has_active: bool,
    loading: bool,
    hovered_hint: &mut Option<String>,
    pending: &mut Option<T>,
) {
    let btn_size = egui::vec2(
        crate::theme::TRANSPORT_BTN_SIZE,
        crate::theme::TRANSPORT_BTN_SIZE,
    );
    let btn_rounding = egui::CornerRadius::same(2);
    for (idx, &action) in actions.iter().enumerate() {
        if !pinned.get(idx).copied().unwrap_or(false) {
            continue;
        }
        let enabled = action.is_enabled(has_active, loading);
        let icon = action.icon();
        let color = action.icon_accent().unwrap_or_else(|| {
            if enabled {
                crate::theme::text_primary()
            } else {
                crate::theme::text_disabled()
            }
        });
        let selected = action.is_selected();
        let resp = ui
            .push_id((id_prefix, action.pinned_index()), |ui| {
                let mut btn = egui::Button::new(
                    icon.rich_text()
                        .size(crate::theme::TRANSPORT_BTN_FONT)
                        .color(color),
                )
                .min_size(btn_size)
                .corner_radius(btn_rounding);
                if selected {
                    let sel_bg = action
                        .icon_accent()
                        .map(|c| c.gamma_multiply(0.18))
                        .unwrap_or(crate::theme::selected_bg());
                    btn = btn.fill(sel_bg);
                }
                ui.add_enabled(enabled, btn)
            })
            .inner;
        if resp.clicked() {
            *pending = Some(action);
        }
        if resp.hovered() {
            *hovered_hint = Some(t!(action.label_key()).to_string());
        }
    }
}
