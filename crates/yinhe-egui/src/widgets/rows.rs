//! 面板排版原语：章节标题 / 分隔 / 键值行 / 表单行 / 列表行。
//!
//! 右栏各面板（track/selection/anchor/layers/history/summary/event_browser）过去
//! 各写各的标题、标签值行、行高与缩进，视觉不统一。这里把共性收敛成少量原语：
//! 字号一律经 [`scaled_font`] 包装（跟随字体缩放），颜色取语义色，几何取自
//! [`crate::theme`] 的排版 token。

use eframe::egui;
use egui_extras::{Size, StripBuilder};

use crate::scaling::scaled_font;
use crate::theme;

/// 章节标题：粗体 + `SUB_TITLE_FONT` + `text_bright`。
pub(crate) fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(theme::GAP_SM);
    ui.label(
        egui::RichText::new(text)
            .strong()
            .size(scaled_font(ui.ctx(), theme::SUB_TITLE_FONT))
            .color(theme::text_bright()),
    );
}

/// 分隔块：`GAP` + 分隔线 + `GAP`。
pub(crate) fn divider(ui: &mut egui::Ui) {
    ui.add_space(theme::GAP);
    ui.separator();
    ui.add_space(theme::GAP);
}

/// 只读键值行：标签（`SMALL_FONT` + `text_label`）+ 值（`BODY_FONT` + `text_bright`）。
pub(crate) fn value_row(ui: &mut egui::Ui, label: impl Into<String>, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label.into())
                .size(scaled_font(ui.ctx(), theme::SMALL_FONT))
                .color(theme::text_label()),
        );
        ui.label(
            egui::RichText::new(value.into())
                .size(scaled_font(ui.ctx(), theme::BODY_FONT))
                .color(theme::text_bright()),
        );
    });
}

/// 表单行：固定宽标签列 + 右侧自定义控件（几何随字体缩放）。
pub(crate) fn form_row(ui: &mut egui::Ui, label: &str, add_control: impl FnOnce(&mut egui::Ui)) {
    let w = scaled_font(ui.ctx(), theme::FIELD_LABEL_W);
    let h = scaled_font(ui.ctx(), theme::ROW_H_COMPACT);
    // 左右列由 StripBuilder 分配，控件不会把标签列挤走。
    crate::widgets::grid::fixed_row(ui, theme::GAP_SM, h, w, |cells| {
        cells.cell(|ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .size(scaled_font(ui.ctx(), theme::SMALL_FONT))
                        .color(theme::text_label()),
                );
            });
        });
        cells.cell(|ui| {
            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center),
                add_control,
            );
        });
    });
}

/// 内联行：把「文本 + 更高控件」放在同一条水平线上时**必须**走它。
///
/// 裸 `ui.horizontal` 中先放的文本会按当时的行高落位，后续放进来的更高控件
/// 把行高撑大后文本不再移动 → 文本偏上、与控件不对齐（量化弹窗 n/d 的根因）。
/// 这里先按控件高分配行高再落内容，保证垂直居中。
pub(crate) fn inline_row(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let h = super::control::h(ui.ctx());
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), h),
        egui::Layout::left_to_right(egui::Align::Center),
        add_contents,
    );
}

/// 设置行：左侧标题（+ 可选的说明文字），右侧控件；行尾分隔。
///
/// `hint` 会渲染在标题下方（设置页的既有风格）。快捷键页**不传**说明，
/// 因为那一页信息密度高、说明会和键盘图抢空间，改用 [`setting_row_plain`]。
pub(crate) fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    hint: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    row_impl(ui, title, hint, true, add_control);
}

/// 无说明文字的设置行（快捷键页用）。
pub(crate) fn setting_row_plain(
    ui: &mut egui::Ui,
    title: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    row_impl(ui, title, "", true, add_control);
}

/// 面板行：左侧标题、右侧控件、垂直居中；**无**行间分隔线（右栏面板用，省空间）。
pub(crate) fn panel_row(ui: &mut egui::Ui, title: &str, add_control: impl FnOnce(&mut egui::Ui)) {
    row_impl(ui, title, "", false, add_control);
}

