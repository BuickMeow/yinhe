//! 基于 `egui_extras::StripBuilder` 的网格容器原语。
//!
//! 只负责“分配格子”，内容仍是普通 egui 流式布局：
//! - 适合：窗口/对话框分栏、固定+剩余列、标题/内容/底栏分段；
//! - 不适合：坐标画布（PR/AR/MIX）、内容驱动行高的表单行、拖拽/虚拟列表。
//!
//! `StripBuilder` 的 cell 不会随子内容增长，所以这里的 API 只暴露
//! “固定 / 剩余”两类最常用语义，避免调用方误用成内容自适应布局。

use eframe::egui;
use egui_extras::{Size, StripBuilder};

/// 顺时针填充的格子集合：按布局方向依次调用 [`Self::cell`]。
pub(crate) struct Cells<'a, 'b> {
    strip: egui_extras::Strip<'a, 'b>,
}

impl Cells<'_, '_> {
    pub fn cell(&mut self, add_contents: impl FnOnce(&mut egui::Ui)) {
        self.strip.cell(add_contents);
    }
}

/// 横向列布局：`sizes` 定义每列宽度，回调里按顺序填 cell。
///
/// **必须先扣掉列间距再把 `StripBuilder` 建起来**：`Size::remainder()`
/// 按「ui 可用宽度 − 固定列宽」计算，而 `item_spacing` 是额外叠加的，
/// 净效果是总宽 = 可用宽度 + gap × (列数−1)，整排向右溢出（设置页右卡片
/// 超出窗口的根因）。这里把父 ui 的可用宽度临时收窄 gap×(列数−1)。
fn horizontal(
    ui: &mut egui::Ui,
    gap: f32,
    sizes: &[Size],
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    let prev = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = gap;
    let gaps = gap * sizes.len().saturating_sub(1) as f32;
    let total = (ui.available_width() - gaps).max(1.0);
    // 把父 ui 的可用宽度收窄到「扣掉列间距」后再建 StripBuilder：
    // `Size::remainder()` 取的是 ui 可用宽度，间距是额外叠加的。
    let max_rect = ui.max_rect();
    let narrowed = egui::Rect::from_min_max(
        max_rect.min,
        egui::pos2(max_rect.min.x + total, max_rect.max.y),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(narrowed), |ui| {
        let mut builder =
            StripBuilder::new(ui).cell_layout(egui::Layout::top_down(egui::Align::Min));
        for &size in sizes {
            builder = builder.size(size);
        }
        builder.horizontal(|strip| add(&mut Cells { strip }));
    });
    ui.spacing_mut().item_spacing.x = prev;
}

/// 纵向行布局：`sizes` 定义每行高度，回调里按顺序填 cell。
fn vertical(
    ui: &mut egui::Ui,
    gap: f32,
    sizes: &[Size],
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    let prev = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = gap;
    let mut builder = StripBuilder::new(ui).cell_layout(egui::Layout::top_down(egui::Align::Min));
    for &size in sizes {
        builder = builder.size(size);
    }
    builder.vertical(|strip| add(&mut Cells { strip }));
    ui.spacing_mut().item_spacing.y = prev;
}

/// 两列：左列固定宽，右列吃掉剩余宽度。
pub(crate) fn two_columns(
    ui: &mut egui::Ui,
    gap: f32,
    left_w: f32,
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    horizontal(ui, gap, &[Size::exact(left_w), Size::remainder()], add);
}

