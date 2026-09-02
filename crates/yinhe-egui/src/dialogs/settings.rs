use eframe::egui;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;

mod appearance;
mod audio;
mod constants;
mod editing;
mod general;
mod language;
mod render;
mod search;
mod shortcuts;
mod theme;

#[allow(unused_imports)]
pub use appearance::show_appearance_tab;
#[allow(unused_imports)]
pub use audio::show_audio_tab;
#[allow(unused_imports)]
pub use constants::{CATEGORY_KEYS, SETTING_ITEMS, SettingItem};
#[allow(unused_imports)]
pub use editing::show_editing_tab;
#[allow(unused_imports)]
pub use general::show_general_tab;
#[allow(unused_imports)]
pub use language::show_language_tab;
#[allow(unused_imports)]
pub use render::show_render_tab;
#[allow(unused_imports)]
pub use search::{item_matches, norm, show_search_results, to_search_keys};
#[allow(unused_imports)]
pub use shortcuts::show_shortcuts_tab;
#[allow(unused_imports)]
pub use theme::show_theme_tab;

/// Zed 风格设置行：标题+描述左，控件靠右，行间分割线。
pub(crate) fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().size(13.0));
            if !desc.is_empty() {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(desc)
                        .size(11.0)
                        .color(crate::theme::text_secondary()),
                );
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add_control(ui);
        });
    });
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
}

/// Show the settings dialog content inside an existing Ui.
/// Returns `true` if settings were changed.
pub fn show_content(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;

    // 左右侧栏等高，以整个窗口高度为基准
    let full_height = ui.available_height();
    ui.horizontal(|ui| {
        // ── 左侧：搜索框 + 分类导航（窄，独立滚动，撑满窗口高度） ──
        ui.vertical(|ui| {
            ui.set_width(132.0);
            ui.set_height(full_height);
            egui::ScrollArea::vertical()
                .id_salt("settings_left_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut settings.settings_search)
                            .hint_text(t!("settings.search_hint").as_ref())
                            .id_salt("settings_search")
                            .desired_width(132.0),
                    );
                    if !settings.settings_search.is_empty()
                        && ui.button(t!("settings.search_clear").as_ref()).clicked()
                    {
                        settings.settings_search.clear();
                    }
                    ui.add_space(8.0);

                    for (i, key) in CATEGORY_KEYS.iter().enumerate() {
                        let selected = settings.settings_tab == i;
                        if ui
                            .add(crate::widgets::menu::menu_item_button(
                                ui,
                                selected,
                                egui::RichText::new(t!(*key).as_ref()).size(
                                    crate::scaling::scaled_font(
                                        ui.ctx(),
                                        crate::theme::FILE_MENU_FONT,
                                    ),
                                ),
                            ))
                            .clicked()
                        {
                            settings.settings_tab = i;
                        }
                        ui.add_space(6.0);
                    }
                });
        });

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(8.0);

        // ── 右侧：搜索结果（搜索中）或当前分类内容（宽，独立滚动，撑满窗口高度） ──
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.set_height(full_height);
            egui::ScrollArea::vertical()
                .id_salt("settings_right_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    changed |= show_search_results(ui, settings, main_ctx);
                });
        });
    });

    changed
}

