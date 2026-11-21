//! LOD 摘要缓存：按 `(key, track, tick_block)` 聚合，只保留当前块宽一档。
//!
//! 设计边界：
//! - 摘要不依赖视口/滚动，只依赖模型 revision 与块宽；
//! - 只保留当前块宽，换档时整层丢弃；不做多档常驻，也不做窗口预取；
//! - 编辑按 `note_revisions[key]` 失效，只重建被编辑的 key；
//! - 构建可以并行：每个 key 独立取桶、构建、再尝试写回。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use yinhe_types::{KEY_COUNT, KeySummary, NoteBucket, NoteSummary};

use crate::model::YinModel;

/// 摘要缓存。`YinModel` clone 时共享同一份缓存；内容是否有效仍由
/// 每个模型自己的 `note_revisions` 判定。
#[derive(Clone, Debug, Default)]
pub struct LodCache {
    inner: Arc<Mutex<LodState>>,
}

#[derive(Debug)]
struct LodState {
    block_ticks: u32,
    keys: Box<[Option<CachedKey>; KEY_COUNT]>,
}

impl Default for LodState {
    fn default() -> Self {
        Self {
            block_ticks: 0,
            keys: Box::new(core::array::from_fn(|_| None)),
        }
    }
}

#[derive(Debug)]
struct CachedKey {
    revision: u64,
    summary: Arc<KeySummary>,
}

fn lock(state: &Mutex<LodState>) -> MutexGuard<'_, LodState> {
    // 摘要构建在渲染线程；即使某次构建 panic，也不能让后续帧永久失败。
    state.lock().unwrap_or_else(|e| e.into_inner())
}

impl LodCache {
    fn get(&self, key: u8, block_ticks: u32, revision: u64) -> Option<Arc<KeySummary>> {
        let state = lock(&self.inner);
        if state.block_ticks != block_ticks {
            return None;
        }
        let cached = state.keys[key as usize].as_ref()?;
        (cached.revision == revision).then(|| Arc::clone(&cached.summary))
    }

    fn insert(&self, key: u8, block_ticks: u32, revision: u64, summary: &Arc<KeySummary>) {
        let mut state = lock(&self.inner);
        if state.block_ticks != block_ticks {
            // 另一个线程已经切到新档位；旧档结果直接丢弃，避免混档。
            return;
        }
        state.keys[key as usize] = Some(CachedKey {
            revision,
            summary: Arc::clone(summary),
        });
    }

    fn reset(&self, block_ticks: u32) -> bool {
        let mut state = lock(&self.inner);
        if state.block_ticks == block_ticks {
            return false;
        }
        state.block_ticks = block_ticks;
        *state.keys = core::array::from_fn(|_| None);
        true
    }
}

impl LodCache {
    /// 取 key 的摘要；缺失时现场构建并写回。
    ///
    /// 同一帧内 rayon 会并行调用不同 key，每个 key 只构建一次。
    pub fn summary_or_build(&self, model: &YinModel, key: u8, block_ticks: u32) -> Arc<KeySummary> {
        let revision = model.note_revisions[key as usize];
        if let Some(summary) = self.get(key, block_ticks, revision) {
            return summary;
        }

        // 切档时清掉旧档；只有真正用到摘要时才释放旧内存。
        let _ = self.reset(block_ticks);

        let bucket = Arc::clone(&model.notes[key as usize]);
        let summary = Arc::new(build_key_summary(&bucket, key, block_ticks));
        self.insert(key, block_ticks, revision, &summary);
        summary
    }
}

