//! 表格构建与单元格渲染。
//!
//! 关键设计：**不**在 `TableBuilder` 上加 `.sense(click)`。
//! 原因：egui 的 "first-wins" 机制下，TableBuilder 的 `.sense(click)` 会
//! claim 整行的 primary + secondary 点击，导致 cell 内的 Label 拿不到
//! `secondary_clicked()`，右键编辑 popup 无法触发。
//!
//! 解决方案：每个 cell 的 Label 自带 `.sense(click)`，自行处理
//! `clicked()`（左键跳转）和 `secondary_clicked()`（右键编辑）。

use eframe::egui;
use egui_material_icons::icons::{ICON_CHEVRON_LEFT as ICON_PREV, ICON_CHEVRON_RIGHT as ICON_NEXT};
use egui_extras::{Column, TableBuilder, TableRow};

use yinhe_types::SegmentShape;

/// owned 副本，避免不可变借用阻塞后续 `&mut doc` 编辑。
#[derive(Clone, Copy)]
pub(super) struct AutomationEventOwned {
    pub tick: u32,
    pub value: f32,
    pub shape: SegmentShape,
}

/// 构建表格。
///
/// `row_cb` 接收 `(row_idx, row, click_key)`，cell 函数用 `click_key`
/// 记录左键点击到 memory，由 `take_row_click` 取出。
pub(super) fn build_table<F>(
    ui: &mut egui::Ui,
    id_salt: &str,
    headers: &[(&str, f32)],
    rows: usize,
    mut row_cb: F,
) where
    F: FnMut(usize, &mut TableRow, egui::Id),
{
    let click_key = ui.id().with(("row_click", id_salt));
    let mut tb = TableBuilder::new(ui)
        .id_salt(id_salt)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for (_, min_w) in headers {
        tb = tb.column(Column::initial(*min_w).at_least(40.0).clip(true));
    }
    tb.header(20.0, |mut h| {
        for (label, _) in headers {
            h.col(|ui| {
                ui.label(egui::RichText::new(*label).strong().size(11.0));
            });
        }
    })
    .body(move |body| {
        body.rows(18.0, rows, move |mut row| {
            let i = row.index();
            row_cb(i, &mut row, click_key);
        });
    });
}

/// 取出 `build_table` 写入 memory 的行点击索引（若存在）。
pub(super) fn take_row_click(ui: &egui::Ui, id_salt: &str) -> Option<usize> {
    let key = ui.id().with(("row_click", id_salt));
    let v = ui.memory(|m| m.data.get_temp::<usize>(key));
    if v.is_some() {
        ui.memory_mut(|m| m.data.remove::<usize>(key));
    }
    v
}

/// 渲染只读文本单元格。左键点击记录到 `click_key` 用于跳转。
pub(super) fn cell_text(
    row: &mut TableRow,
    text: impl Into<String>,
    click_key: egui::Id,
    row_idx: usize,
) {
    let s: String = text.into();
    row.col(|ui| {
        let resp = ui.add(
            egui::Label::new(egui::RichText::new(s).size(11.0).monospace())
                .selectable(false)
                .sense(egui::Sense::click()),
        );
        if resp.clicked() {
            ui.ctx().memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
    });
}

/// 把 `SegmentShape` 格式化为表格单元格文本。
pub(super) fn shape_text(shape: SegmentShape) -> String {
    match shape {
        SegmentShape::Step => "Step".to_string(),
        SegmentShape::Curve { x1, y1, x2, y2 } => {
            if shape.is_linear() {
                "Linear".to_string()
            } else {
                format!("{:.2},{:.2},{:.2},{:.2}", x1, y1, x2, y2)
            }
        }
    }
}

/// 渲染值单元格：显示数值，左键跳转，右键记录到 memory 触发编辑 popup。
///
/// **关键**：edit key 用 `egui::Id::new((id_salt, "edit"))` 全局 id，**不**用 `ui.id()`。
/// 原因：cell 内的 `ui.id()` 与 `apply_automation_popups` 调用处的 `ui.id()` 不同
/// （cell 是 child ui），用 `ui.id()` 会导致 write/read key 不匹配，popup 永远不触发。
pub(super) fn cell_value_editable(
    row: &mut TableRow,
    id_salt: &str,
    row_idx: usize,
    tick: u32,
    value: f32,
    _max_val: f32,
    click_key: egui::Id,
) {
    row.col(|ui| {
        let resp = ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{}", value.round() as i32))
                    .size(11.0)
                    .monospace(),
            )
            .selectable(false)
            .sense(egui::Sense::click()),
        );
        if resp.clicked() {
            ui.ctx().memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
        if resp.secondary_clicked() {
            // 全局 key：与 apply_automation_popups 的 Id::new((val_salt, "edit")) 对齐
            ui.ctx().memory_mut(|m| {
                m.data.insert_temp(egui::Id::new((id_salt, "edit")), (row_idx, tick, value));
            });
        }
    });
}

