//! 单音符写路径原语：按 id 更新/移除、保序插入、跨桶移动。
//!
//! 统一 editor-core 各写路径原先手写的
//! `Arc::make_mut(&mut model.notes[k])` + 桶操作 + `mark_dirty`（+ 有时漏 sort）
//! 模式：桶有序性与 dirty/revision 簿记由模型层负责，调用方只描述变更。
//!
//! 批量（选区级）路径见 `editor_core::batch_ops`（其底层同样落到这些原语）。

use std::sync::Arc;

use yinhe_types::Note;

use crate::model::YinModel;

impl YinModel {
    /// 桶内按 id 定位（只读）。冷路径/测试用；热路径请用 `*_at`。
    pub fn note_by_id(&self, key: u8, id: u32) -> Option<&Note> {
        self.notes[key as usize].iter().find(|n| n.id == id)
    }

    /// 桶内按 id 更新（冷路径/基准用）。生产单音符编辑请用
    /// [`update_note_at`](Self::update_note_at)，避免全桶扫 id + 无条件 sort。
    pub fn update_note_by_id(
        &mut self,
        key: u8,
        id: u32,
        f: impl FnOnce(&mut Note),
    ) -> Option<Note> {
        let bucket = Arc::make_mut(&mut self.notes[key as usize]);
        let updated = bucket.update_by_id(id, f)?;
        bucket.sort();
        self.mark_dirty(key);
        Some(updated)
    }

    /// 按 `(start_tick, track, id)` 单点更新。start 改变时只搬移该音符，
    /// 不做全桶 sort；统计用 O(1) 增量更新，不触发 `rebuild_dirty()`。
    /// 返回 `(before, after)`。
    pub fn update_note_at(
        &mut self,
        key: u8,
        start: u32,
        track: u16,
        id: u32,
        f: impl FnOnce(&mut Note),
    ) -> Option<(Note, Note)> {
        let (before, after) =
            Arc::make_mut(&mut self.notes[key as usize]).update_at(start, track, id, f)?;
        self.apply_note_stat_delta(Some((key, &before)), Some((key, &after)));
        self.note_revisions[key as usize] = self.note_revisions[key as usize].wrapping_add(1);
        Some((before, after))
    }

    /// 按 `(start_tick, track, id)` 单点删除。只重算受影响块，统计增量更新。
    pub fn remove_note_at(&mut self, key: u8, start: u32, track: u16, id: u32) -> Option<Note> {
        let removed = Arc::make_mut(&mut self.notes[key as usize]).remove_at(start, track, id)?;
        self.apply_note_stat_delta(Some((key, &removed)), None);
        self.note_revisions[key as usize] = self.note_revisions[key as usize].wrapping_add(1);
        Some(removed)
    }

    /// 按 `(旧 start, track, id)` 把音符移到 `(new_key, new_start_tick)`；
    /// 保留 id 与 gate。返回 `(before, after)`。
    pub fn move_note_at(
        &mut self,
        old_key: u8,
        start: u32,
        track: u16,
        id: u32,
        new_key: u8,
        new_start_tick: u32,
    ) -> Option<(Note, Note)> {
        let before = {
            let bucket = Arc::make_mut(&mut self.notes[old_key as usize]);
            bucket.remove_at(start, track, id)?
        };
        let gate = before.end_tick.saturating_sub(before.start_tick);
        let new_start = new_start_tick.min(u32::MAX - 1);
        let after = Note {
            start_tick: new_start,
            end_tick: new_start.saturating_add(gate),
            ..before
        };
        Arc::make_mut(&mut self.notes[new_key as usize]).insert_sorted(after);
        self.apply_note_stat_delta(Some((old_key, &before)), Some((new_key, &after)));
        self.note_revisions[old_key as usize] =
            self.note_revisions[old_key as usize].wrapping_add(1);
        if new_key != old_key {
            self.note_revisions[new_key as usize] =
                self.note_revisions[new_key as usize].wrapping_add(1);
        }
        Some((before, after))
    }

    /// 桶内按 id 移除（冷路径/基准用）。生产单音符删除请用
    /// [`remove_note_at`](Self::remove_note_at)。
    pub fn remove_note_by_id(&mut self, key: u8, id: u32) -> Option<Note> {
        let bucket = Arc::make_mut(&mut self.notes[key as usize]);
        let removed = bucket.remove_by_id(id)?;
        self.mark_dirty(key);
        Some(removed)
    }

    /// 保序插入一个音符（内部 `mark_dirty`）。
    pub fn insert_note(&mut self, key: u8, note: Note) {
        Arc::make_mut(&mut self.notes[key as usize]).insert_sorted(note);
        self.mark_dirty(key);
    }

    /// 单音符插入：保序插入 + O(1) 统计增量，不置 dirty、不扫桶。
    pub fn insert_note_at(&mut self, key: u8, note: Note) {
        Arc::make_mut(&mut self.notes[key as usize]).insert_sorted(note);
        self.apply_note_stat_delta(None, Some((key, &note)));
        self.note_revisions[key as usize] = self.note_revisions[key as usize].wrapping_add(1);
    }

