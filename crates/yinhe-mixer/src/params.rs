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

const fn default_instrument_uid() -> u64 {
    1
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

/// 插件格式（持久化进工程；旧工程缺省按 CLAP 处理）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    #[default]
    Clap,
    Vst3,
    /// 内置效果器（yinhe-dsp）。`InsertRef.plugin_id` 为内置效果器标识
    /// （见 `yinhe_dsp::BuiltinEffectKind::id`），`plugin_path` 为空。
    Builtin,
}

/// insert 槽位的插件引用（持久化进工程文件）。
///
/// 插件本体（实例/处理器）由上层（yinhe-egui）管理，这里只存
/// 「哪个插件 + 是否旁通 + 状态字节」。加载时按 id 为主、路径为辅找回插件。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsertRef {
    /// 工程内稳定唯一 id（仅乐器用：自动化按此寻址到具体实例，增删/排序不受影响）。
    /// 0 = 未分配（旧工程），加载时由 `ensure_instrument_uids` 补号。
    #[serde(default)]
    pub uid: u64,
    /// 插件包路径（如 .clap 文件 / .vst3 bundle 目录）。
    pub plugin_path: PathBuf,
    /// 包内插件 id（CLAP id / VST3 32 位十六进制 class id）。
    pub plugin_id: String,
    /// 显示名（持久化：恢复时扫描结果可能不含该插件，仍能显示原名）。
    #[serde(default)]
    pub name: String,
    /// 插件格式（旧工程缺省 CLAP）。
    #[serde(default)]
    pub format: PluginFormat,
    /// 旁通：链上保留槽位但不参与处理。
    #[serde(default)]
    pub bypassed: bool,
    /// 插件状态字节（CLAP state 扩展 / VST3 component+controller 两段）；None = 未保存过。
    #[serde(default)]
    pub state: Option<Vec<u8>>,
}

/// 发送参数：源通道 → 总线（bus / return）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendParams {
    /// 目标总线索引（`MixerParams::buses` 下标）。
    pub bus: u8,
    /// 发送量（线性增益；0.0 = 关闭）。
    #[serde(default)]
    pub amount: f32,
    /// 发送点：true = 推子前（insert 后、fader 前，不受增益/声像影响），
    /// false = 推子后。
    #[serde(default)]
    pub pre_fader: bool,
}

impl Default for SendParams {
    fn default() -> Self {
        Self {
            bus: 0,
            amount: 0.0,
            pre_fader: false,
        }
    }
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
    /// 乐器插件链（索引 = 源 MIDI 通道，固定 CHANNEL_COUNT 长度）：
    /// 空 = 该通道使用默认的内置 XSynth；非空 = 按序**叠加**的插件乐器，
    /// 音符事件广播给链内每个乐器，输出相加。
    /// 混音参数（推子/声像/insert/send）统一走该 MIDI 通道的 strip 表。
    #[serde(default, deserialize_with = "de_instruments")]
    pub instruments: Vec<Vec<InsertRef>>,
    /// 下一个乐器实例 uid（工程内唯一、持久化；加载时对齐到已用最大值 +1）。
    #[serde(default = "default_instrument_uid")]
    pub next_instrument_uid: u64,
    /// 音频通道 strip（索引 = 音频通道号，与 `TrackData::audio_channel` 对齐）。
    #[serde(default)]
    pub audio_channels: Vec<StripParams>,
    /// 每音频通道的 insert 链。
    #[serde(default)]
    pub audio_inserts: Vec<Vec<InsertRef>>,
    /// 每音频通道的 send 列表。
    #[serde(default)]
    pub audio_sends: Vec<Vec<SendParams>>,
    /// 总线（bus / return）通道参数：每元素一条总线（与源通道同语义）。
    #[serde(default)]
    pub buses: Vec<StripParams>,
    /// 每条总线的 insert 链（与 `buses` 等长）。
    #[serde(default)]
    pub bus_inserts: Vec<Vec<InsertRef>>,
    /// 每个源通道的发送列表（按源通道索引；一个通道可发往多条总线）。
    #[serde(default)]
    pub sends: Vec<Vec<SendParams>>,
}

