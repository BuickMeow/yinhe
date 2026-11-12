use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::shortcuts::{self, KeyCombo};

use crate::audio_settings::AudioSettings;

/// 文件类动作（11）。
const FILE_ACTIONS: [&str; 11] = [
    shortcuts::ACTION_NEW_PROJECT,
    shortcuts::ACTION_OPEN,
    shortcuts::ACTION_IMPORT_AUDIO,
    shortcuts::ACTION_SAVE,
    shortcuts::ACTION_SAVE_AS,
    shortcuts::ACTION_CLOSE_DOCUMENT,
    shortcuts::ACTION_EXPORT_AUDIO,
    shortcuts::ACTION_EXPORT_MIDI,
    shortcuts::ACTION_SETTINGS,
    shortcuts::ACTION_PROJECT_SETTINGS,
    shortcuts::ACTION_EXIT,
];

/// 编辑类动作（历史 / 剪贴板 / 选择，12）。
const EDIT_ACTIONS: [&str; 12] = [
    shortcuts::ACTION_UNDO,
    shortcuts::ACTION_REDO,
    shortcuts::ACTION_CUT,
    shortcuts::ACTION_COPY,
    shortcuts::ACTION_PASTE,
    shortcuts::ACTION_PASTE_AT_ORIGINAL,
    shortcuts::ACTION_PASTE_FLIPPED,
    shortcuts::ACTION_SELECT_ALL,
    shortcuts::ACTION_SELECT_NOTES_ONLY,
    shortcuts::ACTION_FILTER_SELECTION,
    shortcuts::ACTION_DUPLICATE,
    shortcuts::ACTION_DELETE,
];

/// 操作类动作（框选后的批量变换，12）。
const OPERATION_ACTIONS: [&str; 12] = [
    shortcuts::ACTION_TRANSPOSE_UP,
    shortcuts::ACTION_TRANSPOSE_DOWN,
    shortcuts::ACTION_LENGTH_DOUBLE,
    shortcuts::ACTION_LENGTH_HALF,
    shortcuts::ACTION_FLIP_HORIZONTAL,
    shortcuts::ACTION_FLIP_VERTICAL,
    shortcuts::ACTION_HUMANIZE,
    shortcuts::ACTION_FILL_GAPS,
    shortcuts::ACTION_MERGE_NOTES,
    shortcuts::ACTION_GRID_SPLIT,
    shortcuts::ACTION_DEDUP_WITHIN_TRACK,
    shortcuts::ACTION_DEDUP_ACROSS_TRACKS,
];

/// 播放类动作（2）。
const PLAY_ACTIONS: [&str; 2] = [shortcuts::ACTION_TOGGLE_PLAY, shortcuts::ACTION_STOP];

/// 工具切换动作（8）。
const TOOL_ACTIONS: [&str; 8] = [
    shortcuts::ACTION_TOOL_SELECT,
    shortcuts::ACTION_TOOL_SELECT_VERTICAL,
    shortcuts::ACTION_TOOL_PAN,
    shortcuts::ACTION_TOOL_PENCIL,
    shortcuts::ACTION_TOOL_SCISSORS,
    shortcuts::ACTION_TOOL_ERASER,
    shortcuts::ACTION_TOOL_LINE,
    shortcuts::ACTION_TOOL_BRUSH,
];

/// 轨道 / 音频 / 事件操作（右键菜单可达，默认不分配快捷键）。
///
/// 列进设置页是为了让用户自己分配：这些操作在右键菜单里天天用，
/// 但每个都强塞一个默认键会和现有键位打架。
const CONTEXT_ACTIONS: [&str; 21] = [
    shortcuts::ACTION_TRACK_ADD_BELOW,
    shortcuts::ACTION_TRACK_ADD_ABOVE,
    shortcuts::ACTION_TRACK_MOVE_UP,
    shortcuts::ACTION_TRACK_MOVE_DOWN,
    shortcuts::ACTION_TRACK_DELETE,
    shortcuts::ACTION_TRACK_RENAME,
    shortcuts::ACTION_TRACK_PROPERTIES,
    shortcuts::ACTION_ADD_AUTOMATION,
    shortcuts::ACTION_DELETE_AUTOMATION,
    shortcuts::ACTION_AUDIO_SPLIT,
    shortcuts::ACTION_AUDIO_DUPLICATE,
    shortcuts::ACTION_AUDIO_REVERSE,
    shortcuts::ACTION_AUDIO_NORMALIZE,
    shortcuts::ACTION_AUDIO_GAIN_UP,
    shortcuts::ACTION_AUDIO_GAIN_DOWN,
    shortcuts::ACTION_AUDIO_GAIN_RESET,
    shortcuts::ACTION_AUDIO_DELETE,
    shortcuts::ACTION_EVENT_INSERT_ABOVE,
    shortcuts::ACTION_EVENT_INSERT_BELOW,
    shortcuts::ACTION_DELETE_MARKER,
    shortcuts::ACTION_INSERT_MARKER,
];

