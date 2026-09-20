//! LOD 摘要层：把大量音符按 tick 块聚合成少量矩形，供小 ppu（全曲/缩小）
//! 时渲染，避免每帧绘制上千万实例。
//!
//! 语义：每 (key, block) 输出一个段 `[min_start, max_end)`，颜色取块内
//! 出现次数最多的 track（主导 track）。块宽在屏幕上 ≤ `SUMMARY_MAX_PX`
//! 时块内空隙不可见，因此视觉上等价于「把小音符合并到块粒度」。
//!
//! 档位（block_ticks）与选择规则都是连续公式，无行为阈值：
//! 从大到小取第一个满足 `block_ticks * ppu <= SUMMARY_MAX_PX` 的档位；
//! 都不满足时，最细档的块宽若仍不超过 `SUMMARY_TAIL_MAX_PX`（2 倍上限），
//! 由它顶替更细档的位置（块宽 2~4px，极端放大时才发生）；再细则用原始
//! 音符层（此时可见音符数本来就不大）。

use std::collections::HashMap;

use rayon::prelude::*;
use yinhe_types::KEY_COUNT;

use crate::vertex::NoteInstance;
/// 摘要档位（tick 块宽），由大到小，取 2 的幂，最细 16。
///
/// 范围依据：
/// - 最粗 1024：ppu 最小值 0.001（view 缩放 clamp）下选择条件
///   `block × ppu ≤ SUMMARY_MAX_PX` 推出可达最大 block = 2000，1024 已够；
/// - 最细 16：更细的档（8/4/2）段数会趋近音符数（摘要退化成原始数据），
///   且它们生效的缩放区间原始层本来只有几十万可见音符（几 ms），
///   收益为零还吃显存，因此删除；16 档可再向下顶替一档（见
///   `SUMMARY_TAIL_MAX_PX`）。
///
/// 不设段数上限：段数超限的档会被静默跳过，导致缩放时「细档凭空消失」
/// （32 直接跳原始层）。显存安全由 `GpuBudget` 兜底（上传失败即清空该档
/// 并回退到更粗档/原始层）。
pub const SUMMARY_BLOCK_TICKS: [u32; 7] = [1024, 512, 256, 128, 64, 32, 16];
/// 摘要块在屏幕上的最大像素宽。块内空隙 ≤ 该宽度时被合并不可见。
pub const SUMMARY_MAX_PX: f32 = 2.0;
/// 最细档的兜底块宽上限：没有档位满足 `SUMMARY_MAX_PX` 时，允许最细档
/// 放大到该宽度继续顶替更细的档（对应原 8/4 档的位置），避免直接掉回
/// 原始层（可见音符多一个数量级）。再细则回原始层。
pub const SUMMARY_TAIL_MAX_PX: f32 = SUMMARY_MAX_PX * 2.0;

/// 根据 ppu 选择摘要档位索引（`None` = 用原始音符层）。
///
/// 取满足 `block * ppu <= SUMMARY_MAX_PX` 的最大 block（列表从大到小，
/// 第一个满足即最大）；无满足项时由最细档兜底（块宽 ≤ `SUMMARY_TAIL_MAX_PX`
/// 时继续用），否则返回 None。
pub fn select_summary_level(ppu: f32) -> Option<usize> {
    if !ppu.is_finite() || ppu <= 0.0 {
        return None;
    }
    if let Some(level) = SUMMARY_BLOCK_TICKS
        .iter()
        .position(|&block| block as f32 * ppu <= SUMMARY_MAX_PX)
    {
        return Some(level);
    }
    let finest = SUMMARY_BLOCK_TICKS.len() - 1;
    let block = SUMMARY_BLOCK_TICKS[finest] as f32;
    (block * ppu <= SUMMARY_TAIL_MAX_PX).then_some(finest)
}

/// 块内 track 计数取主导（次数最多；并列取 track 索引最小）。
fn dominant_track(counts: &HashMap<u16, u32>) -> u16 {
    let mut best = (u16::MAX, 0u32);
    for (&track, &n) in counts {
        if n > best.1 || (n == best.1 && track < best.0) {
            best = (track, n);
        }
    }
    best.0
}

/// 把单个 key 的音符（按 start_tick 升序）聚合为摘要段。
///
/// 输出同样按 start_tick 升序：块按 tick 递增，块内取 min_start 作为段起点，
/// 因此段起点随块严格递增。
pub fn build_key_summary(key: u8, notes: &[NoteInstance], block_ticks: u32) -> Vec<NoteInstance> {
    if notes.is_empty() {
        return Vec::new();
    }
    let block_ticks = block_ticks.max(1);
    let mut out = Vec::new();
    let mut counts: HashMap<u16, u32> = HashMap::new();
    let mut i = 0;
    while i < notes.len() {
        let block = notes[i].start_tick / block_ticks;
        let mut min_s = notes[i].start_tick;
        let mut max_e = notes[i].end_tick;
        counts.clear();
        while i < notes.len() && notes[i].start_tick / block_ticks == block {
            let note = &notes[i];
            min_s = min_s.min(note.start_tick);
            max_e = max_e.max(note.end_tick);
            *counts
                .entry(((note.packed >> 8) & 0xFFFF) as u16)
                .or_insert(0) += 1;
            i += 1;
        }
        out.push(NoteInstance {
            start_tick: min_s,
            end_tick: max_e,
            packed: NoteInstance::pack(key, dominant_track(&counts), 100),
        });
    }
    out
}

