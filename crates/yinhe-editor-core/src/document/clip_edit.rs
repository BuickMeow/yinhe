//! MIDI Clip v1 编辑命令：拆分、合并、重命名、缩放、音符归属归一化。
//!
//! Clip 只保存区间元数据，不保存音符；这些命令不改变音符内容，
//! 唯一例外是 `ensure_clips_for_notes` 会扩展/新建 Clip 以覆盖新增音符。

use std::sync::Arc;

use yinhe_core::{Clip, NoteEvent};

use super::Document;
use crate::history::UndoAction;

impl Document {
    fn clip_list(&self, track: usize) -> Option<Vec<Clip>> {
        self.data.model.tracks.get(track).map(|t| t.clips.clone())
    }

    fn commit_clips(
        &mut self,
        track: usize,
        before: Vec<Clip>,
        after: Vec<Clip>,
    ) -> Option<UndoAction> {
        if before == after {
            return None;
        }
        let model = Arc::make_mut(&mut self.data.model);
        let track_data = Arc::make_mut(model.tracks.get_mut(track)?);
        track_data.clips = after.clone();
        self.data.bump_revision();
        Some(UndoAction::Clips {
            track_idx: track,
            before,
            after,
        })
    }

    /// 在 `tick` 处拆分包含它的 Clip。切点必须在 Clip 内部。
    pub fn split_clip_at(&mut self, track: u16, tick: u32) -> Option<UndoAction> {
        let t = track as usize;
        let before = self.clip_list(t)?;
        let idx = before
            .iter()
            .position(|c| c.start_tick < tick && tick < c.end_tick)?;
        let mut after = before.clone();
        let right_id = {
            let model = Arc::make_mut(&mut self.data.model);
            model.alloc_clip_id()
        };
        let mut right = after[idx].clone();
        right.id = right_id;
        right.start_tick = tick;
        after[idx].end_tick = tick;
        after.insert(idx + 1, right);
        debug_assert!(yinhe_core::validate_clips(&after).is_ok());
        self.commit_clips(t, before, after)
    }

    /// 合并指定 Clip 与它的下一个 Clip；保留左侧的 id/name。
    pub fn merge_clip_with_next(&mut self, track: u16, clip_id: u32) -> Option<UndoAction> {
        let t = track as usize;
        let before = self.clip_list(t)?;
        let idx = before.iter().position(|c| c.id == clip_id)?;
        if idx + 1 >= before.len() {
            return None;
        }
        let mut after = before.clone();
        after[idx].end_tick = after[idx].end_tick.max(after[idx + 1].end_tick);
        after.remove(idx + 1);
        debug_assert!(yinhe_core::validate_clips(&after).is_ok());
        self.commit_clips(t, before, after)
    }

    /// 重命名 Clip。
    pub fn rename_clip(&mut self, track: u16, clip_id: u32, name: String) -> Option<UndoAction> {
        let t = track as usize;
        let before = self.clip_list(t)?;
        let idx = before.iter().position(|c| c.id == clip_id)?;
        if before[idx].name == name {
            return None;
        }
        let mut after = before.clone();
        after[idx].name = name;
        self.commit_clips(t, before, after)
    }

    /// 调整 Clip 边界。非空 Clip 不允许切过已有音符的 start。
    ///
    /// 返回 `None` 表示没有变化或参数非法。
    pub fn resize_clip(
        &mut self,
        track: u16,
        clip_id: u32,
        new_start: u32,
        new_end: u32,
    ) -> Option<UndoAction> {
        let t = track as usize;
        let before = self.clip_list(t)?;
        let idx = before.iter().position(|c| c.id == clip_id)?;
        let clip = &before[idx];
        let prev_end = idx.checked_sub(1).map_or(0, |i| before[i].end_tick);
        let next_start = before.get(idx + 1).map_or(u32::MAX, |c| c.start_tick);

        let (min_start, max_start) = self.note_start_bounds(t, clip.start_tick, clip.end_tick);
        let mut start = new_start.max(prev_end);
        let mut end = new_end.min(next_start);
        if let Some(min_start) = min_start {
            start = start.min(min_start);
        }
        if let Some(max_start) = max_start {
            end = end.max(max_start.saturating_add(1));
        }
        if start >= end || (start == clip.start_tick && end == clip.end_tick) {
            return None;
        }
        if clip.start_tick == start && clip.end_tick == end {
            return None;
        }
        let mut after = before.clone();
        after[idx].start_tick = start;
        after[idx].end_tick = end;
        debug_assert!(yinhe_core::validate_clips(&after).is_ok());
        self.commit_clips(t, before, after)
    }

