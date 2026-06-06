# XSynth 集成 Spec v2

## 目标

在 `yinhe-audio` crate 中集成 `xsynth-core` + `xsynth-soundfonts`，实现：
1. 实时播放（cpal）
2. 离线导出（WAV）
3. 播放与导出使用完全相同的渲染路径
4. 为 VST/CLAP/AU 效果器插件链预留架构

## 架构总览

```
┌─────────────────────────────────────────────────────────────────────┐
│                           AudioEngine                               │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │  ChannelGroup (SynthFormat::Custom { channels: 256 })         │  │
│  │                                                               │  │
│  │  Port A (ch 0-15)  │  Port B (ch 16-31)  │  ...  │ Port P    │  │
│  │  SF: Piano          │  SF: Strings         │       │ (unused)  │  │
│  │  ch9 = percussion   │  ch9 = percussion    │       │           │  │
│  └───────────────────────────────────────────────────────────────┘  │
│       │                                                              │
│       ▼ read_samples()                                               │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │  Limiter (Lookahead Brickwall, -1dBFS threshold)              │  │
│  └───────────────────────────────────────────────────────────────┘  │
│       │                                                              │
│  ┌────┴────────────────────────────────────────────────────────┐    │
│  │              AudioSink (trait)                               │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │    │
│  │  │ CpalSink    │  │ WavSink     │  │ EffectSink  │         │    │
│  │  │ (实时播放)   │  │ (离线导出)   │  │ (future)    │         │    │
│  │  └─────────────┘  └─────────────┘  └─────────────┘         │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │  MidiEventScheduler                                          │    │
│  │  MidiFile → Vec<ScheduledEvent> (按 sample 排序)             │    │
│  │  + Chase Index (seek 时恢复 CC/PC/PB 状态)                   │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │  SoundFontManager                                             │    │
│  │  Arc 缓存 + per-port 映射 + 全局/歌曲两级配置                  │    │
│  └─────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
```

## 核心设计

### 1. Port-Channel 模型

采用 **单 ChannelGroup 管理全部 256 通道**（参考 Yueliang 架构）：

```rust
let config = ChannelGroupConfig {
    channel_init_options: ChannelInitOptions { fade_out_killing: true },
    format: SynthFormat::Custom { channels: 256 },
    audio_params: AudioStreamParams {
        sample_rate,
        channels: ChannelCount::Stereo,
    },
    parallelism: ParallelismOptions::AUTO_PER_CHANNEL,
};
let channel_group = ChannelGroup::new(config);
```

**编码映射**（与现有 Note.channel 一致）：
```
global_channel = port * 16 + midi_channel   (u8, 0-255)
```

**为什么不采用多 ChannelGroup？**
- XSynth 的 ChannelGroup 内部已处理 256 通道的并行渲染和混合
- 单 ChannelGroup 只需一次 `read_samples()` 调用，简化管线
- Yueliang 已验证此架构在百万级音符下的稳定性
- 未来需要 per-track 效果器时，可重构为 per-port ChannelGroup（迁移路径清晰）

**⚠️ 需修复的 bug**：`yinhe-midi/src/midi.rs` 的 `track_info()` 中 port 解码使用 `& 0x07`（限制 8 port），应改为 `& 0x0F`（支持 16 port）。

### 2. 打击乐模式

参考 Yueliang 的自动推断逻辑：

```rust
fn setup_percussion(channel_group: &mut ChannelGroup, midi: &MidiFile) {
    // 1. 默认：每个 Port 的 Channel 10（index 9）设为打击乐
    for port in 0..16 {
        let ch = (port * 16 + 9) as u32;
        channel_group.send_event(SynthEvent::Channel(
            ch,
            ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(true)),
        ));
    }

    // 2. 扫描 CC#0 (Bank Select MSB) 进行覆盖
    //    CC0=120 → XG drum → 设为鼓
    //    CC0=121 → XG melody → 设为乐器（覆盖默认的 ch9）
    for evt in &midi.control_events {
        if let MidiControlEvent::ControlChange { channel, controller: 0, value, .. } = evt {
            let is_drum = *value >= 120;
            channel_group.send_event(SynthEvent::Channel(
                *channel as u32,
                ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(is_drum)),
            ));
        }
    }
}
```

### 3. AudioEngine

```rust
pub struct AudioEngine {
    channel_group: ChannelGroup,
    scheduler: MidiEventScheduler,
    sf_manager: SoundFontManager,
    limiter: Option<VolumeLimiter>,
    sample_rate: u32,
    sample_position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    interleaved_buffer: Vec<f32>,  // 预分配交错缓冲区
    duration_samples: u64,
}
```