/// 快捷键配置页：列出全部可配置动作。
/// 每个动作可绑定多个快捷键：第一个快捷键与动作名同行，其余各占一行；
/// 点击快捷键按钮重新录制，点击 + 追加一个，点击 × 移除。
/// 录制期间 `settings.ui_session.shortcut_recording` 置位，让全局快捷键让位给录制器。
pub fn show_shortcuts_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;

    let rec_id = egui::Id::unique("kb_recording_action");
    // 录制目标：(动作 id, 快捷键索引)；索引 == 列表长度表示追加新快捷键
    let recording: Option<(String, usize)> = ui.data(|d| d.get_temp(rec_id));

    // ── 录制：捕获本帧按键（Esc 取消）──
    if let Some((action_id, idx)) = &recording {
        let mut captured: Option<Option<KeyCombo>> = None; // None = 取消
        let events = ui.input(|i| i.events.clone());
        for ev in &events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = ev
            {
                if *key == egui::Key::Escape {
                    captured = Some(None);
                    break;
                }
                if crate::shortcuts::is_recordable_key(*key) {
                    captured = Some(Some(KeyCombo {
                        command: modifiers.command || modifiers.ctrl,
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        key: crate::shortcuts::key_to_str(*key),
                    }));
                    break;
                }
            }
        }
        if let Some(combo) = captured {
            if let Some(combo) = combo {
                let mut combos = settings.keybindings.get(action_id);
                if *idx < combos.len() {
                    combos[*idx] = combo; // 替换现有快捷键
                } else {
                    combos.push(combo); // 追加新快捷键
                }
                settings.keybindings.set(action_id, combos);
                changed = true;
            }
            ui.data_mut(|d| d.remove::<(String, usize)>(rec_id));
            settings.ui_session.shortcut_recording = false;
        }
    }

    // ── 标题 + 提示 ──
    ui.heading(t!("settings.shortcuts.title").as_ref());
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t!("settings.shortcuts.hint").as_ref())
            .color(crate::theme::text_secondary()),
    );
    ui.add_space(6.0);

    // ── 分组列表 ──
    // 分组只决定「显示在哪个小标题下」，条目不手写：直接按动作 id 归类，
    // 任何新增到 `ALL_ACTION_IDS` 的动作都会自动出现在设置页（漏列=快捷键
    // 无法配置，是之前 12 个动作缺失的根因）。
    let groups: [(&str, &[&str]); 6] = [
        ("settings.shortcuts.group_file", &FILE_ACTIONS),
        ("settings.shortcuts.group_edit", &EDIT_ACTIONS),
        ("settings.shortcuts.group_operation", &OPERATION_ACTIONS),
        ("settings.shortcuts.group_context", &CONTEXT_ACTIONS),
        ("settings.shortcuts.group_play", &PLAY_ACTIONS),
        ("settings.shortcuts.group_tools", &TOOL_ACTIONS),
    ];
    // 自检：分组必须覆盖全部动作，漏一个就在 debug 下炸出来。
    debug_assert_eq!(
        groups.iter().map(|(_, ids)| ids.len()).sum::<usize>(),
        shortcuts::ALL_ACTION_IDS.len(),
        "快捷键设置页的分组必须是 ALL_ACTION_IDS 的一个划分"
    );

    for (group_key, ids) in groups {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(t!(group_key).as_ref())
                .strong()
                .color(crate::theme::text_secondary()),
        );
        for &id in ids {
            let combos = settings.keybindings.get(id);
            let add_idx = combos.len();
            // 是否正在为该动作追加新快捷键（录制中）
            let is_adding = recording
                .as_ref()
                .is_some_and(|(a, idx)| a.as_str() == id && *idx == add_idx);

            // 动作名 + 快捷键列：标签列由 StripBuilder 固定，后续缩进行不再手算 150px。
            let label_key = crate::shortcuts::action_label_key(id);
            crate::widgets::grid::fixed_row(ui, 8.0, 24.0, 150.0, |cells| {
                cells.cell(|ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(t!(label_key).as_ref()))
                                .selectable(false)
                                .wrap_mode(egui::TextWrapMode::Truncate),
                        );
                    });
                });
                cells.cell(|ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if let Some(first) = combos.first() {
                            changed |=
                                shortcut_combo_ui(ui, rec_id, id, 0, first, &recording, settings);
                        }
                        let add_btn = crate::widgets::flat::flat_button_filled_stroked(
                            ui,
                            egui::RichText::new("+").strong(),
                            egui::vec2(24.0, 24.0),
                            None,
                            !is_adding,
                        );
                        if add_btn
                            .on_hover_text(t!("settings.shortcuts.add").as_ref())
                            .clicked()
                        {
                            ui.data_mut(|d| d.insert_temp(rec_id, (id.to_string(), add_idx)));
                            settings.ui_session.shortcut_recording = true;
                            ui.ctx().request_repaint();
                        }
                    });
                });
            });

            // 其余快捷键各占一行，标签列留空。
            for (i, combo) in combos.iter().enumerate().skip(1) {
                crate::widgets::grid::fixed_row(ui, 8.0, 24.0, 150.0, |cells| {
                    cells.cell(|_| {});
                    cells.cell(|ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            changed |=
                                shortcut_combo_ui(ui, rec_id, id, i, combo, &recording, settings);
                        });
                    });
                });
            }

            // 追加录制中：立即在下一行显示录制占位框（按完键后变为新快捷键）
            if is_adding {
                crate::widgets::grid::fixed_row(ui, 8.0, 24.0, 150.0, |cells| {
                    cells.cell(|_| {});
                    cells.cell(|ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let _ = crate::widgets::flat::flat_button_filled_stroked(
                                ui,
                                t!("settings.shortcuts.recording").as_ref(),
                                egui::vec2(140.0, 24.0),
                                None,
                                true,
                            );
                        });
                    });
                });
            }
        }
    }

    // ── 底部：恢复默认 ──
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::flat::flat_button_filled_auto(
                ui,
                t!("settings.shortcuts.reset").as_ref(),
                true,
            )
            .clicked()
            {
                settings.keybindings.reset_to_defaults();
                // 同时取消进行中的录制，避免残留状态干扰
                ui.data_mut(|d| d.remove::<(String, usize)>(rec_id));
                settings.ui_session.shortcut_recording = false;
                changed = true;
            }
        });
    });

    changed
}

