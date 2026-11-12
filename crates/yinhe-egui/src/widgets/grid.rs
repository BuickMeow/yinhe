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
/// `StripBuilder` 的 `to_lengths` 已经把 `item_spacing` 计入总量
/// （`sum_non_remainder` 里含 `spacing * (len - 1)`），所以这里**不能**
/// 再自己扣一遍间距——多扣会把整排压窄，所有列布局一起变形。
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

    /// 记录两列的 `(左列右缘, 右列右缘, 父可用右缘)`。
    fn measure(size: egui::Vec2, gap: f32, left_w: f32) -> (f32, f32, f32) {
        let out = std::rc::Rc::new(std::cell::RefCell::new((0.0f32, 0.0f32, 0.0f32)));
        let o = out.clone();
        let mut harness = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_size(size)
            .build_ui(move |ui| {
                let avail_right = ui.max_rect().min.x + ui.available_width();
                let mut l = 0.0;
                let mut r = 0.0;
                two_columns(ui, gap, left_w, |cells| {
                    cells.cell(|ui| l = ui.max_rect().max.x);
                    cells.cell(|ui| r = ui.max_rect().max.x);
                });
                *o.borrow_mut() = (l, r, avail_right);
            });
        harness.step();
        *out.borrow()
    }

    /// 两列不能超出父 ui 可用宽度。
    #[test]
    fn two_columns_do_not_overflow() {
        for w in [760.0f32, 1200.0, 1750.0] {
            let (l, r, limit) = measure(egui::vec2(w, 600.0), 8.0, 164.0);
            eprintln!("[grid] w={w} left_end={l:.1} right_end={r:.1} limit={limit:.1}");
            assert!(
                r <= limit + 0.5,
                "w={w}: 右列右缘 {r:.1} 超出可用末端 {limit:.1}"
            );
        }
    }

    /// 右列必须真正吃满剩余宽度（不能变窄）。
    #[test]
    fn right_column_fills_remainder() {
        let (l, r, limit) = measure(egui::vec2(760.0, 600.0), 8.0, 164.0);
        let remaining = limit - l - 8.0; // 扣掉一个 gap
        let actual = r - l - 8.0;
        assert!(
            (actual - remaining).abs() <= 1.0,
            "右列宽 {actual:.1} 与应有剩余 {remaining:.1} 不符"
        );
    }
}

/// 模拟设置页完整结构：两列 → 卡片 Frame(内边距) → ScrollArea →
/// 行。量「卡片绘制右缘」与「窗口右缘」。
mod settings_shape {

    #[test]
    fn card_stays_inside_window() {
        for w in [760.0f32, 1200.0, 1750.0] {
            let out = std::rc::Rc::new(std::cell::RefCell::new((0.0f32, 0.0f32, 0.0f32)));
            let o = out.clone();
            let mut harness = egui_kittest::Harness::builder()
                .allow_missing_glyphs()
                .with_size(egui::vec2(w, 800.0))
                .build_ui(move |ui| {
                    let window_right = ui.max_rect().min.x + ui.available_width();
                    let nav_w = 148.0f32;
                    let mut card_right = 0.0f32;
                    let mut ctrl_right = 0.0f32;
                    crate::widgets::grid::two_columns(ui, 8.0, nav_w + 16.0, |cells| {
                        cells.cell(|_| {});
                        cells.cell(|ui| {
                            let card_w = (ui.available_width() - 16.0).max(1.0);
                            egui::Frame::new()
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.set_width(card_w);
                                    egui::ScrollArea::vertical()
                                        .id_salt(("s", w as u32))
                                        .auto_shrink([false; 2])
                                        .show(ui, |ui| {
                                            let iw = ui.available_width();
                                            ui.set_min_width(iw);
                                            ui.set_max_width(iw);
                                            card_right = ui.max_rect().max.x + 8.0;
                                            crate::widgets::rows::setting_row_plain(
                                                ui,
                                                "开启通知",
                                                |ui| {
                                                    let (r, _) = ui.allocate_exact_size(
                                                        egui::vec2(44.0, 22.0),
                                                        egui::Sense::hover(),
                                                    );
                                                    ctrl_right = r.max.x;
                                                },
                                            );
                                        });
                                });
                        });
                    });
                    *o.borrow_mut() = (card_right, ctrl_right, window_right);
                });
            harness.step();
            harness.step();
            let (card, ctrl, limit) = *out.borrow();
            eprintln!("[shape] w={w} card={card:.1} ctrl={ctrl:.1} limit={limit:.1}");
            assert!(
                card <= limit + 0.5,
                "w={w}: 卡片右缘 {card:.1} 超出窗口 {limit:.1}"
            );
        }
    }
}
