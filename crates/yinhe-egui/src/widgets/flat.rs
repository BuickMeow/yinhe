//! 无边框按钮/选项（项目规范：按钮与选项不带边框）。
//!
//! 为什么不用 `egui::Button`：egui 的按钮边框与输入框共用
//! `visuals.widgets.*.bg_stroke`，无法单独去掉按钮边框；带边框的按钮
//! 悬停时边框出现/变化会让内容视觉位移。这里自绘：
//! 常态无背景无描边、悬停/按下只变背景色。

use eframe::egui;

/// 无边框按钮：宽度自适应文字，居中显示；hover/按下只变背景。
pub(crate) fn flat_button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    flat_button_ex(ui, text, None, None, true, false)
}

/// 无边框按钮（可设最小尺寸；`enabled = false` 置灰不可点）。
pub(crate) fn flat_button_sized(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    min_size: egui::Vec2,
    enabled: bool,
) -> egui::Response {
    flat_button_ex(ui, text, Some(min_size), None, enabled, false)
}

/// 无边框按钮（固定尺寸，内容居中；MIX 条等紧凑布局用）。
pub(crate) fn flat_button_fixed(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    size: egui::Vec2,
) -> egui::Response {
    flat_button_ex(ui, text, Some(size), None, true, true)
}

/// 无边框按钮（固定尺寸 + 自定义选中背景；transport 等图标按钮用）。
/// 固定尺寸而非 min：图标 galley 左右 padding 大于上下，用 min 会把宽度撑大
/// （如 32×32 目标被撑成 38×32），图标按钮必须严格正方形。
pub(crate) fn flat_button_custom(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    size: egui::Vec2,
    selected_bg: Option<egui::Color32>,
    enabled: bool,
) -> egui::Response {
    flat_button_ex(ui, text, Some(size), selected_bg, enabled, true)
}

/// 无边框选项按钮：`selected` 时用选中背景 + 强调色文字（替代 `selectable_label`）。
pub(crate) fn flat_selected(
    ui: &mut egui::Ui,
    selected: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    flat_button_ex(
        ui,
        text,
        None,
        selected.then(crate::theme::selected_bg),
        true,
        false,
    )
}

/// 无边框单选值：点击写入 `*current = value`（替代 `selectable_value`）。
pub(crate) fn flat_selectable_value<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let resp = flat_selected(ui, *current == value, text);
    if resp.clicked() {
        *current = value;
    }
    resp
}

/// 自绘实现：解析文本 → 布局 → 分配 rect → 画背景与文字。
fn flat_button_ex(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    min_size: Option<egui::Vec2>,
    selected_bg: Option<egui::Color32>,
    enabled: bool,
    fixed_size: bool,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let scale = |v: f32| crate::scaling::scaled_font(&ctx, v);
    let galley = text
        .into()
        .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button);
    let pad_x = scale(10.0);
    let pad_y = scale(5.0);
    let mut size = egui::vec2(galley.size().x + pad_x * 2.0, galley.size().y + pad_y * 2.0);
    size.y = size.y.max(scale(24.0));
    if let Some(min) = min_size {
        if fixed_size {
            // 固定尺寸：忽略内容自然尺寸（紧凑条用），内容居中可能超出。
            size = min;
        } else {
            size.x = size.x.max(min.x);
            size.y = size.y.max(min.y);
        }
    }
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let resp = if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, galley.text().to_owned())
    });
    if ui.is_rect_visible(rect) {
        let bg = if !enabled {
            egui::Color32::TRANSPARENT
        } else if let Some(sel) = selected_bg {
            sel
        } else if resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(crate::theme::btn_bg())
        } else if resp.hovered() {
            crate::theme::hover_color(crate::theme::btn_bg())
        } else {
            egui::Color32::TRANSPARENT
        };
        if bg != egui::Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, scale(4.0), bg);
        }
        let color = if !enabled {
            crate::theme::text_disabled()
        } else if selected_bg.is_some() {
            crate::theme::accent_active()
        } else {
            crate::theme::text_primary()
        };
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    resp
}
