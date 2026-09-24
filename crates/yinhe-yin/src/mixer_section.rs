//! 混音段（.yin 可选第 4 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;

use yinhe_mixer::MixerParams;

use crate::codec::{deserialize_postcard, encode_versioned_section, serialize_postcard};
use crate::error::YinError;

/// 混音段格式版本（段内前 4 字节）。MixerParams 字段演进时递增，
/// 加载侧版本不符则忽略混音段（工程本体照常打开）。
const MIXER_SECTION_VERSION: u32 = 5;

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
    let mut params: MixerParams = deserialize_postcard(&payload)
        .map_err(|e| tracing::warn!("混音段解析失败，忽略混音设置: {e}"))
        .ok()?;
    params.ensure_len();
    Some(params)
}
