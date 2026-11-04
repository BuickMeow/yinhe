use eframe::egui;

/// Convert a snapped view-local rect to screen-space rect (clipped to content).
/// Shared by both Select and Eraser tool drawing.
pub fn snapped_to_screen(content_rect: egui::Rect, snapped_view_rect: egui::Rect) -> egui::Rect {
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
    sel_raw.intersect(content_rect)
}

/// Draw a selection rectangle with conditional edge strokes.
///
/// `snapped_view_rect` — snapped selection bounds in **view-local** pixels
/// (before offsetting by `content_rect.min`).
/// The function converts to screen coordinates, clips, and draws fill +
/// strokes on edges that were **not** clipped by the content boundary.
///
/// `fill_color` / `stroke_color` — base colors (alpha will be applied via
/// `gamma_multiply`).
pub fn draw(
    painter: &egui::Painter,
    content_rect: egui::Rect,
    snapped_view_rect: egui::Rect,
    fill_color: egui::Color32,
    stroke_color: egui::Color32,
) {
    let sel = snapped_to_screen(content_rect, snapped_view_rect);
    // Reconstruct raw rect for clip detection
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

    // 选框完全在 content 区域之外时，交集退化为零面积矩形，跳过绘制
    if !sel.is_positive() {
        return;
    }

    // Detect which edges were clipped
    let clipped = [
        sel.min.y != sel_raw.min.y, // top
        sel.max.x != sel_raw.max.x, // right
        sel.max.y != sel_raw.max.y, // bottom
        sel.min.x != sel_raw.min.x, // left
    ];

    // Fill
    painter.rect_filled(
        sel,
        0.0,
        fill_color.gamma_multiply(crate::theme::marquee_fill_alpha()),
    );

    // Strokes — only draw edges that weren't clipped
    let stroke = egui::Stroke::new(
        1.0,
        stroke_color.gamma_multiply(crate::theme::marquee_stroke_alpha()),
    );
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

/// 在选框上画右侧三个方块：重复 / 整体变速 / 批量 gate。
///
/// `screen_rect` 是包含键盘偏移后的屏幕坐标矩形（与 `music_sel_to_pixel_rect`
/// 加 `content_rect.min` 后的结果一致）。`gate` 由调用方决定是否画（AR 不画）。
pub fn draw_handles(
    painter: &egui::Painter,
    view: &yinhe_types::PianoRollView,
    screen_rect: egui::Rect,
    quantize: yinhe_editor_core::quantize::QuantizePreset,
    ppq: u32,
    show_gate: bool,
) {
    let interval_px = quantize.tick_interval(ppq).max(1) as f32 * view.base.pixels_per_tick;
    let (corner, scale, gate) =
        super::drag::selection_handle_centers(view, screen_rect, interval_px);
    let fill = crate::theme::app_bg();
    let stroke = egui::Stroke::new(
        1.5,
        crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_stroke_alpha()),
    );
    let square = |p: egui::Pos2| {
        let r = egui::Rect::from_center_size(p, egui::vec2(9.0, 9.0));
        painter.rect_filled(r, 0.0, fill);
        painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
    };
    square(corner);
    square(scale);
    if show_gate && let Some(gate) = gate {
        square(gate);
    }
}

/// AR 选框右侧两个方块：右下角重复、右缘中点整体变速（AR 没有 gate）。
pub fn draw_arrange_handles(painter: &egui::Painter, screen_rect: egui::Rect) {
    let corner = egui::pos2(screen_rect.max.x, screen_rect.max.y);
    let scale = egui::pos2(screen_rect.max.x, screen_rect.center().y);
    let fill = crate::theme::app_bg();
    let stroke = egui::Stroke::new(
        1.5,
        crate::theme::contrast_fg().gamma_multiply(crate::theme::marquee_stroke_alpha()),
    );
    for p in [corner, scale] {
        let r = egui::Rect::from_center_size(p, egui::vec2(9.0, 9.0));
        painter.rect_filled(r, 0.0, fill);
        painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
    }
}
