//! AR 选区批量操作：音符与同一选框内的自动化事件一起处理。
//!
//! AR 把自动化曲线折叠在轨道行里，视觉上已经被选框覆盖；删除、时值
//! ×2/÷2、水平翻转和复制都应该同时作用于这些锚点，否则用户会看到
//! 曲线没有跟着动。

use std::collections::HashMap;

use yinhe_types::automation::{AutomationEdit, AutomationEvent, AutomationTarget};

use crate::FlipAxis;
use crate::document::Document;
use crate::history::UndoAction;

type LaneMoves = (AutomationTarget, Vec<(u32, u32, f32)>);

struct SelectedAuto {
    track_idx: u16,
    lane_idx: usize,
    target: AutomationTarget,
    event: AutomationEvent,
}

impl Document {
    /// 当前选框覆盖的自动化事件（矩形态按 rect，成员态按 id）。
    ///
    /// 与桌面 AR 框选一致：事件只按 ``selection`` 判定，不再额外按当前
    /// 轨道显隐过滤。
    fn selected_automation_events(&self) -> Vec<SelectedAuto> {
        // 只遍历选框覆盖到的轨道，避免单轨选择时扫描全工程自动化。
        let mut track_indices = std::collections::HashSet::new();
        for &(_ts, _te, _kl, _kh, tl, th) in &self.edit.selected.rects {
            for t in tl..=th {
                track_indices.insert(t as usize);
            }
        }
        let mut out = Vec::new();
        for ti in track_indices {
            let Some(track) = self.data.model.tracks.get(ti) else {
                continue;
            };
            for (li, lane) in track.automation_lanes.iter().enumerate() {
                for ev in &lane.events {
                    if self
                        .edit
                        .selected
                        .accepts_automation_event(ti as u16, &lane.target, ev)
                    {
                        out.push(SelectedAuto {
                            track_idx: ti as u16,
                            lane_idx: li,
                            target: lane.target.clone(),
                            event: *ev,
                        });
                    }
                }
            }
        }
        out
    }

    /// 音符选区的 tick 范围（与 `scale_selected_span` / 水平翻转同源）。
    fn selection_tick_bounds(&self) -> Option<(u32, u32)> {
        let mut t0 = u32::MAX;
        let mut t1 = 0u32;
        for &(ts, te, _, _, _, _) in &self.edit.selected.rects {
            t0 = t0.min(ts);
            t1 = t1.max(te);
        }
        (t0 != u32::MAX && t1 > t0).then_some((t0, t1))
    }

    fn combine_undo(note: Option<UndoAction>, auto: Vec<UndoAction>) -> Option<UndoAction> {
        let mut actions = Vec::new();
        if let Some(note) = note {
            actions.push(note);
        }
        actions.extend(auto);
        match actions.len() {
            0 => None,
            1 => actions.into_iter().next(),
            _ => Some(UndoAction::Composite(actions)),
        }
    }

    /// 按 lane 合并自动化移动，生成每 lane 一个 `MoveBatch`（一次 undo 快照）。
    fn automation_move_edits(
        events: &[SelectedAuto],
        map_tick: impl Fn(u32, f32) -> Option<(u32, f32)>,
    ) -> Vec<AutomationEdit> {
        let mut by_lane: HashMap<(u16, usize), LaneMoves> = HashMap::new();
        for e in events {
            let Some((new_tick, new_value)) = map_tick(e.event.tick, e.event.value) else {
                continue;
            };
            by_lane
                .entry((e.track_idx, e.lane_idx))
                .or_insert_with(|| (e.target.clone(), Vec::new()))
                .1
                .push((e.event.tick, new_tick, new_value));
        }
        by_lane
            .into_iter()
            .filter(|(_, (_, moves))| !moves.is_empty())
            .map(
                |((track_idx, lane_idx), (target, moves))| AutomationEdit::MoveBatch {
                    track_idx,
                    lane_idx,
                    target,
                    moves,
                },
            )
            .collect()
    }

