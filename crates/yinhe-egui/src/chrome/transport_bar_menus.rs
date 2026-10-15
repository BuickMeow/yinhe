use crate::audio_settings::AudioSettings;
use crate::file_loader::FileLoader;
use crate::widgets::action_menu::{
    ActionMenuExtra, PopupRow, PopupRowSpec, measure_menu_width, popup_menu_row, show_action_menu,
};
use eframe::egui;
use rust_i18n::t;

use super::transport_bar_actions::{
    EDIT_GROUPS, EditAction, FILE_GROUPS, FileAction, PlayActions, PlayMenuAction, VIEW_GROUPS,
};
use super::transport_bar_recent::{recent_files_section, recent_parent_width};
use crate::view_interaction::FollowMode;

use super::mode_bar::PanelKind;

const RECENT_SUBMENU_OPEN_ID: &str = "recent_files_submenu_open";

pub fn show_file_menu(
    button: &egui::Response,
    file_loader: &FileLoader,
    has_active: bool,
    settings: &mut AudioSettings,
    pending_action: &mut Option<FileAction>,
    pending_open_path: &mut Option<String>,
) {
    let keybindings = &settings.keybindings;
    let pinned = &mut settings.pinned_file_actions;
    let recent = &settings.recent_files;
    let recent_times = &settings.recent_file_times;

    let mut render = |ui: &mut egui::Ui, any_row_hovered: bool| {
        recent_files_section(ui, recent, recent_times, any_row_hovered, pending_open_path);
    };
    let has_recent = !recent.is_empty();
    let outcome = show_action_menu(
        button,
        &FILE_GROUPS,
        has_active,
        file_loader.is_loading(),
        keybindings,
        Some(pinned),
        pending_action,
        has_recent.then(|| ActionMenuExtra {
            after_group: 0,
            min_width: recent_parent_width(&button.ctx),
            render: &mut render,
        }),
    );
    if !outcome.popup_open {
        button
            .ctx
            .data_mut(|d| d.remove::<bool>(egui::Id::new(RECENT_SUBMENU_OPEN_ID)));
    }
    if outcome.pinned_changed {
        settings.save();
    }
}

pub fn show_edit_menu(
    button: &egui::Response,
    has_active: bool,
    settings: &mut AudioSettings,
    pending_action: &mut Option<EditAction>,
) {
    let keybindings = &settings.keybindings;
    let pinned = &mut settings.pinned_edit_actions;
    if show_action_menu(
        button,
        &EDIT_GROUPS,
        has_active,
        false,
        keybindings,
        Some(pinned),
        pending_action,
        None,
    )
    .pinned_changed
    {
        settings.save();
    }
}

pub fn show_play_menu(
    button: &egui::Response,
    ctx: &mut super::transport_bar_actions::TransportContext<'_>,
    is_playing: bool,
    actions: &mut PlayActions,
) {
    let has_active = ctx.doc.is_some();
    let follow_mode = &mut ctx.follow_mode;
    let settings = &mut ctx.settings;
    let groups: [&[PlayMenuAction]; 2] = [
        &[
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
                enabled: settings.automation_write,
            },
        ],
        &[
            PlayMenuAction::Follow(FollowMode::None, **follow_mode == FollowMode::None),
            PlayMenuAction::Follow(FollowMode::Centered, **follow_mode == FollowMode::Centered),
            PlayMenuAction::Follow(FollowMode::Page, **follow_mode == FollowMode::Page),
            PlayMenuAction::Follow(
                FollowMode::Continuous,
                **follow_mode == FollowMode::Continuous,
            ),
        ],
    ];
    let mut pending = None;
    let mut pinned = [
        settings.pinned_play_pause,
        settings.pinned_stop,
        settings.pinned_record,
        settings.pinned_step_input,
        settings.pinned_tap_tempo,
        settings.pinned_automation_write,
    ];
    if show_action_menu(
        button,
        &groups,
        has_active,
        false,
        &settings.keybindings,
        Some(&mut pinned),
        &mut pending,
        None,
    )
    .pinned_changed
    {
        settings.pinned_play_pause = pinned[0];
        settings.pinned_stop = pinned[1];
        settings.pinned_record = pinned[2];
        settings.pinned_step_input = pinned[3];
        settings.pinned_tap_tempo = pinned[4];
        settings.pinned_automation_write = pinned[5];
        settings.save();
    }
    if let Some(action) = pending {
        match action {
            PlayMenuAction::PlayPause { playing } => {
                if playing {
                    actions.pause_return = true;
                } else {
                    actions.toggle_play = true;
                }
            }
            PlayMenuAction::Stop => actions.stop_play = true,
            PlayMenuAction::Record { .. } => actions.record = true,
            PlayMenuAction::StepInput { .. } => actions.step = true,
            PlayMenuAction::TapTempo => actions.tap_tempo = true,
            PlayMenuAction::AutomationWrite { enabled } => {
                actions.set_automation_write = Some(!enabled);
            }
            PlayMenuAction::Follow(mode, _) => **follow_mode = mode,
        }
    }
}

/// 「视图」菜单：底部面板泊出为独立窗口 / 收回停靠（勾选态 = 当前浮动）。
pub fn show_view_menu(
    button: &egui::Response,
    keybindings: &yinhe_editor_core::shortcuts::Keybindings,
    floats: (bool, bool, bool),
    has_active: bool,
    pending_action: &mut Option<PanelKind>,
) {
    let (fp, fd, fm) = floats;
    let selected = |kind: PanelKind| match kind {
        PanelKind::Pianoroll => fp,
        PanelKind::Dock => fd,
        PanelKind::Mixer => fm,
    };
    let width = measure_menu_width(&button.ctx, &VIEW_GROUPS, keybindings);
    egui::Popup::from_toggle_button_response(button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(width)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_min_width(width);
            ui.set_max_width(width);
            for &kind in &PanelKind::ALL {
                let (resp, _) = popup_menu_row(
                    ui,
                    PopupRowSpec {
                        icon: kind.icon(),
                        label: &t!(kind.label_key()),
                        shortcut: None,
                        enabled: has_active,
                        selected: selected(kind),
                        accent: None,
                        pin: None,
                        pin_index: None,
                        chevron: false,
                    },
                );
                if resp.clicked() && has_active {
                    *pending_action = Some(kind);
                    ui.close();
                }
            }
        });
}
