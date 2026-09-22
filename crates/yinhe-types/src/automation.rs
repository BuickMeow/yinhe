use serde::{Deserialize, Serialize};

/// AR 自动化 lane 的 M/S（静音/独奏）试听状态。
///
/// 只影响该 lane 自动化效果是否发送：
/// - mute：该 lane 的效果不发送（试听时旁通）。
/// - solo：该音轨内有任意 lane solo 时，只有被 solo 的 lane 发送，
///   同轨其他 lane 静音；主音轨（音符发声）与其他音轨不受影响。
///
/// 纯试听状态：不进模型、不进 undo（与 track M/S 同语义层级）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AmMsState {
    pub mute: bool,
    pub solo: bool,
}

/// How to interpolate from one automation event to the next.
///
/// Stored per-event on `AutomationEvent::shape`, describing the segment
/// that *starts* at this event. The last event's shape has no effect
/// (no segment after it).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SegmentShape {
    /// 离散：保持当前值，直到下个事件才瞬间跳变。MIDI CC 的原生语义。
    Step,
    /// 三次贝塞尔曲线（CSS handle 风格，偏移量参数化）。
    ///
    /// 归一化空间：起点 P0=(0,0) 对应本事件 (tick, value)，终点 P3=(1,1) 对应下一事件。
    /// 存储值为控制点相对各自锚点的归一化偏移量，内部 `*4` 放大得到实际贝塞尔参数：
    ///
    /// - `(x1, y1)`：P1 相对 P0 的偏移，实际位置 P1 = P0 + (P3-P0)·(x1·4, y1·4)
    /// - `(x2, y2)`：P2 相对 P3 的偏移，实际位置 P2 = P3 + (P3-P0)·(x2·4, y2·4)
    ///
    /// 每个分量 `∈ [-0.5, 0.5]`（DragValue/拖拽范围），内部 `*4` 后实际参数范围 `[-2, 2]`。
    /// 直线（退化）：`(0, 0, 0, 0)` — 0 为中性，偏离 0 即弯曲。
    ///
    /// 两个 handle 从各自锚点指出（P1 从 P0，P2 从 P3），符合 CSS 动画编辑器的视觉直觉。
    Curve { x1: f32, y1: f32, x2: f32, y2: f32 },
}

impl Default for SegmentShape {
    /// MIDI 文件导入与未指定时的默认值。Step 与 MIDI CC 原生语义一致。
    fn default() -> Self {
        SegmentShape::Step
    }
}

impl SegmentShape {
    /// 偏移量参数化的放大系数：存储值 `[-0.5, 0.5]` × 4 = 实际参数 `[-2, 2]`。
    pub const SCALE: f32 = 4.0;

    /// 直线 Curve 的默认偏移量：全部为 0（中性）。
    pub const LINEAR_X1: f32 = 0.0;
    pub const LINEAR_Y1: f32 = 0.0;
    pub const LINEAR_X2: f32 = 0.0;
    pub const LINEAR_Y2: f32 = 0.0;

    /// 直线 Curve 的快捷构造。
    pub const fn linear_curve() -> Self {
        SegmentShape::Curve {
            x1: Self::LINEAR_X1,
            y1: Self::LINEAR_Y1,
            x2: Self::LINEAR_X2,
            y2: Self::LINEAR_Y2,
        }
    }

    /// 在归一化进度 `t ∈ [0, 1]` 上计算插值因子 `f ∈ [0, 1]`。
    /// `value_at = v1 + (v2 - v1) * f`。
    ///
    /// 对于 Curve，t 是 tick 进度。三次贝塞尔的参数 u 不等于 t，
    /// 需要从 x(u)=t 反解 u（数值法），再代入 y(u)。
    #[inline]
    pub fn interpolate(self, t: f32) -> f32 {
        debug_assert!((0.0..=1.0).contains(&t), "interpolate t out of range: {t}");
        let t = t.clamp(0.0, 1.0);
        match self {
            SegmentShape::Step => 0.0, // Step: hold v1 until next event; segment value = v1
            SegmentShape::Curve { x1, y1, x2, y2 } => {
                if Self::is_linear_impl(x1, y1, x2, y2) {
                    return t;
                }
                // 实际控制点（归一化空间，P0=(0,0), P3=(1,1)）：
                // P1 = (x1*4, y1*4), P2 = (1+x2*4, 1+y2*4)
                let u = solve_cubic_bezier_u_for_x(t, x1, x2);
                let u1 = 1.0 - u;
                let p1y = y1 * Self::SCALE;
                let p2y = 1.0 + y2 * Self::SCALE;
                3.0 * u1 * u1 * u * p1y + 3.0 * u1 * u * u * p2y + u * u * u
            }
        }
    }