/// 全量构建：`offsets` 把 `notes` 切成 per-key 段，逐 key 聚合。
pub fn build_summary(
    notes: &[NoteInstance],
    offsets: &[u32; KEY_COUNT + 1],
    block_ticks: u32,
) -> (Vec<NoteInstance>, [u32; KEY_COUNT + 1]) {
    let buckets: Vec<Vec<NoteInstance>> = (0..KEY_COUNT)
        .into_par_iter()
        .map(|key| {
            let start = offsets[key] as usize;
            let end = offsets[key + 1] as usize;
            build_key_summary(key as u8, &notes[start..end], block_ticks)
        })
        .collect();

    let mut summary_offsets = [0u32; KEY_COUNT + 1];
    let mut summary = Vec::new();
    let mut total = 0u32;
    for (key, bucket) in buckets.into_iter().enumerate() {
        summary_offsets[key] = total;
        total += bucket.len() as u32;
        summary.extend(bucket);
    }
    summary_offsets[KEY_COUNT] = total;
    (summary, summary_offsets)
}

/// 按 `SUMMARY_BLOCK_TICKS` 全档位构建。
///
/// 档间串行、档内按 key 并行（`build_summary` 内部）：实测嵌套并行
/// （档间也 rayon）会超订线程池，反而比串行档间慢 ~30%。
pub fn build_summaries(
    notes: &[NoteInstance],
    offsets: &[u32; KEY_COUNT + 1],
) -> Vec<(Vec<NoteInstance>, [u32; KEY_COUNT + 1])> {
    SUMMARY_BLOCK_TICKS
        .iter()
        .map(|&block| build_summary(notes, offsets, block))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(key: u8, track: u16, start: u32, end: u32) -> NoteInstance {
        NoteInstance {
            start_tick: start,
            end_tick: end,
            packed: NoteInstance::pack(key, track, 100),
        }
    }

    #[test]
    fn select_level_by_pixels() {
        // 2 的幂序列（1024..16）：可达 ppu 下选中档的块宽 ∈ (1, 2] px。
        assert_eq!(select_summary_level(2.0 / 1024.0), Some(0));
        assert_eq!(select_summary_level(1.0 / 1024.0), Some(0));
        assert_eq!(select_summary_level(2.0e-3), Some(1)); // 512 档
        assert_eq!(select_summary_level(3.0 / 1024.0), Some(1));
        assert_eq!(select_summary_level(3.0 / 256.0), Some(3)); // 128 档
        assert_eq!(select_summary_level(0.05), Some(5)); // 32 档
        assert_eq!(select_summary_level(0.1), Some(6)); // 16 档
        assert_eq!(select_summary_level(0.125), Some(6));
        // 16 块 > 2px（ppu > 0.125）→ 最细档顶替（块宽 ≤ 4px，X ≥ 4）。
        assert_eq!(select_summary_level(0.2), Some(6));
        assert_eq!(select_summary_level(0.25), Some(6)); // X=4，块宽正好 4px
        // 16 块 > 4px（ppu > 0.25）→ 原始层（更细档已移除）。
        assert_eq!(select_summary_level(0.26), None);
        assert_eq!(select_summary_level(0.5), None);
        assert_eq!(select_summary_level(1.0), None);
        assert_eq!(select_summary_level(2.5), None);
        assert_eq!(select_summary_level(0.0), None);
    }

    #[test]
    fn merge_same_block_and_pick_min_max() {
        let notes = vec![
            note(60, 0, 10, 20),
            note(60, 1, 30, 40),     // 同一 4096 块
            note(60, 0, 5000, 5010), // 下一块
        ];
        let out = build_key_summary(60, &notes, 4096);
        assert_eq!(out.len(), 2, "两个块各一段");
        assert_eq!((out[0].start_tick, out[0].end_tick), (10, 40));
        assert_eq!((out[1].start_tick, out[1].end_tick), (5000, 5010));
        assert!(out[0].start_tick < out[1].start_tick, "输出按 tick 升序");
    }

    #[test]
    fn dominant_track_wins() {
        // 同块：track 2 出现两次，track 1 一次 → 取 track 2。
        let notes = vec![note(60, 1, 0, 10), note(60, 2, 20, 30), note(60, 2, 40, 50)];
        let out = build_key_summary(60, &notes, 4096);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].packed >> 8) & 0xFFFF, 2);
    }

    #[test]
    fn dominant_track_tie_takes_smallest_index() {
        let notes = vec![note(60, 5, 0, 10), note(60, 3, 20, 30)];
        let out = build_key_summary(60, &notes, 4096);
        assert_eq!((out[0].packed >> 8) & 0xFFFF, 3);
    }

    #[test]
    fn empty_key_stays_empty() {
        assert!(build_key_summary(0, &[], 4096).is_empty());
    }

    #[test]
    fn full_build_keeps_per_key_offsets() {
        let notes = vec![note(60, 0, 0, 10), note(62, 1, 0, 10)];
        let mut offsets = [0u32; KEY_COUNT + 1];
        offsets[60] = 0;
        offsets[61] = 1;
        offsets[62] = 1;
        offsets[63] = 2;
        for v in offsets.iter_mut().skip(63) {
            *v = 2;
        }
        let (summary, summary_offsets) = build_summary(&notes, &offsets, 4096);
        assert_eq!(summary.len(), 2);
        assert_eq!(summary_offsets[60], 0);
        assert_eq!(summary_offsets[61], 1);
        assert_eq!(summary_offsets[62], 1);
        assert_eq!(summary_offsets[63], 2);
    }
}
