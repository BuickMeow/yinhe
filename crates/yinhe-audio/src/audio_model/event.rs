use std::cmp::Ordering;

/// 通道控制事件（原 xsynth `ControlEvent` 的等价词汇；引擎只消费这些）。
///
/// 部分变体生产代码不再构造（已用 `AudioEvent::PitchBend/ProgramChange` 一等事件），
/// 但保留作为 MIDI 导出/聚合路径的兼容词汇，测试仍覆盖。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ControlEvent {
    /// 7-bit CC（号, 值 0..127）。
    Raw(u8, u8),
    /// 14-bit Pitch Bend 归一化到 -1..1（xsynth 语义）。
    PitchBendValue(f32),
    /// 直接给 -1..1 的 Pitch Bend（兼容旧事件）。
    PitchBend(f32),
    /// RPN 0：半音数。
    PitchBendSensitivity(f32),
    /// RPN 1：音分。
    FineTune(f32),
    /// RPN 2：半音。
    CoarseTune(f32),
}

/// 通道音频事件（原 xsynth `ChannelAudioEvent` 的等价词汇）。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ChannelAudioEvent {
    NoteOn { key: u8, vel: u8 },
    NoteOff { key: u8 },
    Control(ControlEvent),
    ProgramChange(u8),
}

/// 播放事件流里的一条通道控制事件。
///
/// RPN/NRPN 不再在生成端特化成高层事件或拆 CC 序列：以原生 u16 参数号
/// 为一等事件，由 dispatch 按后端能力适配（详见各适配函数）：
/// - yinhe-synth：原生 `ControlEvent::Rpn/Nrpn`（f32 全链路，无量化）；
/// - XSynth：0/1/2 → 高层 f32 事件，其余 → CC 序列（出口量化）；
/// - VST/CLAP 插件：标准 CC 序列（101/100/6/38 或 99/98/6/38，出口量化）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AudioEvent {
    /// xsynth 回退路径的**音符**事件（NoteOn/NoteOff）直通；控制事件不再走这里。
    Channel(ChannelAudioEvent),
    /// 原始 MIDI CC：`controller` u8 号，`value` f32（0..127 域，**不量化**）。
    /// 引擎内部统一浮点；出口按消费者量化——yinhe-synth 用 f32，其余
    /// （xsynth / VST3 / CLAP / yinhe-dsp）round 成 u8。
    Cc { controller: u8, value: f32 },
    /// Pitch Bend：`value` f32（-1..1）。出口转 14-bit（xsynth/插件）或 f32（yinhe-synth）。
    PitchBend(f32),
    /// Program Change：`value` u8 音色号。
    ProgramChange(u8),
    /// 原生 RPN：`parameter` u16 参数号，`value` 归一化 f32（0..=1，
    /// 与自动化 lane 同值域——全链路浮点，出口才按参数值域量化）。
    Rpn { parameter: u16, value: f32 },
    /// 原生 NRPN：`parameter` u16 参数号，`value` 归一化 f32。
    Nrpn { parameter: u16, value: f32 },
}

pub(crate) struct SortedCC {
    /// 事件时刻（tick 域，u32——模型 NoteEvent/AutomationEvent 的 tick 上限）。
    /// 音频内部统一 tick 域：dispatch/chase 比较不再需要 sample 转换。
    pub(crate) tick: u32,
    pub(crate) channel: u32,
    /// 源音轨索引，用于 mute 时过滤自动化事件。
    pub(crate) track: u16,
    /// 源自动化 lane 索引（轨道内，0..lanes.len()-1）。
    /// `u16::MAX` = 非 lane 事件（ProgramChange 展开），只受 skip_track 过滤。
    /// dispatch 时用它查 AM M/S 动态掩码（与 skip_track 并列），
    /// 使旁通切换不再需要重建事件流。
    pub(crate) lane: u16,
    pub(crate) event: AudioEvent,
    /// 插件参数事件：`Some` 时 `event` 为占位（不影响 xsynth 路径），
    /// dispatch 走乐器通道的 `PluginEvent::ParamValue`。
    pub(crate) plugin_param: Option<PluginParamEvent>,
}

/// 插件参数自动化事件（值域归一化 0..1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PluginParamEvent {
    /// MIDI 全局通道（0..256，与 `TrackData::global_channel()` 对齐）。
    pub(crate) channel: u8,
    /// 目标乐器实例 uid（同一通道可叠加多个乐器，按 uid 精确定位）。
    pub(crate) uid: u64,
    pub(crate) param_id: u32,
    /// 归一化值 0..1。
    pub(crate) value: f32,
}

/// `SortedCC.lane` 哨兵：ProgramChange 事件（不受 AM lane 掩码过滤）。
pub(crate) const PC_LANE: u16 = u16::MAX;

/// 活跃音符（已 NoteOn 待 NoteOff）。
///
/// `Ord` 按 `end_tick` 升序，相同 end_tick 再按 (key, channel) 区分。
/// 配合 `BinaryHeap<Reverse<ActiveNote>>` 用作 min-heap，让最早结束的音符在堆顶，
/// NoteOff 检测从 O(V) retain 全扫降到 O(ended × log V) 逐个 pop。
#[derive(Clone, Copy)]
pub(crate) struct ActiveNote {
    pub(crate) key: u8,
    /// 目标 dense 通道：MIDI 音符 = xsynth dense；乐器音符 = 乐器 dense。
    pub(crate) dense: u32,
    /// 乐器音符的 CLAP 内部 MIDI 通道（= 音轨 global_channel 低 4 位）。MIDI 音符忽略。
    pub(crate) clap_channel: u8,
    /// 是否为乐器音符（true → NoteOff 喂乐器实例）。
    pub(crate) is_instrument: bool,
    pub(crate) end_tick: u32,
    /// 源音轨索引：即时 mute 时据此精确 kill 该轨在响音符（不误伤共享通道）。
    pub(crate) track: u16,
}

impl PartialEq for ActiveNote {
    fn eq(&self, other: &Self) -> bool {
        self.end_tick == other.end_tick
            && self.key == other.key
            && self.dense == other.dense
            && self.is_instrument == other.is_instrument
            && self.clap_channel == other.clap_channel
            && self.track == other.track
    }
}
impl Eq for ActiveNote {}
impl PartialOrd for ActiveNote {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ActiveNote {
    fn cmp(&self, other: &Self) -> Ordering {
        self.end_tick
            .cmp(&other.end_tick)
            .then(self.key.cmp(&other.key))
            .then(self.dense.cmp(&other.dense))
            .then(self.is_instrument.cmp(&other.is_instrument))
            .then(self.clap_channel.cmp(&other.clap_channel))
            .then(self.track.cmp(&other.track))
    }
}
