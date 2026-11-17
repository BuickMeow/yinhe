//! 混音/轨道显示标签的唯一来源。
//!
//! 所有「A01 / LC01 / Audio-01 / Master」文案都必须从这里取，避免 AP 显示
//! 改一处漏一处（历史上 AP 曾显示 `AP01`，与 MIDI 端口/通道对不上）。

use std::collections::HashMap;

use rust_i18n::t;
use yinhe_types::{ChannelId, LogicalChannelId};

/// MIDI 端口 + 通道 → 显示标签（port 0/channel 0 → "A01"，port 15/channel 15 → "P16"）。
pub(crate) fn midi_label(port: u8, channel: u8) -> String {
    format!("{}{:02}", (b'A' + port.min(15)) as char, channel % 16 + 1)
}

/// 源通道号 → 显示标签（0 → "A01"，255 → "P16"）。
pub(crate) fn channel_label(ch: u8) -> String {
    midi_label(ch / 16, ch % 16)
}

/// AP 通道显示标签：按它对应的 MIDI 端口 + 通道显示。
///
/// AP 本质就是一个 MIDI 源通道（A01..P16），显示成 `A01` 才能和导入导出、
/// 外部设备、MIDI 事件的通道号对上。不要用无意义的 `APxx` 序号。
pub(crate) fn ap_label(global: u8) -> String {
    channel_label(global)
}

/// 混音通道标题：AP 与独立 MIDI 通道都显示端口 + 通道（A01）。
pub(crate) fn channel_title(mixer: &yinhe_mixer::MixerParams, ch: u8) -> String {
    if mixer.ap_for_midi(ch).is_some() {
        ap_label(ch)
    } else {
        channel_label(ch)
    }
}

/// 音频通道号 → 显示标签（0 → "Audio-01"）。
pub(crate) fn audio_label(ch: u16) -> String {
    format!("Audio-{:02}", u32::from(ch) + 1)
}

/// 逻辑通道编号 → 显示标签（1 → "LC01"）。
pub(crate) fn lc_label(display: u32) -> String {
    format!("LC{display:02}")
}

/// C 的 `ChannelId` → 聚焦发送标签（LC01 / A01 / Master）。
pub(crate) fn channel_id_label(id: ChannelId, display: &HashMap<ChannelId, u32>) -> String {
    match id {
        ChannelId::Lc(_) => display
            .get(&id)
            .map(|n| lc_label(*n))
            .unwrap_or_else(|| "LC".to_string()),
        ChannelId::Ap(ap) => ap_label(ap.group.0 as u8),
        ChannelId::Master => t!("mix.master").to_string(),
    }
}

/// 轨道音频目标 → AR / 音轨属性显示标签。
///
/// - OwnLc → `LCxx`（按 `lc_labels` 的顺序编号）
/// - AP → 端口 + 通道（`A01`）
/// - DirectMaster → `---`
pub(crate) fn track_target_label(
    target: Option<yinhe_core::TrackAudioTarget>,
    midi_port: u8,
    midi_channel: u8,
    lc_labels: &HashMap<LogicalChannelId, String>,
) -> String {
    match target {
        Some(yinhe_core::TrackAudioTarget::OwnLc { lc, .. }) => lc_labels
            .get(&lc)
            .cloned()
            .unwrap_or_else(|| "LC".to_string()),
        Some(yinhe_core::TrackAudioTarget::DirectMaster { .. }) => "---".to_string(),
        Some(yinhe_core::TrackAudioTarget::Ap { .. }) | None => midi_label(midi_port, midi_channel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_labels_cover_port_and_channel() {
        assert_eq!(midi_label(0, 0), "A01");
        assert_eq!(midi_label(0, 9), "A10");
        assert_eq!(midi_label(1, 0), "B01");
        assert_eq!(midi_label(15, 15), "P16");
        assert_eq!(channel_label(16), "B01");
        assert_eq!(ap_label(9), "A10");
    }

    #[test]
    fn track_target_label_matches_audio_target() {
        let lc = LogicalChannelId(4096);
        let mut labels = HashMap::new();
        labels.insert(lc, "LC03".to_string());
        assert_eq!(
            track_target_label(
                Some(yinhe_core::TrackAudioTarget::OwnLc { lc, rack: None }),
                0,
                0,
                &labels
            ),
            "LC03"
        );
        assert_eq!(
            track_target_label(
                Some(yinhe_core::TrackAudioTarget::Ap {
                    ap: yinhe_types::ApGroupId::new(9)
                }),
                0,
                9,
                &labels
            ),
            "A10"
        );
        assert_eq!(
            track_target_label(
                Some(yinhe_core::TrackAudioTarget::DirectMaster { rack: None }),
                0,
                0,
                &labels
            ),
            "---"
        );
    }
}
