//! `mapping.json` — track tree, soundfont config, view state.
//!
//! Carries the per-track metadata that is needed to display a track
//! list before paying the cost of decoding the full event stream.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use yinhe_core::TrackData;

/// 一个全局通道的内置音源参数（仅非默认值落盘）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSynthParams {
    pub channel: u8,
    pub params: yinhe_types::SynthParams,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MappingFile {
    pub version: u16,
    /// Tracks grouped by port → channel. Order within a channel is the
    /// track-creation order; the same order is used in `data.bin` so the
    /// two stay aligned by index.
    pub ports: Vec<PortMap>,
    /// 通道级默认乐器（按源 MIDI 通道 A01..P16 索引，None = 不设置）。
    /// 旧存档是 16 项数组，serde 仍能读入；加载后统一补齐到 256。
    /// 无 MIDI PC 时生效；任一 PC 事件覆盖它。
    #[serde(default)]
    pub channel_instruments: Vec<Option<yinhe_core::ChannelInstrument>>,
    /// 内置音源参数覆盖（只存非默认项；旧存档缺省为空）。
    #[serde(default)]
    pub synth_params: Vec<ChannelSynthParams>,
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
    /// RGBA（0..1）。
    #[serde(default = "default_track_color")]
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
    /// 文件夹嵌套深度（0 = 根层）。
    #[serde(default)]
    pub folder_depth: u8,
    /// 文件夹是否折叠（仅 Folder 轨有意义）。
    #[serde(default)]
    pub collapsed: bool,
    /// 音频通道号（仅音频轨有意义）。
    #[serde(default)]
    pub audio_channel: Option<u16>,
    /// 每轨独立 LC 的稳定 ID（仅 per-track / per-output 输出模式）。
    #[serde(default)]
    pub lc: Option<yinhe_types::LogicalChannelId>,
    /// 方案 C 的音频目标（旧工程缺省，加载时由 lc/audio_channel 推导）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_target: Option<yinhe_core::TrackAudioTarget>,
    /// 多输出插件：该轨映射到 Rack 的第几个输出。
    #[serde(default)]
    pub rack_output: Option<u16>,
    /// 音频片段（仅音频轨有意义）。放在 mapping.json（自描述 JSON）而非
    /// data 段的 postcard 里：postcard 非自描述，追字段会让旧文件解析失败。
    #[serde(default)]
    pub audio_clips: Vec<yinhe_core::AudioClip>,
    /// MIDI Clip v1（仅 MIDI 轨有意义）。
    #[serde(default)]
    pub clips: Vec<yinhe_core::Clip>,
}

fn default_track_color() -> [f32; 4] {
    yinhe_core::DEFAULT_TRACK_COLOR
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
                folder_depth: t.folder_depth,
                collapsed: t.collapsed,
                audio_channel: t.audio_channel,
                lc: t.lc,
                audio_target: t.audio_target,
                rack_output: t.rack_output,
                audio_clips: t.audio_clips.clone(),
                clips: t.clips.clone(),
            });
        }

        Self {
            version: 3,
            ports,
            ..Default::default()
        }
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

    /// 返回按源通道补齐到 256 的默认乐器表，并迁移旧工程的鼓表示。
    ///
    /// 旧 `mapping.json` 只有 16 项（甚至缺字段）；这里统一补齐，
    /// 调用方不需要再关心数组长度。
    pub fn normalized_channel_instruments(&self) -> Vec<Option<yinhe_core::ChannelInstrument>> {
        let mut out = self.channel_instruments.clone();
        out.resize(yinhe_types::MIDI_CHANNEL_COUNT, None);
        for inst in out.iter_mut().flatten() {
            inst.normalize_legacy_bank();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_16_channel_instruments_deserialize_and_normalize() {
        let json = r#"{"version":3,"ports":[],"channel_instruments":[null,null,null,null,null,null,null,null,null,{"bank_msb":128,"bank_lsb":0,"program":0,"is_drum":false},null,null,null,null,null,null]}"#;
        let mapping: MappingFile = serde_json::from_str(json).expect("legacy mapping");
        assert_eq!(mapping.channel_instruments.len(), 16);

        let instruments = mapping.normalized_channel_instruments();
        assert_eq!(instruments.len(), yinhe_types::MIDI_CHANNEL_COUNT);
        assert_eq!(
            instruments[9],
            Some(yinhe_core::ChannelInstrument {
                bank_msb: 0,
                bank_lsb: 0,
                program: 0,
                is_drum: true,
            })
        );
        assert_eq!(instruments[200], None);
    }
}
