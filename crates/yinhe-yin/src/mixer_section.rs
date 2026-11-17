//! 混音段（.yin 可选第 4 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;

use serde::{Deserialize, Serialize};
use yinhe_mixer::{
    CHANNEL_COUNT, InsertRef, MasterParams, MixerNode, MixerParams, RackParams, SendParams,
    StripParams,
};
use yinhe_types::{BusId, LogicalChannelId, MixerNodeId, RackId, RackOutputMode, TRACK_LC_BASE};

use crate::codec::{deserialize_postcard, encode_versioned_section, serialize_postcard};
use crate::error::YinError;

/// 混音段格式版本（段内前 4 字节）。
///
/// 兼容策略：每次升级都必须保留至少一个旧版本解码器，不能让旧工程静默丢
/// 混音设置。当前支持 v8 / v9 / v10：
/// - v8：总线是 Vec 下标；
/// - v9：稳定 BusId，但仍是并行分表；
/// - v10：Rack + MixerNode 节点表；
/// - v11：每轨 LC（track_channels / next_lc_id）与 Rack 输出拓扑；
/// - v12：MixerNode 拥有自己的 automation（BusGain 不再寄宿 Master）；
/// - v13：LC freeze 音频引用；
/// - v14：Rack 插件输出拓扑与按输出 LC。
const MIXER_SECTION_VERSION: u32 = 14;

/// v8 及更早的发送参数（总线是 Vec 下标，没有稳定 ID）。
/// postcard 非自描述，字段顺序必须与旧 `MixerParams` 完全一致。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SendParamsV8 {
    bus: u16,
    amount: f32,
    pre_fader: bool,
}

/// v8 混音参数快照，只用于旧工程迁移。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerParamsV8 {
    channels: Vec<StripParams>,
    master: MasterParams,
    channel_inserts: Vec<Vec<InsertRef>>,
    master_inserts: Vec<InsertRef>,
    instruments: Vec<Vec<InsertRef>>,
    next_instrument_uid: u64,
    audio_channels: Vec<StripParams>,
    audio_inserts: Vec<Vec<InsertRef>>,
    audio_sends: Vec<Vec<SendParamsV8>>,
    buses: Vec<StripParams>,
    bus_inserts: Vec<Vec<InsertRef>>,
    bus_sends: Vec<Vec<SendParamsV8>>,
    sends: Vec<Vec<SendParamsV8>>,
    midi_outputs: Vec<Option<u16>>,
    audio_outputs: Vec<Option<u16>>,
}

/// v9 混音参数快照：稳定 BusId，但仍是并行分表。
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
    audio_sends: Vec<Vec<SendParams>>,
    buses: Vec<StripParams>,
    bus_ids: Vec<BusId>,
    next_bus_id: BusId,
    bus_inserts: Vec<Vec<InsertRef>>,
    bus_sends: Vec<Vec<SendParams>>,
    sends: Vec<Vec<SendParams>>,
    midi_outputs: Vec<Option<BusId>>,
    audio_outputs: Vec<Option<BusId>>,
}

/// v10 的 Rack 快照（还没有 output_mode）。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RackParamsV10 {
    id: RackId,
    instruments: Vec<InsertRef>,
}

/// v10 的 MixerNode 快照（冻结字段顺序，不能直接用当前结构）。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerNodeV10 {
    id: MixerNodeId,
    strip: StripParams,
    inserts: Vec<InsertRef>,
    sends: Vec<SendParams>,
    output: Option<BusId>,
}

impl MixerNodeV10 {
    fn into_current(self) -> MixerNode {
        MixerNode {
            id: self.id,
            strip: self.strip,
            inserts: self.inserts,
            sends: self.sends,
            output: self.output,
            automation: Vec::new(),
            freeze: None,
        }
    }
}

/// v10 混音参数快照：Rack + MixerNode 节点表，还没有每轨 LC。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerParamsV10 {
    racks: Vec<RackParamsV10>,
    channels: Vec<MixerNodeV10>,
    audio_channels: Vec<MixerNodeV10>,
    buses: Vec<MixerNodeV10>,
    master: MixerNodeV10,
    next_instrument_uid: u64,
    next_bus_id: BusId,
}