    /// 按 id 把音符移到 `(new_key, new_start_tick)`（冷路径/基准用）。
    /// 生产单音符移动请用 [`move_note_at`](Self::move_note_at)。
    ///
    /// 内部完成旧桶移除、新桶保序插入与两侧 `mark_dirty`。
    /// 返回 `(before, after)`；未命中返回 `None`。
    pub fn move_note(
        &mut self,
        old_key: u8,
        id: u32,
        new_key: u8,
        new_start_tick: u32,
    ) -> Option<(Note, Note)> {
        let (before, after) = {
            let bucket = Arc::make_mut(&mut self.notes[old_key as usize]);
            let mut note = bucket.remove_by_id(id)?;
            let before = note;
            note.end_tick = new_start_tick + (note.end_tick - note.start_tick);
            note.start_tick = new_start_tick;
            (before, note)
        };
        Arc::make_mut(&mut self.notes[new_key as usize]).insert_sorted(after);
        self.mark_dirty(old_key);
        if new_key != old_key {
            self.mark_dirty(new_key);
        }
        Some((before, after))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackData;

    fn note(id: u32, start: u32, end: u32) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: end,
            velocity: 100,
            track: 0,
        }
    }

    fn model() -> YinModel {
        YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        }
    }

    /// 更新/插入/移除：桶保持有序，dirty 与 revision 由模型层维护。
    #[test]
    fn update_insert_remove_keep_invariants() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200));
        m.insert_note(60, note(2, 300, 400));
        assert!(m.notes[60].is_sorted());
        assert!(m.dirty_keys[60]);

        // 把 id1 的 start 改到 id2 之后 → sort 兜底保持有序。
        let rev_before = m.note_revisions[60];
        let after = m
            .update_note_by_id(60, 1, |n| n.start_tick = 500)
            .expect("命中");
        assert_eq!(after.start_tick, 500);
        assert!(m.notes[60].is_sorted(), "破坏排序键后应重排");
        assert!(
            m.note_revisions[60] > rev_before,
            "mark_dirty 应推进 revision"
        );

        assert!(m.update_note_by_id(60, 999, |_| unreachable!()).is_none());
        let removed = m.remove_note_by_id(60, 2).expect("移除命中");
        assert_eq!(removed.start_tick, 300);
        assert_eq!(m.notes[60].len(), 1);
        assert!(m.remove_note_by_id(60, 2).is_none(), "重复移除返回 None");
    }

    /// 跨桶移动：旧桶移除、新桶保序插入、两侧标脏，返回 before/after。
    #[test]
    fn move_note_cross_bucket() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200));
        m.insert_note(64, note(2, 300, 400));
        m.dirty_keys = [false; yinhe_types::KEY_COUNT];

        let (before, after) = m.move_note(60, 1, 64, 250).expect("移动命中");
        assert_eq!((before.start_tick, before.end_tick), (100, 200));
        assert_eq!((after.start_tick, after.end_tick), (250, 350));
        assert_eq!(after.id, 1, "id 保留");
        assert!(m.notes[60].is_empty());
        assert!(m.notes[64].is_sorted());
        assert_eq!(m.notes[64].len(), 2);
        assert!(m.dirty_keys[60] && m.dirty_keys[64], "两侧都应标脏");

        // 同桶移动：仍是 remove + 保序插入。
        let (_, after) = m.move_note(64, 2, 64, 50).expect("同桶移动");
        assert_eq!(after.start_tick, 50);
        assert!(m.notes[64].is_sorted());

        assert!(m.move_note(60, 1, 64, 0).is_none(), "旧桶找不到 id");
    }

    /// `*_at` 单点路径：统计 O(1) 增量，不置 dirty，不触发全桶重扫。
    #[test]
    fn single_note_at_updates_stats_without_dirty() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200));
        m.insert_note(60, note(2, 300, 400));
        m.rebuild();
        m.dirty_keys = [false; yinhe_types::KEY_COUNT];
        assert_eq!(m.note_count, 2);
        assert_eq!(m.track_audible_count[0], 2);
        assert_eq!(m.tick_length, 400);

        let rev = m.note_revisions[60];
        m.update_note_at(60, 100, 0, 1, |n| n.velocity = 1)
            .expect("velocity 更新");
        assert_eq!(m.note_count, 2);
        assert_eq!(m.track_note_count[0], 2);
        assert_eq!(m.track_audible_count[0], 1);
        assert!(!m.dirty_keys[60], "单点编辑不应置 dirty");
        assert!(m.note_revisions[60] > rev, "GPU revision 应推进");

        m.update_note_at(60, 100, 0, 1, |n| n.end_tick = 1000)
            .expect("end 更新");
        assert_eq!(m.tick_length, 1000);

        let removed = m.remove_note_at(60, 100, 0, 1).expect("删除");
        assert_eq!(removed.id, 1);
        assert_eq!(m.note_count, 1);
        assert_eq!(m.bucket_note_count[60], 1);
        assert_eq!(m.tick_length, 400);
        assert!(!m.dirty_keys[60]);

        let (before, after) = m.move_note_at(60, 300, 0, 2, 64, 500).expect("跨 key 移动");
        assert_eq!((before.start_tick, before.end_tick), (300, 400));
        assert_eq!((after.start_tick, after.end_tick), (500, 600));
        assert_eq!(m.notes[60].len(), 0);
        assert_eq!(m.notes[64].len(), 1);
        assert_eq!(m.bucket_note_count[60], 0);
        assert_eq!(m.bucket_note_count[64], 1);
        assert_eq!(m.tick_length, 600);
        assert!(!m.dirty_keys[60] && !m.dirty_keys[64]);
    }

    #[test]
    fn try_alloc_note_id_stops_at_max() {
        let mut m = model();
        m.next_note_id = u32::MAX - 1;
        assert_eq!(m.try_alloc_note_id(), Some(u32::MAX - 1));
        assert_eq!(m.next_note_id, u32::MAX);
        assert_eq!(m.try_alloc_note_id(), None);
        assert_eq!(m.next_note_id, u32::MAX, "失败时不得回绕");
    }
}
