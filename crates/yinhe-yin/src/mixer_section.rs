//! 混音段（.yin 可选第 4 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;

use yinhe_mixer::MixerParams;

use crate::codec::{deserialize_postcard, encode_versioned_section, serialize_postcard};
use crate::error::YinError;

/// 混音段格式版本（段内前 4 字节）。
///
/// 方案 C 统一 `ChannelId` 后不再保留 v9 并行分表：没有需要兼容的历史工程，
/// 只支持当前版本。
const MIXER_SECTION_VERSION: u32 = 10;

/// 编码混音段：version u32 LE + zstd(postcard MixerParams)。
pub(crate) fn encode_mixer_section(mixer: &MixerParams, level: i32) -> Result<Vec<u8>, YinError> {
    let payload = serialize_postcard(mixer)?;
    encode_versioned_section(MIXER_SECTION_VERSION, &payload, level)
}

/// 解码混音段。版本不符或损坏时记日志并返回 None（不阻断工程加载）。
pub(crate) fn decode_mixer_section(section: &[u8]) -> Option<MixerParams> {
    if section.len() < 4 {
        return None;
    }
    let version = u32::from_le_bytes(section[..4].try_into().ok()?);
    if version != MIXER_SECTION_VERSION {
        tracing::warn!("混音段版本 {version} 不受支持，忽略混音设置");
        return None;
    }
    let payload = zstd::decode_all(Cursor::new(&section[4..])).ok()?;
    let mut params = deserialize_postcard::<MixerParams>(&payload)
        .map_err(|e| tracing::warn!("v10 混音段解析失败，忽略混音设置: {e}"))
        .ok()?;
    params.ensure_len();
    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::ChannelId;
    use yinhe_types::ChannelSource;

    #[test]
    fn current_version_roundtrips_nodes() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        let track_lc = mixer.add_track_lc(0);
        let audio_lc = mixer.ensure_lc_node(ChannelSource::Audio(0));
        mixer.set_node_output(ChannelId::Lc(track_lc), Some(ChannelId::Lc(audio_lc)));
        mixer
            .node_mut(ChannelId::Lc(track_lc))
            .expect("track lc")
            .automation
            .push(yinhe_types::AutomationLane {
                target: yinhe_types::AutomationTarget::MixerGain {
                    channel: ChannelId::Lc(track_lc),
                },
                track: 0,
                events: vec![yinhe_types::AutomationEvent {
                    id: 1,
                    tick: 0,
                    value: 0.75,
                    shape: yinhe_types::SegmentShape::Step,
                }],
            });
        mixer
            .node_mut(ChannelId::Lc(track_lc))
            .expect("track lc")
            .freeze = Some(yinhe_mixer::FrozenAudio {
            source: "frozen-lc".into(),
            source_revision: 7,
            sample_rate: 48_000,
            frames: 1234,
        });
        mixer
            .node_mut(ChannelId::Lc(track_lc))
            .expect("track lc")
            .strip
            .gain = 0.25;

        let section = encode_mixer_section(&mixer, 0).expect("encode");
        let loaded = decode_mixer_section(&section).expect("decode");
        let node = loaded.node(ChannelId::Lc(track_lc)).expect("track lc");
        assert_eq!(node.strip.gain, 0.25);
        assert_eq!(node.output, Some(ChannelId::Lc(audio_lc)));
        assert!(node.freeze.is_some());
        assert_eq!(node.automation.len(), 1);
        assert!(loaded.node(ChannelId::Lc(audio_lc)).is_some());
    }

    #[test]
    fn ap_node_roundtrips() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        assert!(mixer.ensure_ap(3), "ensure_ap creates group");
        assert!(mixer.ap(yinhe_types::ApGroupId(3)).is_some());
        let ap = ChannelId::Ap(yinhe_types::ApChannelId::new(yinhe_types::ApGroupId(3), 0));
        mixer.node_mut(ap).expect("ap node").strip.gain = 0.5;
        let section = encode_mixer_section(&mixer, 0).expect("encode");
        let loaded = decode_mixer_section(&section).expect("decode");
        assert_eq!(loaded.node(ap).expect("ap node").strip.gain, 0.5);
        assert!(loaded.ap(yinhe_types::ApGroupId(3)).is_some());
    }

    #[test]
    fn ensure_len_has_no_bulk_midi_lcs() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        // 没有 AP/每轨 LC/音频轨时，不应再预建 256 条 MIDI 共享 LC。
        assert!(
            !mixer
                .nodes()
                .any(|node| matches!(node.source, ChannelSource::Track(_)))
        );
        assert!(mixer.nodes().all(|node| node.id != ChannelId::Master));
    }
}