    /// 是否为直线（Curve 且偏移量全部 ≈ 0）。
    #[inline]
    pub fn is_linear(self) -> bool {
        matches!(self, SegmentShape::Curve { x1, y1, x2, y2 }
            if Self::is_linear_impl(x1, y1, x2, y2))
    }

    #[inline]
    fn is_linear_impl(x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
        x1.abs() < 1e-4 && y1.abs() < 1e-4 && x2.abs() < 1e-4 && y2.abs() < 1e-4
    }
}

/// 解三次贝塞尔方程 B_x(u) = t 求 u（Newton 迭代）。
///
/// 偏移量参数化：P1.x = x1·4，P2.x = 1 + x2·4。
/// `B_x(u) = 3(1-u)²u·(x1·4) + 3(1-u)u²·(1+x2·4) + u³`
/// `B_x'(u) = 3(1-u)²·(x1·4) + 6(1-u)u·(1+x2·4 - x1·4) + 3u²·(1 - (1+x2·4))`
///
/// 初值用 u=t（直线时精确）。6 次迭代对 [0,1] 范围足够收敛。
#[inline]
fn solve_cubic_bezier_u_for_x(t: f32, x1: f32, x2: f32) -> f32 {
    let p1x = x1 * SegmentShape::SCALE;
    let p2x = 1.0 + x2 * SegmentShape::SCALE;
    let mut u = t.clamp(0.0, 1.0);
    for _ in 0..6 {
        let u1 = 1.0 - u;
        let f = 3.0 * u1 * u1 * u * p1x + 3.0 * u1 * u * u * p2x + u * u * u - t;
        let df = 3.0 * u1 * u1 * p1x + 6.0 * u1 * u * (p2x - p1x) + 3.0 * u * u * (1.0 - p2x);
        if df.abs() < 1e-6 {
            break;
        }
        u -= f / df;
        u = u.clamp(0.0, 1.0);
    }
    u
}

/// 自动化参数的宿主设备（统一参数模型，与 VST3/CLAP 的"设备 + 参数 id"对齐）。
///
/// 寻址不用效果器槽位序号（插入/删除/重排会变）：内置 DSP 参数是通道级的
/// （挂/删效果器不影响自动化归属）。第三方插入效果器的实例 uuid 寻址作为
/// 阶段 B 扩展（新增变体）。
///
/// 内置与第三方分开变体：两者参数 id 空间独立（内置 id 很小，第三方由插件
/// 定义），同一变体内混用会产生歧义。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ParamDevice {
    /// 通道内置 XSynth 参数（id 见 [`XSYNTH_PARAMS`]）。
    ChannelInstrument { channel: u8 },
    /// 通道乐器插件（VST3/CLAP）参数（id 为插件原生 id）。
    PluginInstrument { channel: u8 },
}

/// 内置参数的 MIDI 绑定（导入/导出/回放的双向映射）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MidiBinding {
    /// CC 控制器号（0–127）。
    Cc(u8),
    /// Pitch Bend（14-bit，中心 8192）。
    PitchBend,
    /// RPN 参数号（0–16383）。
    Rpn(u16),
}

/// 内置参数条目：id 在所属设备内唯一。
///
/// 值域约定（统一参数模型）：事件 `value` 一律归一化 `0..1`；`default` 为
/// 归一化默认值；显示/导出/回放按 [`MidiBinding`] 换算回原始整数。
#[derive(Clone, Copy, Debug)]
pub struct BuiltinParamInfo {
    /// 设备内唯一参数 id（u32，与 VST3 ParamID / CLAP clap_id 对齐）。
    pub id: u32,
    /// 参数显示名（与效果器/合成器面板一致）。
    pub name: &'static str,
    /// MIDI 绑定（导入导出的双向映射）。
    pub midi: MidiBinding,
    /// 归一化默认值（0..1）。
    pub default: f32,
    /// 是否有中心参考线（归一化 0.5 中心）。
    pub center: bool,
    /// 开关类参数：新建事件默认 `Step` 形状。
    pub step: bool,
}

/// XSynth 内置参数 id（`ParamDevice::ChannelInstrument`）。
pub mod xsynth_param {
    /// Sustain（CC64）。
    pub const SUSTAIN: u32 = 0;
    /// Release（CC72）。
    pub const RELEASE: u32 = 1;
    /// Attack（CC73）。
    pub const ATTACK: u32 = 2;
    /// Pitch Bend（PB）。
    pub const PITCH_BEND: u32 = 3;
    /// Pitch Bend Sensitivity（RPN 0，半音数）。
    pub const PB_SENSITIVITY: u32 = 4;
    /// Fine Tune（RPN 1，14-bit 中心 8192）。
    pub const FINE_TUNE: u32 = 5;
    /// Coarse Tune（RPN 2，0..127 表示 -64..+63 半音）。
    pub const COARSE_TUNE: u32 = 6;
}

