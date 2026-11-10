//! MIDI Clip v1：单轨命名时间区间。
//!
//! Clip 只是音符归属的元数据，不保存音符副本。音符仍存放在
//! `YinModel.notes` 的 per-key 桶里；归属规则见 `docs/spec-midi-clip.md`。

use serde::{Deserialize, Serialize};

use crate::model::{TrackData, YinModel};

/// 一个 Clip：单轨上的命名时间区间。
///
/// 归属只看音符 `start_tick`：`clip.start_tick <= note.start_tick < clip.end_tick`。
/// note gate 可以越过 `end_tick`，不截断、不改变归属。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    /// 全曲唯一，发号器分配，不复用。
    pub id: u32,
    /// 显示名；空时 UI 显示 `Clip {id}`。
    #[serde(default)]
    pub name: String,
    /// 区间起点（绝对 tick）。
    pub start_tick: u32,
    /// 区间终点（绝对 tick），必须大于 start_tick。
    pub end_tick: u32,
}

impl Clip {
    pub fn new(id: u32, name: String, start_tick: u32, end_tick: u32) -> Self {
        Self {
            id,
            name,
            start_tick,
            end_tick,
        }
    }

    /// 区间长度。
    pub fn len_ticks(&self) -> u64 {
        (self.end_tick as u64).saturating_sub(self.start_tick as u64)
    }

    pub fn contains_tick(&self, tick: u32) -> bool {
        self.start_tick <= tick && tick < self.end_tick
    }

    /// 显示名（空名回退到 `Clip {id}`）。
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            format!("Clip {}", self.id)
        } else {
            self.name.clone()
        }
    }
}

/// 在有序、不重叠的 Clip 列表中查找包含 tick 的 Clip。
pub fn clip_at(clips: &[Clip], tick: u32) -> Option<&Clip> {
    let idx = clips.partition_point(|c| c.start_tick <= tick);
    if idx == 0 {
        return None;
    }
    let c = &clips[idx - 1];
    (tick < c.end_tick).then_some(c)
}

/// 某轨 Clip 列表中与 `[lo, hi)` 有交集的切片。
///
/// Clip 列表必须按 start_tick 升序且不重叠。
pub fn clips_in_range(clips: &[Clip], lo: u32, hi: u32) -> &[Clip] {
    if lo >= hi {
        return &[];
    }
    let start = clips.partition_point(|c| c.end_tick <= lo);
    let end = clips.partition_point(|c| c.start_tick < hi);
    if start >= end {
        &[]
    } else {
        &clips[start..end]
    }
}

/// 校验单轨 Clip 列表：升序、start < end、互不重叠。
pub fn validate_clips(clips: &[Clip]) -> Result<(), &'static str> {
    let mut prev_end = 0u32;
    for (i, c) in clips.iter().enumerate() {
        if c.start_tick >= c.end_tick {
            return Err("clip start must be less than end");
        }
        if i > 0 && c.start_tick < prev_end {
            return Err("clips overlap or are not sorted");
        }
        prev_end = c.end_tick;
    }
    Ok(())
}

impl TrackData {
    /// 该轨是否已经有 Clip。
    pub fn has_clips(&self) -> bool {
        !self.clips.is_empty()
    }
}

impl YinModel {
    /// 分配一个全曲唯一 Clip id。
    pub fn alloc_clip_id(&mut self) -> u32 {
        let id = self.next_clip_id.max(1);
        self.next_clip_id = id.wrapping_add(1);
        id
    }

    /// 按每轨音符范围初始化 Clip：每轨一个整轨 Clip。
    ///
    /// 只在 `clips` 为空的轨道上创建，不覆盖已加载/已编辑的 Clip。
    /// `bounds` 索引 = track index；`None` 表示该轨没有音符。
    pub fn init_clips_from_bounds(&mut self, bounds: &[Option<(u32, u32)>]) {
        for (track_idx, bound) in bounds.iter().enumerate() {
            if track_idx >= self.tracks.len() {
                break;
            }
            let Some((start, end)) = *bound else {
                continue;
            };
            if !self.tracks[track_idx].clips.is_empty() {
                continue;
            }
            let end = end.max(start.saturating_add(1));
            let id = self.alloc_clip_id();
            let track = std::sync::Arc::make_mut(&mut self.tracks[track_idx]);
            track.clips.push(Clip::new(id, String::new(), start, end));
        }
    }

