//! 混音段（.yin 可选第 4 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;

use serde::{Deserialize, Serialize};
use yinhe_mixer::{InsertRef, MasterParams, MixerParams, SendParams, StripParams};
use yinhe_types::BusId;

use crate::codec::{deserialize_postcard, encode_versioned_section, serialize_postcard};
use crate::error::YinError;

/// 混音段格式版本（段内前 4 字节）。MixerParams 字段演进时递增，
/// 加载侧版本不符则忽略混音段（工程本体照常打开）。
const MIXER_SECTION_VERSION: u32 = 9;

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

impl MixerParamsV8 {
    fn into_current(self) -> MixerParams {
        let bus_ids: Vec<BusId> = (0..self.buses.len())
            .map(|i| BusId(i.min(u16::MAX as usize) as u16))
            .collect();
        let next_bus_id = BusId(self.buses.len().min(u16::MAX as usize) as u16);
        let convert = |s: SendParamsV8| SendParams {
            bus: BusId(s.bus),
            amount: s.amount,
            pre_fader: s.pre_fader,
        };
        MixerParams {
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
        // v8：总线还是 Vec 下标；按原索引生成稳定 BusId，行为不变。
        8 => deserialize_postcard::<MixerParamsV8>(&payload)
            .map_err(|e| tracing::warn!("v8 混音段解析失败，忽略混音设置: {e}"))
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
    fn v8_bus_indices_migrate_to_stable_ids() {
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
        assert_eq!(loaded.bus_ids, vec![BusId(0), BusId(1)]);
        assert_eq!(loaded.next_bus_id, BusId(2));
        assert_eq!(loaded.sends[0][0].bus, BusId(1));
        assert_eq!(loaded.audio_sends[0][0].bus, BusId(1));
        assert_eq!(loaded.midi_outputs[0], Some(BusId(0)));
        assert_eq!(loaded.audio_outputs[0], Some(BusId(1)));
    }

    #[test]
    fn current_version_roundtrips_stable_ids() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        let bus = mixer.add_bus().expect("bus");
        mixer.sends[0] = vec![SendParams {
            bus,
            amount: 0.5,
            pre_fader: false,
        }];
        mixer.midi_outputs[0] = Some(bus);

        let section = encode_mixer_section(&mixer, 0).expect("encode");
        let loaded = decode_mixer_section(&section).expect("decode");
        assert_eq!(loaded.bus_ids, vec![bus]);
        assert_eq!(loaded.sends[0][0].bus, bus);
        assert_eq!(loaded.midi_outputs[0], Some(bus));
    }
}
