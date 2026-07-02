use eframe::egui;
use yinhe_types::view_base::TimelineViewBase;

/// Auto-scroll the view when the pointer is near the edges of `content_rect`.
/// Returns the actual (dx, dy) scroll delta applied, so callers can compensate
/// drag anchors.
///
/// `clamp_fn` is called after modifying scroll to enforce bounds.
/// It receives `(content_width, content_height)` and should call
/// `view.base.clamp_scroll_x(...)` etc.
pub fn auto_scroll_on_drag(
    ui: &egui::Ui,
    base: &mut TimelineViewBase,
    content_rect: egui::Rect,
    pos: egui::Pos2,
    clamp_fn: impl FnOnce(&mut TimelineViewBase, f32, f32),
) -> (f32, f32) {
    const MARGIN: f32 = 20.0;
    const BASE_SPEED: f32 = 15.0;
    let dt = ui.input(|i| i.unstable_dt);
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;

    if pos.x < content_rect.min.x + MARGIN {
        dx = -(content_rect.min.x + MARGIN - pos.x) * BASE_SPEED * dt;
    } else if pos.x > content_rect.max.x - MARGIN {
        dx = (pos.x - (content_rect.max.x - MARGIN)) * BASE_SPEED * dt;
    }

    if pos.y < content_rect.min.y + MARGIN {
        dy = -(content_rect.min.y + MARGIN - pos.y) * BASE_SPEED * dt;
    } else if pos.y > content_rect.max.y - MARGIN {
        dy = (pos.y - (content_rect.max.y - MARGIN)) * BASE_SPEED * dt;
    }

    if dx != 0.0 || dy != 0.0 {
        let old_x = base.scroll_x;
        let old_y = base.scroll_y;
        base.scroll_x += dx;
        base.scroll_y += dy;
        clamp_fn(base, content_rect.width(), content_rect.height());
        let actual_dx = base.scroll_x - old_x;
        let actual_dy = base.scroll_y - old_y;
        if actual_dx != 0.0 || actual_dy != 0.0 {
            base.dirty = true;
            ui.ctx().request_repaint();
            return (actual_dx, actual_dy);
        }
    }
    (0.0, 0.0)
}

/// Convert a persisted music selection `(t_start, t_end, key_lo, key_hi)` to
/// a pixel-space `Rect` in the pianoroll view.
pub fn music_sel_to_pixel_rect(
    base: &TimelineViewBase,
    key_height: f32,
    t_start: f64,
    t_end: f64,
    key_lo: u8,
    key_hi: u8,
) -> egui::Rect {
    let kh = key_height;
    let scroll_y = base.scroll_y;
    let sy = (127.0 - key_hi as f32) * kh - scroll_y;
    let ey = (127.0 - key_lo as f32 + 1.0) * kh - scroll_y;
    let sx = base.tick_to_x(t_start);
    let ex = base.tick_to_x(t_end);
    egui::Rect::from_min_max(
        egui::pos2(sx.min(ex), sy.min(ey)),
        egui::pos2(sx.max(ex), sy.max(ey)),
    )
}
