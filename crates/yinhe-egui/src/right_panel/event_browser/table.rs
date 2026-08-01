//! 表格构建与单元格渲染。
//!
//! 关键设计：
//! 1. **不**在 `TableBuilder` 上加 `.sense(click)`。egui 的 "first-wins" 机制下，
//!    TableBuilder 的 `.sense(click)` 会 claim 整行的 primary + secondary 点击，
//!    导致 cell 内的 widget 拿不到 `secondary_clicked()`，右键编辑 popup 无法触发。
//! 2. cell 点击范围是**整个单元格**（用 `ui.interact(ui.max_rect(), ...)`），
//!    而不是 Label 的 rect。这样一位数和多位数的点击面积相同。
//! 3. 右键编辑请求统一存到全局 `egui::Id::new((id_salt, "edit"))`，
//!    用 `EditRequest` 枚举区分类型，由 `apply_edit_popups` 取出分派。

use eframe::egui;
use egui_extras::{Column, TableBuilder, TableRow};
use egui_material_icons::icons::{
    ICON_ADD, ICON_CHEVRON_LEFT as ICON_PREV, ICON_CHEVRON_RIGHT as ICON_NEXT,
};

use yinhe_types::SegmentShape;

use super::bar_lookup::BarLookup;
use super::state::{EditRequest, EventBrowserState};

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

/// 渲染只读文本单元格。
///
/// 点击范围是**整个单元格**（用 `ui.interact(ui.max_rect(), ...)`），
/// 而不是 Label 的 rect。这样一位数和多位数的点击面积相同。
pub(super) fn cell_text(
    row: &mut TableRow,
    text: impl Into<String>,
    click_key: egui::Id,
    row_idx: usize,
) {
    let s: String = text.into();
    row.col(|ui| {
        // 放 Label 消耗 layout 空间
        ui.add(egui::Label::new(egui::RichText::new(s).size(11.0).monospace()).selectable(false));
        // 整个 cell 加交互
        let cell_rect = ui.max_rect();
        let id = ui.id().with("cell").with(row_idx);
        let resp = ui.interact(cell_rect, id, egui::Sense::click());
        if resp.clicked() {
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
    });
}

/// 渲染只读位置单元格（"小节/小节内 tick" 文本）。
///
/// 左键跳转；右键写入 `EditRequest` 到 `(id_salt, "edit_pos")` key，
/// 由 `apply_*_popups` 取出后弹出位置 popup（小节 + 小节内 tick 两个 DragValue）。
/// 与普通 `(id_salt, "edit")` key 区分，让 popup 能选择位置编辑器而非单数字编辑器。
pub(super) fn cell_position(
    row: &mut TableRow,
    bar_lookup: &BarLookup,
    id_salt: &str,
    row_idx: usize,
    tick: u32,
    tick_edit_request: impl Fn(u32) -> EditRequest,
    click_key: egui::Id,
) {
    row.col(|ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(bar_lookup.format(tick))
                    .size(11.0)
                    .monospace(),
            )
            .selectable(false),
        );
        let cell_rect = ui.max_rect();
        let id = ui.id().with("poscell").with(row_idx);
        let resp = ui.interact(cell_rect, id, egui::Sense::click());
        if resp.clicked() {
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
        if resp.secondary_clicked() {
            ui.ctx().memory_mut(|m| {
                m.data.insert_temp(
                    egui::Id::new((id_salt, "edit_pos")),
                    tick_edit_request(tick),
                );
            });
        }
    });
}

/// 渲染行首"#"列：序号 + 行多选 + 右键菜单（上方插入/下方插入/删除）。
///
/// - 左键单击：选中该行（Ctrl 切换，Shift 范围选择），并触发跳转
/// - 右键：弹出菜单（在上方插入 / 在下方插入 / 删除该行），同时选中该行
///
/// `tick`：该行事件 tick（用于多选状态记录）
/// `all_ticks`：当前页所有行的 tick（用于 Shift 范围选择）
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(super) fn cell_row_header(
    row: &mut TableRow,
    state: &mut EventBrowserState,
    id_salt: &str,
    row_idx: usize,
    page_start: usize,
    tick: u32,
    all_ticks: &[u32],
    click_key: egui::Id,
) {
    row.col(|ui| {
        let is_selected = state.selected_ticks.contains(&tick);
        let label_color = if is_selected {
            egui::Color32::WHITE
        } else {
            crate::theme::TEXT_SECONDARY
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{}", page_start + row_idx + 1))
                    .size(11.0)
                    .monospace()
                    .color(label_color),
            )
            .selectable(false),
        );
        let cell_rect = ui.max_rect();
        let id = ui.id().with("rowhdr").with(row_idx);
        let resp = ui.interact(cell_rect, id, egui::Sense::click());

        // 左键：行选择（Ctrl/Shift 多选）
        if resp.clicked() {
            let modifiers = ui.ctx().input(|i| i.modifiers);
            handle_row_click(state, tick, all_ticks, modifiers);
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }

        // 右键：选中该行 + 弹出菜单
        if resp.secondary_clicked() && !state.selected_ticks.contains(&tick) {
            handle_row_click(state, tick, all_ticks, egui::Modifiers::NONE);
        }
        let edit_key = egui::Id::new((id_salt, "edit"));
        resp.context_menu(|ui| {
            if ui.button("在上方插入").clicked() {
                ui.ctx().memory_mut(|m| {
                    m.data
                        .insert_temp(edit_key, EditRequest::InsertAbove { tick });
                });
                ui.close();
            }
            if ui.button("在下方插入").clicked() {
                ui.ctx().memory_mut(|m| {
                    m.data
                        .insert_temp(edit_key, EditRequest::InsertBelow { tick });
                });
                ui.close();
            }
            ui.separator();
            if ui.button("删除").clicked() {
                ui.ctx().memory_mut(|m| {
                    m.data.insert_temp(edit_key, EditRequest::DeleteSelected);
                });
                ui.close();
            }
        });
    });
}