**关键设计决策**：
- `sample_position` 和 `playing` 使用 `Arc<AtomicU64/AtomicBool>`，支持跨线程（cpal 回调 ↔ UI）
- `interleaved_buffer` 在初始化时预分配，避免音频线程堆分配（参考 Yueliang/Taiyang）
- XSynth 输出交错立体声 `[L, R, L, R, ...]`，需手动拆分写入 cpal 的 deinterleaved buffer

### 4. MidiEventScheduler + Chase

```rust
pub struct MidiEventScheduler {
    events: Vec<ScheduledEvent>,
    cursor: usize,
    chase: ChaseEngine,
}

struct ScheduledEvent {
    sample: u64,
    tick: u64,                   // 保留 tick 用于 Chase 定位
    channel: u32,                // global_channel (port*16+ch)
    event: ChannelAudioEvent,
}
```

**事件构建**：
- NoteOn → `(start_sample, start_tick, ch, NoteOn { key, vel })`
- NoteOff → `(end_sample, end_tick, ch, NoteOff { key })`
- CC → `(sample, tick, ch, Control(ControlEvent::Raw(cc, val)))`
- PitchBend → `(sample, tick, ch, Control(ControlEvent::PitchBendValue(normalized)))`
- ProgramChange → `(sample, tick, ch, ProgramChange(pc))`
- 转换公式：`sample = tick_to_seconds(tick) * sample_rate as f64`

**Chase 机制（Yinhe 优化版）**：

Yueliang 的 Chase 是 seek 时线性扫描全部事件 O(n)，500 万事件约 10ms。
Yinhe 在加载时预计算**检查点快照**，seek 变为 O(log n + k)：

```rust
pub struct ChaseEngine {
    /// 检查点快照，按 tick 升序，间隔约 1000 tick
    checkpoints: Vec<ChaseCheckpoint>,
}

struct ChaseCheckpoint {
    tick: u64,
    /// 每个 channel 的控制器状态（256 个 channel，仅活跃的有值）
    channels: [ChannelState; 256],
}

#[derive(Clone, Default)]
struct ChannelState {
    bank_msb: u8,
    bank_lsb: u8,
    program: u8,
    volume: u8,         // CC7, 默认 100
    pan: u8,            // CC10, 默认 64
    expression: u8,     // CC11, 默认 127
    sustain: u8,        // CC64, 默认 0
    cutoff: u8,         // CC74, 默认 64
    resonance: u8,      // CC71, 默认 64
    attack: u8,         // CC73, 默认 64
    release: u8,        // CC72, 默认 64
    pitch_bend: f32,    // 默认 0.0
    rpn_msb: u8,        // CC101
    rpn_lsb: u8,        // CC100
    data_entry_msb: u8, // CC6
    data_entry_lsb: u8, // CC38
}
```

**加载时构建**：
1. 遍历已排序的 ScheduledEvent 列表
2. 每 ~1000 tick 创建一个 ChaseCheckpoint，记录当前全部 256 channel 的状态
3. 遇到 CC/PC/PB 事件时更新对应 channel 的状态

**Seek 时恢复**：
1. 二分查找 checkpoints，找到 target_tick 之前的最近检查点
2. 从检查点开始线性扫描 events，更新 channel 状态直到 target_tick
3. 将最终状态注入为 Chase 事件（在 target_tick 位置的 NoteOn 之前发送）

**Chase 注入顺序**（RPN 必须先于 Data Entry）：
```
CC101(RPN MSB) → CC100(RPN LSB) → CC6(DataEntry MSB) → CC38(DataEntry LSB)
→ CC0(Bank MSB) → CC32(Bank LSB) → CC7(Volume) → CC10(Pan)
→ CC11(Expression) → CC64(Sustain) → CC73(Attack) → CC72(Release)
→ CC74(Cutoff) → CC71(Resonance) → ProgramChange → PitchBend
```

### 5. SoundFont 管理

参考 Yueliang 的 Arc 缓存 + per-port 加载模式：

```rust
pub struct SoundFontManager {
    /// 全局缓存：路径 → Arc（同一文件只加载一次）
    cache: HashMap<PathBuf, Arc<dyn SoundfontBase>>,
    /// Per-port 音色库配置
    port_configs: [PortSoundFontConfig; 16],
    /// 全局默认配置
    global_config: Option<SoundFontPreset>,
}

pub struct PortSoundFontConfig {
    enabled: bool,
    entries: Vec<SoundFontEntry>,  // 该 port 的音色库列表（支持叠加）
}

pub struct SoundFontEntry {
    pub path: PathBuf,
    pub enabled: bool,
    pub sha256: Option<String>,  // 文件校验，检测变更
}

pub struct SoundFontPreset {
    pub path: PathBuf,
    pub format: SoundFontFormat,  // SF2 / SFZ
}
```