    fn note_start_bounds(&self, track: usize, lo: u32, hi: u32) -> (Option<u32>, Option<u32>) {
        let mut min_start = None;
        let mut max_start = None;
        for bucket in self.data.model.notes.iter() {
            for n in bucket.range(lo, hi) {
                if n.track as usize != track {
                    continue;
                }
                min_start = Some(min_start.map_or(n.start_tick, |m: u32| m.min(n.start_tick)));
                max_start = Some(max_start.map_or(n.start_tick, |m: u32| m.max(n.start_tick)));
            }
        }
        (min_start, max_start)
    }

    /// 把新增/移动后的音符纳入 Clip 归属。
    ///
    /// 规则：落在 Clip 内就归它；否则扩展左侧 Clip；没有左侧 Clip 就新建；
    /// 扩展后与右侧重叠就合并。返回 Clip 变化（若有）。
    pub fn ensure_clips_for_notes(
        &mut self,
        track: u16,
        notes: &[NoteEvent],
    ) -> Option<UndoAction> {
        if notes.is_empty() {
            return None;
        }
        let t = track as usize;
        let before = self.clip_list(t)?;
        let mut clips = before.clone();
        let mut next_id = self.data.model.next_clip_id.max(1);
        let mut notes: Vec<&NoteEvent> = notes.iter().collect();
        notes.sort_by_key(|n| n.start_tick);

        for note in notes {
            let start = note.start_tick;
            let end = note.end_tick.max(start.saturating_add(1));
            let containing = clips
                .iter()
                .position(|c| c.start_tick <= start && start < c.end_tick);
            let idx = if let Some(i) = containing {
                if end > clips[i].end_tick {
                    clips[i].end_tick = end;
                }
                i
            } else {
                let left = clips.iter().rposition(|c| c.start_tick <= start);
                if let Some(i) = left {
                    clips[i].end_tick = clips[i].end_tick.max(end);
                    i
                } else {
                    let clip = Clip::new(next_id, String::new(), start, end);
                    next_id = next_id.wrapping_add(1);
                    let pos = clips.partition_point(|c| c.start_tick < start);
                    clips.insert(pos, clip);
                    pos
                }
            };
            // 扩展/新建后与右侧重叠：合并（保留左侧 id/name）。
            while idx + 1 < clips.len() && clips[idx].end_tick > clips[idx + 1].start_tick {
                let right = clips.remove(idx + 1);
                clips[idx].end_tick = clips[idx].end_tick.max(right.end_tick);
            }
        }

        {
            let model = Arc::make_mut(&mut self.data.model);
            if next_id != model.next_clip_id {
                model.next_clip_id = next_id;
            }
        }
        debug_assert!(yinhe_core::validate_clips(&clips).is_ok());
        self.commit_clips(t, before, clips)
    }
    /// 在 AR 空白处新建一个空 Clip（默认 1 小节）。
    pub fn create_empty_clip(
        &mut self,
        track: u16,
        start_tick: u32,
        len_ticks: u32,
    ) -> Option<UndoAction> {
        let t = track as usize;
        let before = self.clip_list(t)?;
        let end_tick = start_tick.saturating_add(len_ticks.max(1));
        if before
            .iter()
            .any(|c| c.start_tick < end_tick && c.end_tick > start_tick)
        {
            return None;
        }
        let id = {
            let model = Arc::make_mut(&mut self.data.model);
            model.alloc_clip_id()
        };
        let mut after = before.clone();
        let pos = after.partition_point(|c| c.start_tick < start_tick);
        after.insert(pos, Clip::new(id, String::new(), start_tick, end_tick));
        self.commit_clips(t, before, after)
    }