/// 通道 DSP 参数（ChannelGain/Pan/Filter）的 id 与 MIDI 绑定表。
///
/// 无 `ParamDevice` 承载：CC 广播方案下它们作为低层 `CC` 事件存储，
/// 本表供 UI 把 CC 号映射为参数名（如 CC7 ↔ "Volume"）与 dsp 一致性校验。
pub mod channel_dsp_param {
    /// 音量（CC7）。
    pub const VOLUME: u32 = 0;
    /// 表情（CC11）。
    pub const EXPRESSION: u32 = 1;
    /// 声像（CC10，中心 64）。
    pub const PAN: u32 = 2;
    /// 低通截止（CC74，中心 64）。
    pub const CUTOFF: u32 = 3;
    /// 共振（CC71，中心 64）。
    pub const RESONANCE: u32 = 4;
}

/// XSynth 内置参数表（权威唯一：导入/导出/引擎/UI 共用）。
pub const XSYNTH_PARAMS: &[BuiltinParamInfo] = &[
    BuiltinParamInfo {
        id: xsynth_param::SUSTAIN,
        name: "Sustain",
        midi: MidiBinding::Cc(64),
        default: 0.0,
        center: false,
        step: true,
    },
    BuiltinParamInfo {
        id: xsynth_param::RELEASE,
        name: "Release",
        midi: MidiBinding::Cc(72),
        default: 64.0 / 127.0,
        center: true,
        step: false,
    },
    BuiltinParamInfo {
        id: xsynth_param::ATTACK,
        name: "Attack",
        midi: MidiBinding::Cc(73),
        default: 64.0 / 127.0,
        center: true,
        step: false,
    },
    BuiltinParamInfo {
        id: xsynth_param::PITCH_BEND,
        name: "Pitch Bend",
        midi: MidiBinding::PitchBend,
        default: 8192.0 / 16383.0,
        center: true,
        step: false,
    },
    BuiltinParamInfo {
        id: xsynth_param::PB_SENSITIVITY,
        name: "PB Sensitivity",
        midi: MidiBinding::Rpn(0),
        default: 2.0 / 127.0,
        center: false,
        step: false,
    },
    BuiltinParamInfo {
        id: xsynth_param::FINE_TUNE,
        name: "Fine Tune",
        midi: MidiBinding::Rpn(1),
        default: 8192.0 / 16383.0,
        center: true,
        step: false,
    },
    BuiltinParamInfo {
        id: xsynth_param::COARSE_TUNE,
        name: "Coarse Tune",
        midi: MidiBinding::Rpn(2),
        default: 64.0 / 127.0,
        center: true,
        step: false,
    },
];

/// 通道内置 DSP 参数表。
///
/// 与 yinhe-dsp registry `EffectParamInfo` 的一致性由后者侧测试锁定
/// （id 顺序、名称、绑定 cc、默认值）。
pub const CHANNEL_DSP_PARAMS: &[BuiltinParamInfo] = &[
    BuiltinParamInfo {
        id: channel_dsp_param::VOLUME,
        name: "Volume",
        midi: MidiBinding::Cc(7),
        default: 1.0,
        center: false,
        step: false,
    },
    BuiltinParamInfo {
        id: channel_dsp_param::EXPRESSION,
        name: "Expression",
        midi: MidiBinding::Cc(11),
        default: 1.0,
        center: false,
        step: false,
    },
    BuiltinParamInfo {
        id: channel_dsp_param::PAN,
        name: "Pan",
        midi: MidiBinding::Cc(10),
        default: 64.0 / 127.0,
        center: true,
        step: false,
    },
    BuiltinParamInfo {
        id: channel_dsp_param::CUTOFF,
        name: "Cutoff",
        midi: MidiBinding::Cc(74),
        default: 64.0 / 127.0,
        center: true,
        step: false,
    },
    BuiltinParamInfo {
        id: channel_dsp_param::RESONANCE,
        name: "Resonance",
        midi: MidiBinding::Cc(71),
        default: 64.0 / 127.0,
        center: true,
        step: false,
    },
];

