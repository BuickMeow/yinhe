//! 刷子工具：按住拖动，指针经过的每个量化格生成一个音符（松手一次提交）。
//!
//! 与铅笔一致：需要有效写入轨；力度用该轨记忆力度（App 层填充）；
//! gate = 一个量化间隔。拖拽中只输出 ghost，release 发 `AddNotes` 事件。

use std::collections::HashSet;

use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::PianoRollView;

use super::types::PianoViewEvent;

/// 刷子一次落笔的累积（持久化到 egui memory）。
#[derive(Clone, Default)]
struct BrushStroke {
    /// 上一帧所在的格 `(tick 格索引, key)`，用于路径填充。
    last: Option<(i64, u8)>,
    /// 本次落笔已生成的格 `(key, start_tick)`（插入顺序）。
    cells: Vec<(u8, u32)>,
    seen: HashSet<(u8, u32)>,
}

impl BrushStroke {
    fn add(&mut self, key: u8, start: u32) {
        if self.seen.insert((key, start)) {
            self.cells.push((key, start));
        }
    }
}

/// 把一个量化格转成音符 start（格索引 × interval，防溢出）。
fn cell_start(idx: i64, interval: u32) -> u32 {
    ((idx.max(0) as u64) * interval as u64).min(u32::MAX as u64) as u32
}

/// 画线补格（Bresenham）：把 from→to 经过的所有格逐个交给 `f`。
fn for_each_cell(from: (i64, u8), to: (i64, u8), mut f: impl FnMut(i64, u8)) {
    let (x0, y0) = (from.0, from.1 as i64);
    let (x1, y1) = (to.0, to.1 as i64);
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        f(x, y as u8);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// 刷子帧处理：返回 `(ghost 预览, release 提交事件)`。
#[allow(clippy::too_many_arguments)]
pub(crate) fn brush_frame(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    music_rect: egui::Rect,
    view: &mut PianoRollView,
    write_track: Option<u16>,
    track_visible: &[bool],
    conductor_idx: Option<u16>,
    quantize: QuantizePreset,
    ppq: u32,
    total_ticks: f64,
) -> (Vec<super::drag::GhostNote>, Option<PianoViewEvent>) {
    let state_id = ui.id().with("brush_stroke");
    let mut stroke: Option<BrushStroke> =
        ui.data_mut(|d| d.get_persisted(state_id)).unwrap_or(None);

    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        return (Vec::new(), None);
    }
    let pointer = ui.input(|i| i.pointer.clone());

    if stroke.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        ui.data_mut(|d| d.insert_persisted(state_id, Option::<BrushStroke>::None));
        stroke = None;
    }

    let interval = quantize.tick_interval(ppq);
    if interval == 0 {
        return (Vec::new(), None);
    }
    let track = super::pencil::valid_pencil_track(write_track, track_visible, conductor_idx);

    // 屏幕位置 → 量化格 `(tick 格索引, key)`。
    let cell_at = |view: &PianoRollView, pos: egui::Pos2| -> (i64, u8) {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let (main_px, cross_px) = super::drag::main_cross_x_y(view, (local.x, local.y));
        let raw_tick = super::drag::main_px_to_tick_dir(view, main_px);
        let idx = (raw_tick / interval as f64).round().max(0.0) as i64;
        (idx, view.cross_px_to_key(cross_px))
    };

    // Press：无有效写入轨时不动。
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && music_rect.contains(pos)
        && track.is_some()
    {
        let (idx, key) = cell_at(view, pos);
        let mut s = BrushStroke::default();
        s.add(key, cell_start(idx, interval));
        s.last = Some((idx, key));
        stroke = Some(s);
    }

    // 拖拽：Bresenham 路径填充（快速拖动不漏格）+ auto-scroll。
    if let Some(s) = stroke.as_mut()
        && pointer.primary_down()
        && !pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
    {
        super::drag::drag_scroll_and_clamp(ui, view, content_rect, music_rect, total_ticks, pos);
        let clamped = pos.clamp(music_rect.min, music_rect.max);
        let (idx, key) = cell_at(view, clamped);
        if let Some(last) = s.last {
            for_each_cell(last, (idx, key), |i, k| s.add(k, cell_start(i, interval)));
        } else {
            s.add(key, cell_start(idx, interval));
        }
        s.last = Some((idx, key));
    }

    // Release：一次性提交（力度由 App 层填充为该轨记忆力度）。
    if pointer.primary_released()
        && let Some(s) = stroke.take()
    {
        ui.data_mut(|d| d.insert_persisted(state_id, Option::<BrushStroke>::None));
        if let Some(track) = track
            && !s.cells.is_empty()
        {
            let notes = s
                .cells
                .into_iter()
                .map(|(key, start)| yinhe_core::NoteEvent {
                    id: 0,
                    start_tick: start,
                    end_tick: start.saturating_add(interval),
                    key,
                    velocity: 100,
                })
                .collect();
            return (Vec::new(), Some(PianoViewEvent::AddNotes { track, notes }));
        }
        return (Vec::new(), None);
    }

    // 拖拽预览：已累积的格。
    let ghosts = stroke
        .as_ref()
        .map(|s| {
            let track = track.unwrap_or(0);
            s.cells
                .iter()
                .map(|&(key, start)| (start, start.saturating_add(interval), key, track))
                .collect()
        })
        .unwrap_or_default();
    // 持久化本帧状态（下一帧继续累积）。
    ui.data_mut(|d| d.insert_persisted(state_id, stroke));
    (ghosts, None)
}
