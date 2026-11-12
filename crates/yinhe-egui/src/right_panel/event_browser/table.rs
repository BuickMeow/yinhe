//! 表格构建与单元格渲染。
//!
//! 关键设计：
//! 1. **不**在 `TableBuilder` 上加 `.sense(click)`。egui 的 "first-wins" 机制下，
//!    TableBuilder 的 `.sense(click)` 会 claim 整行的 primary + secondary 点击，
//!    导致 cell 内的 widget 拿不到 `secondary_clicked()`，右键编辑 popup 无法触发。
//! 2. cell 点击范围是**整个单元格**（用 `ui.interact(ui.max_rect(), ...)`），
//!    而不是 Label 的 rect。这样一位数和多位数的点击面积相同。
//! 3. 右键编辑请求统一存到全局 `egui::Id::unique((id_salt, "edit"))`，
//!    用 `EditRequest` 枚举区分类型，由 `apply_edit_popups` 取出分派。

use eframe::egui;
use egui_extras::{Column, TableBuilder, TableRow};
use egui_material_icons::icons::ICON_ADD;

use yinhe_types::SegmentShape;

use rust_i18n::t;

use super::bar_lookup::BarLookup;
use super::state::{EditRequest, EventBrowserState};

/// owned 副本，避免不可变借用阻塞后续 `&mut doc` 编辑。
#[derive(Clone, Copy)]
pub(super) struct AutomationEventOwned {
    pub tick: u32,
    pub value: f32,
    pub shape: SegmentShape,
}

/// 事件表格的行高 / 滚动布局状态。
///
/// 独立成小结构：构建表格时从 `EventBrowserState` 取出来单独借用，
/// 表格闭包里还要用 `state` 做选中态与右键请求，不能双重可变借用。
#[derive(Clone, Copy, Debug)]
pub struct EventListLayout {
    /// 行高（像素）。0 = 用主题默认（`ROW_H_COMPACT`）。
    pub row_h: f32,
    /// 纵向滚动偏移（像素）。单一真源：每帧从表格内部读回，下一帧回灌。
    pub scroll_y: f32,
    /// 一次性回灌的滚动偏移（切换事件类型时归零用）。取走即清空。
    pub pending_scroll: Option<f32>,
    /// 上一帧测得的视口高度与顶部 y（缩放锚点计算用）。
    pub view_h: f32,
    pub view_top: f32,
}

impl Default for EventListLayout {
    fn default() -> Self {
        Self {
            row_h: 0.0,
            scroll_y: 0.0,
            pending_scroll: None,
            view_h: 0.0,
            view_top: 0.0,
        }
    }
}

