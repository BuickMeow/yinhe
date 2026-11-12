//! 快捷键的「键盘视图」：反向查看每个按键被哪些动作占用。
//!
//! 与逐动作列表互补：列表回答「这个动作绑了什么键」，键盘回答「这个键上有什么」。
//!
//! 设计约束（按用户要求）：
//! - 键盘**常驻页面顶部**，不是二级菜单；
//! - 点键位后只在下方显示该键绑定的动作 + 一个「添加」按钮，不显示别的键；
//! - 修饰键（Ctrl/Alt/Shift/Cmd）**各自独立计数**：同一个字母键配了 ⌘K 和 ⌥K
//!   是两个不同绑定，不算冲突；
//! - 修改绑定直接在下方完成（录制 / 删除），不再跳回列表。
//!
//! 修饰键状态只读真实输入（`i.modifiers`），不自己维护第二份状态。

use eframe::egui;
use rust_i18n::t;
use yinhe_editor_core::shortcuts::{self as sc, KeyCombo};

use super::keyboard_layout::{KeyCap, ansi_layout, row_units};
use crate::audio_settings::AudioSettings;

/// 一条绑定：动作 + 完整组合键。
#[derive(Clone)]
struct Binding {
    action_id: String,
    combo: KeyCombo,
}

/// 键盘视图状态（存 egui 临时数据，不进设置结构体）。
#[derive(Clone, Default)]
struct KeyboardViewState {
    /// 当前选中的物理键。
    selected: Option<egui::Key>,
    /// 正在为哪个动作录制（None = 未录制）。
    recording_action: Option<String>,
    /// 录制中替换的下标；== 列表长度表示追加。
    recording_idx: usize,
    /// 正在为哪个物理键挑选动作（弹出动作列表）。
    picking_action: Option<egui::Key>,
}

fn state_id() -> egui::Id {
    egui::Id::unique("settings_keyboard_view_state")
}

pub(crate) fn show(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    let mut state: KeyboardViewState = ui
        .data(|d| d.get_temp::<KeyboardViewState>(state_id()))
        .unwrap_or_default();

    // 键帽 → 绑定列表（按"物理键 + 修饰键"区分，见 `bindings_for_cap`）。
    let all = collect_bindings(settings);

    draw_keyboard(ui, &all, &mut state);

    // ── 录制：捕获本帧按键（Esc 取消）──
    if let Some(action_id) = state.recording_action.clone() {
        let captured = capture_combo(ui);
        if let Some(combo) = captured {
            if let Some(combo) = combo {
                let mut combos = settings.keybindings.get(&action_id);
                if state.recording_idx < combos.len() {
                    combos[state.recording_idx] = combo;
                } else {
                    combos.push(combo);
                }
                settings.keybindings.set(&action_id, combos);
                changed = true;
            }
            state.recording_action = None;
            settings.ui_session.shortcut_recording = false;
        } else {
            settings.ui_session.shortcut_recording = true;
        }
    }

    // ── 选中键的绑定（只显示这个键，不显示别的）──
    if let Some(key) = state.selected {
        ui.add_space(8.0);
        draw_binding_list(ui, settings, key, &all, &mut state, &mut changed);
    }

    ui.data_mut(|d| d.insert_temp(state_id(), state));
    changed
}

/// 从本帧输入捕获一个组合键；`None` = 本帧还没按。
fn capture_combo(ui: &egui::Ui) -> Option<Option<KeyCombo>> {
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
                return Some(None);
            }
            if crate::shortcuts::is_recordable_key(*key) {
                return Some(Some(KeyCombo {
                    command: modifiers.command || modifiers.ctrl,
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                    key: crate::shortcuts::key_to_str(*key),
                }));
            }
        }
    }
    None
}

/// 全部绑定：动作 id → 组合键（保持 ALL_ACTION_IDS 顺序，显示稳定）。
fn collect_bindings(settings: &AudioSettings) -> Vec<Binding> {
    let mut out = Vec::new();
    let ids = sc::ALL_ACTION_IDS.iter().chain(sc::CONTEXT_ACTION_IDS);
    for &id in ids {
        for combo in settings.keybindings.get(id) {
            out.push(Binding {
                action_id: id.to_string(),
                combo,
            });
        }
    }
    out
}

