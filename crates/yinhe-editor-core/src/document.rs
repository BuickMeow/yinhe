//! Per-document state: persistent data + editing state + undo history.

use std::sync::Arc;

use yinhe_core::{TrackData, YinModel};
use yinhe_types::TRACK_PALETTE;
use yinhe_yin::{MappingFile, ProjectFile};

use crate::edit_state::EditState;
use crate::history::{UndoEntry, UndoStack};
use crate::project_data::ProjectData;
use crate::quantize::QuantizePreset;

pub mod arrange_move;
pub mod automation_edit;
pub mod note_edit;
pub mod selection;
pub mod track_ops;

/// Per-track mutable overrides (mute, solo).
#[derive(Clone)]
pub struct TrackOverride {
    pub muted: bool,
    pub soloed: bool,
}

impl Default for TrackOverride {
    fn default() -> Self {
        Self {
            muted: false,
            soloed: false,
        }
    }
}

/// Per-document state: persistent data + editing state + undo history.
pub struct Document {
    pub data: ProjectData,
    pub edit: EditState,
    pub history: UndoStack,
    pub file_name: String,
    pub file_path: Option<String>,
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// Accessors + constructors
// ---------------------------------------------------------------------------

impl Document {
    pub fn model(&self) -> &YinModel {
        &self.data.model
    }

    pub fn track_names(&self) -> &[String] {
        &self.data.track_names
    }

    pub fn selected(&self) -> &yinhe_core::Selection {
        &self.edit.selected
    }

    pub fn track_info_cache(&self) -> &[yinhe_core::TrackInfo] {
        &self.edit.track_info_cache
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    /// Mark that this document was loaded from a file.
    /// Called after loading MIDI/.yin to indicate it's not a fresh empty doc.
    pub fn mark_loaded(&mut self) {
        self.history.mark_loaded();
    }

    pub fn empty() -> Self {
        let mut model = YinModel::new_empty_with_16_tracks();
        model.rebuild();

        let track_names = model.tracks.iter().map(|t| t.name.clone()).collect();
        let num_tracks = model.tracks.len();
        let conductor_track_idx = Some(0);

        let data = ProjectData::new(
            Arc::new(model.clone()),
            track_names,
            ProjectFile::from_meta(&model.meta),
            MappingFile::default(),
        );
        let track_info_cache = data.track_info();

        Document {
            data,
            edit: EditState {
                track_visible: vec![true; num_tracks],
                track_pianoroll_visible: vec![true; num_tracks],
                track_overrides: model
                    .tracks
                    .iter()
                    .map(|t| TrackOverride { muted: t.muted, soloed: t.soloed })
                    .collect(),
                track_info_cache,
                track_colors_cache: (0..num_tracks)
                    .map(|i| track_color(i, conductor_track_idx))
                    .collect(),
                conductor_track_idx,
                ..Default::default()
            },
            history: UndoStack::new(),
            file_name: "Untitled".into(),
            file_path: None,
        }
    }

    /// Create a Document from a freshly parsed YinModel.
    /// Path is used only to derive the file name; data ownership comes from
    /// the model. Inserts a conductor track at index 0 if absent.
    pub fn from_model(
        path: &str,
        model: YinModel,
        quantize_arrange: QuantizePreset,
        quantize_pianoroll: QuantizePreset,
        project_file: ProjectFile,
        mapping_file: MappingFile,
    ) -> Result<Self, String> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let mut model = model;

            // Detect conductor track; insert one if missing.
            let conductor_track_idx = detect_conductor_from_model(&model);
            if conductor_track_idx.is_none() {
                let mut conductor = TrackData::new(0, 0);
                conductor.name = "Conductor".to_string();
                // Shift all existing note track indices by 1 to make room.
                for bucket in model.notes.iter_mut() {
                    for n in Arc::make_mut(bucket).iter_mut() {
                        n.track += 1;
                    }
                }
                // Shift automation lane track indices by 1 to match.
                for track in model.tracks.iter_mut() {
                    let track = Arc::make_mut(track);
                    for lane in track.automation_lanes.iter_mut() {
                        lane.track += 1;
                    }
                }
                model.tracks.insert(0, Arc::new(conductor));
                model.rebuild();
            }
            let conductor_track_idx = detect_conductor_from_model(&model);

            let num_tracks = model.tracks.len();
            let track_names: Vec<String> = model.tracks.iter().map(|t| t.name.clone()).collect();
            let track_colors_cache = (0..num_tracks)
                .map(|i| track_color(i, conductor_track_idx))
                .collect();

            let mut data = ProjectData::new(
                Arc::new(model),
                track_names,
                project_file,
                mapping_file,
            );
            data.rebuild_model();

            let track_info_cache = data.track_info();
            let pc_map_cache = data.pc_map_cache();
            let track_overrides: Vec<TrackOverride> = data
                .model
                .tracks
                .iter()
                .map(|t| TrackOverride { muted: t.muted, soloed: t.soloed })
                .collect();

            Ok(Document {
                data,
                edit: EditState {
                    quantize_arrange,
                    quantize_pianoroll,
                    track_visible: vec![true; num_tracks],
                    track_pianoroll_visible: vec![true; num_tracks],
                    track_overrides,
                    track_info_cache,
                    pc_map_cache,
                    track_colors_cache,
                    conductor_track_idx,
                    ..Default::default()
                },
                history: UndoStack::new(),
                file_name,
                file_path: None,
            })
        })
    }

    /// Load a `.yin` file. Returns `(Document, soundfont_project_mode)`.
    pub fn from_yin_path(
        path: &str,
        quantize_arrange: QuantizePreset,
        quantize_pianoroll: QuantizePreset,
    ) -> std::io::Result<(Self, bool)> {
        let (model, sf, mapping) = yinhe_yin::load_yin_with_sf(path).map_err(|e| match e {
            yinhe_yin::YinError::Io(io) => io,
            other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
        })?;
        let project_file = yinhe_yin::ProjectFile::from_meta_with_sf(
            &model.meta,
            sf.mode,
            sf.overrides.clone(),
        );
        let mut doc = Self::from_model(path, model, quantize_arrange, quantize_pianoroll, project_file, mapping).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        doc.file_path = Some(path.to_string());
        Ok((doc, sf.mode))
    }

    pub fn recode_track_names(&mut self, _encoding: yinhe_mid2::MidiImportEncoding) {
        // TODO: implement track name re-encoding for YinModel
        self.data.bump_revision();
    }
}