impl ParamDevice {
    /// 设备的内置参数表（第三方插件返回空表，参数由插件定义）。
    pub fn builtin_params(&self) -> &'static [BuiltinParamInfo] {
        match self {
            ParamDevice::ChannelInstrument { .. } => XSYNTH_PARAMS,
            ParamDevice::PluginInstrument { .. } => &[],
        }
    }

    /// 宿主通道（MIDI 全局通道 0..255）。
    pub fn channel(&self) -> u8 {
        match self {
            ParamDevice::ChannelInstrument { channel }
            | ParamDevice::PluginInstrument { channel } => *channel,
        }
    }
}

/// 查内置参数条目（`None` = 非内置参数，如第三方插件参数）。
pub fn builtin_param(device: &ParamDevice, id: u32) -> Option<&'static BuiltinParamInfo> {
    device.builtin_params().iter().find(|p| p.id == id)
}

/// MIDI 绑定 → XSynth 参数 id（导入映射）。
pub fn xsynth_param_id_for_midi(midi: MidiBinding) -> Option<u32> {
    XSYNTH_PARAMS.iter().find(|p| p.midi == midi).map(|p| p.id)
}

/// MIDI 绑定的原始值上限（7-bit / 14-bit）。
pub fn binding_max(midi: MidiBinding) -> f32 {
    match midi {
        MidiBinding::Cc(_) | MidiBinding::Rpn(0) | MidiBinding::Rpn(2) => 127.0,
        MidiBinding::PitchBend | MidiBinding::Rpn(_) => 16383.0,
    }
}

/// Identifies an automatable parameter.
///
/// 统一参数模型（spec-yinhe-dsp）：设备参数走 [`AutomationTarget::Param`]
/// （值域归一化 0..1，与 VST3/CLAP 对齐）；无法归属到设备的低层 MIDI 数据
/// 保留 [`AutomationTarget::CC`]/[`AutomationTarget::Rpn`]/
/// [`AutomationTarget::Nrpn`]（值域同样归一化）；[`AutomationTarget::Tempo`]
/// 是时间轴数据，值保持 BPM 原值（唯一不归一化的量）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AutomationTarget {
    /// 设备参数（内置 XSynth/DSP 或第三方插件）。`value` 归一化 0..1。
    Param {
        device: ParamDevice,
        /// 设备内唯一参数 id（内置设备见 [`XSYNTH_PARAMS`]/[`CHANNEL_DSP_PARAMS`]）。
        id: u32,
        /// 参数显示名（第三方插件参数必填；内置参数以表为准，可为空）。
        name: String,
    },
    /// 无设备归属的 MIDI CC（`value` 归一化 0..1）。
    CC { controller: u8 },
    /// 无设备归属的标准 RPN（`value` 归一化 0..1）。
    Rpn { parameter: u16 },
    /// NRPN（`value` 归一化 0..1）。
    Nrpn { parameter: u16 },
    /// Tempo（BPM）。全局唯一一条 lane，存于 `ConductorData.tempo`。
    /// `value` 直接装 bpm（f32），是唯一不归一化的量。
    Tempo,
}

impl AutomationTarget {
    /// 宿主通道（`Param` 取 device 的通道；其他变体由 lane 所在轨决定，返回 `None`）。
    pub fn channel(&self) -> Option<u8> {
        match self {
            AutomationTarget::Param { device, .. } => Some(device.channel()),
            _ => None,
        }
    }

    /// 显示值上限：把归一化 value 换算回原始整数用；`None` = 原值显示
    /// （Tempo 是 BPM；第三方插件参数范围未知，按归一化显示）。
    pub fn display_max(&self) -> Option<f32> {
        match self {
            AutomationTarget::Param { device, id, .. } => {
                builtin_param(device, *id).map(|p| binding_max(p.midi))
            }
            AutomationTarget::CC { .. } => Some(127.0),
            AutomationTarget::Rpn { parameter } => Some(rpn_max(*parameter)),
            AutomationTarget::Nrpn { .. } => Some(16383.0),
            AutomationTarget::Tempo => None,
        }
    }

    /// 默认值：归一化 0..1（Tempo 为 BPM 原值）。
    pub fn default_value(&self) -> f32 {
        match self {
            AutomationTarget::Param { device, id, .. } => {
                builtin_param(device, *id).map(|p| p.default).unwrap_or(0.0)
            }
            AutomationTarget::CC {
                controller: 10 | 71 | 72 | 73 | 74,
            } => 64.0 / 127.0,
            AutomationTarget::CC { .. } => 0.0,
            AutomationTarget::Rpn { .. } | AutomationTarget::Nrpn { .. } => 0.0,
            AutomationTarget::Tempo => 120.0,
        }
    }

