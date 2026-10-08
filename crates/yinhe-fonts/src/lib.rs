//! 把系统字体注入 egui，并按界面语言选择 CJK 字体，无需嵌入字体即可显示多语言。
//!
//! 逻辑复刻自 mojie-fonts（macOS 上验证过），并扩展到 Windows / Linux：
//! - macOS：SF Pro Text + PingFang / Hiragino Sans / Apple SD Gothic Neo
//! - Windows：Segoe UI + Microsoft YaHei / JhengHei / Yu Gothic UI / Malgun Gothic
//! - Linux：Noto Sans / DejaVu Sans + Noto Sans CJK SC/TC/JP/KR
//!
//! 这些是 egui 在 macOS 上的坑（AppKit/CoreText 会自动处理，egui 需要手工来）：
//! 1. `FamilyName::SansSerif` 在 macOS 上解析成 Arial，想用 San Francisco 必须显式指定。
//!    这里直接用系统变量字体 `.SFNS`（`/System/Library/Fonts/SFNS.ttf`），与其它
//!    macOS 软件同源；用 `FontTweak::coords` 按 UI 字重写 `wght`、并把 `opsz`
//!    固定为 Text 档（17）。下载版 CFF `SF Pro Text` 仅作 SFNS 缺失时的回退——
//!    egui 渲染 CFF 比 CoreText 明显偏细，而 TrueType 的 SFNS 更接近系统观感。
//! 2. font-kit 用数值字重挑不中下载版 `SF Pro Text` 的 Medium，需要手动按实例字重做最近匹配。
//! 3. font-kit 对 `.ttc` 里的中文字体（PingFang 等）报告的 weight 是错的，必须按
//!    PostScript 名字重后缀精确匹配。
//! 4. 日文 Hiragino 同名义字重看起来比拉丁细，需要上调一档。
//! 5. CJK 字体行高与 SF 不一致会被 epaint 的居中项顶高，需要 `FontTweak` 补偿基线。

use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily, FontTweak};
use font_kit::family_name::FamilyName;
use font_kit::handle::Handle;
use font_kit::properties::{Properties, Style, Weight};
use font_kit::source::SystemSource;

/// 可选字重。
pub const WEIGHT_LIGHT: u16 = 300;
pub const WEIGHT_REGULAR: u16 = 400;
pub const WEIGHT_MEDIUM: u16 = 500;
pub const WEIGHT_SEMIBOLD: u16 = 600;
pub const WEIGHT_BOLD: u16 = 700;

/// 支持的界面语言（与 `yinhe-egui/locales/*.yml` 一一对应）。
pub const LOCALES: &[&str] = &[
    "zh-CN", "zh-HK", "zh-TW", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "it-IT",
    "pt-BR", "ru-RU", "pl-PL", "cs-CZ", "tr-TR",
];

/// 界面语言对应的字形体系。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Simplified,
    Traditional,
    Japanese,
    Korean,
}

/// 把 BCP-47 语言标签（如 `zh-Hant-TW`、`ja-JP`）归到字形体系。
pub fn script(locale: &str) -> Script {
    let lower = locale.to_lowercase().replace('_', "-");
    let mut parts = lower.split('-');
    match parts.next().unwrap_or("") {
        "ja" => Script::Japanese,
        "ko" => Script::Korean,
        "zh" => {
            if parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) {
                Script::Traditional
            } else {
                Script::Simplified
            }
        }
        _ => Script::Simplified,
    }
}

/// 把系统字体注入 egui。`weight` 为 UI 字重，`locale` 为 BCP-47 语言标签。
pub fn install(ctx: &Context, weight: u16, locale: &str) {
    ctx.set_fonts(build(weight, locale));
}

/// 构建注入了系统字体的 [`FontDefinitions`]（拆出来便于测试）。
pub fn build(weight: u16, locale: &str) -> FontDefinitions {
    let source = SystemSource::new();
    let mut fonts = FontDefinitions::default();
    let mut preferred = Vec::new();
    let mut cjk = Vec::new();

    if let Some(handle) =
        system_sans_handle(&source, weight).or_else(|| sans_handle(&source, weight))
        && let Some(key) = insert(&mut fonts, "system-sans", handle, sans_tweak(weight))
    {
        preferred.push(key);
    }

    for family in cjk_families(locale) {
        let key = format!("cjk-{}", slug(family));
        let tweak = FontTweak {
            y_offset_factor: cjk_y_offset_factor(family),
            ..Default::default()
        };
        if let Some(handle) = cjk_handle(&source, family, weight)
            && let Some(key) = insert(&mut fonts, &key, handle, tweak)
        {
            preferred.push(key.clone());
            cjk.push(key);
        }
    }

    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        for key in preferred.into_iter().rev() {
            list.insert(0, key);
        }
    }
    if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
        for key in cjk {
            list.push(key);
        }
    }

    fonts
}

