use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::widgets::action_menu::pinned_action_buttons;
use crate::widgets::timecode::show_timecode_display;
use crate::widgets::tools_panel::ALL_TOOLS;

use super::transport_bar_actions::{PlayActions, PlayMenuAction, tool_hint};
use super::transport_bar_menus::{show_edit_menu, show_file_menu, show_play_menu};

pub use super::transport_bar_actions::{
    EDIT_GROUPS, EditAction, FILE_GROUPS, FileAction, TransportContext, TransportResponse,
};
pub(crate) use super::transport_bar_recent::recent_display_name;
pub use crate::widgets::action_menu::PopupRow;

#[cfg(test)]
#[path = "transport_bar_tests.rs"]
mod tests;

pub fn show(ui: &mut egui::Ui, ctx: &mut TransportContext<'_>) -> TransportResponse {
    let has_active = ctx.doc.is_some();

    let mut play_actions = PlayActions::default();
    let mut pending_file_action = None;
    let mut pending_edit_action = None;
    let mut pending_open_path = None;
    let mut toggle_orientation = false;

    egui::Panel::top("transport_bar")
        .frame(egui::Frame {
            fill: crate::theme::app_bg(),
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 0,
                bottom: 8,
            },
            stroke: egui::Stroke::NONE,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 32.0;

            let mut timecode_rect: Option<egui::Rect> = None;
            let mut hovered_hint: Option<String> = None;

            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let btn_size = egui::vec2(
                    crate::theme::TRANSPORT_BTN_SIZE,
                    crate::theme::TRANSPORT_BTN_SIZE,
                );
                let btn_rounding = egui::CornerRadius::same(2);

                let file_btn =
                    menu_button(ui, "file_menu", ICON_DESCRIPTION, btn_size, btn_rounding);
                if file_btn.hovered() {
                    let m = crate::chrome::mode_bar::mod_key();
                    hovered_hint = Some(format!("{} ({}N/{}O/{}S)", t!("hint.file_menu"), m, m, m));
                }
                show_file_menu(
                    &file_btn,
                    ctx.file_loader,
                    has_active,
                    ctx.settings,
                    &mut pending_file_action,
                    &mut pending_open_path,
                );

                pinned_action_buttons(
                    ui,
                    "pinned_file",
                    &FileAction::ALL,
                    &ctx.settings.pinned_file_actions,
                    has_active,
                    ctx.file_loader.is_loading(),
                    &mut hovered_hint,
                    &mut pending_file_action,
                );

                let edit_btn =
                    menu_button(ui, "edit_menu", ICON_EDIT_SQUARE, btn_size, btn_rounding);
                if edit_btn.hovered() {
                    hovered_hint = Some(t!("hint.edit_menu").to_string());
                }
                show_edit_menu(
                    &edit_btn,
                    has_active,
                    ctx.settings,
                    &mut pending_edit_action,
                );

                pinned_action_buttons(
                    ui,
                    "pinned_edit",
                    &EditAction::ALL,
                    &ctx.settings.pinned_edit_actions,
                    has_active,
                    false,
                    &mut hovered_hint,
                    &mut pending_edit_action,
                );

                let is_playing = ctx
                    .doc
                    .map(|d| d.edit.playback.is_playing())
                    .unwrap_or(false);
                let play_menu_btn =
                    menu_button(ui, "play_menu", ICON_PLAY_CIRCLE, btn_size, btn_rounding);
                if play_menu_btn.hovered() {
                    hovered_hint = Some(t!("hint.play_menu").to_string());
                }
                show_play_menu(&play_menu_btn, ctx, is_playing, &mut play_actions);

                let play_btn_actions: [PlayMenuAction; 4] = [
                    PlayMenuAction::PlayPause {
                        playing: is_playing,
                    },
                    PlayMenuAction::Stop,
                    PlayMenuAction::Record {
                        recording: ctx.is_recording,
                    },
                    PlayMenuAction::StepInput {
                        active: ctx.step_input,
                    },
                ];
                let play_btn_pins = [
                    ctx.settings.pinned_play_pause,
                    ctx.settings.pinned_stop,
                    ctx.settings.pinned_record,
                    ctx.settings.pinned_step_input,
                ];
                let mut pending_play: Option<PlayMenuAction> = None;
                pinned_action_buttons(
                    ui,
                    "pinned_play",
                    &play_btn_actions,
                    &play_btn_pins,
                    has_active,
                    false,
                    &mut hovered_hint,
                    &mut pending_play,
                );
                if let Some(action) = pending_play {
                    match action {
                        PlayMenuAction::PlayPause { playing } => {
                            if playing {
                                play_actions.pause_return = true;
                            } else {
                                play_actions.toggle_play = true;
                            }
                        }
                        PlayMenuAction::Stop => play_actions.stop_play = true,
                        PlayMenuAction::Record { .. } => play_actions.record = true,
                        PlayMenuAction::StepInput { .. } => play_actions.step = true,
                        PlayMenuAction::Follow(..) => unreachable!("跟随档无图钉"),
                    }
                }

                if let Some(doc) = ctx.doc {
                    timecode_rect = Some(show_timecode_display(ui, doc));

                    ui.add_space(4.0);
                    for tool in ALL_TOOLS {
                        let is_active = *ctx.active_tool == tool;
                        let icon = tool.icon();
                        let resp = crate::widgets::hover::hover_button(
                            ui,
                            icon.codepoint,
                            egui::FontId::new(crate::theme::TRANSPORT_BTN_FONT, icon.font_family()),
                            crate::theme::text_label(),
                            is_active,
                        );
                        if resp.clicked() {
                            *ctx.active_tool = tool;
                        }
                        if resp.hovered() {
                            hovered_hint = Some(tool_hint(tool));
                        }
                        ui.add_space(2.0);
                    }

                    ui.add_space(4.0);
                    use egui_material_icons::icons::ICON_DEHAZE;
                    let orientation_icon = ICON_DEHAZE;
                    let ori_font = egui::FontId::new(
                        crate::theme::TRANSPORT_BTN_FONT,
                        orientation_icon.font_family(),
                    );
                    let is_vertical = *ctx.orientation == yinhe_types::Orientation::Vertical;
                    let ori_resp = if is_vertical {
                        crate::widgets::hover::hover_button_rotated(
                            ui,
                            orientation_icon.codepoint,
                            ori_font,
                            crate::theme::text_label(),
                            true,
                            std::f32::consts::FRAC_PI_2,
                        )
                    } else {
                        crate::widgets::hover::hover_button(
                            ui,
                            orientation_icon.codepoint,
                            ori_font,
                            crate::theme::text_label(),
                            true,
                        )
                    };
                    if ori_resp.clicked() {
                        toggle_orientation = true;
                    }
                    if ori_resp.hovered() {
                        hovered_hint = Some(if is_vertical {
                            t!("hint.orientation.vertical").to_string()
                        } else {
                            t!("hint.orientation.horizontal").to_string()
                        });
                    }
                    ui.add_space(2.0);
                }
            });

            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
            if pointer_pos.is_some_and(|p| timecode_rect.is_some_and(|r| r.contains(p))) {
                hovered_hint = Some(t!("hint.timecode").to_string());
            }
            let bar_rect = ui.max_rect();
            if let Some(hint) = hovered_hint {
                *ctx.status_hint = Some(hint);
            } else if pointer_pos.is_some_and(|p| bar_rect.contains(p)) {
                *ctx.status_hint = None;
            }

            const DOUBLE_CLICK_MS: f64 = 400.0;
            let dbl_id = ui.id().with("transport_bar_dbl_click");
            if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            {
                let bar_rect = ui.max_rect();
                let in_bar = bar_rect.contains(pos);
                let in_timecode = timecode_rect
                    .map(|r: egui::Rect| r.contains(pos))
                    .unwrap_or(false);
                let clicked_blank = ui.ctx().interaction_snapshot(|w| w.clicked.is_none());
                if in_bar && !in_timecode && clicked_blank {
                    let now = ui.input(|i| i.time);
                    let last_click: f64 = ui.data_mut(|d| d.get_persisted(dbl_id)).unwrap_or(0.0);
                    if now - last_click < DOUBLE_CLICK_MS / 1000.0 {
                        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        ui.data_mut(|d| d.insert_persisted(dbl_id, 0.0));
                    } else {
                        ui.data_mut(|d| d.insert_persisted(dbl_id, now));
                    }
                }
            }

            let bar_rect = ui.max_rect();
            let drag_id = ui.id().with("tb_drag_started");
            let blank_id = ui.id().with("tb_drag_blank");
            let mut drag_started: bool = ui.data_mut(|d| d.get_temp(drag_id)).unwrap_or(false);

            if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
                && let Some(pos) = ui.input(|i| i.pointer.press_origin())
            {
                let in_bar = bar_rect.contains(pos);
                let in_timecode = timecode_rect
                    .map(|r: egui::Rect| r.contains(pos))
                    .unwrap_or(false);
                let pressed_blank = in_bar && !in_timecode && !ui.ctx().egui_wants_pointer_input();
                ui.data_mut(|d| d.insert_temp(blank_id, pressed_blank));
            }

            if ui.input(|i| i.pointer.primary_down()) {
                if !drag_started && ui.data_mut(|d| d.get_temp(blank_id)).unwrap_or(false) {
                    let moved_past_click_dist = ui.input(|i| {
                        let (hover, origin) = (i.pointer.hover_pos(), i.pointer.press_origin());
                        hover.is_some_and(|p| {
                            origin.is_some_and(|o| {
                                p.distance(o) >= egui::InputOptions::default().max_click_dist
                            })
                        })
                    });
                    if moved_past_click_dist {
                        drag_started = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
            } else {
                drag_started = false;
                ui.data_mut(|d| d.insert_temp(blank_id, false));
            }

            ui.data_mut(|d| d.insert_temp(drag_id, drag_started));
        });

    TransportResponse {
        toggle_play: play_actions.toggle_play,
        pause_return: play_actions.pause_return,
        stop_play: play_actions.stop_play,
        record_toggle: play_actions.record,
        step_toggle: play_actions.step,
        toggle_orientation,
        pending_file_action,
        pending_edit_action,
        pending_open_path,
    }
}

fn menu_button(
    ui: &mut egui::Ui,
    id: &str,
    icon: egui_material_icons::MaterialIcon,
    btn_size: egui::Vec2,
    btn_rounding: egui::CornerRadius,
) -> egui::Response {
    ui.push_id(id, |ui| {
        ui.add(
            egui::Button::new(
                icon.rich_text()
                    .size(crate::theme::TRANSPORT_BTN_FONT)
                    .color(crate::theme::text_primary()),
            )
            .min_size(btn_size)
            .corner_radius(btn_rounding),
        )
    })
    .inner
}
