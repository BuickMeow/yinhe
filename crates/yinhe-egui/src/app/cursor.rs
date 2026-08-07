//! Material 风格鼠标指针。
//!
//! egui 每帧根据控件 hover 状态决定 `CursorIcon`，这里把每种图标映射为
//! 一个 Material Symbols 位图，通过 `PlatformOutput::cursor_image` 交给
//! 操作系统显示（egui-winit → winit `CustomCursor`）。相比在 egui 内部
//! 用 Painter 画光标，位图光标不会被窗口裁剪，且鼠标移出窗口后系统自动
//! 恢复默认光标。

use ab_glyph::{Font, FontArc, PxScale, point};
use egui::{CursorIcon, CustomCursorImage};
use std::sync::Arc;

/// 光标画布边长（逻辑像素），再乘 pixels_per_point 适配高分屏。
const BASE_SIZE: f32 = 32.0;
/// 字形占画布的比例，余量留给 1px 描边与热点。
const GLYPH_FILL: f32 = 0.8;

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];
const CENTER: (f32, f32) = (0.5, 0.5);

/// 光标位图缓存与映射。
pub(crate) struct MaterialCursorState {
    font: Option<FontArc>,
    /// 已栅格化的位图，按 (字符, 画布边长) 缓存，避免每帧重复渲染。
    cache: Vec<(char, u16, CustomCursorImage)>,
}

impl MaterialCursorState {
    pub(crate) fn new() -> Self {
        // 复用 egui_material_icons 内嵌的 Material Symbols Rounded 字体，
        // 保证光标与 UI 图标完全同源。
        let font = egui_material_icons::font_insert().data.font.into_owned();
        Self {
            font: FontArc::try_from_vec(font).ok(),
            cache: Vec::new(),
        }
    }

    /// 读取 egui 本帧决定的光标图标，替换为对应的 Material 位图光标。
    /// 后端会优先使用 `cursor_image`，失败时自动回退到 `cursor_icon`。
    pub(crate) fn apply(&mut self, ctx: &egui::Context) {
        let icon = ctx.output(|o| o.cursor_icon);
        let Some(font) = &self.font else { return };
        let Some((ch, hotspot_frac)) = spec_for(icon) else {
            ctx.output_mut(|o| o.cursor_image = None);
            return;
        };
        let target = ((BASE_SIZE * ctx.pixels_per_point()).round() as u16).clamp(16, 96);
        let image = match self.cache.iter().find(|(c, t, _)| *c == ch && *t == target) {
            Some((_, _, img)) => img.clone(),
            None => {
                let Some(img) = rasterize(font, ch, target, hotspot_frac) else {
                    return;
                };
                self.cache.push((ch, target, img.clone()));
                img
            }
        };
        ctx.output_mut(|o| o.cursor_image = Some(image));
    }
}

/// 把 `CursorIcon` 映射为图标字符与热点在字形内的相对位置。
/// 返回 `None` 时保持系统光标（如 `None` 表示隐藏光标）。
fn spec_for(icon: CursorIcon) -> Option<(char, (f32, f32))> {
    use CursorIcon::*;
    Some(match icon {
        // ads_click：箭头指针，尖端在字形左上角
        Default => ('\u{e762}', (0.12, 0.15)),
        None => return Option::None,
        // more_vert：上下文菜单
        ContextMenu => ('\u{e5d4}', CENTER),
        Help => ('\u{e8fd}', CENTER),
        // pan_tool：手型，指尖在中上部
        PointingHand => ('\u{e925}', (0.5, 0.12)),
        Progress => ('\u{e9d0}', CENTER),
        Wait => ('\u{ebff}', CENTER),
        Cell => ('\u{e3ec}', CENTER),
        Crosshair => ('\u{e55c}', CENTER),
        Text | VerticalText => ('\u{e262}', CENTER),
        Alias => ('\u{e250}', CENTER),
        Copy => ('\u{e14d}', CENTER),
        Move => ('\u{e89f}', CENTER),
        NoDrop | NotAllowed => ('\u{f08c}', CENTER),
        Grab | Grabbing => ('\u{e925}', (0.5, 0.12)),
        AllScroll => ('\u{e89f}', CENTER),
        ResizeHorizontal | ResizeEast | ResizeWest | ResizeColumn => ('\u{e8d4}', CENTER),
        ResizeVertical | ResizeNorth | ResizeSouth | ResizeRow => ('\u{e8d5}', CENTER),
        ResizeNwSe | ResizeSouthEast | ResizeNorthWest => ('\u{f1ce}', CENTER),
        ResizeNeSw | ResizeNorthEast | ResizeSouthWest => ('\u{f1cf}', CENTER),
        ZoomIn => ('\u{e8ff}', CENTER),
        ZoomOut => ('\u{e900}', CENTER),
    })
}

