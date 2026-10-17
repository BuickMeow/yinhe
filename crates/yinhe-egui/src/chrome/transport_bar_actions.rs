use egui_material_icons::icons::*;
use rust_i18n::t;
use yinhe_editor_core::shortcuts;

use crate::widgets::action_menu::{MenuGroup, PopupRow};

/// 文件菜单动作
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    NewProject,
    Open,
    ImportAudio,
    /// 欢迎页面：打开独立欢迎窗口（与启动欢迎窗口同款）。
    Welcome,
    Save,
    SaveAs,
    CloseDocument,
    ExportAudio,
    ExportMidi,
    ProjectSettings,
    Settings,
    Exit,
    /// 关于窗口。
    About,
}

impl FileAction {
    pub const ALL: [FileAction; 13] = [
        FileAction::NewProject,
        FileAction::Open,
        FileAction::ImportAudio,
        FileAction::Welcome,
        FileAction::Save,
        FileAction::SaveAs,
        FileAction::CloseDocument,
        FileAction::ExportAudio,
        FileAction::ExportMidi,
        FileAction::ProjectSettings,
        FileAction::Settings,
        FileAction::Exit,
        FileAction::About,
    ];

    /// 图钉数组索引：新增项追加在末尾，保证既有 pin 持久化数据不错位。
    pub const fn pinned_index(self) -> usize {
        match self {
            FileAction::NewProject => 0,
            FileAction::Open => 1,
            FileAction::ImportAudio => 2,
            FileAction::Save => 3,
            FileAction::SaveAs => 4,
            FileAction::CloseDocument => 5,
            FileAction::ExportAudio => 6,
            FileAction::ExportMidi => 7,
            FileAction::ProjectSettings => 8,
            FileAction::Settings => 9,
            FileAction::Exit => 10,
            FileAction::Welcome => 11,
            FileAction::About => 12,
        }
    }

