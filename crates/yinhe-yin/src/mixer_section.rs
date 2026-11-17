//! 混音段（.yin 可选第 4 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;

use serde::{Deserialize, Serialize};
use yinhe_mixer::{
    CHANNEL_COUNT, InsertRef, MasterParams, MixerNode, MixerParams, RackParams, SendParams,
    StripParams,
};
use yinhe_types::{BusId, ChannelId, LogicalChannelId, RackId, RackOutputMode, TRACK_LC_BASE};

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

/// v9 混音参数快照：稳定 BusId，但仍是并行分表。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct SendParamsV9 {
    bus: BusId,
    amount: f32,
    pre_fader: bool,
}

impl SendParamsV9 {
    fn into_current(self) -> SendParams {
        SendParams {
            target: ChannelId::Bus(self.bus),
            amount: self.amount,
            pre_fader: self.pre_fader,
        }
    }
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
    buses: Vec<StripParams>,
    bus_ids: Vec<BusId>,
    next_bus_id: BusId,
    bus_inserts: Vec<Vec<InsertRef>>,
    bus_sends: Vec<Vec<SendParamsV9>>,
    sends: Vec<Vec<SendParamsV9>>,
    midi_outputs: Vec<Option<BusId>>,
    audio_outputs: Vec<Option<BusId>>,
}