    /// 删除选框内音符 + 自动化事件（一次 undo）。
    pub fn delete_selected_with_automation(&mut self) -> Option<UndoAction> {
        let edits: Vec<AutomationEdit> = self
            .selected_automation_events()
            .into_iter()
            .map(|e| AutomationEdit::Delete {
                track_idx: e.track_idx,
                lane_idx: e.lane_idx,
                target: e.target,
                tick: e.event.tick,
            })
            .collect();
        let note = self.delete_selected();
        let auto = self.apply_automation_edits(edits);
        Self::combine_undo(note, auto)
    }

    /// 时值 ×`num`/`den`：音符和自动化事件都相对选框起点缩放。
    pub fn scale_selected_span_with_automation(
        &mut self,
        num: u64,
        den: u64,
    ) -> Option<UndoAction> {
        if den == 0 {
            return None;
        }
        let (t0, t1) = self.selection_tick_bounds()?;
        let span = u64::from(t1 - t0);
        let new_span = span.saturating_mul(num) / den;
        if new_span == 0 || new_span == span {
            return None;
        }
        let factor = new_span as f64 / span as f64;
        let edits =
            Self::automation_move_edits(&self.selected_automation_events(), |tick, value| {
                let nt = (f64::from(t0) + (f64::from(tick) - f64::from(t0)) * factor)
                    .round()
                    .clamp(0.0, f64::from(u32::MAX)) as u32;
                (nt != tick).then_some((nt, value))
            });
        let note = self.scale_selected_span(num, den);
        if note.is_none() {
            // 选框里只有自动化、没有音符时，scale_selected_span 不会动选框；
            // 这里补一次 tick 缩放，保持选框与结果一致。
            self.edit.scale_sel_ticks(u64::from(t0), factor);
        }
        let auto = self.apply_automation_edits(edits);
        Self::combine_undo(note, auto)
    }

    /// 水平翻转：音符和自动化事件都按选框 tick 范围镜像。
    /// 垂直翻转只对音符有意义，自动化保持原位。
    pub fn flip_selected_with_automation(&mut self, axis: FlipAxis) -> Option<UndoAction> {
        let bounds = self.selection_tick_bounds();
        let events = if axis == FlipAxis::Horizontal {
            self.selected_automation_events()
        } else {
            Vec::new()
        };
        let note = self.flip_selected_notes(axis);
        let Some((t0, t1)) = bounds else {
            return note;
        };
        let edits = Self::automation_move_edits(&events, |tick, value| {
            let nt = (i64::from(t0) + i64::from(t1) - i64::from(tick)).clamp(0, i64::from(u32::MAX))
                as u32;
            (nt != tick).then_some((nt, value))
        });
        let auto = self.apply_automation_edits(edits);
        Self::combine_undo(note, auto)
    }

