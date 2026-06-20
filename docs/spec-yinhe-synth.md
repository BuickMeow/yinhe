# yinhe-synth 设计方案

## 目标

渐进式替换 `xsynth-core` 的渲染引擎，保留 `xsynth-soundfonts` 做 SF2/SFZ 解析。

- **替换**：voice 渲染、channel 管理、CC 处理、ChannelGroup 并行
- **保留**：`xsynth-soundfonts` 的 SF2/SFZ 文件解析（`load_soundfont()` / `parse_soundfont()`）
- **不依赖**：`xsynth-core`（完全脱离 LGPL 约束）
- **SIMD**：nightly + `std::simd`，Apple Silicon 编译为 NEON

## 总体架构

```
xsynth-soundfonts (SF2/SFZ raw parsing -> Sf2Region / SfzRegion)
        |
        v
yinhe-synth::soundfont::loader    <- 唯一接触 xsynth-soundfonts 的地方
        | 将 Sf2Region/SfzRegion 转换为 VoiceSpawnerParams
        v
yinhe-synth::soundfont::spawner   <- VoiceSpawnerMatrix (bank/preset -> key/vel -> params)
        |
        v  spawn_voice()
yinhe-synth::voice                <- 采样器 + 包络 + 滤波 + SIMD 渲染
        |
        v  render_to()
yinhe-synth::channel              <- 128 key 管理 + CC + voice 限制 + 通道效果
        |
        v  read_samples()
yinhe-synth::channel_group        <- 多通道 rayon 并行混合
        |
        v
yinhe-audio::engine               <- 现有的事件调度/播放/导出（不变）
```

## Crate 结构（现代 Rust 写法，不使用 mod.rs）

```
crates/yinhe-synth/
+- Cargo.toml
+- src/
    +- lib.rs                    # crate 入口，pub mod 声明 + re-exports
    |
    +- voice.rs                  # Voice trait, VoiceControlData, ReleaseType, SampledVoice
    +- voice/
    |   +- envelope.rs           # DADSR 包络 + concave/convex/linear 曲线
    |   +- sampler.rs            # 采样播放：插值 + 循环 + release
    |   +- filter.rs             # per-voice biquad 滤波器
    |
    +- channel.rs                # VoiceChannel: 128 key + CC + 通道效果
    +- channel/
    |   +- control.rs            # ControlEventData: CC/RPN/PitchBend 状态
    |   +- key.rs                # KeyData: per-key voice 管理
    |   +- voice_buffer.rs       # VoiceBuffer: voice 列表 + voice stealing + damper
    |
    +- channel_group.rs          # ChannelGroup: 多通道并行渲染
    +- channel_group/
    |   +- config.rs             # ChannelGroupConfig, SynthFormat, ParallelismOptions
    |
    +- soundfont.rs              # SoundfontSource trait, VoiceSpawner trait
    +- soundfont/
    |   +- spawner.rs            # VoiceSpawnerMatrix + Mono/StereoSampledVoiceSpawner
    |   +- loader.rs             # 从 xsynth-soundfonts 加载并构建 spawner matrix
    |   +- config.rs             # SoundfontInitOptions, Interpolation, EnvelopeOptions
    |
    +- events.rs                 # ChannelAudioEvent, ChannelConfigEvent, SynthEvent
```

**模块规则**：`foo.rs` 声明子模块 `mod bar;`，子模块文件为 `foo/bar.rs`。不使用 `foo/mod.rs`。

## SIMD 策略

### 方案：nightly + `std::simd`

```toml
[package]
name = "yinhe-synth"
edition = "2024"
```

```rust
// lib.rs
#![feature(portable_simd)]
```

整个项目用 nightly（`rustup override set nightly`）。Apple Silicon 上 `f32x4` 编译为 NEON，零性能损失。

### 为什么不用 simdeez

| 维度 | simdeez | std::simd |
|------|---------|-----------|
| 代码风格 | 泛型 `<S: Simd>` + `simd_invoke!` 宏 | 普通 Rust，`f32x4` / `f32x8` |
| AI 生成准确率 | 低（宏和泛型约束常出错） | 高（标准 API） |
| 跨平台 | 手写 4 份后端 | LLVM 自动选择 |
| Apple Silicon 性能 | NEON | NEON（相同） |
| 稳定性 | 稳定 crate | nightly feature |

### 渲染分块策略

voice 渲染以 **block 为单位**（不是逐 sample），block 大小 = 音频 callback buffer 大小（通常 64-512 samples）。

