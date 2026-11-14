//! 每轨时间网格位图：轨道 × 拍 1 bit，标记该拍内有没有可听音符起点。
//!
//! **用途（冻结/裁剪的基础设施）**：冻结轨道、按可见范围渲染时，需要
//! 快速回答"这条轨、这段 tick 有没有内容"。逐 key 扫描是 O(全部音符)，
//! Broken World（4400 万音符）一次区间查询约 650µs；本表按拍索引后
//! 是 O(区间字数)，实测约 4ns。
//!
//! 规模：Broken World 193 轨 × 1058 拍 ≈ 25KB，构建约 23ms；
//! start.mid（799 轨 / 1.64 亿音符）约 160KB、构建约 90ms。
//!
//! **语义**：bit = 有音符**起点**落在该拍（`start_tick / ppq`）。
//! 区间查询按拍对齐，查询范围落在某拍中间时可能保守地返回 true
//!（该拍内查询范围之外的音符也会命中）。它用于"这段有没有内容"的
//! 快速否定，不用于精确枚举音符。
//!
//! 这是**只读快照**：音符编辑后需重建。冻结/裁剪是低频操作，重建成本
//! 可接受；暂不做增量维护，避免把位图的生命周期绑到每次编辑上。

use yinhe_core::YinModel;

/// 每轨每拍 1 bit 的活跃表。
pub struct TrackActivityMap {
    /// 每拍 tick 数（`ProjectMeta::ppq`，至少 1）。
    ppq: u32,
    /// 拍数（`tick_length / ppq + 1`，覆盖最后一拍上的音符起点）。
    beats: usize,
    /// 轨道数（= `model.tracks.len()`）。
    tracks: usize,
    /// 每条轨道的字数（`beats.div_ceil(64)`）。
    words_per_track: usize,
    /// 轨道主序位图：`bits[track * words_per_track + word]`。
    bits: Vec<u64>,
}

impl TrackActivityMap {
    /// 从模型构建（扫描全部 key 桶；约 23ms / 4400 万音符）。
    ///
    /// 只统计 `velocity > 1` 的音符——与音频 dispatch 的"可听"判定一致，
    /// 黑乐谱里大量 `velocity == 1` 的占位音符不会让整条轨道显得有内容。
    pub fn build(model: &YinModel) -> Self {
        let ppq = model.meta.ppq.max(1);
        let beats = (model.tick_length / ppq as u64 + 1) as usize;
        let tracks = model.tracks.len();
        let words_per_track = beats.div_ceil(64);
        let mut bits = vec![0u64; tracks * words_per_track];
        for key in 0..yinhe_types::KEY_COUNT {
            for note in model.notes[key].iter() {
                if note.velocity <= 1 {
                    continue;
                }
                let track = note.track as usize;
                if track >= tracks {
                    continue;
                }
                let beat = (note.start_tick / ppq) as usize;
                if beat < beats {
                    bits[track * words_per_track + beat / 64] |= 1u64 << (beat % 64);
                }
            }
        }
        Self {
            ppq,
            beats,
            tracks,
            words_per_track,
            bits,
        }
    }

    /// 轨道数。
    pub fn track_count(&self) -> usize {
        self.tracks
    }

    /// 拍数。
    pub fn beat_count(&self) -> usize {
        self.beats
    }

    /// 位图占用的字节数（诊断/内存面板用）。
    pub fn memory_bytes(&self) -> usize {
        self.bits.len() * std::mem::size_of::<u64>()
    }

    /// 某轨在 `[start_tick, end_tick)` 内有没有可听音符起点。
    ///
    /// 空区间、越界轨道、`start_tick >= end_tick` 返回 false。
    /// 按拍对齐：查询范围只覆盖某拍的一部分时，该拍内的音符也会命中。
    pub fn track_active_in(&self, track: u16, start_tick: u64, end_tick: u64) -> bool {
        let Some((first, last)) = self.beat_range(start_tick, end_tick) else {
            return false;
        };
        self.track_active_words(track as usize, first, last)
    }

