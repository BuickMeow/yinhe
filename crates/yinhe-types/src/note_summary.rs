//! 低倍率音符摘要（LOD）的共享类型。
//!
//! 摘要按 `(key, track, start_tick / block_ticks)` 聚合，只保留每段的
//! `[min_start, max_end]` 与最大力度。它不是视口缓存：不依赖滚动位置，
//! 只依赖模型内容和块宽；换缩放档或编辑时才重建。

/// 一个 `(key, track, tick_block)` 摘要段。
///
/// `block` 由 `start_tick / block_ticks` 得到；段内 `start_tick` 取块内
/// 最小起点，`end_tick` 取块内最大终点，因此长音符会跨越后续块显示，
/// 与原始层视觉一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteSummary {
    pub block: u32,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    pub track: u16,
    /// 段内最大力度（与原始层合并规则一致）。
    pub velocity: u8,
}

/// 单个 key 的摘要层。
#[derive(Clone, Debug, Default)]
pub struct KeySummary {
    /// 按 `(block, track)` 升序；同一 track 内按 block 升序。
    pub segments: Vec<NoteSummary>,
    /// 该 key 内最长音符（tick）。范围查询用它把左边界外扩到可能
    /// 跨越视口起点的块，避免漏掉长音符。
    pub max_note_len: u32,
}

impl KeySummary {
    /// 可见 tick 范围内第一个可能相交的段下标。
    ///
    /// `max_note_len` 让左边界从“可见起点所在块”外扩到更早的块；
    /// 长音符只影响自己所在的段扫描，不需要回看整曲。
    pub fn first_segment(&self, tick_start: u32, block_ticks: u32) -> usize {
        let block_ticks = block_ticks.max(1);
        let lo_tick = tick_start.saturating_sub(self.max_note_len);
        let lo_block = lo_tick / block_ticks;
        self.segments.partition_point(|s| s.block < lo_block)
    }
}