```
对每个 voice:
  1. 计算 block 内的包络增益序列 (f32x4 批量)
  2. 读取采样数据 (f32x4 批量插值)
  3. 采样 x 包络增益 (f32x4 乘法)
  4. 应用 per-voice 滤波器 (标量 biquad，IIR 无法简单 SIMD)
  5. 累加到 channel 输出缓冲区 (f32x4 加法)
```

**关键**：步骤 1-3 是 SIMD 的最大收益点。步骤 4 的 biquad 是 IIR（有反馈），标量处理即可——滤波器只在 voice 有 cutoff 时才运行。

### std::simd vs simdeez 代码对比

```rust
// std::simd: 普通 Rust，AI 直接能写对
use std::simd::f32x4;

fn apply_envelope_simd(samples: &[f32], gains: &[f32], out: &mut [f32]) {
    for (chunk, (s, g)) in out.chunks_exact_mut(4)
        .zip(samples.chunks_exact(4).zip(gains.chunks_exact(4)))
    {
        let result = f32x4::from_slice(s) * f32x4::from_slice(g);
        chunk.copy_from_slice(&result.to_array());
    }
}
```

```rust
// simdeez: 泛型 + 宏 + 运行时分发，AI 容易漏 simd_invoke! 或类型参数
fn apply_envelope_simd<S: Simd>(samples: &[f32], gains: &[f32], out: &mut [f32]) {
    simd_invoke!(S, {
        for (chunk, (s, g)) in out.chunks_exact_mut(S::Vf32::WIDTH)
            .zip(samples.chunks_exact(S::Vf32::WIDTH).zip(gains.chunks_exact(S::Vf32::WIDTH)))
        {
            let result = S::Vf32::load_from_slice(s) * S::Vf32::load_from_slice(g);
            result.store_unaligned(chunk);
        }
    });
}
// 调用端还需要 simd_runtime_generate! 宏做 CPUID 分发
```


## 核心数据结构

### 1. Voice 层 (`voice.rs`)

```rust
#[derive(Copy, Clone, PartialEq)]
pub enum ReleaseType { Standard, Kill }

#[derive(Copy, Clone)]
pub struct VoiceControlData {
    pub voice_pitch_multiplier: f32,
    pub envelope: EnvelopeControlData,
}

#[derive(Copy, Clone)]
pub struct EnvelopeControlData {
    pub attack: Option<u8>,   // CC73 覆盖，None = 用音色库原始值
    pub release: Option<u8>,  // CC72 覆盖
}

pub trait Voice: Send + Sync {
    fn render_to(&mut self, buffer: &mut [f32]);  // 累加，不覆盖
    fn ended(&self) -> bool;
    fn is_releasing(&self) -> bool;
    fn is_killed(&self) -> bool;
    fn velocity(&self) -> u8;
    fn exclusive_class(&self) -> Option<u8>;
    fn signal_release(&mut self, rel_type: ReleaseType);
    fn process_controls(&mut self, control: &VoiceControlData);
}

pub struct SampledVoice {
    sampler: Sampler,
    envelope: Envelope,
    filter: Option<BiQuadFilter>,
    amp: f32,
    pan: f32,
    velocity: u8,
    exclusive_class: Option<u8>,
    killed: bool,
}

impl Voice for SampledVoice {
    fn render_to(&mut self, buffer: &mut [f32]) {
        // 1. sampler.render_block(temp)
        // 2. envelope.render_block(gains)
        // 3. temp *= gains  (f32x4 批量乘)
        // 4. if filter { filter.process each sample }
        // 5. apply pan (L/R gains)
        // 6. buffer += temp  (f32x4 批量加)
    }
}
```

### 2. 采样器 (`voice/sampler.rs`)

```rust
use std::sync::Arc;
use xsynth_soundfonts::LoopMode;

pub struct SampleBuffer {
    pub channels: Arc<[Arc<[f32]>]>,  // [left, right] 或 [mono]
}

#[derive(Clone)]
pub struct LoopParams {
    pub mode: LoopMode,
    pub start: u32,
    pub end: u32,
    pub offset: u32,
    pub stop: Option<u32>,
}

pub enum Interpolation { Nearest, Linear }

pub struct Sampler {
    buffer: SampleBuffer,
    loop_params: LoopParams,
    interpolation: Interpolation,
    position: f64,
    speed_mult: f32,
    pitch_multiplier: f32,
    releasing: bool,
    ended: bool,
}

impl Sampler {
    pub fn render_block(&mut self, out: &mut [f32]);  // f32x4 批量插值
    pub fn signal_release(&mut self);
}
```