    /// 复制选框内音符 + 自动化事件，偏移一个选区宽度（一次 undo）。
    ///
    /// 与桌面 AR 复制一致：偏移取选框 tick 跨度；单 tick 选框回退到 AR 量化间隔。
    pub fn duplicate_selected_with_automation(&mut self) -> Option<UndoAction> {
        let delta = match self.selection_tick_bounds() {
            Some((t0, t1)) => (t1 - t0).max(1) as i64,
            None => {
                let ppq = self.data.model.meta.ppq;
                let (num, den) = self.data.model.tempo_map.time_sig_default;
                self.edit
                    .quantize_arrange
                    .resolved_with_time_sig(
                        self.edit.arrange_view.base.pixels_per_tick,
                        ppq,
                        num,
                        den,
                        false,
                    )
                    .tick_interval(ppq)
                    .max(1) as i64
            }
        };
        let action = self.duplicate_selected_arrange(delta, 0);
        if action.is_some() {
            // selected.rects 已随副本平移；这里补视图选框，避免 AR 框留在原处。
            self.edit.offset_view_rects_ticks(delta);
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yinhe_core::{ConductorData, NoteEvent, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    use crate::FlipAxis;
    use crate::document::Document;

    fn make_doc() -> Document {
        let model = YinModel {
            conductor: Arc::new(ConductorData::default()),
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        Document {
            data: crate::project_data::ProjectData::new(
                Arc::new(model),
                Default::default(),
                Default::default(),
            ),
            edit: crate::edit_state::EditState {
                track_visible: vec![true, true],
                track_pianoroll_visible: vec![true, true],
                ..Default::default()
            },
            history: crate::history::UndoStack::new(),
            file_name: "test".into(),
            file_path: None,
            mixer: Default::default(),
            mixer_dirty: false,
            doc_id: 0,
            density: Default::default(),
            density_pr: Default::default(),
        }
    }

    fn add_note(doc: &mut Document, start: u32, end: u32, key: u8) {
        doc.add_note(
            1,
            NoteEvent {
                id: 0,
                start_tick: start,
                end_tick: end,
                key,
                velocity: 100,
            },
        );
    }

    fn add_cc_lane(doc: &mut Document, ticks: &[u32]) {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[1]);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 1,
            events: ticks
                .iter()
                .map(|&tick| AutomationEvent {
                    id: 0,
                    tick,
                    value: tick as f32,
                    shape: SegmentShape::Step,
                })
                .collect(),
        });
    }

    fn cc_ticks(doc: &Document) -> Vec<u32> {
        doc.data.model.tracks[1].automation_lanes[0]
            .events
            .iter()
            .map(|e| e.tick)
            .collect()
    }

    #[test]
    fn delete_selection_removes_notes_and_automation_in_one_undo() {
        let mut doc = make_doc();
        add_note(&mut doc, 0, 480, 60);
        add_cc_lane(&mut doc, &[0, 240]);
        doc.edit
            .selected
            .add_rect_track(0, 480, 0, yinhe_types::MAX_KEY, 1, 1);

        let action = doc
            .delete_selected_with_automation()
            .expect("应删除音符与自动化");
        assert!(doc.data.model.notes[60].is_empty());
        assert!(cc_ticks(&doc).is_empty());
        assert!(
            matches!(action, crate::history::UndoAction::Composite(_)),
            "音符 + 自动化应合并为一次 undo"
        );
    }

    #[test]
    fn scale_span_scales_note_and_automation_ticks() {
        let mut doc = make_doc();
        add_note(&mut doc, 0, 480, 60);
        add_cc_lane(&mut doc, &[0, 240]);
        doc.edit
            .selected
            .add_rect_track(0, 480, 0, yinhe_types::MAX_KEY, 1, 1);

        doc.scale_selected_span_with_automation(2, 1)
            .expect("应缩放");
        let note = doc.data.model.notes[60]
            .range(0, 1000)
            .next()
            .expect("音符仍在");
        assert_eq!((note.start_tick, note.end_tick), (0, 960));
        assert_eq!(cc_ticks(&doc), vec![0, 480]);
        assert_eq!(doc.edit.selected.rects[0].0, 0);
        assert_eq!(doc.edit.selected.rects[0].1, 960);
    }

    #[test]
    fn horizontal_flip_mirrors_automation_within_selection() {
        let mut doc = make_doc();
        add_note(&mut doc, 100, 200, 60);
        add_cc_lane(&mut doc, &[100, 300]);
        doc.edit
            .selected
            .add_rect_track(0, 400, 0, yinhe_types::MAX_KEY, 1, 1);

        doc.flip_selected_with_automation(FlipAxis::Horizontal)
            .expect("应翻转");
        let events: Vec<_> = doc.data.model.tracks[1].automation_lanes[0]
            .events
            .iter()
            .map(|e| (e.tick, e.value))
            .collect();
        assert!(events.contains(&(100, 300.0)), "原 300 的锚点应移到 100");
        assert!(events.contains(&(300, 100.0)), "原 100 的锚点应移到 300");
    }

    #[test]
    fn duplicate_selection_copies_automation_and_moves_ar_rect() {
        let mut doc = make_doc();
        add_note(&mut doc, 0, 480, 60);
        add_cc_lane(&mut doc, &[0, 240]);
        doc.edit
            .selected
            .add_rect_track(0, 480, 0, yinhe_types::MAX_KEY, 1, 1);
        doc.edit.arr_sel_rect = vec![(0.0, 480.0, 1, 1)];

        doc.duplicate_selected_with_automation().expect("应复制");
        assert_eq!(cc_ticks(&doc), vec![0, 240, 480, 720]);
        assert_eq!(doc.edit.arr_sel_rect[0].0, 480.0);
        assert_eq!(doc.edit.arr_sel_rect[0].1, 960.0);
    }
}
