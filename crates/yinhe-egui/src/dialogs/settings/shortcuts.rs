use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::shortcuts::{self, KeyCombo};

use crate::audio_settings::AudioSettings;

/// 快捷键配置页：列出全部可配置动作。
/// 每个动作可绑定多个快捷键：第一个快捷键与动作名同行，其余各占一行；
/// 点击快捷键按钮重新录制，点击 + 追加一个，点击 × 移除。
/// 录制期间 `settings.shortcut_recording` 置位，让全局快捷键让位给录制器。
pub fn show_shortcuts_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;

    let rec_id = egui::Id::new("kb_recording_action");
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
            settings.shortcut_recording = false;
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
    let groups: [(&str, &[&str]); 4] = [
        (
            "settings.shortcuts.group_file",
            &[
                shortcuts::ACTION_NEW_PROJECT,
                shortcuts::ACTION_OPEN,
                shortcuts::ACTION_SAVE,
                shortcuts::ACTION_SAVE_AS,
                shortcuts::ACTION_CLOSE_DOCUMENT,
                shortcuts::ACTION_EXPORT_AUDIO,
                shortcuts::ACTION_EXPORT_MIDI,
                shortcuts::ACTION_SETTINGS,
                shortcuts::ACTION_EXIT,
            ],
        ),
        (
            "settings.shortcuts.group_edit",
            &[
                shortcuts::ACTION_UNDO,
                shortcuts::ACTION_REDO,
                shortcuts::ACTION_CUT,
                shortcuts::ACTION_COPY,
                shortcuts::ACTION_PASTE,
                shortcuts::ACTION_SELECT_ALL,
                shortcuts::ACTION_DUPLICATE,
                shortcuts::ACTION_DELETE,
                shortcuts::ACTION_TRANSPOSE_UP,
                shortcuts::ACTION_TRANSPOSE_DOWN,
            ],
        ),
        (
            "settings.shortcuts.group_play",
            &[shortcuts::ACTION_TOGGLE_PLAY, shortcuts::ACTION_STOP],
        ),
        (
            "settings.shortcuts.group_tools",
            &[
                shortcuts::ACTION_TOOL_SELECT,
                shortcuts::ACTION_TOOL_SELECT_VERTICAL,
                shortcuts::ACTION_TOOL_PAN,
                shortcuts::ACTION_TOOL_PENCIL,
                shortcuts::ACTION_TOOL_CURVE,
                shortcuts::ACTION_TOOL_SCISSORS,
                shortcuts::ACTION_TOOL_ERASER,
            ],
        ),
    ];

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

            // 第一行：动作名 + 第一个快捷键 + ×（+ 追加按钮）
            // 动作名标签用 Extend 撑满 150 宽，保证与后续缩进行精确对齐
            ui.horizontal(|ui| {
                let label_key = crate::shortcuts::action_label_key(id);
                ui.add_sized(
                    [150.0, 24.0],
                    egui::Label::new(egui::RichText::new(t!(label_key).as_ref()))
                        .selectable(false)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );

                if let Some(first) = combos.first() {
                    changed |= shortcut_combo_ui(ui, rec_id, id, 0, first, &recording, settings);
                }

                // 追加新快捷键（录制中时末尾不再额外显示录制块，仅保留下一行的占位行）
                let add_btn = egui::Button::new(egui::RichText::new("+").strong())
                    .min_size(egui::vec2(28.0, 24.0));
                if ui
                    .add_enabled(!is_adding, add_btn)
                    .on_hover_text(t!("settings.shortcuts.add").as_ref())
                    .clicked()
                {
                    ui.data_mut(|d| d.insert_temp(rec_id, (id.to_string(), add_idx)));
                    settings.shortcut_recording = true;
                    ui.ctx().request_repaint();
                }
            });

            // 其余快捷键各占一行（缩进对齐）。
            // 用 allocate_exact_size 而非 add_space：add_space 推进后不再加
            // item_spacing，会导致比第一行动作名标签（add_sized）少 8px 而错位。
            for (i, combo) in combos.iter().enumerate().skip(1) {
                ui.horizontal(|ui| {
                    ui.allocate_exact_size(egui::vec2(150.0, 24.0), egui::Sense::hover());
                    changed |= shortcut_combo_ui(ui, rec_id, id, i, combo, &recording, settings);
                });
            }

            // 追加录制中：立即在下一行显示录制占位框（按完键后变为新快捷键）
            if is_adding {
                ui.horizontal(|ui| {
                    ui.allocate_exact_size(egui::vec2(150.0, 24.0), egui::Sense::hover());
                    let place_btn = egui::Button::new(t!("settings.shortcuts.recording").as_ref())
                        .min_size(egui::vec2(140.0, 24.0));
                    ui.add(place_btn);
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
            if ui.button(t!("settings.shortcuts.reset").as_ref()).clicked() {
                settings.keybindings.reset_to_defaults();
                // 同时取消进行中的录制，避免残留状态干扰
                ui.data_mut(|d| d.remove::<(String, usize)>(rec_id));
                settings.shortcut_recording = false;
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
    let kb_btn = egui::Button::new(btn_text).min_size(egui::vec2(140.0, 24.0));
    if ui.add(kb_btn).clicked() && !is_recording {
        ui.data_mut(|d| d.insert_temp(rec_id, (action_id.to_string(), idx)));
        settings.shortcut_recording = true;
        ui.ctx().request_repaint();
    }

    if !is_recording
        && ui
            .add(
                egui::Button::new(egui::RichText::new("×").strong())
                    .min_size(egui::vec2(28.0, 24.0)),
            )
            .on_hover_text(t!("settings.shortcuts.clear").as_ref())
            .clicked()
    {
        settings.keybindings.remove(action_id, combo);
        changed = true;
    }

    changed
}
