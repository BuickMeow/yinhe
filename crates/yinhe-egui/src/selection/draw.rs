use eframe::egui;

/// Draw a selection rectangle with conditional edge strokes.
///
/// `snapped_view_rect` — snapped selection bounds in **view-local** pixels
/// (before offsetting by `content_rect.min`).
/// The function converts to screen coordinates, clips, and draws fill +
/// strokes on edges that were **not** clipped by the content boundary.
pub fn draw(painter: &egui::Painter, content_rect: egui::Rect, snapped_view_rect: egui::Rect) {
    let sel_raw = egui::Rect::from_min_max(
        egui::pos2(
            content_rect.min.x + snapped_view_rect.min.x,
            content_rect.min.y + snapped_view_rect.min.y,
        ),
        egui::pos2(
            content_rect.min.x + snapped_view_rect.max.x,
            content_rect.min.y + snapped_view_rect.max.y,
        ),
    );
    let sel = sel_raw.intersect(content_rect);

    // Detect which edges were clipped
    let clipped = [
        sel.min.y != sel_raw.min.y, // top
        sel.max.x != sel_raw.max.x, // right
        sel.max.y != sel_raw.max.y, // bottom
        sel.min.x != sel_raw.min.x, // left
    ];

    // Fill
    painter.rect_filled(sel, 0.0, egui::Color32::WHITE.gamma_multiply(0.15));

    // Strokes — only draw edges that weren't clipped
    let stroke = egui::Stroke::new(1.0, egui::Color32::WHITE.gamma_multiply(0.40));
    let [t, r, b, l] = clipped;
    if !t {
        painter.line_segment(
            [
                egui::pos2(sel.min.x, sel.min.y),
                egui::pos2(sel.max.x, sel.min.y),
            ],
            stroke,
        );
    }
    if !r {
        painter.line_segment(
            [
                egui::pos2(sel.max.x, sel.min.y),
                egui::pos2(sel.max.x, sel.max.y),
            ],
            stroke,
        );
    }
    if !b {
        painter.line_segment(
            [
                egui::pos2(sel.max.x, sel.max.y),
                egui::pos2(sel.min.x, sel.max.y),
            ],
            stroke,
        );
    }
    if !l {
        painter.line_segment(
            [
                egui::pos2(sel.min.x, sel.max.y),
                egui::pos2(sel.min.x, sel.min.y),
            ],
            stroke,
        );
    }
}
