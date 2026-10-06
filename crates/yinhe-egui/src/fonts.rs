//! 界面字体注入：按语言选择的系统字体 + Material 图标。
//!
//! 必须一次性通过 `set_fonts` 注入：egui 0.36 的 `add_font` 与 `set_fonts` 走两个
//! 独立队列，切换语言时再次 `set_fonts` 会把之前 `add_font` 的 Material 图标覆盖掉，
//! 导致所有图标变成方框。这里把两者合进同一份 `FontDefinitions`。

use std::sync::Arc;

use egui::epaint::text::FontPriority;

/// 按界面语言注入系统字体，并合入 Material 图标。
pub(crate) fn install(ctx: &egui::Context, weight: u16, locale: &str) {
    let mut defs = yinhe_fonts::build(weight, locale);

    let mut insert = egui_material_icons::font_insert();
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

    ctx.set_fonts(defs);
}
