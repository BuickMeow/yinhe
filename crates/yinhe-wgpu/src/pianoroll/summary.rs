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
/// 摘要档位（tick 块宽），由大到小。覆盖范围：
/// - 超长曲全曲视图（总长上亿 tick）用 262144/65536；
/// - 短而极密的曲子（ReptilianDarkRitual：51 万 tick / 4000 万音符）
///   在 ppu 0.003~0.06 的连续缩放区间内需要 32~256 档才能让块宽 ≤ 2px；
/// - 16/8/4/2 档把 LOD 分界一路推进到 ppu ≤ 1（1px ≥ 1 tick），
///   密曲在较大缩放下也走摘要（块内最多合并 2 tick 的空隙）。
/// - 更细的档位（1 及以下）没有意义：此时原始层每音符本来就有像素级宽度。
pub const SUMMARY_BLOCK_TICKS: [u32; 13] = [
    262144, 65536, 16384, 4096, 1024, 256, 128, 64, 32, 16, 8, 4, 2,
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
        // 超长曲全曲视图：最大档 262144 块 ≈ 2px 才够，否则选小一档。
        assert_eq!(select_summary_level(2.0 / 262144.0), Some(0));
        assert_eq!(select_summary_level(3.0 / 262144.0), Some(1));
        // 短密曲全曲视图（ppu≈2e-3）：4096 档块宽 8px 太大，256 档 0.5px。
        assert_eq!(select_summary_level(2.0e-3), Some(5));
        assert_eq!(select_summary_level(1.0 / 1024.0), Some(4));
        assert_eq!(select_summary_level(3.0 / 1024.0), Some(5));
        // 中等缩放：256 块 > 2px 时依次下探 128 / 32 / 16。
        assert_eq!(select_summary_level(3.0 / 256.0), Some(6));
        assert_eq!(select_summary_level(0.05), Some(8));
        // 16/8/4/2 档：分界一路到 ppu ≤ 1（1px ≥ 1 tick）。
        assert_eq!(select_summary_level(0.1), Some(9));
        assert_eq!(select_summary_level(0.125), Some(9));
        assert_eq!(select_summary_level(0.2), Some(10));
        assert_eq!(select_summary_level(0.5), Some(11));
        assert_eq!(select_summary_level(0.9), Some(12));
        assert_eq!(select_summary_level(1.0), Some(12));
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