**加载流程**：
1. 检查 cache 是否已有该路径的 Arc → 命中则 clone
2. 未命中 → `SampleSoundfont::new(path, stream_params, default_options)` → `Arc::new(sf)`
3. 写入 cache
4. 将 `Vec<Arc<dyn SoundfontBase>>` 通过 `SetSoundfonts` 发送到该 port 的 16 个 channel

**ProgramChange 处理**：
- MIDI 文件中的 ProgramChange 事件自动切换 bank/preset
- XSynth 的 `ChannelSoundfont` 根据 bank/preset 从 VoiceSpawnerMatrix 查找 voice spawner
- 支持自动（从 MIDI 事件）和手动（UI 选择）两种模式

**配置存储**：
- 全局默认：存入 app config（JSON）
- 歌曲独立：存入工程文件（JSON，当前阶段）

### 6. 渲染管线

```rust
impl AudioEngine {
    fn read_samples(&mut self, output: &mut [f32]) {
        let frames = output.len() / 2;  // stereo
        let start = self.sample_position.load(Ordering::Relaxed);
        let end = start + frames as u64;

        // 1. 推送当前 buffer 范围内的 MIDI 事件
        self.scheduler.push_events(start, end, &mut self.channel_group);

        // 2. 渲染到交错缓冲区
        let interleaved = &mut self.interleaved_buffer[..frames * 2];
        self.channel_group.read_samples(interleaved);

        // 3. 限幅
        if let Some(ref mut limiter) = self.limiter {
            limiter.limit(interleaved);
        }

        // 4. 交错 → deinterleaved（cpal 需要）
        for i in 0..frames {
            output[i] = interleaved[i * 2];       // L
            output[i + frames] = interleaved[i * 2 + 1]; // R
        }

        // 5. 推进位置
        self.sample_position.store(end, Ordering::Relaxed);
    }
}
```

### 7. 暂停行为

支持多种模式（当前先实现默认，设置页面后续做）：

```rust
pub enum PauseBehavior {
    /// 停在当前位置（DAW 标准，如 Ableton/Logic）
    StopAtCurrentPosition,
    /// 回到播放起始点（当前 Yinhe 行为）
    ReturnToStartPoint,
}
```

**Phase 1 默认**：`StopAtCurrentPosition`（DAW 标准）
- Space 暂停 → 停在当前位置
- Escape 停止 → 回到开头
- 播放到末尾 → 回到播放起始点

### 8. 导出流程

```rust
pub fn export_wav(engine: &mut AudioEngine, path: &Path) -> Result<()> {
    let spec = WavSpec {
        channels: 2,  // stereo
        sample_rate: engine.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    let total = engine.duration_samples;
    let buf_size = engine.sample_rate as usize;  // 1 秒 buffer
    let mut buf = vec![0.0f32; buf_size * 2];    // stereo

    engine.reset();
    while engine.sample_position() < total {
        let remaining = total - engine.sample_position();
        let frames = (buf_size).min(remaining as usize);
        engine.read_samples(&mut buf[..frames * 2]);
        for &s in &buf[..frames * 2] {
            writer.write_sample((s * 32767.0) as i16)?;
        }
    }
    writer.finalize()?;
    Ok(())
}
```

### 9. 限幅器

参考 Nezha 的 Lookahead Brickwall Limiter，但 **Nezha 的 limiter 可能有问题，需谨慎实现**：

```rust
pub struct Limiter {
    lookahead_samples: usize,    // 2ms @ sample_rate
    threshold_db: f32,           // -1.0 dBFS
    ceiling_db: f32,             // -0.3 dBFS
    attack_ms: f32,              // 0.5 ms
    release_ms: f32,             // 100 ms
    buffer: Vec<f32>,            // lookahead 延迟缓冲
}
```

**Phase 1**：不实现 limiter，避免引入 bug
**Phase 3（导出）**：实现并充分测试后启用
**Phase 4（播放）**：可选启用

### 10. 插件链架构（Ableton 风格，远期）

```
Track n:
  [XSynth Instrument] → [Effect 1] → [Effect 2] → ... → Track Output
                                                         ↓
Master:                                             [Mix All Tracks]
  [Master Effect 1] → [Master Effect 2] → ... → Output
```

