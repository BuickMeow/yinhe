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

    // ── 批量删除（配合 event browser 多选）──

    /// 删除 `track.lyrics` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_lyrics_events(
        &mut self,
        track: u16,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::LyricsEvent>, Vec<yinhe_types::LyricsEvent>) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else {
            return (Vec::new(), Vec::new());
        };
        let td = Arc::make_mut(td);
        let before = td.lyrics.clone();
        td.lyrics.retain(|e| !ticks.contains(&e.tick));
        let after = td.lyrics.clone();
        self.data.bump_revision();
        (before, after)
    }

    /// 删除 `track.chord` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_chord_events(
        &mut self,
        track: u16,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::ChordEvent>, Vec<yinhe_types::ChordEvent>) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else {
            return (Vec::new(), Vec::new());
        };
        let td = Arc::make_mut(td);
        let before = td.chord.clone();
        td.chord.retain(|e| !ticks.contains(&e.tick));
        let after = td.chord.clone();
        self.data.bump_revision();
        (before, after)
    }

    /// 删除 `track.program_change` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_program_change_events(
        &mut self,
        track: u16,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::PcEvent>, Vec<yinhe_types::PcEvent>) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else {
            return (Vec::new(), Vec::new());
        };
        let td = Arc::make_mut(td);
        let before = td.program_change.clone();
        td.program_change.retain(|e| !ticks.contains(&e.tick));
        let after = td.program_change.clone();
        self.data.bump_revision();
        (before, after)
    }

    // ── 插入新事件（默认值）──

    /// 插入一个 per-track 歌词事件（默认空文本）。
    pub fn insert_lyrics_event(&mut self, track: u16, tick: u32) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else { return };
        let td = Arc::make_mut(td);
        td.lyrics.push(yinhe_types::LyricsEvent {
            tick,
            text: String::new(),
        });
        td.lyrics.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }

    /// 插入一个 per-track 和弦事件（默认空文本）。
    pub fn insert_chord_event(&mut self, track: u16, tick: u32) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else { return };
        let td = Arc::make_mut(td);
        td.chord.push(yinhe_types::ChordEvent {
            tick,
            text: String::new(),
        });
        td.chord.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }

    /// 插入一个 Program Change 事件（默认 program=0, bank_msb=0, bank_lsb=0）。
    pub fn insert_program_change_event(&mut self, track: u16, tick: u32) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else { return };
        let td = Arc::make_mut(td);
        td.program_change.push(yinhe_types::PcEvent {
            tick,
            program: 0,
            bank_msb: 0,
            bank_lsb: 0,
        });
        td.program_change.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }

    /// 按 `old_tick` 找到 `track.program_change` 事件并修改其 tick / program。
    /// 未找到对应 tick 的事件时静默返回。bank_msb / bank_lsb 保持不变。
    pub fn set_program_change_event(
        &mut self,
        track: u16,
        old_tick: u32,
        new_tick: u32,
        new_program: u8,
    ) {
        let model = Arc::make_mut(&mut self.data.model);
        let Some(td) = model.tracks.get_mut(track as usize) else { return };
        let td = Arc::make_mut(td);
        let Some(idx) = td.program_change.iter().position(|e| e.tick == old_tick) else { return };
        {
            let event = &mut td.program_change[idx];
            event.tick = new_tick;
            event.program = new_program;
        }
        td.program_change.sort_by_key(|e| e.tick);
        self.data.bump_revision();
    }
}