    pub const fn action_id(self) -> &'static str {
        match self {
            FileAction::NewProject => shortcuts::ACTION_NEW_PROJECT,
            FileAction::Open => shortcuts::ACTION_OPEN,
            FileAction::ImportAudio => shortcuts::ACTION_IMPORT_AUDIO,
            FileAction::Save => shortcuts::ACTION_SAVE,
            FileAction::SaveAs => shortcuts::ACTION_SAVE_AS,
            FileAction::CloseDocument => shortcuts::ACTION_CLOSE_DOCUMENT,
            FileAction::ExportAudio => shortcuts::ACTION_EXPORT_AUDIO,
            FileAction::ExportMidi => shortcuts::ACTION_EXPORT_MIDI,
            FileAction::ProjectSettings => shortcuts::ACTION_PROJECT_SETTINGS,
            FileAction::Settings => shortcuts::ACTION_SETTINGS,
            FileAction::Exit => shortcuts::ACTION_EXIT,
            // 欢迎页面 / 关于：无快捷键（label_key 特例，不走快捷键表）。
            FileAction::Welcome | FileAction::About => "",
        }
    }

    pub const fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            FileAction::NewProject => ICON_NOTE_ADD,
            FileAction::Open => ICON_FOLDER_OPEN,
            FileAction::ImportAudio => ICON_AUDIO_FILE,
            FileAction::Welcome => ICON_HOME,
            FileAction::Save => ICON_SAVE,
            FileAction::SaveAs => ICON_SAVE_ALT,
            FileAction::CloseDocument => ICON_CLOSE,
            FileAction::ExportAudio => ICON_AUDIO_FILE,
            FileAction::ExportMidi => ICON_MUSIC_NOTE,
            FileAction::ProjectSettings => ICON_TUNE,
            FileAction::Settings => ICON_SETTINGS,
            FileAction::Exit => ICON_EXIT_TO_APP,
            FileAction::About => ICON_INFO,
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            FileAction::Welcome => "menu.welcome_page",
            FileAction::About => "menu.about",
            _ => crate::shortcuts::action_label_key(self.action_id()),
        }
    }

    fn is_enabled(self, has_active: bool, loading: bool) -> bool {
        match self {
            FileAction::NewProject | FileAction::Open | FileAction::ImportAudio => !loading,
            // 欢迎页面/关于/设置/退出：任何时候都可用。
            FileAction::Welcome | FileAction::About | FileAction::Settings | FileAction::Exit => {
                true
            }
            FileAction::Save
            | FileAction::SaveAs
            | FileAction::CloseDocument
            | FileAction::ExportAudio
            | FileAction::ExportMidi
            | FileAction::ProjectSettings => has_active,
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
    PasteAtOriginal,
    PasteFlipped,
    SelectAll,
    SelectNotesOnly,
    FilterSelection,
    Duplicate,
    Delete,
    TransposeUp,
    TransposeDown,
    DedupWithinTrack,
    DedupAcrossTracks,
    /// 人性化：给选中音符注入随机偏差（时间/力度/时值）。
    Humanize,
    /// 填缝：把框选内音符终点贴合到同轨道下一个不同起点（消除断续与叠音）。
    FillGaps,
    /// 合并：把框选内首尾严格相接的同 key 音符合并成一个。
    MergeNotes,
    /// 变速：选中音符时值 ×2。
    LengthDouble,
    /// 变速：选中音符时值 ÷2。
    LengthHalf,
    /// 水平翻转选中音符（时间轴镜像）。
    FlipHorizontal,
    /// 垂直翻转选中音符（音高轴镜像）。
    FlipVertical,
    /// 网格切割：按当前量化切分选框内音符（清空选框）。
    GridSplit,
}

impl EditAction {
    pub const ALL: [EditAction; 24] = [
        EditAction::Undo,
        EditAction::Redo,
        EditAction::Cut,
        EditAction::Copy,
        EditAction::Paste,
        EditAction::PasteAtOriginal,
        EditAction::PasteFlipped,
        EditAction::SelectAll,
        EditAction::SelectNotesOnly,
        EditAction::FilterSelection,
        EditAction::Duplicate,
        EditAction::Delete,
        EditAction::TransposeUp,
        EditAction::TransposeDown,
        EditAction::DedupWithinTrack,
        EditAction::DedupAcrossTracks,
        EditAction::Humanize,
        EditAction::FillGaps,
        EditAction::MergeNotes,
        EditAction::LengthDouble,
        EditAction::LengthHalf,
        EditAction::FlipHorizontal,
        EditAction::FlipVertical,
        EditAction::GridSplit,
    ];

    pub const fn pinned_index(self) -> usize {
        match self {
            EditAction::Undo => 0,
            EditAction::Redo => 1,
            EditAction::Cut => 2,
            EditAction::Copy => 3,
            EditAction::Paste => 4,
            EditAction::PasteAtOriginal => 5,
            EditAction::PasteFlipped => 6,
            EditAction::SelectAll => 7,
            EditAction::SelectNotesOnly => 8,
            EditAction::FilterSelection => 9,
            EditAction::Duplicate => 10,
            EditAction::Delete => 11,
            EditAction::TransposeUp => 12,
            EditAction::TransposeDown => 13,
            EditAction::DedupWithinTrack => 14,
            EditAction::DedupAcrossTracks => 15,
            EditAction::Humanize => 16,
            EditAction::FillGaps => 17,
            EditAction::MergeNotes => 18,
            EditAction::LengthDouble => 19,
            EditAction::LengthHalf => 20,
            EditAction::FlipHorizontal => 21,
            EditAction::FlipVertical => 22,
            EditAction::GridSplit => 23,
        }
    }

    pub const fn action_id(self) -> &'static str {
        match self {
            EditAction::Undo => shortcuts::ACTION_UNDO,
            EditAction::Redo => shortcuts::ACTION_REDO,
            EditAction::Cut => shortcuts::ACTION_CUT,
            EditAction::Copy => shortcuts::ACTION_COPY,
            EditAction::Paste => shortcuts::ACTION_PASTE,
            EditAction::PasteAtOriginal => shortcuts::ACTION_PASTE_AT_ORIGINAL,
            EditAction::PasteFlipped => shortcuts::ACTION_PASTE_FLIPPED,
            EditAction::SelectAll => shortcuts::ACTION_SELECT_ALL,
            EditAction::SelectNotesOnly => shortcuts::ACTION_SELECT_NOTES_ONLY,
            EditAction::FilterSelection => shortcuts::ACTION_FILTER_SELECTION,
            EditAction::Duplicate => shortcuts::ACTION_DUPLICATE,
            EditAction::Delete => shortcuts::ACTION_DELETE,
            EditAction::TransposeUp => shortcuts::ACTION_TRANSPOSE_UP,
            EditAction::TransposeDown => shortcuts::ACTION_TRANSPOSE_DOWN,
            EditAction::DedupWithinTrack => shortcuts::ACTION_DEDUP_WITHIN_TRACK,
            EditAction::DedupAcrossTracks => shortcuts::ACTION_DEDUP_ACROSS_TRACKS,
            EditAction::Humanize => shortcuts::ACTION_HUMANIZE,
            EditAction::FillGaps => shortcuts::ACTION_FILL_GAPS,
            EditAction::MergeNotes => shortcuts::ACTION_MERGE_NOTES,
            EditAction::LengthDouble => shortcuts::ACTION_LENGTH_DOUBLE,
            EditAction::LengthHalf => shortcuts::ACTION_LENGTH_HALF,
            EditAction::FlipHorizontal => shortcuts::ACTION_FLIP_HORIZONTAL,
            EditAction::FlipVertical => shortcuts::ACTION_FLIP_VERTICAL,
            EditAction::GridSplit => shortcuts::ACTION_GRID_SPLIT,
        }
    }

    pub const fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            EditAction::Undo => ICON_UNDO,
            EditAction::Redo => ICON_REDO,
            EditAction::Cut => ICON_CONTENT_CUT,
            EditAction::Copy => ICON_CONTENT_COPY,
            EditAction::Paste => ICON_CONTENT_PASTE,
            EditAction::PasteAtOriginal => ICON_LOCATION_ON,
            EditAction::PasteFlipped => ICON_FLIP,
            EditAction::SelectAll => ICON_SELECT_ALL,
            EditAction::SelectNotesOnly => ICON_MUSIC_NOTE,
            EditAction::FilterSelection => ICON_FILTER_ALT,
            EditAction::Duplicate => ICON_COPY_ALL,
            EditAction::Delete => ICON_DELETE,
            EditAction::TransposeUp => ICON_ARROW_UPWARD,
            EditAction::TransposeDown => ICON_ARROW_DOWNWARD,
            EditAction::DedupWithinTrack => ICON_STACK_OFF,
            EditAction::DedupAcrossTracks => ICON_STACK_OFF,
            EditAction::Humanize => ICON_PERSON,
            EditAction::FillGaps => ICON_COMPARE_ARROWS,
            EditAction::MergeNotes => ICON_LINE_STYLE,
            EditAction::LengthDouble => ICON_EXPAND,
            EditAction::LengthHalf => ICON_COMPRESS,
            EditAction::FlipHorizontal => ICON_FLIP,
            EditAction::FlipVertical => ICON_FLIP,
            EditAction::GridSplit => ICON_GRID_ON,
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
    /// 需要存在选择（框选/自动化锚点）才可执行的动作。
    fn needs_selection(self) -> bool {
        matches!(
            self,
            EditAction::Cut
                | EditAction::Copy
                | EditAction::Duplicate
                | EditAction::Delete
                | EditAction::TransposeUp
                | EditAction::TransposeDown
                | EditAction::DedupWithinTrack
                | EditAction::DedupAcrossTracks
                | EditAction::Humanize
                | EditAction::FillGaps
                | EditAction::MergeNotes
                | EditAction::LengthDouble
                | EditAction::LengthHalf
                | EditAction::FlipHorizontal
                | EditAction::FlipVertical
                | EditAction::GridSplit
        )
    }
    fn is_enabled(self, has_active: bool, _loading: bool) -> bool {
        self.is_enabled(has_active)
    }
}

