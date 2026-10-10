//! 右键拖拽批量擦除：从任意位置（含空白）按下，沿指针路径批量删除音符，
//! 整笔 = 一个 undo 条目。
//!
//! 状态存 egui memory（与 `brush_stroke` / `pencil_drag` 同构）；拖拽期间只累积
//! 命中，松手时一次性返回列表，由 App 层用一个 undo 落地。

use std::collections::HashSet;

use eframe::egui;

use yinhe_types::PianoRollView;

/// 一笔擦除的进行态。
#[derive(Clone, Default)]
struct EraseStroke {
    /// 上一帧指针位置（像素），用于沿线段补点。
    last: Option<egui::Pos2>,
    /// 命中的 `(track, start_tick, key)`，插入序。
    hits: Vec<(u16, u32, u8)>,
    /// 去重集合。
    seen: HashSet<(u16, u32, u8)>,
}

/// 路径采样步长（像素）。
const SAMPLE_STEP_PX: f32 = 4.0;

/// 每帧处理右键擦除。松手且本笔有命中时返回命中列表（触发一次 `QuickErase`）。
pub(crate) fn frame(
    ui: &egui::Ui,
    view: &PianoRollView,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_visible: &[bool],
    track_selected: &HashSet<u16>,
) -> Option<Vec<(u16, u32, u8)>> {
    let id = ui.id().with("quick_erase_stroke");
    let mut stroke = ui
        .data_mut(|d| d.get_temp::<EraseStroke>(id))
        .unwrap_or_default();

    let (down, released, pos, over_popup) = ui.input(|i| {
        (
            i.pointer.button_down(egui::PointerButton::Secondary),
            i.pointer.button_released(egui::PointerButton::Secondary),
            i.pointer.interact_pos(),
            crate::view_interaction::pointer_over_popup(ui.ctx()),
        )
    });

    let mut result = None;

    if down {
        if let Some(p) = pos
            && music_rect.contains(p)
            && !over_popup
        {
            let from = stroke.last.unwrap_or(p);
            sample_segment(
                from,
                p,
                view,
                content_rect,
                midi,
                track_visible,
                track_selected,
                &mut stroke,
            );
            stroke.last = Some(p);
        }
    } else if released {
        if !stroke.hits.is_empty() {
            result = Some(std::mem::take(&mut stroke.hits));
        }
        stroke = EraseStroke::default();
    } else if stroke.last.is_some() || !stroke.hits.is_empty() {
        // 异常中断（未收到 released，如指针离开窗口）：丢弃，不删除。
        stroke = EraseStroke::default();
    }

    ui.data_mut(|d| d.insert_temp(id, stroke));
    result
}

/// 沿 `from → to` 线段按固定步长采样命中音符。
#[allow(clippy::too_many_arguments)]
fn sample_segment(
    from: egui::Pos2,
    to: egui::Pos2,
    view: &PianoRollView,
    content_rect: egui::Rect,
    midi: Option<&dyn yinhe_types::NoteSource>,
    track_visible: &[bool],
    track_selected: &HashSet<u16>,
    stroke: &mut EraseStroke,
) {
    let delta = to - from;
    let steps = ((delta.length() / SAMPLE_STEP_PX).ceil() as usize).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = from + delta * t;
        let local = egui::pos2(p.x - content_rect.min.x, p.y - content_rect.min.y);
        if let Some((_, track, start_tick, _, key)) =
            super::drag::hit_test_note(midi, view, local, track_visible, track_selected)
        {
            let note_id = (track, start_tick, key);
            if stroke.seen.insert(note_id) {
                stroke.hits.push(note_id);
            }
        }
    }
}
