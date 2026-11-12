//! 界面字体注入：主字体（系统 UI 字体 + Medium 字重）+ 系统字体按需回退 + Material 图标。
//!
//! egui 0.37 起，任意文字的按需回退（CJK / 阿拉伯文 / 彩色 emoji）由
//! `egui_system_font_provider` 负责，它用系统自己的回退表（CoreText / DirectWrite /
//! fontconfig / Android fonts.xml）找字体，不需要我们枚举回退字体。
//!
//! 这里只保留两件 provider 不负责的事：
//! 1. **主字体**：egui 默认是 Ubuntu-Light，不是系统 UI 字体；
//! 2. **字重**：回退字体一律 Regular，主字体要按 UI 字重（Medium）单独查。
//!
//! 语言只影响 provider 的 locale（决定同一脚本在简繁日韩之间选哪个字体），
//! 所以切换语言时重建 provider 即可，不需要重新查询 CJK 字体。
//!
//! 字体与 Material 图标必须一次性通过 `set_fonts` 注入：`add_font` 与 `set_fonts`
//! 走不同队列，切换语言时再次 `set_fonts` 会重建 `FontDefinitions`，之前 `add_font`
//! 的图标会丢失，导致所有图标变成方框。两者合进同一份定义。

use std::sync::Arc;

use egui::epaint::text::{FontPriority, VariationCoords};
use egui::{Context, FontData, FontDefinitions, FontFamily, FontTweak};
use fontique::{
    Attributes, Collection, CollectionOptions, FallbackKey, FontWeight, GenericFamily, Language,
    QueryFamily, QueryStatus, Script, SourceCache, SourceCacheOptions,
};

/// UI 字重（当前固定 Medium）。
pub(crate) const WEIGHT_MEDIUM: u16 = 500;

/// 把系统字体注入 egui：主字体 + 按语言选的 CJK 字体 + 系统回退 + Material 图标。
///
/// `weight` 为 UI 字重，`locale` 为 BCP-47 语言标签。
pub(crate) fn install(ctx: &Context, weight: u16, locale: &str) {
    let mut defs = build(weight, locale);

    // filled + outlined 两个字体家族（outlined 用于收藏 star 等空心图标）。
    for mut insert in [
        egui_material_icons::font_insert(),
        egui_material_icons::font_insert_outlined(),
    ] {
        // 默认 y_offset_factor=0.05 会让图标偏下，置 0 居中。
        insert.data.tweak.y_offset_factor = 0.0;
        for family in insert.families {
            let list = defs.families.entry(family.family).or_default();
            match family.priority {
                FontPriority::Highest => list.insert(0, insert.name.clone()),
                FontPriority::Lowest => list.push(insert.name.clone()),
            }
        }
        defs.font_data
            .insert(insert.name.clone(), Arc::new(insert.data));
    }

    ctx.set_fonts(defs);

    // 回退按 UI 语言选字体：同一脚本（Han / 假名）在简繁日韩之间要选对，
    // 系统 locale 未必等于界面语言。
    ctx.set_font_providers(vec![Arc::new(
        egui_system_font_provider::SystemFontProvider::new().with_locale(Some(locale)),
    )]);
}

/// 构建字体定义：系统主字体 + 按语言选的首选 CJK 字体（拆出来便于测试）。
///
/// CJK 主字体**必须**在这里显式注入，不能只靠 `SystemFontProvider`：
/// provider 一律按 Regular 加载回退字体，中文/日文会比西文明显偏细；
/// 而且 provider 不做基线补偿，日文 Hiragino 的 leading 偏大会让假名上移。
/// 其余字形（其它文字系统、emoji）仍交给 provider 按需发现。
pub(crate) fn build(weight: u16, locale: &str) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut cache = SourceCache::new(SourceCacheOptions::default());
    let language = Language::parse(locale).ok();

    let mut preferred: Vec<String> = Vec::new();

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
    /// 字体家族名（CJK 基线补偿按家族名查表）。
    family: String,
    blob: fontique::Blob<u8>,
    index: u32,
    coords: VariationCoords,
}

/// 按界面语言选首选 CJK 字体。
///
/// 用 Hani 回退查询而不是写死字体名：fontique 会按 locale 在
/// PingFang SC/TC、Hiragino、Apple SD Gothic Neo 等系统字体中选出正确的那个。
/// 日文假名也走 Hani 查询，避免 fontique 把假名回退到 PingFang
/// （PingFang 没有假名字形，会逐字回落成别的字体，假名看起来忽大忽小）。
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