/// 处理行点击的多选逻辑（Ctrl 切换、Shift 范围、普通单选）。
fn handle_row_click(
    state: &mut EventBrowserState,
    tick: u32,
    all_ticks: &[u32],
    modifiers: egui::Modifiers,
) {
    if modifiers.ctrl || modifiers.command {
        // Ctrl：切换该行选中
        if state.selected_ticks.contains(&tick) {
            state.selected_ticks.remove(&tick);
        } else {
            state.selected_ticks.insert(tick);
        }
        state.last_clicked_tick = Some(tick);
    } else if modifiers.shift {
        // Shift：范围选择（从上次点击到当前）
        if let Some(anchor) = state.last_clicked_tick {
            let (lo, hi) = if anchor <= tick {
                (anchor, tick)
            } else {
                (tick, anchor)
            };
            // 在 all_ticks 中找范围内的所有 tick
            for &t in all_ticks {
                if t >= lo && t <= hi {
                    state.selected_ticks.insert(t);
                }
            }
        } else {
            state.selected_ticks.clear();
            state.selected_ticks.insert(tick);
        }
    } else {
        // 普通单击：只选该行
        state.selected_ticks.clear();
        state.selected_ticks.insert(tick);
        state.last_clicked_tick = Some(tick);
    }
}

/// 空表格的加号按钮：点击新建第一个事件。
///
/// 用 Label + interact + painter 叠加实现 hover 变蓝（同 mode_bar 风格），
/// 避免 egui::Button 默认 hover 动画导致的跳动。
///
/// 返回 true 表示用户点击了加号（触发 `EditRequest::InsertFirst`）。
pub(super) fn empty_state_add_button(ui: &mut egui::Ui, id_salt: &str) -> bool {
    use crate::theme::ACCENT_ACTIVE;
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        let icon_text = egui::RichText::new(ICON_ADD)
            .size(24.0)
            .color(crate::theme::TEXT_BRIGHT);
        let resp = ui.add(
            egui::Label::new(icon_text)
                .selectable(false)
                .sense(egui::Sense::click()),
        );
        // hover 时叠加蓝色图标（同 mode_bar 的 hover_highlight 机制）
        if resp.hovered() {
            ui.painter().text(
                resp.rect.center(),
                egui::Align2::CENTER_CENTER,
                ICON_ADD.codepoint,
                egui::FontId::proportional(24.0),
                ACCENT_ACTIVE,
            );
        }
        if resp.clicked() {
            clicked = true;
        }
        ui.label(
            egui::RichText::new("点击新建第一个事件")
                .size(11.0)
                .color(crate::theme::TEXT_FAINT),
        );
    });
    if clicked {
        let edit_key = egui::Id::new((id_salt, "edit"));
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp(edit_key, EditRequest::InsertFirst));
    }
    clicked
}

/// 处理 Delete/Backspace 键盘删除（当表格区域有焦点时）。
///
/// 返回 true 表示触发了删除。
pub(super) fn handle_delete_key(ui: &egui::Ui, id_salt: &str, has_selection: bool) -> bool {
    if !has_selection {
        return false;
    }
    let delete_pressed = ui
        .ctx()
        .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
    if delete_pressed {
        let edit_key = egui::Id::new((id_salt, "edit"));
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp(edit_key, EditRequest::DeleteSelected));
        true
    } else {
        false
    }
}