    /// 区间内"有内容"的轨道掩码（bit `t` = 轨道 t 命中）。
    ///
    /// 返回 `tracks.div_ceil(64)` 个字；调用方按 `mask[t / 64] >> (t % 64)`
    /// 判定轨道 t。用于冻结/裁剪时一次性筛掉整段没有音符的轨道。
    pub fn active_tracks(&self, start_tick: u64, end_tick: u64) -> Vec<u64> {
        let mut mask = Vec::new();
        self.active_tracks_into(start_tick, end_tick, &mut mask);
        mask
    }

    /// 同 [`Self::active_tracks`]，复用调用方的掩码缓冲（先清空再写入）。
    pub fn active_tracks_into(&self, start_tick: u64, end_tick: u64, mask: &mut Vec<u64>) {
        let words = self.tracks.div_ceil(64);
        mask.clear();
        mask.resize(words, 0);
        let Some((first, last)) = self.beat_range(start_tick, end_tick) else {
            return;
        };
        for track in 0..self.tracks {
            if self.track_active_words(track, first, last) {
                mask[track / 64] |= 1u64 << (track % 64);
            }
        }
    }

    /// 查询 tick 区间 → 闭区间拍号 `[first, last]`；越界返回 None。
    fn beat_range(&self, start_tick: u64, end_tick: u64) -> Option<(usize, usize)> {
        if end_tick <= start_tick || self.tracks == 0 || self.beats == 0 {
            return None;
        }
        let ppq = self.ppq as u64;
        let first = (start_tick / ppq) as usize;
        if first >= self.beats {
            return None;
        }
        let last = (((end_tick - 1) / ppq) as usize).min(self.beats - 1);
        Some((first, last))
    }

