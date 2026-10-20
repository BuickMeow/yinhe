//! 插入菜单落地：位置/目标轨解析、直插与参数窗口分发。
//!
//! 规则（与「插入」菜单文案一致）：
//! - 位置 = 播放指示线 tick（打开参数窗口时锁定）；
//! - 全局事件（标签/BPM/拍号/调号/歌词/和弦）忽略音轨选择；
//! - 音轨事件（PC/自动化/轨内歌词/和弦）取选中普通轨（多选批量）；
//!   无选中 / 仅选中 Conductor → 歌词/和弦插全局版，PC/自动化回退最后一条普通轨；
//! - 多轨批量插入合并为一个 undo（撤销一次全回退）。

use eframe::egui;
use rust_i18n::t;

use crate::chrome::insert_actions::InsertAction;
use yinhe_editor_core::history::EventListTarget;

use super::App;

/// 普通轨过滤：排除 Conductor（Master）轨。
fn is_normal_track(doc: &yinhe_editor_core::document::Document, track: u16) -> bool {
    doc.edit.track_cache.conductor_idx != Some(track)
}

/// 显式选中的普通轨（升序；无选中 = 空）。
fn selected_normal_tracks(doc: &yinhe_editor_core::document::Document) -> Vec<u16> {
    let mut tracks: Vec<u16> = doc
        .edit
        .track_selected
        .iter()
        .copied()
        .filter(|&t| is_normal_track(doc, t))
        .collect();
    tracks.sort_unstable();
    tracks
}

/// 音轨类插入目标（PC/自动化）：选中普通轨（升序）；
/// 无选中 / 仅 Conductor → 回退最后一条普通轨。
fn insert_target_tracks(doc: &yinhe_editor_core::document::Document) -> Vec<u16> {
    let selected = selected_normal_tracks(doc);
    if !selected.is_empty() {
        return selected;
    }
    let n = doc.data.model.tracks.len();
    (0..n as u16)
        .rev()
        .find(|&t| is_normal_track(doc, t))
        .into_iter()
        .collect()
}

/// 「插入音轨」位置：选中集合中最靠后的轨之后；无选中 → 最后一条轨。
fn insert_track_after(doc: &yinhe_editor_core::document::Document) -> usize {
    let n = doc.data.model.tracks.len();
    match doc.edit.track_selected.iter().copied().max() {
        Some(t) => (t as usize).min(n.saturating_sub(1)),
        None => n.saturating_sub(1),
    }
}

/// 播放指示线 tick（无光标 = 0）。
fn cursor_tick(doc: &yinhe_editor_core::document::Document) -> u32 {
    doc.edit.cursor_tick.map(|t| t.max(0.0) as u32).unwrap_or(0)
}

/// 位置显示文本（小节.拍.tick）。
fn position_label(doc: &yinhe_editor_core::document::Document, tick: u32) -> String {
    let model = &doc.data.model;
    let (def_num, def_den) = model.tempo_map.time_sig_default;
    yinhe_types::time_format::format_tick_bar_beat_with_time_sig(
        tick as f64,
        model.meta.ppq,
        model.tempo_map.time_sig_events.as_slice(),
        def_num,
        def_den,
    )
}

/// 目标轨道显示文本（单条 = 轨道名；多条 = “名字 等 N 条轨道”）。
fn targets_label(doc: &yinhe_editor_core::document::Document, tracks: &[u16]) -> String {
    let name = tracks
        .first()
        .and_then(|&t| doc.data.model.tracks.get(t as usize))
        .map(|t| t.name.clone())
        .unwrap_or_default();
    if tracks.len() <= 1 {
        t!("dialog.insert_pc.targets_single", name = name).to_string()
    } else {
        t!(
            "dialog.insert_pc.targets_multi",
            name = name,
            n = tracks.len()
        )
        .to_string()
    }
}

/// 当前 tick 处的 BPM（取 tempo lane 上前一个事件值；剪切段内显示 0）。
fn bpm_at_tick(doc: &yinhe_editor_core::document::Document, tick: u32) -> f64 {
    if doc.data.model.tempo_map.is_skipped(tick) {
        return 0.0;
    }
    let events = &doc.data.model.conductor.tempo.events;
    events
        .iter()
        .take_while(|e| e.tick <= tick)
        .last()
        .or_else(|| events.first())
        .map(|e| e.value as f64)
        .unwrap_or(120.0)
}