/// 系统 UI 字体（macOS 的 SF Pro Text、Windows 的 Segoe UI、Linux 由 fontconfig 决定）。
///
/// macOS 的 `System Font` 是 `.SFNS` 可变字体（默认光学尺寸偏 Display）；
/// 界面小字号应使用 `SF Pro Text` 这一族。
fn query_primary(
    collection: &mut Collection,
    cache: &mut SourceCache,
    weight: u16,
) -> Option<Picked> {
    #[cfg(target_os = "macos")]
    if let Some(picked) = query_family(collection, cache, QueryFamily::Named("SF Pro Text"), weight)
    {
        return Some(picked);
    }

    query_family(
        collection,
        cache,
        QueryFamily::Generic(GenericFamily::SystemUi),
        weight,
    )
    .or_else(|| {
        query_family(
            collection,
            cache,
            QueryFamily::Generic(GenericFamily::SansSerif),
            weight,
        )
    })
}

fn query_family(
    collection: &mut Collection,
    cache: &mut SourceCache,
    family: QueryFamily<'static>,
    weight: u16,
) -> Option<Picked> {
    let mut candidates = Vec::new();
    {
        let mut query = collection.query(cache);
        query.set_families([family]);
        query.set_attributes(Attributes {
            weight: FontWeight::new(weight as f32),
            ..Attributes::default()
        });
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
    let (family_id, _, blob, index, synthesis) = candidates.into_iter().find(|c| {
        collection
            .family(c.0)
            .and_then(|family| family.fonts().get(c.1).map(|font| font.style()))
            == Some(fontique::FontStyle::Normal)
    })?;
    Some(Picked {
        family: collection
            .family_name(family_id)
            .unwrap_or("system-ui")
            .to_owned(),
        blob,
        index,
        coords: coords(synthesis),
    })
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

    /// 支持的界面语言（与 `locales/*.yml` 一一对应）。
    const LOCALES: &[&str] = &[
        "zh-CN", "zh-HK", "zh-TW", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "it-IT",
        "pt-BR", "ru-RU", "pl-PL", "cs-CZ", "tr-TR",
    ];

    /// 主字体必须是系统 UI 字体且排在 Proportional 最前（回退由 provider 负责）。
    #[test]
    fn primary_is_first_and_resolvable() {
        let defs = build(WEIGHT_MEDIUM, "zh-CN");
        let proportional = &defs.families[&FontFamily::Proportional];
        assert_eq!(
            proportional.first().map(String::as_str),
            Some("system-sans")
        );
        assert!(defs.font_data.contains_key("system-sans"));
    }

    /// CJK 字体必须**显式注入**且排在主字体之后（provider 一律按 Regular 加载，
    /// 不注入的话中文/日文会比西文明显偏细）。
    #[test]
    fn cjk_font_is_injected_after_primary() {
        for locale in LOCALES {
            let defs = build(WEIGHT_MEDIUM, locale);
            let proportional = &defs.families[&FontFamily::Proportional];
            assert!(
                proportional.len() >= 2,
                "{locale}: 必须同时注入主字体与 CJK 字体"
            );
            assert_eq!(
                proportional.first().map(String::as_str),
                Some("system-sans"),
                "{locale}: 主字体排第一"
            );
            let cjk_key = &proportional[1];
            assert!(
                defs.font_data.contains_key(cjk_key),
                "{locale}: CJK 字体 {cjk_key} 未注入"
            );
        }
    }

    /// 每个语言都能挂上 provider（locale 解析失败也不 panic，只是用系统默认）。
    #[test]
    fn provider_accepts_all_locales() {
        for locale in LOCALES {
            let _ = egui_system_font_provider::SystemFontProvider::new().with_locale(Some(locale));
        }
    }

    /// CJK / 假名必须能布局出实际宽度，否则界面上会变成方框或空白。
    #[test]
    fn cjk_has_real_glyph_width() {
        for (locale, sample) in [
            ("zh-CN", '字'),
            ("zh-TW", '字'),
            ("ja-JP", 'あ'),
            ("ko-KR", '가'),
        ] {
            let ctx = Context::default();
            install(&ctx, WEIGHT_MEDIUM, locale);
            let mut width = 0.0f32;
            let mut output = ctx.run_ui(Default::default(), |ctx| {
                let galley = ctx.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(
                        sample.to_string(),
                        egui::FontId::proportional(16.0),
                        egui::Color32::WHITE,
                    )
                });
                width = galley.size().x;
            });
            output.textures_delta.clear();
            assert!(width > 4.0, "{locale}: {sample} 没有字形，宽度 {width}");
        }
    }

    /// 字重回归：CJK 字体也必须按 UI 字重选面（provider 只给 Regular，
    /// 所以必须显式注入 Medium 面，否则中文看起来比西文细一档）。
    #[cfg(target_os = "macos")]
    #[test]
    fn cjk_font_is_also_medium_weight() {
        for (locale, family) in [("zh-CN", "PingFang"), ("ja-JP", "Hiragino")] {
            let defs = build(WEIGHT_MEDIUM, locale);
            let cjk_key = &defs.families[&FontFamily::Proportional][1];
            assert!(
                cjk_key.contains(&family.to_lowercase()),
                "{locale}: 期望 {family}，实际 {cjk_key}"
            );
            // 可变字体用 variation coords 表达字重；静态面则文件名/家族名带字重。
            let data = &defs.font_data[cjk_key];
            let resolved = skrifa::FontRef::from_index(data.bytes(), data.index)
                .ok()
                .and_then(|font| {
                    use skrifa::MetadataProvider as _;
                    font.localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
                        .english_or_first()
                        .map(|s| s.chars().collect::<String>())
                })
                .unwrap_or_default();
            let has_variable_weight = !data.tweak.coords.as_ref().is_empty();
            let named_medium = resolved.to_ascii_lowercase().contains("medium")
                || resolved.to_ascii_lowercase().contains("w5");
            assert!(
                has_variable_weight || named_medium,
                "{locale}: CJK 字体 {cjk_key}（{resolved}）不是 Medium 字重"
            );
        }
    }

    /// 回归：日文 Hiragino 的 leading 偏大，epaint 会把基线上顶约 3px。
    /// `cjk_y_offset_factor` 是唯一的补偿来源，删掉它假名就会上移。
    #[cfg(target_os = "macos")]
    #[test]
    fn japanese_baseline_is_compensated() {
        let defs = build(WEIGHT_MEDIUM, "ja-JP");
        let cjk_key = &defs.families[&FontFamily::Proportional][1];
        assert!(cjk_key.contains("hiragino"), "日文应选 Hiragino：{cjk_key}");
        assert!(
            defs.font_data[cjk_key].tweak.y_offset_factor > 0.1,
            "日文 CJK 字体必须带基线下移补偿"
        );

        // epaint 在布局阶段不应用 y_offset_factor（它只在绘制时抬基线），
        // 所以这里按同样比例把补偿量加到假名位置上再比较。
        let offset = (16.0 * defs.font_data[cjk_key].tweak.y_offset_factor).round();
        let ctx = Context::default();
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(Default::default(), |ctx| {
            let (latin, kana) = ctx.fonts_mut(|fonts| {
                let galley = fonts.layout_no_wrap(
                    "Aあ".to_string(),
                    egui::FontId::proportional(16.0),
                    egui::Color32::WHITE,
                );
                let mut latin = f32::NAN;
                let mut kana = f32::NAN;
                for glyph in &galley.rows[0].glyphs {
                    if glyph.chr == 'A' {
                        latin = glyph.pos.y;
                    } else if glyph.chr == 'あ' {
                        kana = glyph.pos.y + offset;
                    }
                }
                (latin, kana)
            });
            assert!(
                (latin - kana).abs() <= 1.0,
                "日文假名有效基线 {kana:.2} 与拉丁基线 {latin:.2} 偏差过大（offset={offset}）"
            );
        });
        output.textures_delta.clear();
    }

    /// 韩文 Apple SD Gothic Neo 需要小幅补偿；中文 PingFang 与 SF 一致，不需要。
    #[test]
    fn baseline_offset_table_matches_expectations() {
        assert_eq!(cjk_y_offset_factor("Hiragino Sans"), 0.22);
        assert_eq!(cjk_y_offset_factor("Apple SD Gothic Neo"), 0.06);
        assert_eq!(cjk_y_offset_factor("PingFang SC"), 0.0);
        assert_eq!(cjk_y_offset_factor("Segoe UI"), 0.0);
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
}