    /// 把当前选区的音符纳入 Clip 归属（拖动/缩放/复制后调用）。
    pub fn ensure_clips_for_selection(&mut self) -> Option<UndoAction> {
        use std::collections::HashMap;
        let mut by_track: HashMap<u16, Vec<NoteEvent>> = HashMap::new();
        crate::batch_ops::for_each_selected(&self.data.model, &self.edit.selected, |n, key| {
            by_track.entry(n.track).or_default().push(NoteEvent {
                id: n.id,
                start_tick: n.start_tick,
                end_tick: n.end_tick,
                key,
                velocity: n.velocity,
            });
        });
        let mut actions = Vec::new();
        for (track, notes) in by_track {
            if let Some(action) = self.ensure_clips_for_notes(track, &notes) {
                actions.push(action);
            }
        }
        match actions.len() {
            0 => None,
            1 => actions.pop(),
            _ => Some(UndoAction::Composite(actions)),
        }
    }

    /// 把音符编辑的 undo 与 Clip 扩展合并为一个 Composite。
    pub fn with_clip_normalization(&mut self, action: UndoAction) -> UndoAction {
        match self.ensure_clips_for_selection() {
            Some(clip) => UndoAction::Composite(vec![clip, action]),
            None => action,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yinhe_core::{ConductorData, NoteEvent, TrackData, YinModel};

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

    fn add_note(doc: &mut Document, track: u16, start: u32, end: u32) {
        doc.add_note(
            track,
            NoteEvent {
                id: 0,
                start_tick: start,
                end_tick: end,
                key: 60,
                velocity: 100,
            },
        );
    }

    #[test]
    fn add_note_extends_left_clip() {
        let mut doc = make_doc();
        add_note(&mut doc, 1, 100, 200);
        add_note(&mut doc, 1, 500, 600);
        let clips = &doc.model().tracks[1].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].start_tick, 100);
        assert_eq!(clips[0].end_tick, 600);
    }

    #[test]
    fn split_keeps_left_id_and_new_right_id() {
        let mut doc = make_doc();
        add_note(&mut doc, 1, 0, 100);
        let clip_id = doc.model().tracks[1].clips[0].id;
        doc.split_clip_at(1, 50).unwrap();
        let clips = &doc.model().tracks[1].clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].id, clip_id);
        assert_eq!(clips[0].start_tick, 0);
        assert_eq!(clips[0].end_tick, 50);
        assert_ne!(clips[1].id, clip_id);
        assert_eq!(clips[1].start_tick, 50);
        assert_eq!(clips[1].end_tick, 100);
    }

    #[test]
    fn merge_keeps_left_name_and_union_range() {
        let mut doc = make_doc();
        add_note(&mut doc, 1, 0, 100);
        add_note(&mut doc, 1, 200, 300);
        let id = doc.model().tracks[1].clips[0].id;
        doc.rename_clip(1, id, "A".into()).unwrap();
        doc.split_clip_at(1, 150).unwrap();
        doc.merge_clip_with_next(1, id).unwrap();
        let clips = &doc.model().tracks[1].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].id, id);
        assert_eq!(clips[0].name, "A");
        assert_eq!(clips[0].start_tick, 0);
        assert_eq!(clips[0].end_tick, 300);
    }

    #[test]
    fn ensure_clips_creates_new_clip_before_first() {
        let mut doc = make_doc();
        add_note(&mut doc, 1, 1000, 1100);
        let before = doc.model().tracks[1].clips.len();
        doc.add_note(
            1,
            NoteEvent {
                id: 0,
                start_tick: 10,
                end_tick: 20,
                key: 61,
                velocity: 100,
            },
        );
        let clips = &doc.model().tracks[1].clips;
        assert_eq!(clips.len(), before + 1);
        assert_eq!(clips[0].start_tick, 10);
        assert_eq!(clips[0].end_tick, 20);
    }
}