/// 播放菜单动作
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMenuAction {
    PlayPause {
        playing: bool,
    },
    Stop,
    Record {
        recording: bool,
    },
    StepInput {
        active: bool,
    },
    TapTempo,
    /// 写入自动化开关：开 = 拖动效果器旋钮时写入自动化 lane；
    /// 关（默认）= 只实时预览声音。
    AutomationWrite {
        enabled: bool,
    },
}

impl PopupRow for PlayMenuAction {
    fn pinned_index(self) -> usize {
        match self {
            PlayMenuAction::PlayPause { .. } => 0,
            PlayMenuAction::Stop => 1,
            PlayMenuAction::Record { .. } => 2,
            PlayMenuAction::StepInput { .. } => 3,
            PlayMenuAction::TapTempo => 4,
            PlayMenuAction::AutomationWrite { .. } => 5,
        }
    }

    fn action_id(self) -> &'static str {
        match self {
            PlayMenuAction::PlayPause { .. } => shortcuts::ACTION_TOGGLE_PLAY,
            PlayMenuAction::Stop => shortcuts::ACTION_STOP,
            _ => "",
        }
    }

    fn icon(self) -> egui_material_icons::MaterialIcon {
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
            PlayMenuAction::TapTempo => ICON_SPEED,
            PlayMenuAction::AutomationWrite { .. } => ICON_EDIT_NOTE,
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            PlayMenuAction::PlayPause { .. } => "shortcuts.play_toggle",
            PlayMenuAction::Stop => "shortcuts.stop",
            PlayMenuAction::Record { .. } => "menu.record",
            PlayMenuAction::StepInput { .. } => "menu.step_input",
            PlayMenuAction::TapTempo => "menu.tap_tempo",
            PlayMenuAction::AutomationWrite { .. } => "menu.automation_write",
        }
    }

    fn icon_accent(self) -> Option<egui::Color32> {
        match self {
            PlayMenuAction::Record { .. } => Some(crate::theme::danger()),
            PlayMenuAction::StepInput { active } if active => Some(crate::theme::accent_active()),
            PlayMenuAction::AutomationWrite { enabled } if enabled => {
                Some(crate::theme::accent_active())
            }
            _ => None,
        }
    }

    fn is_selected(self) -> bool {
        match self {
            PlayMenuAction::Record { recording } => recording,
            PlayMenuAction::StepInput { active } => active,
            PlayMenuAction::AutomationWrite { enabled } => enabled,
            _ => false,
        }
    }

    fn is_enabled(self, _has_active: bool, _loading: bool) -> bool {
        true
    }
}

