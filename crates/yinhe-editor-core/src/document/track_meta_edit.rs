//! Per-track metadata event editing: Lyrics / Chord.
//!
//! 与 `conductor_edit` 同样用全量 before/after 快照，
//! 因为歌词/和弦事件数量极少。

use std::sync::Arc;

use yinhe_types::{ChordEvent, LyricsEvent};

use super::Document;

impl Document {
    /// 按 `old_tick` 找到 `track.lyrics` 事件并修改其字段。
    ///
    /// 返回 `(before, after)` 全量快照供调用方 push undo。
    pub fn set_lyrics_event(
        &mut self,
        track: u16,
        old_tick: u32,
        new_tick: u32,
        new_text: String,
    ) -> Option<(Vec<LyricsEvent>, Vec<LyricsEvent>)> {
        let model = Arc::make_mut(&mut self.data.model);
        let td = model.tracks.get_mut(track as usize)?;
        let td = Arc::make_mut(td);
        let idx = td.lyrics.iter().position(|e| e.tick == old_tick)?;
        let before = td.lyrics.clone();
        {
            let event = &mut td.lyrics[idx];
            event.tick = new_tick;
            event.text = new_text;
        }
        td.lyrics.sort_by_key(|e| e.tick);
        let after = td.lyrics.clone();
        self.data.bump_revision();
        Some((before, after))
    }

    /// 按 `old_tick` 找到 `track.chord` 事件并修改其字段。
    ///
    /// 返回 `(before, after)` 全量快照供调用方 push undo。
    pub fn set_chord_event(
        &mut self,
        track: u16,
        old_tick: u32,
        new_tick: u32,
        new_text: String,
    ) -> Option<(Vec<ChordEvent>, Vec<ChordEvent>)> {
        let model = Arc::make_mut(&mut self.data.model);
        let td = model.tracks.get_mut(track as usize)?;
        let td = Arc::make_mut(td);
        let idx = td.chord.iter().position(|e| e.tick == old_tick)?;
        let before = td.chord.clone();
        {
            let event = &mut td.chord[idx];
            event.tick = new_tick;
            event.text = new_text;
        }
        td.chord.sort_by_key(|e| e.tick);
        let after = td.chord.clone();
        self.data.bump_revision();
        Some((before, after))
    }
}
