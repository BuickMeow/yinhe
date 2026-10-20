use std::sync::Arc;

use yinhe_core::YinModel;
use yinhe_types::KEY_COUNT;

use crate::audio_model::{
    AudibleDelta, AudibleNote, AudioModel, PreparedModel, flatten_automation_to_cc_events,
};

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
/// `ignore_velocity`：忽略力度 ≤ 该值的音符（0 = 不忽略）。
/// 不接收 AM M/S 旁通：所有 lane 全量展平，旁通由 dispatch 动态掩码负责。
pub(crate) fn prepare_model(
    model: &Arc<YinModel>,
    sample_rate: u32,
    density: u32,
    ignore_velocity: u8,
) -> PreparedModel {
    let cc_events = flatten_automation_to_cc_events(model, density);

    let duration_samples = (model_duration_seconds(model) * sample_rate as f64) as u64;

    let audible_notes = build_audible_notes(model, ignore_velocity);

    PreparedModel {
        model: AudioModel::from_model(model),
        yin_model: Arc::clone(model),
        cc_events,
        audible_notes,
        duration_samples,
    }
}

/// Notes-only 增量准备：只重建 `dirty` 掩码标记的 key 桶，其余桶保持不动。
///
/// 用于 `UpdateNotes` 纯音符编辑：1 亿音符工程每次编辑只扫变化的桶，
/// 而不是全量重扫 KEY_COUNT 桶 × 全部音符。`dirty` 由 worker 对比前后
/// `note_revisions` 得出（全 true = 全量，首次同步/全量 rebuild 后）。
pub(crate) fn prepare_notes_dirty(
    model: &Arc<YinModel>,
    sample_rate: u32,
    dirty: &[bool; KEY_COUNT],
    ignore_velocity: u8,
) -> (AudioModel, Arc<YinModel>, AudibleDelta, u64) {
    let duration_samples = (model_duration_seconds(model) * sample_rate as f64) as u64;

    let mut delta: AudibleDelta = Box::new(core::array::from_fn(|_| None));
    for key in 0..KEY_COUNT {
        if dirty[key] {
            delta[key] = Some(build_bucket(model, key, ignore_velocity));
        }
    }
    (
        AudioModel::from_model(model),
        Arc::clone(model),
        delta,
        duration_samples,
    )
}

/// 单桶构建：key 桶内 vel > `ignore_velocity` 的音符，时刻存 tick（u32）。
/// 模型桶按 start_tick 排序，tick 天然单调，**无需 sort**（旧 sample 域因
/// tempo 变速段的局部非单调才需要保险性 sort）。
fn build_bucket(model: &YinModel, key: usize, ignore_velocity: u8) -> Vec<AudibleNote> {
    let mut dst: Vec<AudibleNote> = Vec::with_capacity(model.notes[key].len());
    for n in model.notes[key].iter() {
        if n.velocity <= ignore_velocity {
            continue;
        }
        // 0 BPM 剪切：落在剪切区间的音符不发声；跨过剪切起点的音符在起点截断。
        let Some(end_tick) = model.tempo_map.playable_note_end(n.start_tick, n.end_tick) else {
            continue;
        };
        dst.push(AudibleNote {
            start_tick: n.start_tick,
            end_tick,
            track: n.track,
            velocity: n.velocity,
        });
    }
    dst
}

/// 遍历 YinModel KEY_COUNT 个 key 桶，过滤力度 ≤ `ignore_velocity` 的音符
///（时刻存 tick）。桶内天然升序（YinModel.notes[key] 按 start_tick 升序）。
pub(crate) fn build_audible_notes(
    model: &YinModel,
    ignore_velocity: u8,
) -> Box<[Vec<AudibleNote>; KEY_COUNT]> {
    let mut buckets: Box<[Vec<AudibleNote>; KEY_COUNT]> =
        Box::new(std::array::from_fn(|_| Vec::new()));
    for key in 0..KEY_COUNT {
        buckets[key] = build_bucket(model, key, ignore_velocity);
    }
    buckets
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yinhe_core::{ConductorData, NoteEvent, YinModel};
    use yinhe_types::{AutomationEvent, SegmentShape};

    use super::build_bucket;

    fn note(start: u32, end: u32, key: u8) -> NoteEvent {
        NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: end,
            key,
            velocity: 100,
        }
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
            note(0, 1000, 60),   // 跨剪切起点 → 截断到 480
            note(600, 700, 61),  // 剪切内 → 不发声
            note(960, 1440, 62), // 剪切后 → 保留
        ]]);
        m.rebuild();

        let cross = build_bucket(&m, 60, 0);
        assert_eq!(cross.len(), 1);
        assert_eq!(cross[0].start_tick, 0);
        assert_eq!(cross[0].end_tick, 480);

        assert!(build_bucket(&m, 61, 0).is_empty());

        let after = build_bucket(&m, 62, 0);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].end_tick, 1440);
    }
}