impl MixerParamsV10 {
    fn into_current(self) -> MixerParams {
        MixerParams {
            aps: Vec::new(),
            racks: self
                .racks
                .into_iter()
                .map(|rack| RackParams {
                    id: rack.id,
                    instruments: rack.instruments,
                    output_mode: RackOutputMode::Shared,
                    outputs: yinhe_types::RackOutputTopology::default(),
                    output_lcs: Vec::new(),
                })
                .collect(),
            channels: self
                .channels
                .into_iter()
                .map(MixerNodeV10::into_current)
                .collect(),
            track_channels: Vec::new(),
            audio_channels: self
                .audio_channels
                .into_iter()
                .map(MixerNodeV10::into_current)
                .collect(),
            buses: self
                .buses
                .into_iter()
                .map(MixerNodeV10::into_current)
                .collect(),
            master: self.master.into_current(),
            next_instrument_uid: self.next_instrument_uid,
            next_bus_id: self.next_bus_id,
            next_lc_id: LogicalChannelId(TRACK_LC_BASE),
        }
    }
}

/// v11 的 Rack 快照。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RackParamsV11 {
    id: RackId,
    instruments: Vec<InsertRef>,
    output_mode: RackOutputMode,
}

/// v11 的 MixerNode 快照（还没有节点 automation）。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerNodeV11 {
    id: MixerNodeId,
    strip: StripParams,
    inserts: Vec<InsertRef>,
    sends: Vec<SendParams>,
    output: Option<BusId>,
}

impl MixerNodeV11 {
    fn into_current(self) -> MixerNode {
        MixerNode {
            id: self.id,
            strip: self.strip,
            inserts: self.inserts,
            sends: self.sends,
            output: self.output,
            automation: Vec::new(),
            freeze: None,
        }
    }
}

/// v11 混音参数快照：已有每轨 LC，但 BusGain 仍在 Master 轨。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerParamsV11 {
    racks: Vec<RackParamsV11>,
    channels: Vec<MixerNodeV11>,
    track_channels: Vec<MixerNodeV11>,
    audio_channels: Vec<MixerNodeV11>,
    buses: Vec<MixerNodeV11>,
    master: MixerNodeV11,
    next_instrument_uid: u64,
    next_bus_id: BusId,
    next_lc_id: LogicalChannelId,
}

impl MixerParamsV11 {
    fn into_current(self) -> MixerParams {
        MixerParams {
            aps: Vec::new(),
            racks: self
                .racks
                .into_iter()
                .map(|rack| RackParams {
                    id: rack.id,
                    instruments: rack.instruments,
                    output_mode: rack.output_mode,
                    outputs: yinhe_types::RackOutputTopology::default(),
                    output_lcs: Vec::new(),
                })
                .collect(),
            channels: self
                .channels
                .into_iter()
                .map(MixerNodeV11::into_current)
                .collect(),
            track_channels: self
                .track_channels
                .into_iter()
                .map(MixerNodeV11::into_current)
                .collect(),
            audio_channels: self
                .audio_channels
                .into_iter()
                .map(MixerNodeV11::into_current)
                .collect(),
            buses: self
                .buses
                .into_iter()
                .map(MixerNodeV11::into_current)
                .collect(),
            master: self.master.into_current(),
            next_instrument_uid: self.next_instrument_uid,
            next_bus_id: self.next_bus_id,
            next_lc_id: self.next_lc_id,
        }
    }
}

/// v12 的 Rack 快照。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RackParamsV12 {
    id: RackId,
    instruments: Vec<InsertRef>,
    output_mode: RackOutputMode,
}

/// v12 的 MixerNode 快照（已有 automation，还没有 freeze）。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerNodeV12 {
    id: MixerNodeId,
    strip: StripParams,
    inserts: Vec<InsertRef>,
    sends: Vec<SendParams>,
    output: Option<BusId>,
    automation: Vec<yinhe_types::AutomationLane>,
}