/// 视图菜单动作：跟随档（单选）｜钢琴卷帘方向（单选）｜自动化面板显隐。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewAction {
    /// 跟随档（第二项 = 当前选中档）。
    Follow(crate::view_interaction::FollowMode, bool),
    /// 钢琴卷帘方向（第二项 = 当前方向）。
    Orientation(yinhe_types::Orientation, bool),
    /// 自动化面板显隐开关：`show` = 当前是否显示；`enabled` = 当前是否可切换
    /// （纵向瀑布流不提供 AM 视图，始终不可选）。
    AutomationPanels { show: bool, enabled: bool },
}

impl PopupRow for ViewAction {
    fn pinned_index(self) -> usize {
        0
    }

    fn has_pin(self) -> bool {
        false
    }

    fn action_id(self) -> &'static str {
        ""
    }

    fn icon(self) -> egui_material_icons::MaterialIcon {
        use crate::view_interaction::FollowModeExt;
        match self {
            ViewAction::Follow(mode, _) => mode.icon(),
            ViewAction::Orientation(orientation, _) => match orientation {
                yinhe_types::Orientation::Horizontal => ICON_EAST,
                yinhe_types::Orientation::Vertical => ICON_SOUTH,
            },
            ViewAction::AutomationPanels { .. } => ICON_BAR_CHART,
        }
    }

    fn label_key(self) -> &'static str {
        use crate::view_interaction::FollowMode;
        match self {
            ViewAction::Follow(mode, _) => match mode {
                FollowMode::None => "follow.none",
                FollowMode::Centered => "follow.centered",
                FollowMode::Page => "follow.page",
                FollowMode::Continuous => "follow.continuous",
            },
            ViewAction::Orientation(orientation, _) => match orientation {
                yinhe_types::Orientation::Horizontal => "menu.orientation.horizontal",
                yinhe_types::Orientation::Vertical => "menu.orientation.vertical",
            },
            ViewAction::AutomationPanels { .. } => "menu.automation_panels",
        }
    }

    fn is_selected(self) -> bool {
        match self {
            ViewAction::Follow(_, selected) => selected,
            ViewAction::Orientation(_, selected) => selected,
            ViewAction::AutomationPanels { show, .. } => show,
        }
    }

    fn is_enabled(self, has_active: bool, _loading: bool) -> bool {
        match self {
            // 跟随是全局播放模式，无文档也可切换。
            ViewAction::Follow(..) => true,
            ViewAction::Orientation(..) => has_active,
            ViewAction::AutomationPanels { enabled, .. } => has_active && enabled,
        }
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
    pub tap_tempo: bool,
    /// 切换「写入自动化」开关（Some = 设为该值）。
    pub set_automation_write: Option<bool>,
}