    /// 是否有中心参考线（归一化 0.5 中心）。
    pub fn has_center_line(&self) -> bool {
        match self {
            AutomationTarget::Param { device, id, .. } => {
                builtin_param(device, *id).is_some_and(|p| p.center)
            }
            AutomationTarget::CC { controller } => matches!(controller, 10 | 71 | 72 | 73 | 74),
            _ => false,
        }
    }

    /// 新建事件默认形状：开关类 Step，其余直线 Curve。
    pub fn default_shape(&self) -> SegmentShape {
        match self {
            AutomationTarget::Param { device, id, .. } => {
                if builtin_param(device, *id).is_some_and(|p| p.step) {
                    SegmentShape::Step
                } else {
                    SegmentShape::linear_curve()
                }
            }
            AutomationTarget::CC {
                controller: 64..=68,
            } => SegmentShape::Step,
            _ => SegmentShape::linear_curve(),
        }
    }

    /// 归一化值 → 显示值（UI 显示换算的唯一入口；`display_max` 为 `None`
    /// 时原样返回，如 Tempo 的 BPM、第三方插件参数的归一化值）。
    pub fn to_display_value(&self, value: f32) -> f32 {
        match self.display_max() {
            Some(max) => value * max,
            None => value,
        }
    }

    /// 显示值 → 归一化值（编辑写入换算的唯一入口，与
    /// [`AutomationTarget::to_display_value`] 互逆）。
    pub fn from_display_value(&self, display: f32) -> f32 {
        match self.display_max() {
            Some(max) if max > 0.0 => display / max,
            _ => display,
        }
    }

    /// 显示名（下拉/AR/事件浏览器共用）。
    pub fn display_name(&self) -> String {
        match self {
            AutomationTarget::Param { device, id, name } => match builtin_param(device, *id) {
                Some(p) => p.name.to_string(),
                None if !name.is_empty() => name.clone(),
                None => match device {
                    ParamDevice::PluginInstrument { channel } => {
                        format!("Plugin Param {id} (ch {channel})")
                    }
                    _ => format!("Param {id}"),
                },
            },
            AutomationTarget::CC { controller } => {
                let name = cc_name(*controller);
                if name.is_empty() {
                    format!("CC {}", controller)
                } else {
                    format!("CC {} ({})", controller, name)
                }
            }
            AutomationTarget::Rpn { parameter } => format!("RPN {}", parameter),
            AutomationTarget::Nrpn { parameter } => format!("NRPN {}", parameter),
            AutomationTarget::Tempo => "Tempo".into(),
        }
    }
}

/// RPN 参数号的原始值上限（RPN 0/2 是 7-bit，其余 14-bit）。
fn rpn_max(parameter: u16) -> f32 {
    match parameter {
        0 | 2 => 127.0,
        _ => 16383.0,
    }
}

/// Common MIDI CC names (standard GM/GS assignments).
fn cc_name(cc: u8) -> &'static str {
    match cc {
        0 => "Bank Select MSB",
        1 => "Mod Wheel",
        2 => "Breath",
        4 => "Foot",
        5 => "Portamento Time",
        6 => "Data Entry MSB",
        7 => "Volume",
        8 => "Balance",
        10 => "Pan",
        11 => "Expression",
        32 => "Bank Select LSB",
        38 => "Data Entry LSB",
        64 => "Sustain",
        65 => "Portamento",
        66 => "Sostenuto",
        67 => "Soft Pedal",
        68 => "Legato",
        71 => "Resonance",
        72 => "Release",
        73 => "Attack",
        74 => "Cutoff",
        84 => "Portamento Control",
        91 => "Reverb",
        92 => "Tremolo",
        93 => "Chorus",
        94 => "Detune",
        95 => "Phaser",
        100 => "RPN LSB",
        101 => "RPN MSB",
        _ => "",
    }
}

/// A single automation event: a value at a point in time.
///
/// Channel and track are not stored here — they are implied by the
/// owning `AutomationLane` (which mirrors `TrackData`'s per-track design).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationEvent {
    /// 全局唯一身份（0 = 未分配）。
    ///
    /// 不落盘（`serde(skip)`）：postcard 非自描述，新增序列化字段会破坏旧
    /// `.yin` 文件；加载后由 `YinModel::renumber_automation_ids` 统一发号，
    /// 与 note id 的"会话内身份（选择集/undo/音频匹配）"设计一致。
    #[serde(skip)]
    pub id: u32,
    pub tick: u32,
    /// 归一化值 0..1（统一参数模型）；Tempo 例外，存 bpm（如 120.0）。
    /// 原始整数语义（CC 0..127、PB 0..16383）在导入时归一化，导出/回放时
    /// 按 target 的 [`AutomationTarget::display_max`] 还原（无损往返）。
    pub value: f32,
    /// 描述"从本事件到下一事件"的插值形状。
    /// 默认 `Step`（保留 MIDI 原生语义），编辑器新建事件时由
    /// `AutomationTarget::default_shape()` 提供更合适的默认。
    #[serde(default)]
    pub shape: SegmentShape,
}