### 3. 包络 (`voice/envelope.rs`)

```rust
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EnvelopeStage {
    Delay = 0, Attack = 1, Hold = 2, Decay = 3,
    Sustain = 4, Release = 5, Finished = 6,
}

#[derive(Debug, Clone, Copy)]
pub enum EnvelopePart {
    Lerp { target: f32, duration: u32 },
    LerpConcave { target: f32, duration: u32 },  // (1-f)^8
    LerpConvex { target: f32, duration: u32 },   // f^8
    Hold(f32),
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct EnvelopeDescriptor {
    pub start_percent: f32,
    pub delay: f32, pub attack: f32, pub hold: f32,
    pub decay: f32, pub sustain_percent: f32, pub release: f32,
}

impl EnvelopeDescriptor {
    pub fn to_params(self, sample_rate: u32, options: EnvelopeOptions) -> EnvelopeParameters;
}

pub struct Envelope {
    params: EnvelopeParameters,
    stage: EnvelopeStage,
    stage_position: u32,
    current_value: f32,
    releasing: bool,
    killed: bool,
    allow_release: bool,
}

impl Envelope {
    pub fn render_block(&mut self, out: &mut [f32]);
    pub fn signal_release(&mut self);
    pub fn signal_kill(&mut self);
    pub fn ended(&self) -> bool;
}
```

### 4. per-voice 滤波器 (`voice/filter.rs`)

```rust
use biquad::*;
use xsynth_soundfonts::FilterType;

pub struct BiQuadFilter { filter: DirectForm1<f32> }

impl BiQuadFilter {
    pub fn new(filter_type: FilterType, freq: f32, sample_rate: f32, q: Option<f32>) -> Self;
    #[inline(always)]
    pub fn process(&mut self, input: f32) -> f32;
}
```

### 5. Soundfont 层 (`soundfont.rs` + `soundfont/`)

```rust
// soundfont.rs
pub trait VoiceSpawner: Send + Sync {
    fn spawn_voice(&self, control: &VoiceControlData) -> Box<dyn Voice>;
    fn exclusive_class(&self) -> Option<u8> { None }
}

pub trait SoundfontSource: Send + Sync + std::fmt::Debug {
    fn get_attack_voice_spawners_at(
        &self, bank: u8, preset: u8, key: u8, vel: u8,
    ) -> Vec<Box<dyn VoiceSpawner>>;
    fn get_release_voice_spawners_at(
        &self, bank: u8, preset: u8, key: u8, vel: u8,
    ) -> Vec<Box<dyn VoiceSpawner>>;
}
```

```rust
// soundfont/spawner.rs
pub struct VoiceSpawnerParams {
    pub volume: f32, pub pan: f32, pub speed_mult: f32,
    pub cutoff: Option<f32>, pub resonance: f32, pub filter_type: FilterType,
    pub loop_params: LoopParams, pub envelope: Arc<EnvelopeParameters>,
    pub sample: Arc<[Arc<[f32]>]>, pub interpolator: Interpolation,
    pub exclusive_class: Option<u8>,
}

pub struct SoundfontInstrument {
    pub bank: u8, pub preset: u8,
    spawner_params: Vec<Vec<Arc<VoiceSpawnerParams>>>,  // 16384 slots (128*128)
}

pub struct SampleSoundfont {
    instruments: Vec<SoundfontInstrument>,
    stream_params: AudioStreamParams,
}
```

```rust
// soundfont/loader.rs — 唯一接触 xsynth-soundfonts 的地方
use xsynth_soundfonts::sf2::{load_soundfont, Sf2Preset, Sf2Region};
use xsynth_soundfonts::sfz;

pub fn load_sf2(path: &Path, stream_params: AudioStreamParams,
                options: SoundfontInitOptions) -> Result<SampleSoundfont, Sf2ParseError> {
    // 1. load_soundfont(path, sample_rate) -> Vec<Sf2Preset>
    // 2. 遍历 preset -> region -> key/vel
    // 3. 对每个 (key, vel) 计算 speed_mult, envelope, cutoff, pan, volume, loop_params
    // 4. 构建 VoiceSpawnerParams, 存入 spawner_params[key * 128 + vel]
}

pub fn load_sfz(path: &Path, stream_params: AudioStreamParams,
                options: SoundfontInitOptions) -> Result<SampleSoundfont, LoadSfzError> {
    // 同理，用 xsynth_soundfonts::sfz::parse_soundfont
}
```

