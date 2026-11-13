//! 合成器事件类型（sample 域）：CPU 后端与预览引擎共用。
//!
//! 原先定义在 `gpu_synth` 里（GPU 后端已删除），现在独立成模块，
//! 语义不变：`NoteOn` 自带结束时间，事件按 sample 有序。

/// 合成器事件（sample 域，按 sample 排序后由 `load_events` 加载）。
#[derive(Clone, Copy, Debug)]
pub enum SynthEvent {
    /// NoteOn 携带音符结束时间 `end_sample`：voice 到期自行 release。
    /// 引擎的事件表**不再生成 NoteOff**（分页装载时 NoteOff 的时间戳跨窗口会
    /// 破坏事件顺序；自带 end 后事件量也减半）。
    NoteOn {
        sample: u64,
        channel: u8,
        key: u8,
        velocity: u8,
        end_sample: u64,
        /// 来源音轨（AR 每轨电平表用；预览/测试可填 0）。
        track: u16,
    },
    /// 立即释放该 (channel, key) 最老的未释放 voice。
    /// 引擎事件表不使用（NoteOn 自带 end 取代）；保留给实时 MIDI 输入/提前释放。
    NoteOff { sample: u64, channel: u8, key: u8 },
    Control {
        sample: u64,
        channel: u8,
        event: ControlEvent,
    },
}

impl SynthEvent {
    pub fn sample(&self) -> u64 {
        match self {
            SynthEvent::NoteOn { sample, .. }
            | SynthEvent::NoteOff { sample, .. }
            | SynthEvent::Control { sample, .. } => *sample,
        }
    }
}

/// 通道控制事件（语义与 xsynth `ControlEvent` 对齐）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlEvent {
    /// 原始 MIDI CC 事件 (controller, value)。`value` 为 f32（0..127 域）：
    /// 自动化插值不再量化到 7-bit，音源侧按需取整（如 RPN data / bank）。
    Raw(u8, f32),
    /// 弯音值 -1..1。
    PitchBend(f32),
    /// 弯音灵敏度（半音）。
    PitchBendSensitivity(f32),
    /// 微调（音分）。
    FineTune(f32),
    /// 粗调（半音）。
    CoarseTune(f32),
    /// 原生 RPN：`parameter` u16 参数号，`value` 归一化 f32（0..=1）。
    /// 由 yinhe-audio 的原生 RPN 事件流直接投递（不再拆 CC 序列）；
    /// 0/1/2 解析为弯音灵敏度/微调/粗调，其余号保留给扩展参数。
    Rpn { parameter: u16, value: f32 },
    /// 原生 NRPN：`parameter` u16 参数号，`value` 归一化 f32（0..=1）。
    Nrpn { parameter: u16, value: f32 },
    /// 音色更换：选择该通道的音色库条目（bank, preset 语义见 `PercussionMode`）。
    ProgramChange(u8),
    /// 鼓组模式（等价 xsynth `SetPercussionMode` 配置）：true 置 bank=128，false 置 bank=0。
    /// 由 yinhe-audio 在模型加载/seek 时根据通道声明注入，后续 CC0 不能改鼓组 bank。
    PercussionMode(bool),
}
