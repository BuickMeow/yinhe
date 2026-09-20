//! LOD 摘要层：把大量音符按 tick 块聚合成少量矩形，供小 ppu（全曲/缩小）
//! 时渲染，避免每帧绘制上千万实例。
//!
//! 语义：每 (key, block) 输出一个段 `[min_start, max_end)`，颜色取块内
//! 出现次数最多的 track（主导 track）。块宽在屏幕上 ≤ `SUMMARY_MAX_PX`
//! 时块内空隙不可见，因此视觉上等价于「把小音符合并到块粒度」。
//!
//! 档位（block_ticks）与选择规则都是连续公式，无行为阈值：
//! 从大到小取第一个满足 `block_ticks * ppu <= SUMMARY_MAX_PX` 的档位；
//! 都不满足则用原始音符层（此时可见音符数本来就不大）。

use std::collections::HashMap;

use rayon::prelude::*;
use yinhe_types::KEY_COUNT;

use crate::vertex::NoteInstance;
/// 摘要档位（tick 块宽），由大到小，取 2 的幂的完整序列。
///
/// 完整序列让任意 ppu 下选中的档位块宽都落在 (1, 2] px，缩放过程档位
/// 切换平滑（相邻档只差 2 倍）；缺档会让某些 ppu 区间的块宽掉到 0.5px
/// 或被迫用更粗的档。
///
/// 覆盖范围：
/// - 超长曲全曲视图（总长上亿 tick）用 262144/131072/65536；
/// - 短而极密的曲子（ReptilianDarkRitual：51 万 tick / 4000 万音符）
///   从全曲到 1px=1tick 的连续缩放区间需要 2~256 全部档位；
/// - 1 及以下没有意义：ppu > 2 时原始层每音符本来就有像素级宽度。
pub const SUMMARY_BLOCK_TICKS: [u32; 18] = [
    262144, 131072, 65536, 32768, 16384, 8192, 4096, 2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4, 2,
];
/// 摘要块在屏幕上的最大像素宽。块内空隙 ≤ 该宽度时被合并不可见。
pub const SUMMARY_MAX_PX: f32 = 2.0;

/// 单个档位的段数上限（约 192MB/档）。超过则该档不构建（选择时向更细档
/// 或原始层回退）。段数与总 tick 成正比、与音符数无关：4/2 这类细档只对
/// 「短而极密」的曲子有意义；长曲上细档段数可达上亿，必须设上限。
pub const SUMMARY_MAX_SEGMENTS: usize = 16_000_000;

/// 根据 ppu 选择摘要档位索引（`None` = 用原始音符层）。
///
/// 取满足 `block * ppu <= SUMMARY_MAX_PX` 的最大 block（列表从大到小，
/// 第一个满足即最大）；无满足项时返回 None。
pub fn select_summary_level(ppu: f32) -> Option<usize> {
    if !ppu.is_finite() || ppu <= 0.0 {
        return None;
    }
    SUMMARY_BLOCK_TICKS
        .iter()
        .position(|&block| block as f32 * ppu <= SUMMARY_MAX_PX)
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
///
/// 总段数超过 `SUMMARY_MAX_SEGMENTS` 时返回空（该档不构建，渲染回退）。
/// 累计计数在并行构建中检查，超限后剩余 key 直接跳过（早停）。
pub fn build_summary(
    notes: &[NoteInstance],
    offsets: &[u32; KEY_COUNT + 1],
    block_ticks: u32,
) -> (Vec<NoteInstance>, [u32; KEY_COUNT + 1]) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let total = AtomicUsize::new(0);
    let buckets: Vec<Vec<NoteInstance>> = (0..KEY_COUNT)
        .into_par_iter()
        .map(|key| {
            if total.load(Ordering::Relaxed) > SUMMARY_MAX_SEGMENTS {
                return Vec::new();
            }
            let start = offsets[key] as usize;
            let end = offsets[key + 1] as usize;
            let bucket = build_key_summary(key as u8, &notes[start..end], block_ticks);
            total.fetch_add(bucket.len(), Ordering::Relaxed);
            bucket
        })
        .collect();

    if total.load(Ordering::Relaxed) > SUMMARY_MAX_SEGMENTS {
        return (Vec::new(), [0u32; KEY_COUNT + 1]);
    }

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
        .map(|&block| {
            let (summary, summary_offsets) = build_summary(notes, offsets, block);
            if summary.len() > SUMMARY_MAX_SEGMENTS {
                (Vec::new(), [0u32; KEY_COUNT + 1])
            } else {
                (summary, summary_offsets)
            }
        })
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
        // 完整 2 的幂序列：任意 ppu 下选中档的块宽 ∈ (1, 2] px。
        assert_eq!(select_summary_level(2.0 / 262144.0), Some(0));
        assert_eq!(select_summary_level(3.0 / 262144.0), Some(1));
        assert_eq!(select_summary_level(1.0 / 1024.0), Some(7)); // 2048 档
        assert_eq!(select_summary_level(2.0e-3), Some(9)); // 512 档
        assert_eq!(select_summary_level(3.0 / 1024.0), Some(9));
        assert_eq!(select_summary_level(3.0 / 256.0), Some(11)); // 128 档
        assert_eq!(select_summary_level(0.05), Some(13)); // 32 档
        assert_eq!(select_summary_level(0.1), Some(14)); // 16 档
        assert_eq!(select_summary_level(0.125), Some(14));
        assert_eq!(select_summary_level(0.2), Some(15)); // 8 档
        assert_eq!(select_summary_level(0.5), Some(16)); // 4 档
        assert_eq!(select_summary_level(0.9), Some(17)); // 2 档
        assert_eq!(select_summary_level(1.0), Some(17));
        // 1px < 1 tick（ppu > 2）才回原始层。
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
