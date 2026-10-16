//! 滚动条音符密度预览的派生数据。
//!
//! 全曲可见音符按 `(key, tick 桶)` 计数：**每个 key 一行**（全分辨率，
//! 只含当前可见轨道的音符）。绘制端按滚动条的实际像素高度把 key 行聚合
//! 成像素行（24px 高 → 24 行 × 5.3 key/行；滚动条加高则自动更细），
//! 每帧绘制成本 O(桶数 × key 行)，与音符数无关——1 亿音符也只是"计数更大"。
//!
//! 增量机制与音频/GPU 上传同构：对比 `note_revisions` 快照找出脏 key，
//! 只重扫脏 key（覆盖整行计数）。`sync` 每帧在预算内处理一部分，
//! UI 线程永不被全量扫描阻塞。轨道显隐变化用 `bucket_track_stats`
//! 定位"含该轨的 key"，同样只重扫受影响的 key。

use std::time::{Duration, Instant};

use yinhe_core::YinModel;

/// 初始桶宽（tick）。
const INITIAL_BUCKET_TICKS: u32 = 1024;
/// 桶数上限（2 的幂保证翻倍不改变对齐）。超出后桶宽翻倍。
const MAX_BUCKETS: usize = 8192;
/// 每帧默认重扫预算：显著低于一帧的渲染开销，编辑期不掉帧。
pub const SYNC_BUDGET: Duration = Duration::from_millis(4);

/// 预览统计的显示域（= PR 显示键数）。扩展音域（key ≥ 128）不统计、
/// 不绘制——与 PR 画布语义一致，同时省一半内存。
pub const PREVIEW_KEYS: usize = 128;

/// 滚动条绘制的只读视图。
pub struct NoteDensityView<'a> {
    /// `PREVIEW_KEYS × n_buckets` 行主序：`rows[key * n_buckets + bucket]`
    /// = 该 key 在该 tick 桶的可见音符数。tick 覆盖
    /// `[0, n_buckets * bucket_ticks)`，之外（滚动条 padding 区）视为 0。
    pub rows: &'a [u32],
    /// 每行的桶数。
    pub n_buckets: usize,
    /// 每桶覆盖的 tick 数。
    pub bucket_ticks: u32,
    /// 单个 (key, 桶) 格的最大计数（亮度归一化基准；绘制端聚合后允许超过）。
    /// 0 = 无数据。
    pub max_count: u32,
}

/// 全曲音符密度（每 key 一行 × tick 桶）。
pub struct NoteDensityMap {
    bucket_ticks: u32,
    n_buckets: usize,
    /// `PREVIEW_KEYS × n_buckets` 行主序计数（已按可见轨道过滤）。
    rows: Vec<u32>,
    /// 每行的最大格计数（维护全局 `max_count` 用，避免全量扫描）。
    row_max: [u32; PREVIEW_KEYS],
    max_count: u32,
    /// 每个 key 上次重扫时的 revision；`None` = 必须重扫（新模型/桶宽变化）。
    /// 只追踪显示域 key（≥ 128 无关预览）。
    rev_snapshot: [Option<u64>; PREVIEW_KEYS],
    visible_snapshot: Vec<bool>,
    model_ptr: usize,
    /// 待重扫 key（配合 `pending_mask` 去重）。
    pending: Vec<u8>,
    pending_mask: [bool; PREVIEW_KEYS],
    /// `rescan_key` 的复用缓冲，避免每次分配。
    scratch: Vec<u32>,
}

impl Default for NoteDensityMap {
    fn default() -> Self {
        Self::new()
    }
}

impl NoteDensityMap {
    pub fn new() -> Self {
        Self {
            bucket_ticks: INITIAL_BUCKET_TICKS,
            n_buckets: 0,
            rows: Vec::new(),
            row_max: [0; PREVIEW_KEYS],
            max_count: 0,
            rev_snapshot: [None; PREVIEW_KEYS],
            visible_snapshot: Vec::new(),
            model_ptr: 0,
            pending: Vec::new(),
            pending_mask: [false; PREVIEW_KEYS],
            scratch: Vec::new(),
        }
    }