impl Default for MixerParams {
    fn default() -> Self {
        Self {
            channels: vec![StripParams::default(); CHANNEL_COUNT],
            master: MasterParams::default(),
            channel_inserts: vec![Vec::new(); CHANNEL_COUNT],
            master_inserts: Vec::new(),
            instruments: vec![Vec::new(); CHANNEL_COUNT],
            next_instrument_uid: 1,
            audio_channels: Vec::new(),
            audio_inserts: Vec::new(),
            audio_sends: Vec::new(),
            buses: Vec::new(),
            bus_inserts: Vec::new(),
            sends: Vec::new(),
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
        self.sends.resize(CHANNEL_COUNT, Vec::new());
        self.sends.truncate(CHANNEL_COUNT);
        self.instruments.resize(CHANNEL_COUNT, Vec::new());
        self.instruments.truncate(CHANNEL_COUNT);
        self.ensure_instrument_uids();
        self.bus_inserts.resize(self.buses.len(), Vec::new());
        self.bus_inserts.truncate(self.buses.len());
        // 防御：清理指向不存在总线的 send（工程被手工编辑/版本迁移残留）。
        let bus_count = self.buses.len();
        for list in &mut self.sends {
            list.retain(|s| (s.bus as usize) < bus_count);
        }
    }

    /// 总线数量。
    pub fn bus_count(&self) -> usize {
        self.buses.len()
    }

    /// 分配一个新的乐器实例 uid（恒 ≥1）。
    pub fn alloc_instrument_uid(&mut self) -> u64 {
        let uid = self.next_instrument_uid.max(1);
        self.next_instrument_uid = uid + 1;
        uid
    }

    /// 加载后调用：给未分配 uid 的乐器补号，并把 `next_instrument_uid`
    /// 对齐到已用最大值 +1。幂等。
    pub fn ensure_instrument_uids(&mut self) {
        for refs in &mut self.instruments {
            for r in refs.iter_mut() {
                if r.uid == 0 {
                    let uid = self.next_instrument_uid.max(1);
                    self.next_instrument_uid = uid + 1;
                    r.uid = uid;
                } else if r.uid >= self.next_instrument_uid {
                    self.next_instrument_uid = r.uid + 1;
                }
            }
        }
    }

    /// 新增一条总线，返回其索引。
    pub fn add_bus(&mut self) -> u8 {
        self.buses.push(StripParams::default());
        self.bus_inserts.push(Vec::new());
        (self.buses.len() - 1) as u8
    }

    /// 删除总线 `bus`：移除其参数与 insert 链，清理指向它的 send、
    /// 并把更高索引的 send 目标前移一位（保持路由语义）。
    pub fn remove_bus(&mut self, bus: u8) {
        let idx = bus as usize;
        if idx >= self.buses.len() {
            return;
        }
        self.buses.remove(idx);
        if idx < self.bus_inserts.len() {
            self.bus_inserts.remove(idx);
        }
        for list in &mut self.sends {
            list.retain(|s| s.bus != bus);
            for s in list.iter_mut() {
                if s.bus > bus {
                    s.bus -= 1;
                }
            }
        }
    }

    /// 某源通道的 strip 参数（越界给默认值，防御性）。
    pub fn strip(&self, channel: u8) -> StripParams {
        self.channels
            .get(channel as usize)
            .copied()
            .unwrap_or_default()
    }

    /// 音频通道 `ch` 的 strip 参数（越界给默认值，防御性）。
    pub fn audio_strip(&self, ch: u16) -> StripParams {
        self.audio_channels
            .get(ch as usize)
            .copied()
            .unwrap_or_default()
    }

    /// 把音频通道相关表补齐到 `count` 条（通道数变化时调用，只增不减：
    /// 通道号是稳定索引，删轨道后重新加同号通道要恢复原设置）。
    /// 参数表长度与通道号索引严格对应，缺位补默认值。
    pub fn ensure_channel_tables(&mut self, audio_count: usize) {
        if self.audio_channels.len() < audio_count {
            self.audio_channels
                .resize(audio_count, StripParams::default());
        }
        if self.audio_inserts.len() < audio_count {
            self.audio_inserts.resize(audio_count, Vec::new());
        }
        if self.audio_sends.len() < audio_count {
            self.audio_sends.resize(audio_count, Vec::new());
        }
    }
}

/// 反序列化 `instruments`：兼容旧格式。
/// 旧：每通道 `Option<InsertRef>`（`null` 或对象）；新：每通道 `Vec<InsertRef>`。
fn de_instruments<'de, D>(de: D) -> Result<Vec<Vec<InsertRef>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        One(Option<InsertRef>),
        Many(Vec<InsertRef>),
    }
    let raw: Vec<Entry> = Vec::deserialize(de)?;
    Ok(raw
        .into_iter()
        .map(|e| match e {
            Entry::One(Some(r)) => vec![r],
            Entry::One(None) => Vec::new(),
            Entry::Many(v) => v,
        })
        .collect())
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
    fn remove_bus_remaps_send_targets() {
        let mut p = MixerParams::default();
        p.ensure_len();
        p.add_bus();
        p.add_bus();
        p.add_bus();
        p.sends[0] = vec![
            SendParams {
                bus: 1,
                amount: 0.5,
                pre_fader: false,
            },
            SendParams {
                bus: 2,
                amount: 0.5,
                pre_fader: true,
            },
        ];
        p.remove_bus(0);
        assert_eq!(p.buses.len(), 2);
        let s = &p.sends[0];
        assert_eq!(s.len(), 2, "指向其他 bus 的 send 保留");
        assert_eq!(s[0].bus, 0, "bus 1 → 0 前移");
        assert_eq!(s[1].bus, 1, "bus 2 → 1 前移");
    }

    #[test]
    fn remove_bus_drops_sends_to_it() {
        let mut p = MixerParams::default();
        p.ensure_len();
        p.add_bus();
        p.add_bus();
        p.sends[3] = vec![SendParams {
            bus: 0,
            amount: 1.0,
            pre_fader: false,
        }];
        p.remove_bus(0);
        assert!(p.sends[3].is_empty(), "指向被删 bus 的 send 应清理");
    }

    #[test]
    fn ensure_len_prunes_dangling_sends() {
        let mut p = MixerParams::default();
        p.ensure_len();
        p.sends[0] = vec![SendParams {
            bus: 5,
            amount: 1.0,
            pre_fader: false,
        }];
        p.ensure_len();
        assert!(p.sends[0].is_empty(), "越界 bus 的 send 应被清理");
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

    /// 旧格式（每通道 Option<InsertRef>）与新格式（每通道 Vec）都能反序列化。
    #[test]
    fn de_instruments_accepts_old_and_new() {
        #[derive(Deserialize)]
        struct W {
            #[serde(deserialize_with = "super::de_instruments")]
            instruments: Vec<Vec<InsertRef>>,
        }
        let one = r#"{"plugin_path":"/x","plugin_id":"a","name":"A","format":"Clap","bypassed":false,"state":null}"#;
        let old = format!(r#"{{"instruments":[null,{one}]}}"#);
        let w: W = serde_json::from_str(&old).unwrap();
        assert!(w.instruments[0].is_empty(), "旧 null → 空链");
        assert_eq!(w.instruments[1].len(), 1);
        assert_eq!(w.instruments[1][0].plugin_id, "a");

        let new = format!(r#"{{"instruments":[[{one}],[]]}}"#);
        let w2: W = serde_json::from_str(&new).unwrap();
        assert_eq!(w2.instruments[0].len(), 1);
        assert!(w2.instruments[1].is_empty());
    }
}
