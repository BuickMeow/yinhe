//! 快捷键的「键盘视图」：反向查看每个按键被哪些动作占用。
//!
//! 与设置页逐行动作的列表互补：列表回答「这个动作绑了什么键」，
//! 键盘回答「这个键上有什么」。后者才能一眼看出键位冲突与空缺。
//!
//! 交互约束：
//! - 键帽上的修饰键状态**只读真实输入**（`i.modifiers`），不自己维护一份状态，
//!   永远不可能显示与实际输入不一致；
//! - 点击有绑定的键 = 在下方列出占用它的动作，可跳到对应动作行；
//! - 这里**不直接编辑绑定**：改键仍走列表里的录制器，避免两套编辑逻辑漂移。

use eframe::egui;
use rust_i18n::t;

use super::keyboard_layout::{KeyCap, ansi_layout};
use crate::audio_settings::AudioSettings;

/// 键盘视图状态（存在 egui 临时数据里，不需要进设置结构体）。
#[derive(Clone, Default)]
struct KeyboardViewState {
    /// 当前选中的键（点击后下方列出占用它的动作）。
    selected: Option<eframe::egui::Key>,
}

fn state_id() -> egui::Id {
    egui::Id::unique("settings_keyboard_view_state")
}

/// 绘制整块键盘视图。返回被点击、需要跳转的动作 id（若有）。
pub(crate) fn show(ui: &mut egui::Ui, settings: &AudioSettings) -> Option<String> {
    let mut jump: Option<String> = None;
    let mut state: KeyboardViewState = ui
        .data(|d| d.get_temp::<KeyboardViewState>(state_id()))
        .unwrap_or_default();

    // 键位 → 占用它的动作（一个键可被多个动作绑定，允许冲突）。
    let bound = collect_bindings(settings);

    ui.heading(t!("settings.shortcuts.keyboard.title").as_ref());
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(t!("settings.shortcuts.keyboard.hint").as_ref())
            .color(crate::theme::text_secondary()),
    );
    ui.add_space(6.0);

    draw_keyboard(ui, &bound, &mut state, &mut jump);

    // ── 选中键的详情 ──
    ui.add_space(8.0);
    match state.selected {
        Some(key) => {
            let label = crate::shortcuts::key_display(key);
            let actions = bound.get(&key).cloned().unwrap_or_default();
            ui.label(
                egui::RichText::new(
                    t!("settings.shortcuts.keyboard.bound_to", key = label).to_string(),
                )
                .strong()
                .color(crate::theme::text_bright()),
            );
            if actions.is_empty() {
                ui.label(
                    egui::RichText::new(t!("settings.shortcuts.keyboard.unbound").as_ref())
                        .color(crate::theme::text_muted()),
                );
            } else {
                for action_id in actions {
                    let name = t!(crate::shortcuts::action_label_key(&action_id)).to_string();
                    // 显示「动作名 · 完整快捷键」，冲突时一眼能看出差在修饰键。
                    let combo = settings
                        .keybindings
                        .get(&action_id)
                        .iter()
                        .filter(|c| crate::shortcuts::str_to_key(&c.key) == Some(key))
                        .map(crate::shortcuts::display_combo)
                        .collect::<Vec<_>>()
                        .join(" / ");
                    let text = if combo.is_empty() {
                        name
                    } else {
                        format!("{name} · {combo}")
                    };
                    if ui
                        .add(crate::widgets::menu::menu_item_button(ui, false, text))
                        .on_hover_text(t!("settings.shortcuts.keyboard.jump").as_ref())
                        .clicked()
                    {
                        jump = Some(action_id);
                    }
                }
            }
        }
        None => {
            ui.label(
                egui::RichText::new(t!("settings.shortcuts.keyboard.select_hint").as_ref())
                    .color(crate::theme::text_muted()),
            );
        }
    }

    ui.data_mut(|d| d.insert_temp(state_id(), state));
    jump
}

/// 键 → 绑定它的动作 id（保持 `ALL_ACTION_IDS` 顺序，便于稳定显示）。
fn collect_bindings(settings: &AudioSettings) -> std::collections::HashMap<egui::Key, Vec<String>> {
    let mut map: std::collections::HashMap<egui::Key, Vec<String>> =
        std::collections::HashMap::new();
    let ids = yinhe_editor_core::shortcuts::ALL_ACTION_IDS
        .iter()
        .chain(yinhe_editor_core::shortcuts::CONTEXT_ACTION_IDS);
    for &id in ids {
        for combo in settings.keybindings.get(id) {
            if let Some(key) = crate::shortcuts::str_to_key(&combo.key) {
                let list = map.entry(key).or_default();
                if !list.iter().any(|e| e == id) {
                    list.push(id.to_string());
                }
            }
        }
    }
    map
}

fn draw_keyboard(
    ui: &mut egui::Ui,
    bound: &std::collections::HashMap<egui::Key, Vec<String>>,
    state: &mut KeyboardViewState,
    jump: &mut Option<String>,
) {
    let ctx = ui.ctx().clone();
    let base = crate::scaling::scaled_font(&ctx, 30.0);
    let gap = crate::scaling::scaled_font(&ctx, 4.0);
    let row_h = base * 0.8;
    let avail = ui.available_width();
    // 以最长行为基准换算基础键宽，保证整块键盘不超出面板宽度。
    let max_units = ansi_layout()
        .iter()
        .map(|r| r.indent + r.caps.iter().map(|c| c.width).sum::<f32>())
        .fold(0.0f32, f32::max)
        .max(1.0);
    let unit_w = ((avail - gap * (max_units - 1.0)) / max_units).max(4.0);

    // 只读真实修饰键状态（不自己维护一份）。
    let mods = ui.input(|i| i.modifiers);

    for row in ansi_layout() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if row.indent > 0.0 {
                ui.add_space(row.indent * unit_w);
            }
            for cap in row.caps {
                draw_cap(ui, cap, unit_w, row_h, mods, bound, state, jump);
            }
        });
    }
}