    /// 从当前音符重新推导每轨整轨 Clip 范围。
    ///
    /// 仅用于测试/兜底，不做增量维护；正常编辑应走编辑器命令。
    pub fn init_clips_from_notes(&mut self) {
        let mut bounds: Vec<Option<(u32, u32)>> = vec![None; self.tracks.len()];
        for bucket in self.notes.iter() {
            for n in bucket.iter() {
                let t = n.track as usize;
                if t >= bounds.len() {
                    continue;
                }
                let b = bounds[t].get_or_insert((n.start_tick, n.end_tick));
                b.0 = b.0.min(n.start_tick);
                b.1 = b.1.max(n.end_tick);
            }
        }
        self.init_clips_from_bounds(&bounds);
    }

    /// 加载后同步 Clip 发号器到已用最大值 +1。
    pub fn sync_next_clip_id(&mut self) {
        let max = self
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(|c| c.id))
            .max()
            .unwrap_or(0);
        self.next_clip_id = max.saturating_add(1).max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u32, start: u32, end: u32) -> Clip {
        Clip::new(id, String::new(), start, end)
    }

    #[test]
    fn clip_at_uses_start_half_open_interval() {
        let clips = vec![clip(1, 0, 100), clip(2, 200, 300)];
        assert_eq!(clip_at(&clips, 0).map(|c| c.id), Some(1));
        assert_eq!(clip_at(&clips, 99).map(|c| c.id), Some(1));
        assert_eq!(clip_at(&clips, 100), None);
        assert_eq!(clip_at(&clips, 250).map(|c| c.id), Some(2));
        assert_eq!(clip_at(&clips, 300), None);
    }

    #[test]
    fn clips_in_range_returns_intersections() {
        let clips = vec![clip(1, 0, 100), clip(2, 200, 300), clip(3, 400, 500)];
        assert_eq!(
            clips_in_range(&clips, 50, 250)
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(clips_in_range(&clips, 100, 200).len(), 0);
        assert_eq!(clips_in_range(&clips, 0, 1000).len(), 3);
    }

    #[test]
    fn validate_rejects_overlap_and_bad_range() {
        assert!(validate_clips(&[clip(1, 0, 100), clip(2, 100, 200)]).is_ok());
        assert!(validate_clips(&[clip(1, 0, 100), clip(2, 99, 200)]).is_err());
        assert!(validate_clips(&[clip(1, 100, 100)]).is_err());
    }

    #[test]
    fn loading_notes_creates_one_clip_per_non_empty_track() {
        use crate::events::NoteEvent;
        use std::sync::Arc;
        let mut m = YinModel {
            tracks: vec![
                Arc::new(TrackData::new(0, 0)),
                Arc::new(TrackData::new(0, 1)),
            ],
            ..Default::default()
        };
        m.load_track_notes(vec![
            vec![
                NoteEvent {
                    id: 0,
                    start_tick: 10,
                    end_tick: 20,
                    key: 60,
                    velocity: 100,
                },
                NoteEvent {
                    id: 0,
                    start_tick: 30,
                    end_tick: 90,
                    key: 61,
                    velocity: 100,
                },
            ],
            Vec::new(),
        ]);
        assert_eq!(m.tracks[0].clips.len(), 1);
        assert_eq!(m.tracks[0].clips[0].start_tick, 10);
        assert_eq!(m.tracks[0].clips[0].end_tick, 90);
        assert!(m.tracks[1].clips.is_empty());
        assert_eq!(m.next_clip_id, 2);
        assert!(validate_clips(&m.tracks[0].clips).is_ok());
    }
}