```rust
// soundfont/config.rs
pub enum Interpolation { Nearest, Linear }
pub enum EnvelopeCurveType { Linear, Exponential }
pub struct EnvelopeOptions {
    pub attack_curve: EnvelopeCurveType,   // 默认 Exponential
    pub decay_curve: EnvelopeCurveType,    // 默认 Linear
    pub release_curve: EnvelopeCurveType,  // 默认 Linear
}
pub struct SoundfontInitOptions {
    pub bank: Option<u8>,
    pub preset: Option<u8>,
    pub vol_envelope_options: EnvelopeOptions,
    pub use_effects: bool,
    pub interpolator: Interpolation,
}
```

### 6. Channel 层 (`channel.rs` + `channel/`)

```rust
// channel.rs
pub struct VoiceChannel {
    key_voices: Vec<Key>,              // 128 个 key
    params: VoiceChannelParams,        // bank/preset/layer_count/soundfonts
    threadpool: Option<Arc<rayon::ThreadPool>>,
    stream_params: AudioStreamParams,
    control_event_data: ControlEventData,
    voice_control_data: VoiceControlData,
    cutoff_filter: MultiChannelBiQuad,  // 通道级 CC74 cutoff
}

impl VoiceChannel {
    pub fn new(options, stream_params, threadpool) -> Self;
    pub fn push_events_iter(&mut self, iter: impl Iterator<Item = ChannelEvent>);
    /// 渲染：遍历 128 key -> 每个 key 渲染所有 voice -> 累加 -> 通道效果
    pub fn read_samples(&mut self, out: &mut [f32]);
}
```

```rust
// channel/control.rs — CC/RPN/PitchBend 状态机
pub struct ControlEventData {
    selected_lsb: i8, selected_msb: i8,
    pitch_bend_sensitivity: f32, pitch_bend_value: f32,
    fine_tune_value: f32, coarse_tune_value: f32,
    pub volume: ValueLerp,    // CC7, 0.0-1.0
    pub pan: ValueLerp,       // CC10
    pub expression: ValueLerp, // CC11
    pub cutoff: Option<f32>,  // CC74
    pub resonance: Option<f32>, // CC71
}

// CC 处理（与 xsynth 一致）:
// CC0=Bank, CC6/26/100/101=RPN, CC7=Volume, CC8/10=Pan,
// CC11=Expression, CC64=Damper, CC71=Resonance, CC72=Release,
// CC73=Attack, CC74=Cutoff, CC120=AllSoundsOff,
// CC121=ResetAllControllers, CC123=AllNotesOff
```

```rust
// channel/key.rs — per-key voice 管理
pub struct KeyData {
    key: u8,
    voices: VoiceBuffer,
    shared_voice_counter: Arc<AtomicU64>,
}

impl KeyData {
    pub fn send_event(&mut self, event: KeyNoteEvent,
                      control: &VoiceControlData,
                      channel_sf: &ChannelSoundfont,
                      max_layers: Option<usize>);
    pub fn render_to(&mut self, out: &mut [f32]);
}
```

```rust
// channel/voice_buffer.rs — voice 列表 + voice stealing + damper
pub struct VoiceBuffer {
    buffer: VecDeque<GroupVoice>,
    damper_held: bool,
    held_by_damper: Vec<usize>,
    options: ChannelInitOptions,
}

impl VoiceBuffer {
    pub fn push_voices(&mut self, voices, max_voices: Option<usize>);
    pub fn release_next_voice(&mut self) -> Option<u8>;
    pub fn remove_ended_voices(&mut self);
    pub fn kill_all_voices(&mut self);
    pub fn kill_by_exclusive_class(&mut self, class: u8);
    pub fn set_damper(&mut self, damper: bool);
    pub fn iter_voices_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Voice>>;
}
```

### 7. ChannelGroup (`channel_group.rs` + `channel_group/config.rs`)

```rust
// channel_group.rs
pub struct ChannelGroup {
    thread_pool: Option<rayon::ThreadPool>,
    channels: Box<[VoiceChannel]>,
    channel_events_cache: Box<[Vec<ChannelAudioEvent>]>,
    sample_cache_vecs: Box<[Vec<f32>]>,
    audio_params: AudioStreamParams,
}

impl ChannelGroup {
    pub fn new(config: ChannelGroupConfig) -> Self;
    pub fn send_event(&mut self, event: SynthEvent);
    /// 渲染：flush events -> rayon 并行渲染各 channel -> sum_simd 混合
    pub fn read_samples(&mut self, out: &mut [f32]);
    pub fn voice_count(&self) -> u64;
}
```

