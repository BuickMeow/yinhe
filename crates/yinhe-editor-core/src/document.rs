//! Per-document state: persistent data + editing state + undo history.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yinhe_core::{TrackData, YinModel};
use yinhe_mixer::MixerParams;
use yinhe_types::{Audibility, MixerChannelId, TRACK_PALETTE};
use yinhe_yin::{MappingFile, ProjectFile};

use crate::edit_state::{EditState, TrackCache};
use crate::history::{EditSnapshot, UndoAction, UndoEntry, UndoStack};
use crate::project_data::ProjectData;
use crate::quantize::QuantizePreset;

/// 会话内文档 id 分配器（不持久化，仅用于自动保存文件命名等关联）。
static NEXT_DOC_ID: AtomicU64 = AtomicU64::new(1);

fn next_doc_id() -> u64 {
    NEXT_DOC_ID.fetch_add(1, Ordering::Relaxed)
}

pub mod ap_edit;
pub mod arrange_edit;
pub mod arrange_move;
pub mod audio_clip_edit;
pub mod automation_edit;
pub mod clip_edit;
pub mod conductor_edit;
pub(crate) mod event_edit;
pub mod event_list_edit;
pub mod freeze_edit;
pub mod midi_import;
pub mod note_edit;
pub mod note_pencil;
pub mod rack_edit;
pub mod selection;
pub mod track_meta_edit;
pub mod track_ops;

/// Per-track mutable overrides (mute, solo).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackOverride {
    pub muted: bool,
    pub soloed: bool,
}

impl TrackOverride {
    pub fn audibility(&self) -> Audibility {
        if self.soloed {
            Audibility::Soloed
        } else if self.muted {
            Audibility::Muted
        } else {
            Audibility::Normal
        }
    }

    pub fn set_audibility(&mut self, audibility: Audibility) {
        self.muted = audibility == Audibility::Muted;
        self.soloed = audibility == Audibility::Soloed;
    }
}

/// Per-document state: persistent data + editing state + undo history.
pub struct Document {
    pub data: ProjectData,
    pub edit: EditState,
    pub history: UndoStack,
    pub file_name: String,
    pub file_path: Option<String>,
    /// 混音台参数（按源 MIDI 通道索引）。持久化进 .yin，但不进 undo 历史
    /// （DAW 惯例：推子移动不可撤销），脏标记由 `mixer_dirty` 单独跟踪。
    pub mixer: MixerParams,
    /// 混音参数自上次保存以来是否被修改（并进 is_dirty）。
    pub mixer_dirty: bool,
    /// 混音参数的单调修改计数：异步保存按“发起时计数”判断保存期间是否
    /// 又改过混音，避免把新改动误标为已保存。会话内状态，不落盘。
    pub mixer_revision: u64,
    /// 会话内稳定 id（自动保存文件命名与恢复关联用；不持久化、不参与界面对比）。
    pub doc_id: u64,
    /// 滚动条音符密度预览——AR/通道 AR 用（跟随 `track_visible` 显隐）。
    /// 每帧由 `sync_density` 在预算内增量刷新。
    pub density: crate::density::NoteDensityMap,
    /// 滚动条音符密度预览——PR 用（跟随 PR 专属显隐
    /// `track_pianoroll_visible ∪ 主音轨`，与 PR 画布一致）。
    ///
    /// 与 [`Self::density`] 分开维护的原因：PR 与 AR 的轨道显隐互相独立，
    /// 同一份数据无法同时满足两种过滤；编辑时两份一起增量重扫，成本可忽略。
    pub density_pr: crate::density::NoteDensityMap,
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

    pub fn selected(&self) -> &yinhe_core::Selection {
        &self.edit.selected
    }

    pub fn track_info_cache(&self) -> &[yinhe_core::TrackInfo] {
        &self.edit.track_cache.info
    }

    pub fn is_dirty(&self) -> bool {
        // “未完成就提示”：历史/混音有未保存修改，或尚未落盘且已有内容（file_path == None
        // 且音符数 >0）就视为脏，需弹“是否保存”。纯空工程（0 音符、无历史）不视为脏，
        // 避免首启空白 tab 关闭时无意义弹窗；should_replace 侧已对 0 音符空工程放行。
        self.history.is_dirty()
            || self.mixer_dirty
            || (self.file_path.is_none() && self.data.model.note_count > 0)
    }