    /// 某轨在闭区间拍号 `[first, last]` 内是否置位（`beat_range` 已保证有效）。
    fn track_active_words(&self, track: usize, first: usize, last: usize) -> bool {
        let base = track * self.words_per_track;
        let Some(words) = self.bits.get(base..base + self.words_per_track) else {
            return false;
        };
        for (w, &src) in words
            .iter()
            .enumerate()
            .take(last / 64 + 1)
            .skip(first / 64)
        {
            let mut word = src;
            if w == first / 64 {
                word &= u64::MAX << (first % 64);
            }
            if w == last / 64 {
                word &= u64::MAX >> (63 - last % 64);
            }
            if word != 0 {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yinhe_core::{ProjectMeta, TrackData, YinModel};
    use yinhe_types::Note;

    use super::*;

    /// 空模型 + 指定轨道数（无 Conductor 行；音符的 track 直接索引）。
    fn model(tracks: usize) -> YinModel {
        let mut m = YinModel {
            tracks: (0..tracks)
                .map(|i| Arc::new(TrackData::new(0, (i % 16) as u8)))
                .collect(),
            meta: ProjectMeta {
                ppq: 480,
                ..Default::default()
            },
            ..Default::default()
        };
        m.load_track_notes(vec![Vec::new(); tracks]);
        m.rebuild();
        m
    }

    fn note(id: u32, start: u32, end: u32, velocity: u8, track: u16) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: end,
            velocity,
            track,
        }
    }

    fn insert(m: &mut YinModel, key: u8, n: Note) {
        m.insert_note(key, n);
        m.rebuild_dirty();
    }

    #[test]
    fn finds_notes_per_track_and_beat() {
        let mut m = model(4);
        insert(&mut m, 60, note(1, 0, 480, 100, 1));
        insert(&mut m, 62, note(2, 480 * 3, 480 * 3 + 10, 100, 2));
        insert(&mut m, 64, note(3, 480 * 3 + 100, 480 * 3 + 200, 100, 1));
        let map = TrackActivityMap::build(&m);
        assert_eq!(map.track_count(), 4);
        assert_eq!(map.beat_count(), 4, "tick_length 1440 → 0..=3 拍");

        assert!(map.track_active_in(1, 0, 480), "track 1 在拍 0 有音符");
        assert!(!map.track_active_in(1, 480, 960), "拍 1 为空");
        assert!(map.track_active_in(1, 960, 1920), "跨拍区间命中拍 3");
        assert!(!map.track_active_in(1, 1920, 2400), "区间外");
        assert!(!map.track_active_in(0, 0, 1920), "空轨不命中");
        assert!(!map.track_active_in(2, 0, 1440), "track 2 的音符只在拍 3");
        assert!(map.track_active_in(2, 1440, 1450), "最后一拍也覆盖");
        assert!(!map.track_active_in(99, 0, 480), "越界轨道返回 false");
    }

    /// 按拍对齐：查询只覆盖某拍的一部分时，同拍其它位置的音符也会命中
    ///（保守方向，用于冻结时宁可多算不可漏算）。
    #[test]
    fn query_is_conservative_for_partial_beats() {
        let mut m = model(1);
        insert(&mut m, 60, note(1, 0, 100, 100, 0));
        let map = TrackActivityMap::build(&m);
        assert!(map.track_active_in(0, 400, 480), "同拍内保守命中");
        assert!(!map.track_active_in(0, 480, 960), "下一拍不命中");
        assert!(!map.track_active_in(0, 100, 100), "空区间");
        assert!(!map.track_active_in(0, 960, 480), "反向区间");
    }

    #[test]
    fn active_tracks_mask_spans_words() {
        let mut m = model(130);
        insert(&mut m, 60, note(1, 0, 100, 100, 0));
        insert(&mut m, 60, note(2, 960, 1000, 100, 64));
        insert(&mut m, 61, note(3, 960, 1000, 100, 70));
        let map = TrackActivityMap::build(&m);

        let all = map.active_tracks(0, 1920);
        assert_eq!(all.len(), 130_usize.div_ceil(64));
        assert_eq!(all[0] & 1, 1, "track 0");
        assert_eq!(all[1] & 1, 1, "track 64 → 第 2 个字 bit 0");
        assert_eq!(all[1] >> 6 & 1, 1, "track 70 → 第 2 个字 bit 6");
        assert_eq!(all[0] >> 1, 0, "track 1 无音符");

        let beat2 = map.active_tracks(960, 1440);
        assert_eq!(beat2[0] & 1, 0, "track 0 只在拍 0");
        assert_eq!(beat2[1] & 1, 1);
        assert_eq!(beat2[1] >> 6 & 1, 1);

        // 复用缓冲：旧位必须被清掉。
        let mut buf = vec![u64::MAX; 8];
        map.active_tracks_into(960, 1440, &mut buf);
        assert_eq!(buf.len(), 130_usize.div_ceil(64));
        assert_eq!(buf[0], 0, "复用缓冲先清空");
    }

    #[test]
    fn ignores_silent_notes_and_out_of_range_tracks() {
        let mut m = model(2);
        insert(&mut m, 60, note(1, 0, 100, 1, 1)); // velocity 1 = 不可听
        insert(&mut m, 61, note(2, 0, 100, 2, 0)); // velocity 2 = 可听
        let map = TrackActivityMap::build(&m);
        assert!(map.track_active_in(0, 0, 480), "velocity 2 计入");
        assert!(!map.track_active_in(1, 0, 480), "velocity 1 不计入");
        assert_eq!(map.memory_bytes(), 2 * 8, "2 轨 × 1 拍 → 各 1 个字");

        // 模型里不存在 track 2 的音符：build 时防御性跳过。
        m.insert_note(62, note(3, 0, 100, 100, 2));
        m.rebuild_dirty();
        let map = TrackActivityMap::build(&m);
        assert_eq!(map.track_count(), 2);
        assert!(!map.track_active_in(1, 0, 480));
    }
}
