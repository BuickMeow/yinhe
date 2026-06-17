//! `mapping.json` — track tree, soundfont config, view state.
//!
//! Carries the per-track metadata that is needed to display a track
//! list before paying the cost of decoding the full event stream.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use yinhe_core::TrackData;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MappingFile {
    pub version: u16,
    /// Tracks grouped by port → channel. Order within a channel is the
    /// track-creation order; the same order is used in `data.bin` so the
    /// two stay aligned by index.
    pub ports: Vec<PortMap>,
    /// Soundfont paths per port (0..15).
    #[serde(default)]
    pub soundfonts: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub view: ViewState,
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
    pub color: [f32; 3],
    #[serde(default)]
    pub channel_prefix: Option<u8>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub soloed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewState {
    #[serde(default = "default_zoom")]
    pub zoom_x: f32,
    #[serde(default = "default_zoom")]
    pub zoom_y: f32,
    #[serde(default)]
    pub scroll_tick: u32,
    #[serde(default = "default_scroll_key")]
    pub scroll_key: u8,
    #[serde(default)]
    pub active_track_uuid: Option<String>,
}

fn default_zoom() -> f32 {
    1.0
}
fn default_scroll_key() -> u8 {
    60
}

impl MappingFile {
    /// Build a MappingFile from a YinModel's tracks.
    ///
    /// Tracks are grouped by `(port, channel)`. The order of tracks
    /// within a channel preserves their position in `model.tracks`.
    pub fn from_tracks(tracks: &[std::sync::Arc<TrackData>]) -> Self {
        // Group preserving original order: walk tracks, push into a
        // (port, channel) bucket map.
        let mut bucket: BTreeMap<(u8, u8), Vec<TrackMap>> = BTreeMap::new();
        for t in tracks {
            bucket.entry((t.port, t.channel)).or_default().push(TrackMap {
                uuid: t.uuid.clone(),
                name: t.name.clone(),
                color: t.color,
                channel_prefix: t.channel_prefix,
                muted: t.muted,
                soloed: t.soloed,
            });
        }

        // Re-assemble into ports[].channels[] structure.
        let mut ports_map: BTreeMap<u8, BTreeMap<u8, Vec<TrackMap>>> = BTreeMap::new();
        for ((port, channel), tms) in bucket {
            ports_map.entry(port).or_default().insert(channel, tms);
        }

        let ports: Vec<PortMap> = ports_map
            .into_iter()
            .map(|(port, channels)| PortMap {
                port,
                channels: channels
                    .into_iter()
                    .map(|(channel, tracks)| ChannelMap { channel, tracks })
                    .collect(),
            })
            .collect();

        Self {
            version: 2,
            ports,
            soundfonts: BTreeMap::new(),
            view: ViewState::default(),
        }
    }

    /// Flat ordered list of (port, channel, TrackMap), iterating ports
    /// in ascending order, channels in ascending order, tracks in stored
    /// order. Used to align `mapping.json` track entries with `data.bin`
    /// track payloads.
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
