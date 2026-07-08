use eframe::egui;
#[cfg(not(target_os = "macos"))]
use egui_material_icons::icons::*;

/// Build a `ViewportBuilder` for a dialog window, matching the main window's
/// custom chrome style (no native title bar, transparent background).
pub(crate) fn viewport_builder(
    title: &str,
    size: [f32; 2],
    resizable: bool,
) -> egui::ViewportBuilder {
    let mut vb = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(resizable)
        .with_transparent(true);

    #[cfg(target_os = "macos")]
    {
        vb = vb
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

    // Background strip
    ui.painter().rect_filled(
        egui::Rect::from_min_size(bar_rect.min, egui::vec2(bar_rect.max.x, height)),
        0.0,
        crate::theme::APP_BG,
    );

    // ── Close button (right side, non-macOS only) ──
    #[cfg(not(target_os = "macos"))]
    {
        let close_rect = egui::Rect::from_min_size(
            egui::pos2(bar_rect.max.x - height, bar_rect.min.y),
            egui::vec2(height, height),
        );
        let close_hover =
            close_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
        if close_hover {
            ui.painter()
                .rect_filled(close_rect, 0.0, crate::theme::DANGER_HOVER);
        }
        if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
            && close_rect.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default())
        {
            *close = true;
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_CLOSE.codepoint,
            egui::FontId::new(12.0, ICON_CLOSE.font_family()),
            if close_hover {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_gray(160)
            },
        );
    }

    // ── Centered title (both platforms) ──
    ui.painter().text(
        egui::pos2(bar_rect.center().x, bar_rect.min.y + height / 2.0),
        egui::Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(13.0),
        egui::Color32::from_gray(200),
    );

    // ── Drag region (both platforms) ──
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(bar_rect.min.x, bar_rect.min.y),
        egui::pos2(bar_rect.max.x, bar_rect.min.y + height),
    );
    let drag_resp = ui.interact(drag_rect, ui.next_auto_id(), egui::Sense::drag());
    if drag_resp.dragged_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    // Reserve space
    ui.allocate_space(egui::vec2(0.0, height));
}