/// 传输栏上下文与响应
pub struct TransportContext<'a> {
    pub file_loader: &'a mut crate::file_loader::FileLoader,
    pub doc: Option<&'a yinhe_editor_core::document::Document>,
    pub follow_mode: &'a mut crate::view_interaction::FollowMode,
    pub is_recording: bool,
    pub step_input: bool,
    pub settings: &'a mut crate::audio_settings::AudioSettings,
    /// 当前文档 PR 是否为纵向瀑布流（只读，切换由 main_loop 落地到文档视口）。
    pub orientation_vertical: bool,
    /// 当前聚焦视图是否为 PR（量化按钮显示/写入聚焦视图的量化）。
    pub focus_is_pianoroll: bool,
    /// AR 量化（PR 量化在 `pr.quantize`）。
    pub quantize_arrange: yinhe_editor_core::quantize::QuantizePreset,
    /// `Auto` 档在当前 AR 缩放下解析出的具体网格（量化弹窗内显示）。
    pub quantize_arrange_auto_resolved: yinhe_editor_core::quantize::QuantizePreset,
    /// PR 控制组数据（量化/和弦；无活动文档 = None，右侧组不显示）。
    pub pr: Option<super::transport_bar_pr::PrBarData>,
    /// 总输出电平显示值 `((level L,R), (hold L,R))`（时间码第四栏竖表）。
    pub master_meter: ((f32, f32), (f32, f32)),
    /// 当前是否存在 PR 框选（决定选择动作按钮是否可用；None 文档 = false）。
    pub selection_available: bool,
}

pub struct TransportResponse {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub record_toggle: bool,
    pub step_toggle: bool,
    pub tap_tempo: bool,
    /// 直接设定钢琴卷帘方向（单按钮切换时下发目标方向；None = 未点击）。
    pub set_orientation: Option<yinhe_types::Orientation>,
    /// 视图菜单「自动化面板」开关（Some = 设为该值）。
    pub set_automation_panels: Option<bool>,
    /// 时间码编辑事件（BPM/拍号/PPQ/位置/调式/量化）。
    pub timecode_events: Vec<crate::widgets::timecode::TimecodeEvent>,
    /// transport bar 上选择动作按钮的点击（删除/复制/移调/翻转；None = 未点击）。
    pub selection_action: Option<crate::widgets::selection_actions::SelectionAction>,
    pub pending_file_action: Option<FileAction>,
    pub pending_edit_action: Option<EditAction>,
    /// 插入菜单点击（None = 未点击）。
    pub pending_insert_action: Option<super::insert_actions::InsertAction>,
    pub pending_open_path: Option<String>,
}

