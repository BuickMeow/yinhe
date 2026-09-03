use egui_material_icons::icons::*;
use rust_i18n::t;
use yinhe_editor_core::shortcuts;

use crate::widgets::action_menu::PopupRow;

/// 文件菜单动作
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    NewProject,
    Open,
    Save,
    SaveAs,
    CloseDocument,
    ExportAudio,
    ExportMidi,
    ProjectSettings,
    Settings,
    Exit,
}

impl FileAction {
    pub const ALL: [FileAction; 10] = [
        FileAction::NewProject,
        FileAction::Open,
        FileAction::Save,
        FileAction::SaveAs,
        FileAction::CloseDocument,
        FileAction::ExportAudio,
        FileAction::ExportMidi,
        FileAction::ProjectSettings,
        FileAction::Settings,
        FileAction::Exit,
    ];

    pub const fn pinned_index(self) -> usize {
        match self {
            FileAction::NewProject => 0,
            FileAction::Open => 1,
            FileAction::Save => 2,
            FileAction::SaveAs => 3,
            FileAction::CloseDocument => 4,
            FileAction::ExportAudio => 5,
            FileAction::ExportMidi => 6,
            FileAction::ProjectSettings => 7,
            FileAction::Settings => 8,
            FileAction::Exit => 9,
        }
    }

    pub const fn action_id(self) -> &'static str {
        match self {
            FileAction::NewProject => shortcuts::ACTION_NEW_PROJECT,
            FileAction::Open => shortcuts::ACTION_OPEN,
            FileAction::Save => shortcuts::ACTION_SAVE,
            FileAction::SaveAs => shortcuts::ACTION_SAVE_AS,
            FileAction::CloseDocument => shortcuts::ACTION_CLOSE_DOCUMENT,
            FileAction::ExportAudio => shortcuts::ACTION_EXPORT_AUDIO,
            FileAction::ExportMidi => shortcuts::ACTION_EXPORT_MIDI,
            FileAction::ProjectSettings => shortcuts::ACTION_PROJECT_SETTINGS,
            FileAction::Settings => shortcuts::ACTION_SETTINGS,
            FileAction::Exit => shortcuts::ACTION_EXIT,
        }
    }

    pub const fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            FileAction::NewProject => ICON_NOTE_ADD,
            FileAction::Open => ICON_FOLDER_OPEN,
            FileAction::Save => ICON_SAVE,
            FileAction::SaveAs => ICON_SAVE_ALT,
            FileAction::CloseDocument => ICON_CLOSE,
            FileAction::ExportAudio => ICON_AUDIO_FILE,
            FileAction::ExportMidi => ICON_MUSIC_NOTE,
            FileAction::ProjectSettings => ICON_TUNE,
            FileAction::Settings => ICON_SETTINGS,
            FileAction::Exit => ICON_EXIT_TO_APP,
        }
    }

    pub fn label_key(self) -> &'static str {
        crate::shortcuts::action_label_key(self.action_id())
    }

    fn is_enabled(self, has_active: bool, loading: bool) -> bool {
        match self {
            FileAction::NewProject | FileAction::Open => !loading,
            FileAction::Save
            | FileAction::SaveAs
            | FileAction::CloseDocument
            | FileAction::ExportAudio
            | FileAction::ExportMidi
            | FileAction::ProjectSettings => has_active,
            FileAction::Settings | FileAction::Exit => true,
        }
    }
}

impl PopupRow for FileAction {
    fn pinned_index(self) -> usize {
        self.pinned_index()
    }
    fn action_id(self) -> &'static str {
        self.action_id()
    }
    fn icon(self) -> egui_material_icons::MaterialIcon {
        self.icon()
    }
    fn label_key(self) -> &'static str {
        self.label_key()
    }
    fn is_enabled(self, has_active: bool, loading: bool) -> bool {
        self.is_enabled(has_active, loading)
    }
}

/// 编辑菜单动作
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Duplicate,
    Delete,
    TransposeUp,
    TransposeDown,
    DedupWithinTrack,
    DedupAcrossTracks,
}

impl EditAction {
    pub const ALL: [EditAction; 12] = [
        EditAction::Undo,
        EditAction::Redo,
        EditAction::Cut,
        EditAction::Copy,
        EditAction::Paste,
        EditAction::SelectAll,
        EditAction::Duplicate,
        EditAction::Delete,
        EditAction::TransposeUp,
        EditAction::TransposeDown,
        EditAction::DedupWithinTrack,
        EditAction::DedupAcrossTracks,
    ];

