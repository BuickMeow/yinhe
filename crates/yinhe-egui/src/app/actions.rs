//! 应用动作：键盘分发、音符编辑、剪贴板、选择、文件与保存导出。

use eframe::egui;

use crate::app::App;
use crate::chrome::transport_bar::{EditAction, FileAction};
use yinhe_editor_core::shortcuts;

mod clipboard;
mod dispatch;
mod file;
mod history;
mod note_edit;
mod save;
mod selection;
mod timecode;

pub(crate) use save::SaveSnapshot;

/// Actions detected from keyboard input in the current frame.
///
/// 编辑类动作收敛成单个 `edit_action`（与顶栏菜单 / macOS 原生菜单同一条分发路径
/// `App::handle_edit_action`）：否则每加一个动作都要在键盘分发里补一段 if，
/// 漏补就会「表里有快捷键但按了没反应」。
#[derive(Default)]
pub(crate) struct KeyboardActions {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    /// 编辑动作（删除/复制/移调/变速/翻转/人性化/填缝/合并/网格切割/去重…）。
    pub edit_action: Option<EditAction>,
    /// 上下文动作 id（轨道/片段/事件类，见 `ACTION_TRACK_*` 等）。
    /// 这些动作默认没有快捷键，用户分配后走这里。
    pub context_action: Option<&'static str>,
    /// 工具切换快捷键触发的目标工具（None = 本帧未触发）。
    pub tool_to_activate: Option<crate::widgets::tools_panel::Tool>,
    /// 文件菜单动作（非 macOS 平台由键盘触发；macOS 走原生菜单栏）。
    pub file_action: Option<FileAction>,
}

impl App {
    /// Handle keyboard shortcuts.
    /// Returns a `KeyboardActions` struct describing which actions were triggered.
    pub(crate) fn handle_keyboard_shortcuts(&self, ui: &egui::Ui) -> KeyboardActions {
        let mut actions = KeyboardActions::default();

        // 文本输入焦点（TextEdit/DragValue 等）优先：全局快捷键让位给输入框，
        // 与成熟 DAW 一致（Backspace/Delete/Cmd+C/V/Z 等作用于文本而非选区）。
        // 设置窗口打开或快捷键录制期间同样让位：设置页里不允许任何快捷键触发动作
        // （Esc 例外：由设置页录制器消费用于取消录制）。
        if ui.ctx().egui_wants_keyboard_input()
            || self.audio_settings.ui_session.show_settings
            || self.audio_settings.ui_session.shortcut_recording
        {
            return actions;
        }

        let is_playing_any = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);

        // 本帧唯一一次主键按下（排除纯修饰键）。
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if crate::shortcuts::is_recordable_key(*key) => Some((*key, *modifiers)),
                _ => None,
            })
        });

        let kb = &self.audio_settings.keybindings;
        // 一个动作可绑定多个快捷键，任一匹配即触发。
        // macOS：第一个快捷键若带修饰键（⌘/⇧/⌥），由原生菜单加速键在系统层面
        // 处理（AppKit 拦截，egui 收不到那个键），这里跳过它避免双触发；
        // 无修饰键的快捷键（如 Space）AppKit 不会拦截菜单加速键，必须由 egui
        // 处理，否则按键会失效。其余快捷键（第二个及以后）同样由 egui 处理。
        let matches = |id: &str, key: egui::Key, modifiers: egui::Modifiers| {
            kb.get(id).iter().enumerate().any(|(i, c)| {
                #[cfg(target_os = "macos")]
                {
                    if i == 0 && crate::shortcuts::native_menu_handles(c) {
                        return false;
                    }
                }
                crate::shortcuts::matches_combo(c, modifiers, key)
            })
        };

        if let Some((key, modifiers)) = pressed {
            // ── 文件动作 ──
            // macOS：第一个快捷键走原生菜单；其余快捷键在这里分发。
            for action in FileAction::ALL {
                if matches(action.action_id(), key, modifiers) {
                    actions.file_action = Some(action);
                    break;
                }
            }

            // ── 播放/停止 ──
            if matches(shortcuts::ACTION_TOGGLE_PLAY, key, modifiers) {
                if is_playing_any {
                    actions.pause_return = true;
                } else {
                    actions.toggle_play = true;
                }
            }
            if matches(shortcuts::ACTION_STOP, key, modifiers) {
                actions.stop_play = true;
            }

            // ── 编辑动作 ──
            // 全部走 `EditAction::ALL` + `handle_edit_action`：单一分发路径，
            // 不会出现「配置表里有这个动作、键盘却没人检查」的漏项。
            // macOS：第一个快捷键由原生菜单加速键处理（上面的 matches 已跳过）。
            for action in EditAction::ALL {
                let id = action.action_id();
                if !id.is_empty() && matches(id, key, modifiers) {
                    actions.edit_action = Some(action);
                    break;
                }
            }

            // ── 上下文动作（轨道 / 片段 / 事件，默认无快捷键）──
            // 用户若在设置页分配了键位，这里才能触发；未分配时 `kb.get` 返回空列表。
            if actions.edit_action.is_none() {
                for &id in yinhe_editor_core::shortcuts::CONTEXT_ACTION_IDS {
                    if matches(id, key, modifiers) {
                        actions.context_action = Some(id);
                        break;
                    }
                }
            }

            // ── 工具切换 ──
            // 工具动作不在 macOS 原生菜单中，egui 一定能收到按键，
            // 因此这里不跳过第一个快捷键（与文件/编辑动作的 macOS 处理不同）。
            for tool in crate::widgets::tools_panel::ALL_TOOLS {
                if kb
                    .get(tool.action_id())
                    .iter()
                    .any(|c| crate::shortcuts::matches_combo(c, modifiers, key))
                {
                    actions.tool_to_activate = Some(tool);
                    break;
                }
            }
        }

        // ── 兼容别名（配置表之外的历史默认）──
        ui.input(|i| {
            // Backspace 等同 Delete
            if i.key_pressed(egui::Key::Backspace) {
                actions.edit_action = Some(EditAction::Delete);
            }
            // Cmd/Ctrl+Y 也触发重做
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            if cmd && i.key_pressed(egui::Key::Y) {
                actions.edit_action = Some(EditAction::Redo);
            }
        });

        actions
    }
}
