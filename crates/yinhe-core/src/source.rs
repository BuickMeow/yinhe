//! Implement `yinhe_types::NoteSource` for `YinModel`.
//!
//! This trait is the contract between data sources and rendering / audio
//! consumers. By implementing it on YinModel directly, every component
//! that already speaks NoteSource (PianoRoll, Arrangement) works without
//! changes.

use yinhe_types::{NoteBucket, NoteSource, TimeSigEvent};

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