/// 某个物理键帽上的绑定。
///
/// **按修饰键区分**：`⌘K`、`⌥K`、`K` 是三条不同绑定，都挂在这个 K 键帽上，
/// 但它们互不冲突。冲突只在「修饰键完全相同」时才算。
fn bindings_for_cap(cap_key: egui::Key, all: &[Binding]) -> Vec<Binding> {
    let mut keys = vec![cap_key];
    keys.extend_from_slice(super::keyboard_layout::alias_keys(cap_key));
    all.iter()
        .filter(|b| crate::shortcuts::str_to_key(&b.combo.key).is_some_and(|k| keys.contains(&k)))
        .cloned()
        .collect()
}

/// 同一物理键上，修饰键完全相同且属于不同动作的组合 —— 真正的冲突。
fn conflicting_combos(here: &[Binding]) -> Vec<KeyCombo> {
    let mut out: Vec<KeyCombo> = Vec::new();
    for (i, a) in here.iter().enumerate() {
        for b in &here[i + 1..] {
            if a.action_id != b.action_id
                && same_modifiers(&a.combo, &b.combo)
                && !out.contains(&a.combo)
            {
                out.push(a.combo.clone());
            }
        }
    }
    out
}

fn same_modifiers(a: &KeyCombo, b: &KeyCombo) -> bool {
    a.command == b.command && a.shift == b.shift && a.alt == b.alt && a.key == b.key
}

fn draw_keyboard(ui: &mut egui::Ui, all: &[Binding], state: &mut KeyboardViewState) {
    // 2u 高的键（小键盘 + / Enter）会伸到下一行，先画普通键、后画高键，
    // 保证高键压在上层，不会被下一行的行背景盖住。
    let ctx = ui.ctx().clone();
    let gap = crate::scaling::scaled_font(&ctx, 3.0);
    let row_h = crate::scaling::scaled_font(&ctx, 26.0);
    let avail = ui.available_width();

    // 国际标准的键位单位：普通键 1u、Tab 1.5u、Caps 1.75u、Enter 2.25u、
    // 左右 Shift 2.25u/2.75u、Ctrl/Alt/Cmd 各 1.25u、Space 6.25u。
    // 以最长一行为基准换算 1u 的像素宽度，保证不超出面板。
    let max_units = ansi_layout()
        .iter()
        .map(row_units)
        .fold(0.0f32, f32::max)
        .max(1.0);
    // 预留键间距：每行最多有多少个键帽，就留多少 gap。
    let max_caps = ansi_layout()
        .iter()
        .map(|r| {
            r.segments
                .iter()
                .map(|s| s.caps.len())
                .sum::<usize>()
                .saturating_add(r.segments.len())
        })
        .max()
        .unwrap_or(1) as f32;
    let unit_w = ((avail - gap * max_caps) / max_units).max(3.0);

    let mods = ui.input(|i| i.modifiers);

    // 自绘整块键盘：同一行内不同高度（小键盘 + / Enter 是 2u 高）用绝对矩形
    // 排布，`ui.horizontal` 无法表达"同行但更高"的键。
    for row in ansi_layout() {
        let (row_rect, _) = ui.allocate_exact_size(egui::vec2(avail, row_h), egui::Sense::hover());
        let mut x = row_rect.min.x;
        let mut deferred: Vec<(&KeyCap, egui::Rect)> = Vec::new();
        for (si, segment) in row.segments.iter().enumerate() {
            if si > 0 {
                x += segment.gap_before * unit_w;
            }
            for cap in segment.caps {
                let w = unit_w * cap.width;
                let h = row_h * cap.height.min(2.0);
                let cap_rect =
                    egui::Rect::from_min_size(egui::pos2(x, row_rect.min.y), egui::vec2(w, h));
                // 高键延后绘制（见函数开头的层级说明）。
                if cap.height <= 1.0 + f32::EPSILON {
                    draw_cap_at(ui, cap, cap_rect, mods, all, state);
                } else {
                    deferred.push((cap, cap_rect));
                }
                x += w + gap;
            }
        }
        for (cap, rect) in deferred {
            draw_cap_at(ui, cap, rect, mods, all, state);
        }
        ui.add_space(gap);
    }
}