/// 文件/编辑分组常量
pub const FILE_GROUPS: [MenuGroup<'static, FileAction>; 5] = [
    MenuGroup::plain(&[
        FileAction::NewProject,
        FileAction::Open,
        FileAction::ImportAudio,
        FileAction::Welcome,
    ]),
    MenuGroup::plain(&[
        FileAction::Save,
        FileAction::SaveAs,
        FileAction::CloseDocument,
    ]),
    MenuGroup::plain(&[FileAction::ProjectSettings]),
    MenuGroup::plain(&[FileAction::ExportAudio, FileAction::ExportMidi]),
    // 组 5 不进 macOS 文件菜单（设置/退出在 App 菜单），About 也随之只在应用内菜单出现。
    MenuGroup::plain(&[FileAction::Settings, FileAction::About, FileAction::Exit]),
];

/// 编辑菜单分组：撤销｜剪贴板｜选择｜基础编辑｜变换（音高/时值/方向）｜清理与改造。
/// 插入类动作已迁至「插入」菜单（见 `insert_actions`）。
pub const EDIT_GROUPS: [MenuGroup<'static, EditAction>; 6] = [
    MenuGroup::plain(&[EditAction::Undo, EditAction::Redo]),
    MenuGroup::plain(&[
        EditAction::Cut,
        EditAction::Copy,
        EditAction::Paste,
        EditAction::PasteAtOriginal,
        EditAction::PasteFlipped,
    ]),
    MenuGroup::plain(&[
        EditAction::SelectAll,
        EditAction::SelectNotesOnly,
        EditAction::FilterSelection,
    ]),
    MenuGroup::plain(&[EditAction::Delete, EditAction::Duplicate]),
    MenuGroup::plain(&[
        EditAction::TransposeUp,
        EditAction::TransposeDown,
        EditAction::LengthHalf,
        EditAction::LengthDouble,
        EditAction::FlipHorizontal,
        EditAction::FlipVertical,
    ]),
    MenuGroup::plain(&[
        EditAction::Humanize,
        EditAction::FillGaps,
        EditAction::MergeNotes,
        EditAction::GridSplit,
        EditAction::DedupWithinTrack,
        EditAction::DedupAcrossTracks,
    ]),
];

pub fn tool_hint(tool: crate::widgets::tools_panel::Tool) -> String {
    use crate::widgets::tools_panel::Tool;
    match tool {
        Tool::Select => t!("hint.tool.select").to_string(),
        Tool::SelectVertical => t!("hint.tool.select_vertical").to_string(),
        Tool::Pan => t!("hint.tool.pan").to_string(),
        Tool::Pencil => t!("hint.tool.pencil").to_string(),
        Tool::Scissors => t!("hint.tool.scissors").to_string(),
        Tool::Eraser => t!("hint.tool.eraser").to_string(),
        Tool::Line => t!("hint.tool.line").to_string(),
        Tool::Brush => t!("hint.tool.brush").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 图钉数组长度必须覆盖全部菜单动作：新加动作后忘了同步
    /// （`PINNED_FILE_ACTIONS_LEN` / `PINNED_EDIT_ACTIONS_LEN`）会让该项的
    /// pin 点击静默失效（越界被 `get_mut` 忽略）。
    #[test]
    fn pinned_arrays_cover_all_actions() {
        let s = crate::audio_settings::AudioSettings::default();
        assert!(
            s.pinned_file_actions.len() >= FileAction::ALL.len(),
            "pinned_file_actions({}) < FileAction::ALL({})",
            s.pinned_file_actions.len(),
            FileAction::ALL.len()
        );
        assert!(
            s.pinned_edit_actions.len() >= EditAction::ALL.len(),
            "pinned_edit_actions({}) < EditAction::ALL({})",
            s.pinned_edit_actions.len(),
            EditAction::ALL.len()
        );
    }
}