**预留接口**：
```rust
pub trait AudioProcessor: Send {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32);
    fn reset(&mut self);
    fn name(&self) -> &str;
}
```

**迁移路径**：当需要 per-track 效果器时，将单 ChannelGroup 重构为 per-port ChannelGroup，每个 port 的输出独立送入 effect chain。

## Crate 结构

```
yinhe-audio/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── engine.rs          # AudioEngine 核心
    ├── scheduler.rs       # MidiEventScheduler + ChaseIndex
    ├── soundfont.rs       # SoundFontManager (Arc 缓存 + per-port)
    ├── limiter.rs         # Lookahead Brickwall Limiter
    └── sink/
        ├── mod.rs          # AudioSink trait
        ├── cpal.rs         # CpalSink (实时播放)
        └── wav.rs          # WavSink (离线导出)
```

## 依赖

```toml
[dependencies]
xsynth-core = "0.4"
xsynth-soundfonts = "0.4"
cpal = "0.17"
hound = "3.5"

yinhe-types = { path = "../yinhe-types" }
yinhe-midi = { path = "../yinhe-midi" }
```

## 已确认的决策

| 决策 | 方案 | 来源 |
|------|------|------|
| Channel 数量 | 单 ChannelGroup，256 通道 | Yueliang 架构 |
| SoundFont 共享 | Arc 缓存，同路径只加载一次 | Yueliang/Taiyang |
| SoundFont 加载 | Per-port，支持叠加多个 SF | 用户确认（必备） |
| SoundFont 校验 | SHA256 检测文件变更 | 用户确认 |
| 打击乐模式 | 默认 ch9/port，CC#0 自动推断 | Yueliang |
| 暂停行为 | 停在当前位置（DAW 标准） | 用户确认 |
| Voice 层限制 | 默认 32（单键最大同时发声数） | Nezha |
| 导出格式 | WAV（hound） | 用户确认 |
| 限幅器 | Phase 1 不实现，Phase 3 谨慎实现 | 用户确认（Nezha 有问题） |
| 工程文件 | JSON（当前），未来可迁移 LMPJ | 用户确认 |
| Chase 机制 | Yinhe 内检查点快照 O(log n + k) | 用户确认（优于 Yueliang O(n)） |

## 实现阶段

### Phase 1: 基础播放 + Chase ⬅️ 当前
- [ ] yinhe-audio crate 创建
- [ ] AudioEngine 初始化（ChannelGroup 256ch）
- [ ] MidiEventScheduler 构建（notes + control_events → ScheduledEvent）
- [ ] ChaseEngine（检查点快照构建 + seek 恢复）
- [ ] 打击乐模式设置（默认 ch9 + CC#0 推断）
- [ ] SoundFontManager（Arc 缓存 + per-port 叠加 + SHA256 校验）
- [ ] CpalSink 实现
- [ ] 与现有 PlaybackState 对接（改用音频时钟）
- [ ] 暂停行为改为停在当前位置
- [ ] 基础播放/暂停/停止/seek

### Phase 2: 导出
- [ ] WavSink 实现
- [ ] 导出对话框 UI
- [ ] 进度显示

### Phase 3: 播放增强
- [ ] Limiter（谨慎实现，充分测试）
- [ ] Per-port 音量/声像控制
- [ ] 暂停行为配置（设置页面）

### Phase 4: 编辑响应（远期）
- [ ] 画音符时触发 NoteOn
- [ ] 删除音符时触发 NoteOff
- [ ] 实时 CC 调整

### Phase 5: 插件链（远期）
- [ ] AudioProcessor trait
- [ ] 重构为 per-port ChannelGroup
- [ ] clack-host CLAP 支持
- [ ] VST3 支持
- [ ] AU 支持

## 从相关项目可复用的代码/模式

| 来源 | 可复用内容 |
|------|-----------|
| **Yueliang** | `SynthFormat::Custom { channels: 256 }` 初始化、Arc sf_cache、per-port SF 加载、CC#0 打击乐推断、Chase CC 列表和注入顺序、交错缓冲区预分配、rfd 文件对话框异步方案 |
| **Taiyang** | 全局 `LazyLock<RwLock<HashMap<(String, u32), Arc<...>>>>` SF 缓存（跨实例共享）、脏检查参数同步、Envelope Auto 模式（-1 = 不覆盖） |
| **Nezha** | Channel 9 打击乐自动设置、Lookahead Limiter 参数（需修复后使用）、分块渲染 + 尾音释放流程、Seek Index 设计思路 |
| **Lumino RS** | LMPJ metadata.toml 结构（未来工程文件参考）、CompactEvent 12字节格式参考 |