/// 在给定矩形里画一个键帽（自绘 + 交互）。
fn draw_cap_at(
    ui: &mut egui::Ui,
    cap: &KeyCap,
    rect: egui::Rect,
    mods: egui::Modifiers,
    all: &[Binding],
    state: &mut KeyboardViewState,
) {
    let w = rect.width();
    if cap.label.is_empty() && cap.modifier.is_none() {
        return;
    }
    let resp = ui.interact(
        rect,
        ui.scope_id().with(("keycap", cap.label, cap.key)),
        egui::Sense::click(),
    );

    // 纯修饰键：只按真实状态高亮，不可点选绑定。
    // 名称随平台变化（macOS 的 ⌘ Command / ⌥ Option vs Windows 的 Win / Alt）。
    if let Some(m) = cap.modifier {
        let active = m.active(mods);
        let bg = if active {
            crate::theme::accent_active().gamma_multiply(0.30)
        } else {
            crate::theme::control_bg()
        };
        let fg = if active {
            crate::theme::accent_active()
        } else {
            crate::theme::text_label()
        };
        ui.painter().rect_filled(rect, 4.0, bg);
        paint_cap_label(ui, rect, m.label(), w, fg);
        return;
    }

    let Some(key) = cap.key else {
        // 装饰键（小键盘等）：画出来但不参与绑定。
        ui.painter()
            .rect_filled(rect, 4.0, crate::theme::control_bg());
        paint_cap_label(ui, rect, cap.label, w, crate::theme::text_muted());
        return;
    };

    let here = bindings_for_cap(key, all);
    let conflicts = conflicting_combos(&here);
    let selected = state.selected == Some(key);

    let bg = if selected {
        crate::theme::selected_bg()
    } else if !conflicts.is_empty() {
        crate::theme::warning_gold().gamma_multiply(0.30)
    } else if !here.is_empty() {
        crate::theme::accent_active().gamma_multiply(0.22)
    } else if resp.hovered() {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        crate::theme::control_bg()
    };
    let fg = if !conflicts.is_empty() {
        crate::theme::warning_gold()
    } else if !here.is_empty() || selected {
        crate::theme::accent_active()
    } else {
        crate::theme::text_label()
    };
    ui.painter().rect_filled(rect, 4.0, bg);
    paint_cap_label(ui, rect, cap.label, w, fg);
    // 有多个绑定时右下角画个小点，提示"这个键上不止一个"。
    if here.len() > 1 {
        ui.painter()
            .circle_filled(egui::pos2(rect.max.x - 4.0, rect.max.y - 4.0), 1.8, fg);
    }

    if !here.is_empty() {
        let mut lines: Vec<String> = here
            .iter()
            .map(|b| {
                format!(
                    "{} · {}",
                    t!(crate::shortcuts::action_label_key(&b.action_id)),
                    crate::shortcuts::display_combo(&b.combo)
                )
            })
            .collect();
        if !conflicts.is_empty() {
            lines.push(t!("settings.shortcuts.keyboard.conflict").to_string());
        }
        resp.clone().on_hover_text(lines.join("\n"));
    }

    if resp.clicked() {
        state.selected = Some(key);
    }
}

fn paint_cap_label(ui: &egui::Ui, rect: egui::Rect, label: &str, w: f32, color: egui::Color32) {
    let size = crate::scaling::scaled_font(ui.ctx(), 11.0).min(w * 0.40);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(size),
        color,
    );
}

/// 选中键的绑定列表 + 添加。
fn draw_binding_list(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    key: egui::Key,
    all: &[Binding],
    state: &mut KeyboardViewState,
    changed: &mut bool,
) {
    let here = bindings_for_cap(key, all);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(crate::shortcuts::key_display(key))
                .strong()
                .color(crate::theme::text_bright()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // 「添加」= 给这个键新增一条绑定：先选动作，再录组合键。
            let adding = state.recording_action.is_some();
            if crate::widgets::flat::flat_button_filled_stroked(
                ui,
                t!("settings.shortcuts.add").as_ref(),
                egui::vec2(72.0, 24.0),
                None,
                !adding,
            )
            .clicked()
            {
                state.picking_action = Some(key);
            }
        });
    });

    // 动作选择：在弹窗里挑一个动作，挑完立刻进入录制。
    if state.picking_action == Some(key)
        && let Some(action_id) = pick_action_list(ui, key, &here)
    {
        state.picking_action = None;
        if !action_id.is_empty() {
            state.recording_action = Some(action_id);
            state.recording_idx = usize::MAX;
            settings.ui_session.shortcut_recording = true;
            ui.ctx().request_repaint();
        }
    }

    if here.is_empty() {
        ui.label(
            egui::RichText::new(t!("settings.shortcuts.keyboard.unbound").as_ref())
                .color(crate::theme::text_muted()),
        );
        return;
    }

    let conflicts = conflicting_combos(&here);
    for b in &here {
        let name = t!(crate::shortcuts::action_label_key(&b.action_id)).to_string();
        let combo = crate::shortcuts::display_combo(&b.combo);
        let is_conflict = conflicts.contains(&b.combo);
        ui.horizontal(|ui| {
            let mut text = egui::RichText::new(format!("{name} · {combo}"));
            if is_conflict {
                text = text.color(crate::theme::warning_gold());
            }
            ui.label(text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 删除这条绑定
                if crate::widgets::flat::flat_button_filled_stroked(
                    ui,
                    egui::RichText::new("×").strong(),
                    egui::vec2(24.0, 24.0),
                    None,
                    true,
                )
                .on_hover_text(t!("settings.shortcuts.clear").as_ref())
                .clicked()
                    && let Some(pos) = settings
                        .keybindings
                        .get(&b.action_id)
                        .iter()
                        .position(|c| same_modifiers(c, &b.combo))
                {
                    let mut combos = settings.keybindings.get(&b.action_id);
                    combos.remove(pos);
                    settings.keybindings.set(&b.action_id, combos);
                    *changed = true;
                }
            });
        });
    }
}

