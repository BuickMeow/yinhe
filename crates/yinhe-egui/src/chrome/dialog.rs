use eframe::egui;
#[cfg(not(target_os = "macos"))]
use egui_material_icons::icons::*;

/// Bring a child viewport to the front if it already exists.
///
/// egui does not automatically raise an existing sub-viewport when the user
/// clicks the "open window" button again (e.g. after the sub-window was hidden
/// behind the main window). This helper sends `Visible(true)` + `Focus` to the
/// given viewport, which on all platforms activates and raises the window.
///
/// Safe to call every frame; idempotent.
pub(crate) fn raise_viewport(ctx: &egui::Context, id: egui::ViewportId) {
    ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
}

/// Build a `ViewportBuilder` for a dialog window, matching the main window's
/// custom chrome style (no native title bar).
pub(crate) fn viewport_builder(
    title: &str,
    size: [f32; 2],
    resizable: bool,
) -> egui::ViewportBuilder {
    let mut vb = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(resizable);

    #[cfg(target_os = "macos")]
    {
        vb = vb
            .with_transparent(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }

    #[cfg(not(target_os = "macos"))]
    {
        vb = vb.with_decorations(false);
    }

    vb
}

/// 对话框内容区标准布局：主体内容垂直居中占据按钮行以上的空间，
/// 按钮行用 `bottom_up` 布局固定贴底。
///
/// `btn_zone_h` 是底部按钮区预留高度（按钮行 + 间距）。
/// `content` 在 `top_down(Center)` 布局中执行，`buttons` 在 `bottom_up(Center)` 中执行。
pub(crate) fn content_with_bottom_buttons(
    ui: &mut egui::Ui,
    btn_zone_h: f32,
    content: impl FnOnce(&mut egui::Ui),
    buttons: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(
            ui.available_width(),
            (ui.available_height() - btn_zone_h).max(0.0),
        ),
        egui::Layout::top_down(egui::Align::Center),
        content,
    );
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), buttons);
}

/// 绘制统一的 "X" 关闭按钮（对话框标题栏与文档 tab 共用）：
/// hover 时红色底 + 图标变白，按下时按统一按下增益加深，其余灰色。
pub(crate) fn paint_close_button(
    painter: &egui::Painter,
    rect: egui::Rect,
    hovered: bool,
    pressed: bool,
) {
    if hovered {
        let bg = if pressed {
            crate::theme::pressed_color(crate::theme::danger_hover())
        } else {
            crate::theme::danger_hover()
        };
        painter.rect_filled(rect, 4.0, bg);
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_material_icons::icons::ICON_CLOSE.codepoint,
        egui::FontId::new(
            crate::theme::ICON_FONT_SM,
            egui_material_icons::icons::ICON_CLOSE.font_family(),
        ),
        if hovered {
            crate::theme::contrast_fg()
        } else {
            crate::theme::text_muted()
        },
    );
}

/// Draw a custom title bar for a dialog window.
///
/// - macOS: draws a background colour strip, centered title, and drag
///   region. The native traffic-light buttons remain visible and functional
///   (via `with_fullsize_content_view`).
/// - Other platforms: draws an X close button on the right, centered title,
///   and drag region.
///
/// Sets `*close = true` when the close button is clicked.
pub(crate) fn title_bar(ui: &mut egui::Ui, title: &str, close: &mut bool) {
    #[cfg(target_os = "macos")]
    let _ = close;

    let height = crate::theme::TITLE_BAR_H;
    let bar_rect = ui.max_rect();

    // 不画背景条：弹窗的 CentralPanel frame 已铺满 app_bg（叠两层是 bug）。

    // ── Close button (right side, non-macOS only) ──
    #[cfg(not(target_os = "macos"))]
    {
        let close_rect = egui::Rect::from_min_size(
            egui::pos2(bar_rect.max.x - height, bar_rect.min.y),
            egui::vec2(height, height),
        );
        let close_hover =
            close_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
        let close_pressed =
            close_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        paint_close_button(&ui.painter(), close_rect, close_hover, close_pressed);
        if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
            && close_rect.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default())
        {
            *close = true;
        }
    }

    // ── Centered title (both platforms) ──
    ui.painter().text(
        egui::pos2(bar_rect.center().x, bar_rect.min.y + height / 2.0),
        egui::Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
        crate::theme::text_bright(),
    );

    // ── Drag region (both platforms) ──
    // 用固定 Id（非 next_auto_id），否则双击跨帧 Id 变化导致 double_clicked 永远为 false
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(bar_rect.min.x, bar_rect.min.y),
        egui::pos2(bar_rect.max.x, bar_rect.min.y + height),
    );
    // 非 macOS 的关闭按钮区域不参与拖拽/双击
    #[cfg(not(target_os = "macos"))]
    let close_rect_for_drag = egui::Rect::from_min_size(
        egui::pos2(bar_rect.max.x - height, bar_rect.min.y),
        egui::vec2(height, height),
    );
    let drag_resp = ui.interact(
        drag_rect,
        ui.id().with("dialog_title_drag"),
        egui::Sense::click_and_drag(),
    );
    if drag_resp.dragged_by(egui::PointerButton::Primary) {
        #[cfg(not(target_os = "macos"))]
        {
            let over_close = ui.input(|i| {
                i.pointer
                    .press_origin()
                    .is_some_and(|p| close_rect_for_drag.contains(p))
            });
            if !over_close {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
        #[cfg(target_os = "macos")]
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }
    // 双击标题栏切换最大化/还原（与主窗口 title_bar/transport_bar 一致：400ms 内两次单击空白区）
    const DOUBLE_CLICK_MS: f64 = 400.0;
    let dbl_id = ui.id().with("dialog_title_dbl_click");
    if drag_resp.clicked_by(egui::PointerButton::Primary) {
        // 关闭按钮上的单击不计入双击
        #[cfg(not(target_os = "macos"))]
        let on_close = drag_resp
            .interact_pointer_pos()
            .is_some_and(|p| close_rect_for_drag.contains(p));
        #[cfg(target_os = "macos")]
        let on_close = false;
        if !on_close {
            let now = ui.input(|i| i.time);
            let last: f64 = ui.data_mut(|d| d.get_persisted(dbl_id)).unwrap_or(0.0);
            if now - last < DOUBLE_CLICK_MS / 1000.0 {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                ui.data_mut(|d| d.insert_persisted(dbl_id, 0.0));
            } else {
                ui.data_mut(|d| d.insert_persisted(dbl_id, now));
            }
        }
    }

    // Reserve space
    ui.allocate_space(egui::vec2(0.0, height));
}