// ---------------------------------------------------------------------------
// Undo/redo
// ---------------------------------------------------------------------------

impl Document {
    /// Undo the most recent operation. Returns true if something was undone.
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.history.past.pop_back() else {
            return false;
        };

        // Save current selection so redo can restore it.
        let current_selected = self.edit.selected.clone();
        let current_track_selected = self.edit.track_selected.clone();
        let current_sel_rect = self.edit.sel_rect.clone();

        // 反转 action（消耗 entry.action，零克隆），apply 一次后再 move 进 redo 栈。
        let reversed = entry.action.reversed();
        reversed.redo(self);

        // Restore selection from the undo entry.
        self.edit.selected = entry.selected;
        self.edit.track_selected = entry.track_selected;
        self.edit.sel_rect = entry.sel_rect;

        // Push reversed action onto the redo stack.
        self.history.future.push(UndoEntry {
            action: reversed,
            label: entry.label,
            selected: current_selected,
            track_selected: current_track_selected,
            sel_rect: current_sel_rect,
        });

        true
    }

    /// Redo the most recently undone operation. Returns true if something was redone.
    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.history.future.pop() else {
            return false;
        };

        let current_selected = self.edit.selected.clone();
        let current_track_selected = self.edit.track_selected.clone();
        let current_sel_rect = self.edit.sel_rect.clone();

        // future 栈里存的是 reversed action，再反转一次回到原始方向并 apply。
        let reversed = entry.action.reversed();
        reversed.redo(self);

        self.edit.selected = entry.selected;
        self.edit.track_selected = entry.track_selected;
        self.edit.sel_rect = entry.sel_rect;

        self.history.past.push_back(UndoEntry {
            action: reversed,
            label: entry.label,
            selected: current_selected,
            track_selected: current_track_selected,
            sel_rect: current_sel_rect,
        });

        true
    }

    /// Rebuild track_info_cache, track_colors_cache, and resize track_visible/
    /// track_pianoroll_visible/track_overrides to match current track count.
    /// Called after track structure changes (add/remove/move/undo/redo).
    pub(crate) fn sync_track_caches(&mut self) {
        self.edit.track_info_cache = self.data.track_info();
        let num_tracks = self.data.model.tracks.len();
        self.edit.track_colors_cache = (0..num_tracks)
            .map(|i| track_color(i, self.edit.conductor_track_idx))
            .collect();
        while self.edit.track_visible.len() < num_tracks {
            self.edit.track_visible.push(true);
        }
        while self.edit.track_pianoroll_visible.len() < num_tracks {
            self.edit.track_pianoroll_visible.push(true);
        }
        while self.edit.track_overrides.len() < num_tracks {
            self.edit.track_overrides.push(Default::default());
        }
        while self.edit.track_visible.len() > num_tracks {
            self.edit.track_visible.pop();
        }
        while self.edit.track_pianoroll_visible.len() > num_tracks {
            self.edit.track_pianoroll_visible.pop();
        }
        while self.edit.track_overrides.len() > num_tracks {
            self.edit.track_overrides.pop();
        }
    }

    /// 合并 mute/solo 状态为音频引擎用的 skip mask。
    ///
    /// 规则：有任意 solo 时，只有 soloed 轨道不 skip；无 solo 时，muted 轨道 skip。
    /// 音频引擎读此 mask 过滤音符和自动化事件。
    pub fn compute_skip_mask(&self) -> Vec<bool> {
        let has_solo = self.edit.track_overrides.iter().any(|t| t.soloed);
        self.edit
            .track_overrides
            .iter()
            .map(|ov| if has_solo { !ov.soloed } else { ov.muted })
            .collect()
    }

    /// 保存前把运行时 mute/solo 状态写回 model.tracks，
    /// 使 `sync_mapping_file` 能读到正确的持久化值。
    ///
    /// `TrackData` 是 `Arc`，用 `Arc::make_mut` 做 copy-on-write，
    /// 不影响 undo/redo 快照持有的旧 Arc。
    pub fn sync_overrides_to_model(&mut self) {
        let model = Arc::make_mut(&mut self.data.model);
        for (i, ov) in self.edit.track_overrides.iter().enumerate() {
            if let Some(td) = model.tracks.get_mut(i) {
                let td = Arc::make_mut(td);
                td.muted = ov.muted;
                td.soloed = ov.soloed;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Detect the conductor track: track 0 with no notes and no control data.
pub fn detect_conductor_from_model(model: &YinModel) -> Option<u16> {
    if model.tracks.is_empty() {
        return None;
    }
    let first = &model.tracks[0];
    if model.track_note_count.first().copied().unwrap_or(0) > 0 {
        return None;
    }
    if !first.automation_lanes.is_empty() || !first.program_change.is_empty() {
        return None;
    }
    Some(0)
}

/// Track color from palette, with conductor offset.
pub fn track_color(idx: usize, conductor_idx: Option<u16>) -> [f32; 3] {
    if Some(idx as u16) == conductor_idx {
        return [0.94, 0.94, 0.94];
    }
    let palette_idx = match conductor_idx {
        Some(c) if (idx as u16) > c => idx - 1,
        _ => idx,
    };
    TRACK_PALETTE[palette_idx % TRACK_PALETTE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_creates_valid_document_with_conductor_and_16_tracks() {
        let doc = Document::empty();
        assert_eq!(doc.model().tracks.len(), 17);
        assert_eq!(doc.model().tracks[0].name, "Conductor");
        assert_eq!(doc.model().tracks[1].name, "A1");
        assert_eq!(doc.model().tracks[16].name, "A16");
        assert_eq!(doc.track_names().len(), 17);
        assert_eq!(doc.edit.conductor_track_idx, Some(0));
        assert_eq!(doc.edit.track_visible.len(), 17);
        assert_eq!(doc.edit.track_pianoroll_visible.len(), 17);
        assert_eq!(doc.file_name, "Untitled");
        // Conductor track channels: A1 on ch0, A16 on ch15
        assert_eq!(doc.model().tracks[1].channel, 0);
        assert_eq!(doc.model().tracks[16].channel, 15);
    }

    #[test]
    fn detect_conductor_none_when_track0_has_notes() {
        let t = TrackData::new(0, 0);
        let mut model = YinModel {
            tracks: vec![Arc::new(t)],
            ..Default::default()
        };
        model.load_track_notes(vec![vec![yinhe_core::NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        }]]);
        model.rebuild();
        assert_eq!(detect_conductor_from_model(&model), None);
    }

    #[test]
    fn detect_conductor_some_when_track0_has_no_notes_and_no_ctrl() {
        let mut t1 = TrackData::new(0, 0);
        t1.name = "Conductor".into();
        let mut t2 = TrackData::new(0, 0);
        t2.name = "Piano".into();
        let mut model = YinModel {
            tracks: vec![Arc::new(t1), Arc::new(t2)],
            ..Default::default()
        };
        model.load_track_notes(vec![vec![], vec![yinhe_core::NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
        }]]);
        model.rebuild();
        assert_eq!(detect_conductor_from_model(&model), Some(0));
    }

    #[test]
    fn track_color_conductor_is_whiteish() {
        let color = track_color(0, Some(0));
        assert_eq!(color, [0.94, 0.94, 0.94]);
    }

    #[test]
    fn track_color_cycles_through_palette() {
        let first = track_color(0, None);
        assert_eq!(first, TRACK_PALETTE[0]);
        let second = track_color(1, None);
        assert_eq!(second, TRACK_PALETTE[1]);
        let wrap = track_color(16, None);
        assert_eq!(wrap, TRACK_PALETTE[0]);
    }

    #[test]
    fn track_color_offsets_after_conductor() {
        let color = track_color(1, Some(0));
        assert_eq!(color, TRACK_PALETTE[0]);
    }

    /// 回归测试：删除轨道后 undo 必须恢复被删轨道上的音符。
    /// 之前 TrackStructure 只存轨道元数据，被删音符在 retain 中物理消失，
    /// undo 时彻底丢失。修复后 deleted_notes 字段携带被删音符供 undo 恢复。
    #[test]
    fn remove_track_undo_restores_deleted_notes() {
        use crate::edit_state::SelRectState;
        use crate::history::UndoEntry;
        use std::collections::HashSet;
        use yinhe_core::NoteEvent;

        let mut doc = Document::empty();
        // conductor 在 track 0，A1 在 track 1。给 track 1 加 2 个音符。
        let notes_to_add = vec![
            NoteEvent { id: 0, start_tick: 0, end_tick: 480, key: 60, velocity: 100 },
            NoteEvent { id: 0, start_tick: 480, end_tick: 960, key: 64, velocity: 80 },
        ];
        for n in notes_to_add {
            let action = doc.add_note(1, n).expect("add_note should succeed");
            doc.history.push(UndoEntry {
                action,
                label: "add_note".to_string(),
                selected: Default::default(),
                track_selected: HashSet::new(),
                sel_rect: SelRectState::default(),
            });
        }
        assert_eq!(doc.model().note_count, 2, "添加后应有 2 个音符");
        assert_eq!(doc.model().track_note_count[1], 2, "track 1 应有 2 个音符");

        // 删除 track 1（其上的 2 个音符应被物理删除）
        let action = doc.remove_track(1).expect("remove_track should succeed");
        doc.history.push(UndoEntry {
            action,
            label: "remove_track".to_string(),
            selected: Default::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
        });
        assert_eq!(doc.model().note_count, 0, "删除轨道后音符应清零");
        // 删除后原 track 2..N 各自前移 1，track_note_count 长度也少了 1
        assert_eq!(doc.model().tracks.len(), 16, "轨道数应减 1");

        // Undo：必须恢复被删轨道上的音符
        assert!(doc.undo(), "undo 应成功");
        assert_eq!(doc.model().tracks.len(), 17, "undo 后轨道数应恢复");
        assert_eq!(doc.model().note_count, 2, "undo 后音符数必须恢复（这是 bug 修复点）");
        assert_eq!(doc.model().track_note_count[1], 2, "track 1 的音符必须恢复");
        // 具体音符内容也要核对（start_tick + key）
        assert_eq!(doc.model().notes[60].len(), 1);
        assert_eq!(doc.model().notes[60][0].start_tick, 0);
        assert_eq!(doc.model().notes[64].len(), 1);
        assert_eq!(doc.model().notes[64][0].start_tick, 480);

        // Redo：再次删除，音符应再次清零
        assert!(doc.redo(), "redo 应成功");
        assert_eq!(doc.model().tracks.len(), 16);
        assert_eq!(doc.model().note_count, 0, "redo 后音符应再次清零");
    }

    /// 回归测试：跨轨拖 automation 被 clamp 回原轨时事件不能蒸发。
    /// 之前 arrange_move.rs:157 在 dst==src 时 continue，但 phase 1 已把
    /// 被拖事件从源 lane 剔除，导致事件彻底丢失。修复后会把事件加回源 lane。
    #[test]
    fn arrange_move_automation_clamped_to_source_preserves_events() {
        use crate::edit_state::SelRectState;
        use crate::history::UndoEntry;
        use std::collections::HashSet;
        use yinhe_core::Selection;
        use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

        let mut doc = Document::empty();
        // conductor 在 track 0，给 track 1 加一条 CC7 lane，含 2 个事件
        {
            let model = Arc::make_mut(&mut doc.data.model);
            let track = Arc::make_mut(&mut model.tracks[1]);
            track.automation_lanes.push(AutomationLane {
                target: AutomationTarget::CC { controller: 7 },
                track: 1,
                events: vec![
                    AutomationEvent { tick: 0, value: 64.0, shape: SegmentShape::Step },
                    AutomationEvent { tick: 480, value: 80.0, shape: SegmentShape::Step },
                ],
            });
        }

        // 选区覆盖 track 1 的 tick 0..481（半开区间，包含 tick 0 和 480 两个事件）
        doc.edit.selected = Selection::default();
        doc.edit.selected.add_rect_track(0, 481, 0, 127, 1, 1);

        // 跨轨上移 1：raw_dst=0 是 conductor，被 clamp 回 track 1
        let action = doc.move_selected_arrange(100, -1)
            .expect("move_selected_arrange should return an action");
        doc.history.push(UndoEntry {
            action,
            label: "arrange_move".to_string(),
            selected: Default::default(),
            track_selected: HashSet::new(),
            sel_rect: SelRectState::default(),
        });

        // 关键断言：事件不能蒸发，应该在 track 1 的 CC7 lane 里
        let lane = &doc.model().tracks[1].automation_lanes[0];
        assert_eq!(lane.events.len(), 2, "事件数量必须保持 2（bug 修复点：之前变 0）");
        // delta_ticks=100 已应用
        assert_eq!(lane.events[0].tick, 100, "第一个事件 tick 应偏移 +100");
        assert_eq!(lane.events[1].tick, 580, "第二个事件 tick 应偏移 +100");

        // Undo：事件回到原 tick
        assert!(doc.undo());
        let lane = &doc.model().tracks[1].automation_lanes[0];
        assert_eq!(lane.events.len(), 2, "undo 后事件数量仍为 2");
        assert_eq!(lane.events[0].tick, 0, "undo 后第一个事件回到 tick 0");
        assert_eq!(lane.events[1].tick, 480, "undo 后第二个事件回到 tick 480");

        // Redo：事件再次偏移
        assert!(doc.redo());
        let lane = &doc.model().tracks[1].automation_lanes[0];
        assert_eq!(lane.events.len(), 2, "redo 后事件数量仍为 2");
        assert_eq!(lane.events[0].tick, 100);
        assert_eq!(lane.events[1].tick, 580);
    }
}