/// 构建表格。
///
/// `row_cb` 接收 `(行号, row, click_key)`，cell 函数用 `click_key`
/// 记录左键点击到 memory，由 `take_row_click` 取出。
///
/// `layout` 持有行高与滚动偏移：滚轮 + Ctrl/Cmd（或触控板捏合）改行高，
/// 与 PR/AR 的时间轴缩放操作一致；行高变了按指针位置做锚点补偿，
/// 否则缩放时列表会往上/往下“跑”。
pub(super) fn build_table<F>(
    ui: &mut egui::Ui,
    id_salt: &str,
    headers: &[(&str, f32)],
    rows: usize,
    layout: &mut EventListLayout,
    mut row_cb: F,
) where
    F: FnMut(usize, &mut TableRow, egui::Id),
{
    let click_key = ui.scope_id().with(("row_click", id_salt));
    let mut row_h = if layout.row_h > 0.0 {
        layout.row_h
    } else {
        crate::scaling::scaled_font(ui.ctx(), crate::theme::ROW_H_COMPACT)
    };
    let mut scroll_y = layout.scroll_y;

    // 表格占满剩余高度：事件列表要像 PR/AR 一样整块可滚动，
    // 不能按内容高度撑开（那样外层滚动条会把内部滚动吃掉）。
    let area = ui.available_rect_before_wrap();
    apply_zoom(ui, area, &mut row_h, &mut scroll_y);
    let pending = layout.pending_scroll.take();

    let mut tb = TableBuilder::new(ui)
        .id_salt(id_salt)
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .max_scroll_height(f32::INFINITY)
        .min_scrolled_height(0.0)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    if let Some(offset) = pending {
        tb = tb.vertical_scroll_offset(offset.max(0.0));
    }
    for (_, min_w) in headers {
        tb = tb.column(Column::initial(*min_w).at_least(40.0).clip(true));
    }
    let out = tb
        .header(row_h, |mut h| {
            for (label, _) in headers {
                h.col(|ui| {
                    ui.label(egui::RichText::new(*label).strong().size(
                        crate::scaling::scaled_font(ui.ctx(), crate::theme::SMALL_FONT),
                    ));
                });
            }
        })
        .body(move |body| {
            body.rows(row_h, rows, move |mut row| {
                let i = row.index();
                row_cb(i, &mut row, click_key);
            });
        });

    // 单一真源：读回实际偏移与视口，供下一帧缩放锚点使用。
    layout.row_h = row_h;
    layout.scroll_y = out.state.clamped_offset().y;
    layout.view_h = out.inner_rect.height();
    layout.view_top = out.inner_rect.top();

    // 列边界拖拽：TableBuilder 未直接暴露 resize Response，从其内部 id
    // 读取本帧响应，指针命中列边界时补讲解行。
    let state_id = ui.scope_id().with(id_salt);
    for i in 0..headers.len() {
        if let Some(resp) = ui
            .ctx()
            .read_response(state_id.with("resize_column").with(i))
        {
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.eb.column_resize"));
        }
    }
}

/// 行高缩放范围（像素）。下限保证两行文字仍可读，上限避免单行占满整屏。
const ROW_H_MIN: f32 = 16.0;
const ROW_H_MAX: f32 = 48.0;

/// 按指针位置缩放行高（锚点补偿）。
fn apply_zoom(ui: &egui::Ui, area: egui::Rect, row_h: &mut f32, scroll_y: &mut f32) {
    let (zoom, hover) = ui.ctx().input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
    if (zoom - 1.0).abs() < 1e-4 {
        return;
    }
    let Some(pointer) = hover else {
        return;
    };
    if !area.contains(pointer) {
        return;
    }
    let base = *row_h;
    let new_h = (base * zoom).clamp(ROW_H_MIN, ROW_H_MAX);
    if (new_h - base).abs() < 1e-4 {
        return;
    }
    // 指针下的内容位置在缩放前后保持不变。
    let local_y = (pointer.y - area.top()).max(0.0);
    let frac = (*scroll_y + local_y) / base;
    *row_h = new_h;
    *scroll_y = (frac * new_h - local_y).max(0.0);
}

/// 行号 → tick 的只读索引。
///
/// 用回调而不是 `Vec<u32>`：事件列表可能有上百万行，每帧物化一份 tick 数组
/// 既费内存也费时间；Shift 范围选择只在真正点击时线性扫一次。
#[derive(Clone, Copy)]
pub(super) struct TickIndex<'a> {
    pub len: usize,
    tick_at: &'a dyn Fn(usize) -> u32,
}

impl<'a> TickIndex<'a> {
    pub fn new(len: usize, tick_at: &'a dyn Fn(usize) -> u32) -> Self {
        Self { len, tick_at }
    }

    pub fn get(&self, i: usize) -> u32 {
        if i < self.len { (self.tick_at)(i) } else { 0 }
    }
}