/// 渲染形状单元格：显示形状文本，左键跳转，右键记录到 memory 触发编辑 popup。
///
/// edit key 同样用 `egui::Id::new((id_salt, "edit"))` 全局 id，与 `cell_value_editable` 同理。
pub(super) fn cell_shape_editable(
    row: &mut TableRow,
    id_salt: &str,
    row_idx: usize,
    tick: u32,
    shape: SegmentShape,
    click_key: egui::Id,
) {
    row.col(|ui| {
        let resp = ui.add(
            egui::Label::new(egui::RichText::new(shape_text(shape)).size(11.0).monospace())
                .selectable(false)
                .sense(egui::Sense::click()),
        );
        if resp.clicked() {
            ui.ctx().memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
        if resp.secondary_clicked() {
            ui.ctx().memory_mut(|m| {
                m.data.insert_temp(egui::Id::new((id_salt, "edit")), (row_idx, tick, shape));
            });
        }
    });
}

/// 每页行数。100 行在常规字体下约填满半屏~一屏，翻页频率适中。
pub(super) const EVENT_PAGE_SIZE: usize = 100;

/// 计算总页数（至少 1 页）。
pub(super) fn total_pages(total: usize) -> usize {
    total.div_ceil(EVENT_PAGE_SIZE).max(1)
}

/// 根据当前 `state.event_page` 切片出当前页。
///
/// 返回 `(page, page_start, page_slice)`：
/// - `page`：0-based 页码（已做越界保护，删除数据后自动夹回末页）
/// - `page_start`：当前页起始索引
/// - `page_slice`：当前页的切片
pub(super) fn paginate<'a, T>(
    state: &mut super::EventBrowserState,
    items: &'a [T],
) -> (usize, usize, &'a [T]) {
    let total = items.len();
    let tp = total_pages(total);
    if state.event_page >= tp {
        state.event_page = tp - 1;
    }
    let page = state.event_page;
    let start = page * EVENT_PAGE_SIZE;
    let end = (start + EVENT_PAGE_SIZE).min(total);
    (page, start, &items[start..end])
}

/// 渲染翻页控件（右对齐），返回 `Some(新页码)` 如果用户改变了页码（0-based）。
pub(super) fn render_pager(ui: &mut egui::Ui, page: usize, total_pages: usize) -> Option<usize> {
    let mut new_page = None;
    let mem_key = ui.id().with("eb_page_input");
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let next_enabled = page + 1 < total_pages;
        if ui.add_enabled(
            next_enabled,
            egui::Label::new(ICON_NEXT.rich_text().size(14.0).color(egui::Color32::from_gray(200))).sense(egui::Sense::click()),
        )
        .clicked()
        {
            new_page = Some(page + 1);
        }
        ui.label(egui::RichText::new(format!("/ {}", total_pages)).size(11.0).color(egui::Color32::from_gray(140)));
        let buf: String = ui.memory(|m| m.data.get_temp(mem_key).unwrap_or_else(|| (page + 1).to_string()));
        let mut buf = buf;
        let resp = ui.add(
            egui::TextEdit::singleline(&mut buf)
                .desired_width(28.0)
                .font(egui::FontId::proportional(11.0))
                .horizontal_align(egui::Align::Center),
        );
        let edited_buf = buf.clone();
        if resp.has_focus() {
            ui.memory_mut(|m| m.data.insert_temp(mem_key, buf));
        }
        if resp.lost_focus() {
            if let Ok(n) = edited_buf.trim().parse::<usize>() {
                if n >= 1 && n <= total_pages && n - 1 != page {
                    new_page = Some(n - 1);
                }
            }
            ui.memory_mut(|m| m.data.remove::<String>(mem_key));
        }
        let prev_enabled = page > 0;
        if ui.add_enabled(
            prev_enabled,
            egui::Label::new(ICON_PREV.rich_text().size(14.0).color(egui::Color32::from_gray(200))).sense(egui::Sense::click()),
        )
        .clicked()
        {
            new_page = Some(page - 1);
        }
    });
    new_page
}