/// 字体家族名转成 egui `font_data` 的安全 key。
fn slug(family: &str) -> String {
    family.to_lowercase().replace(' ', "-")
}

/// 按语言决定 CJK 字体优先级：主字体在前，其余语言字体全部作为回退。
fn cjk_families(locale: &str) -> Vec<&'static str> {
    with_primary(primary_cjk(script(locale)), cjk_fallback())
}

fn with_primary(primary: &'static str, all: &[&'static str]) -> Vec<&'static str> {
    let mut list = vec![primary];
    for &family in all {
        if !list.contains(&family) {
            list.push(family);
        }
    }
    list
}

// ── 平台相关的字体家族名 ──

#[cfg(target_os = "macos")]
const SANS_CANDIDATES: &[&str] = &["SF Pro Text"];

#[cfg(target_os = "windows")]
const SANS_CANDIDATES: &[&str] = &["Segoe UI"];

#[cfg(target_os = "linux")]
const SANS_CANDIDATES: &[&str] = &["Noto Sans", "DejaVu Sans"];

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
const SANS_CANDIDATES: &[&str] = &["Arial"];

#[cfg(target_os = "macos")]
fn primary_cjk(script: Script) -> &'static str {
    match script {
        Script::Simplified => "PingFang SC",
        Script::Traditional => "PingFang TC",
        Script::Japanese => "Hiragino Sans",
        Script::Korean => "Apple SD Gothic Neo",
    }
}

#[cfg(target_os = "macos")]
fn cjk_fallback() -> &'static [&'static str] {
    &[
        "PingFang SC",
        "PingFang TC",
        "Apple SD Gothic Neo",
        "Hiragino Sans",
    ]
}

#[cfg(target_os = "windows")]
fn primary_cjk(script: Script) -> &'static str {
    match script {
        Script::Simplified => "Microsoft YaHei",
        Script::Traditional => "Microsoft JhengHei",
        Script::Japanese => "Yu Gothic UI",
        Script::Korean => "Malgun Gothic",
    }
}

#[cfg(target_os = "windows")]
fn cjk_fallback() -> &'static [&'static str] {
    &[
        "Microsoft YaHei",
        "Microsoft JhengHei",
        "Yu Gothic UI",
        "Yu Gothic",
        "Malgun Gothic",
    ]
}

#[cfg(target_os = "linux")]
fn primary_cjk(script: Script) -> &'static str {
    match script {
        Script::Simplified => "Noto Sans CJK SC",
        Script::Traditional => "Noto Sans CJK TC",
        Script::Japanese => "Noto Sans CJK JP",
        Script::Korean => "Noto Sans CJK KR",
    }
}

