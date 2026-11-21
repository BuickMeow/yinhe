//! Implement `yinhe_types::NoteSource` for `YinModel`.
//!
//! This trait is the contract between data sources and rendering / audio
//! consumers. By implementing it on YinModel directly, every component
//! that already speaks NoteSource (PianoRoll, Arrangement) works without
//! changes.

use yinhe_types::{NoteBucket, NoteRangeIter, NoteSource, TimeSigEvent};

use crate::model::YinModel;

impl NoteSource for YinModel {
    fn key_notes(&self, key: u8) -> &NoteBucket {
        self.notes[key as usize].as_ref()
    }

    fn duration(&self) -> f64 {
        self.tempo_map.duration_seconds()
    }

    fn ticks_per_beat(&self) -> Option<u32> {
        Some(self.meta.ppq)
    }

    fn tick_at_time(&self, time: f64) -> Option<f64> {
        Some(self.tempo_map.tick_at_time(time))
    }

    fn tick_length(&self) -> Option<u64> {
        Some(self.tick_length)
    }

    fn max_note_len(&self) -> u32 {
        self.max_note_len
    }

    /// 用每个 key 自己的最长音符收紧左界：工程里某条超长 pad 不应拖慢
    /// 其他 key 的范围查询。统计为 0（手工构造的模型）时回退全局值，保证不漏。
    fn key_notes_in_range(&self, key: u8, tick_start: u32, tick_end: u32) -> NoteRangeIter<'_> {
        let key_max = self.max_note_len_by_key[key as usize];
        let max_len = if key_max == 0 {
            self.max_note_len
        } else {
            key_max
        };
        let lo = tick_start.saturating_sub(max_len);
        self.notes[key as usize].range(lo, tick_end)
    }

    fn display_key_count(&self) -> u16 {
        // 黑乐谱扩展音域：项目里只要有任何 key ≥ 128 的音符，AR lane
        // 就用 256 键纵向范围，否则这些音符会画到 lane 外面（压到上一条轨）。
        // O(128) 的空桶检查，每帧调用一次，不需要维护缓存。
        if self.notes[yinhe_types::KEY_COUNT / 2..]
            .iter()
            .any(|bucket| !bucket.is_empty())
        {
            256
        } else {
            128
        }
    }

    fn time_sig_default(&self) -> (u8, u8) {
        self.tempo_map.time_sig_default
    }

    fn time_sig_events(&self) -> &[TimeSigEvent] {
        &self.tempo_map.time_sig_events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::Note;

    fn note(id: u32, start: u32) -> Note {
        Note {
            id,
            start_tick: start,
            end_tick: start + 120,
            velocity: 100,
            track: 0,
        }
    }

    #[test]
    fn key_notes_in_range_uses_per_key_max_len() {
        let mut model = YinModel::new_empty_with_16_tracks();
        model.insert_note(
            60,
            Note {
                id: 1,
                start_tick: 0,
                end_tick: 100_000,
                velocity: 100,
                track: 0,
            },
        );
        model.insert_note(61, note(2, 5_000));
        // 脏桶在 rebuild 前按未知处理，保证并发只读查询不漏。
        assert_eq!(model.max_note_len_by_key[60], u32::MAX);
        assert_eq!(model.max_note_len_by_key[61], u32::MAX);
        model.rebuild_dirty();

        assert_eq!(model.max_note_len_by_key[60], 100_000);
        assert_eq!(model.max_note_len_by_key[61], 120);
        // key 61 只有短音符：10_000 附近的范围不应把 5_000 的音符带出来。
        assert_eq!(model.key_notes_in_range(61, 10_000, 11_000).count(), 0);
        assert_eq!(model.key_notes_in_range(61, 5_000, 5_200).count(), 1);
    }

    /// AR lane 音域：常规 128；出现 key ≥ 128 的音符后切到 256，
    /// 删除扩展音符后恢复 128。
    #[test]
    fn display_key_count_follows_extended_notes() {
        let mut model = YinModel::new_empty_with_16_tracks();
        assert_eq!(model.display_key_count(), 128);

        model.insert_note(60, note(1, 0));
        assert_eq!(model.display_key_count(), 128);

        model.insert_note(200, note(2, 0));
        assert_eq!(model.display_key_count(), 256);

        model.remove_note_by_id(200, 2);
        assert_eq!(model.display_key_count(), 128);
    }
}