/// 单个快捷键行：录制按钮（点击重新录制）+ × 移除。
/// 同一快捷键允许被多个动作绑定，不做冲突限制。
/// 返回是否修改了设置。
fn shortcut_combo_ui(
    ui: &mut egui::Ui,
    rec_id: egui::Id,
    action_id: &str,
    idx: usize,
    combo: &KeyCombo,
    recording: &Option<(String, usize)>,
    settings: &mut AudioSettings,
) -> bool {
    let mut changed = false;

    let is_recording = recording
        .as_ref()
        .is_some_and(|(a, i)| a.as_str() == action_id && *i == idx);
    let btn_text = if is_recording {
        t!("settings.shortcuts.recording").to_string()
    } else {
        crate::shortcuts::display_combo(combo)
    };
    if crate::widgets::flat::flat_button_filled_stroked(
        ui,
        btn_text,
        egui::vec2(140.0, 24.0),
        None,
        true,
    )
    .clicked()
        && !is_recording
    {
        ui.data_mut(|d| d.insert_temp(rec_id, (action_id.to_string(), idx)));
        settings.ui_session.shortcut_recording = true;
        ui.ctx().request_repaint();
    }

    if !is_recording
        && crate::widgets::flat::flat_button_filled_stroked(
            ui,
            egui::RichText::new("×").strong(),
            egui::vec2(24.0, 24.0),
            None,
            true,
        )
        .on_hover_text(t!("settings.shortcuts.clear").as_ref())
        .clicked()
    {
        settings.keybindings.remove(action_id, combo);
        changed = true;
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 分组必须完整覆盖 `ALL_ACTION_IDS`，且不能重复。
    /// 之前设置页只列了 45 个动作里的 33 个，用户无法给缺失的动作配快捷键。
    #[test]
    fn groups_cover_all_actions_exactly_once() {
        let mut all: Vec<&str> = Vec::new();
        all.extend(FILE_ACTIONS);
        all.extend(EDIT_ACTIONS);
        all.extend(OPERATION_ACTIONS);
        all.extend(CONTEXT_ACTIONS);
        all.extend(PLAY_ACTIONS);
        all.extend(TOOL_ACTIONS);
        all.sort_unstable();
        let mut expected = shortcuts::ALL_ACTION_IDS.to_vec();
        expected.sort_unstable();
        assert_eq!(all, expected, "设置页分组必须与 ALL_ACTION_IDS 完全一致");
    }
}