/// 取出 `build_table` 写入 memory 的行点击索引（若存在）。
pub(super) fn take_row_click(ui: &egui::Ui, id_salt: &str) -> Option<usize> {
    let key = ui.scope_id().with(("row_click", id_salt));
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
        ui.add(
            egui::Label::new(egui::RichText::new(s).size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::SMALL_FONT,
            )))
            .selectable(false),
        );
        // 整个 cell 加交互
        let cell_rect = ui.max_rect();
        let id = ui.scope_id().with("cell").with(row_idx);
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
            egui::Label::new(egui::RichText::new(bar_lookup.format(tick)).size(
                crate::scaling::scaled_font(ui.ctx(), crate::theme::SMALL_FONT),
            ))
            .selectable(false),
        );
        let cell_rect = ui.max_rect();
        let id = ui.scope_id().with("poscell").with(row_idx);
        let resp = ui.interact(cell_rect, id, egui::Sense::click());
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.eb.cell_position"));
        if resp.clicked() {
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
        if resp.secondary_clicked() {
            ui.ctx().memory_mut(|m| {
                m.data.insert_temp(
                    egui::Id::unique((id_salt, "edit_pos")),
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
/// `ticks`：所有行的 tick 索引（用于 Shift 范围选择）
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(super) fn cell_row_header(
    row: &mut TableRow,
    state: &mut EventBrowserState,
    id_salt: &str,
    row_idx: usize,
    tick: u32,
    ticks: &TickIndex<'_>,
    click_key: egui::Id,
) {
    row.col(|ui| {
        let is_selected = state.selected_ticks.contains(&tick);
        let label_color = if is_selected {
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_secondary()
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{}", row_idx + 1))
                    .size(crate::scaling::scaled_font(
                        ui.ctx(),
                        crate::theme::SMALL_FONT,
                    ))
                    .color(label_color),
            )
            .selectable(false),
        );
        let cell_rect = ui.max_rect();
        let id = ui.scope_id().with("rowhdr").with(row_idx);
        let resp = ui.interact(cell_rect, id, egui::Sense::click());
        // 选中行提示 Delete 删除，未选中提示多选/右键编辑。
        let row_hint = if is_selected {
            t!("hint.eb.delete_key")
        } else {
            t!("hint.eb.row_header")
        };
        crate::widgets::hint::hover(ui.ctx(), &resp, row_hint);

        // 左键：行选择（Ctrl/Shift 多选）
        if resp.clicked() {
            let modifiers = ui.ctx().input(|i| i.modifiers);
            handle_row_click(state, tick, ticks, modifiers);
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }

        // 右键：选中该行 + 弹出菜单
        if resp.secondary_clicked() && !state.selected_ticks.contains(&tick) {
            handle_row_click(state, tick, ticks, egui::Modifiers::NONE);
        }
        let edit_key = egui::Id::unique((id_salt, "edit"));
        resp.context_menu(|ui| {
            ui.set_min_width(160.0);
            ui.set_max_width(160.0);
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("event_browser.menu.insert_above").to_string(),
                ))
                .clicked()
            {
                ui.ctx().memory_mut(|m| {
                    m.data
                        .insert_temp(edit_key, EditRequest::InsertAbove { tick });
                });
                ui.close();
            }
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("event_browser.menu.insert_below").to_string(),
                ))
                .clicked()
            {
                ui.ctx().memory_mut(|m| {
                    m.data
                        .insert_temp(edit_key, EditRequest::InsertBelow { tick });
                });
                ui.close();
            }
            ui.separator();
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("common.delete").to_string(),
                ))
                .clicked()
            {
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
    ticks: &TickIndex<'_>,
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
            // 在可见范围内找所有 tick（Shift 范围选择只在点击时扫一次）。
            for i in 0..ticks.len {
                let t = ticks.get(i);
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
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        let icon_text = egui::RichText::new(ICON_ADD.codepoint)
            .family(ICON_ADD.font_family())
            .size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::ICON_FONT_XL,
            ))
            .color(crate::theme::text_bright());
        let resp = ui.add(
            egui::Label::new(icon_text)
                .selectable(false)
                .sense(egui::Sense::click()),
        );
        // hover 时叠加高亮图标（同 mode_bar 的 hover_highlight 机制）
        if resp.hovered() {
            ui.painter().text(
                resp.rect.center(),
                egui::Align2::CENTER_CENTER,
                ICON_ADD.codepoint,
                egui::FontId::new(
                    crate::scaling::scaled_font(ui.ctx(), crate::theme::ICON_FONT_XL),
                    ICON_ADD.font_family(),
                ),
                crate::theme::contrast_fg(),
            );
        }
        if resp.clicked() {
            clicked = true;
        }
        ui.label(
            egui::RichText::new(t!("event_browser.empty_add_hint").to_string())
                .size(crate::scaling::scaled_font(
                    ui.ctx(),
                    crate::theme::SMALL_FONT,
                ))
                .color(crate::theme::text_label()),
        );
    });
    if clicked {
        let edit_key = egui::Id::unique((id_salt, "edit"));
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp(edit_key, EditRequest::InsertFirst));
    }
    clicked
}