    pub const fn pinned_index(self) -> usize {
        match self {
            EditAction::Undo => 0,
            EditAction::Redo => 1,
            EditAction::Cut => 2,
            EditAction::Copy => 3,
            EditAction::Paste => 4,
            EditAction::SelectAll => 5,
            EditAction::Duplicate => 6,
            EditAction::Delete => 7,
            EditAction::TransposeUp => 8,
            EditAction::TransposeDown => 9,
            EditAction::DedupWithinTrack => 10,
            EditAction::DedupAcrossTracks => 11,
        }
    }

    pub const fn action_id(self) -> &'static str {
        match self {
            EditAction::Undo => shortcuts::ACTION_UNDO,
            EditAction::Redo => shortcuts::ACTION_REDO,
            EditAction::Cut => shortcuts::ACTION_CUT,
            EditAction::Copy => shortcuts::ACTION_COPY,
            EditAction::Paste => shortcuts::ACTION_PASTE,
            EditAction::SelectAll => shortcuts::ACTION_SELECT_ALL,
            EditAction::Duplicate => shortcuts::ACTION_DUPLICATE,
            EditAction::Delete => shortcuts::ACTION_DELETE,
            EditAction::TransposeUp => shortcuts::ACTION_TRANSPOSE_UP,
            EditAction::TransposeDown => shortcuts::ACTION_TRANSPOSE_DOWN,
            EditAction::DedupWithinTrack => shortcuts::ACTION_DEDUP_WITHIN_TRACK,
            EditAction::DedupAcrossTracks => shortcuts::ACTION_DEDUP_ACROSS_TRACKS,
        }
    }

    pub const fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            EditAction::Undo => ICON_UNDO,
            EditAction::Redo => ICON_REDO,
            EditAction::Cut => ICON_CONTENT_CUT,
            EditAction::Copy => ICON_CONTENT_COPY,
            EditAction::Paste => ICON_CONTENT_PASTE,
            EditAction::SelectAll => ICON_SELECT_ALL,
            EditAction::Duplicate => ICON_COPY_ALL,
            EditAction::Delete => ICON_DELETE,
            EditAction::TransposeUp => ICON_ARROW_UPWARD,
            EditAction::TransposeDown => ICON_ARROW_DOWNWARD,
            EditAction::DedupWithinTrack => ICON_STACK_OFF,
            EditAction::DedupAcrossTracks => ICON_STACK_OFF,
        }
    }

    pub fn label_key(self) -> &'static str {
        crate::shortcuts::action_label_key(self.action_id())
    }

    fn is_enabled(self, has_active: bool) -> bool {
        has_active
    }
}

impl PopupRow for EditAction {
    fn pinned_index(self) -> usize {
        self.pinned_index()
    }
    fn action_id(self) -> &'static str {
        self.action_id()
    }
    fn icon(self) -> egui_material_icons::MaterialIcon {
        self.icon()
    }
    fn label_key(self) -> &'static str {
        self.label_key()
    }
    fn is_enabled(self, has_active: bool, _loading: bool) -> bool {
        self.is_enabled(has_active)
    }
}

/// 播放菜单动作
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMenuAction {
    PlayPause { playing: bool },
    Stop,
    Record { recording: bool },
    StepInput { active: bool },
    Follow(crate::view_interaction::FollowMode, bool),
}

impl PopupRow for PlayMenuAction {
    fn pinned_index(self) -> usize {
        match self {
            PlayMenuAction::PlayPause { .. } => 0,
            PlayMenuAction::Stop => 1,
            PlayMenuAction::Record { .. } => 2,
            PlayMenuAction::StepInput { .. } => 3,
            PlayMenuAction::Follow(..) => 0,
        }
    }

    fn has_pin(self) -> bool {
        matches!(
            self,
            PlayMenuAction::PlayPause { .. }
                | PlayMenuAction::Stop
                | PlayMenuAction::Record { .. }
                | PlayMenuAction::StepInput { .. }
        )
    }

