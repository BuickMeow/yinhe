use std::sync::Arc;

use std::collections::HashMap;

use yinhe_core::YinModel;
use yinhe_types::KEY_COUNT;

use crate::audio_model::{
    AudibleDelta, AudibleNote, AudioModel, PreparedModel, flatten_automation_to_cc_events,
};

/// Build `PreparedModel` on a worker thread (no `&mut AudioEngine` needed).
/// This is the expensive part; the result is applied cheaply on the audio thread.
///
/// `density`: Linear/Curve 自动化段的中间事件 tick 间隔。
pub(crate) fn prepare_model(
    model: &Arc<YinModel>,
    sample_rate: u32,
    density: u32,
    am_ms: &HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>,
) -> PreparedModel {
    let cc_events = flatten_automation_to_cc_events(model, density, am_ms);

    let duration_samples =
        (model.tempo_map.tick_to_seconds(model.tick_length) * sample_rate as f64) as u64;

    let audible_notes = build_audible_notes(model);

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
) -> (AudioModel, Arc<YinModel>, AudibleDelta, u64) {
    let duration_samples =
        (model.tempo_map.tick_to_seconds(model.tick_length) * sample_rate as f64) as u64;

    let mut delta: AudibleDelta = Box::new(core::array::from_fn(|_| None));
    for key in 0..KEY_COUNT {
        if dirty[key] {
            delta[key] = Some(build_bucket(model, key));
        }
    }
    (
        AudioModel::from_model(model),
        Arc::clone(model),
        delta,
        duration_samples,
    )
}

/// 单桶构建：key 桶内 vel > 1 的音符，时刻存 tick（u32）。
/// 模型桶按 start_tick 排序，tick 天然单调，**无需 sort**（旧 sample 域因
/// tempo 变速段的局部非单调才需要保险性 sort）。
fn build_bucket(model: &YinModel, key: usize) -> Vec<AudibleNote> {
    let mut dst: Vec<AudibleNote> = Vec::with_capacity(model.notes[key].len());
    for n in model.notes[key].iter() {
        if n.velocity <= 1 {
            continue;
        }
        dst.push(AudibleNote {
            start_tick: n.start_tick,
            end_tick: n.end_tick,
            id: n.id,
            track: n.track,
            velocity: n.velocity,
        });
    }
    dst
}

/// 遍历 YinModel KEY_COUNT 个 key 桶，过滤 vel > 1 的音符（时刻存 tick）。
/// 桶内天然升序（YinModel.notes[key] 按 start_tick 升序）。
pub(crate) fn build_audible_notes(model: &YinModel) -> Box<[Vec<AudibleNote>; KEY_COUNT]> {
    let mut buckets: Box<[Vec<AudibleNote>; KEY_COUNT]> =
        Box::new(std::array::from_fn(|_| Vec::new()));
    for key in 0..KEY_COUNT {
        buckets[key] = build_bucket(model, key);
    }
    buckets
}
