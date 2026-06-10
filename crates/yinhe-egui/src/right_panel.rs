pub mod config;
pub mod info_panel;
pub mod project_info;
pub mod sf_list;
pub mod soundbank;

use eframe::egui;

use crate::dialogs::settings::AudioSettings;
use crate::document::Document;

#[derive(PartialEq, Clone, Copy)]
pub enum RightTab {
    Info,
    SoundBank,
    Project,
}

/// Render the right panel (if a tab is active).
///
/// `rect` is the full area reserved for the right panel, including a 4px
/// split-handle strip at its left edge.  Returns `true` if the audio engine
/// needs to be reloaded (soundfont config changed).
pub fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    right_panel_width: &mut f32,
    right_tab: &mut Option<RightTab>,
    audio_settings: &mut AudioSettings,
    doc: Option<&mut Document>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
) -> bool {
    let tab = *right_tab;
    if tab.is_none() {
        return false;
    }

    let theme = crate::widgets::theme::RIGHT_PANEL_MIN_WIDTH;
    let total_avail = ui.available_rect_before_wrap().width();
    let max_w = (total_avail - 60.0).max(theme + 4.0);
    let clamp_w = (*right_panel_width + 4.0).clamp(theme + 4.0, max_w);
    *right_panel_width = (clamp_w - 4.0).max(theme);

    // ── Split handle (4px at the left edge) ──
    let handle_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y),
        egui::pos2(rect.min.x + 4.0, rect.max.y),
    );
    let resp = crate::widgets::split_handle::vertical(ui, "__right_split__", handle_rect);
    if resp.dragged() {
        *right_panel_width = (*right_panel_width + resp.drag_delta().x).clamp(theme, max_w - 4.0);
    }

    // ── Panel content area (after the handle) ──
    let content_rect = egui::Rect::from_min_max(egui::pos2(rect.min.x + 4.0, rect.min.y), rect.max);

    let mut changed = false;

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
        ui.set_clip_rect(content_rect);

        // Background
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, crate::widgets::theme::APP_BG);

        // ── Title bar ──
        let close_clicked = ui
            .horizontal(|ui| {
                let title = match tab.unwrap() {
                    RightTab::Info => "信息",
                    RightTab::SoundBank => "音色库",
                    RightTab::Project => "项目信息",
                };
                ui.label(egui::RichText::new(title).size(13.0).strong());
                let mut clicked = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        clicked = true;
                    }
                });
                clicked
            })
            .inner;
        if close_clicked {
            *right_tab = None;
        }

        ui.separator();

        // ── Content ──
        match tab.unwrap() {
            RightTab::Info => {
                info_panel::show(ui, doc, audio);
            }
            RightTab::SoundBank => {
                changed |= soundbank::show(ui, audio_settings, doc);
            }
            RightTab::Project => {
                project_info::show(ui, doc);
            }
        }
    });

    changed
}