thread_local! {
    /// 页面级固定行宽（由滚动容器在进入页面时设定）。
    ///
    /// `setting_row` 用它作为行宽，而不是自己读 `ui.available_width()`：
    /// 后者会被上一行的内容宽度污染，导致右侧控件逐行右移。
    static PAGE_ROW_WIDTH: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// 设定页面级固定行宽（滚动容器用）。
///
/// 返回守卫，**离开作用域时自动还原**——避免中途早退/提前 return 时
/// 行宽残留在全局状态里，污染下一个页面。
pub(crate) fn page_row_width_scope(width: f32) -> PageRowWidthScope {
    PAGE_ROW_WIDTH.with(|w| w.set(Some(width)));
    PageRowWidthScope
}

/// [`page_row_width_scope`] 的作用域守卫。
pub(crate) struct PageRowWidthScope;

impl Drop for PageRowWidthScope {
    fn drop(&mut self) {
        PAGE_ROW_WIDTH.with(|w| w.set(None));
    }
}

/// 测试用：记录每个设置行里控件右缘与行右缘（`(标题, 控件右缘, 行右缘)`）。
#[cfg(test)]
pub(crate) fn take_row_probe() -> Vec<(String, f32, f32)> {
    ROW_PROBE
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

#[cfg(test)]
fn record_row_probe(title: &str, cell_right: f32, row_w: f32) {
    if let Ok(mut g) = ROW_PROBE.lock() {
        g.push((title.to_owned(), cell_right, row_w));
    }
}

#[cfg(test)]
static ROW_PROBE: std::sync::Mutex<Vec<(String, f32, f32)>> = std::sync::Mutex::new(Vec::new());

fn row_impl(
    ui: &mut egui::Ui,
    title: &str,
    hint: &str,
    divider: bool,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let label_w = scaled_font(ui.ctx(), theme::FIELD_LABEL_W);
    // 行宽取当前可用宽度。
    //
    // **不能**改用 `ui.max_rect().width()`：ScrollArea 内部用
    // `set_max_width` 收窄过内容宽度时，`max_rect` 仍是视口原始宽度，
    // 行会比容器宽、控件被顶到卡片外面（开关跑到右边界外就是这个）。
    // 行宽优先用页面级固定值；没有时退回当前可用宽度。
    let row_w = PAGE_ROW_WIDTH
        .with(|w| w.get())
        .unwrap_or_else(|| ui.available_width());
    if hint.is_empty() {
        // 单行：固定行高 + 两列（标签固定宽、控件剩余宽）。
        let h = super::control::h(ui.ctx());
        ui.allocate_ui_with_layout(
            egui::vec2(row_w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                StripBuilder::new(ui)
                    .size(Size::exact(label_w))
                    .size(Size::remainder())
                    .horizontal(|mut strip| {
                        strip.cell(|ui| {
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(title)
                                            .strong()
                                            .size(scaled_font(ui.ctx(), theme::SUB_TITLE_FONT)),
                                    );
                                },
                            );
                        });
                        strip.cell(|ui| {
                            // 每行一个稳定作用域：避免不同行的按钮/开关共用自动 Id，
                            // 出现“点 A 命中 B、B 本身点不动”的错位。
                            // 控件列宽 = 行宽 − 标签列宽 − 一个列间距。
                            // **不能**用 `ui.available_width()`：它会被上一次
                            // 绘制残留的 `max_rect` 影响，逐行漂移（每行右移
                            // 17.8px 那种累加误差就是这么来的）。
                            let cell_w = (row_w - label_w - theme::GAP_SM).max(1.0);
                            ui.set_max_width(cell_w);
                            ui.push_id(title, |ui| {
                                ui.set_max_width(cell_w);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| ui.name_inputs(title, add_control),
                                );
                            });
                            #[cfg(test)]
                            record_row_probe(title, cell_w, row_w);
                        });
                    });
            },
        );
    } else {
        // 带说明的行：标题行（左标题 + 右控件）在上，说明文字独占一行在下。
        //
        // 不能把「标题 + 说明」塞进固定宽的标签列（那是 72px）：说明会被
        // 挤成三四行，行高暴涨、控件也跟着错位。说明跨整行宽度才读得顺，
        // 控件位置也与无说明的行完全一致。
        let h = super::control::h(ui.ctx());
        ui.allocate_ui_with_layout(
            egui::vec2(row_w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                StripBuilder::new(ui)
                    .size(Size::exact(label_w))
                    .size(Size::remainder())
                    .horizontal(|mut strip| {
                        strip.cell(|ui| {
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(title)
                                            .strong()
                                            .size(scaled_font(ui.ctx(), theme::SUB_TITLE_FONT)),
                                    );
                                },
                            );
                        });
                        strip.cell(|ui| {
                            // 把 cell 的宽度**钉死**：`right_to_left` 布局里
                            // 控件若比 cell 宽（长下拉文字、步进器等），会把
                            // cell 的 `max_rect` 撑大，`StripBuilder` 按
                            // `rect.right()` 推进光标，于是下一行整体右移 ——
                            // 表现为「每行右侧控件都比上一行更靠右」。
                            // 控件列宽 = 行宽 − 标签列宽 − 一个列间距。
                            // **不能**用 `ui.available_width()`：它会被上一次
                            // 绘制残留的 `max_rect` 影响，逐行漂移（每行右移
                            // 17.8px 那种累加误差就是这么来的）。
                            let cell_w = (row_w - label_w - theme::GAP_SM).max(1.0);
                            ui.set_max_width(cell_w);
                            ui.push_id(title, |ui| {
                                ui.set_max_width(cell_w);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| ui.name_inputs(title, add_control),
                                );
                            });
                            #[cfg(test)]
                            record_row_probe(title, cell_w, row_w);
                        });
                    });
            },
        );
        ui.add_space(theme::GAP_TIGHT);
        // 说明必须**独占整行宽度**。
        //
        // 坑：上面 `StripBuilder` 的两列 cell 执行完后，父 ui 的可用宽度已被
        // 它们的 `max_rect` 收窄，直接 `ui.label` 会在残余宽度里换行 ——
        // 表现为说明被压成三四行、行高暴涨、控件跟着错位（用户截图即此）。
        // 因此这里显式按「父 ui 的 max_rect 宽度」分配一块新区域再画。
        // 先按整行宽度排版出高度，再按该高度分配区域：
        // `allocate_ui_with_layout` 传 0 高度会让 egui 拿剩余整块高度，
        // 文字被垂直居中、行距被拉大（表现为行高异常）。
        let galley = ui.painter().layout(
            hint.to_owned(),
            egui::FontId::proportional(scaled_font(ui.ctx(), theme::SMALL_FONT)),
            theme::text_secondary(),
            row_w,
        );
        let text_h = galley.size().y;
        ui.allocate_ui_with_layout(
            egui::vec2(row_w, text_h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(row_w);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(hint)
                            .size(scaled_font(ui.ctx(), theme::SMALL_FONT))
                            .color(theme::text_secondary()),
                    )
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
            },
        );
    }
    if divider {
        ui.add_space(theme::GAP);
        ui.separator();
        ui.add_space(theme::GAP);
    } else {
        ui.add_space(theme::GAP_TIGHT);
    }
}

/// 列表/树行：整行可点，选中/悬停底色 + 圆角。
///
/// 返回行 [`egui::Response`]；内容由 `add_contents` 在行内水平排布（垂直居中）。
/// 悬停用几何判定（`rect_contains_pointer`），避免行内交互控件顶掉行级 hover。
pub(crate) fn list_row(
    ui: &mut egui::Ui,
    selected: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let h = super::control::h(ui.ctx());
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, theme::ROW_RADIUS, theme::selected_bg());
    } else if ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, theme::ROW_RADIUS, theme::hover_color(theme::app_bg()));
    }
    let pad = scaled_font(ui.ctx(), theme::PAD_X);
    let inner = egui::Rect::from_min_max(rect.min + egui::vec2(pad, 0.0), rect.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::GAP_TIGHT;
            add_contents(ui);
        },
    );
    resp
}
