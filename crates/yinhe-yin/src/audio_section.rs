//! 音频素材段（.yin 可选第 5 段）编解码：version u32 LE + zstd(postcard)。

use std::io::Cursor;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use yinhe_core::YinModel;

use crate::codec::{deserialize_postcard, encode_versioned_section, serialize_postcard};
use crate::error::YinError;

/// 音频段格式版本（段内前 4 字节）。素材结构演进时递增。
const AUDIO_SECTION_VERSION: u32 = 1;

/// 单个内嵌音频素材的持久化形态。
#[derive(Serialize, Deserialize)]
struct AudioSourcePayload {
    uuid: String,
    name: String,
    /// 原始文件字节（wav/mp3/flac/ogg/m4a）。
    data: Vec<u8>,
    duration_seconds: f64,
}

#[derive(Serialize, Deserialize)]
struct AudioPayload {
    sources: Vec<AudioSourcePayload>,
}

/// 编码音频段：version u32 LE + zstd(postcard 素材表)。
/// 无音频素材时返回 None（不写段，保持旧文件布局）。
pub(crate) fn encode_audio_section(
    model: &YinModel,
    level: i32,
) -> Result<Option<Vec<u8>>, YinError> {
    if model.audio_sources.is_empty() {
        return Ok(None);
    }
    let payload = AudioPayload {
        sources: model
            .audio_sources
            .iter()
            .map(|s| AudioSourcePayload {
                uuid: s.uuid.clone(),
                name: s.name.clone(),
                data: (*s.data).clone(),
                duration_seconds: s.duration_seconds,
            })
            .collect(),
    };
    let raw = serialize_postcard(&payload)?;
    Ok(Some(encode_versioned_section(
        AUDIO_SECTION_VERSION,
        &raw,
        level,
    )?))
}

/// 解码音频段。版本不符或损坏时记日志并返回空表（不阻断工程加载，
/// 但片段引用会找不到素材 —— 与文件损坏同级的降级）。
pub(crate) fn decode_audio_section(section: &[u8]) -> Vec<Arc<yinhe_core::AudioSource>> {
    if section.len() < 4 {
        return Vec::new();
    }
    let version = u32::from_le_bytes(match section[..4].try_into() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    });
    if version != AUDIO_SECTION_VERSION {
        tracing::warn!("音频段版本 {version} 不受支持，忽略内嵌音频");
        return Vec::new();
    }
    let payload = match zstd::decode_all(Cursor::new(&section[4..])) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("音频段解压失败，忽略内嵌音频: {e}");
            return Vec::new();
        }
    };
    let payload: AudioPayload = match deserialize_postcard(&payload) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("音频段解析失败，忽略内嵌音频: {e}");
            return Vec::new();
        }
    };
    payload
        .sources
        .into_iter()
        .map(|s| {
            Arc::new(yinhe_core::AudioSource {
                uuid: s.uuid,
                name: s.name,
                data: Arc::new(s.data),
                duration_seconds: s.duration_seconds,
            })
        })
        .collect()
}
