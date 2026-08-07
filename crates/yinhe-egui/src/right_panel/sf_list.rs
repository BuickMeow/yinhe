use eframe::egui;
use egui_material_icons::icons::ICON_DRAG_INDICATOR;
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use yinhe_editor_core::config::SfEntry;

/// 行高（两行布局：第一行名称、第二行路径）。
const ROW_H: f32 = 40.0;
/// 拖拽指针贴近可视区边缘时的自动滚动速度（px/帧）。
const AUTO_SCROLL_SPEED: f32 = 32.0;
/// 触发自动滚动的边缘距离。
const AUTO_SCROLL_MARGIN: f32 = 20.0;

/// 列表跨帧状态：多选 + 拖拽排序。
/// `salt` 区分不同列表（全局/各 port），防止状态串用。
#[derive(Default, Clone, Serialize, Deserialize)]
struct ListState {
    /// 选中的行（已排序）。
    selected: Vec<usize>,
    /// 最近一次点击的行（shift 范围选择的锚点）。
    last_click: Option<usize>,
    /// 拖拽进行中。
    drag: Option<DragState>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DragState {
    /// 被拖拽的行（原始索引，已排序，可能多个）。
    indices: Vec<usize>,
    /// 插入位置：删除被拖行后，按剩余行计数（0..=visible）。
    insert_idx: usize,
}

/// 可复用音色列表：多选 + 拖拽排序 + 启用复选框 + 右键菜单。
///
/// 返回 `true` 表示列表被修改（排序/切换/删除），调用方据此重载音频。
pub fn sf_list(ui: &mut egui::Ui, entries: &mut Vec<SfEntry>, salt: &str) -> bool {
    let state_id = ui.id().with(("sf_list", salt));
    let mut state: ListState = ui
        .data_mut(|d| d.get_persisted(state_id))
        .unwrap_or_default();

    let mut changed = false;
    let mut remove_idx: Option<usize> = None;
    let mut item_rects: Vec<egui::Rect> = Vec::new();
    let mut auto_scroll = 0.0;

    let scroll_id = ui.id().with(("sf_scroll", salt));
    egui::ScrollArea::vertical()
        .id_salt(scroll_id)
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let dragging = state.drag.is_some();

            let total = entries.len();
            // 行位置显式计算，不依赖光标推进（checkbox 的 ui.put 会推进光标，
            // 与行高无关）；循环结束后再把光标推进到列表末尾，保证 ScrollArea
            // 内容高度正确。
            let start_y = ui.available_rect_before_wrap().min.y;
            let mut row_y = start_y;
            let mut last_row_rect: Option<egui::Rect> = None;

            for i in 0..total {
                let row_rect = egui::Rect::from_min_size(
                    egui::pos2(ui.available_rect_before_wrap().min.x, row_y),
                    egui::vec2(ui.available_width(), ROW_H),
                );
                row_y += ROW_H;
                last_row_rect = Some(row_rect);
                item_rects.push(row_rect);

                let is_selected = state.selected.contains(&i);

                // ── 行背景：选中（含拖拽中，被拖行必在选中集合内）与音轨面板同款
                // ROW_SELECTED_BG；hover 同音轨面板的白色 3% 提亮 ──
                if is_selected {
                    ui.painter()
                        .rect_filled(row_rect, 0.0, crate::theme::ROW_SELECTED_BG);
                } else if ui.rect_contains_pointer(row_rect) {
                    ui.painter().rect_filled(
                        row_rect,
                        0.0,
                        egui::Color32::WHITE.gamma_multiply(0.03),
                    );
                }

                let row_id = ui.id().with(("sf_row", i));
                let resp = ui.interact(row_rect, row_id, egui::Sense::click_and_drag());

                // ── 行内容：复选框 + 名称 + 路径 ──
                let cb_rect = egui::Rect::from_min_max(
                    egui::pos2(row_rect.min.x + 4.0, row_rect.center().y - 9.0),
                    egui::pos2(row_rect.min.x + 22.0, row_rect.center().y + 9.0),
                );
                // 每行的 ui.id() 相同，push_id 保证 checkbox 的自动 id 唯一。
                let cb_changed = ui
                    .push_id(("sf_cb", i), |ui| {
                        ui.put(cb_rect, egui::Checkbox::new(&mut entries[i].enabled, ""))
                    })
                    .inner
                    .changed();
                if cb_changed {
                    changed = true;
                }

                let text_x = row_rect.min.x + 22.0;
                ui.painter().text(
                    egui::pos2(text_x, row_rect.min.y + 10.0),
                    egui::Align2::LEFT_CENTER,
                    &entries[i].name,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
                ui.painter().text(
                    egui::pos2(text_x, row_rect.min.y + 28.0),
                    egui::Align2::LEFT_CENTER,
                    truncate_path(&entries[i].path),
                    egui::FontId::proportional(10.0),
                    crate::theme::TEXT_DIM,
                );

                // ── 右侧拖拽手柄（Material 图标）──
                ui.painter().text(
                    egui::pos2(row_rect.max.x - 10.0, row_rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    ICON_DRAG_INDICATOR.codepoint,
                    egui::FontId::new(14.0, ICON_DRAG_INDICATOR.font_family()),
                    crate::theme::TEXT_LABEL,
                );

                // ── 点击选择（拖拽中不响应）──
                if resp.clicked() && !dragging {
                    handle_click(&mut state, i, ui);
                }

                // ── 拖拽开始：未选中的行先单选，然后拖起整个选中集合 ──
                if resp.drag_started() && !dragging {
                    if !is_selected {
                        state.selected.clear();
                        state.selected.push(i);
                        state.last_click = Some(i);
                    }
                    state.selected.sort();
                    state.drag = Some(DragState {
                        indices: state.selected.clone(),
                        insert_idx: i,
                    });
                }

                // ── 右键菜单 ──
                let mut action: Option<SfAction> = None;
                resp.context_menu(|ui| {
                    ui.set_min_width(100.0);
                    if i > 0 && ui.button(t!("sf_list.move_up").as_ref()).clicked() {
                        action = Some(SfAction::MoveUp);
                        ui.close();
                    }
                    if i + 1 < total && ui.button(t!("sf_list.move_down").as_ref()).clicked() {
                        action = Some(SfAction::MoveDown);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(t!("sf_list.delete").as_ref()).clicked() {
                        action = Some(SfAction::Remove);
                        ui.close();
                    }
                });
                match action {
                    Some(SfAction::MoveUp) => {
                        entries.swap(i, i - 1);
                        swap_selection(&mut state, i, i - 1);
                        changed = true;
                    }
                    Some(SfAction::MoveDown) => {
                        entries.swap(i, i + 1);
                        swap_selection(&mut state, i, i + 1);
                        changed = true;
                    }
                    Some(SfAction::Remove) => remove_idx = Some(i),
                    None => {}
                }
            }

            // 光标推进到列表末尾（含间距），ScrollArea 据此计算内容高度
            if let Some(r) = last_row_rect {
                ui.advance_cursor_after_rect(r);
            }

            // ── 拖拽中：插入线 + ghost + 自动滚动 + 释放排序 ──
            if let Some(drag) = &mut state.drag {
                let pointer = ui.ctx().input(|i| i.pointer.interact_pos());

                if let Some(p) = pointer {
                    // 插入位置：指针越过剩余行的中线则后移一位
                    let mut insert = 0usize;
                    for (i, rect) in item_rects.iter().enumerate() {
                        if drag.indices.contains(&i) {
                            continue;
                        }
                        if p.y < rect.center().y {
                            break;
                        }
                        insert += 1;
                    }
                    drag.insert_idx = insert;

                    // 自动滚动：指针贴近可视区边缘
                    if p.y < viewport.top() + AUTO_SCROLL_MARGIN {
                        auto_scroll = -AUTO_SCROLL_SPEED;
                    } else if p.y > viewport.bottom() - AUTO_SCROLL_MARGIN {
                        auto_scroll = AUTO_SCROLL_SPEED;
                    }
                }

                // 插入位置线
                if let Some(y) = insert_line_y(&drag.indices, drag.insert_idx, &item_rects) {
                    let x1 = item_rects[0].left() + 4.0;
                    let x2 = item_rects[0].right() - 4.0;
                    ui.painter().line_segment(
                        [egui::pos2(x1, y), egui::pos2(x2, y)],
                        egui::Stroke::new(3.0, crate::theme::ACCENT_ACTIVE),
                    );
                }

                // 释放：应用拖拽排序（保持被拖项相对顺序）
                if ui.input(|i| i.pointer.any_released()) {
                    apply_drop(entries, &drag.indices, drag.insert_idx);
                    changed = true;
                    state.selected.clear();
                    state.last_click = None;
                    state.drag = None;
                }
            }
        });

    // 自动滚动：直接改 ScrollArea 的持久化状态（不干扰用户滚轮/滚动条）
    if auto_scroll != 0.0 {
        let mut sa =
            egui::containers::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
        sa.offset.y = (sa.offset.y + auto_scroll).max(0.0);
        sa.store(ui.ctx(), scroll_id);
    }

    if let Some(idx) = remove_idx {
        entries.remove(idx);
        // 删除后修正选中索引
        state.selected.retain(|&x| x != idx);
        for x in &mut state.selected {
            if *x > idx {
                *x -= 1;
            }
        }
        changed = true;
    }

    ui.data_mut(|d| d.insert_persisted(state_id, state));
    changed
}

/// 应用拖拽排序：从后往前删被拖行（避免索引错乱），再按插入点恢复，
/// 被拖项保持原有相对顺序。`indices` 必须已排序。
fn apply_drop<T: Clone>(entries: &mut Vec<T>, indices: &[usize], insert_idx: usize) {
    let mut dragged: Vec<T> = Vec::with_capacity(indices.len());
    for &idx in indices {
        if idx < entries.len() {
            dragged.push(entries[idx].clone());
        }
    }
    for &idx in indices.iter().rev() {
        if idx < entries.len() {
            entries.remove(idx);
        }
    }
    let insert_at = insert_idx.min(entries.len());
    for (k, item) in dragged.into_iter().enumerate() {
        entries.insert(insert_at + k, item);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SfAction {
    Remove,
    MoveUp,
    MoveDown,
}

/// 点击选择：无修饰键单选；cmd/ctrl 切换；shift 范围选择（以 last_click 为锚点）。
fn handle_click(state: &mut ListState, i: usize, ui: &egui::Ui) {
    let mods = ui.input(|i| i.modifiers);
    if mods.command || mods.ctrl {
        if let Some(pos) = state.selected.iter().position(|&x| x == i) {
            state.selected.remove(pos);
        } else {
            state.selected.push(i);
        }
    } else if mods.shift {
        let anchor = state.last_click.unwrap_or(i);
        let (lo, hi) = (anchor.min(i), anchor.max(i));
        for x in lo..=hi {
            if !state.selected.contains(&x) {
                state.selected.push(x);
            }
        }
        state.selected.sort();
    } else {
        state.selected.clear();
        state.selected.push(i);
    }
    state.last_click = Some(i);
}

/// 右键上移/下移后，选中索引与行一起交换。
fn swap_selection(state: &mut ListState, a: usize, b: usize) {
    for x in &mut state.selected {
        if *x == a {
            *x = b;
        } else if *x == b {
            *x = a;
        }
    }
    state.last_click = match state.last_click {
        Some(x) if x == a => Some(b),
        Some(x) if x == b => Some(a),
        other => other,
    };
}

/// 插入线 y 坐标：删除被拖行后，第 `insert_idx` 个剩余行的顶部；
/// 插到末尾时为最后一个可见行的底部。
fn insert_line_y(
    drag_indices: &[usize],
    insert_idx: usize,
    item_rects: &[egui::Rect],
) -> Option<f32> {
    let mut visible = 0usize;
    for (i, rect) in item_rects.iter().enumerate() {
        if drag_indices.contains(&i) {
            continue;
        }
        if visible == insert_idx {
            return Some(rect.top());
        }
        visible += 1;
    }
    item_rects.last().map(|r| r.bottom())
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
    use super::{apply_drop, truncate_path};

    /// 拖拽排序：单行拖到中间。
    #[test]
    fn drop_single_row_to_middle() {
        let mut v = vec!["a", "b", "c", "d", "e"];
        // 拖 'a'（index 0）到 index 3 的位置（'c' 和 'd' 之间）
        apply_drop(&mut v, &[0], 2);
        assert_eq!(v, vec!["b", "c", "a", "d", "e"]);
    }

    /// 拖拽排序：多行一起拖到开头。
    #[test]
    fn drop_multiple_rows_to_front() {
        let mut v = vec!["a", "b", "c", "d", "e"];
        // 拖 'c'、'd' 到最前面
        apply_drop(&mut v, &[2, 3], 0);
        assert_eq!(v, vec!["c", "d", "a", "b", "e"]);
    }

    /// 拖拽排序：多行拖到末尾，保持相对顺序。
    #[test]
    fn drop_multiple_rows_to_end() {
        let mut v = vec!["a", "b", "c", "d", "e"];
        // 拖 'a'、'b' 到末尾
        apply_drop(&mut v, &[0, 1], 3);
        assert_eq!(v, vec!["c", "d", "e", "a", "b"]);
    }

    /// 拖拽排序：插入索引越界时 clamp 到末尾，不 panic。
    #[test]
    fn drop_insert_idx_out_of_bounds_is_clamped() {
        let mut v = vec!["a", "b", "c"];
        apply_drop(&mut v, &[0], 99);
        assert_eq!(v, vec!["b", "c", "a"]);
    }

    /// 拖拽排序：不连续多选（cmd 点选），保持选中顺序。
    #[test]
    fn drop_non_contiguous_selection() {
        let mut v = vec!["a", "b", "c", "d", "e", "f"];
        // 拖 'b'、'e' 到 'c' 的位置
        apply_drop(&mut v, &[1, 4], 1);
        assert_eq!(v, vec!["a", "b", "e", "c", "d", "f"]);
    }

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
