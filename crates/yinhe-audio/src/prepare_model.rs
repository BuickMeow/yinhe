use std::sync::Arc;

use yinhe_core::YinModel;

use crate::audio_model::{AudioModel, PreparedModel, flatten_automation_to_cc_events};

/// Build `PreparedModel` on a worker thread (no `&mut AudioEngine` needed).
/// This is the expensive part; the result is applied cheaply on the audio thread.
///
/// `density`: Linear/Curve 自动化段的中间事件 tick 间隔。
pub(crate) fn prepare_model(
    model: &Arc<YinModel>,
    sample_rate: u32,
    density: u32,
    _active_mask: &[bool],
    _channel_map: &[u32; 256],
) -> PreparedModel {
    let cc_events = flatten_automation_to_cc_events(model, sample_rate, density);

    let duration_samples = (model.tempo_map.tick_to_seconds(model.tick_length) * sample_rate as f64) as u64;

    PreparedModel {
        model: AudioModel::from_model(model),
        yin_model: Arc::clone(model),
        cc_events,
        duration_samples,
    }
}
