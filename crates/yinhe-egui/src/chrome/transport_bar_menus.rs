use crate::audio_settings::AudioSettings;
use crate::file_loader::FileLoader;
use crate::widgets::action_menu::{ActionMenuExtra, MenuGroup, show_action_menu};
use eframe::egui;

use super::transport_bar_actions::{
    EDIT_GROUPS, EditAction, FILE_GROUPS, FileAction, PlayActions, PlayMenuAction,
};
use super::transport_bar_recent::{recent_files_section, recent_parent_width};
use crate::view_interaction::FollowMode;
use yinhe_types::Orientation;

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
        false,
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
    selection_available: bool,
    settings: &mut AudioSettings,
    pending_action: &mut Option<EditAction>,
) {
    let keybindings = &settings.keybindings;
    let pinned = &mut settings.pinned_edit_actions;
    if show_action_menu(
        button,
        &EDIT_GROUPS,
        has_active,
        selection_available,
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
    let settings = &mut ctx.settings;
    let play_items = [
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
    ];
    let groups = [MenuGroup::plain(&play_items)];
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
        }
    }
}

/// 视图菜单：跟随档（单选）｜钢琴卷帘方向（单选）｜自动化面板显隐。
/// 点击结果：跟随直接写 `ctx.follow_mode`；方向与面板开关经 out 参数
/// 交回 `transport_bar::show` 打包进 `TransportResponse`。
pub fn show_view_menu(
    button: &egui::Response,
    ctx: &mut super::transport_bar_actions::TransportContext<'_>,
    set_orientation: &mut Option<yinhe_types::Orientation>,
    set_automation_panels: &mut Option<bool>,
) {
    use super::transport_bar_actions::{ViewAction, ViewAction as V};
    let has_active = ctx.doc.is_some();
    let vertical = ctx.orientation_vertical;
    // 自动化面板当前状态（无文档 = 关）与可用性（纵向瀑布流不提供 AM 视图）。
    let panels = ctx.doc.map(|d| d.edit.show_controller_panels);
    let panels_enabled = has_active && !vertical;
    let follow_mode = &mut ctx.follow_mode;
    let settings = &mut ctx.settings;

    let follow_items = [
        V::Follow(FollowMode::None, **follow_mode == FollowMode::None),
        V::Follow(FollowMode::Centered, **follow_mode == FollowMode::Centered),
        V::Follow(FollowMode::Page, **follow_mode == FollowMode::Page),
        V::Follow(
            FollowMode::Continuous,
            **follow_mode == FollowMode::Continuous,
        ),
    ];
    let control_items = [
        V::Orientation(Orientation::Horizontal, !vertical),
        V::Orientation(Orientation::Vertical, vertical),
        V::AutomationPanels {
            show: panels.unwrap_or(false),
            enabled: panels_enabled,
        },
    ];
    let groups = [
        MenuGroup::plain(&follow_items),
        MenuGroup::plain(&control_items),
    ];

    let mut pending: Option<ViewAction> = None;
    show_action_menu(
        button,
        &groups,
        has_active,
        false,
        false,
        &settings.keybindings,
        None,
        &mut pending,
        None,
    );
    if let Some(action) = pending {
        match action {
            V::Follow(mode, _) => **follow_mode = mode,
            V::Orientation(orientation, _) => *set_orientation = Some(orientation),
            V::AutomationPanels { show, .. } => *set_automation_panels = Some(!show),
        }
    }
}