/// 把一个 key 的原始音符聚合成 `(block, track)` 摘要段。
///
/// 输入 `bucket` 已按 `start_tick` 升序；输出按 `(block, track)` 升序。
/// 每段保留块内最小起点、最大终点与最大力度，长音符跨块显示。
pub fn build_key_summary(bucket: &NoteBucket, key: u8, block_ticks: u32) -> KeySummary {
    let block_ticks = block_ticks.max(1);
    let mut segments = Vec::new();
    let mut states: Vec<TrackState> = Vec::new();
    let mut index: HashMap<u16, usize> = HashMap::new();
    let mut current_block = u32::MAX;
    let mut max_note_len = 0u32;

    for note in bucket.iter() {
        let block = note.start_tick / block_ticks;
        if block != current_block {
            flush_block(&mut states, &mut index, &mut segments, key, current_block);
            current_block = block;
        }

        max_note_len = max_note_len.max(note.end_tick.saturating_sub(note.start_tick));
        let slot = *index.entry(note.track).or_insert_with(|| {
            states.push(TrackState {
                track: note.track,
                start_tick: note.start_tick,
                end_tick: note.end_tick,
                velocity: note.velocity,
            });
            states.len() - 1
        });
        let state = &mut states[slot];
        state.start_tick = state.start_tick.min(note.start_tick);
        state.end_tick = state.end_tick.max(note.end_tick);
        state.velocity = state.velocity.max(note.velocity);
    }
    flush_block(&mut states, &mut index, &mut segments, key, current_block);

    KeySummary {
        segments,
        max_note_len,
    }
}

struct TrackState {
    track: u16,
    start_tick: u32,
    end_tick: u32,
    velocity: u8,
}

fn flush_block(
    states: &mut Vec<TrackState>,
    index: &mut HashMap<u16, usize>,
    out: &mut Vec<NoteSummary>,
    key: u8,
    block: u32,
) {
    if states.is_empty() {
        return;
    }
    // 固定按 track 升序输出，便于测试与跨帧稳定比较。
    states.sort_unstable_by_key(|s| s.track);
    out.extend(states.iter().map(|s| NoteSummary {
        block,
        start_tick: s.start_tick,
        end_tick: s.end_tick,
        key,
        track: s.track,
        velocity: s.velocity,
    }));
    states.clear();
    index.clear();
}

impl YinModel {
    /// 取 key 的 LOD 摘要；缺失时构建（见 [`LodCache::summary_or_build`]）。
    pub fn key_summary_for_render(&self, key: u8, block_ticks: u32) -> Arc<KeySummary> {
        self.lod.summary_or_build(self, key, block_ticks)
    }
}

#[cfg(test)]
mod tests {
    use yinhe_types::Note;

    use super::*;

    fn note(track: u16, start: u32, end: u32, velocity: u8) -> Note {
        Note {
            id: start + u32::from(track),
            start_tick: start,
            end_tick: end,
            velocity,
            track,
        }
    }

    #[test]
    fn groups_by_block_and_track() {
        let bucket = NoteBucket::from_sorted(vec![
            note(1, 0, 10, 40),
            note(0, 2, 20, 80),
            note(1, 8, 30, 90),
            note(0, 100, 110, 20),
        ]);
        let summary = build_key_summary(&bucket, 60, 64);
        assert_eq!(
            summary.segments,
            vec![
                NoteSummary {
                    block: 0,
                    start_tick: 2,
                    end_tick: 20,
                    key: 60,
                    track: 0,
                    velocity: 80,
                },
                NoteSummary {
                    block: 0,
                    start_tick: 0,
                    end_tick: 30,
                    key: 60,
                    track: 1,
                    velocity: 90,
                },
                NoteSummary {
                    block: 1,
                    start_tick: 100,
                    end_tick: 110,
                    key: 60,
                    track: 0,
                    velocity: 20,
                },
            ]
        );
        assert_eq!(summary.max_note_len, 22);
    }

    #[test]
    fn long_note_widens_query_left_bound() {
        let bucket =
            NoteBucket::from_sorted(vec![note(0, 0, 10_000, 100), note(0, 9_000, 9_100, 100)]);
        let summary = build_key_summary(&bucket, 60, 128);
        assert_eq!(summary.max_note_len, 10_000);
        // 从 tick=8000 查询时，必须回看到 block 0（长音符起点）。
        assert_eq!(summary.first_segment(8_000, 128), 0);
    }

    #[test]
    fn cache_reuses_until_revision_changes() {
        let mut model = YinModel::new_empty_with_16_tracks();
        model.insert_note(60, note(0, 0, 10, 100));
        model.rebuild();

        let first = model.key_summary_for_render(60, 64);
        let second = model.key_summary_for_render(60, 64);
        assert!(Arc::ptr_eq(&first, &second));

        model.insert_note(60, note(0, 20, 30, 100));
        model.rebuild();
        let third = model.key_summary_for_render(60, 8);
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(third.segments.len(), 2);
    }
}
