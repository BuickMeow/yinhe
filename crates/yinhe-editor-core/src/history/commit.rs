//! Undo stack, pending-edit tracking, and convenience commit helpers.

use std::collections::{HashMap, HashSet, VecDeque};

use yinhe_core::Selection;
use yinhe_types::{AnchorSelRect, AutomationPanelView};

use crate::document::Document;
use crate::edit_state::SelRectState;

use super::UndoAction;

// ---------------------------------------------------------------------------
// UndoEntry
// ---------------------------------------------------------------------------

/// 编辑前捕获的界面状态快照（undo/redo 恢复用）。
#[derive(Clone, Default)]
pub struct EditSnapshot {
    pub selected: Selection,
    pub track_selected: HashSet<u16>,
    pub sel_rect: SelRectState,
    /// AR 选框（f64 tick + usize track）。
    pub arr_sel_rect: Vec<(f64, f64, usize, usize)>,
    /// 通道 AR 选框（tick 范围 + 轨道集合）。
    pub chan_ar_sel: Vec<crate::edit_state::ChanSelRect>,
    /// 每面板的 AM 选框（controller_panels 顺序）。
    pub anchor_sel_rects: Vec<Vec<AnchorSelRect>>,
    /// 每面板的 AM 锚点成员位图（与 `anchor_sel_rects` 对齐；None = 矩形态）。
    pub anchor_members: Vec<Option<yinhe_types::NoteBitset>>,
    /// AR 展开 lane 的 AM 视图（选框 + 成员），按 (track, target) 键。
    pub arr_am_views:
        std::collections::HashMap<(u16, yinhe_types::AutomationTarget), AutomationPanelView>,
}