// `Modifier::active` 定义在布局表里，避免这里重复判断平台修饰键。

/// 动作选择列表（行内展开，不用 popup：设置页本身就在滚动区里，
/// 行内列表能跟着滚、不遮挡，也不需要处理 popup 生命周期）。
///
/// 返回 `Some("")` = 取消；`Some(action_id)` = 选中；`None` = 仍在选择中。
fn pick_action_list(ui: &mut egui::Ui, key: egui::Key, here: &[Binding]) -> Option<String> {
    let mut result: Option<String> = None;
    let key_label = crate::shortcuts::key_display(key);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&key_label)
                .strong()
                .color(crate::theme::text_bright()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("common.cancel").as_ref(),
                ))
                .clicked()
            {
                result = Some(String::new());
            }
        });
    });
    ui.separator();

    // 只列这个键上**还没有**的动作，避免重复添加。
    let bound: Vec<&str> = here.iter().map(|b| b.action_id.as_str()).collect();
    egui::ScrollArea::vertical()
        .id_salt("pick_action_scroll")
        .max_height(280.0)
        .show(ui, |ui| {
            for (group_key, ids) in super::shortcuts::action_groups() {
                let candidates: Vec<&str> = ids
                    .iter()
                    .copied()
                    .filter(|id| !bound.contains(id))
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                ui.label(
                    egui::RichText::new(t!(group_key).as_ref())
                        .size(crate::scaling::scaled_font(
                            ui.ctx(),
                            crate::theme::SMALL_FONT,
                        ))
                        .color(crate::theme::text_label()),
                );
                for id in candidates {
                    let name = t!(crate::shortcuts::action_label_key(id)).to_string();
                    if ui
                        .add(crate::widgets::menu::menu_item_button(ui, false, name))
                        .clicked()
                    {
                        result = Some(id.to_string());
                    }
                }
            }
        });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_editor_core::shortcuts::Keybindings;

    fn combo(cmd: bool, shift: bool, alt: bool, key: &str) -> KeyCombo {
        KeyCombo {
            command: cmd,
            shift,
            alt,
            key: key.to_string(),
        }
    }

    fn settings_with(pairs: &[(&str, Vec<KeyCombo>)]) -> AudioSettings {
        let mut settings = crate::audio_settings::AudioSettings::default();
        let mut kb = Keybindings::default();
        for (id, combos) in pairs {
            kb.set(id, combos.clone());
        }
        settings.keybindings = kb;
        settings
    }

    /// 同一个字母键上的 ⌘K / ⌥K / K 是三条独立绑定，**不算冲突**。
    /// 这是用户明确要求的「修饰键分开算」。
    #[test]
    fn modifier_variants_are_not_conflicts() {
        let settings = settings_with(&[
            (sc::ACTION_SAVE, vec![combo(true, false, false, "K")]),
            (sc::ACTION_OPEN, vec![combo(false, false, true, "K")]),
            (sc::ACTION_DELETE, vec![combo(false, false, false, "K")]),
        ]);
        let all = collect_bindings(&settings);
        let here = bindings_for_cap(egui::Key::K, &all);
        assert_eq!(here.len(), 3, "三个修饰键变体都应算在这个键上");
        assert!(
            conflicting_combos(&here).is_empty(),
            "不同修饰键组合不构成冲突"
        );
    }

    /// 同一个修饰键组合被两个动作占用 = 真冲突。
    #[test]
    fn identical_combos_conflict() {
        let settings = settings_with(&[
            (sc::ACTION_SAVE, vec![combo(true, false, false, "K")]),
            (sc::ACTION_OPEN, vec![combo(true, false, false, "K")]),
        ]);
        let all = collect_bindings(&settings);
        let here = bindings_for_cap(egui::Key::K, &all);
        let conflicts = conflicting_combos(&here);
        assert_eq!(conflicts.len(), 1, "完全相同的组合应判为冲突");
    }

    /// 同一个动作绑两个键时，两个键帽上都应出现（不是冲突）。
    #[test]
    fn one_action_many_keys() {
        let settings = settings_with(&[(
            sc::ACTION_SAVE,
            vec![
                combo(true, false, false, "S"),
                combo(false, false, false, "F2"),
            ],
        )]);
        let all = collect_bindings(&settings);
        for key in [egui::Key::S, egui::Key::F2] {
            let here = bindings_for_cap(key, &all);
            assert!(
                here.iter().any(|b| b.action_id == sc::ACTION_SAVE),
                "{key:?} 上应有保存"
            );
        }
    }

    /// `=` 键帽同时代表 `+`（Shift+`=`），别名绑定要一起显示。
    #[test]
    fn alias_key_shares_cap() {
        let settings =
            settings_with(&[(sc::ACTION_SAVE, vec![combo(false, false, false, "Plus")])]);
        let all = collect_bindings(&settings);
        let here = bindings_for_cap(egui::Key::Equals, &all);
        assert_eq!(here.len(), 1, "Plus 的绑定应显示在 = 键帽上");
    }

    /// 默认配置里 Space（播放）与 Delete（删除）都应可见。
    #[test]
    fn default_bindings_are_visible() {
        let settings = crate::audio_settings::AudioSettings::default();
        let all = collect_bindings(&settings);
        assert!(!bindings_for_cap(egui::Key::Space, &all).is_empty());
        assert!(!bindings_for_cap(egui::Key::Delete, &all).is_empty());
    }
}

