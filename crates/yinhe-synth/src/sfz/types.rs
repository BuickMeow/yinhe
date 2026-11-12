//! 类型化 SFZ 数据模型。
//!
//! 只承载**音源层需要**的字段；ARIA 里引擎尚不实现的（`effect1`、`fillfo_*`、
//! `sustain_sw`、`pitch_keytrack`）也解析出来，供诊断与后续实现使用，但不参与
//! 参数展开。

use std::ops::RangeInclusive;
use std::path::PathBuf;

use xsynth_soundfonts::FilterType;

/// 采样触发时机（SFZ `trigger`）。
///
/// `Release` 用于钢琴踏板/键释放噪声：在音符结束（或踏板松开）时发声，
/// 而不是 note_on。此前整类 opcode 被引擎丢弃，导致音尾"死得没有理由"。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    #[default]
    Attack,
    Release,
}

/// 交叉淡化门（`xfin_*` / `xfout_*`）。
///
/// `xfin_loccN/hiccN`：CC 从 lo 到 hi 线性淡入；`xfout_*` 反向淡出。
/// 二者可同时存在（先淡入再淡出），增益相乘。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CcGate {
    pub cc: u8,
    pub lo: u8,
    pub hi: u8,
    pub dir: GateDir,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateDir {
    In,
    Out,
}

impl CcGate {
    /// 线性增益：`xfin` 从 lo 淡入到 hi，`xfout` 反向。
    #[inline]
    pub fn gain(&self, cc: &[u8; 128]) -> f32 {
        let v = cc[self.cc as usize] as f32;
        let (lo, hi) = (self.lo as f32, self.hi as f32);
        let t = if hi > lo {
            ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else if v >= hi {
            1.0
        } else {
            0.0
        };
        if self.dir == GateDir::Out { 1.0 - t } else { t }
    }
}

impl OnGate {
    #[inline]
    pub fn passes(&self, cc: &[u8; 128]) -> bool {
        let v = cc[self.cc as usize];
        v >= self.lo && v <= self.hi
    }
}

/// 二进制 CC 区间门（`on_loccN` / `on_hiccN`）：区间外整层不发声。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnGate {
    pub cc: u8,
    pub lo: u8,
    pub hi: u8,
}

/// SFZ 包络参数（秒 / 归一化）。
#[derive(Clone, Copy, Debug)]
pub struct Ampeg {
    pub start: f32,
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    /// 0..100（SFZ 百分比），展开时归一化。
    pub sustain: f32,
    pub release: f32,
    pub vel2release: f32,
}

impl Default for Ampeg {
    fn default() -> Self {
        // 默认对齐 xsynth（`AmpegEnvelopeParams::default`）：attack/release 10ms
        // 而非 SFZ 规范的 1ms。音色库不写 release 时（如 Starry），1ms 会让
        // 每个音符结束像硬切（密集音符连成 click）。
        Self {
            start: 0.0,
            delay: 0.0,
            attack: 0.01,
            hold: 0.0,
            decay: 0.001,
            sustain: 100.0,
            release: 0.01,
            vel2release: 0.0,
        }
    }
}

/// 循环模式（与 SFZ `loop_mode` 一一对应）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopMode {
    #[default]
    NoLoop,
    OneShot,
    LoopContinuous,
    LoopSustain,
}

/// LFO 定义（振幅/音高/滤波共用字段；语义按用途解释）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    /// 深度：振幅 LFO 为 dB，音高 LFO 为 cents，滤波 LFO 为 cents。
    pub depth: f32,
    /// 频率（Hz）。
    pub freq: f32,
    pub phase: f32,
}

/// 展开后的一条 SFZ region。
#[derive(Clone, Debug)]
pub struct SfzRegion {
    pub sample_path: PathBuf,
    /// 原始 `lokey..hikey`；可能含 -1（`c#-1`），展开时裁到 0..=127。
    pub keyrange: RangeInclusive<i32>,
    pub velrange: RangeInclusive<u8>,
    pub pitch_keycenter: i32,
    /// 音分（`tune`）。
    pub tune: f32,
    pub volume_db: f32,
    /// -100..100。
    pub pan: f32,
    pub amp_veltrack: f32,
    pub amp_keytrack: f32,
    pub amp_keycenter: i32,
    pub pan_veltrack: f32,
    pub pan_keytrack: f32,
    pub pan_keycenter: i32,
    pub fil_veltrack: f32,
    pub fil_keytrack: f32,
    pub fil_keycenter: i32,
    pub filter_type: FilterType,
    pub cutoff: Option<f32>,
    pub resonance_db: f32,
    pub ampeg: Ampeg,
    pub loop_mode: LoopMode,
    pub loop_start: u32,
    pub loop_end: u32,
    pub offset: u32,
    pub trigger: Trigger,
    /// SFZ `group`（组标签，非 NoteOn 层组）。
    pub group: u32,
    /// 新音触发时掐掉的组。
    pub off_by: u32,
    pub seq_length: u32,
    pub seq_position: u32,
    pub sw_last: Option<u8>,
    pub sw_lokey: Option<u8>,
    pub sw_hikey: Option<u8>,
    pub cc_gates: Vec<CcGate>,
    pub on_gates: Vec<OnGate>,
    pub lorand: Option<f32>,
    pub hirand: Option<f32>,
    /// 音分抖动（`pitch_random`）。
    pub pitch_random: f32,
    pub amplfo: Option<Lfo>,
    pub pitchlfo: Option<Lfo>,
    pub fillfo: Option<Lfo>,
    pub effect1: f32,
}

/// 一个 SFZ 文件的解析结果。
#[derive(Clone, Debug, Default)]
pub struct SfzFile {
    pub regions: Vec<SfzRegion>,
    /// `<control> set_ccN=v`：加载时写入通道初始 CC。
    ///
    /// 必需：Starry 的键/锤/踏板三层都用 `xfin_locc14/15/20` 门控，
    /// 不先写入 `set_cc14=127 / 15=85 / 20=100` 会让整架钢琴静音或丢层。
    pub cc_defaults: Vec<(u8, u8)>,
    /// 引擎尚不支持的 opcode → 出现次数（诊断用）。
    pub unknown_opcodes: std::collections::BTreeMap<String, usize>,
    /// 引用但读不到采样文件的 region 数（诊断用）。
    pub missing_samples: usize,
}