    pub fn mark_saved(&mut self) {
        let marker = self.history.marker();
        let mixer_revision = self.mixer_revision;
        self.mark_saved_at(marker, mixer_revision);
    }

    /// 异步保存完成：按**发起保存时的状态标记**标记（保存期间的新编辑保持
    /// dirty）。
    pub fn mark_saved_at(&mut self, marker: Option<u64>, mixer_revision: u64) {
        self.history.mark_saved_at(marker);
        // 保存快照之后又改过混音：保持 dirty，等下一次保存落盘。
        if self.mixer_revision == mixer_revision {
            self.mixer_dirty = false;
        }
    }

    /// 混音参数被修改：标脏并推进修改计数。
    pub fn mark_mixer_dirty(&mut self) {
        self.mixer_dirty = true;
        self.mixer_revision = self.mixer_revision.wrapping_add(1);
    }

    /// 当前撤销栈的状态标记（发起异步保存时取，完成后原样传回）。
    pub fn undo_marker(&self) -> Option<u64> {
        self.history.marker()
    }

    /// 每帧预算内增量刷新滚动条密度预览。
    ///
    /// 维护两份数据：`density` 跟随 `track_visible`（AR/通道 AR 用），
    /// `density_pr` 跟随 PR 专属显隐（`track_pianoroll_visible ∪ 主音轨`，
    /// 与 PR 画布一致）。预算对半分给两份，编辑时各自只重扫脏 key。
    pub fn sync_density(&mut self, budget: std::time::Duration) {
        let Self {
            data,
            edit,
            density,
            density_pr,
            ..
        } = self;
        let half = budget / 2;
        density.sync(&data.model, &edit.track_visible, half);
        // 与 PR 画布同一份显隐规则（EditState::pr_visible）。
        let pr_visible = edit.pr_visible();
        density_pr.sync(&data.model, &pr_visible, half);
    }

    /// 当前撤销栈长度（异步保存的版本快照用）。
    pub fn undo_past_len(&self) -> usize {
        self.history.past_len()
    }

    /// 修改混音台参数的入口（MIX 界面）：写入并标脏，同时由调用方同步音频引擎。
    pub fn mixer_mut(&mut self) -> &mut MixerParams {
        self.mark_mixer_dirty();
        &mut self.mixer
    }

    /// Mark that this document was loaded from a file.
    /// Called after loading MIDI/.yin to indicate it's not a fresh empty doc.
    pub fn mark_loaded(&mut self) {
        self.history.mark_loaded();
    }

    /// 新工程/导入 MIDI 的默认混音台：不预设任何总线，降低学习曲线；
    /// 需要总线时由用户在 MIX 或 AR 手动添加。
    fn default_mixer() -> MixerParams {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        mixer
    }

    pub fn empty() -> Self {
        Self::from_empty_model(YinModel::new_empty_with_16_tracks())
    }

    /// 空白工作区（桌面/手机「新建工程」与手机启动占位共用）：
    /// 只保留 Conductor，其余音轨由 AR 的「+」创建。
    pub fn empty_conductor_only() -> Self {
        let mut model = YinModel::new_empty_with_16_tracks();
        model.tracks.truncate(1);
        Self::from_empty_model(model)
    }

