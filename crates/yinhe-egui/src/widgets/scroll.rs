//! 按整行取高的滚动容器。
//!
//! egui 的 `ScrollArea` 在视口底边**硬裁**：当视口高度不是「行高 + 行距」的整数倍
//! 时，底部那一行只露出一截（“半截窄行”）。这里把视口高度对齐到行边界，固定行高
//! 列表调用一次即可，无需各自手算 `max_height`。

use eframe::egui;

/// 固定行高列表的垂直滚动容器。返回内容闭包的返回值。
///
/// - `row_h`：单行高度（不含行距，行距取当前 `ui.spacing().item_spacing.y`）。
/// - `max_rows`：最多显示多少整行（`None` = 按可用高度取整行）。
///
/// 视口高度 = `floor(可用高 / (row_h + 行距)) × (row_h + 行距)`，底边永远压在行边界
/// 上；内容不足一屏时 `auto_shrink` 收缩到内容高度，天然无半截行。
pub fn rows_scroll<R>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    row_h: f32,
    max_rows: Option<usize>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    rows_scroll_impl(ui, id, row_h, max_rows, false, true, add_contents)
}

/// 同 [`rows_scroll`]，但**不**把视口高度对齐到行距整数倍：用满可用高度。
/// 列表下方不需要对齐留白时用（如图层列表）。
pub fn rows_scroll_full<R>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    row_h: f32,
    max_rows: Option<usize>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    rows_scroll_impl(ui, id, row_h, max_rows, false, false, add_contents)
}

/// 同 [`rows_scroll`]，但允许横向滚动（长文本列表，如树图）。
pub fn rows_scroll_both<R>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    row_h: f32,
    max_rows: Option<usize>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    rows_scroll_impl(ui, id, row_h, max_rows, true, true, add_contents)
}

/// 把可用高度对齐到行距整数倍（`pitch` 已含行距）。`max_rows` 见 [`rows_scroll`]。
///
/// 供需要自己 `show_rows`（行虚拟化）或自定义 `ScrollArea` 的固定行高列表使用。
pub fn snap_rows(avail: f32, pitch: f32, max_rows: Option<usize>) -> f32 {
    let by_avail = (avail / pitch).floor().max(1.0);
    let rows = match max_rows {
        Some(m) => by_avail.min(m as f32),
        None => by_avail,
    };
    rows * pitch
}

fn rows_scroll_impl<R>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    row_h: f32,
    max_rows: Option<usize>,
    both: bool,
    snap: bool,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let step = row_h + ui.spacing().item_spacing.y;
    let avail = ui.available_height();
    let max_h = if snap {
        snap_rows(avail, step, max_rows)
    } else {
        match max_rows {
            Some(m) => avail.min(m as f32 * step),
            None => avail,
        }
    };
    let area = if both {
        egui::ScrollArea::both()
    } else {
        egui::ScrollArea::vertical()
    };
    area.id_salt(id)
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, |ui| add_contents(ui))
        .inner
}
