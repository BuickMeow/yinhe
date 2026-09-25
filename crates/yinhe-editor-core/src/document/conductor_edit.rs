//! Conductor-track event editing: TimeSig / KeySig / Marker.
//!
//! 与 `automation_edit` 不同，conductor 的拍号/调号/标记事件数量极少
//! （通常 < 10），因此用全量 before/after 快照而非 per-event delta，
//! 简化 undo 逻辑。
//!
//! popup 层自己用 `record_*_before` / `finalize_*_undo` 管理 undo 快照，
//! 这里只负责修改数据，不返回快照。

use std::sync::Arc;

use super::Document;
use super::event_edit::{conductor_delete_events, conductor_insert_event, conductor_set_event};

impl Document {
    /// 按 `old_tick` 找到 `conductor.time_sig` 事件并修改其字段。
    ///
    /// 修改 tick 后重新排序，保持 `time_sig` 按 tick 升序。
    /// TempoMap 依赖 time_sig，会同步重建。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_time_sig_event(
        &mut self,
        old_tick: u32,
        new_tick: u32,
        new_numerator: u8,
        new_denominator: u8,
    ) {
        conductor_set_event!(rebuild tempo_map; self, time_sig, old_tick, |event| {
            event.tick = new_tick;
            event.numerator = new_numerator;
            event.denominator = new_denominator;
        });
    }

    /// 按 `old_tick` 找到 `conductor.key_sig` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_keysig_event(
        &mut self,
        old_tick: u32,
        new_tick: u32,
        new_root: u8,
        new_scale: yinhe_types::ScaleType,
    ) {
        conductor_set_event!(self, key_sig, old_tick, |event| {
            event.tick = new_tick;
            event.root = new_root;
            event.scale = new_scale;
        });
    }

    /// 按 `old_tick` 找到 `conductor.markers` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_marker_event(&mut self, old_tick: u32, new_tick: u32, new_text: String) {
        conductor_set_event!(self, markers, old_tick, |event| {
            event.tick = new_tick;
            event.text = new_text;
        });
    }

    /// 按 `old_tick` 找到 `conductor.lyrics` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_conductor_lyrics_event(&mut self, old_tick: u32, new_tick: u32, new_text: String) {
        conductor_set_event!(self, lyrics, old_tick, |event| {
            event.tick = new_tick;
            event.text = new_text;
        });
    }

    /// 按 `old_tick` 找到 `conductor.chord` 事件并修改其字段。
    /// 未找到对应 tick 的事件时静默返回。
    pub fn set_conductor_chord_event(&mut self, old_tick: u32, new_tick: u32, new_text: String) {
        conductor_set_event!(self, chord, old_tick, |event| {
            event.tick = new_tick;
            event.text = new_text;
        });
    }

    // ── 批量删除（配合 event browser 多选）──

    /// 删除 `conductor.time_sig` 中所有 tick 在 `ticks` 集合内的事件。
    /// 返回 (before, after) 用于 undo。TempoMap 会同步重建。
    pub fn delete_time_sig_events(
        &mut self,
        ticks: &std::collections::HashSet<u32>,
    ) -> (
        Vec<yinhe_types::TimeSigEvent>,
        Vec<yinhe_types::TimeSigEvent>,
    ) {
        conductor_delete_events!(rebuild tempo_map; self, time_sig, ticks)
    }

    /// 删除 `conductor.key_sig` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_key_sig_events(
        &mut self,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::KeySigEvent>, Vec<yinhe_types::KeySigEvent>) {
        conductor_delete_events!(self, key_sig, ticks)
    }

    /// 删除 `conductor.markers` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_marker_events(
        &mut self,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::MarkerEvent>, Vec<yinhe_types::MarkerEvent>) {
        conductor_delete_events!(self, markers, ticks)
    }

    /// 删除 `conductor.lyrics` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_conductor_lyrics_events(
        &mut self,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::LyricsEvent>, Vec<yinhe_types::LyricsEvent>) {
        conductor_delete_events!(self, lyrics, ticks)
    }

    /// 删除 `conductor.chord` 中所有 tick 在 `ticks` 集合内的事件。
    pub fn delete_conductor_chord_events(
        &mut self,
        ticks: &std::collections::HashSet<u32>,
    ) -> (Vec<yinhe_types::ChordEvent>, Vec<yinhe_types::ChordEvent>) {
        conductor_delete_events!(self, chord, ticks)
    }

    // ── 插入新事件（默认值）──

    /// 插入一个 TimeSig 事件（默认 4/4）。
    pub fn insert_time_sig_event(&mut self, tick: u32) {
        conductor_insert_event!(
            rebuild tempo_map;
            self,
            time_sig,
            yinhe_types::TimeSigEvent {
                tick,
                numerator: 4,
                denominator: 4,
            }
        );
    }

    /// 插入一个 KeySig 事件（默认 C 大调）。
    pub fn insert_key_sig_event(&mut self, tick: u32) {
        conductor_insert_event!(
            self,
            key_sig,
            yinhe_types::KeySigEvent {
                tick,
                root: 0,
                scale: yinhe_types::ScaleType::Major,
            }
        );
    }

    /// 插入一个 Marker 事件（默认空文本）。
    pub fn insert_marker_event(&mut self, tick: u32) {
        conductor_insert_event!(
            self,
            markers,
            yinhe_types::MarkerEvent {
                tick,
                text: String::new(),
            }
        );
    }

    /// 插入一个 conductor 歌词事件（默认空文本）。
    pub fn insert_conductor_lyrics_event(&mut self, tick: u32) {
        conductor_insert_event!(
            self,
            lyrics,
            yinhe_types::LyricsEvent {
                tick,
                text: String::new(),
            }
        );
    }

    /// 插入一个 conductor 和弦事件（默认空文本）。
    pub fn insert_conductor_chord_event(&mut self, tick: u32) {
        conductor_insert_event!(
            self,
            chord,
            yinhe_types::ChordEvent {
                tick,
                text: String::new(),
            }
        );
    }
}
