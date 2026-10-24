//! 插入菜单：动作定义、分组（带副标题）与 popup 渲染。
//!
//! 插入位置规则（详见 `app::insert`）：
//! - 全局事件（标签/BPM/拍号/调号/歌词/和弦）只取播放指示线 tick；
//! - 音轨事件（PC/自动化/轨内歌词/和弦）取「选中音轨 × 指示线」，
//!   多选音轨时每条轨各插一个；无选中回退到最后一条普通轨；
//! - 歌词/和弦自动分流：选中普通轨 → 轨内版；否则（含仅选中 Conductor）→ 全局版。

use egui_material_icons::icons::*;

use crate::audio_settings::AudioSettings;
use crate::widgets::action_menu::{MenuGroup, PopupRow, show_action_menu};

/// 插入菜单动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertAction {
    /// 新音轨：插在选中最后一条轨之后（无选中 = 最末）。
    Track,
    /// 标签（MIDI marker）：默认名直插，事后双击改名。
    Marker,
    /// BPM：弹窗输入数值。
    Bpm,
    /// 拍号：弹窗选分子/分母。
    TimeSig,
    /// 调号 / 调式：弹窗选主音/音阶。
    KeySig,
    /// 歌词：默认空文本直插；自动分流轨内/全局。
    Lyrics,
    /// 和弦：默认空文本直插；自动分流轨内/全局。
    Chord,
    /// 音色（PC）：弹窗输入音色号；插到选中音轨（多选批量）。
    ProgramChange,
    /// 自动化：弹窗选 target（CC/参数等）；插到选中音轨（多选批量）。
    Automation,
}

impl InsertAction {
    /// 图钉数组索引：与菜单顺序一致，新增项追加在末尾。
    pub const fn pinned_index(self) -> usize {
        match self {
            InsertAction::Track => 0,
            InsertAction::Marker => 1,
            InsertAction::Bpm => 2,
            InsertAction::TimeSig => 3,
            InsertAction::KeySig => 4,
            InsertAction::Lyrics => 5,
            InsertAction::Chord => 6,
            InsertAction::ProgramChange => 7,
            InsertAction::Automation => 8,
        }
    }

    pub const ALL: [InsertAction; 9] = [
        InsertAction::Track,
        InsertAction::Marker,
        InsertAction::Bpm,
        InsertAction::TimeSig,
        InsertAction::KeySig,
        InsertAction::Lyrics,
        InsertAction::Chord,
        InsertAction::ProgramChange,
        InsertAction::Automation,
    ];
}

impl PopupRow for InsertAction {
    fn pinned_index(self) -> usize {
        self.pinned_index()
    }

    fn has_pin(self) -> bool {
        true
    }

    fn action_id(self) -> &'static str {
        ""
    }

    fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            InsertAction::Track => ICON_QUEUE_MUSIC,
            InsertAction::Marker => ICON_BOOKMARK,
            InsertAction::Bpm => ICON_SPEED,
            InsertAction::TimeSig => ICON_SCHEDULE,
            InsertAction::KeySig => ICON_MUSIC_NOTE,
            InsertAction::Lyrics => ICON_LYRICS,
            InsertAction::Chord => ICON_PIANO,
            InsertAction::ProgramChange => ICON_PALETTE,
            InsertAction::Automation => ICON_SHOW_CHART,
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            InsertAction::Track => "menu.insert.track",
            // 复用既有「插入标签」文案（编辑菜单迁入）。
            InsertAction::Marker => "menu.insert_marker",
            InsertAction::Bpm => "menu.insert.bpm",
            InsertAction::TimeSig => "menu.insert.timesig",
            InsertAction::KeySig => "menu.insert.keysig",
            InsertAction::Lyrics => "menu.insert.lyrics",
            InsertAction::Chord => "menu.insert.chord",
            InsertAction::ProgramChange => "menu.insert.pc",
            InsertAction::Automation => "menu.insert.automation",
        }
    }

    fn is_enabled(self, has_active: bool, _loading: bool) -> bool {
        has_active
    }
}

/// 插入菜单分组：音轨（结构）｜全局事件｜音轨事件。
/// 桌面 popup 显示小标题；macOS 原生菜单只用分组间的分割线。
pub const INSERT_GROUPS: [MenuGroup<'static, InsertAction>; 3] = [
    MenuGroup::titled("menu.insert.group.structure", &[InsertAction::Track]),
    MenuGroup::titled(
        "menu.insert.group.global",
        &[
            InsertAction::Marker,
            InsertAction::Bpm,
            InsertAction::TimeSig,
            InsertAction::KeySig,
            InsertAction::Lyrics,
            InsertAction::Chord,
        ],
    ),
    MenuGroup::titled(
        "menu.insert.group.track_events",
        &[InsertAction::ProgramChange, InsertAction::Automation],
    ),
];

/// 渲染插入菜单（按钮 toggle popup）。返回本帧被点击的动作。
pub fn show_insert_menu(
    button: &egui::Response,
    has_active: bool,
    settings: &mut AudioSettings,
) -> Option<InsertAction> {
    let mut pending = None;
    let outcome = {
        let pinned = &mut settings.pinned_insert_actions;
        show_action_menu(
            button,
            &INSERT_GROUPS,
            has_active,
            // 插入不需要既有选择：无选中时音轨类项回退到最后一条普通轨。
            false,
            false,
            &settings.keybindings,
            Some(pinned),
            &mut pending,
            None,
        )
    };
    if outcome.pinned_changed {
        settings.save();
    }
    pending
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 菜单项文案 key 不重复（防复制粘贴时漏改）。
    #[test]
    fn insert_labels_are_unique() {
        let all = [
            InsertAction::Track,
            InsertAction::Marker,
            InsertAction::Bpm,
            InsertAction::TimeSig,
            InsertAction::KeySig,
            InsertAction::Lyrics,
            InsertAction::Chord,
            InsertAction::ProgramChange,
            InsertAction::Automation,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.label_key(), b.label_key(), "文案 key 重复: {a:?}");
            }
        }
    }
}