#[cfg(target_os = "linux")]
fn cjk_fallback() -> &'static [&'static str] {
    &[
        "Noto Sans CJK SC",
        "Noto Sans CJK TC",
        "Noto Sans CJK JP",
        "Noto Sans CJK KR",
        "Source Han Sans SC",
        "WenQuanYi Micro Hei",
        "Droid Sans Fallback",
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn primary_cjk(script: Script) -> &'static str {
    match script {
        Script::Simplified => "Noto Sans CJK SC",
        Script::Traditional => "Noto Sans CJK TC",
        Script::Japanese => "Noto Sans CJK JP",
        Script::Korean => "Noto Sans CJK KR",
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn cjk_fallback() -> &'static [&'static str] {
    &["Noto Sans CJK SC", "Noto Sans CJK TC"]
}

/// CJK 字体的基线校正量（`FontTweak.y_offset_factor`，正值下移）。
///
/// epaint 会把字体行高比主字体（SF）多出来的部分按 0.5 的比例把基线往上顶。
/// Hiragino 的 leading 很大（行高≈1.5em），基线比 SF 高约 3px，所以下移补偿；
/// PingFang 行高恰好接近 SF，无需补偿。Windows / Linux 字体暂不补偿。
fn cjk_y_offset_factor(family: &str) -> f32 {
    match family {
        "Hiragino Sans" => 0.22,
        "Apple SD Gothic Neo" => 0.06,
        _ => 0.0,
    }
}

/// 系统无衬线字体句柄。
///
/// macOS 直接用系统变量字体 `.SFNS`（`/System/Library/Fonts/SFNS.ttf`）：与其它
/// macOS 软件同源，且 egui 对 TrueType 的栅格化比下载版 CFF `SF Pro Text` 更接近
/// CoreText（后者明显偏细）。文件缺失时返回 `None`，由 [`sans_handle`] 回退。
#[cfg(target_os = "macos")]
fn system_sans_handle(_source: &SystemSource, _weight: u16) -> Option<Handle> {
    let path = std::path::PathBuf::from("/System/Library/Fonts/SFNS.ttf");
    path.is_file().then_some(Handle::Path {
        path,
        font_index: 0,
    })
}

#[cfg(not(target_os = "macos"))]
fn system_sans_handle(_source: &SystemSource, _weight: u16) -> Option<Handle> {
    None
}

/// 系统无衬线字体的 `FontTweak`。
///
/// macOS 的 `.SFNS` 是变量字体：按 UI 字重写 `wght`；`opsz`（光学尺寸）固定为
/// Text 档（17）——egui 不会随字号自动调 `opsz`，固定 17 才能让小字号与
/// 系统 UI 的间距一致（默认值 28 会使小字偏挤）。其它平台为普通静态字体，无需变化。
fn sans_tweak(weight: u16) -> FontTweak {
    #[cfg(target_os = "macos")]
    {
        FontTweak {
            coords: egui::epaint::text::VariationCoords::new([
                ("wght", weight as f32),
                ("opsz", 17.0),
            ]),
            ..Default::default()
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = weight;
        FontTweak::default()
    }
}

/// 在候选无衬线家族里挑最接近目标字重的正体实例。
///
/// font-kit 的 `select_best_match` 用数值字重挑不中 Medium，所以手动做最近匹配。
/// 候选家族都拿不到时退回系统泛型无衬线。
fn sans_handle(source: &SystemSource, weight: u16) -> Option<Handle> {
    for name in SANS_CANDIDATES {
        if let Ok(family) = source.select_family_by_name(name)
            && let Some(handle) = nearest_normal(family.fonts(), weight)
        {
            return Some(handle);
        }
    }

    let properties = Properties {
        weight: Weight(weight as f32),
        ..Properties::new()
    };
    source
        .select_best_match(&[FamilyName::SansSerif], &properties)
        .ok()
}

/// 在一组字体句柄里挑 `Style::Normal` 且字重最接近 `weight` 的一个。
fn nearest_normal(handles: &[Handle], weight: u16) -> Option<Handle> {
    let mut best: Option<(f32, Handle)> = None;
    for handle in handles {
        let Ok(font) = font_kit::font::Font::from_handle(handle) else {
            continue;
        };
        if font.properties().style != Style::Normal {
            continue;
        }
        let diff = (font.properties().weight.0 - weight as f32).abs();
        if best.as_ref().is_none_or(|(best_diff, _)| diff < *best_diff) {
            best = Some((diff, handle.clone()));
        }
    }
    best.map(|(_, handle)| handle)
}

/// 在 CJK 家族里挑目标字重的正体面。
///
/// 注意：font-kit 对 .ttc 里的中文字体（PingFang 等）报告的 weight 是错的，
/// 所以这里优先按 PostScript 名的字重后缀精确匹配，匹配不到再退回最近字重。
fn cjk_handle(source: &SystemSource, family_name: &str, weight: u16) -> Option<Handle> {
    let family = source.select_family_by_name(family_name).ok()?;
    let hints = weight_hints(family_name, weight);
    let mut regular = None;
    let mut nearest: Option<(f32, Handle)> = None;

    for handle in family.fonts() {
        let Ok(font) = font_kit::font::Font::from_handle(handle) else {
            continue;
        };
        if font.properties().style != Style::Normal {
            continue;
        }
        let postscript = font.postscript_name().unwrap_or_default();
        let suffix = postscript
            .rsplit('-')
            .next()
            .unwrap_or(&postscript)
            .to_lowercase();
        if hints.contains(&suffix.as_str()) {
            return Some(handle.clone());
        }
        if suffix == "regular" {
            regular = Some(handle.clone());
        }
        let diff = (font.properties().weight.0 - weight as f32).abs();
        if nearest.as_ref().is_none_or(|(best, _)| diff < *best) {
            nearest = Some((diff, handle.clone()));
        }
    }

    regular.or(nearest.map(|(_, handle)| handle))
}

/// 各字重对应的 PostScript 名后缀候选（Hiragino 用 W 编号）。
///
/// **只有日文 Hiragino** 同名义字重看起来偏细，整体上调一档匹配拉丁；
/// 中文（PingFang）等按原字重选择。
fn weight_hints(family: &str, weight: u16) -> &'static [&'static str] {
    if family == "Hiragino Sans" {
        match weight {
            WEIGHT_LIGHT => &["regular", "w3"],
            WEIGHT_REGULAR => &["medium", "w4"],
            WEIGHT_MEDIUM => &["semibold", "w5"],
            WEIGHT_SEMIBOLD => &["bold", "w6", "semibold"],
            WEIGHT_BOLD => &["heavy", "w7", "bold", "semibold"],
            _ => &["regular", "w3"],
        }
    } else {
        match weight {
            WEIGHT_LIGHT => &["light"],
            WEIGHT_REGULAR => &["regular", "w3"],
            WEIGHT_MEDIUM => &["medium", "w4"],
            WEIGHT_SEMIBOLD => &["semibold", "w5"],
            WEIGHT_BOLD => &["bold", "w6", "semibold"],
            _ => &["regular", "w3"],
        }
    }
}

fn insert(
    fonts: &mut FontDefinitions,
    key: &str,
    handle: Handle,
    tweak: FontTweak,
) -> Option<String> {
    let (bytes, index) = match handle {
        Handle::Path { path, font_index } => (std::fs::read(path).ok()?, font_index),
        Handle::Memory { bytes, font_index } => (bytes.as_ref().clone(), font_index),
    };
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    data = data.tweak(tweak);
    fonts.font_data.insert(key.to_owned(), Arc::new(data));
    Some(key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_maps_all_locales() {
        assert_eq!(script("zh-CN"), Script::Simplified);
        assert_eq!(script("zh-Hans"), Script::Simplified);
        assert_eq!(script("zh-Hans-CN"), Script::Simplified);
        assert_eq!(script("zh_TW"), Script::Traditional);
        assert_eq!(script("zh-HK"), Script::Traditional);
        assert_eq!(script("zh-Hant"), Script::Traditional);
        assert_eq!(script("ja-JP"), Script::Japanese);
        assert_eq!(script("ja"), Script::Japanese);
        assert_eq!(script("ko-KR"), Script::Korean);
        assert_eq!(script("fr-FR"), Script::Simplified);
        assert_eq!(script("en-US"), Script::Simplified);
    }

    #[test]
    fn cjk_primary_is_first_and_no_duplicate() {
        for (locale, expected) in [
            ("zh-CN", primary_cjk(Script::Simplified)),
            ("zh-TW", primary_cjk(Script::Traditional)),
            ("ja-JP", primary_cjk(Script::Japanese)),
            ("ko-KR", primary_cjk(Script::Korean)),
        ] {
            let list = cjk_families(locale);
            assert_eq!(
                list.first().copied(),
                Some(expected),
                "{locale} 主字体不在首位"
            );
            let mut uniq = list.clone();
            uniq.sort_unstable();
            uniq.dedup();
            assert_eq!(uniq.len(), list.len(), "{locale} 回退列表有重复");
        }
    }

    #[test]
    fn all_locales_are_covered() {
        assert_eq!(LOCALES.len(), 15);
        for locale in LOCALES {
            let list = cjk_families(locale);
            assert!(!list.is_empty(), "{locale} 没有可用 CJK 字体");
        }
    }

    #[test]
    fn built_fonts_all_parse() {
        for locale in LOCALES {
            let defs = build(WEIGHT_REGULAR, locale);
            for key in &defs.families[&FontFamily::Proportional] {
                let data = &defs.font_data[key];
                ab_glyph::FontVec::try_from_vec_and_index(data.font.to_vec(), data.index)
                    .unwrap_or_else(|err| panic!("{locale}: 字体 {key} 解析失败: {err:?}"));
            }
        }
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;

        fn postscript_of(handle: &Handle) -> String {
            font_kit::font::Font::from_handle(handle)
                .map(|font| font.postscript_name().unwrap_or_default())
                .unwrap_or_default()
        }

        #[test]
        fn selects_requested_sf_weights() {
            let source = SystemSource::new();
            for (weight, expected) in [
                (WEIGHT_LIGHT, "Light"),
                (WEIGHT_REGULAR, "Regular"),
                (WEIGHT_MEDIUM, "Medium"),
                (WEIGHT_SEMIBOLD, "Semibold"),
                (WEIGHT_BOLD, "Bold"),
            ] {
                let handle = sans_handle(&source, weight).expect("挑不到 SF 字重");
                let name = postscript_of(&handle);
                assert!(
                    name.contains(expected),
                    "SF 字重 {weight} 期望 {expected}，实际 {name}"
                );
            }
        }

        #[test]
        fn selects_requested_cjk_weights() {
            let source = SystemSource::new();
            for (weight, expected) in [
                (WEIGHT_LIGHT, "Light"),
                (WEIGHT_REGULAR, "Regular"),
                (WEIGHT_MEDIUM, "Medium"),
                (WEIGHT_SEMIBOLD, "Semibold"),
            ] {
                let handle = cjk_handle(&source, "PingFang SC", weight).expect("挑不到苹方字重");
                let name = postscript_of(&handle);
                assert!(
                    name.contains(expected),
                    "苹方字重 {weight} 期望 {expected}，实际 {name}"
                );
            }
        }

        #[test]
        fn selects_hiragino_weights() {
            let source = SystemSource::new();
            for (weight, expected) in [
                (WEIGHT_LIGHT, "W3"),
                (WEIGHT_REGULAR, "W4"),
                (WEIGHT_MEDIUM, "W5"),
                (WEIGHT_SEMIBOLD, "W6"),
                (WEIGHT_BOLD, "W7"),
            ] {
                let handle = cjk_handle(&source, "Hiragino Sans", weight).expect("挑不到 Hiragino");
                let name = postscript_of(&handle);
                assert!(
                    name.contains(expected),
                    "Hiragino 字重 {weight} 期望 {expected}，实际 {name}"
                );
            }
        }

        #[test]
        fn japanese_prefers_hiragino() {
            let defs = build(WEIGHT_REGULAR, "ja-JP");
            let proportional = &defs.families[&FontFamily::Proportional];
            let hiragino = proportional.iter().position(|key| key.contains("hiragino"));
            let pingfang = proportional
                .iter()
                .position(|key| key.contains("pingfang-sc"));
            assert!(hiragino.is_some(), "日文缺少 Hiragino");
            assert!(hiragino < pingfang, "日文 Hiragino 应排在 PingFang SC 之前");
            assert_eq!(
                proportional.first().map(String::as_str),
                Some("system-sans")
            );
        }

        /// CJK 字形基线应与拉丁一致（补偿生效，`y_offset` 只移动字形图像）。
        #[test]
        fn cjk_baseline_matches_latin() {
            for (locale, sample) in [
                ("ja-JP", 'あ'),
                ("ko-KR", '가'),
                ("zh-CN", '字'),
                ("zh-TW", '字'),
            ] {
                let offset = (16.0 * cjk_y_offset_factor(cjk_families(locale)[0])).round();
                let ctx = Context::default();
                ctx.set_fonts(build(WEIGHT_REGULAR, locale));
                let mut latin = f32::NAN;
                let mut cjk = f32::NAN;
                let mut output = ctx.run_ui(Default::default(), |ctx| {
                    let galley = ctx.fonts_mut(|fonts| {
                        fonts.layout_no_wrap(
                            format!("A{sample}"),
                            egui::FontId::proportional(16.0),
                            egui::Color32::WHITE,
                        )
                    });
                    for glyph in &galley.rows[0].glyphs {
                        if glyph.chr == 'A' {
                            latin = glyph.pos.y;
                        } else if glyph.chr == sample {
                            cjk = glyph.pos.y + offset;
                        }
                    }
                });
                output.textures_delta.clear();
                assert!(
                    (latin - cjk).abs() <= 1.0,
                    "{locale}: 拉丁基线 {latin:.2} 与 CJK 有效基线 {cjk:.2} 不一致"
                );
            }
        }
    }
}
