//! 把系统字体注入 egui：主字体用系统 UI 字体，中文按界面语言选一个首选 CJK
//! 字体，其余缺失字形交给 eframe 的 `SystemFontProvider` 按需发现。
//!
//! egui 0.37 起，系统字体的按需回退（含彩色 emoji、任意文字系统）由 eframe 内置
//! 的 `egui_system_font_provider` 负责，不再需要在这里枚举所有 CJK 回退字体。
//! 这里只负责三件它不负责的事：
//! 1. 选主字体：egui 默认主字体是 Ubuntu-Light，不是系统 UI 字体；
//! 2. 字重：回退字体按 Regular 加载，CJK 主字体需要按 UI 字重单独选；
//! 3. 日文假名：fontique 的 Hiragana 回退在 macOS 上会选到 PingFang，
//!    用日文 Hani 回退字体（Hiragino）同时覆盖汉字和假名。

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::{Context, FontData, FontDefinitions, FontFamily, FontTweak};
use fontique::{
    Attributes, Collection, CollectionOptions, FallbackKey, FontWeight, GenericFamily, Language,
    QueryFamily, QueryStatus, Script, SourceCache, SourceCacheOptions,
};

/// UI 字重（当前固定 Medium）。
pub const WEIGHT_MEDIUM: u16 = 500;

/// 把系统字体注入 egui。`weight` 为 UI 字重，`locale` 为 BCP-47 语言标签。
pub fn install(ctx: &Context, weight: u16, locale: &str) {
    ctx.set_fonts(build(weight, locale));
}

/// 构建注入了系统字体的 [`FontDefinitions`]（拆出来便于测试）。
pub fn build(weight: u16, locale: &str) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut cache = SourceCache::new(SourceCacheOptions::default());
    let language = Language::parse(locale).ok();

    let mut preferred = Vec::new();

    if let Some(font) = query_primary(&mut collection, &mut cache, weight) {
        let key = "system-sans".to_owned();
        insert(&mut fonts, &key, font, 0.0);
        preferred.push(key);
    }

    if let Some(font) = query_cjk(&mut collection, &mut cache, language.as_ref(), weight) {
        let key = format!("cjk-{}", slug(&font.family));
        let offset = cjk_y_offset_factor(&font.family);
        insert(&mut fonts, &key, font, offset);
        preferred.push(key);
    }

    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        for key in preferred.into_iter().rev() {
            list.insert(0, key);
        }
    }

    fonts
}

struct Picked {
    family: String,
    blob: fontique::Blob<u8>,
    index: u32,
    coords: VariationCoords,
}

/// 系统 UI 字体（macOS 的 SF Pro Text、Windows 的 Segoe UI、Linux 由 fontconfig 决定）。
///
/// macOS 的 `System Font` 是 `.SFNS` 可变字体（默认光学尺寸偏 Display）；
/// 界面小字号应使用 `SF Pro Text` 这一族，与旧实现保持一致。
fn query_primary(
    collection: &mut Collection,
    cache: &mut SourceCache,
    weight: u16,
) -> Option<Picked> {
    #[cfg(target_os = "macos")]
    if let Some(picked) =
        query_primary_family(collection, cache, QueryFamily::Named("SF Pro Text"), weight)
    {
        return Some(picked);
    }

    query_primary_family(
        collection,
        cache,
        QueryFamily::Generic(GenericFamily::SystemUi),
        weight,
    )
    .or_else(|| {
        query_primary_family(
            collection,
            cache,
            QueryFamily::Generic(GenericFamily::SansSerif),
            weight,
        )
    })
}

fn query_primary_family(
    collection: &mut Collection,
    cache: &mut SourceCache,
    family: QueryFamily<'static>,
    weight: u16,
) -> Option<Picked> {
    let mut candidates = Vec::new();
    {
        let mut query = collection.query(cache);
        query.set_families([family]);
        query.set_attributes(attributes(weight));
        query.matches_with(|font| {
            candidates.push((
                font.family.0,
                font.family.1,
                font.blob.clone(),
                font.index,
                font.synthesis,
            ));
            QueryStatus::Continue
        });
    }
    // 同一家族的斜体面可能排在正体前面，挑第一个正体。
    let (family_id, _family_index, blob, index, synthesis) = candidates.into_iter().find(|c| {
        collection
            .family(c.0)
            .and_then(|family| family.fonts().get(c.1).map(|font| font.style()))
            == Some(fontique::FontStyle::Normal)
    })?;
    let family = collection
        .family_name(family_id)
        .unwrap_or("system-ui")
        .to_owned();
    Some(Picked {
        family,
        blob,
        index,
        coords: coords(synthesis),
    })
}

