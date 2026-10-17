//! 事件列表的通用「全量快照」写入：给「插入菜单」等入口提供
//! before/after 快照式 undo（与 event browser 的插入行为一致）。
//!
//! 单独的插入原语（`insert_*_event`，见 conductor_edit / track_meta_edit）
//! 只改数据、不含 undo；这里在其上包一层：
//! - [`Document::event_list_snapshot`]：按 target 取列表快照（before/after 共用）；
//! - [`Document::event_list_edit_with_undo`]：单列表编辑（插入/修改/删除皆可）；
//! - [`Document::track_events_batch_with_undo`]：多轨批量插入同类型事件，
//!   合并为一个 Composite undo（撤销一次全回退）。

use crate::history::{EventListDelta, EventListItem, EventListTarget, UndoAction};

use super::Document;

impl Document {
    /// 按 target 取对应事件列表的快照（顺序与存储一致）。
    pub fn event_list_snapshot(&self, target: &EventListTarget) -> Vec<EventListItem> {
        let conductor = &self.data.model.conductor;
        match target {
            EventListTarget::TimeSig => conductor
                .time_sig
                .iter()
                .cloned()
                .map(EventListItem::TimeSig)
                .collect(),
            EventListTarget::KeySig => conductor
                .key_sig
                .iter()
                .cloned()
                .map(EventListItem::KeySig)
                .collect(),
            EventListTarget::Marker => conductor
                .markers
                .iter()
                .cloned()
                .map(EventListItem::Marker)
                .collect(),
            EventListTarget::ConductorLyrics => conductor
                .lyrics
                .iter()
                .cloned()
                .map(EventListItem::Lyrics)
                .collect(),
            EventListTarget::ConductorChord => conductor
                .chord
                .iter()
                .cloned()
                .map(EventListItem::Chord)
                .collect(),
            EventListTarget::Lyrics { track } => self
                .data
                .model
                .tracks
                .get(*track as usize)
                .map(|t| {
                    t.lyrics
                        .iter()
                        .cloned()
                        .map(EventListItem::Lyrics)
                        .collect()
                })
                .unwrap_or_default(),
            EventListTarget::Chord { track } => self
                .data
                .model
                .tracks
                .get(*track as usize)
                .map(|t| t.chord.iter().cloned().map(EventListItem::Chord).collect())
                .unwrap_or_default(),
            EventListTarget::ProgramChange { track } => self
                .data
                .model
                .tracks
                .get(*track as usize)
                .map(|t| {
                    t.program_change
                        .iter()
                        .cloned()
                        .map(EventListItem::ProgramChange)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// 单事件列表的「编辑 + 全量快照 undo」。
    /// `edit` 内执行插入/修改；before == after 时不 push undo 并返回 false。
    pub fn event_list_edit_with_undo(
        &mut self,
        target: EventListTarget,
        label: &str,
        edit: impl FnOnce(&mut Self),
    ) -> bool {
        let snapshot = self.capture_snapshot();
        let before = self.event_list_snapshot(&target);
        edit(self);
        let after = self.event_list_snapshot(&target);
        if before == after {
            return false;
        }
        self.push_undo(
            UndoAction::EventList(EventListDelta {
                target,
                old: before,
                new: after,
            }),
            label,
            snapshot,
        );
        true
    }

    /// 多轨批量插入同类型事件：每条轨各执行一次 `insert`，
    /// 合并为 Composite undo（撤销一次全回退）。
    /// `target_of` 给出每条轨对应的列表目标（与 `insert` 写入的列表一致）。
    pub fn track_events_batch_with_undo(
        &mut self,
        tracks: &[u16],
        label: &str,
        mut insert: impl FnMut(&mut Self, u16),
        target_of: impl Fn(u16) -> EventListTarget,
    ) -> bool {
        let snapshot = self.capture_snapshot();
        let mut deltas = Vec::new();
        for &track in tracks {
            let target = target_of(track);
            let before = self.event_list_snapshot(&target);
            insert(self, track);
            let after = self.event_list_snapshot(&target);
            if before != after {
                deltas.push(UndoAction::EventList(EventListDelta {
                    target,
                    old: before,
                    new: after,
                }));
            }
        }
        if deltas.is_empty() {
            return false;
        }
        self.push_undo(UndoAction::Composite(deltas), label, snapshot);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单列表插入 + undo/redo 往返（以拍号为例；empty 文档自带默认拍号）。
    #[test]
    fn single_list_insert_undo_roundtrip() {
        let mut doc = Document::empty();
        let baseline = doc.event_list_snapshot(&EventListTarget::TimeSig).len();
        assert!(doc.event_list_edit_with_undo(
            EventListTarget::TimeSig,
            "undo.insert_timesig_event",
            |d| d.insert_time_sig_event(480),
        ));
        assert_eq!(
            doc.event_list_snapshot(&EventListTarget::TimeSig).len(),
            baseline + 1
        );

        assert!(doc.undo());
        assert_eq!(
            doc.event_list_snapshot(&EventListTarget::TimeSig).len(),
            baseline
        );
        assert!(doc.redo());
        assert_eq!(
            doc.event_list_snapshot(&EventListTarget::TimeSig).len(),
            baseline + 1
        );
    }

    /// 无改动（轨道不存在）时不 push undo。
    #[test]
    fn no_change_does_not_push_undo() {
        let mut doc = Document::empty();
        // 对不存在的轨道插入歌词：无改动。
        assert!(!doc.track_events_batch_with_undo(
            &[99],
            "undo.insert_lyrics_event",
            |d, t| d.insert_lyrics_event(t, 0),
            |t| EventListTarget::Lyrics { track: t },
        ));
        assert!(!doc.undo(), "空操作不应产生 undo 条目");
    }

    /// 多轨批量插入：一次 undo 全回退，redo 全恢复。
    #[test]
    fn batch_insert_single_composite_undo() {
        let mut doc = Document::empty();
        assert!(doc.track_events_batch_with_undo(
            &[0, 1, 2],
            "undo.insert_lyrics_event",
            |d, t| d.insert_lyrics_event(t, 240),
            |t| EventListTarget::Lyrics { track: t },
        ));
        for t in 0..3 {
            assert_eq!(
                doc.event_list_snapshot(&EventListTarget::Lyrics { track: t })
                    .len(),
                1
            );
        }
        // 一次 undo → 三条全回退。
        assert!(doc.undo());
        for t in 0..3 {
            assert!(
                doc.event_list_snapshot(&EventListTarget::Lyrics { track: t })
                    .is_empty()
            );
        }
        assert!(doc.redo());
        for t in 0..3 {
            assert_eq!(
                doc.event_list_snapshot(&EventListTarget::Lyrics { track: t })
                    .len(),
                1
            );
        }
    }

    /// 快照与存储顺序一致（插入后按 tick 排序）。
    #[test]
    fn snapshot_follows_sorted_order() {
        let mut doc = Document::empty();
        doc.insert_program_change_event(0, 960);
        doc.insert_program_change_event(0, 0);
        let list = doc.event_list_snapshot(&EventListTarget::ProgramChange { track: 0 });
        let ticks: Vec<u32> = list
            .iter()
            .map(|e| match e {
                EventListItem::ProgramChange(p) => p.tick,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(ticks, vec![0, 960]);
    }
}
