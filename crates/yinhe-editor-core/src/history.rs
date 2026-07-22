//! Undo/redo history using command pattern.
//!
//! Instead of storing full snapshots (which cost O(model) memory per entry),
//! each undo entry stores only the delta — what changed. For note operations
//! this is the before/after state of the affected notes, typically a few
//! hundred bytes instead of hundreds of megabytes.

use std::sync::Arc;

use yinhe_types::{AutomationEvent, Note};

pub mod apply;
pub mod commit;
#[cfg(test)]
mod tests;

pub use commit::{
    begin_edit, commit_artist, commit_compression_level, commit_description,
    commit_ppq, commit_project_name, commit_track_name,
    PendingEdits, UndoEntry, UndoStack,
};

/// Maximum number of past edits kept in the undo stack.
pub const MAX_DEPTH: usize = 100;

// ---------------------------------------------------------------------------
// Delta types
// ---------------------------------------------------------------------------

/// Before/after state of affected notes for a single operation.
///
/// `before` = notes as they were before the edit (at their original positions).
/// `after`  = notes as they are after the edit (at their new positions).
///
/// For a delete: `before = removed`, `after = []`.
/// For an add:    `before = []`,      `after = added`.
/// For a move:    `before = originals`, `after = moved`.
#[derive(Clone, Debug)]
pub struct NoteDelta {
    pub before: Vec<(Note, u8)>,
    pub after: Vec<(Note, u8)>,
}

/// Automation lane before/after snapshot.
///
/// Stores the full event list of the affected lane before and after the edit.
/// Automation lanes typically contain few events (hundreds at most), so
/// full-snapshot undo is simpler and cheaper than per-event deltas.
///
/// `target` 让 `apply_automation_delta` 可以分派到 `track.automation_lanes`
/// 或 `conductor.tempo`：Tempo 走 conductor 路径，其他走 track 路径。
#[derive(Clone, Debug)]
pub struct AutomationDelta {
    pub track_idx: usize,
    pub lane_idx: usize,
    pub target: yinhe_types::AutomationTarget,
    pub before: Vec<AutomationEvent>,
    pub after: Vec<AutomationEvent>,
}

// ---------------------------------------------------------------------------
// Action enum
// ---------------------------------------------------------------------------

/// What changed — the delta needed to undo/redo an operation.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// Note-level changes (delete, add, move, resize, duplicate, transpose).
    Notes(NoteDelta),
    /// Automation lane event changes (add/move/delete/shape).
    Automation(AutomationDelta),
    /// A track name was edited.
    TrackName {
        track_idx: usize,
        old: String,
        new: String,
    },
    /// Project metadata was edited.
    ProjectName { old: String, new: String },
    ProjectArtist { old: String, new: String },
    ProjectDescription { old: String, new: String },
    ProjectPpq { old: u32, new: u32, rescale: bool },
    CompressionLevel { old: i32, new: i32 },
    /// Track structure changed (add/remove/move track).
    /// Stores full before/after track lists (metadata only) and
    /// a remap table: `note_remap[old_track_idx] = new_track_idx` (or u16::MAX if deleted).
    ///
    /// `deleted_notes` 捕获 remove_track 时被物理删除的音符（含其所在 key），
    /// 用于 undo 时把它们插回模型。add_track / move_track 该字段为空。
    /// 不参与 `reversed()` 的 before/after 交换——方向由 `tracks_after` 与
    /// `tracks_before` 的长度差决定（undo remove_track 时 tracks_after 更长，
    /// 此时才需要把 deleted_notes 插回）。
    TrackStructure {
        tracks_before: Vec<Arc<yinhe_core::TrackData>>,
        tracks_after: Vec<Arc<yinhe_core::TrackData>>,
        note_remap: Vec<u16>,  // old_track → new_track (u16::MAX = deleted)
        note_remap_inverse: Vec<u16>,  // new_track -> old_track (for undo)
        deleted_notes: Vec<(Note, u8)>,  // 被 remove_track 删掉的音符（含 key），用于 undo 恢复
    },
    /// Multiple actions applied atomically (undo/redo as a single step).
    Composite(Vec<UndoAction>),
}

impl UndoAction {
    /// Return the inverse action (swap before/after, old/new).
    ///
    /// 消耗 self 并用 `mem::swap` 交换 before/after，零克隆。
    /// 调用方应先取出 entry，再 `let rev = action.reversed(); rev.redo(doc);`
    /// 最后把 `rev` move 进对端栈。
    pub fn reversed(self) -> Self {
        match self {
            UndoAction::Notes(mut delta) => {
                std::mem::swap(&mut delta.before, &mut delta.after);
                UndoAction::Notes(delta)
            }
            UndoAction::Automation(mut delta) => {
                std::mem::swap(&mut delta.before, &mut delta.after);
                UndoAction::Automation(delta)
            }
            UndoAction::TrackName { track_idx, mut old, mut new } => {
                std::mem::swap(&mut old, &mut new);
                UndoAction::TrackName { track_idx, old, new }
            }
            UndoAction::ProjectName { mut old, mut new } => {
                std::mem::swap(&mut old, &mut new);
                UndoAction::ProjectName { old, new }
            }
            UndoAction::ProjectArtist { mut old, mut new } => {
                std::mem::swap(&mut old, &mut new);
                UndoAction::ProjectArtist { old, new }
            }
            UndoAction::ProjectDescription { mut old, mut new } => {
                std::mem::swap(&mut old, &mut new);
                UndoAction::ProjectDescription { old, new }
            }
            UndoAction::ProjectPpq { old, new, rescale } => UndoAction::ProjectPpq {
                old: new,
                new: old,
                rescale,
            },
            UndoAction::CompressionLevel { old, new } => UndoAction::CompressionLevel {
                old: new,
                new: old,
            },
            UndoAction::TrackStructure {
                mut tracks_before,
                mut tracks_after,
                mut note_remap,
                mut note_remap_inverse,
                deleted_notes,
            } => {
                // 交换前后 + 交换 remap / inverse_remap。
                // deleted_notes 不交换：它记录的是 remove_track 删掉的音符，
                // 在 undo（tracks_after.len() > tracks_before.len()）时插回。
                std::mem::swap(&mut tracks_before, &mut tracks_after);
                std::mem::swap(&mut note_remap, &mut note_remap_inverse);
                UndoAction::TrackStructure {
                    tracks_before,
                    tracks_after,
                    note_remap,
                    note_remap_inverse,
                    deleted_notes,
                }
            }
            UndoAction::Composite(actions) => {
                // Reverse order so that reversed().redo() undoes in reverse order,
                // matching the original undo() semantics.
                UndoAction::Composite(
                    actions.into_iter().rev().map(|a| a.reversed()).collect(),
                )
            }
        }
    }
}
