use eframe::egui;
use egui_material_icons::icons::ICON_DRAG_INDICATOR;
use rust_i18n::t;

use yinhe_editor_core::config::SfEntry;

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
        if let Some((origin, _)) = drag_state
            && action == Some(SfAction::Dragging)
            && origin != i
        {
            // Hovering over a different row while dragging
            drag_state = Some((origin, i));
            ui.data_mut(|d| d.insert_persisted(drag_id, drag_state));
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
    if let Some((src, dst)) = drag_state
        && !ui.input(|i| i.pointer.any_down())
        && src != dst
    {
        // Sort-of-reorder by remove/insert
        if src < entries.len() && dst < entries.len() {
            let e = entries.remove(src);
            entries.insert(dst.min(entries.len()), e);
            changed = true;
        }
        ui.data_mut(|d| d.insert_persisted::<Option<(usize, usize)>>(drag_id, None));
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
/// 两行布局：第一行名称、第二行路径，复选框垂直居中，右侧拖拽手柄。
fn sf_row(
    ui: &mut egui::Ui,
    entry: &mut SfEntry,
    index: usize,
    total: usize,
) -> (bool, Option<SfAction>) {
    let height = 40.0;
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

    // ── Checkbox（原生控件，矢量对勾不依赖字体，垂直居中于两行）──
    // 每行的 ui.id() 相同，push_id 保证 checkbox 的自动 id 唯一。
    let cb_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 4.0, rect.center().y - 9.0),
        egui::pos2(rect.min.x + 22.0, rect.center().y + 9.0),
    );
    let cb_changed = ui
        .push_id(("sf_cb", index), |ui| {
            ui.put(cb_rect, egui::Checkbox::new(&mut entry.enabled, ""))
        })
        .inner
        .changed();
    if cb_changed {
        return (true, None);
    }

    let text_x = rect.min.x + 22.0;

    // ── 第一行：名称 ──
    ui.painter().text(
        egui::pos2(text_x, rect.min.y + 10.0),
        egui::Align2::LEFT_CENTER,
        &entry.name,
        egui::FontId::proportional(12.0),
        egui::Color32::WHITE,
    );

    // ── 第二行：路径（截断，灰色小字）──
    ui.painter().text(
        egui::pos2(text_x, rect.min.y + 28.0),
        egui::Align2::LEFT_CENTER,
        truncate_path(&entry.path),
        egui::FontId::proportional(10.0),
        crate::theme::TEXT_DIM,
    );

    // ── Drag handle（Material 图标）──
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(rect.max.x - 16.0, rect.min.y),
        egui::pos2(rect.max.x - 4.0, rect.max.y),
    );
    ui.painter().text(
        drag_rect.center(),
        egui::Align2::CENTER_CENTER,
        ICON_DRAG_INDICATOR.codepoint,
        egui::FontId::new(14.0, ICON_DRAG_INDICATOR.font_family()),
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
        if index > 0 && ui.button(t!("sf_list.move_up").as_ref()).clicked() {
            action = Some(SfAction::MoveUp);
            ui.close();
        }
        if index < total - 1 && ui.button(t!("sf_list.move_down").as_ref()).clicked() {
            action = Some(SfAction::MoveDown);
            ui.close();
        }
        ui.separator();
        if ui.button(t!("sf_list.delete").as_ref()).clicked() {
            action = Some(SfAction::Remove);
            ui.close();
        }
    });

    if let Some(a) = action {
        return (false, Some(a));
    }

    (false, None)
}

/// 截断音色库路径用于显示：超过 40 字符时保留尾部 37 字符、前缀加省略号。
///
/// 必须按字符（而非字节）截断：按字节切片可能落在多字节 UTF-8 字符中间，
/// 中文路径（如「下载/钢琴音色库/xxx.sf2」）会触发 char boundary panic，
/// 在 release 构建（panic=abort）下直接闪退。
fn truncate_path(path: &str) -> String {
    if path.chars().count() > 40 {
        // nth_back(36) = 尾部第 37 个字符（nth_back(0) 是最后一个），
        // 保证截断后恰为尾部 37 个字符，且起始索引必在字符边界上。
        let start = path
            .char_indices()
            .nth_back(36)
            .map(|(i, _)| i)
            .unwrap_or(0);
        format!("…{}", &path[start..])
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_path;

    /// 回归测试：中文字符路径截断必须安全（旧实现按字节切片会 panic 闪退）。
    #[test]
    fn cjk_path_truncation_is_char_boundary_safe() {
        // 44 个字符，超过截断阈值 40
        let path = "/Users/jieneng/下载/钢琴音色库合集/斯坦威大钢琴精选音源完整版.sf2";
        let t = truncate_path(path);
        assert!(t.starts_with('…'));
        assert!(t.chars().count() <= 38);
        // 截断结果必须是合法 UTF-8（字节切片落在字符中间时会 panic）
        assert!(t.is_char_boundary(0));
    }

    #[test]
    fn ascii_path_truncation() {
        let path = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
        let t = truncate_path(path);
        assert!(t.starts_with('…'));
        assert!(t.ends_with("(No Hammer).sfz"));
    }

    #[test]
    fn short_path_kept_as_is() {
        assert_eq!(truncate_path("short.sf2"), "short.sf2");
    }
}
