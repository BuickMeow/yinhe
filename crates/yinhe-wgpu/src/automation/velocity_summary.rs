//! AM 力度条的 LOD 摘要：按 tick 块聚合「块内 gate 最长的音符」。
//!
//! 缩小时同一条像素列有海量力度条，逐音符构建（读 + 全局排序 + 去重）会到
//! 数百 ms/帧（ReptilianDarkRitual 全曲 1.6s/帧、常规滚动视口 434ms/帧）。
//! 摘要把数据量压到「块数」（与音符数无关）：缩小时直接从摘要切片生成 bar，
//! 每块一条 = 块内最长音符的真实区间——bar 宽度仍指示音符 gate（长音符是
//! 长条，短音符是细条），高度/颜色取该音符自身的 velocity/track。
//!
//! 档位复用 `pianoroll` 的 `SUMMARY_BLOCK_TICKS` / `SUMMARY_MAX_PX`：
//! 块宽 ≤ 2px 时启用；最细档 16 tick（更细档段数会趋近音符数，已移除）。

use rayon::prelude::*;
use yinhe_types::{MAX_KEY, NoteSource};

use crate::pianoroll::{SUMMARY_BLOCK_TICKS, select_summary_level};

/// 摘要中的一个块（tick 升序存放）。
///
/// 代表音符 = 块内 gate 最长的音符（gate 相同取 velocity 大者）：
/// 缩小时长音符仍显示为真实长度的长条，短音符为细条——保留「bar 宽度
/// 指示音符长度」的语义（若只按最大 velocity 取代表，bar 宽度会退化成
/// 固定块宽，失去长度信息）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VelocityBlock {
    /// 代表音符的起点 tick。
    pub start_tick: u32,
    /// 代表音符的终点 tick（bar 长度 = end - start）。
    pub end_tick: u32,
    pub velocity: u8,
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
                // 每块 [start, end, packed]；packed == 0 表示空块。
                // 维护块内 gate 最长的音符（gate 相同取 packed 大者）。
                let mut best = vec![[0u32; 3]; nblocks];
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
                        let gate = note.end_tick.saturating_sub(note.start_tick);
                        let cur = &mut best[idx];
                        let cur_gate = if cur[2] != 0 {
                            cur[1].saturating_sub(cur[0])
                        } else {
                            0
                        };
                        if cur[2] == 0 || gate > cur_gate || (gate == cur_gate && packed > cur[2]) {
                            *cur = [note.start_tick, note.end_tick, packed];
                        }
                    }
                }
                let mut out = Vec::new();
                for a in best {
                    if a[2] != 0 {
                        out.push(VelocityBlock {
                            start_tick: a[0],
                            end_tick: a[1],
                            velocity: (a[2] >> 16) as u8,
                            track: a[2] as u16,
                        });
                    }
                }
                out
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

    fn make_gate(tick: u32, gate: u32, vel: u8, track: u16) -> Note {
        Note {
            id: 0,
            start_tick: tick,
            end_tick: tick + gate,
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

    /// 每块取 gate 最长的音符为代表（gate 相同取 velocity 大者），
    /// bar 保留代表音符的真实区间（长度指示 gate）。
    #[test]
    fn blocks_take_longest_note_with_real_gate() {
        let s = build(vec![
            make(0, 50, 0),
            make(100, 90, 1), // 同一 block 内（ppu=0.005 → block=256），同 gate 取 vel 大者
            make(300, 70, 2), // 下一块
        ]);
        let level = s.level_for_ppu(0.005).expect("应触发摘要档");
        let blocks = s.level(level);
        assert_eq!(SUMMARY_BLOCK_TICKS[level], 256);
        assert_eq!(blocks.len(), 2);
        // 代表音符是 tick=100 的那个（gate 同为 10，velocity 更大）。
        assert_eq!((blocks[0].start_tick, blocks[0].end_tick), (100, 110));
        assert_eq!(blocks[0].velocity, 90);
        assert_eq!(blocks[0].track, 1);
        // 第二块的代表音符不从块边界开始，而是音符真实起点。
        assert_eq!((blocks[1].start_tick, blocks[1].end_tick), (300, 310));
        assert_eq!(blocks[1].velocity, 70);
        assert!(
            blocks[0].start_tick < blocks[1].start_tick,
            "输出按 tick 升序"
        );
    }

    /// 长音符优先于短强音：bar 用长音符的真实长度（恢复长度指示），
    /// 高度/颜色取该代表音符自身的值。
    #[test]
    fn longest_note_beats_short_loud_note() {
        let s = build(vec![
            make_gate(10, 5, 120, 0),  // 短强音
            make_gate(20, 200, 40, 1), // 长弱音
        ]);
        let level = s.level_for_ppu(0.005).expect("摘要档");
        let blocks = s.level(level);
        assert_eq!(blocks.len(), 1, "同一块只输出一个代表");
        assert_eq!((blocks[0].start_tick, blocks[0].end_tick), (20, 220));
        assert_eq!(blocks[0].velocity, 40);
        assert_eq!(blocks[0].track, 1);
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

    /// 全曲视图 ppu 很小时能选到档位；最细 16 档覆盖到 ppu=0.125。
    #[test]
    fn level_selected_for_small_ppu() {
        let s = build(vec![make(0, 100, 0)]);
        assert!(s.level_for_ppu(2.0e-3).is_some());
        assert!(s.level_for_ppu(0.1).is_some(), "16 档区间仍走摘要");
        assert!(s.level_for_ppu(1.0).is_none(), "16 块 > 2px 回原始路径");
    }
}