/// A single entry on the undo/redo stack.
pub struct UndoEntry {
    pub action: UndoAction,
    pub label: String,
    pub snapshot: EditSnapshot,
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// Per-document undo/redo stack using command pattern.
///
/// Each entry stores only the delta, so memory usage is proportional to
/// the number of affected notes, not the total model size.
pub struct UndoStack {
    /// `VecDeque` 而非 `Vec`：`push` 超过 `MAX_DEPTH` 时需要弹出最旧条目，
    /// `VecDeque::pop_front` 是 O(1)，`Vec::remove(0)` 是 O(n)。
    pub(crate) past: VecDeque<UndoEntry>,
    /// 与 `past` 一一对应的状态 id（单调递增，不因淘汰复用）。
    /// 用于“保存点”判定：比较状态身份而不是栈长度——超过 `MAX_DEPTH`
    /// 后每 push 都淘汰一条，长度恒定，用长度会把所有新编辑误判为未修改。
    past_ids: VecDeque<u64>,
    pub(crate) future: Vec<UndoEntry>,
    future_ids: Vec<u64>,
    next_id: u64,
    /// 上次保存时的状态 id（`None` = 栈底/初始状态）。
    saved_id: Option<u64>,
    /// Whether the document has an established "saved base state".
    /// - `true` for a fresh empty document (closing without save is fine)
    /// - `false` after loading a file (closing without save should prompt)
    /// - Set to `true` after first save or mark_loaded()
    pub(crate) has_saved_base: bool,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            past: VecDeque::new(),
            past_ids: VecDeque::new(),
            future: Vec::new(),
            future_ids: Vec::new(),
            next_id: 1,
            saved_id: None,
            // New empty document is considered "saved base" — closing without save is fine.
            has_saved_base: true,
        }
    }

    /// 当前状态的身份 id（撤销栈顶；空栈 = 初始状态 `None`）。
    pub fn marker(&self) -> Option<u64> {
        self.past_ids.back().copied()
    }

    /// Whether the document has unsaved changes.
    /// Returns true if:
    /// - The current state is not the last saved state, OR
    /// - The document was loaded from a file but never saved (`!has_saved_base`)
    pub fn is_dirty(&self) -> bool {
        self.marker() != self.saved_id || !self.has_saved_base
    }

    /// Mark the current state as saved (called after a successful save).
    pub fn mark_saved(&mut self) {
        self.mark_saved_at(self.marker());
    }

    /// 按**发起保存时的状态标记**标记已保存（异步保存用：保存期间的新编辑
    /// 不应被误标为已保存）。
    pub fn mark_saved_at(&mut self, marker: Option<u64>) {
        self.saved_id = marker;
        self.has_saved_base = true;
    }

    /// Mark that this document was loaded from a file (not a fresh empty doc).
    /// Called after loading MIDI/.yin. Sets `has_saved_base = false` so that
    /// closing without save will prompt the user.
    pub fn mark_loaded(&mut self) {
        self.saved_id = None;
        self.has_saved_base = false;
    }

    /// Record an undo entry (called *after* the edit is done).
    pub fn push(&mut self, entry: UndoEntry) {
        if self.past.len() >= super::MAX_DEPTH {
            self.past.pop_front();
            self.past_ids.pop_front();
        }
        let id = self.next_id;
        self.next_id += 1;
        self.past.push_back(entry);
        self.past_ids.push_back(id);
        self.future.clear();
        self.future_ids.clear();
    }

    /// undo/redo 内部搬运：取出撤销栈顶（状态 id + 条目）。
    pub(crate) fn take_past(&mut self) -> Option<(u64, UndoEntry)> {
        let id = self.past_ids.pop_back()?;
        let entry = self.past.pop_back()?;
        Some((id, entry))
    }

    /// undo 后把“当前状态”放入 redo 栈，沿用被撤销条目的状态 id。
    pub(crate) fn put_future(&mut self, id: u64, entry: UndoEntry) {
        self.future.push(entry);
        self.future_ids.push(id);
    }

    /// 取出 redo 栈顶（状态 id + 条目）。
    pub(crate) fn take_future(&mut self) -> Option<(u64, UndoEntry)> {
        let id = self.future_ids.pop()?;
        let entry = self.future.pop()?;
        Some((id, entry))
    }

    /// redo 后把条目放回撤销栈，沿用其状态 id。
    pub(crate) fn put_past(&mut self, id: u64, entry: UndoEntry) {
        self.past.push_back(entry);
        self.past_ids.push_back(id);
    }

    /// 不可逆操作 undo 后清空 redo 栈。
    pub(crate) fn clear_future(&mut self) {
        self.future.clear();
        self.future_ids.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Number of entries in the undo stack (public read access).
    pub fn past_len(&self) -> usize {
        self.past.len()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// 撤销栈内每步操作的标签，按**从旧到新**排列（`past` 顺序）。
    pub fn past_labels(&self) -> Vec<&str> {
        self.past.iter().map(|e| e.label.as_str()).collect()
    }

    /// 重做栈内每步操作的标签，按**从旧到新**排列（`future` 顺序）。
    pub fn future_labels(&self) -> Vec<&str> {
        self.future.iter().map(|e| e.label.as_str()).collect()
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.past_ids.clear();
        self.future.clear();
        self.future_ids.clear();
        self.saved_id = None;
        self.has_saved_base = true; // Reset to "fresh empty document" state
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PendingEdits — tracks old values for text-field edits
// ---------------------------------------------------------------------------

/// Tracks old values for TextEdit-like fields.
/// On `commit`, the old value is compared with the current value and an
/// `UndoAction` is pushed if they differ.
#[derive(Default)]
pub struct PendingEdits {
    map: HashMap<u64, String>,
}

impl PendingEdits {
    pub fn has(&self, id: u64) -> bool {
        self.map.contains_key(&id)
    }

    /// Save the old value before a text edit begins.
    pub fn begin(&mut self, id: u64, old_value: &str) {
        self.map.insert(id, old_value.to_string());
    }

    /// Take the saved old value without removing it (for comparison).
    pub fn get(&self, id: u64) -> Option<&str> {
        self.map.get(&id).map(|s| s.as_str())
    }

    /// Remove and return the saved old value.
    pub fn take(&mut self, id: u64) -> Option<String> {
        self.map.remove(&id)
    }
}

// ---------------------------------------------------------------------------
// Convenience helpers for text-field edits
// ---------------------------------------------------------------------------

/// Begin tracking a TextEdit/DragValue keyed by `id`.
pub fn begin_edit(pending: &mut PendingEdits, id: u64, old_value: &str) {
    pending.begin(id, old_value);
}

/// Generic commit: take old value from pending, compare with new, push undo entry if changed.
///
/// 快照在 push 时捕获（`doc.capture_snapshot()`）：文本类编辑不改变选区/选框，
/// 捕获时刻的界面状态即编辑前的状态。
fn commit_field<T: PartialEq>(
    doc: &mut Document,
    id: u64,
    new_value: T,
    parse_old: impl FnOnce(&str) -> T,
    make_action: impl FnOnce(T, T) -> UndoAction,
    label: &str,
) {
    let Some(old_str) = doc.edit.pending_edits.take(id) else {
        return;
    };
    let old = parse_old(&old_str);
    if old == new_value {
        return;
    }
    let snapshot = doc.capture_snapshot();
    doc.push_undo(make_action(old, new_value), label, snapshot);
}

/// Commit a track-name edit.
pub fn commit_track_name(doc: &mut Document, id: u64, track_idx: usize, new_name: &str) {
    commit_field(
        doc,
        id,
        new_name.to_string(),
        |s| s.to_string(),
        |old, new| UndoAction::TrackName {
            track_idx,
            old,
            new,
        },
        "Edit track name",
    );
}

/// Commit a PPQ edit.
///
/// `rescale` = true 表示此次 PPQ 变更同时 rescale 了所有音符/automation 的 tick，
/// undo/redo 需要反向 rescale 还原。
pub fn commit_ppq(doc: &mut Document, id: u64, new_value: u32, rescale: bool) {
    commit_field(
        doc,
        id,
        new_value,
        |s| s.parse().unwrap_or(480),
        |old, new| UndoAction::ProjectPpq { old, new, rescale },
        "Edit PPQ",
    );
}

/// Commit a compression-level edit.
pub fn commit_compression_level(doc: &mut Document, id: u64, new_value: i32) {
    commit_field(
        doc,
        id,
        new_value,
        |s| s.parse().unwrap_or(3),
        |old, new| UndoAction::CompressionLevel { old, new },
        "Edit zstd level",
    );
}
