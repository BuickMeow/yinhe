//! 混音段（.yin 可选第 4 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;

use serde::{Deserialize, Serialize};
use yinhe_mixer::{
    CHANNEL_COUNT, InsertRef, MasterParams, MixerNode, MixerParams, RackParams, StripParams,
};
use yinhe_types::{
    ChannelId, ChannelSource, LogicalChannelId, RackId, RackOutputMode, TRACK_LC_BASE,
};

use crate::codec::{deserialize_postcard, encode_versioned_section, serialize_postcard};
use crate::error::YinError;

/// 混音段格式版本（段内前 4 字节）。
///
/// 兼容策略：每次升级都必须保留至少一个旧版本解码器，不能让旧工程静默丢
/// 混音设置。方案 C 的混音段版本。
///
/// 只保留 v9（旧开发基线）和 v10（C）。v10 是当前结构；旧 v10..v15
/// 开发快照不再兼容，加载时会明确警告。
const MIXER_SECTION_VERSION: u32 = 10;

/// v9 混音参数快照：并行分表。
///
/// 方案 C 没有 Bus，`bus` 仅保留为裸 u16 以维持 v9 二进制布局；
/// 解码后这些总线发送直接丢弃。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct SendParamsV9 {
    bus: u16,
    amount: f32,
    pre_fader: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerParamsV9 {
    channels: Vec<StripParams>,
    master: MasterParams,
    channel_inserts: Vec<Vec<InsertRef>>,
    master_inserts: Vec<InsertRef>,
    instruments: Vec<Vec<InsertRef>>,
    next_instrument_uid: u64,
    audio_channels: Vec<StripParams>,
    audio_inserts: Vec<Vec<InsertRef>>,
    audio_sends: Vec<Vec<SendParamsV9>>,
    /// v9 的 Bus 字段只用于维持反序列化布局，解码后整体丢弃。
    #[allow(dead_code)]
    buses: Vec<StripParams>,
    #[allow(dead_code)]
    bus_ids: Vec<u16>,
    #[allow(dead_code)]
    next_bus_id: u16,
    #[allow(dead_code)]
    bus_inserts: Vec<Vec<InsertRef>>,
    #[allow(dead_code)]
    bus_sends: Vec<Vec<SendParamsV9>>,
    sends: Vec<Vec<SendParamsV9>>,
    midi_outputs: Vec<Option<u16>>,
    audio_outputs: Vec<Option<u16>>,
}

