use std::sync::Arc;

use yinhe_core::YinModel;

use crate::audio_model::{AudioModel, PreparedModel, flatten_automation_to_cc_events};

/// 全曲时长（秒）：MIDI/自动化末尾与音频片段末尾取大者。
pub(crate) fn model_duration_seconds(model: &YinModel) -> f64 {
    // 音频片段按秒存储，0 BPM 剪切后需要跟着左移/截断。
    let audio_end = model
        .tracks
        .iter()
        .flat_map(|t| t.audio_clips.iter())
        .filter_map(|c| {
            model
                .tempo_map
                .cut_clip_times(c.start_seconds, c.end_seconds())
        })
        .map(|(_, end)| end)
        .fold(0.0, f64::max);
    model
        .tempo_map
        .tick_to_seconds(model.tick_length)
        .max(audio_end)
}

/// Build `PreparedModel` on a worker thread (no `&mut AudioEngine` needed).
/// This is the expensive part; the result is applied cheaply on the audio thread.
///
/// `density`: Linear/Curve 自动化段的中间事件 tick 间隔。
/// 不接收 AM M/S 旁通：所有 lane 全量展平，旁通由 dispatch 动态掩码负责。
pub(crate) fn prepare_model(
    model: &Arc<YinModel>,
    sample_rate: u32,
    density: u32,
) -> PreparedModel {
    let cc_events = flatten_automation_to_cc_events(model, density);

    let duration_samples = (model_duration_seconds(model) * sample_rate as f64) as u64;

    PreparedModel {
        model: AudioModel::from_model(model),
        yin_model: Arc::clone(model),
        cc_events,
        duration_samples,
    }
}

/// Notes-only 准备：只换模型引用与派生快照，不再构建任何音符副本。
///
/// 旧的「dirty 桶 + `audible_notes` 增量」整条链已删除：dispatch 直接从
/// `YinModel.notes[key]` 顺序读，力度过滤/0 BPM 剪切在派发时逐音符现算。
pub(crate) fn prepare_notes(
    model: &Arc<YinModel>,
    sample_rate: u32,
) -> (AudioModel, Arc<YinModel>, u64) {
    let duration_samples = (model_duration_seconds(model) * sample_rate as f64) as u64;
    (
        AudioModel::from_model(model),
        Arc::clone(model),
        duration_samples,
    )
}

/// dispatch 时的可听性判定：力度过滤 + 0 BPM 剪切。
///
/// 返回截断后的 `end_tick`；`None` = 这个音符不发声。
/// 逐音符现算的成本 = 一次比较 + 一次 skip-range 查找（通常为空），
/// 整曲只随光标顺序扫一遍，不需要预构建 `audible_notes` 副本。
#[inline]
pub(crate) fn audible_end(
    model: &YinModel,
    note: &yinhe_types::Note,
    ignore_velocity: u8,
) -> Option<u32> {
    if note.velocity <= ignore_velocity {
        return None;
    }
    model
        .tempo_map
        .playable_note_end(note.start_tick, note.end_tick)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yinhe_core::{ConductorData, NoteEvent, YinModel};
    use yinhe_types::{AutomationEvent, SegmentShape};

    use super::audible_end;

    fn note(start: u32, end: u32, key: u8, velocity: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity,
        }
    }

    fn model_with(notes: Vec<NoteEvent>) -> YinModel {
        let mut m = YinModel::default();
        m.load_track_notes(vec![notes]);
        m.rebuild();
        m
    }

    fn first(model: &YinModel, key: usize) -> yinhe_types::Note {
        model.notes[key].iter().next().copied().expect("note")
    }

    /// 力度 ≤ 阈值不发声；> 阈值按原样发声。
    #[test]
    fn velocity_threshold_filters_at_dispatch() {
        let m = model_with(vec![
            note(0, 10, 60, 0),
            note(10, 20, 60, 1),
            note(20, 30, 60, 2),
            note(30, 40, 60, 100),
        ]);
        assert_eq!(audible_end(&m, &first(&m, 60), 1), None, "vel 0 被过滤");
        // 第二个音符 vel=1 同样被过滤
        let second = m.notes[60].iter().nth(1).copied().unwrap();
        assert_eq!(audible_end(&m, &second, 1), None);
        let third = m.notes[60].iter().nth(2).copied().unwrap();
        assert_eq!(audible_end(&m, &third, 1), Some(30));
        // 阈值 0：只有 vel 0 被过滤
        assert_eq!(audible_end(&m, &second, 0), Some(20));
        // 阈值 100：全部过滤
        let last = m.notes[60].iter().nth(3).copied().unwrap();
        assert_eq!(audible_end(&m, &last, 100), None);
    }

    #[test]
    fn zero_bpm_cut_filters_and_clamps_notes() {
        let mut conductor = ConductorData::default();
        for (tick, bpm) in [(0u32, 120.0f32), (480, 0.0), (960, 60.0)] {
            conductor.tempo.events.push(AutomationEvent {
                id: 0,
                tick,
                value: bpm,
                shape: SegmentShape::Step,
            });
        }
        let mut m = YinModel {
            conductor: Arc::new(conductor),
            ..Default::default()
        };
        m.load_track_notes(vec![vec![
            note(0, 1000, 60, 100),   // 跨剪切起点 → 截断到 480
            note(600, 700, 61, 100),  // 剪切内 → 不发声
            note(960, 1440, 62, 100), // 剪切后 → 保留
        ]]);
        m.rebuild();

        let cross = m.notes[60].iter().next().copied().unwrap();
        assert_eq!(audible_end(&m, &cross, 0), Some(480));
        let cut = m.notes[61].iter().next().copied().unwrap();
        assert_eq!(audible_end(&m, &cut, 0), None);
        let after = m.notes[62].iter().next().copied().unwrap();
        assert_eq!(audible_end(&m, &after, 0), Some(1440));
    }
}
