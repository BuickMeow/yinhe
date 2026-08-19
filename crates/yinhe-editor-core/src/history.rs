//! Undo/redo history using command pattern.
//!
//! Instead of storing full snapshots (which cost O(model) memory per entry),
//! each undo entry stores only the delta — what changed. For note operations
//! this is the before/after state of the affected notes, typically a few
//! hundred bytes instead of hundreds of megabytes.

use std::sync::Arc;

use yinhe_types::{AutomationEvent, MAX_KEY, Note};

pub mod apply;
pub mod commit;
#[cfg(test)]
mod tests;

pub use commit::{
    EditSnapshot, PendingEdits, UndoEntry, UndoStack, begin_edit, commit_artist,
    commit_compression_level, commit_description, commit_ppq, commit_project_name,
    commit_track_name,
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

/// 事件列表的写入目标。一个变体对应 conductor 或某个 track 上的一个事件列表字段。
///
/// 取消了原先 8 个 `UndoAction` 变体（TimeSig/KeySig/Marker/Lyrics/Chord/
/// ConductorLyrics/ConductorChord/ProgramChange）——它们全是同一模式：
/// "把某事件列表整体替换为 new"。这里用一个 target 枚举 + 一份 old/new 即可表达。
#[derive(Clone, Debug)]
pub enum EventListTarget {
    TimeSig,
    KeySig,
    Marker,
    ConductorLyrics,
    ConductorChord,
    Lyrics { track: u16 },
    Chord { track: u16 },
    ProgramChange { track: u16 },
}

/// `EventList` 快照中的单个事件项。覆盖所有事件列表类型。
#[derive(Clone, Debug, PartialEq)]
pub enum EventListItem {
    TimeSig(yinhe_types::TimeSigEvent),
    KeySig(yinhe_types::KeySigEvent),
    Marker(yinhe_types::MarkerEvent),
    Lyrics(yinhe_types::LyricsEvent),
    Chord(yinhe_types::ChordEvent),
    ProgramChange(yinhe_types::PcEvent),
}

/// 事件列表整体替换的 before/after 快照。
#[derive(Clone, Debug)]
pub struct EventListDelta {
    pub target: EventListTarget,
    pub old: Vec<EventListItem>,
    pub new: Vec<EventListItem>,
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
    /// Automation lane 结构变化（创建/删除整条 lane）。
    /// before/after 为 None 表示该侧 lane 不存在。
    AutomationLane {
        track_idx: usize,
        lane_idx: usize,
        before: Option<yinhe_types::AutomationLane>,
        after: Option<yinhe_types::AutomationLane>,
    },
    /// A track name was edited.
    TrackName {
        track_idx: usize,
        old: String,
        new: String,
    },
    /// A track color was edited (source of the ImageToMidi color event). RGBA.
    TrackColor {
        track_idx: usize,
        old: [f32; 4],
        new: [f32; 4],
    },
    /// Project metadata was edited.
    ProjectName {
        old: String,
        new: String,
    },
    ProjectArtist {
        old: String,
        new: String,
    },
    ProjectDescription {
        old: String,
        new: String,
    },
    ProjectPpq {
        old: u32,
        new: u32,
        rescale: bool,
    },
    CompressionLevel {
        old: i32,
        new: i32,
    },
    /// 事件列表整体替换（time_sig / key_sig / marker / lyrics / chord / program_change
    /// 等所有 conductor 级或 per-track 级事件列表共用）。具体目标由 `target` 指定。
    EventList(EventListDelta),
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
        note_remap: Vec<u16>, // old_track → new_track (u16::MAX = deleted)
        note_remap_inverse: Vec<u16>, // new_track -> old_track (for undo)
        deleted_notes: Vec<(Note, u8)>, // 被 remove_track 删掉的音符（含 key），用于 undo 恢复
    },
    /// 几何平移类操作（move/transpose）的操作式 undo：不存数据副本，
    /// 存逆操作参数（O(1) 内存，与受影响音符数无关）。
    ///
    /// `rects` = 操作前选区矩形（含 track 过滤）。redo 在 rects 收集音符
    /// 施加 (+delta)；undo 时 `reversed()` 把 rects 平移到操作后位置并取反
    /// delta，在操作后位置收集音符施加 (−delta)——与 redo 共用同一 apply。
    ///
    /// 前提：操作不触发 tick/key 边界 clamp（触发时生成端回退 `Notes`
    /// 副本制，保证 undo 精确）。栈序保证 undo 时对象集 = 操作刚完成时。
    MoveNotes {
        rects: Vec<(u32, u32, u8, u8, u16, u16)>,
        delta_ticks: i64,
        delta_keys: i32,
    },
    /// 镜像翻转（flip）的操作式 undo：两次镜像恒等，自逆操作。
    /// `bounds` = 镜像边界 (t0, t1, kl, kh)（选框整体范围，翻转后不变）。
    /// 前提：音符都在选框内（跨出选框的音符镜像会触发 clamp，生成端
    /// 检测到后回退 `Notes` 副本制）。
    FlipNotes {
        rects: Vec<(u32, u32, u8, u8, u16, u16)>,
        bounds: (u64, u64, u8, u8),
        axis: crate::document::note_edit::FlipAxis,
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
            UndoAction::AutomationLane {
                track_idx,
                lane_idx,
                mut before,
                mut after,
            } => {
                std::mem::swap(&mut before, &mut after);
                UndoAction::AutomationLane {
                    track_idx,
                    lane_idx,
                    before,
                    after,
                }
            }
            UndoAction::TrackName {
                track_idx,
                mut old,
                mut new,
            } => {
                std::mem::swap(&mut old, &mut new);
                UndoAction::TrackName {
                    track_idx,
                    old,
                    new,
                }
            }
            UndoAction::TrackColor {
                track_idx,
                mut old,
                mut new,
            } => {
                std::mem::swap(&mut old, &mut new);
                UndoAction::TrackColor {
                    track_idx,
                    old,
                    new,
                }
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
            UndoAction::CompressionLevel { old, new } => {
                UndoAction::CompressionLevel { old: new, new: old }
            }
            UndoAction::EventList(mut delta) => {
                std::mem::swap(&mut delta.old, &mut delta.new);
                UndoAction::EventList(delta)
            }
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
            UndoAction::MoveNotes {
                rects,
                delta_ticks,
                delta_keys,
            } => {
                // 逆操作：rects 平移到操作后位置（undo 时音符在此），delta 取反。
                // apply 统一为“在 rects 收集 + 施加 delta”。
                let moved_rects = rects
                    .into_iter()
                    .map(|(ts, te, kl, kh, tl, th)| {
                        (
                            (ts as i64 + delta_ticks).max(0) as u32,
                            (te as i64 + delta_ticks).max(0) as u32,
                            (kl as i32 + delta_keys).clamp(0, MAX_KEY as i32) as u8,
                            (kh as i32 + delta_keys).clamp(0, MAX_KEY as i32) as u8,
                            tl,
                            th,
                        )
                    })
                    .collect();
                UndoAction::MoveNotes {
                    rects: moved_rects,
                    delta_ticks: -delta_ticks,
                    delta_keys: -delta_keys,
                }
            }
            // 两次镜像恒等：自逆，原样返回。
            UndoAction::FlipNotes { .. } => self,
            UndoAction::Composite(actions) => {
                // Reverse order so that reversed().redo() undoes in reverse order,
                // matching the original undo() semantics.
                UndoAction::Composite(actions.into_iter().rev().map(|a| a.reversed()).collect())
            }
        }
    }
}