impl MixerParamsV9 {
    fn into_current(self) -> MixerParams {
        // 方案 C：LC ID 是独立 u16 身份，按分配顺序发放；角色写入 source 字段。
        let mut next_lc = TRACK_LC_BASE;
        let mut alloc = || {
            let id = next_lc;
            next_lc = next_lc.saturating_add(1);
            LogicalChannelId(id)
        };
        let audio_count = [
            self.audio_channels.len(),
            self.audio_inserts.len(),
            self.audio_sends.len(),
            self.audio_outputs.len(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);

        let racks = (0..CHANNEL_COUNT)
            .map(|i| RackParams {
                id: RackId(i as u16),
                instruments: self.instruments.get(i).cloned().unwrap_or_default(),
                output_mode: RackOutputMode::Shared,
            })
            .collect();
        let channels: Vec<MixerNode> = (0..CHANNEL_COUNT)
            .map(|i| MixerNode {
                id: ChannelId::Lc(alloc()),
                source: ChannelSource::Midi(i as u8),
                strip: self.channels.get(i).copied().unwrap_or_default(),
                inserts: self.channel_inserts.get(i).cloned().unwrap_or_default(),
                // 方案 C 没有 Bus：v9 的 bus 发送/主输出直接丢弃。
                sends: Vec::new(),
                output: None,
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let audio_channels: Vec<MixerNode> = (0..audio_count)
            .map(|i| MixerNode {
                id: ChannelId::Lc(alloc()),
                source: ChannelSource::Audio(i as u16),
                strip: self.audio_channels.get(i).copied().unwrap_or_default(),
                inserts: self.audio_inserts.get(i).cloned().unwrap_or_default(),
                sends: Vec::new(),
                output: None,
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let master = MixerNode {
            id: ChannelId::Master,
            source: ChannelSource::Master,
            strip: StripParams {
                gain: self.master.gain,
                ..StripParams::default()
            },
            inserts: self.master_inserts,
            sends: Vec::new(),
            output: None,
            automation: Vec::new(),
            freeze: None,
        };
        let nodes: Vec<MixerNode> = channels.into_iter().chain(audio_channels).collect();
        MixerParams {
            aps: Vec::new(),
            channel_sends: Vec::new(),
            racks,
            nodes,
            master,
            next_instrument_uid: self.next_instrument_uid.max(1),
            next_lc_id: LogicalChannelId(next_lc),
        }
    }
}

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
    let payload = zstd::decode_all(Cursor::new(&section[4..])).ok()?;
    let mut params = match version {
        10 => deserialize_postcard::<MixerParams>(&payload)
            .map_err(|e| tracing::warn!("v10 混音段解析失败，忽略混音设置: {e}"))
            .ok()?,
        // v9：最后开发基线；稳定 BusId + 并行分表，转成 C 结构。
        9 => deserialize_postcard::<MixerParamsV9>(&payload)
            .map_err(|e| tracing::warn!("v9 混音段解析失败，忽略混音设置: {e}"))
            .ok()?
            .into_current(),
        _ => {
            tracing::warn!("混音段版本 {version} 不受支持，忽略混音设置");
            return None;
        }
    };
    params.ensure_len();
    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_mixer::SendParams;

    #[test]
    fn v9_tables_migrate_to_v10_nodes() {
        let mut old = MixerParamsV9 {
            channels: vec![StripParams::default(); yinhe_mixer::CHANNEL_COUNT],
            master: MasterParams { gain: 0.8 },
            channel_inserts: vec![Vec::new(); yinhe_mixer::CHANNEL_COUNT],
            master_inserts: Vec::new(),
            instruments: vec![Vec::new(); yinhe_mixer::CHANNEL_COUNT],
            next_instrument_uid: 7,
            audio_channels: vec![StripParams {
                gain: 0.4,
                ..StripParams::default()
            }],
            audio_inserts: vec![Vec::new()],
            audio_sends: vec![Vec::new()],
            buses: vec![StripParams::default()],
            bus_ids: vec![5],
            next_bus_id: 6,
            bus_inserts: vec![Vec::new()],
            bus_sends: vec![Vec::new()],
            sends: vec![Vec::new(); yinhe_mixer::CHANNEL_COUNT],
            midi_outputs: vec![None; yinhe_mixer::CHANNEL_COUNT],
            audio_outputs: vec![None],
        };
        old.channels[3].gain = 0.25;
        old.sends[3].push(SendParamsV9 {
            bus: 5,
            amount: 0.5,
            pre_fader: false,
        });
        old.midi_outputs[3] = Some(5);

        let payload = serialize_postcard(&old).expect("serialize v9");
        let section = encode_versioned_section(9, &payload, 0).expect("encode v9");
        let loaded = decode_mixer_section(&section).expect("decode v9");

        assert_eq!(loaded.racks.len(), yinhe_mixer::CHANNEL_COUNT);
        assert_eq!(
            loaded
                .nodes()
                .filter(|node| matches!(node.source, ChannelSource::Midi(_)))
                .count(),
            yinhe_mixer::CHANNEL_COUNT
        );
        let midi3 = loaded.midi_node(3).expect("midi 3");
        assert_eq!(midi3.gain, 0.25);
        // 方案 C 没有 Bus：v9 的 bus 发送与主输出直接丢弃。
        assert!(midi3.sends.is_empty());
        assert!(midi3.output.is_none());
        assert_eq!(loaded.audio_node(0).expect("audio 0").gain, 0.4);
        assert_eq!(loaded.master.gain, 0.8);
        assert_eq!(loaded.next_instrument_uid, 7);
        // v9 没有每轨 LC：不应出现 source=Track 的节点。
        assert!(
            !loaded
                .nodes()
                .any(|node| matches!(node.source, ChannelSource::Track(_)))
        );
        // v9 迁移给 256 个 MIDI LC + 音频 LC 分配独立 u16 ID，next_lc_id 已推进。
        assert!(loaded.next_lc_id.0 > TRACK_LC_BASE);
        assert_eq!(loaded.racks[0].output_mode, RackOutputMode::Shared);
    }

    #[test]
    fn current_version_roundtrips_nodes() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        let track_lc = mixer.add_track_lc();
        let send = SendParams {
            target: ChannelId::Lc(track_lc),
            amount: 0.5,
            pre_fader: false,
        };
        mixer.set_node_sends(yinhe_types::MixerChannelId::Midi(0), vec![send]);
        mixer.set_node_output(
            yinhe_types::MixerChannelId::Midi(0),
            Some(ChannelId::Lc(track_lc)),
        );
        let node = mixer.track_node_mut(track_lc).expect("track lc");
        node.automation.push(yinhe_types::AutomationLane {
            target: yinhe_types::AutomationTarget::MixerGain {
                channel: yinhe_types::MixerChannelId::TrackLc(track_lc),
            },
            track: 0,
            events: vec![yinhe_types::AutomationEvent {
                id: 1,
                tick: 0,
                value: 0.75,
                shape: yinhe_types::SegmentShape::Step,
            }],
        });
        mixer.midi_node_mut(0).expect("midi 0").freeze = Some(yinhe_mixer::FrozenAudio {
            source: "frozen-lc".into(),
            source_revision: 7,
            sample_rate: 48_000,
            frames: 1234,
        });
        mixer.track_node_mut(track_lc).expect("track lc").strip.gain = 0.25;

        let section = encode_mixer_section(&mixer, 0).expect("encode");
        let loaded = decode_mixer_section(&section).expect("decode");
        assert_eq!(
            loaded.midi_node(0).expect("midi 0").sends[0].target,
            ChannelId::Lc(track_lc)
        );
        assert_eq!(
            loaded.midi_node(0).expect("midi 0").output,
            Some(ChannelId::Lc(track_lc))
        );
        assert_eq!(loaded.master.id, ChannelId::Master);
        assert_eq!(
            loaded.track_node(track_lc).expect("track lc").strip.gain,
            0.25
        );
        assert_eq!(
            loaded
                .track_node(track_lc)
                .expect("track lc")
                .automation
                .len(),
            1
        );
        assert_eq!(
            loaded
                .midi_node(0)
                .expect("midi 0")
                .freeze
                .as_ref()
                .map(|f| f.frames),
            Some(1234)
        );
    }
}