#[allow(clippy::too_many_arguments)] // 绘制上下文透传
fn draw_cap(
    ui: &mut egui::Ui,
    cap: &KeyCap,
    unit_w: f32,
    row_h: f32,
    mods: egui::Modifiers,
    bound: &std::collections::HashMap<egui::Key, Vec<String>>,
    state: &mut KeyboardViewState,
    jump: &mut Option<String>,
) {
    let w = unit_w * cap.width;
    if cap.label.is_empty() {
        ui.add_space(w);
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, row_h), egui::Sense::click());
    let selected = cap.key.is_some() && state.selected == cap.key;
    // 别名键（如 `=` 同时代表 `+`）也计入这个键帽的占用。
    let mut here: Vec<String> = Vec::new();
    if let Some(k) = cap.key {
        if let Some(ids) = bound.get(&k) {
            here.extend(ids.iter().cloned());
        }
        for &alias in super::keyboard_layout::alias_keys(k) {
            if let Some(more) = bound.get(&alias) {
                for id in more {
                    if !here.contains(id) {
                        here.push(id.clone());
                    }
                }
            }
        }
    }
    let occupied = here.len() == 1;
    // 冲突：同一个键被两个及以上动作绑定。
    let conflict = here.len() > 1;
    // 修饰键高亮跟随真实输入状态。
    let mod_active = matches!(cap.label, "Shift" | "Ctrl" | "Alt" | "Cmd")
        && (mods.shift || mods.ctrl || mods.alt || mods.command);

    let bg = if selected {
        crate::theme::selected_bg()
    } else if conflict {
        crate::theme::warning_gold().gamma_multiply(0.28)
    } else if occupied {
        crate::theme::accent_active().gamma_multiply(0.22)
    } else if mod_active {
        crate::theme::control_bg().gamma_multiply(1.4)
    } else if resp.hovered() {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        crate::theme::control_bg()
    };
    let fg = if conflict {
        crate::theme::warning_gold()
    } else if occupied || selected {
        crate::theme::accent_active()
    } else {
        crate::theme::text_label()
    };
    ui.painter().rect_filled(rect, 4.0, bg);
    // 键帽文字：宽度不足以显示完整标签时缩小字号，避免溢出到邻键。
    let font_size = crate::scaling::scaled_font(ui.ctx(), 11.0).min(w * 0.42);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        cap.label,
        egui::FontId::proportional(font_size),
        fg,
    );

    if !here.is_empty() {
        let names: Vec<String> = here
            .iter()
            .map(|id| t!(crate::shortcuts::action_label_key(id)).to_string())
            .collect();
        resp.clone().on_hover_text(names.join("\n"));
    }

    if resp.clicked()
        && let Some(key) = cap.key
    {
        state.selected = Some(key);
        // 只有一个动作时直接跳转，省一次点击。
        if let [only] = here.as_slice() {
            *jump = Some(only.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_editor_core::shortcuts::{self as sc, KeyCombo, Keybindings};

    fn combo(key: &str) -> KeyCombo {
        KeyCombo {
            command: false,
            shift: false,
            alt: false,
            key: key.to_string(),
        }
    }

    /// 一个键被多个动作绑定时要全部收集（允许键位冲突，这正是键盘视图的意义）。
    #[test]
    fn collects_conflicting_bindings() {
        let mut settings = crate::audio_settings::AudioSettings::default();
        let mut kb = Keybindings::default();
        kb.set(sc::ACTION_DELETE, vec![combo("K")]);
        kb.set(sc::ACTION_DUPLICATE, vec![combo("K")]);
        settings.keybindings = kb;

        let bound = collect_bindings(&settings);
        let actions = bound.get(&egui::Key::K).expect("K 应被占用");
        assert!(actions.iter().any(|a| a == sc::ACTION_DELETE));
        assert!(actions.iter().any(|a| a == sc::ACTION_DUPLICATE));
    }

    /// 同一个动作绑两个键时，两个键都要显示它。
    #[test]
    fn one_action_many_keys_shows_on_each_key() {
        let mut settings = crate::audio_settings::AudioSettings::default();
        let mut kb = Keybindings::default();
        kb.set(sc::ACTION_SAVE, vec![combo("S"), combo("F2")]);
        settings.keybindings = kb;

        let bound = collect_bindings(&settings);
        for key in [egui::Key::S, egui::Key::F2] {
            assert!(
                bound
                    .get(&key)
                    .is_some_and(|v| v.iter().any(|a| a == sc::ACTION_SAVE)),
                "{key:?} 上应能看到保存"
            );
        }
    }

    /// 默认配置里 Space（播放）与 Delete 都应可见，说明默认值真的被收集。
    #[test]
    fn default_bindings_are_visible() {
        let settings = crate::audio_settings::AudioSettings::default();
        let bound = collect_bindings(&settings);
        assert!(bound.contains_key(&egui::Key::Space), "Space 默认绑播放");
        assert!(bound.contains_key(&egui::Key::Delete), "Delete 默认绑删除");
    }
}