    fn action_id(self) -> &'static str {
        match self {
            PlayMenuAction::PlayPause { .. } => shortcuts::ACTION_TOGGLE_PLAY,
            PlayMenuAction::Stop => shortcuts::ACTION_STOP,
            _ => "",
        }
    }

    fn icon(self) -> egui_material_icons::MaterialIcon {
        use crate::view_interaction::FollowModeExt;
        match self {
            PlayMenuAction::PlayPause { playing } => {
                if playing {
                    ICON_PAUSE
                } else {
                    ICON_PLAY_ARROW
                }
            }
            PlayMenuAction::Stop => ICON_STOP,
            PlayMenuAction::Record { .. } => ICON_FIBER_MANUAL_RECORD,
            PlayMenuAction::StepInput { .. } => ICON_STEP,
            PlayMenuAction::Follow(mode, _) => mode.icon(),
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            PlayMenuAction::PlayPause { .. } => "shortcuts.play_toggle",
            PlayMenuAction::Stop => "shortcuts.stop",
            PlayMenuAction::Record { .. } => "menu.record",
            PlayMenuAction::StepInput { .. } => "menu.step_input",
            PlayMenuAction::Follow(mode, _) => match mode {
                crate::view_interaction::FollowMode::None => "follow.none",
                crate::view_interaction::FollowMode::Centered => "follow.centered",
                crate::view_interaction::FollowMode::Page => "follow.page",
                crate::view_interaction::FollowMode::Continuous => "follow.continuous",
            },
        }
    }

    fn icon_accent(self) -> Option<egui::Color32> {
        match self {
            PlayMenuAction::Record { recording } if recording => {
                Some(egui::Color32::from_rgb(232, 17, 35))
            }
            PlayMenuAction::StepInput { active } if active => Some(crate::theme::accent_active()),
            PlayMenuAction::Follow(_, selected) if selected => Some(crate::theme::accent_active()),
            _ => None,
        }
    }

    fn is_selected(self) -> bool {
        match self {
            PlayMenuAction::Follow(_, sel) => sel,
            _ => false,
        }
    }

    fn is_enabled(self, _has_active: bool, _loading: bool) -> bool {
        true
    }
}

/// 播放动作聚合
#[derive(Default)]
pub struct PlayActions {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub record: bool,
    pub step: bool,
}

/// 传输栏上下文与响应
pub struct TransportContext<'a> {
    pub file_loader: &'a mut crate::file_loader::FileLoader,
    pub doc: Option<&'a yinhe_editor_core::document::Document>,
    pub follow_mode: &'a mut crate::view_interaction::FollowMode,
    pub active_tool: &'a mut crate::widgets::tools_panel::Tool,
    pub is_recording: bool,
    pub step_input: bool,
    pub status_hint: &'a mut Option<String>,
    pub settings: &'a mut crate::audio_settings::AudioSettings,
    pub orientation: &'a mut yinhe_types::Orientation,
}

pub struct TransportResponse {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub record_toggle: bool,
    pub step_toggle: bool,
    pub toggle_orientation: bool,
    pub pending_file_action: Option<FileAction>,
    pub pending_edit_action: Option<EditAction>,
    pub pending_open_path: Option<String>,
}

/// 文件/编辑分组常量
pub const FILE_GROUPS: [&[FileAction]; 5] = [
    &[FileAction::NewProject, FileAction::Open],
    &[
        FileAction::Save,
        FileAction::SaveAs,
        FileAction::CloseDocument,
    ],
    &[FileAction::ProjectSettings],
    &[FileAction::ExportAudio, FileAction::ExportMidi],
    &[FileAction::Settings, FileAction::Exit],
];

pub const EDIT_GROUPS: [&[EditAction]; 5] = [
    &[EditAction::Undo, EditAction::Redo],
    &[EditAction::Cut, EditAction::Copy, EditAction::Paste],
    &[
        EditAction::SelectAll,
        EditAction::Duplicate,
        EditAction::Delete,
    ],
    &[EditAction::TransposeUp, EditAction::TransposeDown],
    &[EditAction::DedupWithinTrack, EditAction::DedupAcrossTracks],
];

pub fn tool_hint(tool: crate::widgets::tools_panel::Tool) -> String {
    use crate::widgets::tools_panel::Tool;
    match tool {
        Tool::Select => t!("hint.tool.select").to_string(),
        Tool::SelectVertical => t!("hint.tool.select_vertical").to_string(),
        Tool::Pan => t!("hint.tool.pan").to_string(),
        Tool::Pencil => t!("hint.tool.pencil").to_string(),
        Tool::Curve => t!("hint.tool.curve").to_string(),
        Tool::Scissors => t!("hint.tool.scissors").to_string(),
        Tool::Eraser => t!("hint.tool.eraser").to_string(),
    }
}