    /// 绘制视图（为空/未就绪时 `max_count == 0`）。
    pub fn view(&self) -> NoteDensityView<'_> {
        NoteDensityView {
            rows: &self.rows,
            n_buckets: self.n_buckets,
            bucket_ticks: self.bucket_ticks.max(1),
            max_count: self.max_count,
        }
    }

    /// 每帧调用：在 `budget` 内增量重扫脏 key。至少完成一个 key
    /// （预算再小也不会饿死），其余留到下帧。
    pub fn sync(&mut self, model: &YinModel, track_visible: &[bool], budget: Duration) {
        self.ensure_model(model);
        self.ensure_layout(model.tick_length);
        self.collect_visible_changes(model, track_visible);
        self.collect_revision_changes(model);
        self.consume(model, track_visible, budget);
    }

    // ── 内部 ──

    /// 模型换代 / 桶宽变化：清空计数与快照，全部 key 重扫。
    ///
    /// `rescale_ppq` 就地替换 `doc.data.model` 后必须显式调用（tick 语义
    /// 全变）；其余模型替换由 [`Self::ensure_model`] 的指针检测兜底。
    pub fn reset(&mut self) {
        self.reset_counts();
        self.rev_snapshot = [None; PREVIEW_KEYS];
        self.visible_snapshot.clear();
        self.mark_all();
    }

    /// 模型替换检测（加载/新建/undo 恢复）：指针变化即为新模型。
    /// 指针复用（ABA）概率极低且叠加 revision 对比兜底；rescale 等
    /// 显式替换点直接调用 [`Self::reset`]。
    fn ensure_model(&mut self, model: &YinModel) {
        let ptr = model as *const YinModel as usize;
        if ptr == self.model_ptr {
            return;
        }
        self.model_ptr = ptr;
        self.reset();
    }

    /// 桶宽/桶数维护：曲长超出桶数上限时桶宽翻倍（旧坐标作废 → 全量重扫）；
    /// 桶数按需增长（1.5x 摊还，曲尾连续加音符不频繁重排）。
    fn ensure_layout(&mut self, tick_len: u64) {
        let tick_len = tick_len.max(1);
        let mut bucket = self.bucket_ticks.max(1);
        while tick_len.div_ceil(bucket as u64) > MAX_BUCKETS as u64 {
            bucket *= 2;
        }
        if bucket != self.bucket_ticks {
            self.bucket_ticks = bucket;
            self.reset();
        }
        let need = tick_len.div_ceil(self.bucket_ticks as u64) as usize;
        if need > self.n_buckets {
            self.grow(need);
        }
    }

    /// 行主序数组扩容（补零保留旧数据）。
    fn grow(&mut self, need: usize) {
        let old_n = self.n_buckets;
        let n = need.max(old_n + old_n / 2).max(16);
        let mut new_rows = vec![0u32; PREVIEW_KEYS * n];
        if old_n > 0 {
            for k in 0..PREVIEW_KEYS {
                new_rows[k * n..k * n + old_n]
                    .copy_from_slice(&self.rows[k * old_n..k * old_n + old_n]);
            }
        }
        self.rows = new_rows;
        self.scratch.resize(n, 0);
        self.n_buckets = n;
    }

    fn reset_counts(&mut self) {
        self.rows.fill(0);
        self.row_max = [0; PREVIEW_KEYS];
        self.max_count = 0;
    }

    fn mark(&mut self, key: usize) {
        if key >= PREVIEW_KEYS {
            return;
        }
        if !self.pending_mask[key] {
            self.pending_mask[key] = true;
            self.pending.push(key as u8);
        }
    }

    fn mark_all(&mut self) {
        for k in 0..PREVIEW_KEYS {
            self.mark(k);
        }
    }

    /// 显隐变化：用 `bucket_track_stats` 找出"含该轨的 key"重扫。
    /// 只算变化的 track（通常 1 个）；长度差按新增轨默认可见处理。
    fn collect_visible_changes(&mut self, model: &YinModel, visible: &[bool]) {
        if visible == self.visible_snapshot.as_slice() {
            return;
        }
        let n = visible.len().max(self.visible_snapshot.len());
        for t in 0..n {
            let old = self.visible_snapshot.get(t).copied().unwrap_or(true);
            let new = visible.get(t).copied().unwrap_or(true);
            if old == new {
                continue;
            }
            let track = t as u16;
            for k in 0..PREVIEW_KEYS {
                if model.bucket_track_stats[k].contains_key(&track) {
                    self.mark(k);
                }
            }
        }
        self.visible_snapshot = visible.to_vec();
    }

    /// revision 变化（编辑/undo/批量，由 note_write 统一 bump）→ 标脏。
    /// 分帧期间已扫过的 key 快照已更新，不会重复入队。
    fn collect_revision_changes(&mut self, model: &YinModel) {
        for k in 0..PREVIEW_KEYS {
            if self.rev_snapshot[k] != Some(model.note_revisions[k]) {
                self.mark(k);
            }
        }
    }

    /// 预算内消费待重扫队列。
    fn consume(&mut self, model: &YinModel, visible: &[bool], budget: Duration) {
        if self.pending.is_empty() {
            return;
        }
        let start = Instant::now();
        let mut rescanned = false;
        while let Some(k) = self.pending.pop() {
            self.pending_mask[k as usize] = false;
            self.rescan_key(model, visible, k as usize);
            rescanned = true;
            if self.pending.is_empty() {
                break;
            }
            // 至少完成一个 key；超出预算留到下帧。
            if start.elapsed() >= budget {
                break;
            }
        }
        if rescanned {
            // 全局最大 = 各行最大值的最大（O(key 行)，无需全量扫描）。
            self.max_count = self.row_max.iter().copied().max().unwrap_or(0);
        }
    }

    /// 重扫一个 key：统计可见音符到 scratch，覆盖该行并更新行最大。
    fn rescan_key(&mut self, model: &YinModel, visible: &[bool], key: usize) {
        let n = self.n_buckets;
        if n == 0 || key >= PREVIEW_KEYS {
            return;
        }
        let bucket = self.bucket_ticks.max(1);
        self.scratch.clear();
        self.scratch.resize(n, 0);
        for note in model.notes[key].iter() {
            if !visible.get(note.track as usize).copied().unwrap_or(true) {
                continue;
            }
            let b = (note.start_tick / bucket) as usize;
            if b < n {
                self.scratch[b] += 1;
            }
        }
        let row = &mut self.rows[key * n..(key + 1) * n];
        let mut row_peak = 0u32;
        for (slot, &new) in row.iter_mut().zip(self.scratch.iter()) {
            *slot = new;
            if new > row_peak {
                row_peak = new;
            }
        }
        self.row_max[key] = row_peak;
        self.rev_snapshot[key] = Some(model.note_revisions[key]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::Note;

    fn note(id: u32, start: u32, end: u32, track: u16) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: end,
            velocity: 100,
            track,
        }
    }

    fn model() -> YinModel {
        let mut m = YinModel::new_empty_with_16_tracks();
        m.rebuild();
        m
    }

    fn visible() -> Vec<bool> {
        vec![true; 16]
    }

    /// 全量预算：一次 sync 完成所有待扫 key。
    const FULL: Duration = Duration::MAX;

    /// 视图里 (key, 桶) 的计数。
    fn cell(d: &NoteDensityMap, key: usize, bucket: usize) -> u32 {
        let v = d.view();
        v.rows[key * v.n_buckets + bucket]
    }

    #[test]
    fn counts_notes_per_key_and_bucket() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200, 0));
        m.insert_note(60, note(2, 3000, 3100, 0));
        m.insert_note(30, note(3, 100, 200, 0)); // 低音 bass
        m.rebuild_dirty();
        let mut d = NoteDensityMap::new();
        d.sync(&m, &visible(), FULL);
        let v = d.view();
        assert_eq!(v.bucket_ticks, INITIAL_BUCKET_TICKS);
        // tick 100 → 桶 0；tick 3000 → 桶 2。
        assert_eq!(cell(&d, 60, 0), 1);
        assert_eq!(cell(&d, 60, 2), 1);
        assert_eq!(cell(&d, 30, 0), 1, "低音有独立的 key 行");
        assert_eq!(v.max_count, 1);
        let sum: u32 = v.rows.iter().sum();
        assert_eq!(sum, 3);
    }

    /// 编辑后只重扫脏 key，其他 key 的行不受影响。
    #[test]
    fn incremental_rescan_updates_dirty_key_only() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200, 0));
        m.rebuild_dirty();
        let mut d = NoteDensityMap::new();
        d.sync(&m, &visible(), FULL);
        assert_eq!(cell(&d, 60, 0), 1);

        m.insert_note(64, note(2, 100, 200, 0));
        m.rebuild_dirty();
        d.sync(&m, &visible(), FULL);
        assert_eq!(cell(&d, 60, 0), 1, "未动的 key 行保持不变");
        assert_eq!(cell(&d, 64, 0), 1, "新 key 计入自己的行");
        assert_eq!(d.view().max_count, 1);
    }

    /// 删除音符后计数回落。
    #[test]
    fn removal_decrements_counts() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200, 0));
        m.insert_note(60, note(2, 100, 200, 0));
        m.rebuild_dirty();
        let mut d = NoteDensityMap::new();
        d.sync(&m, &visible(), FULL);
        assert_eq!(cell(&d, 60, 0), 2);
        assert_eq!(d.view().max_count, 2);

        m.remove_note_by_id(60, 1);
        m.rebuild_dirty();
        d.sync(&m, &visible(), FULL);
        assert_eq!(cell(&d, 60, 0), 1);
        assert_eq!(d.view().max_count, 1);
    }

    /// 显隐切换：不可见轨的音符从计数消失，恢复后回来。
    #[test]
    fn visibility_filtering_round_trip() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200, 0));
        m.insert_note(62, note(2, 100, 200, 1));
        m.rebuild_dirty();
        let mut d = NoteDensityMap::new();
        let mut vis = visible();
        d.sync(&m, &vis, FULL);
        assert_eq!(cell(&d, 60, 0), 1);
        assert_eq!(cell(&d, 62, 0), 1);

        vis[1] = false;
        d.sync(&m, &vis, FULL);
        assert_eq!(cell(&d, 60, 0), 1);
        assert_eq!(cell(&d, 62, 0), 0, "隐藏 track 1 后其音符消失");

        vis[1] = true;
        d.sync(&m, &vis, FULL);
        assert_eq!(cell(&d, 62, 0), 1, "恢复显隐后计数回来");
    }

    /// 曲长增长：桶数自动扩容且旧计数保留。
    #[test]
    fn bucket_growth_preserves_counts() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200, 0));
        m.rebuild_dirty();
        let mut d = NoteDensityMap::new();
        d.sync(&m, &visible(), FULL);
        let n_before = d.n_buckets;
        assert!(n_before >= 16);

        // tick_length 增长远超当前桶数（1.5x 摊还会额外扩），旧桶数据必须保留。
        m.insert_note(60, note(2, 4_000_000, 4_000_100, 0));
        m.rebuild_dirty();
        d.sync(&m, &visible(), FULL);
        assert!(d.n_buckets > n_before, "桶数应增长");
        let v = d.view();
        assert_eq!(cell(&d, 60, 0), 1, "旧音符仍在桶 0");
        let b = 4_000_000 / v.bucket_ticks as u64;
        assert_eq!(cell(&d, 60, b as usize), 1, "新音符落在其桶");
        let sum: u32 = v.rows.iter().sum();
        assert_eq!(sum, 2);
    }

    /// 曲长超出桶数上限 → 桶宽翻倍（保持对齐），计数仍正确。
    #[test]
    fn bucket_width_doubles_beyond_max_buckets() {
        let mut m = model();
        let far = (MAX_BUCKETS as u64 * INITIAL_BUCKET_TICKS as u64 + 10_000) as u32;
        m.insert_note(60, note(1, 100, 200, 0));
        m.insert_note(60, note(2, far, far + 100, 0));
        m.rebuild_dirty();
        let mut d = NoteDensityMap::new();
        d.sync(&m, &visible(), FULL);
        let v = d.view();
        assert_eq!(v.bucket_ticks, INITIAL_BUCKET_TICKS * 2, "桶宽应翻倍");
        assert!(v.n_buckets <= MAX_BUCKETS, "桶数不超上限");
        assert_eq!(cell(&d, 60, 0), 1);
        let b = far as u64 / v.bucket_ticks as u64;
        assert_eq!(cell(&d, 60, b as usize), 1);
    }

    /// 预算再小也至少完成一个 key，多次 sync 后结果与全量一致。
    #[test]
    fn budget_splits_work_across_syncs() {
        let mut m = model();
        for k in [50u8, 55, 60, 64, 70, 80] {
            m.insert_note(k, note(k as u32, 100, 200, 0));
        }
        m.rebuild_dirty();

        let mut split = NoteDensityMap::new();
        let zero = Duration::ZERO;
        let mut frames = 0;
        loop {
            split.sync(&m, &visible(), zero);
            frames += 1;
            if split.pending.is_empty() {
                break;
            }
            assert!(frames < 300, "分帧应有限步完成");
        }
        assert!(frames >= 2, "零预算应分成多帧（实际 {frames}）");

        let mut full = NoteDensityMap::new();
        full.sync(&m, &visible(), FULL);
        assert_eq!(split.view().rows, full.view().rows);
    }

    /// 模型替换（加载/新建）：旧计数作废，全量重扫新模型。
    #[test]
    fn model_swap_resets_counts() {
        let mut m1 = model();
        m1.insert_note(60, note(1, 100, 200, 0));
        m1.rebuild_dirty();
        let mut d = NoteDensityMap::new();
        d.sync(&m1, &visible(), FULL);
        assert_eq!(cell(&d, 60, 0), 1);

        let mut m2 = model();
        m2.insert_note(64, note(9, 5000, 5100, 0));
        m2.insert_note(64, note(10, 5000, 5100, 0));
        m2.rebuild_dirty();
        d.sync(&m2, &visible(), FULL);
        assert_eq!(cell(&d, 60, 0), 0, "旧模型的计数不得残留");
        assert_eq!(cell(&d, 64, 4), 2, "tick 5000 → 桶 4");
        assert_eq!(d.view().max_count, 2);
    }

    /// 扩展音域（key ≥ 128）不进预览：不统计、也不占用重扫预算。
    #[test]
    fn extended_key_range_excluded() {
        let mut m = model();
        m.insert_note(60, note(1, 100, 200, 0));
        let mut d = NoteDensityMap::new();
        d.sync(&m, &visible(), FULL);
        assert_eq!(cell(&d, 60, 0), 1);

        m.insert_note(130, note(2, 100, 200, 0));
        m.rebuild_dirty();
        d.sync(&m, &visible(), FULL);
        let v = d.view();
        assert_eq!(v.max_count, 1, "扩展音域不参与归一化");
        let sum: u32 = v.rows.iter().sum();
        assert_eq!(sum, 1);
    }
}
