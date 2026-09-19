use deunicode::deunicode_char;
use eframe::egui;
use pinyin::ToPinyinMulti;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;

use super::appearance::show_appearance_tab;
use super::audio::show_audio_tab;
use super::constants::{CATEGORY_KEYS, SETTING_ITEMS, SettingItem};
use super::editing::show_editing_tab;
use super::general::show_general_tab;
use super::language::show_language_tab;
use super::midi_export::show_midi_export_tab;
use super::notification::show_notification_tab;
use super::render::show_render_tab;
use super::saving::show_saving_tab;
use super::shortcuts::show_shortcuts_tab;
use super::soundfont::show_soundfont_tab;
use super::theme::show_theme_tab;

/// 归一化：小写并去掉所有空白（拼音/罗马音的带空格输入也能匹配）。
pub fn norm(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// 把任意语言文本折叠成检索键：`(全拼/原文变体, 首字母缩写变体)`。
///
/// 多音字展开全部读音组合（「去重」→ quchong/quzhong，缩写 qc/qz），
/// 任一读音都能命中；非汉字不参与首字母缩写（走原文匹配）。
pub fn to_search_keys(s: &str) -> (Vec<String>, Vec<String>) {
    let mut fulls = vec![String::new()];
    let mut initials = vec![String::new()];
    for c in s.chars() {
        let mut plains: Vec<String> = Vec::new();
        let mut heads: Vec<String> = Vec::new();
        if let Some(multi) = c.to_pinyin_multi() {
            for py in multi {
                let p = py.plain().replace('ü', "v");
                if !plains.contains(&p) {
                    plains.push(p);
                }
                let h = py.first_letter().to_string();
                if !heads.contains(&h) {
                    heads.push(h);
                }
            }
        } else if let Some(lat) = deunicode_char(c) {
            plains.push(lat.to_string());
        } else if !c.is_whitespace() {
            plains.push(c.to_string());
        }
        if !plains.is_empty() {
            fulls = cross_join(&fulls, &plains);
        }
        if !heads.is_empty() {
            initials = cross_join(&initials, &heads);
        }
    }
    (fulls.into_iter().map(|v| norm(&v)).collect(), initials)
}

/// 前缀集合 × 候选读音集合的笛卡尔积（每个候选追加到每个前缀后）。
fn cross_join(prefixes: &[String], suffixes: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(prefixes.len() * suffixes.len());
    for p in prefixes {
        for s in suffixes {
            out.push(format!("{p}{s}"));
        }
    }
    out
}

/// 搜索词是否匹配设置项：任意语言名称原文、中文拼音（含多音字）或其首字母缩写。
pub fn item_matches(item: &SettingItem, query: &str) -> bool {
    let q = norm(query);
    let (q_fulls, _) = to_search_keys(query);
    [item.zh, item.en, item.ja, item.ko].iter().any(|name| {
        let (fulls, initials) = to_search_keys(name);
        fulls
            .iter()
            .any(|f| q_fulls.iter().any(|qf| !qf.is_empty() && f.contains(qf)))
            || initials
                .iter()
                .any(|i| !i.is_empty() && !q.is_empty() && i.contains(&q))
    })
}

/// 右侧内容：有搜索词时显示跨分类搜索结果，否则显示当前分类。
pub fn show_search_results(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let query = settings.settings_search.trim().to_string();
    if query.is_empty() {
        return match settings.settings_tab {
            0 => show_theme_tab(ui, settings, main_ctx),
            1 => show_appearance_tab(ui, settings, main_ctx),
            2 => show_language_tab(ui, settings),
            3 => show_audio_tab(ui, settings),
            4 => show_render_tab(ui, settings),
            5 => show_midi_export_tab(ui, settings),
            6 => show_editing_tab(ui, settings),
            7 => show_shortcuts_tab(ui, settings),
            8 => show_notification_tab(ui, settings),
            10 => show_saving_tab(ui, settings),
            11 => show_soundfont_tab(ui, settings),
            _ => show_general_tab(ui, settings),
        };
    }

    ui.heading(t!("settings.search_results").as_ref());
    ui.add_space(6.0);
    let mut matched = 0usize;
    for item in SETTING_ITEMS {
        if !item_matches(item, &query) {
            continue;
        }
        matched += 1;
        let cat = item.cat;
        let cat_name = t!(CATEGORY_KEYS[cat]).to_string();
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                format!("{}  ·  {}", cat_name, item.zh),
            ))
            .clicked()
        {
            settings.settings_tab = cat;
            settings.settings_search.clear();
        }
    }
    if matched == 0 {
        ui.colored_label(
            crate::theme::text_disabled(),
            t!("settings.search_none").as_ref(),
        );
    }
    false
}