/// A sorted lane of automation events for one parameter on one track.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    /// Track index (matches `TrackData` position in `YinModel.tracks`).
    pub track: u16,
    /// Events sorted by `tick`.
    pub events: Vec<AutomationEvent>,
}

impl AutomationLane {
    /// Returns a slice of events whose `tick` falls in `[start_tick, end_tick)`.
    ///
    /// Uses binary search since `events` is sorted by tick.
    pub fn events_in_range(&self, start_tick: u32, end_tick: u32) -> &[AutomationEvent] {
        let lo = self.events.partition_point(|e| e.tick < start_tick);
        let hi = self.events.partition_point(|e| e.tick < end_tick);
        &self.events[lo..hi]
    }

    /// 求 `target` 处的值（chase/预览/UI 共用）：
    /// - Step：最后一条 `tick < target` 的事件值（保持语义）；
    /// - Linear/Curve：target 落在段内时**实时插值**（真实值，与 flatten 的 density 无关）；
    ///   target == 下一事件 tick 时取曲线终点值（连续），Step 则保持上一值。
    ///
    /// 返回 `(value, tick)`；tick 用于与播放事件流一致的排序（曲线插值用 target）。
    pub fn value_at(&self, target: u32) -> Option<(f32, u32)> {
        let events = &self.events;
        let idx = events.partition_point(|e| e.tick < target);
        if idx == 0 {
            return None; // target 之前没有任何事件
        }
        let e = &events[idx - 1];
        if idx < events.len() {
            let next = &events[idx];
            if !matches!(e.shape, SegmentShape::Step) && target < next.tick {
                // 曲线段内：插值真实值（事件 tick 用 target，排序时位于本段生效点）
                let frac = (target - e.tick) as f32 / (next.tick - e.tick) as f32;
                let v = e.value + (next.value - e.value) * e.shape.interpolate(frac);
                return Some((v, target));
            }
            if !matches!(e.shape, SegmentShape::Step) && target == next.tick {
                // 曲线终点：连续到达 next.value（下一事件 tick == target 由 dispatch 处理，
                // chase 提供同值兜底，chase_skip 会跳过已 dispatch 的控制器）
                return Some((next.value, next.tick));
            }
        }
        Some((e.value, e.tick))
    }
}

/// 用户在 automation 面板上的编辑操作。
///
/// 由 automation 面板产生，由 `Document::apply_automation_edits` 应用。
///
/// `target` 字段在所有变体上都存在，让 `apply_automation_edits` 可以直接
/// 根据 target 分派到 `track.automation_lanes` 或 `conductor.tempo`，
/// 不依赖 `lane_idx` 来推断存储位置。
#[derive(Clone, Debug)]
pub enum AutomationEdit {
    /// 添加新事件。如果 lane 不存在会自动创建。
    Add {
        track_idx: u16,
        target: AutomationTarget,
        tick: u32,
        value: f32,
        shape: SegmentShape,
    },
    /// 移动已有事件。
    Move {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        old_tick: u32,
        new_tick: u32,
        new_value: f32,
    },
    /// 批量移动同一 lane 上的多个事件（一次 undo 快照）。
    ///
    /// `moves = [(old_tick, new_tick, new_value)]`。
    /// 用于 Select 工具拖拽多个选中锚点：先移除所有 old_tick，
    /// 再按 new_tick 排序后插入，避免逐个 Move 导致链式覆盖
    /// （如 1→2, 2→3 时 1→2 会删掉原 2，2→3 找不到原 2）。
    MoveBatch {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        moves: Vec<(u32, u32, f32)>,
    },
    /// 切换已有事件的 shape（双击）。
    CycleShape {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        tick: u32,
    },
    /// 直接设置已有事件的 shape（用于控制点拖拽）。
    SetShape {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        tick: u32,
        shape: SegmentShape,
    },
    /// 删除已有事件。
    Delete {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        tick: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lane(target: AutomationTarget, ticks: &[u32]) -> AutomationLane {
        AutomationLane {
            target,
            track: 0,
            events: ticks
                .iter()
                .map(|&t| AutomationEvent {
                    id: 0,
                    tick: t,
                    value: 64.0,
                    shape: SegmentShape::Step,
                })
                .collect(),
        }
    }

    #[test]
    fn test_events_in_range() {
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[100, 200, 300, 400, 500],
        );
        let slice = lane.events_in_range(150, 450);
        assert_eq!(slice.len(), 3);
        assert_eq!(slice[0].tick, 200);
        assert_eq!(slice[2].tick, 400);
    }

