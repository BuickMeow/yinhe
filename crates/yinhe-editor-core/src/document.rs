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

mod io;
mod sync;
mod undo;

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
}

// ---------------------------------------------------------------------------
// Undo/redo
// ---------------------------------------------------------------------------

impl Document {
    /// Rebuild the track caches (`track_cache.info` / `track_cache.colors`) and
    /// resize track_visible/track_pianoroll_visible/track_overrides to match
    /// current track count.
    /// Called after track structure changes (add/remove/move/undo/redo).
    pub(crate) fn sync_track_caches(&mut self) {
        self.sync_track_caches_with_dark(true);
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
