//! Unified selection model for notes.
//!
//! 选区由两部分组成：
//! - `rects`：矩形范围 (tick, key, track)，负责框选输入、选框显示与扫描范围；
//! - `members`：可选的显式成员位图（按 note id）。PR 框选提交时物化，
//!   之后命中判定以成员为准，音符移动/复制后成员身份不变，落点处的
//!   其他音符不会被"顶替"进选区。
//!
//! 矩形态（`members = None`）保留给全选 / AR 轨道选择 / 临时删除指令等
//! 无法（或无需）按 id 物化的场景，判定语义与旧版一致。
//!
//! Memory: 1 亿音符全选（矩形态）= 1 个 rect (~40 bytes)；框选物化后
//! 成员位图 1 bit/音符（1 亿 ≈ 12.5MB），远低于旧的 `HashSet` (800MB)。
//! 属性筛选 `filter` 始终是动态谓词，叠加在矩形/成员判定之上。

use yinhe_types::{AutomationTarget, MAX_KEY, Note, NoteSource};

use crate::NoteBitset;

/// 选区属性边界（作用于选中判定的附加约束）。
///
/// 默认全空 = 只按矩形空间判定（现状）。筛选弹窗写入这里的字段，
/// 之后所有选择操作（拖动/删除/复制/统计）自动按边界过滤。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectionFilter {
    /// 音高范围（含端点，0–127），与选框 key 范围取交集。
    pub key: Option<(u8, u8)>,
    /// 轨道范围（含端点），与选框 track 范围取交集。
    pub track: Option<(u16, u16)>,
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
    /// 只在至少一个音符属性边界存在时生效。
    pub invert: bool,
}

impl SelectionFilter {
    /// 是否未设任何边界（此时判定与无筛选完全一致）。
    pub fn is_empty(&self) -> bool {
        self.key.is_none()
            && self.track.is_none()
            && self.velocity.is_none()
            && self.gate.is_none()
            && self.automation_targets.is_none()
            && self.automation_value.is_none()
            && !self.invert
    }

    /// 是否设置了音符属性边界（key/track/velocity/gate）。
    pub fn has_note_bounds(&self) -> bool {
        self.key.is_some() || self.track.is_some() || self.velocity.is_some() || self.gate.is_some()
    }

    /// 是否设置了自动化边界。
    pub fn has_automation_bounds(&self) -> bool {
        self.automation_targets.is_some() || self.automation_value.is_some()
    }