impl MixerNodeV12 {
    fn into_current(self) -> MixerNode {
        MixerNode {
            id: self.id,
            strip: self.strip,
            inserts: self.inserts,
            sends: self.sends,
            output: self.output,
            automation: self.automation,
            freeze: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerParamsV12 {
    racks: Vec<RackParamsV12>,
    channels: Vec<MixerNodeV12>,
    track_channels: Vec<MixerNodeV12>,
    audio_channels: Vec<MixerNodeV12>,
    buses: Vec<MixerNodeV12>,
    master: MixerNodeV12,
    next_instrument_uid: u64,
    next_bus_id: BusId,
    next_lc_id: LogicalChannelId,
}

impl MixerParamsV12 {
    fn into_current(self) -> MixerParams {
        MixerParams {
            aps: Vec::new(),
            racks: self
                .racks
                .into_iter()
                .map(|rack| RackParams {
                    id: rack.id,
                    instruments: rack.instruments,
                    output_mode: rack.output_mode,
                    outputs: yinhe_types::RackOutputTopology::default(),
                    output_lcs: Vec::new(),
                })
                .collect(),
            channels: self
                .channels
                .into_iter()
                .map(MixerNodeV12::into_current)
                .collect(),
            track_channels: self
                .track_channels
                .into_iter()
                .map(MixerNodeV12::into_current)
                .collect(),
            audio_channels: self
                .audio_channels
                .into_iter()
                .map(MixerNodeV12::into_current)
                .collect(),
            buses: self
                .buses
                .into_iter()
                .map(MixerNodeV12::into_current)
                .collect(),
            master: self.master.into_current(),
            next_instrument_uid: self.next_instrument_uid,
            next_bus_id: self.next_bus_id,
            next_lc_id: self.next_lc_id,
        }
    }
}

/// v13 的 Rack 快照（还没有插件输出拓扑）。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RackParamsV13 {
    id: RackId,
    instruments: Vec<InsertRef>,
    output_mode: RackOutputMode,
}

/// v13 的 MixerNode 快照（与当前节点一致，仅用于冻结旧字段顺序）。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerNodeV13 {
    id: MixerNodeId,
    strip: StripParams,
    inserts: Vec<InsertRef>,
    sends: Vec<SendParams>,
    output: Option<BusId>,
    automation: Vec<yinhe_types::AutomationLane>,
    freeze: Option<yinhe_mixer::FrozenAudio>,
}

impl MixerNodeV13 {
    fn into_current(self) -> MixerNode {
        MixerNode {
            id: self.id,
            strip: self.strip,
            inserts: self.inserts,
            sends: self.sends,
            output: self.output,
            automation: self.automation,
            freeze: self.freeze,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MixerParamsV13 {
    racks: Vec<RackParamsV13>,
    channels: Vec<MixerNodeV13>,
    track_channels: Vec<MixerNodeV13>,
    audio_channels: Vec<MixerNodeV13>,
    buses: Vec<MixerNodeV13>,
    master: MixerNodeV13,
    next_instrument_uid: u64,
    next_bus_id: BusId,
    next_lc_id: LogicalChannelId,
}

impl MixerParamsV13 {
    fn into_current(self) -> MixerParams {
        MixerParams {
            aps: Vec::new(),
            racks: self
                .racks
                .into_iter()
                .map(|rack| RackParams {
                    id: rack.id,
                    instruments: rack.instruments,
                    output_mode: rack.output_mode,
                    outputs: yinhe_types::RackOutputTopology::default(),
                    output_lcs: Vec::new(),
                })
                .collect(),
            channels: self
                .channels
                .into_iter()
                .map(MixerNodeV13::into_current)
                .collect(),
            track_channels: self
                .track_channels
                .into_iter()
                .map(MixerNodeV13::into_current)
                .collect(),
            audio_channels: self
                .audio_channels
                .into_iter()
                .map(MixerNodeV13::into_current)
                .collect(),
            buses: self
                .buses
                .into_iter()
                .map(MixerNodeV13::into_current)
                .collect(),
            master: self.master.into_current(),
            next_instrument_uid: self.next_instrument_uid,
            next_bus_id: self.next_bus_id,
            next_lc_id: self.next_lc_id,
        }
    }
}

impl MixerParamsV8 {
    fn into_v9(self) -> MixerParamsV9 {
        let bus_ids: Vec<BusId> = (0..self.buses.len())
            .map(|i| BusId(i.min(u16::MAX as usize) as u16))
            .collect();
        let next_bus_id = BusId(self.buses.len().min(u16::MAX as usize) as u16);
        let convert = |s: SendParamsV8| SendParams {
            bus: BusId(s.bus),
            amount: s.amount,
            pre_fader: s.pre_fader,
        };
        MixerParamsV9 {
            channels: self.channels,
            master: self.master,
            channel_inserts: self.channel_inserts,
            master_inserts: self.master_inserts,
            instruments: self.instruments,
            next_instrument_uid: self.next_instrument_uid,
            audio_channels: self.audio_channels,
            audio_inserts: self.audio_inserts,
            audio_sends: self
                .audio_sends
                .into_iter()
                .map(|list| list.into_iter().map(convert).collect())
                .collect(),
            buses: self.buses,
            bus_ids,
            next_bus_id,
            bus_inserts: self.bus_inserts,
            bus_sends: self
                .bus_sends
                .into_iter()
                .map(|list| list.into_iter().map(convert).collect())
                .collect(),
            sends: self
                .sends
                .into_iter()
                .map(|list| list.into_iter().map(convert).collect())
                .collect(),
            midi_outputs: self
                .midi_outputs
                .into_iter()
                .map(|b| b.map(BusId))
                .collect(),
            audio_outputs: self
                .audio_outputs
                .into_iter()
                .map(|b| b.map(BusId))
                .collect(),
        }
    }
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
                outputs: yinhe_types::RackOutputTopology::default(),
                output_lcs: Vec::new(),
            })
            .collect();
        let channels = (0..CHANNEL_COUNT)
            .map(|i| MixerNode {
                id: MixerNodeId::LogicalChannel(LogicalChannelId(i as u32)),
                strip: self.channels.get(i).copied().unwrap_or_default(),
                inserts: self.channel_inserts.get(i).cloned().unwrap_or_default(),
                sends: self.sends.get(i).cloned().unwrap_or_default(),
                output: self.midi_outputs.get(i).copied().flatten(),
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let audio_channels = (0..audio_count)
            .map(|i| MixerNode {
                id: MixerNodeId::LogicalChannel(LogicalChannelId(CHANNEL_COUNT as u32 + i as u32)),
                strip: self.audio_channels.get(i).copied().unwrap_or_default(),
                inserts: self.audio_inserts.get(i).cloned().unwrap_or_default(),
                sends: self.audio_sends.get(i).cloned().unwrap_or_default(),
                output: self.audio_outputs.get(i).copied().flatten(),
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let buses = self
            .buses
            .iter()
            .enumerate()
            .map(|(i, strip)| MixerNode {
                id: MixerNodeId::Bus(
                    self.bus_ids
                        .get(i)
                        .copied()
                        .unwrap_or(BusId(i.min(u16::MAX as usize) as u16)),
                ),
                strip: *strip,
                inserts: self.bus_inserts.get(i).cloned().unwrap_or_default(),
                sends: self.bus_sends.get(i).cloned().unwrap_or_default(),
                output: None,
                automation: Vec::new(),
                freeze: None,
            })
            .collect();
        let master = MixerNode {
            id: MixerNodeId::Master,
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
        MixerParams {
            aps: Vec::new(),
            racks,
            channels,
            track_channels: Vec::new(),
            audio_channels,
            buses,
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
        MIXER_SECTION_VERSION => deserialize_postcard::<MixerParams>(&payload)
            .map_err(|e| tracing::warn!("混音段解析失败，忽略混音设置: {e}"))
            .ok()?,
        // v13：已有 freeze，还没有 Rack 输出拓扑。
        13 => deserialize_postcard::<MixerParamsV13>(&payload)
            .map_err(|e| tracing::warn!("v13 混音段解析失败，忽略混音设置: {e}"))
            .ok()?
            .into_current(),
        // v12：已有节点 automation，还没有 freeze。
        12 => deserialize_postcard::<MixerParamsV12>(&payload)
            .map_err(|e| tracing::warn!("v12 混音段解析失败，忽略混音设置: {e}"))
            .ok()?
            .into_current(),
        // v11：已有每轨 LC，但节点还没有 automation。
        11 => deserialize_postcard::<MixerParamsV11>(&payload)
            .map_err(|e| tracing::warn!("v11 混音段解析失败，忽略混音设置: {e}"))
            .ok()?
            .into_current(),
        // v10：Rack + MixerNode，还没有每轨 LC。
        10 => deserialize_postcard::<MixerParamsV10>(&payload)
            .map_err(|e| tracing::warn!("v10 混音段解析失败，忽略混音设置: {e}"))
            .ok()?
            .into_current(),
        // v9：稳定 BusId + 并行分表；转成 Rack + MixerNode。
        9 => deserialize_postcard::<MixerParamsV9>(&payload)
            .map_err(|e| tracing::warn!("v9 混音段解析失败，忽略混音设置: {e}"))
            .ok()?
            .into_current(),
        // v8：总线还是 Vec 下标；先升 v9，再升 v10。
        8 => deserialize_postcard::<MixerParamsV8>(&payload)
            .map_err(|e| tracing::warn!("v8 混音段解析失败，忽略混音设置: {e}"))
            .ok()?
            .into_v9()
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
    fn v8_bus_indices_migrate_to_v10_nodes() {
        let old = MixerParamsV8 {
            channels: vec![StripParams::default(); yinhe_mixer::CHANNEL_COUNT],
            master: MasterParams::default(),
            channel_inserts: vec![Vec::new(); yinhe_mixer::CHANNEL_COUNT],
            master_inserts: Vec::new(),
            instruments: vec![Vec::new(); yinhe_mixer::CHANNEL_COUNT],
            next_instrument_uid: 1,
            audio_channels: Vec::new(),
            audio_inserts: Vec::new(),
            audio_sends: vec![vec![SendParamsV8 {
                bus: 1,
                amount: 0.5,
                pre_fader: true,
            }]],
            buses: vec![StripParams::default(); 2],
            bus_inserts: vec![Vec::new(); 2],
            bus_sends: vec![Vec::new(); 2],
            sends: vec![vec![SendParamsV8 {
                bus: 1,
                amount: 1.0,
                pre_fader: false,
            }]],
            midi_outputs: vec![Some(0)],
            audio_outputs: vec![Some(1)],
        };
        let payload = serialize_postcard(&old).expect("serialize v8");
        let section = encode_versioned_section(8, &payload, 0).expect("encode v8");

        let loaded = decode_mixer_section(&section).expect("decode v8");
        assert_eq!(loaded.bus_count(), 2);
        assert_eq!(loaded.bus_id_at(0), Some(BusId(0)));
        assert_eq!(loaded.bus_id_at(1), Some(BusId(1)));
        assert_eq!(loaded.next_bus_id, BusId(2));
        assert_eq!(loaded.channels[0].sends[0].bus, BusId(1));
        assert_eq!(loaded.audio_channels[0].sends[0].bus, BusId(1));
        assert_eq!(loaded.channels[0].output, Some(BusId(0)));
        assert_eq!(loaded.audio_channels[0].output, Some(BusId(1)));
    }

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
        old.sends[3].push(SendParams {
            bus: BusId(5),
            amount: 0.5,
            pre_fader: false,
        });
        old.midi_outputs[3] = Some(BusId(5));

        let payload = serialize_postcard(&old).expect("serialize v9");
        let section = encode_versioned_section(9, &payload, 0).expect("encode v9");
        let loaded = decode_mixer_section(&section).expect("decode v9");

        assert_eq!(loaded.racks.len(), yinhe_mixer::CHANNEL_COUNT);
        assert_eq!(loaded.channels.len(), yinhe_mixer::CHANNEL_COUNT);
        assert_eq!(loaded.channels[3].gain, 0.25);
        assert_eq!(loaded.channels[3].sends[0].bus, BusId(5));
        assert_eq!(loaded.channels[3].output, Some(BusId(5)));
        assert_eq!(loaded.audio_channels.len(), 1);
        assert_eq!(loaded.audio_channels[0].gain, 0.4);
        assert_eq!(loaded.bus_id_at(0), Some(BusId(5)));
        assert_eq!(loaded.next_bus_id, BusId(6));
        assert_eq!(loaded.master.gain, 0.8);
        assert_eq!(loaded.next_instrument_uid, 7);
        assert!(loaded.track_channels.is_empty());
        assert_eq!(loaded.next_lc_id, LogicalChannelId(TRACK_LC_BASE));
        assert_eq!(loaded.racks[0].output_mode, RackOutputMode::Shared);
    }

    #[test]
    fn v10_nodes_migrate_to_v11_track_lc_support() {
        let old = MixerParamsV10 {
            racks: vec![RackParamsV10 {
                id: RackId(0),
                instruments: Vec::new(),
            }],
            channels: vec![MixerNodeV10 {
                id: MixerNodeId::LogicalChannel(LogicalChannelId(0)),
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
            }],
            audio_channels: vec![MixerNodeV10 {
                id: MixerNodeId::LogicalChannel(LogicalChannelId(256)),
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
            }],
            buses: vec![MixerNodeV10 {
                id: MixerNodeId::Bus(BusId(1)),
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
            }],
            master: MixerNodeV10 {
                id: MixerNodeId::Master,
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
            },
            next_instrument_uid: 3,
            next_bus_id: BusId(2),
        };
        let payload = serialize_postcard(&old).expect("serialize v10");
        let section = encode_versioned_section(10, &payload, 0).expect("encode v10");
        let loaded = decode_mixer_section(&section).expect("decode v10");

        assert_eq!(loaded.racks[0].output_mode, RackOutputMode::Shared);
        assert!(loaded.track_channels.is_empty());
        assert_eq!(loaded.next_lc_id, LogicalChannelId(TRACK_LC_BASE));
        assert_eq!(loaded.bus_id_at(0), Some(BusId(1)));
        assert_eq!(loaded.next_instrument_uid, 3);
    }

    #[test]
    fn v11_nodes_migrate_to_v12_with_empty_automation() {
        let old = MixerParamsV11 {
            racks: vec![RackParamsV11 {
                id: RackId(0),
                instruments: Vec::new(),
                output_mode: RackOutputMode::Shared,
            }],
            channels: vec![MixerNodeV11 {
                id: MixerNodeId::LogicalChannel(LogicalChannelId(0)),
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
            }],
            track_channels: Vec::new(),
            audio_channels: Vec::new(),
            buses: vec![MixerNodeV11 {
                id: MixerNodeId::Bus(BusId(3)),
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
            }],
            master: MixerNodeV11 {
                id: MixerNodeId::Master,
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
            },
            next_instrument_uid: 1,
            next_bus_id: BusId(4),
            next_lc_id: LogicalChannelId(TRACK_LC_BASE),
        };
        let payload = serialize_postcard(&old).expect("serialize v11");
        let section = encode_versioned_section(11, &payload, 0).expect("encode v11");
        let loaded = decode_mixer_section(&section).expect("decode v11");

        let bus = loaded.bus_index(BusId(3)).expect("bus");
        assert!(loaded.buses[bus].automation.is_empty());
        assert_eq!(loaded.next_bus_id, BusId(4));
    }

    #[test]
    fn v12_nodes_migrate_to_v13_without_freeze() {
        let old = MixerParamsV12 {
            racks: vec![RackParamsV12 {
                id: RackId(0),
                instruments: Vec::new(),
                output_mode: RackOutputMode::Shared,
            }],
            channels: vec![MixerNodeV12 {
                id: MixerNodeId::LogicalChannel(LogicalChannelId(0)),
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
                automation: Vec::new(),
            }],
            track_channels: Vec::new(),
            audio_channels: Vec::new(),
            buses: Vec::new(),
            master: MixerNodeV12 {
                id: MixerNodeId::Master,
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
                automation: Vec::new(),
            },
            next_instrument_uid: 1,
            next_bus_id: BusId::default(),
            next_lc_id: LogicalChannelId(TRACK_LC_BASE),
        };
        let payload = serialize_postcard(&old).expect("serialize v12");
        let section = encode_versioned_section(12, &payload, 0).expect("encode v12");
        let loaded = decode_mixer_section(&section).expect("decode v12");
        assert_eq!(loaded.channels[0].freeze, None);
    }

    #[test]
    fn v13_nodes_migrate_to_v14_without_output_topology() {
        let old = MixerParamsV13 {
            racks: vec![RackParamsV13 {
                id: RackId(0),
                instruments: Vec::new(),
                output_mode: RackOutputMode::Shared,
            }],
            channels: vec![MixerNodeV13 {
                id: MixerNodeId::LogicalChannel(LogicalChannelId(0)),
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
                automation: Vec::new(),
                freeze: None,
            }],
            track_channels: Vec::new(),
            audio_channels: Vec::new(),
            buses: Vec::new(),
            master: MixerNodeV13 {
                id: MixerNodeId::Master,
                strip: StripParams::default(),
                inserts: Vec::new(),
                sends: Vec::new(),
                output: None,
                automation: Vec::new(),
                freeze: None,
            },
            next_instrument_uid: 1,
            next_bus_id: BusId::default(),
            next_lc_id: LogicalChannelId(TRACK_LC_BASE),
        };
        let payload = serialize_postcard(&old).expect("serialize v13");
        let section = encode_versioned_section(13, &payload, 0).expect("encode v13");
        let loaded = decode_mixer_section(&section).expect("decode v13");
        assert!(loaded.racks[0].outputs.outputs.is_empty());
        assert!(loaded.racks[0].output_lcs.is_empty());
        assert_eq!(loaded.racks[0].output_mode, RackOutputMode::Shared);
    }

    #[test]
    fn current_version_roundtrips_nodes() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        let bus = mixer.add_bus().expect("bus");
        let send = SendParams {
            bus,
            amount: 0.5,
            pre_fader: false,
        };
        mixer.set_node_sends(yinhe_types::MixerChannelId::Midi(0), vec![send]);
        mixer.set_node_output(yinhe_types::MixerChannelId::Midi(0), Some(bus));
        let bus_node = mixer.buses.first_mut().expect("bus");
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
        mixer.channels[0].freeze = Some(yinhe_mixer::FrozenAudio {
            source: "frozen-lc".into(),
            source_revision: 7,
            sample_rate: 48_000,
            frames: 1234,
        });
        let out_a = mixer.add_track_lc();
        let out_b = mixer.add_track_lc();
        mixer.racks[0].outputs.outputs = vec![
            yinhe_types::RackOutput {
                name: "Out 1".into(),
                channels: 2,
            },
            yinhe_types::RackOutput {
                name: "Out 2".into(),
                channels: 2,
            },
        ];
        mixer.racks[0].output_lcs = vec![out_a, out_b];
        mixer.racks[0].output_mode = RackOutputMode::PerOutput;
        let track_lc = mixer.add_track_lc();
        mixer.track_node_mut(track_lc).expect("track lc").strip.gain = 0.25;

        let section = encode_mixer_section(&mixer, 0).expect("encode");
        let loaded = decode_mixer_section(&section).expect("decode");
        assert_eq!(loaded.bus_count(), 1);
        assert_eq!(loaded.bus_id_at(0), Some(bus));
        assert_eq!(loaded.channels[0].sends[0].bus, bus);
        assert_eq!(loaded.channels[0].output, Some(bus));
        assert_eq!(loaded.master.id, MixerNodeId::Master);
        assert_eq!(
            loaded.track_node(track_lc).expect("track lc").strip.gain,
            0.25
        );
    }
}
