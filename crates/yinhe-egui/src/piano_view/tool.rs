//! 选择/铅笔临时切换逻辑（Alt 键）。

use std::collections::HashSet;

use eframe::egui;
use yinhe_types::NoteSource;
use yinhe_types::PianoRollView;

use crate::widgets::tools_panel::Tool;

/// 按住 Alt（Option）时的有效工具（Cubase 风格临时切换）：
/// - Select/SelectVertical + Alt：悬停在音符或选框上 = 保持选择（Alt 拖拽复制）；
///   悬停空白 = 临时铅笔（画音符）。
///   例外：选框拖拽状态机进行中（含 Alt 克隆）时锁定选择工具——
///   拖拽中鼠标移出音符原位后 hover 命中会失败，不得据此切成铅笔。
/// - Pencil + Alt = 临时选择（框选/移动）。
/// - 其余工具不受影响。
///
/// 自动化面板不使用该映射（Alt 在那里是"复制锚点"）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn effective_tool(
    ui: &egui::Ui,
    active: Tool,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    track_visible: &[bool],
    track_selected: &HashSet<u16>,
    sel_rect: &yinhe_editor_core::edit_state::SelRectState,
    write_track: Option<u16>,
    conductor_idx: Option<u16>,
) -> Tool {
    if !ui.input(|i| i.modifiers.alt) {
        return active;
    }
    match active {
        Tool::Pencil => Tool::Select,
        Tool::Select | Tool::SelectVertical => {
            // 拖拽进行中（含 Alt 克隆）→ 锁定选择工具，不得切成铅笔。
            if super::drag::sel_drag_in_progress(ui) {
                return active;
            }
            // 无编辑目标（未选音轨）时：不做 hit-test，直接视为空白→临时铅笔。
            // 避免每帧遍历音符的性能损耗，且此时任何编辑光标都不应出现。
            let can_edit =
                super::pencil::valid_pencil_track(write_track, track_visible, conductor_idx)
                    .is_some();
            if !can_edit {
                return Tool::Pencil;
            }
            // 悬停音符或选框 = 保留选择（Alt 拖拽复制）；空白 = 临时铅笔。
            let hit = ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| {
                if !music_rect.contains(pos) {
                    return false;
                }
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                super::drag::hit_test_note(midi, view, local, track_visible, track_selected)
                    .is_some()
                    || sel_rect.effective_rects().iter().any(|&(t0, t1, k0, k1)| {
                        crate::selection::drag::music_sel_to_pixel_rect(view, t0, t1, k0, k1)
                            .contains(local)
                    })
            });
            if hit { active } else { Tool::Pencil }
        }
        t => t,
    }
}
