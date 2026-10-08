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

    // ── 带 undo 的 marker 编辑（标尺标签 / 编辑菜单用）──

    fn marker_list_snapshot(&self) -> Vec<crate::history::EventListItem> {
        self.data
            .model
            .conductor
            .markers
            .iter()
            .cloned()
            .map(crate::history::EventListItem::Marker)
            .collect()
    }

    /// before != after 时 push 一个 Marker 事件列表 undo；返回是否 push。
    fn push_marker_undo(
        &mut self,
        before: Vec<crate::history::EventListItem>,
        after: Vec<crate::history::EventListItem>,
        label: &str,
        snapshot: crate::history::EditSnapshot,
    ) -> bool {
        if before == after {
            return false;
        }
        self.push_undo(
            crate::history::UndoAction::EventList(crate::history::EventListDelta {
                target: crate::history::EventListTarget::Marker,
                old: before,
                new: after,
            }),
            label,
            snapshot,
        );
        true
    }

    /// 在 `tick` 插入一个 marker（文本 `text`）并 push undo。返回是否有改动。
    pub fn insert_marker_with_undo(&mut self, tick: u32, text: String, label: &str) -> bool {
        let snapshot = self.capture_snapshot();
        let before = self.marker_list_snapshot();
        let model = std::sync::Arc::make_mut(&mut self.data.model);
        let conductor = std::sync::Arc::make_mut(&mut model.conductor);
        conductor
            .markers
            .push(yinhe_types::MarkerEvent { tick, text });
        conductor.markers.sort_by_key(|e| e.tick);
        self.data.bump_revision();
        let after = self.marker_list_snapshot();
        self.push_marker_undo(before, after, label, snapshot)
    }

    /// 修改 `old_tick` 处 marker 的 tick/文本并 push undo。返回是否有改动。
    pub fn set_marker_with_undo(
        &mut self,
        old_tick: u32,
        new_tick: u32,
        text: String,
        label: &str,
    ) -> bool {
        let snapshot = self.capture_snapshot();
        let before = self.marker_list_snapshot();
        self.set_marker_event(old_tick, new_tick, text);
        let after = self.marker_list_snapshot();
        self.push_marker_undo(before, after, label, snapshot)
    }

    /// 删除 `tick` 处 marker 并 push undo。返回是否有改动。
    pub fn delete_marker_with_undo(&mut self, tick: u32, label: &str) -> bool {
        let snapshot = self.capture_snapshot();
        let before = self.marker_list_snapshot();
        let ticks: std::collections::HashSet<u32> = std::iter::once(tick).collect();
        self.delete_marker_events(&ticks);
        let after = self.marker_list_snapshot();
        self.push_marker_undo(before, after, label, snapshot)
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

    // ── 时间码直接输入（transport bar 第三列/前两列）──

    /// 时间码 BPM 输入。
    ///
    /// - tempo lane 为空 / 所有事件值相同：全部改为 `bpm`（空则插入 `at_tick` 处）。
    /// - 有多个不同值：在 `at_tick` 处 upsert 一个新 tempo 事件。
    ///
    /// 返回是否有实际改动。
    pub fn set_tempo_bpm(&mut self, at_tick: u32, bpm: f64) -> bool {
        let value = bpm as f32;
        if !value.is_finite() || value <= 0.0 {
            return false;
        }
        let model = std::sync::Arc::make_mut(&mut self.data.model);
        let conductor = std::sync::Arc::make_mut(&mut model.conductor);
        let lane = &mut conductor.tempo;
        let events = &mut lane.events;
        let all_same = events
            .first()
            .is_none_or(|first| events.iter().all(|e| e.value == first.value));
        if all_same {
            if events.is_empty() {
                lane.upsert(yinhe_types::AutomationEvent {
                    id: 0,
                    tick: at_tick,
                    value,
                    shape: yinhe_types::AutomationTarget::Tempo.default_shape(),
                });
            } else if events[0].value == value {
                return false;
            } else {
                for e in events.iter_mut() {
                    e.value = value;
                }
            }
        } else {
            if events.iter().any(|e| e.tick == at_tick && e.value == value) {
                return false;
            }
            lane.upsert(yinhe_types::AutomationEvent {
                id: 0,
                tick: at_tick,
                value,
                shape: yinhe_types::AutomationTarget::Tempo.default_shape(),
            });
        }
        model.commit_automation(&yinhe_types::AutomationTarget::Tempo);
        self.data.bump_revision();
        true
    }

    /// 时间码拍号输入。
    ///
    /// - time_sig 为空 / 所有事件相同：全部改为新拍号（空则插入 `at_tick` 处）。
    /// - 有多个不同值：在 `at_tick` 处插入/覆盖一个新拍号事件。
    ///
    /// `denominator_power` 为 2 的幂编码（4/4 → 2）。返回是否有实际改动。
    pub fn set_time_sig_value(
        &mut self,
        at_tick: u32,
        numerator: u8,
        denominator_power: u8,
    ) -> bool {
        let events = &self.data.model.conductor.time_sig;
        let all_same = events.first().is_none_or(|first| {
            events
                .iter()
                .all(|e| e.numerator == first.numerator && e.denominator == first.denominator)
        });
        let matches_new = |e: &yinhe_types::TimeSigEvent| {
            e.numerator == numerator && e.denominator == denominator_power
        };
        if all_same {
            if events.first().is_some_and(matches_new) {
                return false;
            }
            let model = std::sync::Arc::make_mut(&mut self.data.model);
            let conductor = std::sync::Arc::make_mut(&mut model.conductor);
            if conductor.time_sig.is_empty() {
                conductor.time_sig.push(yinhe_types::TimeSigEvent {
                    tick: at_tick,
                    numerator,
                    denominator: denominator_power,
                });
                conductor.time_sig.sort_by_key(|e| e.tick);
            } else {
                for e in conductor.time_sig.iter_mut() {
                    e.numerator = numerator;
                    e.denominator = denominator_power;
                }
            }
            model.rebuild_tempo_map();
            self.data.bump_revision();
            true
        } else {
            if events.iter().any(|e| e.tick == at_tick && matches_new(e)) {
                return false;
            }
            let model = std::sync::Arc::make_mut(&mut self.data.model);
            let conductor = std::sync::Arc::make_mut(&mut model.conductor);
            if let Some(e) = conductor.time_sig.iter_mut().find(|e| e.tick == at_tick) {
                e.numerator = numerator;
                e.denominator = denominator_power;
            } else {
                conductor.time_sig.push(yinhe_types::TimeSigEvent {
                    tick: at_tick,
                    numerator,
                    denominator: denominator_power,
                });
                conductor.time_sig.sort_by_key(|e| e.tick);
            }
            model.rebuild_tempo_map();
            self.data.bump_revision();
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BPM：空/单值/同值 → 改所有事件（不改 tick）；多值 → 在光标处插入。
    #[test]
    fn tempo_bpm_single_value_updates_all() {
        let mut doc = Document::empty();
        assert!(doc.set_tempo_bpm(0, 140.0));
        let events = &doc.data.model.conductor.tempo.events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].value, 140.0);

        // 单事件：改值不改 tick（at_tick 被忽略）
        assert!(doc.set_tempo_bpm(960, 90.0));
        let events = &doc.data.model.conductor.tempo.events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].value, 90.0);
        assert_eq!(events[0].tick, 0);

        // 同值：无改动
        assert!(!doc.set_tempo_bpm(0, 90.0));
        // 非法值：无改动
        assert!(!doc.set_tempo_bpm(0, 0.0));
    }

    /// BPM：多值 lane 在 at_tick upsert。
    #[test]
    fn tempo_bpm_multiple_values_inserts_at_tick() {
        let mut doc = Document::empty();
        // 手动构造多值 lane（模拟导入的 MIDI 工程）
        {
            let model = std::sync::Arc::make_mut(&mut doc.data.model);
            let conductor = std::sync::Arc::make_mut(&mut model.conductor);
            conductor.tempo.events = vec![
                yinhe_types::AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    ..Default::default()
                },
                yinhe_types::AutomationEvent {
                    tick: 960,
                    value: 140.0,
                    ..Default::default()
                },
            ];
        }
        assert_eq!(doc.data.model.conductor.tempo.events.len(), 2);

        assert!(doc.set_tempo_bpm(480, 100.0));
        let events = &doc.data.model.conductor.tempo.events;
        assert_eq!(events.len(), 3);
        assert_eq!((events[1].tick, events[1].value), (480, 100.0));

        // 同 tick 同值：无改动
        assert!(!doc.set_tempo_bpm(480, 100.0));
        // 同 tick 异值：覆盖
        assert!(doc.set_tempo_bpm(480, 110.0));
        assert_eq!(doc.data.model.conductor.tempo.events[1].value, 110.0);
    }

    /// 拍号：空/单值/同值 → 改所有事件；多值 → 在光标处插入。
    #[test]
    fn time_sig_single_value_updates_all() {
        let mut doc = Document::empty();
        assert!(doc.set_time_sig_value(0, 3, 2)); // 3/4
        let ts = &doc.data.model.conductor.time_sig;
        assert_eq!(ts.len(), 1);
        assert_eq!((ts[0].numerator, ts[0].denominator), (3, 2));
        assert!(!doc.set_time_sig_value(0, 3, 2));
    }

    #[test]
    fn time_sig_multiple_values_inserts_at_tick() {
        let mut doc = Document::empty();
        // 手动构造多值列表（模拟导入的 MIDI 工程）
        {
            let model = std::sync::Arc::make_mut(&mut doc.data.model);
            let conductor = std::sync::Arc::make_mut(&mut model.conductor);
            conductor.time_sig = vec![
                yinhe_types::TimeSigEvent {
                    tick: 0,
                    numerator: 4,
                    denominator: 2,
                },
                yinhe_types::TimeSigEvent {
                    tick: 1920,
                    numerator: 3,
                    denominator: 2,
                },
            ];
        }
        assert_eq!(doc.data.model.conductor.time_sig.len(), 2);

        assert!(doc.set_time_sig_value(480, 6, 3)); // 在 480 插入 6/8
        let ts = &doc.data.model.conductor.time_sig;
        assert_eq!(ts.len(), 3);
        assert_eq!(
            (ts[1].tick, ts[1].numerator, ts[1].denominator),
            (480, 6, 3)
        );

        // 同 tick 同值：无改动；同 tick 异值：覆盖
        assert!(!doc.set_time_sig_value(480, 6, 3));
        assert!(doc.set_time_sig_value(480, 7, 3));
        assert_eq!(doc.data.model.conductor.time_sig[1].numerator, 7);
    }

    /// 标尺标签编辑：insert / rename / delete 都应 push undo，且 undo 能还原。
    #[test]
    fn marker_edit_with_undo_roundtrip() {
        let mut doc = Document::empty();

        // 插入：push 一条 undo，数据出现。
        assert!(doc.insert_marker_with_undo(0, "A".into(), "undo.insert_marker_event"));
        assert_eq!(doc.data.model.conductor.markers.len(), 1);
        assert_eq!(doc.data.model.conductor.markers[0].text, "A");
        assert!(doc.undo());
        assert!(doc.data.model.conductor.markers.is_empty());

        // 重新插入两个，测试改名与删除。
        doc.insert_marker_with_undo(0, "A".into(), "undo.insert_marker_event");
        doc.insert_marker_with_undo(960, "B".into(), "undo.insert_marker_event");
        assert_eq!(doc.data.model.conductor.markers.len(), 2);

        // 改名：同 tick 改文本。
        assert!(doc.set_marker_with_undo(0, 0, "A2".into(), "undo.rename_marker_event"));
        assert_eq!(doc.data.model.conductor.markers[0].text, "A2");
        assert!(doc.undo());
        assert_eq!(doc.data.model.conductor.markers[0].text, "A");

        // 移动：改 tick（文本保留）。
        assert!(doc.set_marker_with_undo(0, 480, "A".into(), "undo.move_marker_event"));
        assert!(
            doc.data
                .model
                .conductor
                .markers
                .iter()
                .any(|m| m.tick == 480)
        );
        assert!(doc.undo());
        assert!(doc.data.model.conductor.markers.iter().any(|m| m.tick == 0));

        // 删除：整条移除。
        assert!(doc.delete_marker_with_undo(960, "undo.delete_marker_event"));
        assert_eq!(doc.data.model.conductor.markers.len(), 1);
        assert!(doc.undo());
        assert_eq!(doc.data.model.conductor.markers.len(), 2);
    }

    /// 未找到目标 / 无实际改动时不 push undo（避免空 undo 条目）。
    #[test]
    fn marker_edit_noop_does_not_push_undo() {
        let mut doc = Document::empty();
        // 删除不存在的 tick：无改动。
        assert!(!doc.delete_marker_with_undo(123, "undo.delete_marker_event"));
        // 改名不存在的 tick：无改动。
        assert!(!doc.set_marker_with_undo(123, 123, "x".into(), "undo.rename_marker_event"));
        // 没有可撤销的条目。
        assert!(!doc.undo());
    }
}