```rust
// channel_group/config.rs
pub enum SynthFormat { Midi, Custom { channels: u32 } }
pub enum ThreadCount { None, Auto, Manual(usize) }

pub struct ParallelismOptions {
    pub channel: ThreadCount,  // 通道间并行
    pub key: ThreadCount,      // key 间并行
}

pub struct ChannelGroupConfig {
    pub channel_init_options: ChannelInitOptions,
    pub format: SynthFormat,
    pub audio_params: AudioStreamParams,
    pub parallelism: ParallelismOptions,
}
```

### 8. 事件类型 (`events.rs`)

```rust
pub enum ChannelAudioEvent {
    NoteOn { key: u8, vel: u8 },
    NoteOff { key: u8 },
    AllNotesOff,
    AllNotesKilled,
    ResetControl,
    Control(ControlEvent),
    ProgramChange(u8),
    SystemReset,
}

pub enum ControlEvent {
    Raw(u8, u8),                // CC number + value
    PitchBend(f32),             // 合并后的半音数
    PitchBendValue(f32),        // -1.0 to 1.0
    PitchBendSensitivity(f32),  // 半音数
    FineTune(f32),              // cents
    CoarseTune(f32),            // 半音
}

pub enum ChannelConfigEvent {
    SetSoundfonts(Vec<Arc<dyn SoundfontSource>>),
    SetLayerCount(Option<usize>),
    SetPercussionMode(bool),
}

pub enum ChannelEvent {
    Audio(ChannelAudioEvent),
    Config(ChannelConfigEvent),
}

pub enum SynthEvent {
    Channel(u32, ChannelEvent),
    AllChannels(ChannelEvent),
}
```

## 与 yinhe-audio 的集成

### 替换前（当前）

```rust
// yinhe-audio/src/engine.rs
use xsynth_core::channel_group::{ChannelGroup, ChannelGroupConfig, SynthEvent, ...};
use xsynth_core::soundfont::{SampleSoundfont, SoundfontBase, SoundfontInitOptions};

struct AudioEngine {
    channel_group: ChannelGroup,       // xsynth
    sf_manager: SoundFontManager,       // 包装 xsynth 的 SampleSoundfont
}
```

### 替换后

```rust
// yinhe-audio/src/engine.rs
use yinhe_synth::channel_group::{ChannelGroup, ChannelGroupConfig, SynthEvent, ...};
use yinhe_synth::soundfont::{SampleSoundfont, SoundfontSource, SoundfontInitOptions};

struct AudioEngine {
    channel_group: ChannelGroup,       // yinhe-synth（API 完全一致）
    sf_manager: SoundFontManager,       // 改为加载 yinhe-synth 的 SampleSoundfont
}
```

**改动量**：`yinhe-audio` 只需修改 `use` 语句，将 `xsynth_core::` 替换为 `yinhe_synth::`。
事件类型（`ChannelAudioEvent` / `ControlEvent` / `SynthEvent`）的 API 保持一致。
`ChannelState`（chase 用的 CC 快照）不需要改动。

## 依赖关系

```toml
# yinhe-synth/Cargo.toml
[dependencies]
xsynth-soundfonts = "0.4"   # 仅用于 SF2/SFZ 解析
biquad = "0.4"               # 滤波器
rayon = "1"                  # 并行渲染
thiserror = "2"
atomic_refcell = "0.1"       # 音频线程安全引用

# yinhe-audio/Cargo.toml（替换后）
[dependencies]
# xsynth-core = "0.4"         # <- 删除
# xsynth-soundfonts = "0.4"   # <- 删除
yinhe-synth = { path = "../yinhe-synth" }  # <- 新增
cpal = "0.17"
hound = "3.5"
```

**结果**：`yinhe-audio` 不再直接依赖任何 xsynth crate。
`yinhe-synth` 只依赖 `xsynth-soundfonts`（用于解析），不依赖 `xsynth-core`。

## 实现阶段

### Phase 1: 标量版引擎 — 听到声音（5-7 天）

目标：用标量代码跑通 SF2 播放，不追求性能。

