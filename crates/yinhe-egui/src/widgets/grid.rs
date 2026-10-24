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
fn horizontal(
    ui: &mut egui::Ui,
    gap: f32,
    sizes: &[Size],
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    let prev = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = gap;
    let mut builder = StripBuilder::new(ui).cell_layout(egui::Layout::top_down(egui::Align::Min));
    for &size in sizes {
        builder = builder.size(size);
    }
    builder.horizontal(|strip| add(&mut Cells { strip }));
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

/// 内容 + 底栏：内容占剩余高度，底栏固定高。
pub(crate) fn content_footer(
    ui: &mut egui::Ui,
    gap: f32,
    footer_h: f32,
    add: impl for<'a, 'b> FnOnce(&mut Cells<'a, 'b>),
) {
    vertical(ui, gap, &[Size::remainder(), Size::exact(footer_h)], add);
}
