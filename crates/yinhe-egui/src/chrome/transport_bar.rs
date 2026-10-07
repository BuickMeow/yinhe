use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::widgets::action_menu::pinned_action_buttons;
use crate::widgets::timecode::show_timecode_display;

use super::transport_bar_actions::{PlayActions, PlayMenuAction};
use super::transport_bar_menus::{show_edit_menu, show_file_menu, show_play_menu};
use super::transport_bar_pr;

pub use super::transport_bar_actions::{
    EDIT_GROUPS, EditAction, FILE_GROUPS, FileAction, TransportContext, TransportResponse,
};
pub use super::transport_bar_pr::PrBarData;
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
    let mut set_orientation = None;
    let mut timecode_events: Vec<crate::widgets::timecode::TimecodeEvent> = Vec::new();

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

                let file_btn = menu_button(ui, "file_menu", ICON_DESCRIPTION, btn_size);
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

                let edit_btn = menu_button(ui, "edit_menu", ICON_EDIT_SQUARE, btn_size);
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
                let play_menu_btn = menu_button(ui, "play_menu", ICON_PLAY_CIRCLE, btn_size);
                if play_menu_btn.hovered() {
                    hovered_hint = Some(t!("hint.play_menu").to_string());
                }
                show_play_menu(&play_menu_btn, ctx, is_playing, &mut play_actions);

                let play_btn_actions: [PlayMenuAction; 6] = [
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
                    PlayMenuAction::TapTempo,
                    PlayMenuAction::AutomationWrite {
                        enabled: ctx.settings.automation_write,
                    },
                ];
                let play_btn_pins = [
                    ctx.settings.pinned_play_pause,
                    ctx.settings.pinned_stop,
                    ctx.settings.pinned_record,
                    ctx.settings.pinned_step_input,
                    ctx.settings.pinned_tap_tempo,
                    ctx.settings.pinned_automation_write,
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
                        PlayMenuAction::TapTempo => play_actions.tap_tempo = true,
                        PlayMenuAction::AutomationWrite { enabled } => {
                            play_actions.set_automation_write = Some(!enabled);
                        }
                        PlayMenuAction::Follow(..) => unreachable!("跟随档无图钉"),
                    }
                }

                // ── 工具区已迁移到左侧工具栏（见 chrome::tool_bar）──

                if let Some(doc) = ctx.doc {
                    // ── 时间码（三列居中：BPM/拍号+PPQ | 位置/时间 | 量化/调式）──
                    if let Some(pr) = ctx.pr.as_ref() {
                        let quantize = if ctx.focus_is_pianoroll {
                            pr.quantize
                        } else {
                            ctx.quantize_arrange
                        };
                        let (tc_rect, tc_events) = show_timecode_display(
                            ui,
                            crate::widgets::timecode::TimecodeData {
                                doc,
                                quantize,
                                quantize_is_pr: ctx.focus_is_pianoroll,
                            },
                        );
                        timecode_rect = Some(tc_rect);
                        timecode_events.extend(tc_events);
                    }
                    ui.add_space(4.0);

                    if let Some(pr) = ctx.pr.as_ref() {
                        // ── 右侧 PR 控制组（靠右，永不收缩）：III / 三 / 和弦 ──
                        transport_bar_pr::show_right_group(
                            ui,
                            pr,
                            ctx.orientation_vertical,
                            &mut set_orientation,
                            &mut hovered_hint,
                        );
                    }
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

    // 「写入自动化」开关：直接落 settings（无需经 response 往返）。
    if let Some(enabled) = play_actions.set_automation_write {
        ctx.settings.automation_write = enabled;
        ctx.settings.save();
    }

    TransportResponse {
        toggle_play: play_actions.toggle_play,
        pause_return: play_actions.pause_return,
        stop_play: play_actions.stop_play,
        record_toggle: play_actions.record,
        step_toggle: play_actions.step,
        tap_tempo: play_actions.tap_tempo,
        set_orientation,
        timecode_events,
        pending_file_action,
        pending_edit_action,
        pending_open_path,
    }
}

/// 在 `rect` 右下角画一个朝下的实心小三角（菜单标识）。
/// 边长随控件缩放；不接触 rect 填充，仅叠加前景。
fn draw_corner_triangle(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let s = (rect.height() * 0.16).clamp(3.0, 6.0);
    let pad = s * 0.6;
    let br = egui::pos2(rect.max.x - pad, rect.max.y - pad);
    let p = [
        egui::pos2(br.x - s, br.y),
        egui::pos2(br.x, br.y),
        egui::pos2(br.x, br.y - s),
    ];
    painter.add(egui::Shape::convex_polygon(
        p.to_vec(),
        color,
        egui::Stroke::NONE,
    ));
}

fn menu_button(
    ui: &mut egui::Ui,
    id: &str,
    icon: egui_material_icons::MaterialIcon,
    btn_size: egui::Vec2,
) -> egui::Response {
    let resp = ui
        .push_id(id, |ui| {
            crate::widgets::flat::flat_button_filled(
                ui,
                icon.rich_text()
                    .size(crate::theme::TRANSPORT_BTN_FONT)
                    .color(crate::theme::text_primary()),
                btn_size,
                None,
                true,
            )
        })
        .inner;
    // 右下角三角：颜色取按钮此刻的图标色（hover/按下与图标一致），提示这是下拉菜单。
    let tri_color =
        crate::widgets::hover::hover_button_color(&resp, crate::theme::text_primary(), false);
    draw_corner_triangle(ui.painter(), resp.rect, tri_color);
    resp
}
