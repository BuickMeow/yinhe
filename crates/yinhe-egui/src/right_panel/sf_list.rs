use eframe::egui;

use crate::right_panel::config::SfEntry;

/// A reusable, compact list of SoundFont entries with checkboxes.
///
/// Returns `true` if the list was modified (toggle, reorder, remove).
pub fn sf_list(ui: &mut egui::Ui, entries: &mut Vec<SfEntry>) -> bool {
    let mut changed = false;

    // Use a simple drag-reorder approach via stored indices.
    let drag_id = ui.id().with("sf_drag");
    let mut drag_state: Option<(usize, usize)> =
        ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);

    let mut remove_idx: Option<usize> = None;

    // ── Render rows ──
    let total = entries.len();
    for i in 0..total {
        let (row_changed, action) = sf_row(ui, &mut entries[i], i, total);
        if row_changed {
            changed = true;
        }

        // Track drag reorder
        if let Some((origin, _)) = drag_state {
            if action == Some(SfAction::Dragging) && origin != i {
                // Hovering over a different row while dragging
                drag_state = Some((origin, i));
                ui.data_mut(|d| d.insert_persisted(drag_id, drag_state));
            }
        }

        match action {
            Some(SfAction::Remove) => remove_idx = Some(i),
            Some(SfAction::StartDrag) => {
                drag_state = Some((i, i));
                ui.data_mut(|d| d.insert_persisted(drag_id, drag_state));
            }
            Some(SfAction::MoveUp) if i > 0 => {
                entries.swap(i, i - 1);
                changed = true;
            }
            Some(SfAction::MoveDown) if i < total - 1 => {
                entries.swap(i, i + 1);
                changed = true;
            }
            _ => {}
        }
    }

    // Apply reorder on drop
    if let Some((src, dst)) = drag_state {
        if !ui.input(|i| i.pointer.any_down()) && src != dst {
            // Sort-of-reorder by remove/insert
            if src < entries.len() && dst < entries.len() {
                let e = entries.remove(src);
                entries.insert(dst.min(entries.len()), e);
                changed = true;
            }
            ui.data_mut(|d| d.insert_persisted::<Option<(usize, usize)>>(drag_id, None));
        }
    }

    if let Some(idx) = remove_idx {
        entries.remove(idx);
        changed = true;
    }

    changed
}

#[derive(Clone, Copy, PartialEq)]
enum SfAction {
    Remove,
    MoveUp,
    MoveDown,
    StartDrag,
    Dragging,
}

/// Render a single SF entry row. Returns (changed, action).
fn sf_row(
    ui: &mut egui::Ui,
    entry: &mut SfEntry,
    index: usize,
    total: usize,
) -> (bool, Option<SfAction>) {
    let height = 24.0;
    let id = ui.id().with(format!("sf_{}", index));

    // Allocate space and get the rect
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );
    let resp = ui.interact(rect, id, egui::Sense::click_and_drag());
    let hovered = resp.hovered();

    // Background highlight
    if hovered {
        ui.painter()
            .rect_filled(rect, 2.0, egui::Color32::from_black_alpha(20));
    }

    // ── Checkbox ──
    let cb_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 4.0, rect.center().y - 6.0),
        egui::pos2(rect.min.x + 16.0, rect.center().y + 6.0),
    );
    let cb_resp = ui.interact(cb_rect, id.with("cb"), egui::Sense::click());
    if cb_resp.clicked() {
        entry.enabled = !entry.enabled;
        return (true, None);
    }

    // Draw checkbox
    let cb_color = if entry.enabled {
        crate::widgets::theme::ACCENT_ACTIVE
    } else {
        egui::Color32::GRAY
    };
    ui.painter().rect_filled(cb_rect, 2.0, cb_color);
    if entry.enabled {
        ui.painter().text(
            cb_rect.center(),
            egui::Align2::CENTER_CENTER,
            "✓",
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );
    }

    // ── Name ──
    let name_x = rect.min.x + 22.0;
    ui.painter().text(
        egui::pos2(name_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &entry.name,
        egui::FontId::proportional(12.0),
        egui::Color32::WHITE,
    );

    // Approximate text end
    let name_end = name_x + entry.name.len() as f32 * 7.0 + 8.0;

    // ── Path (truncated) ──
    let path_x = name_end.max(rect.min.x + 120.0);
    let path_text = if entry.path.len() > 40 {
        format!("…{}", &entry.path[entry.path.len() - 37..])
    } else {
        entry.path.clone()
    };
    ui.painter().text(
        egui::pos2(path_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        path_text,
        egui::FontId::proportional(10.0),
        egui::Color32::from_gray(120),
    );

    // ── Drag handle ──
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(rect.max.x - 16.0, rect.min.y),
        egui::pos2(rect.max.x - 4.0, rect.max.y),
    );
    ui.painter().text(
        drag_rect.center(),
        egui::Align2::CENTER_CENTER,
        "⠿",
        egui::FontId::proportional(10.0),
        egui::Color32::GRAY,
    );

    // ── Detect drag start ──
    if resp.drag_started() {
        return (false, Some(SfAction::StartDrag));
    }

    // ── Context menu on right-click ──
    let mut action: Option<SfAction> = None;
    resp.context_menu(|ui| {
        ui.set_min_width(100.0);
        if index > 0 && ui.button("上移").clicked() {
            action = Some(SfAction::MoveUp);
            ui.close_menu();
        }
        if index < total - 1 && ui.button("下移").clicked() {
            action = Some(SfAction::MoveDown);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("删除").clicked() {
            action = Some(SfAction::Remove);
            ui.close_menu();
        }
    });

    if let Some(a) = action {
        return (false, Some(a));
    }

    (false, None)
}
