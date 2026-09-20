//! AM 力度条的 LOD 摘要：按 tick 块聚合「块内最大 velocity + 代表 track」。
//!
//! 缩小时同一条像素列有海量力度条，逐音符构建（读 + 全局排序 + 去重）会到
//! 数百 ms/帧（ReptilianDarkRitual 全曲 1.6s/帧、常规滚动视口 434ms/帧）。
//! 摘要把数据量压到「块数」（与音符数无关）：缩小时直接从摘要切片生成 bar，
//! 每块最多一条（取块内最大 velocity，保证强音可见）。
//!
//! 档位复用 `pianoroll` 的 `SUMMARY_BLOCK_TICKS` / `SUMMARY_MAX_PX`：
//! 块宽 ≤ 2px 时启用；更细的档位由 `SUMMARY_MAX_SEGMENTS` 上限保护。

use rayon::prelude::*;
use yinhe_types::{MAX_KEY, NoteSource};

use crate::pianoroll::{SUMMARY_BLOCK_TICKS, SUMMARY_MAX_SEGMENTS, select_summary_level};

/// 摘要中的一个块（tick 升序存放）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VelocityBlock {
    /// 块起点 tick（= block_index * block_ticks）。
    pub start_tick: u32,
    /// 块内最大 velocity（构建时已过滤 ≤1）。
    pub velocity: u8,
    /// 取到最大 velocity 的代表 track。
    pub track: u16,
}

/// 力度条摘要：与 `SUMMARY_BLOCK_TICKS` 对齐的每档块列表。
pub struct VelocitySummary {
    levels: Vec<Vec<VelocityBlock>>,
}

impl VelocitySummary {
    /// 遍历全量音符构建所有档位。过滤规则与 `build_velocity_bars` 一致：
    /// track_visible 与 velocity > 1。
    ///
    /// 实现：每档一个并行任务，串行扫描 128 个 key 的音符，用普通数组
    /// 记录每块最大 `(vel << 16) | track`（vel 优先），最后转稀疏列表。
    /// 9 档并行，总墙钟约为单遍扫描的时间。
    pub fn build(midi: &dyn NoteSource, track_visible: &[bool]) -> Self {
        let total_ticks = midi
            .tick_length()
            .unwrap_or(0)
            .clamp(1, u64::from(u32::MAX)) as u32;
        let levels: Vec<Vec<VelocityBlock>> = SUMMARY_BLOCK_TICKS
            .par_iter()
            .map(|&block_ticks| {
                let nblocks = (total_ticks / block_ticks + 1) as usize;
                let mut best = vec![0u32; nblocks];
                for key in 0u8..=MAX_KEY {
                    for note in midi.key_notes(key).iter() {
                        if note.velocity <= 1 {
                            continue;
                        }
                        if !track_visible
                            .get(note.track as usize)
                            .copied()
                            .unwrap_or(true)
                        {
                            continue;
                        }
                        let idx = (note.start_tick / block_ticks) as usize;
                        let packed = (u32::from(note.velocity) << 16) | u32::from(note.track);
                        if packed > best[idx] {
                            best[idx] = packed;
                        }
                    }
                }
                let mut out = Vec::new();
                for (i, packed) in best.into_iter().enumerate() {
                    if packed != 0 {
                        out.push(VelocityBlock {
                            start_tick: i as u32 * block_ticks,
                            velocity: (packed >> 16) as u8,
                            track: packed as u16,
                        });
                    }
                }
                // 段数超上限：该档不构建（渲染向更细档/原始层回退）。
                if out.len() > SUMMARY_MAX_SEGMENTS {
                    Vec::new()
                } else {
                    out
                }
            })
            .collect();
        Self { levels }
    }

    /// 按 ppu 选档（块宽 ≤ `SUMMARY_MAX_PX` 的最大档；未构建的档返回 None，
    /// 由调用方向更细档/原始路径回退）。
    pub fn level_for_ppu(&self, ppu: f32) -> Option<usize> {
        let best = select_summary_level(ppu)?;
        (best..self.levels.len()).find(|&level| !self.levels[level].is_empty())
    }

    pub fn level(&self, level: usize) -> &[VelocityBlock] {
        self.levels.get(level).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{Note, NoteBucket, NoteSource};

    /// 全部音符放 key 60（力度条摘要不分 key）。
    struct MockSource {
        notes: NoteBucket,
    }

    impl NoteSource for MockSource {
        fn key_notes(&self, key: u8) -> &NoteBucket {
            if key == 60 {
                &self.notes
            } else {
                static EMPTY: std::sync::LazyLock<NoteBucket> =
                    std::sync::LazyLock::new(NoteBucket::default);
                &EMPTY
            }
        }

        fn duration(&self) -> f64 {
            10.0
        }

        fn tick_length(&self) -> Option<u64> {
            Some(100_000)
        }
    }

    fn make(tick: u32, vel: u8, track: u16) -> Note {
        Note {
            id: 0,
            start_tick: tick,
            end_tick: tick + 10,
            velocity: vel,
            track,
        }
    }

    fn build(notes: Vec<Note>) -> VelocitySummary {
        let mut notes = notes;
        notes.sort_by_key(|n| n.start_tick);
        VelocitySummary::build(
            &MockSource {
                notes: NoteBucket::from_sorted(notes),
            },
            &[true; 4],
        )
    }

    /// 输出必须跨 key 聚合且按 tick 升序（块索引升序）。
    #[test]
    fn blocks_are_sorted_and_take_max_velocity() {
        let s = build(vec![
            make(0, 50, 0),
            make(100, 90, 1), // 同一 block 内（ppu=0.003 → block=256）
            make(300, 70, 2), // 下一块
        ]);
        let level = s.level_for_ppu(0.003).expect("应触发摘要档");
        let blocks = s.level(level);
        let block_ticks = SUMMARY_BLOCK_TICKS[level];
        assert_eq!(block_ticks, 256);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].start_tick, 0);
        assert_eq!(blocks[0].velocity, 90, "取块内最大 velocity");
        assert_eq!(blocks[0].track, 1);
        assert_eq!(blocks[1].start_tick, 256);
        assert_eq!(blocks[1].velocity, 70);
    }

    /// velocity ≤ 1 与隐藏轨道不参与摘要（与渲染过滤一致）→ 无可用档位。
    #[test]
    fn filters_muted_velocity_and_hidden_tracks() {
        let mut notes = vec![make(0, 1, 0), make(0, 60, 1)];
        notes.sort_by_key(|n| n.start_tick);
        let s = VelocitySummary::build(
            &MockSource {
                notes: NoteBucket::from_sorted(notes),
            },
            &[true, false],
        );
        assert!(
            s.level_for_ppu(0.003).is_none(),
            "全部被过滤时没有任何档位可用"
        );
        // 对照：轨道可见时同一批数据有摘要。
        let mut notes = vec![make(0, 1, 0), make(0, 60, 1)];
        notes.sort_by_key(|n| n.start_tick);
        let s = VelocitySummary::build(
            &MockSource {
                notes: NoteBucket::from_sorted(notes),
            },
            &[true, true],
        );
        let level = s.level_for_ppu(0.003).expect("轨道可见时应有摘要");
        assert_eq!(s.level(level).len(), 1);
        assert_eq!(s.level(level)[0].velocity, 60);
    }

    /// 全曲视图 ppu 很小时能选到档位。
    #[test]
    fn level_selected_for_small_ppu() {
        let s = build(vec![make(0, 100, 0)]);
        assert!(s.level_for_ppu(2.0e-3).is_some());
        assert!(s.level_for_ppu(1.0).is_none(), "放大后走原始路径");
    }
}
