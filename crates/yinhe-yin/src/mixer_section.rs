//! 混音段（.yin 可选第 4 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;

use yinhe_mixer::MixerParams;

use crate::codec::{deserialize_postcard, encode_versioned_section, serialize_postcard};
use crate::error::YinError;

/// 混音段格式版本（段内前 4 字节）。MixerParams 字段演进时递增，
/// 加载侧版本不符则忽略混音段（工程本体照常打开）。
///
/// v2：新增 instruments（当时按乐器通道索引）。加载 v1（无该字段）时按旧结构
/// 解码并补空；v5 起 instruments 改为按 MIDI 通道索引（旧语义作废，迁移时清空）。
///
/// v3：新增 buses / bus_inserts / sends（总线与发送）。加载 v1/v2 时补空。
///
/// v4：新增乐器通道 strip/inserts/sends 与音频通道 strip/inserts/sends
///（乐器通道命名空间已废弃，v4 不再支持）。
///
/// v5：乐器挂载统一到 MIDI 通道：删除 instrument_strips/inserts/sends，
/// instruments 固定 CHANNEL_COUNT 长度（None = 内置 XSynth）。
const MIXER_SECTION_VERSION: u32 = 5;

/// v1 混音段结构（无 instruments 字段），供旧版工程迁移解码。
/// 字段必须与编码顺序完整对齐（postcard 非自描述），未用字段允许 dead_code。
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MixerParamsV1 {
    pub channels: Vec<yinhe_mixer::StripParams>,
    pub master: yinhe_mixer::MasterParams,
    pub channel_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub master_inserts: Vec<yinhe_mixer::InsertRef>,
}

/// v2 混音段结构（无 buses/bus_inserts/sends），供旧版工程迁移解码。
/// `instruments` 是旧乐器通道语义，迁移时丢弃。
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MixerParamsV2 {
    pub channels: Vec<yinhe_mixer::StripParams>,
    pub master: yinhe_mixer::MasterParams,
    pub channel_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub master_inserts: Vec<yinhe_mixer::InsertRef>,
    pub instruments: Vec<Option<yinhe_mixer::InsertRef>>,
}

/// v3 混音段结构（无乐器/音频通道 strip 表），供旧版工程迁移解码。
/// `instruments` 是旧乐器通道语义，迁移时丢弃。
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MixerParamsV3 {
    pub channels: Vec<yinhe_mixer::StripParams>,
    pub master: yinhe_mixer::MasterParams,
    pub channel_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub master_inserts: Vec<yinhe_mixer::InsertRef>,
    pub instruments: Vec<Option<yinhe_mixer::InsertRef>>,
    pub buses: Vec<yinhe_mixer::StripParams>,
    pub bus_inserts: Vec<Vec<yinhe_mixer::InsertRef>>,
    pub sends: Vec<Vec<yinhe_mixer::SendParams>>,
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
    let mut params: MixerParams = match version {
        // 当前版本：完整结构（含乐器/音频通道 strip 表）。
        MIXER_SECTION_VERSION => deserialize_postcard(&payload)
            .map_err(|e| tracing::warn!("混音段解析失败，忽略混音设置: {e}"))
            .ok()?,
        // 旧版 v3：按旧结构解码；乐器表是旧通道语义、随命名空间废弃清空
        //（其它设置保留）。postcard 必须整段对齐，不能跳过未知字段。
        3 => {
            let v3: MixerParamsV3 = deserialize_postcard(&payload)
                .map_err(|e| tracing::warn!("旧版混音段解析失败，忽略混音设置: {e}"))
                .ok()?;
            MixerParams {
                channels: v3.channels,
                master: v3.master,
                channel_inserts: v3.channel_inserts,
                master_inserts: v3.master_inserts,
                buses: v3.buses,
                bus_inserts: v3.bus_inserts,
                sends: v3.sends,
                ..Default::default()
            }
        }
        // 旧版 v2：总线/发送留空（不丢其它混音设置）。
        2 => {
            let v2: MixerParamsV2 = deserialize_postcard(&payload)
                .map_err(|e| tracing::warn!("旧版混音段解析失败，忽略混音设置: {e}"))
                .ok()?;
            MixerParams {
                channels: v2.channels,
                master: v2.master,
                channel_inserts: v2.channel_inserts,
                master_inserts: v2.master_inserts,
                ..Default::default()
            }
        }
        // 旧版 v1：乐器表与总线留空（不丢其它混音设置）。
        1 => {
            let v1: MixerParamsV1 = deserialize_postcard(&payload)
                .map_err(|e| tracing::warn!("旧版混音段解析失败，忽略混音设置: {e}"))
                .ok()?;
            MixerParams {
                channels: v1.channels,
                master: v1.master,
                channel_inserts: v1.channel_inserts,
                master_inserts: v1.master_inserts,
                ..Default::default()
            }
        }
        other => {
            tracing::warn!("混音段版本 {other} 不受支持，忽略混音设置");
            return None;
        }
    };
    params.ensure_len();
    Some(params)
}
