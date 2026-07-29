//! Per-track metadata event editing: Lyrics / Chord.
//!
//! 与 `conductor_edit` 同样用全量 before/after 快照，
//! 因为歌词/和弦事件数量极少。
//!
//! popup 层自己用 `record_*_before` / `finalize_*_undo` 管理 undo 快照，
//! 这里只负责修改数据，不返回快照。

use std::sync::Arc;

use super::Document;

impl Document {
    /// 按 `old_tick` 找到 `track.lyrics` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_lyrics_event(
        &mut self,
        track: u16,
        old_tick: u32,
        new_tick: u32,
        new_text: String,
    ) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else { return };
        let td = Arc::make_mut(td);
        let Some(idx) = td.lyrics.iter().position(|e| e.tick == old_tick) else { return };
        {
            let event = &mut td.lyrics[idx];
            event.tick = new_tick;
            event.text = new_text;
        }
        td.lyrics.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }

    /// 按 `old_tick` 找到 `track.chord` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_chord_event(
        &mut self,
        track: u16,
        old_tick: u32,
        new_tick: u32,
        new_text: String,
    ) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else { return };
        let td = Arc::make_mut(td);
        let Some(idx) = td.chord.iter().position(|e| e.tick == old_tick) else { return };
        {
            let event = &mut td.chord[idx];
            event.tick = new_tick;
            event.text = new_text;
        }
        td.chord.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }
}