    fn from_empty_model(mut model: YinModel) -> Self {
        model.rebuild();

        let num_tracks = model.tracks.len();
        let conductor_track_idx = Some(0);

        let data = ProjectData::new(
            Arc::new(model.clone()),
            ProjectFile::from_meta(&model.meta),
            MappingFile::default(),
        );
        let track_info_cache = data.track_info();

        let mut doc = Document {
            data,
            edit: EditState {
                track_visible: vec![true; num_tracks],
                track_pianoroll_visible: vec![true; num_tracks],
                track_locked: vec![false; num_tracks],
                track_overrides: model
                    .tracks
                    .iter()
                    .map(|t| TrackOverride {
                        muted: t.muted,
                        soloed: t.soloed,
                    })
                    .collect(),
                track_cache: TrackCache {
                    info: track_info_cache,
                    colors: (0..num_tracks)
                        .map(|i| track_color(&model.tracks[i], i, conductor_track_idx))
                        .collect(),
                    conductor_idx: conductor_track_idx,
                    ..Default::default()
                },
                ..Default::default()
            },
            history: UndoStack::new(),
            file_name: "Untitled".into(),
            file_path: None,
            mixer: Self::default_mixer(),
            doc_id: next_doc_id(),
            mixer_dirty: false,
            mixer_revision: 0,
            density: Default::default(),
            density_pr: Default::default(),
        };
        doc.default_empty_project_own_lc();
        doc
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
        mixer: Option<MixerParams>,
    ) -> Result<Self, String> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let mut model = model;
            // 自动化事件 id 不落盘：文档构建时兜底统一发号（幂等，已发号的
            // 只是重新编号；覆盖各平台加载路径与直接构造模型的调用方）。
            model.renumber_automation_ids();

            // Detect conductor track; insert one if missing.
            //（插入后需 rebuild 让 `track_note_count` 与轨号一致；新建的
            // conductor 一定在 0 号位，无需再判定。）
            let conductor_track_idx = if model.ensure_conductor_track() {
                model.rebuild();
                Some(0)
            } else {
                detect_conductor_from_model(&model)
            };
            model.normalize_folder_structure();
            // 权限清理：Folder 不承载任何内容，Master 不承载音符/音频片段；
            // 历史版本误拖入的内容在加载时一次性移除。
            model.sanitize_track_content();

            let num_tracks = model.tracks.len();
            let track_colors_cache = (0..num_tracks)
                .map(|i| track_color(&model.tracks[i], i, conductor_track_idx))
                .collect();

            // 无混音数据的导入（.mid / 压缩包内 MIDI / 老 .yin 无混音段）走新工程
            // 默认混音台：不预设总线；.yin 工程自带混音数据（Some）时原样保留。
            let mut mixer = mixer.unwrap_or_else(Self::default_mixer);
            mixer.ensure_len();
            let mut data = ProjectData::new(Arc::new(model), project_file, mapping_file);
            data.rebuild_model();

            let track_info_cache = data.track_info();
            let pc_map_cache = data.pc_map_cache();
            let track_overrides: Vec<TrackOverride> = data
                .model
                .tracks
                .iter()
                .map(|t| TrackOverride {
                    muted: t.muted,
                    soloed: t.soloed,
                })
                .collect();

