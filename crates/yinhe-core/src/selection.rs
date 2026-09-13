//! Unified selection model for notes.
//!
//! Replaces the old `HashSet<(u16, u32, u8)>` with a compact representation:
//! a list of rectangular ranges in (tick, key, track) space plus an optional
//! attribute filter (velocity / gate / automation target+value).
//! A note is selected iff it falls within at least one rectangle AND passes
//! the attribute filter.
//!
//! Memory: 1000 万音符的矩形选择 = 1 个 rect (~40 bytes) vs 800MB HashSet.
//! 筛选只是在矩形上再加几个边界值，不物化匹配音符，内存仍是 O(rect 数)。

use yinhe_types::{AutomationTarget, MAX_KEY, Note};

/// 选区属性边界（作用于选中判定的附加约束）。
///
/// 默认全空 = 只按矩形空间判定（现状）。筛选弹窗写入这里的字段，
/// 之后所有选择操作（拖动/删除/复制/统计）自动按边界过滤。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectionFilter {
    /// 力度范围（含端点，0–127）。
    pub velocity: Option<(u8, u8)>,
    /// gate 范围（`end_tick - start_tick`，含端点）。
    pub gate: Option<(u32, u32)>,
    /// 自动化事件类型白名单（`None` = 不限制）。作用于 AR 联动的
    /// 自动化搬运与锚点选择收窄。
    pub automation_targets: Option<Vec<AutomationTarget>>,
    /// 自动化 value 范围（含端点，`None` = 不限制）。
    pub automation_value: Option<(f32, f32)>,
    /// 反选：矩形范围内不满足音符属性边界的音符变为选中。
    /// 只在至少一个音符属性边界（velocity/gate）存在时生效。
    pub invert: bool,
}

impl SelectionFilter {
    /// 是否未设任何边界（此时判定与无筛选完全一致）。
    pub fn is_empty(&self) -> bool {
        self.velocity.is_none()
            && self.gate.is_none()
            && self.automation_targets.is_none()
            && self.automation_value.is_none()
            && !self.invert
    }

    /// 是否设置了音符属性边界（velocity/gate）。
    pub fn has_note_bounds(&self) -> bool {
        self.velocity.is_some() || self.gate.is_some()
    }

    /// 是否设置了自动化边界。
    pub fn has_automation_bounds(&self) -> bool {
        self.automation_targets.is_some() || self.automation_value.is_some()
    }

    /// 音符是否满足属性边界（invert 之前）。
    pub fn note_matches(&self, note: &Note) -> bool {
        if let Some((lo, hi)) = self.velocity
            && !(lo..=hi).contains(&note.velocity)
        {
            return false;
        }
        if let Some((lo, hi)) = self.gate {
            let gate = note.end_tick.saturating_sub(note.start_tick);
            if !(lo..=hi).contains(&gate) {
                return false;
            }
        }
        true
    }

    /// 完整音符判定（含反选）。无音符边界时恒为 true（反选无对象）。
    pub fn accepts_note(&self, note: &Note) -> bool {
        if !self.has_note_bounds() {
            return true;
        }
        self.note_matches(note) != self.invert
    }

    /// 自动化锚点判定：target 在白名单内且 value 在范围内。
    pub fn accepts_automation(&self, target: &AutomationTarget, value: f32) -> bool {
        if let Some(list) = &self.automation_targets
            && !list.iter().any(|t| t == target)
        {
            return false;
        }
        if let Some((lo, hi)) = self.automation_value
            && !(lo..=hi).contains(&value)
        {
            return false;
        }
        true
    }
}