impl MixerParamsV9 {
    fn into_current(self) -> MixerParams {
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
                id: ChannelId::Lc(LogicalChannelId(i as u32)),
                strip: self.channels.get(i).copied().unwrap_or_default(),
                inserts: self.channel_inserts.get(i).cloned().unwrap_or_default(),
                sends: self
                    .sends
                    .get(i)
                    .map(|list| {
                        list.iter()
                            .copied()
                            .map(SendParamsV9::into_current)
                            .collect()
                    })
                    .unwrap_or_default(),
                output: self
                    .midi_outputs
                    .get(i)
                    .copied()
                    .flatten()
                    .map(ChannelId::Bus),
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let audio_channels: Vec<MixerNode> = (0..audio_count)
            .map(|i| MixerNode {
                id: ChannelId::Lc(LogicalChannelId(CHANNEL_COUNT as u32 + i as u32)),
                strip: self.audio_channels.get(i).copied().unwrap_or_default(),
                inserts: self.audio_inserts.get(i).cloned().unwrap_or_default(),
                sends: self
                    .audio_sends
                    .get(i)
                    .map(|list| {
                        list.iter()
                            .copied()
                            .map(SendParamsV9::into_current)
                            .collect()
                    })
                    .unwrap_or_default(),
                output: self
                    .audio_outputs
                    .get(i)
                    .copied()
                    .flatten()
                    .map(ChannelId::Bus),
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let buses: Vec<MixerNode> = self
            .buses
            .iter()
            .enumerate()
            .map(|(i, strip)| MixerNode {
                id: ChannelId::Bus(
                    self.bus_ids
                        .get(i)
                        .copied()
                        .unwrap_or(BusId(i.min(u16::MAX as usize) as u16)),
                ),
                strip: *strip,
                inserts: self.bus_inserts.get(i).cloned().unwrap_or_default(),
                sends: self
                    .bus_sends
                    .get(i)
                    .map(|list| {
                        list.iter()
                            .copied()
                            .map(SendParamsV9::into_current)
                            .collect()
                    })
                    .unwrap_or_default(),
                output: None,
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let master = MixerNode {
            id: ChannelId::Master,
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
        let nodes: Vec<MixerNode> = channels
            .into_iter()
            .chain(audio_channels)
            .chain(buses)
            .collect();
        MixerParams {
            aps: Vec::new(),
            channel_sends: Vec::new(),
            racks,
            nodes,
            master,
            next_instrument_uid: self.next_instrument_uid.max(1),
            next_bus_id: self.next_bus_id,
            next_lc_id: LogicalChannelId(TRACK_LC_BASE),
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
            bus_ids: vec![BusId(5)],
            next_bus_id: BusId(6),
            bus_inserts: vec![Vec::new()],
            bus_sends: vec![Vec::new()],
            sends: vec![Vec::new(); yinhe_mixer::CHANNEL_COUNT],
            midi_outputs: vec![None; yinhe_mixer::CHANNEL_COUNT],
            audio_outputs: vec![None],
        };
        old.channels[3].gain = 0.25;
        old.sends[3].push(SendParamsV9 {
            bus: BusId(5),
            amount: 0.5,
            pre_fader: false,
        });
        old.midi_outputs[3] = Some(BusId(5));

        let payload = serialize_postcard(&old).expect("serialize v9");
        let section = encode_versioned_section(9, &payload, 0).expect("encode v9");
        let loaded = decode_mixer_section(&section).expect("decode v9");

        assert_eq!(loaded.racks.len(), yinhe_mixer::CHANNEL_COUNT);
        assert_eq!(
            loaded
                .nodes()
                .filter(|node| matches!(node.id, ChannelId::Lc(id) if id.0 < yinhe_mixer::CHANNEL_COUNT as u32))
                .count(),
            yinhe_mixer::CHANNEL_COUNT
        );
        let midi3 = loaded.midi_node(3).expect("midi 3");
        assert_eq!(midi3.gain, 0.25);
        assert_eq!(midi3.sends[0].target, ChannelId::Bus(BusId(5)));
        assert_eq!(midi3.output, Some(ChannelId::Bus(BusId(5))));
        assert_eq!(loaded.audio_node(0).expect("audio 0").gain, 0.4);
        assert_eq!(loaded.bus_id_at(0), Some(BusId(5)));
        assert_eq!(loaded.next_bus_id, BusId(6));
        assert_eq!(loaded.master.gain, 0.8);
        assert_eq!(loaded.next_instrument_uid, 7);
        assert!(
            !loaded
                .nodes()
                .any(|node| matches!(node.id, ChannelId::Lc(id) if id.0 >= TRACK_LC_BASE))
        );
        assert_eq!(loaded.next_lc_id, LogicalChannelId(TRACK_LC_BASE));
        assert_eq!(loaded.racks[0].output_mode, RackOutputMode::Shared);
    }

    #[test]
    fn current_version_roundtrips_nodes() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        let bus = mixer.add_bus().expect("bus");
        let send = SendParams {
            target: ChannelId::Bus(bus),
            amount: 0.5,
            pre_fader: false,
        };
        mixer.set_node_sends(yinhe_types::MixerChannelId::Midi(0), vec![send]);
        mixer.set_node_output(
            yinhe_types::MixerChannelId::Midi(0),
            Some(ChannelId::Bus(bus)),
        );
        let bus_node = mixer.bus_node_mut(bus).expect("bus");
        bus_node.automation.push(yinhe_types::AutomationLane {
            target: yinhe_types::AutomationTarget::BusGain { bus },
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
        let track_lc = mixer.add_track_lc();
        mixer.track_node_mut(track_lc).expect("track lc").strip.gain = 0.25;

        let section = encode_mixer_section(&mixer, 0).expect("encode");
        let loaded = decode_mixer_section(&section).expect("decode");
        assert_eq!(loaded.bus_count(), 1);
        assert_eq!(loaded.bus_id_at(0), Some(bus));
        assert_eq!(
            loaded.midi_node(0).expect("midi 0").sends[0].target,
            ChannelId::Bus(bus)
        );
        assert_eq!(
            loaded.midi_node(0).expect("midi 0").output,
            Some(ChannelId::Bus(bus))
        );
        assert_eq!(loaded.master.id, ChannelId::Master);
        assert_eq!(
            loaded.track_node(track_lc).expect("track lc").strip.gain,
            0.25
        );
    }
}
