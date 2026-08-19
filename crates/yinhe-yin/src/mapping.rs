//! `mapping.json` — track tree, soundfont config, view state.
//!
//! Carries the per-track metadata that is needed to display a track
//! list before paying the cost of decoding the full event stream.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use yinhe_core::TrackData;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MappingFile {
    pub version: u16,
    /// Tracks grouped by port → channel. Order within a channel is the
    /// track-creation order; the same order is used in `data.bin` so the
    /// two stay aligned by index.
    pub ports: Vec<PortMap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMap {
    pub port: u8,
    pub channels: Vec<ChannelMap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMap {
    pub channel: u8,
    pub tracks: Vec<TrackMap>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackMap {
    pub uuid: String,
    pub name: String,
    /// RGBA（0..1）。旧存档为 RGB，反序列化时自动补 alpha=1.0。
    #[serde(
        default = "default_track_color",
        deserialize_with = "deserialize_track_color"
    )]
    pub color: [f32; 4],
    #[serde(default)]
    pub channel_prefix: Option<u8>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub soloed: bool,
    /// 音轨种类（旧存档无此字段，默认 Midi）。
    #[serde(default)]
    pub kind: yinhe_core::TrackKind,
    /// 乐器通道号（仅乐器轨有意义）。
    #[serde(default)]
    pub instrument_channel: Option<u16>,
}

fn default_track_color() -> [f32; 4] {
    yinhe_core::DEFAULT_TRACK_COLOR
}

/// 兼容旧存档的 RGB 三元素数组（补 alpha=1.0）与新格式的 RGBA 四元素数组。
fn deserialize_track_color<'de, D>(d: D) -> Result<[f32; 4], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Vec::<f32>::deserialize(d)?;
    match v.len() {
        3 => Ok([v[0], v[1], v[2], 1.0]),
        4 => Ok([v[0], v[1], v[2], v[3]]),
        n => Err(serde::de::Error::invalid_length(n, &"3 or 4 elements")),
    }
}

impl MappingFile {
    /// Build a MappingFile from a YinModel's tracks.
    ///
    /// Tracks are grouped by `(port, channel)`. Group order (ports,
    /// channels) and track order within a group follow the position in
    /// `model.tracks` (first-appearance order), so the file reads like
    /// the model. Note: the nested grouping itself cannot express a
    /// model order where ports interleave (same-port tracks must stay
    /// contiguous); `data.bin` is therefore linked by uuid, never by
    /// flat order.
    pub fn from_tracks(tracks: &[std::sync::Arc<TrackData>]) -> Self {
        let mut ports: Vec<PortMap> = Vec::new();
        // 首次出现顺序保存分组；用 map 记录 (port / (port, channel)) → 下标，O(tracks)。
        let mut port_idx: HashMap<u8, usize> = HashMap::with_capacity(tracks.len());
        let mut ch_idx: HashMap<(u8, u8), usize> = HashMap::with_capacity(tracks.len());
        for t in tracks {
            let p = *port_idx.entry(t.port).or_insert_with(|| {
                ports.push(PortMap {
                    port: t.port,
                    channels: Vec::new(),
                });
                ports.len() - 1
            });
            let pm = &mut ports[p];
            let c = *ch_idx.entry((t.port, t.channel)).or_insert_with(|| {
                pm.channels.push(ChannelMap {
                    channel: t.channel,
                    tracks: Vec::new(),
                });
                pm.channels.len() - 1
            });
            pm.channels[c].tracks.push(TrackMap {
                uuid: t.uuid.clone(),
                name: t.name.clone(),
                color: t.color,
                channel_prefix: t.channel_prefix,
                muted: t.muted,
                soloed: t.soloed,
                kind: t.kind,
                instrument_channel: t.instrument_channel,
            });
        }

        Self { version: 2, ports }
    }

    /// Flat ordered list of (port, channel, TrackMap) — ports and
    /// channels in first-appearance order, tracks in stored order.
    /// Not used to order loaded tracks: `data.bin` payloads carry the
    /// authoritative track order, linked to mapping entries by uuid.
    pub fn flat_tracks(&self) -> impl Iterator<Item = (u8, u8, &TrackMap)> {
        self.ports.iter().flat_map(|p| {
            let port = p.port;
            p.channels.iter().flat_map(move |c| {
                let channel = c.channel;
                c.tracks.iter().map(move |t| (port, channel, t))
            })
        })
    }
}
