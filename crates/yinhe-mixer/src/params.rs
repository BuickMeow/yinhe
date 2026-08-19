//! 混音台持久化参数（serde，进工程文件）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 源 MIDI 通道总数：A01..P16（16 port × 16 通道）。
pub const CHANNEL_COUNT: usize = 256;

/// 单个通道条的持久化参数。
///
/// 注意与 MIDI CC7/11（音量/表情）区分：那是乐曲内容、作用于合成器内部；
/// 这里的 gain/pan 是工程混音设置、作用于音频域，两层串联互不相干。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StripParams {
    /// 线性增益（1.0 = 0 dB）。
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// 声像，-1.0（左）~ 1.0（右），0.0 居中。
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
}

const fn default_gain() -> f32 {
    1.0
}

impl Default for StripParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }
}

/// 主输出的持久化参数。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MasterParams {
    /// 线性增益（1.0 = 0 dB）。
    #[serde(default = "default_gain")]
    pub gain: f32,
}

impl Default for MasterParams {
    fn default() -> Self {
        Self { gain: 1.0 }
    }
}

/// insert 槽位的插件引用（持久化进工程文件）。
///
/// 插件本体（实例/处理器）由上层（yinhe-egui）管理，这里只存
/// 「哪个插件 + 是否旁通 + 状态字节」。加载时按 id 为主、路径为辅找回插件。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsertRef {
    /// 插件包路径（如 .clap 文件 / bundle 目录）。
    pub plugin_path: PathBuf,
    /// 包内插件 id（如 "com.u-he.diva"）。
    pub plugin_id: String,
    /// 显示名（持久化：恢复时扫描结果可能不含该插件，仍能显示原名）。
    #[serde(default)]
    pub name: String,
    /// 旁通：链上保留槽位但不参与处理。
    #[serde(default)]
    pub bypassed: bool,
    /// 插件状态字节（CLAP state 扩展产出）；None = 插件无状态扩展或未保存过。
    #[serde(default)]
    pub state: Option<Vec<u8>>,
}

/// 整个混音台的持久化参数。
///
/// 索引语义：`channels[i]` / `channel_inserts[i]` 对应**源 MIDI 通道 i**
/// （A01 = 0，P16 = 255），不是工程轨道、也不是压缩后的 dense 索引——
/// 源通道号在音轨增删后保持稳定，dense 索引随布局重建变化。
/// 未被工程使用的通道的条目闲置无害。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerParams {
    /// 按源 MIDI 通道索引的 strip 参数，固定 CHANNEL_COUNT 长度。
    #[serde(default)]
    pub channels: Vec<StripParams>,
    #[serde(default)]
    pub master: MasterParams,
    /// 每源通道的 insert 链（固定 CHANNEL_COUNT 长度，元素为有序槽位列表）。
    #[serde(default)]
    pub channel_inserts: Vec<Vec<InsertRef>>,
    /// 主输出 insert 链。
    #[serde(default)]
    pub master_inserts: Vec<InsertRef>,
    /// 乐器通道（0 起，与 `TrackData::instrument_channel` 对齐）→ 插件引用。
    /// 索引 = 乐器通道号；未用到的通道为 `None`。仅工程用到乐器轨时非空。
    /// 与 MIDI 源通道命名空间独立（乐器通道是另一套）。
    #[serde(default)]
    pub instruments: Vec<Option<InsertRef>>,
}

impl Default for MixerParams {
    fn default() -> Self {
        Self {
            channels: vec![StripParams::default(); CHANNEL_COUNT],
            master: MasterParams::default(),
            channel_inserts: vec![Vec::new(); CHANNEL_COUNT],
            master_inserts: Vec::new(),
            instruments: Vec::new(),
        }
    }
}

impl MixerParams {
    /// 反序列化后调用：老版本工程可能缺字段/长度不足，补齐到固定通道数。
    pub fn ensure_len(&mut self) {
        self.channels.resize(CHANNEL_COUNT, StripParams::default());
        self.channel_inserts.resize(CHANNEL_COUNT, Vec::new());
        self.channels.truncate(CHANNEL_COUNT);
        self.channel_inserts.truncate(CHANNEL_COUNT);
    }

    /// 某源通道的 strip 参数（越界给默认值，防御性）。
    pub fn strip(&self, channel: u8) -> StripParams {
        self.channels
            .get(channel as usize)
            .copied()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_len_pads_and_truncates() {
        let mut p = MixerParams {
            channels: vec![StripParams {
                gain: 0.5,
                ..StripParams::default()
            }],
            ..MixerParams::default()
        };
        p.channel_inserts.clear();
        p.ensure_len();
        assert_eq!(p.channels.len(), CHANNEL_COUNT);
        assert_eq!(p.channel_inserts.len(), CHANNEL_COUNT);
        assert_eq!(p.channels[0].gain, 0.5);
        assert!(p.channels[1..].iter().all(|s| *s == StripParams::default()));
    }

    #[test]
    fn strip_out_of_range_gives_default() {
        let p = MixerParams {
            channels: Vec::new(),
            channel_inserts: Vec::new(),
            ..MixerParams::default()
        };
        assert_eq!(p.strip(200), StripParams::default());
    }
}