    #[test]
    fn test_events_in_range_empty() {
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[100, 200]);
        assert!(lane.events_in_range(300, 400).is_empty());
    }

    /// 内置参数表一致性：id 与 MIDI 绑定在表内唯一、默认值在 0..1。
    #[test]
    fn test_builtin_param_tables_are_consistent() {
        for table in [XSYNTH_PARAMS, CHANNEL_DSP_PARAMS] {
            let mut ids: Vec<u32> = table.iter().map(|p| p.id).collect();
            ids.sort_unstable();
            let total = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), total, "参数 id 必须唯一");

            let mut binds: Vec<String> = table.iter().map(|p| format!("{:?}", p.midi)).collect();
            binds.sort();
            let total = binds.len();
            binds.dedup();
            assert_eq!(binds.len(), total, "MIDI 绑定必须唯一");

            for p in table {
                assert!(
                    (0.0..=1.0).contains(&p.default),
                    "{} 默认值必须在 0..1",
                    p.name
                );
            }
        }
    }

    /// 导入映射：MIDI 绑定 → 内置参数 id（含不命中的否定断言）。
    #[test]
    fn test_midi_reverse_lookup() {
        assert_eq!(
            xsynth_param_id_for_midi(MidiBinding::Cc(64)),
            Some(xsynth_param::SUSTAIN)
        );
        assert_eq!(
            xsynth_param_id_for_midi(MidiBinding::PitchBend),
            Some(xsynth_param::PITCH_BEND)
        );
        assert_eq!(
            xsynth_param_id_for_midi(MidiBinding::Rpn(0)),
            Some(xsynth_param::PB_SENSITIVITY)
        );
        assert_eq!(xsynth_param_id_for_midi(MidiBinding::Cc(7)), None);
    }

    fn param(device: &ParamDevice, id: u32) -> AutomationTarget {
        AutomationTarget::Param {
            device: device.clone(),
            id,
            name: String::new(),
        }
    }

    /// 设备参数：内置查表（显示名/上限/默认/中心/形状），第三方走缓存名。
    #[test]
    fn test_param_target_methods() {
        let xs = ParamDevice::ChannelInstrument { channel: 0 };
        let sus = param(&xs, xsynth_param::SUSTAIN);
        assert_eq!(sus.default_shape(), SegmentShape::Step);
        assert_eq!(sus.display_max(), Some(127.0));
        assert!(!sus.has_center_line());

        let pb = param(&xs, xsynth_param::PITCH_BEND);
        assert_eq!(pb.display_max(), Some(16383.0));
        assert!(pb.has_center_line());

        // 第三方插件参数：无内置表 → 归一化显示（无换算上限），名字用缓存。
        let plug = AutomationTarget::Param {
            device: ParamDevice::PluginInstrument { channel: 2 },
            id: 42,
            name: "Cutoff".into(),
        };
        assert_eq!(plug.display_name(), "Cutoff");
        assert_eq!(plug.display_max(), None);
        assert_eq!(plug.default_value(), 0.0);
        assert!(!plug.has_center_line());
    }

    /// 低层变体：CC/RPN/NRPN 的显示与换算上限；Tempo 原值。
    #[test]
    fn test_low_level_targets() {
        let cc7 = AutomationTarget::CC { controller: 7 };
        assert_eq!(cc7.display_name(), "CC 7 (Volume)");
        assert_eq!(cc7.display_max(), Some(127.0));
        assert_eq!(cc7.default_value(), 0.0);
        assert!(!cc7.has_center_line());
        assert_eq!(
            AutomationTarget::CC { controller: 64 }.default_shape(),
            SegmentShape::Step
        );
        assert_eq!(
            AutomationTarget::CC { controller: 7 }.default_shape(),
            SegmentShape::linear_curve()
        );

        assert_eq!(
            AutomationTarget::Rpn { parameter: 5 }.display_name(),
            "RPN 5"
        );
        assert_eq!(
            AutomationTarget::Rpn { parameter: 5 }.display_max(),
            Some(16383.0)
        );
        assert_eq!(
            AutomationTarget::Rpn { parameter: 0 }.display_max(),
            Some(127.0)
        );
        assert_eq!(
            AutomationTarget::Nrpn { parameter: 1 }.display_max(),
            Some(16383.0)
        );
        assert_eq!(AutomationTarget::Tempo.display_max(), None);
        assert_eq!(AutomationTarget::Tempo.default_value(), 120.0);
        assert_eq!(
            AutomationTarget::Tempo.default_shape(),
            SegmentShape::linear_curve()
        );
    }

    #[test]
    fn test_segment_shape_interpolate_endpoints() {
        // Step 在区间内始终返回 0（值仍为 v1，由调用方处理）
        assert_eq!(SegmentShape::Step.interpolate(0.0), 0.0);
        assert_eq!(SegmentShape::Step.interpolate(0.5), 0.0);
        assert_eq!(SegmentShape::Step.interpolate(1.0), 0.0);

        // 直线 Curve（偏移量全 0）端点和中点
        let lin = SegmentShape::linear_curve();
        assert_eq!(lin.interpolate(0.0), 0.0);
        assert_eq!(lin.interpolate(1.0), 1.0);
        assert!((lin.interpolate(0.5) - 0.5).abs() < 1e-6);

        // 贝塞尔端点：无论控制点位置，端点始终为 0 和 1
        // 偏移量 (x1,y1,x2,y2)：P1=(x1*4,y1*4), P2=(1+x2*4, 1+y2*4)
        assert_eq!(
            SegmentShape::Curve {
                x1: 0.1,
                y1: 0.2,
                x2: -0.1,
                y2: -0.2
            }
            .interpolate(0.0),
            0.0
        );
        assert_eq!(
            SegmentShape::Curve {
                x1: 0.1,
                y1: 0.2,
                x2: -0.1,
                y2: -0.2
            }
            .interpolate(1.0),
            1.0
        );
        assert_eq!(
            SegmentShape::Curve {
                x1: 0.25,
                y1: -0.5,
                x2: -0.25,
                y2: 0.5
            }
            .interpolate(0.0),
            0.0
        );
        assert_eq!(
            SegmentShape::Curve {
                x1: 0.25,
                y1: -0.5,
                x2: -0.25,
                y2: 0.5
            }
            .interpolate(1.0),
            1.0
        );
    }

    #[test]
    fn test_segment_shape_bezier_midpoint() {
        // 直线（偏移量全 0）：B_y(0.5) = 0.5
        assert!((SegmentShape::linear_curve().interpolate(0.5) - 0.5).abs() < 1e-6);

        // ease-in-out 近似 CSS cubic-bezier(0.42, 0, 0.58, 1)
        // → 偏移量 (x1=0.42/4, y1=0, x2=(0.58-1)/4, y2=(1-1)/4) = (0.105, 0, -0.105, 0)
        // B_y(0.5) 接近 0.5
        let ease_io = SegmentShape::Curve {
            x1: 0.105,
            y1: 0.0,
            x2: -0.105,
            y2: 0.0,
        };
        let v = ease_io.interpolate(0.5);
        assert!(
            (v - 0.5).abs() < 0.02,
            "ease-in-out mid expected ~0.5, got {v}"
        );

        // 控制点全部偏到 v_end（实际 y1=y2=1）：
        // P1.y = y1*4 = 1 → y1 = 0.25；P2.y = 1 + y2*4 = 1 → y2 = 0
        // B_y(0.5) = 0.375*1 + 0.375*1 + 0.125 = 0.875
        let v_end = SegmentShape::Curve {
            x1: 0.075,
            y1: 0.25,
            x2: -0.075,
            y2: 0.0,
        }
        .interpolate(0.5);
        assert!((v_end - 0.875).abs() < 1e-6, "expected 0.875, got {v_end}");

        // 控制点全部偏到 v_start（实际 y1=y2=0）：
        // P1.y = y1*4 = 0 → y1 = 0；P2.y = 1 + y2*4 = 0 → y2 = -0.25
        // B_y(0.5) = 0.125
        let v_start = SegmentShape::Curve {
            x1: 0.075,
            y1: 0.0,
            x2: -0.075,
            y2: -0.25,
        }
        .interpolate(0.5);
        assert!(
            (v_start - 0.125).abs() < 1e-6,
            "expected 0.125, got {v_start}"
        );
    }

    #[test]
    fn test_segment_shape_is_linear() {
        assert!(SegmentShape::linear_curve().is_linear());
        assert!(
            !SegmentShape::Curve {
                x1: 0.0,
                y1: 0.1,
                x2: 0.0,
                y2: 0.0
            }
            .is_linear()
        );
        assert!(
            !SegmentShape::Curve {
                x1: 0.1,
                y1: 0.0,
                x2: 0.0,
                y2: 0.0
            }
            .is_linear()
        );
        assert!(
            !SegmentShape::Curve {
                x1: 0.0,
                y1: 0.0,
                x2: 0.1,
                y2: 0.0
            }
            .is_linear()
        );
        assert!(!SegmentShape::Step.is_linear());
    }
}