/// 处理 Delete/Backspace 键盘删除（当表格区域有焦点时）。
///
/// 返回 true 表示触发了删除。
pub(super) fn handle_delete_key(ui: &egui::Ui, id_salt: &str, has_selection: bool) -> bool {
    // 输入框聚焦（编辑 popup 的 DragValue/TextEdit）时按键应作用于文本，
    // 不应触发表格删除（与全局快捷键的 egui_wants_keyboard_input 保护一致）。
    if !has_selection || ui.ctx().egui_wants_keyboard_input() {
        return false;
    }
    let delete_pressed = ui
        .ctx()
        .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
    if delete_pressed {
        let edit_key = egui::Id::unique((id_salt, "edit"));
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp(edit_key, EditRequest::DeleteSelected));
        true
    } else {
        false
    }
}

/// 渲染可编辑文本单元格：左键跳转，右键写入 `EditRequest` 到 memory 触发 popup。
///
/// **关键**：edit key 用 `egui::Id::unique((id_salt, "edit"))` 全局 id，**不**用 `ui.scope_id()`。
/// 原因：cell 内的 `ui.scope_id()` 与 `apply_edit_popups` 调用处的 `ui.scope_id()` 不同
/// （cell 是 child ui），用 `ui.scope_id()` 会导致 write/read key 不匹配，popup 永远不触发。
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
        ui.add(
            egui::Label::new(egui::RichText::new(s).size(crate::scaling::scaled_font(
                ui.ctx(),
                crate::theme::SMALL_FONT,
            )))
            .selectable(false),
        );
        let cell_rect = ui.max_rect();
        let id = ui.scope_id().with("cell").with(row_idx);
        let resp = ui.interact(cell_rect, id, egui::Sense::click());
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.eb.cell_editable"));
        if resp.clicked() {
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(click_key, row_idx));
        }
        if resp.secondary_clicked() {
            // 全局 key：与 apply_edit_popups 的 Id::new((id_salt, "edit")) 对齐
            ui.ctx().memory_mut(|m| {
                m.data
                    .insert_temp(egui::Id::unique((id_salt, "edit")), edit_request);
            });
        }
    });
}

/// 查看 cell 写入的 `EditRequest`（不删除）。
///
/// `apply_*_popups` 每帧调用此函数，popup 显示期间 `EditRequest` 一直在 memory 里，
/// 直到 `remove_edit_request` 关闭时清除。
pub(super) fn peek_edit_request(ui: &egui::Ui, id_salt: &str) -> Option<EditRequest> {
    let key = egui::Id::unique((id_salt, "edit"));
    ui.memory(|m| m.data.get_temp::<EditRequest>(key))
}

/// 清除 `EditRequest`（popup 关闭时调用）。
pub(super) fn remove_edit_request(ui: &egui::Ui, id_salt: &str) {
    let key = egui::Id::unique((id_salt, "edit"));
    ui.memory_mut(|m| m.data.remove::<EditRequest>(key));
}

/// 查看位置 cell 写入的 `EditRequest`（不删除）。与 `peek_edit_request` 平行。
pub(super) fn peek_pos_edit_request(ui: &egui::Ui, id_salt: &str) -> Option<EditRequest> {
    let key = egui::Id::unique((id_salt, "edit_pos"));
    ui.memory(|m| m.data.get_temp::<EditRequest>(key))
}

/// 清除位置编辑请求（位置 popup 关闭时调用）。
pub(super) fn remove_pos_edit_request(ui: &egui::Ui, id_salt: &str) {
    let key = egui::Id::unique((id_salt, "edit_pos"));
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
