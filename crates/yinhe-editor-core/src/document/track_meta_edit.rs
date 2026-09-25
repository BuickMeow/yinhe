//! Per-track metadata event editing: Lyrics / Chord.
//!
//! 与 `conductor_edit` 同样用全量 before/after 快照，
//! 因为歌词/和弦事件数量极少。
//!
//! popup 层自己用 `record_*_before` / `finalize_*_undo` 管理 undo 快照，
//! 这里只负责修改数据，不返回快照。

use std::sync::Arc;

use super::Document;
use super::event_edit::{track_delete_events, track_insert_event, track_set_event};

impl Document {
    /// 按 `old_tick` 找到 `track.lyrics` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_lyrics_event(&mut self, track: u16, old_tick: u32, new_tick: u32, new_text: String) {
        track_set_event!(self, track, lyrics, old_tick, |event| {
            event.tick = new_tick;
            event.text = new_text;
        });
    }

    /// 按 `old_tick` 找到 `track.chord` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_chord_event(&mut self, track: u16, old_tick: u32, new_tick: u32, new_text: String) {
        track_set_event!(self, track, chord, old_tick, |event| {
            event.tick = new_tick;
            event.text = new_text;
        });
    }

    // ── 批量删除（配合 event browser 多选）──

    /// 删除 `track.lyrics` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_lyrics_events(
        &mut self,
        track: u16,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::LyricsEvent>, Vec<yinhe_types::LyricsEvent>) {
        track_delete_events!(self, track, lyrics, ticks)
    }

    /// 删除 `track.chord` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_chord_events(
        &mut self,
        track: u16,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::ChordEvent>, Vec<yinhe_types::ChordEvent>) {
        track_delete_events!(self, track, chord, ticks)
    }

    /// 删除 `track.program_change` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_program_change_events(
        &mut self,
        track: u16,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::PcEvent>, Vec<yinhe_types::PcEvent>) {
        track_delete_events!(self, track, program_change, ticks)
    }

    // ── 插入新事件（默认值）──

    /// 插入一个 per-track 歌词事件（默认空文本）。
    pub fn insert_lyrics_event(&mut self, track: u16, tick: u32) {
        track_insert_event!(
            self,
            track,
            lyrics,
            yinhe_types::LyricsEvent {
                tick,
                text: String::new(),
            }
        );
    }

    /// 插入一个 per-track 和弦事件（默认空文本）。
    pub fn insert_chord_event(&mut self, track: u16, tick: u32) {
        track_insert_event!(
            self,
            track,
            chord,
            yinhe_types::ChordEvent {
                tick,
                text: String::new(),
            }
        );
    }

    /// 插入一个 Program Change 事件（默认 program=0, bank_msb=0, bank_lsb=0）。
    pub fn insert_program_change_event(&mut self, track: u16, tick: u32) {
        track_insert_event!(
            self,
            track,
            program_change,
            yinhe_types::PcEvent {
                tick,
                program: 0,
                bank_msb: 0,
                bank_lsb: 0,
            }
        );
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
        track_set_event!(self, track, program_change, old_tick, |event| {
            event.tick = new_tick;
            event.program = new_program;
        });
    }
}
