use std::sync::Arc;

use yinhe_core::YinModel;

use crate::audio_model::{
    AudibleDelta, AudibleNote, AudioModel, PreparedModel, flatten_automation_to_cc_events,
    tick_to_sample,
};

/// Build `PreparedModel` on a worker thread (no `&mut AudioEngine` needed).
/// This is the expensive part; the result is applied cheaply on the audio thread.
///
/// `density`: Linear/Curve 自动化段的中间事件 tick 间隔。
pub(crate) fn prepare_model(
    model: &Arc<YinModel>,
    sample_rate: u32,
    density: u32,
) -> PreparedModel {
    let cc_events = flatten_automation_to_cc_events(model, sample_rate, density);

    let duration_samples =
        (model.tempo_map.tick_to_seconds(model.tick_length) * sample_rate as f64) as u64;

    let audible_notes = build_audible_notes(model, sample_rate);

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
/// 而不是全量重扫 128 桶 × 全部音符。`dirty` 由 worker 对比前后
/// `note_revisions` 得出（全 true = 全量，首次同步/全量 rebuild 后）。
pub(crate) fn prepare_notes_dirty(
    model: &Arc<YinModel>,
    sample_rate: u32,
    dirty: &[bool; 128],
) -> (AudioModel, Arc<YinModel>, AudibleDelta, u64) {
    let duration_samples =
        (model.tempo_map.tick_to_seconds(model.tick_length) * sample_rate as f64) as u64;
    let segments = &model.tempo_map.tempo_segments;
    let tpb = model.tempo_map.ticks_per_beat;
    let sr = sample_rate as f64;

    let mut delta: Box<[Option<Vec<AudibleNote>>; 128]> = Box::new(core::array::from_fn(|_| None));
    for key in 0..128usize {
        if dirty[key] {
            delta[key] = Some(build_bucket(model, key, segments, tpb, sr));
        }
    }
    (
        AudioModel::from_model(model),
        Arc::clone(model),
        delta,
        duration_samples,
    )
}

/// 单桶构建：key 桶内 vel > 1 的音符，tick 预转换为 sample，并按 start_sample 排序。
/// 全量构建与 dirty 增量构建共用。
fn build_bucket(
    model: &YinModel,
    key: usize,
    segments: &[yinhe_core::TempoSegment],
    tpb: u32,
    sr: f64,
) -> Vec<AudibleNote> {
    let src = model.notes[key].as_slice();
    let mut dst: Vec<AudibleNote> = Vec::with_capacity(src.len());
    for n in src.iter() {
        if n.velocity <= 1 {
            continue;
        }
        dst.push(AudibleNote {
            start_sample: tick_to_sample(n.start_tick as u64, segments, tpb, sr),
            end_sample: tick_to_sample(n.end_tick as u64, segments, tpb, sr),
            id: n.id,
            track: n.track,
            velocity: n.velocity,
        });
    }
    // 容忍 tick→sample 在 tempo 变速段的局部非单调（理论单调，保险起见 sort 一次）。
    // 大多数情况下桶已升序，sort 是 O(n) 的 nearly-sorted 快路径。
    dst.sort_by_key(|n| n.start_sample);
    dst
}

/// 遍历 YinModel 128 个 key 桶，过滤 vel > 1 的音符，将 tick 预转换为 sample。
/// 桶内天然升序（YinModel.notes[key] 按 start_tick 升序，tick→sample 单调）。
pub(crate) fn build_audible_notes(
    model: &YinModel,
    sample_rate: u32,
) -> Box<[Vec<AudibleNote>; 128]> {
    let segments = &model.tempo_map.tempo_segments;
    let tpb = model.tempo_map.ticks_per_beat;
    let sr = sample_rate as f64;

    let mut buckets: Box<[Vec<AudibleNote>; 128]> = Box::new(std::array::from_fn(|_| Vec::new()));
    for key in 0..128usize {
        buckets[key] = build_bucket(model, key, segments, tpb, sr);
    }
    buckets
}