/// 按语言选首选 CJK 字体。
///
/// 用 Hani 回退查询而不是写死字体名：fontique 会按 locale 在
/// PingFang SC/TC、Hiragino、Apple SD Gothic Neo 等系统字体中选出正确的那个。
/// 日文假名也走 Hani 查询，避免 fontique 把假名回退到 PingFang。
fn query_cjk(
    collection: &mut Collection,
    cache: &mut SourceCache,
    language: Option<&Language>,
    weight: u16,
) -> Option<Picked> {
    let mut picked = None;
    {
        let mut query = collection.query(cache);
        query.set_families([QueryFamily::Generic(GenericFamily::SansSerif)]);
        query.set_fallbacks(FallbackKey::new(Script::from_bytes(*b"Hani"), language));
        query.set_attributes(attributes(weight));
        query.matches_with(|font| {
            let has_glyph = font
                .charmap()
                .and_then(|charmap| charmap.map('字'))
                .is_some();
            if has_glyph {
                picked = Some((font.family.0, font.blob.clone(), font.index, font.synthesis));
                QueryStatus::Stop
            } else {
                QueryStatus::Continue
            }
        });
    }
    let (family_id, blob, index, synthesis) = picked?;
    let family = collection
        .family_name(family_id)
        .unwrap_or("cjk")
        .to_owned();
    Some(Picked {
        family,
        blob,
        index,
        coords: coords(synthesis),
    })
}

fn attributes(weight: u16) -> Attributes {
    Attributes {
        weight: FontWeight::new(weight as f32),
        ..Attributes::default()
    }
}

fn coords(synthesis: fontique::Synthesis) -> VariationCoords {
    VariationCoords::new(
        synthesis
            .variation_settings()
            .iter()
            .map(|(tag, value)| (tag.to_be_bytes(), *value)),
    )
}

/// CJK 字体的基线校正量（`FontTweak.y_offset_factor`，正值下移）。
///
/// epaint 会把字体行高比主字体（SF）多出来的部分按 0.5 的比例把基线往上顶。
/// Hiragino 的 leading 很大（行高≈1.5em），基线比 SF 高约 3px，所以下移补偿；
/// Apple SD Gothic Neo 行高略大，做小幅补偿。PingFang 与 SF 基本一致。
fn cjk_y_offset_factor(family: &str) -> f32 {
    match family {
        "Hiragino Sans" => 0.22,
        "Apple SD Gothic Neo" => 0.06,
        _ => 0.0,
    }
}

/// 字体家族名转成 egui `font_data` 的安全 key。
fn slug(family: &str) -> String {
    family.to_lowercase().replace(' ', "-")
}

fn insert(fonts: &mut FontDefinitions, key: &str, font: Picked, y_offset_factor: f32) {
    let (blob, _) = font.blob.into_raw_parts();
    let mut data = FontData::from_blob(blob, font.index);
    data.tweak = FontTweak {
        coords: font.coords,
        y_offset_factor,
        ..FontTweak::default()
    };
    fonts.font_data.insert(key.to_owned(), Arc::new(data));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 支持的界面语言（与 `yinhe-egui/locales/*.yml` 一一对应）。
    const LOCALES: &[&str] = &[
        "zh-CN", "zh-HK", "zh-TW", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "it-IT",
        "pt-BR", "ru-RU", "pl-PL", "cs-CZ", "tr-TR",
    ];

    #[test]
    fn all_locales_build() {
        for locale in LOCALES {
            let defs = build(WEIGHT_MEDIUM, locale);
            let proportional = &defs.families[&FontFamily::Proportional];
            assert_eq!(
                proportional.first().map(String::as_str),
                Some("system-sans"),
                "{locale}"
            );
            for key in proportional {
                assert!(defs.font_data.contains_key(key), "{locale}: 缺字体 {key}");
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn picks_locale_specific_cjk() {
        for (locale, expected) in [
            ("zh-CN", "PingFang SC"),
            ("zh-HK", "PingFang HK"),
            ("zh-TW", "PingFang TC"),
            ("ja-JP", "Hiragino Sans"),
            ("ko-KR", "Apple SD Gothic Neo"),
        ] {
            let defs = build(WEIGHT_MEDIUM, locale);
            let keys = &defs.families[&FontFamily::Proportional];
            let cjk = &keys[1];
            assert!(
                cjk.contains(&slug(expected)),
                "{locale}: 期望 {expected}，实际 {cjk}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn primary_is_medium_weight() {
        let defs = build(WEIGHT_MEDIUM, "zh-CN");
        let data = &defs.font_data["system-sans"];
        let font = skrifa::FontRef::from_index(data.bytes(), data.index).unwrap();
        use skrifa::MetadataProvider as _;
        let name: String = font
            .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
            .english_or_first()
            .unwrap()
            .chars()
            .collect();
        assert!(name.contains("Medium"), "主字体不是 Medium：{name}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cjk_baseline_matches_latin() {
        for (locale, sample) in [
            ("ja-JP", 'あ'),
            ("ko-KR", '가'),
            ("zh-CN", '字'),
            ("zh-TW", '字'),
        ] {
            let defs = build(WEIGHT_MEDIUM, locale);
            let offset = if let Some(key) = defs.families[&FontFamily::Proportional].get(1) {
                (16.0 * defs.font_data[key].tweak.y_offset_factor).round()
            } else {
                0.0
            };
            let ctx = Context::default();
            ctx.set_fonts(defs);
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