    /// 音符是否满足属性边界（invert 之前）。
    pub fn note_matches(&self, note: &Note, key: u8) -> bool {
        if let Some((lo, hi)) = self.key
            && !(lo..=hi).contains(&key)
        {
            return false;
        }
        if let Some((lo, hi)) = self.track
            && !(lo..=hi).contains(&note.track)
        {
            return false;
        }
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
    pub fn accepts_note(&self, note: &Note, key: u8) -> bool {
        if !self.has_note_bounds() {
            return true;
        }
        self.note_matches(note, key) != self.invert
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
    /// 显式成员集（按 note id，见 [`Selection::materialize_rect`]）。
    /// `Some` = 成员态：命中以位图为准，`rects` 退化为扫描范围/选框显示；
    /// `None` = 矩形态：全选 / AR 轨道选择 / 临时删除指令等按矩形判定。
    members: Option<NoteBitset>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
        self.filter = SelectionFilter::default();
        self.members = None;
    }

    /// Clear only the rectangles, keeping the attribute filter.
    /// Used by操作式 undo 重建选择集时（filter 属于"筛选器"状态，不随操作回滚）。
    pub fn clear_rects(&mut self) {
        self.rects.clear();
        self.members = None;
    }

    /// 是否处于成员态（框选物化后）。
    pub fn has_explicit_members(&self) -> bool {
        self.members.is_some()
    }

    /// 显式成员数量（矩形态返回 `None`）。
    pub fn explicit_member_count(&self) -> Option<u64> {
        self.members.as_ref().map(NoteBitset::count)
    }

    /// 把一个矩形范围物化为显式成员（PR 框选提交时调用）。
    ///
    /// 只按空间采样（`start_tick` 起点 ∈ 范围 + key + track），不应用属性
    /// 筛选——`filter` 保持动态谓词，由 [`Selection::accepts_note`] 叠加。
    #[allow(clippy::too_many_arguments)] // 矩形坐标透传，见 AGENTS 约定
    pub fn materialize_rect(
        &mut self,
        source: &dyn NoteSource,
        tick_start: u32,
        tick_end: u32,
        key_lo: u8,
        key_hi: u8,
        track_lo: u16,
        track_hi: u16,
    ) {
        let bits = self.members.get_or_insert_with(NoteBitset::default);
        for key in key_lo..=key_hi {
            for n in source.key_notes_in_range(key, tick_start, tick_end) {
                // `key_notes_in_range` 左边界按 max_note_len 保守外扩，需精确过滤。
                if n.start_tick < tick_start || n.start_tick >= tick_end {
                    continue;
                }
                if n.track < track_lo || n.track > track_hi {
                    continue;
                }
                if n.id == 0 {
                    continue; // 发号器哨兵（未分配），不进位图
                }
                bits.insert(n.id);
            }
        }
    }

    /// 用给定 id 重建显式成员集（复制/粘贴后让选区跟随新音符）。
    pub fn set_members(&mut self, ids: impl IntoIterator<Item = u32>) {
        let mut bits = NoteBitset::default();
        for id in ids {
            if id != 0 {
                bits.insert(id);
            }
        }
        self.members = Some(bits);
    }

    /// 丢弃显式成员集，退回矩形态。供"纯几何查询"使用（目标位置重叠
    /// 检测等内部场景），不是用户可见的选择操作。
    pub fn drop_members(&mut self) {
        self.members = None;
    }

    /// 渲染缓存用的选择状态指纹：矩形 + 音符属性边界 + 成员位图。
    /// 只包含影响音符判定的字段（automation 筛选不影响音符渲染）。
    pub fn state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.rects.hash(&mut h);
        self.filter.key.hash(&mut h);
        self.filter.track.hash(&mut h);
        self.filter.velocity.hash(&mut h);
        self.filter.gate.hash(&mut h);
        self.filter.invert.hash(&mut h);
        self.members
            .as_ref()
            .map(NoteBitset::state_hash)
            .hash(&mut h);
        h.finish()
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
    /// 纯矩形判定（成员态也走矩形）——选框绘制/hit-test/Android 等几何
    /// 场景专用。编辑路径请用 [`Selection::accepts_note`]，它优先成员位图。
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

    /// Full note acceptance: 成员态查位图，矩形态查矩形；再叠加属性筛选。
    pub fn accepts_note(&self, note: &Note, key: u8) -> bool {
        let hit = match &self.members {
            Some(bits) => bits.contains(note.id),
            None => self.contains(note.track, note.start_tick, key),
        };
        if !self.filter.has_note_bounds() {
            return hit;
        }
        hit && self.filter.accepts_note(note, key)
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
    ///
    /// 摘除任何 rect 都会整体丢弃成员位图（被摘 rect 对应的成员无法精确
    /// 识别），退回矩形态——保守但不会错删。
    pub fn remove_rects(&mut self, rects: &[(u32, u32, u8, u8)]) {
        let before = self.rects.len();
        self.rects.retain(|r| {
            !rects
                .iter()
                .any(|q| q.0 == r.0 && q.1 == r.1 && q.2 == r.2 && q.3 == r.3)
        });
        if self.rects.len() != before {
            self.members = None;
        }
    }

    /// Remove rects matching the given AR selection-box rects
    /// `(tick_start, tick_end, track_lo, track_hi)`.
    ///
    /// AR 的 rect 在 Selection 中总是 key 全范围 (kl=0, kh=MAX_KEY)，据此匹配避免误伤 PR 的 rect。
    pub fn remove_rects_track(&mut self, rects: &[(u32, u32, u16, u16)]) {
        let before = self.rects.len();
        self.rects.retain(|r| {
            !(r.2 == 0
                && r.3 == MAX_KEY
                && rects
                    .iter()
                    .any(|q| q.0 == r.0 && q.1 == r.1 && q.2 == r.4 && q.3 == r.5))
        });
        if self.rects.len() != before {
            self.members = None;
        }
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

    fn note_id(id: u32, start: u32, end: u32, velocity: u8, track: u16) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: end,
            velocity,
            track,
        }
    }

    /// 测试用音符源：按 key 分桶。
    struct MockSource {
        buckets: Vec<yinhe_types::NoteBucket>,
    }

    impl MockSource {
        fn new(notes: &[(u8, Note)]) -> Self {
            let mut by_key: Vec<Vec<Note>> = (0..128).map(|_| Vec::new()).collect();
            for (key, n) in notes {
                by_key[*key as usize].push(*n);
            }
            Self {
                buckets: by_key
                    .into_iter()
                    .map(yinhe_types::NoteBucket::from_sorted)
                    .collect(),
            }
        }
    }

    impl yinhe_types::NoteSource for MockSource {
        fn key_notes(&self, key: u8) -> &yinhe_types::NoteBucket {
            &self.buckets[key as usize]
        }
        fn duration(&self) -> f64 {
            0.0
        }
    }

    /// 核心回归：框选物化后，选区平移到落点不会把落点处未被选中的音符
    /// 拉进选区（修复"拖动两次拐卖路人"）。
    #[test]
    fn materialized_members_stay_stable_across_move() {
        let n1 = note_id(1, 0, 100, 100, 0);
        let n2 = note_id(2, 300, 400, 100, 0);
        let source = MockSource::new(&[(60, n1), (60, n2)]);

        let mut sel = Selection::default();
        sel.materialize_rect(&source, 0, 100, 60, 60, 0, u16::MAX);
        assert!(sel.has_explicit_members());
        assert_eq!(sel.explicit_member_count(), Some(1));
        assert!(sel.accepts_note(&n1, 60));
        assert!(!sel.accepts_note(&n2, 60));

        // 模拟移动提交：选区矩形平移到落点（+300），n1 也移动到 300..400。
        sel.offset(300, 0);
        let moved_n1 = note_id(1, 300, 400, 100, 0);
        assert!(sel.accepts_note(&moved_n1, 60), "移动后的成员仍被选中");
        assert!(!sel.accepts_note(&n2, 60), "落点处的路人音符不得被选中");
    }

    /// 物化只采样矩形内按起点判定的音符：长音符起点在范围外不入选，
    /// track 范围外的音符不入选。
    #[test]
    fn materialize_rect_respects_tick_and_track_bounds() {
        let long = note_id(1, 0, 500, 100, 0); // 起点在查询范围 [200,300) 之前
        let inside = note_id(2, 250, 260, 100, 0);
        let other_track = note_id(3, 250, 260, 100, 5);
        let source = MockSource::new(&[(60, long), (60, inside), (60, other_track)]);

        let mut sel = Selection::default();
        sel.materialize_rect(&source, 200, 300, 60, 60, 0, 0);

        assert!(!sel.accepts_note(&long, 60), "起点在范围外不入选");
        assert!(sel.accepts_note(&inside, 60));
        assert!(!sel.accepts_note(&other_track, 60), "track 范围外不入选");
        assert_eq!(sel.explicit_member_count(), Some(1));
    }

    /// filter 仍是动态谓词：物化后改筛选只收窄/翻转，不改变成员位图。
    #[test]
    fn filter_stays_dynamic_on_top_of_members() {
        let loud = note_id(1, 0, 100, 100, 0);
        let quiet = note_id(2, 10, 100, 30, 0);
        let source = MockSource::new(&[(60, loud), (60, quiet)]);

        let mut sel = Selection::default();
        sel.materialize_rect(&source, 0, 200, 60, 60, 0, u16::MAX);
        assert_eq!(sel.explicit_member_count(), Some(2));

        sel.filter.velocity = Some((90, 127));
        assert!(sel.accepts_note(&loud, 60));
        assert!(!sel.accepts_note(&quiet, 60));
        assert_eq!(
            sel.explicit_member_count(),
            Some(2),
            "筛选不物化、不收缩成员位图"
        );
    }

    /// 矩形态（未物化）的判定语义与旧版一致。
    #[test]
    fn rect_mode_unchanged_without_members() {
        let mut sel = Selection::default();
        sel.add_rect(100, 200, 60, 60);
        assert!(sel.accepts_note(&note_id(9, 150, 160, 100, 0), 60));
        assert!(!sel.accepts_note(&note_id(9, 250, 260, 100, 0), 60));
    }

    /// 摘除矩形时成员位图整体失效（保守降级，避免残留过期成员）。
    #[test]
    fn remove_rects_drops_members() {
        let n = note_id(1, 0, 100, 100, 0);
        let source = MockSource::new(&[(60, n)]);
        let mut sel = Selection::default();
        sel.add_rect(0, 100, 60, 60);
        sel.materialize_rect(&source, 0, 100, 60, 60, 0, u16::MAX);
        assert!(sel.has_explicit_members());

        sel.remove_rects(&[(0, 100, 60, 60)]);
        assert!(!sel.has_explicit_members());
        assert!(sel.rects.is_empty());
    }

    /// set_members 用新 id 重建成员（复制/粘贴后跟随副本）。
    #[test]
    fn set_members_rebuilds_member_set() {
        let mut sel = Selection::default();
        sel.add_rect(0, 100, 60, 60);
        sel.set_members([10, 20, 20, 0]);
        assert_eq!(sel.explicit_member_count(), Some(2), "重复与哨兵 0 不计入");
        assert!(sel.accepts_note(&note_id(10, 0, 50, 100, 0), 60));
        assert!(
            sel.accepts_note(&note_id(20, 999, 1050, 100, 0), 60),
            "成员态不受矩形限制"
        );
        assert!(!sel.accepts_note(&note_id(30, 0, 50, 100, 0), 60));
    }

    /// 渲染缓存指纹：矩形/成员/筛选变化都会改变。
    #[test]
    fn state_hash_tracks_selection_changes() {
        let n = note_id(1, 0, 100, 100, 0);
        let source = MockSource::new(&[(60, n)]);
        let mut sel = Selection::default();
        let h_empty = sel.state_hash();
        sel.add_rect(0, 100, 60, 60);
        let h_rect = sel.state_hash();
        assert_ne!(h_empty, h_rect);
        sel.materialize_rect(&source, 0, 100, 60, 60, 0, u16::MAX);
        let h_members = sel.state_hash();
        assert_ne!(h_rect, h_members, "物化成员后指纹变化");
        sel.filter.velocity = Some((10, 20));
        assert_ne!(h_members, sel.state_hash(), "筛选变化后指纹变化");
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