            let mut doc = Document {
                data,
                edit: EditState {
                    quantize_arrange,
                    quantize_pianoroll,
                    track_visible: vec![true; num_tracks],
                    track_pianoroll_visible: vec![true; num_tracks],
                    track_locked: vec![false; num_tracks],
                    track_overrides,
                    track_cache: TrackCache {
                        info: track_info_cache,
                        colors: track_colors_cache,
                        pc_map: pc_map_cache,
                        conductor_idx: conductor_track_idx,
                    },
                    ..Default::default()
                },
                history: UndoStack::new(),
                file_name,
                file_path: None,
                mixer,
                mixer_dirty: false,
                mixer_revision: 0,
                doc_id: next_doc_id(),
                density: Default::default(),
                density_pr: Default::default(),
            };
            doc.migrate_shared_audio_home_lcs();
            doc.ensure_audio_targets();
            doc.ensure_rack_track_lcs();
            doc.adopt_loaded_freezes();
            Ok(doc)
        })
    }

    /// Load a `.yin` file. 工程内每通道音色库覆盖会填入 `edit.project_sf`。
    pub fn from_yin_path(
        path: &str,
        quantize_arrange: QuantizePreset,
        quantize_pianoroll: QuantizePreset,
    ) -> std::io::Result<Self> {
        let (model, sf, mapping, mixer) =
            yinhe_yin::load_yin_with_sf(path).map_err(|e| match e {
                yinhe_yin::YinError::Io(io) => io,
                other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
            })?;
        let project_file = yinhe_yin::ProjectFile::from_meta(&model.meta);
        let mut doc = Self::from_model(
            path,
            model,
            quantize_arrange,
            quantize_pianoroll,
            project_file,
            mapping,
            mixer,
        )
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        doc.apply_sf_overrides(&sf.overrides);
        doc.sync_sf_overrides_to_project_file();
        doc.file_path = Some(path.to_string());
        Ok(doc)
    }

    /// 把工程内的每通道音色库覆盖装入编辑态（加载工程后调用）。
    pub fn apply_sf_overrides(&mut self, overrides: &[yinhe_yin::SfChannelOverride]) {
        self.edit.project_sf.overrides = overrides
            .iter()
            .map(|co| {
                let entries = co
                    .entries
                    .iter()
                    .map(|e| crate::config::SfEntry {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect();
                (co.channel, entries)
            })
            .collect();
    }

    /// 把编辑态的每通道音色库覆盖写回 `project_file`（保存前调用）。
    pub fn sync_sf_overrides_to_project_file(&mut self) {
        self.data.project_file.sf_channel_overrides = self
            .edit
            .project_sf
            .overrides
            .iter()
            .map(|(channel, entries)| yinhe_yin::SfChannelOverride {
                channel: *channel,
                entries: entries
                    .iter()
                    .map(|e| yinhe_yin::SfEntryJson {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect(),
            })
            .collect();
    }

    pub fn recode_track_names(&mut self, _encoding: yinhe_midi::MidiImportEncoding) {
        // TODO: implement track name re-encoding for YinModel
        self.data.bump_revision();
    }
}

// ---------------------------------------------------------------------------
// Undo/redo
// ---------------------------------------------------------------------------

/// 按比例缩放快照里的所有 tick 字段（PPQ rescale 跨 undo/redo 用）。
fn scale_snapshot_ticks(snapshot: &EditSnapshot, factor: f64) -> EditSnapshot {
    fn tick_u32(v: u32, f: f64) -> u32 {
        (v as f64 * f).round() as u32
    }
    fn rect_f64(r: &mut (f64, f64, u8, u8), f: f64) {
        r.0 *= f;
        r.1 *= f;
    }
    let mut out = snapshot.clone();
    // 已换算成目标 PPQ，避免恢复时被再次缩放。
    out.ppq = 0;
    for r in &mut out.selected.rects {
        r.0 = tick_u32(r.0, factor);
        r.1 = tick_u32(r.1, factor);
    }
    if let Some((g0, g1)) = out.selected.filter.gate {
        out.selected.filter.gate = Some((tick_u32(g0, factor), tick_u32(g1, factor)));
    }
    for r in &mut out.sel_rect.rects {
        rect_f64(r, factor);
    }
    for r in &mut out.arr_sel_rect {
        r.0 *= factor;
        r.1 *= factor;
    }
    for r in &mut out.chan_ar_sel {
        r.t_start *= factor;
        r.t_end *= factor;
    }
    for rects in &mut out.anchor_sel_rects {
        for r in rects {
            r.tick_start *= factor;
            r.tick_end *= factor;
        }
    }
    for v in out.arr_am_views.values_mut() {
        for r in &mut v.anchor_sel_rects {
            r.tick_start *= factor;
            r.tick_end *= factor;
        }
    }
    out
}

impl Document {
    /// 捕获当前编辑状态快照（undo/redo 恢复用）。编辑**前**调用。
    pub fn capture_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            selected: self.edit.selected.clone(),
            track_selected: self.edit.track_selected.clone(),
            sel_rect: self.edit.sel_rect.clone(),
            arr_sel_rect: self.edit.arr_sel_rect.clone(),
            chan_ar_sel: self.edit.chan_ar_sel.clone(),
            anchor_sel_rects: self
                .edit
                .controller_panels
                .iter()
                .map(|p| p.anchor_sel_rects.clone())
                .collect(),
            anchor_members: self
                .edit
                .controller_panels
                .iter()
                .map(|p| p.anchor_members.clone())
                .collect(),
            arr_am_views: self.edit.arr_am_views.clone(),
            ppq: self.data.model.meta.ppq,
        }
    }

    /// 用编辑**前**快照 push 一个 undo entry。
    pub fn push_undo(&mut self, action: UndoAction, label: &str, before: EditSnapshot) {
        self.history.push(UndoEntry {
            action,
            label: label.to_string(),
            snapshot: before,
        });
    }

    /// 把快照恢复到 edit 状态（undo/redo 共用）。
    /// AM 选框/成员按面板索引对齐恢复，面板数量变化时防御跳过。
    /// PPQ 不同（undo/redo 跨过 PPQ rescale）时先按比例缩放快照里的 tick。
    fn restore_snapshot(edit: &mut EditState, snapshot: &EditSnapshot, current_ppq: u32) {
        let scaled;
        let snapshot = if snapshot.ppq != 0 && current_ppq != 0 && snapshot.ppq != current_ppq {
            scaled = scale_snapshot_ticks(snapshot, current_ppq as f64 / snapshot.ppq as f64);
            &scaled
        } else {
            snapshot
        };
        edit.selected = snapshot.selected.clone();
        edit.track_selected = snapshot.track_selected.clone();
        edit.sel_rect = snapshot.sel_rect.clone();
        edit.arr_sel_rect = snapshot.arr_sel_rect.clone();
        edit.chan_ar_sel = snapshot.chan_ar_sel.clone();
        for (i, panel) in edit.controller_panels.iter_mut().enumerate() {
            if let Some(rects) = snapshot.anchor_sel_rects.get(i) {
                panel.anchor_sel_rects = rects.clone();
            }
            if let Some(members) = snapshot.anchor_members.get(i) {
                panel.anchor_members = members.clone();
                // 成员由快照精确恢复，视为全部 rects 已物化。
                panel.materialized_anchor_rects = if members.is_some() {
                    panel.anchor_sel_rects.len()
                } else {
                    0
                };
            }
            if !panel.anchor_sel_rects.is_empty() {
                panel.dirty = true;
            }
        }
        edit.arr_am_views = snapshot.arr_am_views.clone();
    }

    /// PPQ rescale 后把当前编辑态里的 tick 按比例缩放（光标、各种选框、
    /// gate 筛选、循环区间）。临时绘制路径（钢笔/剪刀）直接清空。
    pub fn rescale_edit_ticks(&mut self, old_ppq: u32, new_ppq: u32) {
        if old_ppq == 0 || new_ppq == 0 || old_ppq == new_ppq {
            return;
        }
        let factor = new_ppq as f64 / old_ppq as f64;
        let tick_u32 = |v: u32| (v as f64 * factor).round() as u32;

        if let Some(t) = self.edit.cursor_tick {
            self.edit.cursor_tick = Some(t * factor);
        }
        for r in &mut self.edit.sel_rect.rects {
            r.0 *= factor;
            r.1 *= factor;
        }
        for r in &mut self.edit.selected.rects {
            r.0 = tick_u32(r.0);
            r.1 = tick_u32(r.1);
        }
        if let Some((g0, g1)) = self.edit.selected.filter.gate {
            self.edit.selected.filter.gate = Some((tick_u32(g0), tick_u32(g1)));
        }
        for r in &mut self.edit.arr_sel_rect {
            r.0 *= factor;
            r.1 *= factor;
        }
        for r in &mut self.edit.chan_ar_sel {
            r.t_start *= factor;
            r.t_end *= factor;
        }
        for panel in &mut self.edit.controller_panels {
            for r in &mut panel.anchor_sel_rects {
                r.tick_start *= factor;
                r.tick_end *= factor;
            }
            panel.dirty = true;
        }
        for view in self.edit.arr_am_views.values_mut() {
            for r in &mut view.anchor_sel_rects {
                r.tick_start *= factor;
                r.tick_end *= factor;
            }
            view.dirty = true;
        }
        // 临时绘制路径的 tick 语义已变，直接丢弃，避免落笔到错误位置。
        self.edit.pen_path = None;
        self.edit.scissors_line = None;
        self.edit.playback.rescale_ticks(factor);
    }

    /// 撤销栈内每步操作的标签（旧→新），供历史记录面板显示。
    pub fn undo_labels(&self) -> Vec<&str> {
        self.history.past_labels()
    }

    /// 重做栈内每步操作的标签（旧→新）。
    pub fn redo_labels(&self) -> Vec<&str> {
        self.history.future_labels()
    }

    /// Undo the most recent operation. Returns true if something was undone.
    pub fn undo(&mut self) -> bool {
        let Some((id, entry)) = self.history.take_past() else {
            return false;
        };

        // Save current selection so redo can restore it.
        let current = self.capture_snapshot();

        // 导入不保存源数据（1 亿音符不存两份），undo 后清空 redo 栈。
        let irreversible = matches!(&entry.action, UndoAction::ImportMidi { .. });

        // 反转 action（消耗 entry.action，零克隆），apply 一次后再 move 进 redo 栈。
        let reversed = entry.action.reversed();
        reversed.redo(self);

        // Restore selection from the undo entry.
        let current_ppq = self.data.model.meta.ppq;
        Self::restore_snapshot(&mut self.edit, &entry.snapshot, current_ppq);

        // Push reversed action onto the redo stack（沿用原状态 id：redo 后回到该状态）。
        self.history.put_future(
            id,
            UndoEntry {
                action: reversed,
                label: entry.label,
                snapshot: current,
            },
        );
        if irreversible {
            self.history.clear_future();
        }

        true
    }

    /// Redo the most recently undone operation. Returns true if something was redone.
    pub fn redo(&mut self) -> bool {
        let Some((id, entry)) = self.history.take_future() else {
            return false;
        };

        let current = self.capture_snapshot();

        // future 栈里存的是 reversed action，再反转一次回到原始方向并 apply。
        let reversed = entry.action.reversed();
        reversed.redo(self);

        // Restore selection from the redo entry.
        let current_ppq = self.data.model.meta.ppq;
        Self::restore_snapshot(&mut self.edit, &entry.snapshot, current_ppq);

        self.history.put_past(
            id,
            UndoEntry {
                action: reversed,
                label: entry.label,
                snapshot: current,
            },
        );

        true
    }

    /// 跳到撤销栈的指定长度（0 = 最初状态）：
    /// 向前/向后连续 undo / redo，**保留** future 栈，便于再点其它历史行跳转。
    /// 之后只要有新编辑，`push` 会照常清空 future，旧分支被覆盖。
    ///
    /// 返回是否有状态变化。不可逆操作（导入 MIDI）undo 时会清空 future，
    /// 此时循环自然停在能到达的位置。
    pub fn jump_history_to(&mut self, target_past_len: usize) -> bool {
        let mut changed = false;
        while self.history.past_len() > target_past_len {
            if !self.undo() {
                break;
            }
            changed = true;
        }
        while self.history.past_len() < target_past_len {
            if !self.redo() {
                break;
            }
            changed = true;
        }
        changed
    }

    /// Rebuild the track caches (`track_cache.info` / `track_cache.colors`) and
    /// resize track_visible/track_pianoroll_visible/track_overrides to match
    /// current track count.
    /// Called after track structure changes (add/remove/move/undo/redo).
    pub(crate) fn sync_track_caches(&mut self) {
        self.sync_track_caches_with_dark(true);
    }

    /// `sync_track_caches` with explicit dark/light theme for Conductor color.
    pub fn sync_track_caches_with_dark(&mut self, is_dark: bool) {
        let conductor_color = if is_dark {
            let c = yinhe_theme::base::BaseColors::DARK.text;
            [
                c.r as f32 / 255.0,
                c.g as f32 / 255.0,
                c.b as f32 / 255.0,
                1.0,
            ]
        } else {
            let c = yinhe_theme::base::BaseColors::LIGHT.text;
            [
                c.r as f32 / 255.0,
                c.g as f32 / 255.0,
                c.b as f32 / 255.0,
                1.0,
            ]
        };
        self.sync_track_caches_with_conductor_color(conductor_color);
    }

    /// `sync_track_caches` with precise Conductor color (`text_primary` from theme).
    pub fn sync_track_caches_with_conductor_color(&mut self, conductor_color: [f32; 4]) {
        self.ensure_audio_targets();
        self.edit.track_cache.rebuild(&self.data, conductor_color);
        let num_tracks = self.data.model.tracks.len();
        while self.edit.track_visible.len() < num_tracks {
            self.edit.track_visible.push(true);
        }
        while self.edit.track_pianoroll_visible.len() < num_tracks {
            self.edit.track_pianoroll_visible.push(true);
        }
        while self.edit.track_locked.len() < num_tracks {
            self.edit.track_locked.push(false);
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
        while self.edit.track_locked.len() > num_tracks {
            self.edit.track_locked.pop();
        }
        while self.edit.track_overrides.len() > num_tracks {
            self.edit.track_overrides.pop();
        }
        // 轨道结构/种类变化后同步 AP 表（共享 MIDI 通道 -> AP）。
        self.ensure_aps();
    }

    /// 设置 Track 的三态听感。Track solo 是内容门；进入 Track solo 时清空
    /// 所有 Channel solo，避免两层 solo 叠加。
    pub fn set_track_audibility(&mut self, idx: usize, audibility: Audibility) -> bool {
        let Some(ov) = self.edit.track_overrides.get_mut(idx) else {
            return false;
        };
        if ov.audibility() == audibility {
            return false;
        }
        ov.set_audibility(audibility);
        if audibility == Audibility::Soloed {
            let _ = self.clear_channel_solos();
        }
        true
    }

    /// 设置 Channel 的三态听感。进入 Channel solo 时清空所有 Track solo。
    pub fn set_channel_audibility(&mut self, node: MixerChannelId, audibility: Audibility) -> bool {
        if self.mixer.node_strip(node).audibility() == audibility {
            return false;
        }
        let mut strip = self.mixer.node_strip(node);
        strip.set_audibility(audibility);
        if !self.mixer.set_node_strip(node, strip) {
            return false;
        }
        if audibility == Audibility::Soloed {
            self.clear_track_solos();
        }
        self.mark_mixer_dirty();
        true
    }

    /// 清空所有 Channel solo，返回被改动的通道（用于推引擎）。
    pub fn clear_channel_solos(&mut self) -> Vec<MixerChannelId> {
        let mut changed = Vec::new();
        for node in self.mixer.nodes_mut() {
            if node.solo {
                node.solo = false;
                changed.push(match node.source {
                    yinhe_types::ChannelSource::Midi(ch) => MixerChannelId::Midi(ch),
                    yinhe_types::ChannelSource::Audio(ch) => MixerChannelId::Audio(ch),
                    yinhe_types::ChannelSource::Track(_) => match node.id {
                        yinhe_types::ChannelId::Lc(id) => MixerChannelId::TrackLc(id),
                        _ => MixerChannelId::Master,
                    },
                    yinhe_types::ChannelSource::Ap(_) | yinhe_types::ChannelSource::Master => {
                        MixerChannelId::Master
                    }
                });
            }
        }
        if !changed.is_empty() {
            self.mark_mixer_dirty();
        }
        changed
    }

    fn clear_track_solos(&mut self) {
        for ov in &mut self.edit.track_overrides {
            if ov.soloed {
                ov.soloed = false;
            }
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
    ///
    /// 只复制真正有变化的轨道：保存大工程时渲染线程通常持有同一份
    /// `Arc`，无条件 `make_mut` 会为每个轨道复制整条 `TrackData`。
    pub fn sync_overrides_to_model(&mut self) {
        let changed: Vec<usize> = self
            .edit
            .track_overrides
            .iter()
            .enumerate()
            .filter_map(|(i, ov)| {
                let td = self.data.model.tracks.get(i)?;
                (td.muted != ov.muted || td.soloed != ov.soloed).then_some(i)
            })
            .collect();
        if changed.is_empty() {
            return;
        }
        let model = Arc::make_mut(&mut self.data.model);
        for i in changed {
            let ov = &self.edit.track_overrides[i];
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
    // 判定逻辑统一在 `YinModel::has_conductor_track`（与 ensure 共用同一不变量）。
    model.has_conductor_track().then_some(0)
}

/// Track display color: prefers `TrackData.color` (when set, i.e. not the
/// default placeholder), otherwise falls back to the palette with conductor
/// offset. Conductor track uses main text color (`text_primary`) rather than
/// a fixed white/black. RGBA.
pub fn track_color(track: &TrackData, idx: usize, conductor_idx: Option<u16>) -> [f32; 4] {
    track_color_with_dark(track, idx, conductor_idx, true)
}

/// `track_color` with explicit dark/light theme — Conductor now equals
/// `text_primary`: dark `BaseColors::DARK.text` (220,220,220), light
/// `BaseColors::LIGHT.text` (30,30,34). For custom light presets the precise
/// value comes from `track_color_with_conductor_color` which the UI calls with
/// `theme::conductor_color_f32()`.
pub fn track_color_with_dark(
    track: &TrackData,
    idx: usize,
    conductor_idx: Option<u16>,
    is_dark: bool,
) -> [f32; 4] {
    let conductor_color = if is_dark {
        let c = yinhe_theme::base::BaseColors::DARK.text;
        [
            c.r as f32 / 255.0,
            c.g as f32 / 255.0,
            c.b as f32 / 255.0,
            1.0,
        ]
    } else {
        let c = yinhe_theme::base::BaseColors::LIGHT.text;
        [
            c.r as f32 / 255.0,
            c.g as f32 / 255.0,
            c.b as f32 / 255.0,
            1.0,
        ]
    };
    track_color_with_conductor_color(track, idx, conductor_idx, conductor_color)
}

/// `track_color` with explicit Conductor color (precise `text_primary` from theme).
pub fn track_color_with_conductor_color(
    track: &TrackData,
    idx: usize,
    conductor_idx: Option<u16>,
    conductor_color: [f32; 4],
) -> [f32; 4] {
    if Some(idx as u16) == conductor_idx {
        return conductor_color;
    }
    if track.color != yinhe_core::DEFAULT_TRACK_COLOR {
        return track.color;
    }
    let palette_idx = match conductor_idx {
        Some(c) if (idx as u16) > c => idx - 1,
        _ => idx,
    };
    let c = TRACK_PALETTE[palette_idx % TRACK_PALETTE.len()];
    [c[0], c[1], c[2], 1.0]
}

/// 把 `keep` 之外的轨道颜色转灰度（去色），保留 alpha。
///
/// `keep` 与 `colors` 按轨道索引对齐；没有任何 true 时原样返回，避免
/// “没有当前轨/选中轨”时把整个工程变成黑白。灰度用 Rec.601 亮度系数，
/// 只改 RGB，不改透明度和几何数据。
pub fn gray_other_track_colors(colors: &[[f32; 4]], keep: &[bool]) -> Vec<[f32; 4]> {
    if !keep.iter().any(|&k| k) {
        return colors.to_vec();
    }
    colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if keep.get(i).copied().unwrap_or(false) {
                *c
            } else {
                let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
                [y, y, y, c[3]]
            }
        })
        .collect()
}

/// Riff 横向重复的一段：从选区复制 `len` tick 到目标位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepeatTile {
    /// 源窗口起点（选区内的绝对 tick）。
    pub src_start: i64,
    /// 目标起点（绝对 tick）。
    pub dst_start: i64,
    /// 拷贝长度。
    pub len: i64,
}

/// 按 `extent`（从选区边界向外延伸的 tick 数）生成横向重复段。
///
/// 选区宽度是重复周期：先铺满完整周期，最后一段按剩余长度裁剪。
/// 例如选区 1 小节、延伸到 5/4 小节：先复制 1 小节，再复制开头 1/4 小节。
/// 向左延伸时裁剪的是选区尾部，保证重复序列连续。
pub fn repeat_tiles_x(sel_start: i64, sel_end: i64, extent: i64) -> Vec<RepeatTile> {
    let width = sel_end - sel_start;
    if width <= 0 || extent == 0 {
        return Vec::new();
    }
    let distance = extent.saturating_abs();
    let dir = extent.signum();
    let mut out = Vec::new();
    let mut covered = 0i64;
    while covered < distance {
        let len = (distance - covered).min(width);
        let (src_start, dst_start) = if dir > 0 {
            (sel_start, sel_end + covered)
        } else {
            (sel_end - len, sel_start - covered - len)
        };
        out.push(RepeatTile {
            src_start,
            dst_start,
            len,
        });
        covered += len;
    }
    out
}

#[cfg(test)]
#[path = "document/tests.rs"]
mod tests;