/// Unified selection model for notes.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    /// Rectangular ranges: (tick_start, tick_end, key_lo, key_hi, track_lo, track_hi).
    /// tick_end is exclusive (half-open range). track_lo..=track_hi inclusive.
    pub rects: Vec<(u32, u32, u8, u8, u16, u16)>,
    /// 属性边界（筛选）。空 = 只按矩形空间判定。
    pub filter: SelectionFilter,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
        self.filter = SelectionFilter::default();
    }

    /// Clear only the rectangles, keeping the attribute filter.
    /// Used by操作式 undo 重建选择集时（filter 属于"筛选器"状态，不随操作回滚）。
    pub fn clear_rects(&mut self) {
        self.rects.clear();
    }

    /// Add a rect with full (tick, key, track) range.
    /// Defaults track_lo=0, track_hi=65535 (match all tracks).
    pub fn add_rect(&mut self, tick_start: u32, tick_end: u32, key_lo: u8, key_hi: u8) {
        self.add_rect_track(tick_start, tick_end, key_lo, key_hi, 0, u16::MAX);
    }

    /// Add a rect with explicit track range.
    pub fn add_rect_track(
        &mut self,
        tick_start: u32,
        tick_end: u32,
        key_lo: u8,
        key_hi: u8,
        track_lo: u16,
        track_hi: u16,
    ) {
        if tick_end > tick_start {
            self.rects
                .push((tick_start, tick_end, key_lo, key_hi, track_lo, track_hi));
        }
    }

    /// Check if a specific note is selected **by space only** (filter ignored).
    ///
    /// Prefer [`Selection::accepts_note`] in edit paths so the attribute
    /// filter is honored; this remains for callers that already apply the
    /// filter themselves.
    pub fn contains(&self, track: u16, start_tick: u32, key: u8) -> bool {
        self.rects.iter().any(|&(ts, te, kl, kh, tl, th)| {
            track >= tl
                && track <= th
                && key >= kl
                && key <= kh
                && start_tick >= ts
                && start_tick < te
        })
    }

    /// Full note acceptance: space hit AND attribute filter pass.
    pub fn accepts_note(&self, note: &Note, key: u8) -> bool {
        if !self.filter.has_note_bounds() {
            return self.contains(note.track, note.start_tick, key);
        }
        self.contains(note.track, note.start_tick, key) && self.filter.accepts_note(note)
    }

    /// 是否设置了任何筛选边界。
    pub fn has_filter(&self) -> bool {
        !self.filter.is_empty()
    }

    /// Number of rects (for undo snapshot size estimation).
    pub fn len(&self) -> usize {
        self.rects.len()
    }

    /// Offset all rects by (delta_ticks, delta_keys).
    /// Clamps key to [0, MAX_KEY], tick to >= 0. Track range unchanged.
    /// Attribute filter touched by neither ticks nor keys.
    pub fn offset(&mut self, delta_ticks: i64, delta_keys: i32) {
        for rect in &mut self.rects {
            let (ts, te, kl, kh, tl, th) = *rect;
            let new_ts = (ts as i64 + delta_ticks).max(0) as u32;
            let new_te = (te as i64 + delta_ticks).max(0) as u32;
            let new_kl = (kl as i32 + delta_keys).clamp(0, MAX_KEY as i32) as u8;
            let new_kh = (kh as i32 + delta_keys).clamp(0, MAX_KEY as i32) as u8;
            if new_te > new_ts {
                *rect = (new_ts, new_te, new_kl, new_kh, tl, th);
            }
        }
    }

    /// Offset only the tick range of all rects (used by AR arrange drag).
    pub fn offset_ticks(&mut self, delta_ticks: i64) {
        for rect in &mut self.rects {
            let (ts, te, kl, kh, tl, th) = *rect;
            let new_ts = (ts as i64 + delta_ticks).max(0) as u32;
            let new_te = (te as i64 + delta_ticks).max(0) as u32;
            if new_te > new_ts {
                *rect = (new_ts, new_te, kl, kh, tl, th);
            }
        }
    }

    /// Offset only the track range of all rects (used by AR arrange drag).
    pub fn offset_tracks(&mut self, delta_tracks: i32) {
        for rect in &mut self.rects {
            let (ts, te, kl, kh, tl, th) = *rect;
            let new_tl = (tl as i32 + delta_tracks).max(0) as u16;
            let new_th = (th as i32 + delta_tracks).max(0) as u16;
            *rect = (ts, te, kl, kh, new_tl, new_th);
        }
    }

    /// Remove rects matching the given PR selection-box rects
    /// `(tick_start, tick_end, key_lo, key_hi)`. Used by cross-view selection
    /// exclusivity (PR/AR/AM 三视图选框互斥).
    pub fn remove_rects(&mut self, rects: &[(u32, u32, u8, u8)]) {
        self.rects.retain(|r| {
            !rects
                .iter()
                .any(|q| q.0 == r.0 && q.1 == r.1 && q.2 == r.2 && q.3 == r.3)
        });
    }

    /// Remove rects matching the given AR selection-box rects
    /// `(tick_start, tick_end, track_lo, track_hi)`.
    ///
    /// AR 的 rect 在 Selection 中总是 key 全范围 (kl=0, kh=MAX_KEY)，据此匹配避免误伤 PR 的 rect。
    pub fn remove_rects_track(&mut self, rects: &[(u32, u32, u16, u16)]) {
        self.rects.retain(|r| {
            !(r.2 == 0
                && r.3 == MAX_KEY
                && rects
                    .iter()
                    .any(|q| q.0 == r.0 && q.1 == r.1 && q.2 == r.4 && q.3 == r.5))
        });
    }

    /// Compute an order-independent XOR hash of all rects (for GPU cache keys).
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0;
        for &(ts, te, kl, kh, tl, th) in &self.rects {
            h ^= (ts as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (te as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (kl as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (kh as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (tl as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (th as u64).wrapping_mul(0x9e3779b97f4a7c15);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: u32, end: u32, velocity: u8, track: u16) -> Note {
        Note {
            id: 0,
            start_tick: start,
            end_tick: end,
            velocity,
            track,
        }
    }

    #[test]
    fn remove_rects_only_matches_exact_pr_rects() {
        let mut sel = Selection::default();
        // 两个 PR 矩形（key 局部范围）+ 一个 AR 矩形（key 全范围）
        sel.add_rect(0, 100, 60, 70); // PR
        sel.add_rect(200, 300, 40, 50); // PR
        sel.add_rect_track(0, 100, 0, MAX_KEY, 3, 5); // AR

        sel.remove_rects(&[(0, 100, 60, 70)]);

        // 只删掉第一个 PR 矩形，其余保留（用不重叠的 tick/key 验证）
        assert_eq!(sel.rects.len(), 2);
        assert!(!sel.contains(0, 50, 65));
        assert!(sel.contains(0, 250, 45));
        assert!(sel.contains(3, 50, 65));
    }

    #[test]
    fn remove_rects_track_removes_ar_rects_not_pr() {
        let mut sel = Selection::default();
        sel.add_rect(0, 100, 60, 70); // PR：key 局部范围，必须保留
        sel.add_rect_track(0, 100, 0, MAX_KEY, 3, 5); // AR：命中 track 3..=5
        sel.add_rect_track(0, 100, 0, MAX_KEY, 7, 9); // AR：不命中 track，保留

        sel.remove_rects_track(&[(0, 100, 3, 5)]);

        assert_eq!(sel.rects.len(), 2);
        assert!(sel.contains(0, 50, 65)); // PR 矩形不受影响
        assert!(!sel.contains(3, 50, 80)); // 命中的 AR 矩形被删除（key 80 避开 PR 矩形范围）
        assert!(sel.contains(7, 50, 80)); // 未命中的 AR 矩形保留
    }

    #[test]
    fn remove_rects_track_matches_full_tick_range_only() {
        let mut sel = Selection::default();
        sel.add_rect_track(0, 100, 0, MAX_KEY, 0, 0); // tick 范围相同
        sel.add_rect_track(50, 150, 0, MAX_KEY, 0, 0); // tick 不同，保留

        sel.remove_rects_track(&[(0, 100, 0, 0)]);

        assert_eq!(sel.rects.len(), 1);
        assert!(!sel.contains(0, 10, 64)); // tick 10 只属于被删的矩形
        assert!(sel.contains(0, 60, 64)); // tick 60 只属于保留的矩形
    }

    #[test]
    fn empty_filter_is_transparent() {
        let mut sel = Selection::default();
        sel.add_rect(0, 1000, 0, MAX_KEY);
        let n = note(100, 200, 50, 0);
        assert!(sel.filter.is_empty());
        assert!(sel.accepts_note(&n, 60));
    }

    #[test]
    fn velocity_filter_narrows_selection() {
        let mut sel = Selection::default();
        sel.add_rect(0, 1000, 0, MAX_KEY);
        sel.filter.velocity = Some((1, 63));

        assert!(sel.accepts_note(&note(100, 200, 1, 0), 60));
        assert!(sel.accepts_note(&note(100, 200, 63, 0), 60));
        assert!(!sel.accepts_note(&note(100, 200, 64, 0), 60));
        assert!(!sel.accepts_note(&note(100, 200, 127, 0), 60));
        // 矩形外仍然不选
        assert!(!sel.accepts_note(&note(2000, 2100, 10, 0), 60));
    }

    #[test]
    fn gate_filter_uses_note_length() {
        let mut sel = Selection::default();
        sel.add_rect(0, 1000, 0, MAX_KEY);
        sel.filter.gate = Some((100, 200));

        assert!(sel.accepts_note(&note(0, 100, 10, 0), 60)); // gate 100
        assert!(sel.accepts_note(&note(0, 200, 10, 0), 60)); // gate 200
        assert!(!sel.accepts_note(&note(0, 99, 10, 0), 60));
        assert!(!sel.accepts_note(&note(0, 201, 10, 0), 60));
    }

    #[test]
    fn invert_flips_note_bounds_only() {
        let mut sel = Selection::default();
        sel.add_rect(0, 1000, 0, MAX_KEY);
        sel.filter.velocity = Some((1, 63));
        sel.filter.invert = true;

        assert!(!sel.accepts_note(&note(100, 200, 30, 0), 60));
        assert!(sel.accepts_note(&note(100, 200, 100, 0), 60));
        // 没有音符边界时 invert 不生效（避免"全部反选掉"）
        let mut sel2 = Selection::default();
        sel2.add_rect(0, 1000, 0, MAX_KEY);
        sel2.filter.invert = true;
        assert!(sel2.accepts_note(&note(100, 200, 30, 0), 60));
    }

    #[test]
    fn automation_filter_checks_target_and_value() {
        let mut filter = SelectionFilter::default();
        let cc = AutomationTarget::CC { controller: 74 };
        let tempo = AutomationTarget::Tempo;
        assert!(filter.accepts_automation(&cc, 10.0));

        filter.automation_targets = Some(vec![cc.clone()]);
        assert!(filter.accepts_automation(&cc, 10.0));
        assert!(!filter.accepts_automation(&tempo, 120.0));

        filter.automation_value = Some((0.0, 64.0));
        assert!(filter.accepts_automation(&cc, 64.0));
        assert!(!filter.accepts_automation(&cc, 65.0));
    }

    #[test]
    fn clear_resets_filter_but_clear_rects_keeps_it() {
        let mut sel = Selection::default();
        sel.add_rect(0, 100, 0, MAX_KEY);
        sel.filter.velocity = Some((1, 63));

        let mut keep = sel.clone();
        keep.clear_rects();
        assert!(keep.rects.is_empty());
        assert_eq!(keep.filter.velocity, Some((1, 63)));

        sel.clear();
        assert!(sel.rects.is_empty());
        assert!(sel.filter.is_empty());
    }
}
