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
    Attributes, Collection, CollectionOptions, FontWeight, GenericFamily, QueryFamily, QueryStatus,
    SourceCache, SourceCacheOptions,
};

/// UI 字重（当前固定 Medium）。
pub(crate) const WEIGHT_MEDIUM: u16 = 500;

/// 把系统字体注入 egui：主字体 + 按语言回退 + Material 图标。
///
/// `weight` 为 UI 字重，`locale` 为 BCP-47 语言标签。
pub(crate) fn install(ctx: &Context, weight: u16, locale: &str) {
    let mut defs = build(weight);

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

/// 构建注入了系统主字体的 [`FontDefinitions`]（拆出来便于测试）。
///
/// 只注入主字体；CJK 等回退交给 `SystemFontProvider` 按需加载。
pub(crate) fn build(weight: u16) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let Some(font) = query_primary(weight) else {
        return fonts;
    };
    let key = "system-sans".to_owned();
    insert(&mut fonts, &key, font);
    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        list.insert(0, key);
    }
    fonts
}

struct Picked {
    blob: fontique::Blob<u8>,
    index: u32,
    coords: VariationCoords,
}

/// 系统 UI 字体（macOS 的 SF Pro Text、Windows 的 Segoe UI、Linux 由 fontconfig 决定）。
///
/// macOS 的 `System Font` 是 `.SFNS` 可变字体（默认光学尺寸偏 Display）；
/// 界面小字号应使用 `SF Pro Text` 这一族。
fn query_primary(weight: u16) -> Option<Picked> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut cache = SourceCache::new(SourceCacheOptions::default());

    #[cfg(target_os = "macos")]
    if let Some(picked) = query_family(
        &mut collection,
        &mut cache,
        QueryFamily::Named("SF Pro Text"),
        weight,
    ) {
        return Some(picked);
    }

    query_family(
        &mut collection,
        &mut cache,
        QueryFamily::Generic(GenericFamily::SystemUi),
        weight,
    )
    .or_else(|| {
        query_family(
            &mut collection,
            &mut cache,
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
    let (_, _, blob, index, synthesis) = candidates.into_iter().find(|c| {
        collection
            .family(c.0)
            .and_then(|family| family.fonts().get(c.1).map(|font| font.style()))
            == Some(fontique::FontStyle::Normal)
    })?;
    Some(Picked {
        blob,
        index,
        coords: VariationCoords::new(
            synthesis
                .variation_settings()
                .iter()
                .map(|(tag, value)| (tag.to_be_bytes(), *value)),
        ),
    })
}

fn insert(fonts: &mut FontDefinitions, key: &str, font: Picked) {
    let (blob, _) = font.blob.into_raw_parts();
    let mut data = FontData::from_blob(blob, font.index);
    data.tweak = FontTweak {
        coords: font.coords,
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
        let defs = build(WEIGHT_MEDIUM);
        let proportional = &defs.families[&FontFamily::Proportional];
        assert_eq!(
            proportional.first().map(String::as_str),
            Some("system-sans")
        );
        assert!(defs.font_data.contains_key("system-sans"));
    }

    /// 每个语言都能挂上 provider（locale 解析失败也不 panic，只是用系统默认）。
    #[test]
    fn provider_accepts_all_locales() {
        for locale in LOCALES {
            let _ = egui_system_font_provider::SystemFontProvider::new().with_locale(Some(locale));
        }
    }

    /// CJK / 假名靠 provider 按需回退：布局出的字形必须有实际宽度，
    /// 否则界面上中文会变成方框或空白。
    #[test]
    fn cjk_falls_back_through_provider() {
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
            assert!(width > 4.0, "{locale}: {sample} 回退失败，宽度 {width}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn primary_is_medium_weight() {
        let defs = build(WEIGHT_MEDIUM);
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