- [ ] `voice/envelope.rs` — DADSR 包络 + concave/convex/linear 曲线
- [ ] `voice/sampler.rs` — 线性插值 + NoLoop/LoopContinuous/LoopSustain
- [ ] `voice/filter.rs` — biquad 滤波器（直接用 biquad crate）
- [ ] `voice.rs` — SampledVoice 组装 + Voice trait 实现
- [ ] `soundfont/spawner.rs` — VoiceSpawnerParams + StereoSampledVoiceSpawner
- [ ] `soundfont/loader.rs` — SF2 加载（复用 xsynth-soundfonts 解析）
- [ ] `channel/voice_buffer.rs` — voice 列表 + voice stealing + damper
- [ ] `channel/key.rs` — KeyData
- [ ] `channel/control.rs` — CC 处理（从 xsynth 对照移植）
- [ ] `channel.rs` — VoiceChannel
- [ ] `channel_group.rs` — ChannelGroup + rayon 并行
- [ ] `events.rs` — 事件类型
- [ ] 集成到 yinhe-audio，替换 xsynth-core

**验证**：A/B 对比测试，同一段 MIDI 渲染后逐 sample 比较输出。

### Phase 2: SIMD 优化（2-3 天）

- [ ] `voice/sampler.rs` — render_block 加 f32x4 批量插值
- [ ] `voice/envelope.rs` — render_block 加 f32x4 批量计算
- [ ] `voice.rs` — SampledVoice::render_to 的 "采样 x 包络" 和 "累加" 加 f32x4
- [ ] helpers — sum_simd 用 f32x4
- [ ] 性能基准测试：对比 xsynth 的 samples_per_second

### Phase 3: SFZ 支持 + 完整 CC（3-4 天）

- [ ] `soundfont/loader.rs` — SFZ 加载
- [ ] `channel/control.rs` — 完整 CC（CC71/72/73/74）
- [ ] RPN 完整实现（pitch bend sensitivity, fine tune, coarse tune）
- [ ] 通道效果完善（volume x expression 平滑、pan 等功率、cutoff 滤波）

### Phase 4: 优化与打磨（2-3 天）

- [ ] voice fade out killing（1ms 淡出）
- [ ] exclusive class 处理
- [ ] OneShot 模式（不允许 release）
- [ ] 释放音（release voice spawners）
- [ ] 内存优化（Arc 共享、预分配缓冲区）

## 对比测试策略

```rust
// tests/parity_test.rs
#[test]
fn parity_with_xsynth() {
    let midi = "test_assets/black.mid";
    let sf2 = "test_assets/GeneralUser.sf2";

    let xsynth_output = render_with_xsynth(midi, sf2, 44100);
    let yinhe_output = render_with_yinhe_synth(midi, sf2, 44100);

    for (i, (a, b)) in xsynth_output.iter().zip(yinhe_output.iter()).enumerate() {
        let diff = (a - b).abs();
        assert!(diff < 0.01, "Sample {} differs: {} vs {}", i, a, b);
    }
}
```

## 行数预估

| 模块 | 行数 |
|------|------|
| voice.rs (trait + SampledVoice) | ~200 |
| voice/envelope.rs | ~400 |
| voice/sampler.rs | ~350 |
| voice/filter.rs | ~80 |
| soundfont.rs (trait) | ~60 |
| soundfont/spawner.rs | ~300 |
| soundfont/loader.rs | ~400 |
| soundfont/config.rs | ~120 |
| channel.rs | ~350 |
| channel/control.rs | ~300 |
| channel/key.rs | ~100 |
| channel/voice_buffer.rs | ~250 |
| channel_group.rs | ~250 |
| channel_group/config.rs | ~110 |
| events.rs | ~100 |
| lib.rs + helpers | ~100 |
| **合计** | **~3470** |

## 关键设计决策

| 决策 | 方案 | 理由 |
|------|------|------|
| SIMD 方案 | nightly + std::simd | Apple Silicon 零性能损失，AI 生成准确率高 |
| SF2/SFZ 解析 | 复用 xsynth-soundfonts | 不重复造轮子，解析层无 LGPL 运行时约束 |
| 模块组织 | foo.rs + foo/ 子目录 | 现代写法，不使用 mod.rs |
| 包络曲线 | concave=(1-f)^8, convex=f^8 | 与 SF2 规范一致，与 xsynth 完全对等 |
| 渲染策略 | block-based，标量优先 | 先正确再优化，SIMD 只加在最热点 |
| voice stealing | 按 velocity 优先杀最低 | 与 xsynth 一致 |
| damper | VecDeque + held_by_damper | 与 xsynth 一致 |
| 并行 | rayon par_iter per channel | 与 xsynth 一致 |