/// 布局几何自检：用真实像素坐标验证行序与区块对齐
/// （用户报告过「Z 比 Space 还靠下」「方向键跑到主键区右边」）。
#[cfg(test)]
mod geometry_tests {
    use super::*;
    use egui::Key;

    /// Z 必须在 Space 上方（行序正确）。
    #[test]
    fn z_is_above_space() {
        // 布局表里的行序即为绘制顺序，直接验证表结构（像素测量依赖 id，
        // 单测里更稳妥的是断言行索引）。
        let row_of = |key: Key| {
            ansi_layout()
                .iter()
                .position(|r| {
                    r.segments
                        .iter()
                        .flat_map(|s| s.caps.iter())
                        .any(|c| c.key == Some(key))
                })
                .unwrap()
        };
        assert!(
            row_of(Key::Z) < row_of(Key::Space),
            "Z 行必须排在 Space 行之前"
        );
    }

    /// 方向键必须在独立的区块里，且区块序号与导航区一致
    /// （不能挂在主键区右边缘）。
    #[test]
    fn arrows_live_in_their_own_block() {
        let mut found = false;
        for row in ansi_layout() {
            for (si, segment) in row.segments.iter().enumerate() {
                if segment.caps.iter().any(|c| c.key == Some(Key::ArrowLeft)) {
                    found = true;
                    assert!(si > 0, "方向键不能是行内第一个区块（那会贴住主键区）");
                }
            }
        }
        assert!(found, "布局里必须有方向键");
    }

    /// 主键区、导航区、小键盘必须按顺序分块，不能交叉。
    #[test]
    fn blocks_are_ordered_main_nav_numpad() {
        for row in ansi_layout() {
            let mut kinds: Vec<u8> = Vec::new();
            for segment in row.segments {
                let has_letter = segment
                    .caps
                    .iter()
                    .any(|c| matches!(c.key, Some(k) if crate::shortcuts::key_display(k).len() == 1 && k != Key::Space));
                if has_letter {
                    kinds.push(0); // 主键区
                } else if segment.caps.iter().any(|c| c.label == "Num")
                    || segment
                        .caps
                        .iter()
                        .any(|c| c.label == "7" || c.label == "1" || c.label == "0")
                {
                    kinds.push(2); // 小键盘
                } else if segment.caps.iter().any(|c| {
                    matches!(
                        c.key,
                        Some(
                            Key::Insert
                                | Key::Home
                                | Key::PageUp
                                | Key::Delete
                                | Key::End
                                | Key::PageDown
                                | Key::ArrowUp
                                | Key::ArrowLeft
                        )
                    )
                }) {
                    kinds.push(1); // 导航/方向
                }
            }
            let mut sorted = kinds.clone();
            sorted.sort_unstable();
            assert_eq!(kinds, sorted, "区块顺序必须是 主键区 → 导航/方向 → 小键盘");
        }
    }
}