/// 渲染可编辑文本单元格：左键跳转，右键写入 `EditRequest` 到 memory 触发 popup。
///
/// **关键**：edit key 用 `egui::Id::new((id_salt, "edit"))` 全局 id，**不**用 `ui.id()`。
/// 原因：cell 内的 `ui.id()` 与 `apply_edit_popups` 调用处的 `ui.id()` 不同
/// （cell 是 child ui），用 `ui.id()` 会导致 write/read key 不匹配，popup 永远不触发。
pub(super) fn cell_editable(
    row: &mut TableRow,
    id_salt: &str,
    row_idx: usize,
    text: impl Into<String>,
    edit_request: EditRequest,
    click_key: egui::Id,
) {
    let s: String = text.into();
    row.col(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(s).size(11.0).monospace()).selectable(false));
        let cell_rect = ui.max_rect();
        let id = ui.id().with("cell").with(row_idx);
        let resp = ui.interact(cell_rect, id, egui::Sense::click());
        if resp.clicked() {
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
        if resp.secondary_clicked() {
            // 全局 key：与 apply_edit_popups 的 Id::new((id_salt, "edit")) 对齐
            ui.ctx().memory_mut(|m| {
                m.data
                    .insert_temp(egui::Id::new((id_salt, "edit")), edit_request);
            });
        }
    });
}

/// 查看 cell 写入的 `EditRequest`（不删除）。
///
/// `apply_*_popups` 每帧调用此函数，popup 显示期间 `EditRequest` 一直在 memory 里，
/// 直到 `remove_edit_request` 关闭时清除。
pub(super) fn peek_edit_request(ui: &egui::Ui, id_salt: &str) -> Option<EditRequest> {
    let key = egui::Id::new((id_salt, "edit"));
    ui.memory(|m| m.data.get_temp::<EditRequest>(key))
}

/// 清除 `EditRequest`（popup 关闭时调用）。
pub(super) fn remove_edit_request(ui: &egui::Ui, id_salt: &str) {
    let key = egui::Id::new((id_salt, "edit"));
    ui.memory_mut(|m| m.data.remove::<EditRequest>(key));
}

/// 查看位置 cell 写入的 `EditRequest`（不删除）。与 `peek_edit_request` 平行。
pub(super) fn peek_pos_edit_request(ui: &egui::Ui, id_salt: &str) -> Option<EditRequest> {
    let key = egui::Id::new((id_salt, "edit_pos"));
    ui.memory(|m| m.data.get_temp::<EditRequest>(key))
}

/// 清除位置编辑请求（位置 popup 关闭时调用）。
pub(super) fn remove_pos_edit_request(ui: &egui::Ui, id_salt: &str) {
    let key = egui::Id::new((id_salt, "edit_pos"));
    ui.memory_mut(|m| m.data.remove::<EditRequest>(key));
}

/// 把 `SegmentShape` 格式化为表格单元格文本（仅类型名；曲线参数见 `curve_points_text`）。
pub(super) fn shape_text(shape: SegmentShape) -> String {
    match shape {
        SegmentShape::Step => "Step".to_string(),
        SegmentShape::Curve { .. } if shape.is_linear() => "Linear".to_string(),
        SegmentShape::Curve { .. } => "Curve".to_string(),
    }
}

/// 曲线控制点四分量文本，顺序为 (X1, Y1, X2, Y2)。
/// 离散（Step）没有控制点，全部返回 "N/A"。
pub(super) fn curve_points_text(shape: SegmentShape) -> [String; 4] {
    match shape {
        SegmentShape::Step => [
            "N/A".to_string(),
            "N/A".to_string(),
            "N/A".to_string(),
            "N/A".to_string(),
        ],
        SegmentShape::Curve { x1, y1, x2, y2 } => [
            format!("{:.2}", x1),
            format!("{:.2}", y1),
            format!("{:.2}", x2),
            format!("{:.2}", y2),
        ],
    }
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
        if ui
            .add_enabled(
                next_enabled,
                egui::Label::new(
                    ICON_NEXT
                        .rich_text()
                        .size(14.0)
                        .color(crate::theme::TEXT_BRIGHT),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            new_page = Some(page + 1);
        }
        ui.label(
            egui::RichText::new(format!("/ {}", total_pages))
                .size(11.0)
                .color(crate::theme::TEXT_FAINT),
        );
        let buf: String = ui.memory(|m| {
            m.data
                .get_temp(mem_key)
                .unwrap_or_else(|| (page + 1).to_string())
        });
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
            if let Ok(n) = edited_buf.trim().parse::<usize>()
                && n >= 1
                && n <= total_pages
                && n - 1 != page
            {
                new_page = Some(n - 1);
            }
            ui.memory_mut(|m| m.data.remove::<String>(mem_key));
        }
        let prev_enabled = page > 0;
        if ui
            .add_enabled(
                prev_enabled,
                egui::Label::new(
                    ICON_PREV
                        .rich_text()
                        .size(14.0)
                        .color(crate::theme::TEXT_BRIGHT),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            new_page = Some(page - 1);
        }
    });
    new_page
}