/// 固定行高的任意列：`sizes` 定义每列宽度。用于表单行，避免横向 strip 吃满剩余高度。
pub(crate) fn fixed_columns(
    ui: &mut egui::Ui,
    gap: f32,
    h: f32,
    sizes: &[Size],
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    let total = ui.available_width().max(1.0);
    ui.allocate_ui_with_layout(
        egui::vec2(total, h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| horizontal(ui, gap, sizes, add),
    );
}

/// 固定行高的两列：左固定宽，右剩余宽。
pub(crate) fn fixed_row(
    ui: &mut egui::Ui,
    gap: f32,
    h: f32,
    left_w: f32,
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    fixed_columns(ui, gap, h, &[Size::exact(left_w), Size::remainder()], add);
}

/// 两列：左剩余宽，右固定宽（贴右）。用于“端口填满 + 通道固定”等行。
pub(crate) fn two_columns_right(
    ui: &mut egui::Ui,
    gap: f32,
    right_w: f32,
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    horizontal(ui, gap, &[Size::remainder(), Size::exact(right_w)], add);
}

/// 三列：左固定、中间剩余、右固定。
pub(crate) fn three_columns(
    ui: &mut egui::Ui,
    gap: f32,
    left_w: f32,
    right_w: f32,
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    horizontal(
        ui,
        gap,
        &[Size::exact(left_w), Size::remainder(), Size::exact(right_w)],
        add,
    );
}

/// 按权重分配列宽（WPF 的 `1*` / `2*` / `4*`）。
///
/// 先扣掉列间距，再按权重比例分配，避免 `Size::relative` 的总和与 gap 冲突。
/// 权重必须为正数；全为 0 时退化为等分。
pub(crate) fn weighted_columns(
    ui: &mut egui::Ui,
    gap: f32,
    weights: &[f32],
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    if weights.is_empty() {
        return;
    }
    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    let usable = (ui.available_width() - gap * weights.len().saturating_sub(1) as f32).max(0.0);
    let unit = if total > 0.0 {
        usable / total
    } else {
        usable / weights.len() as f32
    };
    let sizes: Vec<Size> = weights
        .iter()
        .map(|w| Size::exact(unit * w.max(0.0)))
        .collect();
    horizontal(ui, gap, &sizes, add);
}

/// 内容 + 底栏：内容占剩余高度，底栏固定高。
pub(crate) fn content_footer(
    ui: &mut egui::Ui,
    gap: f32,
    footer_h: f32,
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    vertical(ui, gap, &[Size::remainder(), Size::exact(footer_h)], add);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 任何列布局的总宽都必须**严格等于**父 ui 可用宽度。
    ///
    /// `Size::remainder()` 按「ui 可用宽度」算，`item_spacing` 却是额外叠加的，
    /// 净效果是整排比可用宽度多出 `gap × (列数−1)`，一路向右溢出窗口。
    /// 设置页右卡片被挤出窗口就是它。
    fn assert_fits(label: &str, gap: f32, build: impl Fn(&mut egui::Ui, &mut dyn FnMut(f32))) {
        let mut harness = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_size(egui::vec2(760.0, 600.0))
            .build_ui(move |ui| {
                let avail = ui.available_width();
                let start = ui.max_rect().min.x;
                let mut right = start;
                build(ui, &mut |x: f32| right = right.max(x));
                assert!(
                    right <= start + avail + 0.5,
                    "{label}: 右缘 {right:.1} 超出可用末端 {:.1}（多 {:.1}px）",
                    start + avail,
                    right - (start + avail)
                );
            });
        harness.step();
        let _ = gap;
    }

    #[test]
    fn two_columns_fit_exactly() {
        assert_fits("two_columns", 8.0, |ui, out| {
            two_columns(ui, 8.0, 164.0, |cells| {
                cells.cell(|ui| out(ui.max_rect().max.x));
                cells.cell(|ui| out(ui.max_rect().max.x));
            });
        });
    }

    #[test]
    fn fixed_row_fits_exactly() {
        assert_fits("fixed_row", 6.0, |ui, out| {
            fixed_row(ui, 6.0, 24.0, 150.0, |cells| {
                cells.cell(|ui| out(ui.max_rect().max.x));
                cells.cell(|ui| out(ui.max_rect().max.x));
            });
        });
    }

    #[test]
    fn three_columns_fit_exactly() {
        assert_fits("three_columns", 12.0, |ui, out| {
            three_columns(ui, 12.0, 44.0, 190.0, |cells| {
                cells.cell(|ui| out(ui.max_rect().max.x));
                cells.cell(|ui| out(ui.max_rect().max.x));
                cells.cell(|ui| out(ui.max_rect().max.x));
            });
        });
    }

    #[test]
    fn weighted_columns_fit_exactly() {
        assert_fits("weighted_columns", 8.0, |ui, out| {
            weighted_columns(ui, 8.0, &[1.0, 2.0], |cells| {
                cells.cell(|ui| out(ui.max_rect().max.x));
                cells.cell(|ui| out(ui.max_rect().max.x));
            });
        });
    }
}
