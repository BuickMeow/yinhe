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
    /// 桶内按 id 定位（只读）。
    pub fn note_by_id(&self, key: u8, id: u32) -> Option<&Note> {
        self.notes[key as usize].iter().find(|n| n.id == id)
    }

    /// 桶内按 id 更新：`f` 修改命中音符；内部 `sort`（`is_sorted` 早退）
    /// + `mark_dirty`。返回更新后的音符副本；未命中返回 `None`。
    pub fn update_note_by_id(
        &mut self,
        key: u8,
        id: u32,
        f: impl FnOnce(&mut Note),
    ) -> Option<Note> {
        let bucket = Arc::make_mut(&mut self.notes[key as usize]);
        let n = bucket.find_mut(id)?;
        f(n);
        let updated = *n;
        bucket.sort();
        self.mark_dirty(key);
        Some(updated)
    }

    /// 桶内按 id 移除（内部 `mark_dirty`）。
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

    /// 按 id 把音符移到 `(new_key, new_start_tick)`（保留 id 与 **gate**）。
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
}