/// 当前 tick 处的拍号（取 time_sig 上前一个事件；无事件 = 默认拍号）。
fn time_sig_at_tick(doc: &yinhe_editor_core::document::Document, tick: u32) -> (u8, u8) {
    let events = &doc.data.model.conductor.time_sig;
    events
        .iter()
        .take_while(|e| e.tick <= tick)
        .last()
        .or_else(|| events.first())
        .map(|e| (e.numerator, e.denominator))
        .unwrap_or(doc.data.model.tempo_map.time_sig_default)
}

/// 当前 tick 处的调号（取 key_sig 上前一个事件）。
fn key_sig_at_tick(
    doc: &yinhe_editor_core::document::Document,
    tick: u32,
) -> Option<(u8, yinhe_types::ScaleType)> {
    let events = &doc.data.model.conductor.key_sig;
    events
        .iter()
        .take_while(|e| e.tick <= tick)
        .last()
        .or_else(|| events.first())
        .map(|e| (e.root, e.scale))
}

impl App {
    /// 插入菜单动作落地（transport bar 插入菜单 / macOS 原生菜单共用）。
    pub(crate) fn handle_insert_action(&mut self, action: InsertAction, ctx: &egui::Context) {
        use InsertAction as A;
        let Some(doc_idx) = self.workspace.active_doc else {
            return;
        };
        let tick = cursor_tick(&self.workspace.documents[doc_idx]);
        match action {
            A::Track => self.insert_track_after_selection(),
            A::Marker => self.insert_marker_at_cursor(),
            A::Lyrics => self.insert_text_event(doc_idx, tick, true),
            A::Chord => self.insert_text_event(doc_idx, tick, false),
            A::Bpm => self.open_insert_bpm(doc_idx, tick, ctx),
            A::TimeSig => self.open_insert_timesig(doc_idx, tick, ctx),
            A::KeySig => self.open_insert_keysig(doc_idx, tick, ctx),
            A::ProgramChange => self.open_insert_pc(doc_idx, tick, ctx),
            A::Automation => self.open_insert_automation(doc_idx, tick, ctx),
        }
    }

    /// 「插入音轨」：在选中最后一条轨之后插入（无选中 = 最末），并选中新轨。
    /// 音轨结构变化 → teardown，下帧按新模型重建引擎（同新建音轨对话框）。
    fn insert_track_after_selection(&mut self) {
        let Some(doc_idx) = self.workspace.active_doc else {
            return;
        };
        let after = insert_track_after(&self.workspace.documents[doc_idx]);
        let doc = &mut self.workspace.documents[doc_idx];
        let before = doc.capture_snapshot();
        let created = if let Some(action) = doc.add_track(after) {
            doc.push_undo(action, t!("undo.add_track").as_ref(), before);
            true
        } else {
            false
        };
        if created {
            self.teardown_audio();
        }
    }

    /// 歌词 / 和弦直插：自动分流（选中普通轨 → 轨内批量；
    /// 无选中 / 仅 Conductor → 全局版）。
    fn insert_text_event(&mut self, doc_idx: usize, tick: u32, lyrics: bool) {
        let doc = &mut self.workspace.documents[doc_idx];
        let targets = selected_normal_tracks(doc);
        if targets.is_empty() {
            // 全局版（无普通轨或仅选中 Conductor）。
            let target = if lyrics {
                EventListTarget::ConductorLyrics
            } else {
                EventListTarget::ConductorChord
            };
            let label = if lyrics {
                "undo.insert_lyrics_event"
            } else {
                "undo.insert_chord_event"
            };
            doc.event_list_edit_with_undo(target, label, |d| {
                if lyrics {
                    d.insert_conductor_lyrics_event(tick);
                } else {
                    d.insert_conductor_chord_event(tick);
                }
            });
        } else {
            let label = if lyrics {
                "undo.insert_lyrics_event"
            } else {
                "undo.insert_chord_event"
            };
            doc.track_events_batch_with_undo(
                &targets,
                label,
                |d, track| {
                    if lyrics {
                        d.insert_lyrics_event(track, tick);
                    } else {
                        d.insert_chord_event(track, tick);
                    }
                },
                |track| {
                    if lyrics {
                        EventListTarget::Lyrics { track }
                    } else {
                        EventListTarget::Chord { track }
                    }
                },
            );
        }
    }

