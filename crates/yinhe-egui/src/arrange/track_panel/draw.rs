use eframe::egui;

/// Paint an 18x18 inline button with a one-letter label and click handling.
pub fn draw_inline_button(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    active: bool,
    active_color: egui::Color32,
    id: egui::Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, egui::Sense::click());
    let hovered = resp.hovered();
    let pressed = resp.is_pointer_button_down_on();

    let (fill, text_col) = if active {
        let f = if pressed {
            crate::theme::pressed_color(active_color)
        } else if hovered {
            crate::theme::hover_color(active_color)
        } else {
            active_color
        };
        (f, egui::Color32::BLACK)
    } else {
        let base = crate::theme::btn_bg();
        let f = if pressed {
            crate::theme::pressed_color(base)
        } else if hovered {
            crate::theme::hover_color(base)
        } else {
            base
        };
        (f, crate::theme::text_secondary())
    };

    painter.rect_filled(rect, 3.0, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        text_col,
    );

    resp
}

/// 每轨力度表（向下单条）：自顶边起向下填充，`level`/`hold` 为归一化力度 0..1。
/// 与电平表（向上、L/R 双条）以方向区分；仅播放时非零。
pub fn velocity_meter(painter: &egui::Painter, rect: egui::Rect, level: f32, hold: f32) {
    painter.rect_filled(rect, 0.0, crate::theme::track_bg());
    let frac = level.clamp(0.0, 1.0);
    if frac > 0.0 {
        let h = rect.height() * frac;
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), h)),
            0.0,
            velocity_color(level),
        );
    }
    let hf = hold.clamp(0.0, 1.0);
    if hf > frac + 1e-4 {
        let y = rect.min.y + rect.height() * hf;
        painter.hline(
            rect.x_range(),
            y,
            egui::Stroke::new(1.0, velocity_color(hold)),
        );
    }
}

/// 力度着色：接近满力度红、>0.75 金、其余强调色。
fn velocity_color(v: f32) -> egui::Color32 {
    if v >= 0.96 {
        crate::theme::danger_text()
    } else if v > 0.75 {
        crate::theme::warning_gold()
    } else {
        crate::theme::accent_active()
    }
}