pub(crate) fn show_viewport(
    ctx: &eframe::egui::Context,
    settings: &mut AudioSettings,
    audio: &Option<yinhe_audio::CpalAudioHandle>,
) -> bool {
    let viewport_id = eframe::egui::ViewportId::from_hash_of("settings_dialog");
    if !settings.show_settings {
        return false;
    }

    let prev_xsynth_layers = settings.xsynth_layers;
    let prev_ui_scale = settings.ui_scale;
    let prev_font_scale = settings.font_scale;
    let settings_rc = std::rc::Rc::new(std::cell::RefCell::new(Some(std::mem::take(settings))));
    let ctx_clone = ctx.clone();
    let main_ctx = ctx.clone();
    let settings_cb = settings_rc.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("settings.title").as_ref(),
            [760.0, 620.0],
            true,
        ),
        move |vctx, _class| {
            let mut slot = settings_cb.borrow_mut().take();
            if let Some(ref mut s) = slot {
                let mut close = false;
                if vctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
                eframe::egui::CentralPanel::default()
                    .frame(eframe::egui::Frame {
                        fill: crate::theme::app_bg(),
                        ..Default::default()
                    })
                    .show(vctx, |ui| {
                        crate::chrome::dialog::title_bar(
                            ui,
                            t!("settings.title").as_ref(),
                            &mut close,
                        );
                        eframe::egui::Frame::new()
                            .inner_margin(eframe::egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                let changed = show_content(ui, s, &main_ctx);
                                if changed {
                                    s.save();
                                }
                            });
                    });
                if close {
                    vctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
                    s.show_settings = false;
                    s.shortcut_recording = false;
                }
            }
            *settings_cb.borrow_mut() = slot;
        },
    );

    if let Some(s) = std::rc::Rc::into_inner(settings_rc).and_then(|rc| rc.into_inner()) {
        *settings = s;
        if settings.xsynth_layers != prev_xsynth_layers
            && let Some(audio) = audio
        {
            let count = if settings.xsynth_layers == 0 {
                None
            } else {
                Some(settings.xsynth_layers as usize)
            };
            audio
                .handle
                .send(yinhe_audio::AudioCommand::SetLayerCount { count });
        }
        if (settings.ui_scale - prev_ui_scale).abs() > f32::EPSILON {
            crate::scaling::apply_ui_scale(ctx, settings.ui_scale);
        }
        if (settings.font_scale - prev_font_scale).abs() > f32::EPSILON {
            crate::scaling::apply_font_scale(ctx, settings.font_scale);
        }
        !settings.show_settings
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(zh: &str) -> &'static SettingItem {
        SETTING_ITEMS
            .iter()
            .find(|i| i.zh == zh)
            .expect("settings item exists")
    }

    #[test]
    fn search_matches_original_names_of_all_languages() {
        assert!(item_matches(item("主题预设"), "主题"));
        assert!(item_matches(item("主题预设"), "Theme"));
        assert!(item_matches(item("主题预设"), "プリセット"));
        assert!(item_matches(item("主题预设"), "프리셋"));
        assert!(item_matches(item("背景"), "배경"));
    }

    #[test]
    fn search_matches_pinyin_and_initials() {
        assert!(item_matches(item("主题预设"), "zhuti"));
        assert!(item_matches(item("主题预设"), "zhu ti"));
        assert!(item_matches(item("主题预设"), "zt"));
        assert!(item_matches(item("缓冲区大小"), "huanchongqu"));
        assert!(!item_matches(item("主题预设"), "xyz"));
    }

    #[test]
    fn search_is_case_insensitive() {
        assert!(item_matches(item("采样率"), "SAMPLE"));
        assert!(item_matches(item("采样率"), "caiyanglv"));
    }

    #[test]
    fn shortcut_rows_align() {
        let mut first_x = 0.0f32;
        let mut second_x = 0.0f32;
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let output = ctx.run_ui(Default::default(), |ui| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [150.0, 24.0],
                    egui::Label::new(egui::RichText::new("保存"))
                        .selectable(false)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
                let resp = ui.add(egui::Button::new("⌘S").min_size(egui::vec2(140.0, 24.0)));
                first_x = resp.rect.min.x;
            });
            ui.horizontal(|ui| {
                ui.allocate_exact_size(egui::vec2(150.0, 24.0), egui::Sense::hover());
                let resp = ui.add(egui::Button::new("⌘S").min_size(egui::vec2(140.0, 24.0)));
                second_x = resp.rect.min.x;
            });
        });
        output.drop_without_applying_deltas();
        assert!(
            (first_x - second_x).abs() < 0.5,
            "快捷键两行未对齐：第一行 x={first_x}，第二行 x={second_x}"
        );
    }
}