    /// 打开「插入 BPM」窗口。
    fn open_insert_bpm(&mut self, doc_idx: usize, tick: u32, ctx: &egui::Context) {
        let doc = &self.workspace.documents[doc_idx];
        let current = bpm_at_tick(doc, tick);
        let position = position_label(doc, tick);
        self.insert_bpm.open(tick, current, position);
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("insert_bpm_dialog"),
        );
    }

    /// 打开「插入拍号」窗口。
    fn open_insert_timesig(&mut self, doc_idx: usize, tick: u32, ctx: &egui::Context) {
        let doc = &self.workspace.documents[doc_idx];
        let (numerator, denominator) = time_sig_at_tick(doc, tick);
        let position = position_label(doc, tick);
        self.insert_timesig
            .open(tick, numerator, denominator, position);
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("insert_timesig_dialog"),
        );
    }

    /// 打开「插入音色（PC）」窗口（目标轨在打开时锁定）。
    fn open_insert_pc(&mut self, doc_idx: usize, tick: u32, ctx: &egui::Context) {
        let doc = &self.workspace.documents[doc_idx];
        let tracks = insert_target_tracks(doc);
        if tracks.is_empty() {
            return;
        }
        let position = position_label(doc, tick);
        let targets = targets_label(doc, &tracks);
        self.insert_pc.open(tick, position, targets, tracks);
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("insert_pc_dialog"),
        );
    }

    /// 打开「插入调号 / 调式」窗口（复用调式对话框的插入模式）。
    fn open_insert_keysig(&mut self, doc_idx: usize, tick: u32, ctx: &egui::Context) {
        let doc = &self.workspace.documents[doc_idx];
        let current = key_sig_at_tick(doc, tick);
        self.scale_dialog.open_for_insert(tick, current);
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of(crate::dialogs::scale::VIEWPORT_ID),
        );
    }

    /// 调式窗口插入模式确认：在 tick 处插入调号事件。
    pub(in crate::app) fn confirm_insert_key_sig(
        &mut self,
        tick: u32,
        root: u8,
        scale: yinhe_types::ScaleType,
    ) {
        let Some(doc_idx) = self.workspace.active_doc else {
            return;
        };
        let doc = &mut self.workspace.documents[doc_idx];
        doc.event_list_edit_with_undo(EventListTarget::KeySig, "undo.insert_keysig_event", |d| {
            d.insert_key_sig_event(tick, root, scale)
        });
    }

    /// 打开「插入自动化」窗口（复用自动化选择窗口的插入模式；
    /// 条目以第一条选中普通轨的设备为上下文）。
    fn open_insert_automation(&mut self, doc_idx: usize, tick: u32, ctx: &egui::Context) {
        let tracks = {
            let doc = &self.workspace.documents[doc_idx];
            insert_target_tracks(doc)
        };
        let Some(&first) = tracks.first() else {
            return;
        };
        let Some((title, device_name, entries, _)) = self.automation_picker_entries(first as usize)
        else {
            return;
        };
        self.automation_picker
            .open_for_insert(tick, tracks, title, device_name, entries);
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("automation_picker"),
        );
    }

    /// 自动化窗口插入模式确认：在 tick 处向 tracks 插入 target 事件。
    /// CC / Pitch Bend / RPN / NRPN 批量插全部选中轨；
    /// 设备参数（Param）只插到设备所属轨（第一条）。
    pub(in crate::app) fn apply_automation_insert(
        &mut self,
        tick: u32,
        tracks: &[u16],
        target: &yinhe_types::AutomationTarget,
    ) {
        let Some(doc_idx) = self.workspace.active_doc else {
            return;
        };
        let is_param = matches!(target, yinhe_types::AutomationTarget::Param { .. });
        let doc = &mut self.workspace.documents[doc_idx];
        let before = doc.capture_snapshot();
        let mut actions = Vec::new();
        for &track in tracks {
            if is_param && !actions.is_empty() {
                // 设备参数只对设备所属轨生效，多选中只插第一条。
                break;
            }
            let event = yinhe_types::AutomationEvent {
                id: 0,
                tick,
                value: target.default_value(),
                shape: yinhe_types::SegmentShape::Step,
            };
            if let Some((_, _, action)) =
                doc.add_automation_event(track as usize, target.clone(), event)
            {
                actions.push(action);
            }
        }
        if actions.is_empty() {
            return;
        }
        let action = if actions.len() == 1 {
            actions.remove(0)
        } else {
            yinhe_editor_core::history::UndoAction::Composite(actions)
        };
        doc.push_undo(action, "undo.insert_automation_event", before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> yinhe_editor_core::document::Document {
        yinhe_editor_core::document::Document::empty()
    }

    /// 显式选中普通轨：过滤 Conductor、升序排列。
    #[test]
    fn selected_normal_tracks_filters_conductor_and_sorts() {
        let mut d = doc();
        // empty() 的 conductor 在 0 号位。
        for t in [0u16, 3, 1] {
            d.edit.track_selected.insert(t);
        }
        assert_eq!(selected_normal_tracks(&d), vec![1, 3]);
    }

    /// PC/自动化目标：无选中 / 仅 Conductor → 回退最后一条普通轨。
    #[test]
    fn insert_target_tracks_falls_back_to_last_normal() {
        let mut d = doc();
        let last = (d.data.model.tracks.len() - 1) as u16;
        assert_eq!(insert_target_tracks(&d), vec![last]);
        d.edit.track_selected.insert(0);
        assert_eq!(insert_target_tracks(&d), vec![last]);
        d.edit.track_selected.insert(5);
        assert_eq!(insert_target_tracks(&d), vec![5]);
    }

    /// 歌词/和弦分流：仅显式选中普通轨才走轨内版（无选中 = 空 → 全局）。
    #[test]
    fn text_event_targets_do_not_fall_back() {
        let mut d = doc();
        assert!(selected_normal_tracks(&d).is_empty());
        d.edit.track_selected.insert(0);
        assert!(
            selected_normal_tracks(&d).is_empty(),
            "Conductor 不参与轨内插入"
        );
        d.edit.track_selected.insert(2);
        assert_eq!(selected_normal_tracks(&d), vec![2]);
    }

    /// 「插入音轨」位置：选中最后一条之后；无选中 = 最末。
    #[test]
    fn insert_track_after_uses_last_selected() {
        let mut d = doc();
        let last = d.data.model.tracks.len() - 1;
        assert_eq!(insert_track_after(&d), last);
        d.edit.track_selected.insert(2);
        d.edit.track_selected.insert(9);
        assert_eq!(insert_track_after(&d), 9);
        d.edit.track_selected.clear();
        d.edit.track_selected.insert(0);
        assert_eq!(insert_track_after(&d), 0, "仅选 Conductor → 插在其后");
    }

    /// BPM 默认值取 tick 前一个事件；无事件 = 120。
    #[test]
    fn bpm_at_tick_reads_previous_event() {
        let mut d = doc();
        assert_eq!(bpm_at_tick(&d, 0), 120.0);
        let model = std::sync::Arc::make_mut(&mut d.data.model);
        let conductor = std::sync::Arc::make_mut(&mut model.conductor);
        conductor.tempo.events.clear();
        conductor.tempo.events.push(yinhe_types::AutomationEvent {
            id: 1,
            tick: 0,
            value: 100.0,
            shape: yinhe_types::SegmentShape::Step,
        });
        conductor.tempo.events.push(yinhe_types::AutomationEvent {
            id: 2,
            tick: 960,
            value: 140.0,
            shape: yinhe_types::SegmentShape::Step,
        });
        assert_eq!(bpm_at_tick(&d, 480), 100.0);
        assert_eq!(bpm_at_tick(&d, 960), 140.0);
        assert_eq!(bpm_at_tick(&d, 1920), 140.0);
    }
}