/// 把图标字形栅格化为 `target × target` 的 RGBA 位图（白色填充 + 1px 黑色描边）。
fn rasterize(
    font: &FontArc,
    ch: char,
    target: u16,
    hotspot_frac: (f32, f32),
) -> Option<CustomCursorImage> {
    let target_f = target as f32;
    let gid = font.glyph_id(ch);

    // 先量出 1em 下的字形包围盒，反推目标像素高度所需的字号。
    let bbox_1em = font
        .outline_glyph(gid.with_scale(PxScale::from(1.0)))?
        .px_bounds();
    if bbox_1em.height() <= 0.0 || bbox_1em.width() <= 0.0 {
        return None;
    }
    let scale = target_f * GLYPH_FILL / bbox_1em.height();

    let bbox = font
        .outline_glyph(gid.with_scale(PxScale::from(scale)))?
        .px_bounds();
    // 字形水平垂直居中，保证 draw 坐标全部落在画布内。
    let offset = point(
        (target_f - bbox.width()) / 2.0 - bbox.min.x,
        (target_f - bbox.height()) / 2.0 - bbox.min.y,
    );
    let hotspot = [
        ((offset.x + bbox.min.x + bbox.width() * hotspot_frac.0).round() as u16).min(target - 1),
        ((offset.y + bbox.min.y + bbox.height() * hotspot_frac.1).round() as u16).min(target - 1),
    ];

    let mut rgba = vec![0u8; target as usize * target as usize * 4];
    // 1px 黑色描边：上下左右四个偏移各画一遍，再叠白色填充。
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        let glyph =
            gid.with_scale_and_position(PxScale::from(scale), point(offset.x + dx, offset.y + dy));
        let outline = font.outline_glyph(glyph)?;
        outline.draw(|x, y, alpha| blend(&mut rgba, x, y, target, BLACK, alpha * 0.9));
    }
    let outline = font.outline_glyph(gid.with_scale_and_position(PxScale::from(scale), offset))?;
    outline.draw(|x, y, alpha| blend(&mut rgba, x, y, target, WHITE, alpha));

    Some(CustomCursorImage {
        rgba: Arc::from(rgba),
        size: [target; 2],
        hotspot,
    })
}

/// 以直通（straight）RGBA 写入一个像素；同像素多次写入时取更高覆盖率。
fn blend(rgba: &mut [u8], x: u32, y: u32, target: u16, color: [u8; 3], alpha: f32) {
    if x >= target as u32 || y >= target as u32 {
        return;
    }
    let i = ((y * target as u32 + x) * 4) as usize;
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    if a > rgba[i + 3] {
        rgba[i] = color[0];
        rgba[i + 1] = color[1];
        rgba[i + 2] = color[2];
        rgba[i + 3] = a;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_font() -> FontArc {
        FontArc::try_from_vec(egui_material_icons::font_insert().data.font.into_owned()).unwrap()
    }

    #[test]
    fn spec_for_default_is_material_arrow() {
        let (ch, _) = spec_for(CursorIcon::Default).unwrap();
        assert_eq!(ch, '\u{e762}'); // ads_click
    }

    #[test]
    fn spec_for_none_keeps_system_cursor() {
        assert!(spec_for(CursorIcon::None).is_none());
    }

    #[test]
    fn rasterize_produces_valid_cursor_image() {
        let font = test_font();
        let img = rasterize(&font, '\u{e762}', 32, (0.12, 0.15)).unwrap();
        assert_eq!(img.size, [32, 32]);
        assert!(img.hotspot[0] < 32 && img.hotspot[1] < 32);
        // 必须有非透明像素（字形渲染成功）
        assert!(img.rgba.chunks_exact(4).any(|px| px[3] > 0));
    }

    #[test]
    fn rasterize_keeps_hotspot_within_bounds() {
        let font = test_font();
        for (ch, frac) in [
            ('\u{e925}', (0.5, 0.12)),
            ('\u{e55c}', CENTER),
            ('\u{e8d4}', CENTER),
        ] {
            let img = rasterize(&font, ch, 48, frac).unwrap();
            assert!(img.hotspot[0] < 48 && img.hotspot[1] < 48);
            assert!(img.rgba.chunks_exact(4).any(|px| px[3] > 0));
        }
    }
}
