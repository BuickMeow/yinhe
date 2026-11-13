# 乐器参数目录与「音色参数」子窗口设计

> 状态：设计稿 + 首批落地（2026-11-13）。回答两个问题：
> (1) SF2 / SFZ 应该开放哪些可调参数；(2) 是否需要独立的参数子窗口、如何分类。
> 对齐基准与实测见 `eval-synth-params-and-xsynth.md`。

---

## 零、原则

1. **只做「单参数旋钮」，不做「一键变亮」这类宏**。用户要逐个试验参数能力，
   宏会把因果藏起来。
2. **参数是相对偏移，不是覆盖库文件**。所有修改存工程（`.yin`），库只读。
3. **每个参数必须显示「库值 → 实际值」**。用户要知道自己改了多少、原来是多少。
4. **参数作用域 = 乐器实例（通道 × 音色库槽位）**。同一工程可以给不同通道
   同一音色库不同的调校。
5. **不做等宽字体、不加边框、不加说明小字**（项目 UI 红线）。帮助信息走 tooltip。
6. 不可实时生效的参数要明确标注「需重载」，不能让用户拧了没反应还以为坏了。

---

## 一、参数目录

### 1.1 状态图例

| 标记 | 含义 |
|---|---|
| ✅ | 引擎已支持，可实时生效（新音符立即生效，发声中的按参数类型决定） |
| 🟡 | 引擎已支持，但只在 note_on 时求值（改完只影响新音符） |
| ⚙ | 需要重新加载音色库/重建 keymap 才生效 |
| ❌ | 引擎尚未实现（UI 可以先隐藏，或显示为禁用） |

### 1.2 SF2 生成器（按用途分组）

#### A. 音量 / 声像

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `initialAttenuation` | 音色基础衰减（cB） | 🟡 |
| `pan` | 声像（-500..500） | 🟡 |
| 默认 modulator：vel→attenuation | 力度曲线 | 🟡（曲线与 BASSMIDI 有差异，见 §3） |

#### B. 滤波器

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `initialFilterFc` | 截止频率（绝对音分） | ✅（CC74 已实时；直接改参数 🟡） |
| `initialFilterQ` | 共振（cB） | ✅（CC71 已实时；直接改参数 🟡） |
| `modEnvToFilterFc` | 调制包络→滤波 | ❌（需要 ModEnv） |
| `modLfoToFilterFc` | LFO→滤波 | ❌ |

#### C. 振幅包络

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `delayVolEnv` / `attackVolEnv` / `holdVolEnv` / `decayVolEnv` / `sustainVolEnv` / `releaseVolEnv` | ADSR | 🟡（CC72/73 可实时改 attack/release） |
| `keynumToVolEnvHold` / `keynumToVolEnvDecay` | 键位对包络时长 | 🟡 |
| 自定义 modulator：vel/key→包络时间 | 力度/键位曲线 | 🟡（xsynth 子集） |

#### D. 音高

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `coarseTune` / `fineTune` | 粗调 / 微调 | 🟡 |
| `scaleTuning` | 键位跟踪比例 | 🟡 |
| `overridingRootKey` | 根音 | 🟡 |
| `keynum` / `velocity`（fixed） | 固定键/力度 | ❌（字段已解析，未消费） |

#### E. 调制包络（ModEnv）

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `delayModEnv` / `attackModEnv` / `holdModEnv` / `decayModEnv` / `sustainModEnv` / `releaseModEnv` | 第二包络 | ❌ |
| `modEnvToPitch` | ModEnv→音高 | ❌ |
| `keynumToModEnvHold/Decay` | 键位跟踪 | ❌ |

#### F. LFO（SF2）

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `delayModLFO` / `freqModLFO` | Mod LFO | ❌ |
| `modLfoToPitch` / `modLfoToVolume` | Mod LFO 目标 | ❌ |
| `delayVibLFO` / `freqVibLFO` | Vib LFO | ❌ |
| `vibLfoToPitch` | 颤音 | ❌（CC1 Mod Wheel 也没有消费方） |

#### G. 效果发送

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `reverbEffectsSend` | 混响发送 | ❌（等 GM2 效果） |
| `chorusEffectsSend` | 合唱发送 | ❌ |

#### H. 采样 / 循环 / 映射

| SF2 生成器 | 作用 | 状态 |
|---|---|---|
| `startAddrsOffset` / `endAddrsOffset`（+coarse） | 采样起止 | ⚙ |
| `startloopAddrsOffset` / `endloopAddrsOffset`（+coarse） | 循环点 | ⚙ |
| `sampleModes` | 循环模式 | ⚙ |
| `keyRange` / `velRange` | 键位/力度区间 | ⚙（keymap 结构） |
| `exclusiveClass` | 鼓组互斥 | ✅（本次已接） |
| `sampleID` / `instrument` | 结构字段，不是旋钮 | — |

### 1.3 SFZ 额外参数（SF2 没有的）

#### A. 力度/键位曲线（SFZ 的强项）

| opcode | 作用 | 状态 |
|---|---|---|
| `amp_veltrack` / `amp_keytrack` / `amp_keycenter` | 力度/键位→音量 | 🟡 |
| `pan_veltrack` / `pan_keytrack` / `pan_keycenter` | 力度/键位→声像 | 🟡 |
| `fil_veltrack` / `fil_keytrack` / `fil_keycenter` | 力度/键位→滤波 | 🟡 |
| `amp_velcurve_N` / `velocity_*` | 力度曲线形状 | ❌（当前是固定二次曲线） |
| `xfin_lovel/hivel` / `xfout_lovel/hivel` | 力度交叉淡化 | ❌ |
| `xfin_lokey/hikey` / `xfout_lokey/hikey` | 键位交叉淡化 | ❌ |

#### B. 包络（SFZ）

| opcode | 作用 | 状态 |
|---|---|---|
| `ampeg_start/delay/attack/hold/decay/sustain/release` | 振幅包络 | ✅/🟡 |
| `ampeg_vel2release` | 力度→释放时长 | 🟡 |
| `ampeg_vel2attack` / `vel2decay` / `vel2delay` / `vel2hold` / `vel2sustain` | 力度→各阶段 | ❌ |
| `ampeg_key2attack` / `key2decay` / `key2release` | 键位→时长 | ❌ |
| `fileg_*` | 滤波包络 | ❌（解析并计数，未实现） |
| `pitcheg_*` | 音高包络 | ❌ |
| `eg01..eg08` | 通用包络 | ❌ |

#### C. LFO（SFZ）

| opcode | 作用 | 状态 |
|---|---|---|
| `amplfo_depth/freq/phase` | 振幅 LFO | ✅ |
| `pitchlfo_depth/freq/phase` | 音高 LFO | 🟡（解析，未施加） |
| `fillfo_depth/freq/phase` | 滤波 LFO | 🟡（解析，未施加） |
| `lfo01..lfo08` + `target` | 通用 LFO 数组 | ❌ |

#### D. 采样播放

| opcode | 作用 | 状态 |
|---|---|---|
| `offset` / `end` | 起止 | ⚙ |
| `loop_start` / `loop_end` / `loop_mode` | 循环 | ⚙ |
| `direction=reverse` | 反向 | ❌ |
| `loop_crossfade` | 循环交叉淡化 | ❌ |
| `count` | 连续触发次数 | ❌ |

#### E. 演奏行为

| opcode | 作用 | 状态 |
|---|---|---|
| `trigger=release` | 释放采样 | ✅ |
| `one_shot` | 忽略 note_off | ✅ |
| `group` / `off_by` | 掐音组 | ✅ |
| `seq_length` / `seq_position` | Round-robin | ✅ |
| `lorand` / `hirand` | 随机选层 | ✅ |
| `pitch_random` | 音高随机 | ✅ |
| `sw_last` / `sw_lokey` / `sw_hikey` | Keyswitch | ✅ |
| `sw_default` / `sw_down` / `sw_up` | Keyswitch 变体 | ❌ |
| `polyphony` / `note_polyphony` | 复音限制 | ❌ |
| `note_off_velocity` | 抬键力度 | ❌（暂缓，BASSMIDI 也不支持） |
| `rt_decay` | 释放衰减 | ❌ |
| `sustain_sw` | 延音开关 | ❌（与 CC64 标准冲突，需先定语义） |

#### F. CC 调制（SFZ 官方"旋钮"机制）

| opcode | 作用 | 状态 |
|---|---|---|
| `xfin_loccN/hiccN` / `xfout_loccN/hiccN` | CC 交叉淡化 | ✅ |
| `on_loccN/hiccN` | CC 区间门 | ✅ |
| `cutoff_ccN` / `volume_ccN` / `pan_ccN` | CC→参数 | ❌ |
| `controller_*` | 参数路由 | ❌ |
| `effect1` / `effect2` | 效果发送 | 🟡（解析，未消费） |

---

## 1.4 首批已落地（2026-11-13）

以下参数已进入工程模型 `SynthParams`（按全局通道存 `mapping.json`），
并通过独立窗口「乐器参数」提供旋钮：

- Brightness（音分偏移，叠加到 voice cutoff）
- Resonance（dB 偏移）
- Volume（dB 偏移）
- Pan（-1..1）
- Attack / Release（倍率，0.1x..10x）
- Tune（音分）

滤波类参数对**正在发声**的 voice 实时生效；音量/声像/包络/音高对之后
的新音符生效。PR 试听（预览合成器）也跟随这些参数。

## 二、子窗口设计

### 2.1 结论

**需要独立子窗口**，理由：

- 参数数量在 30~60 个量级，塞进右栏会把现有轨道属性挤爆；
- 用户要「逐个试参数」，需要同时看到多个参数的值并连续调整；
- 需要 A/B、重置、按分类折叠，这些在窄栏里做不了；
- 以后 `.sfy` 会继续加参数，必须有可扩展容器。

### 2.2 结构

```
┌ 乐器参数 ───────────────────────────────────────────────┐
│ [通道 1 · Studio Grand]        [A/B] [全部重置] [关闭]   │
├──────────────┬──────────────────────────────────────────┤
│ 搜索…        │  滤波器                                   │
│              │   Brightness   [库 0.0 dB]  [====|====]   │
│ ▾ 音量/声像  │   Resonance    [库 0.0 dB]  [====|====]   │
│ ▾ 滤波器     │   Cutoff       [库 8.0 kHz][====|====]   │
│ ▾ 振幅包络   │                                           │
│ ▾ 音高       │  振幅包络                                 │
│ ▾ 调制       │   Attack       [库 10 ms]  [====|====]    │
│ ▾ 采样/循环  │   Decay        [库 1 ms]   [====|====]    │
│ ▾ 键位/力度  │   Sustain      [库 100%]   [====|====]    │
│ ▾ 演奏行为   │   Release      [库 10 ms]  [====|====]    │
│              │                                           │
│              │  ⚙ 采样/循环（需重新加载音色库）           │
│              │   Offset       [库 0]      [====|====]    │
└──────────────┴──────────────────────────────────────────┘
```

交互约定：

- 每行：`名称 | 库值 | 滑块 | 数值输入 | 重置`。滑块双击回库值。
- 顶栏 A/B：临时旁通全部覆盖（对比原始音色）。
- 「需重载」分类整体折叠，展开时标题带 ⚙，不写说明小字（tooltip 说明）。
- 搜索按参数名 + 别名（如 "brightness" 命中 cutoff）。
- 参数很多时，只显示当前音色库**实际用到**的参数（从解析结果统计），
  没出现的分类折叠成一行"未使用"，避免空面板。

### 2.3 数据模型

工程侧新增（`.yin` 版本 +1）：

```rust
struct InstrumentParams {
    channel: u8,          // 或 soundfont slot 标识
    overrides: Vec<ParamOverride>,
}

struct ParamOverride {
    id: InstrumentParamId, // 枚举，不是字符串
    /// 相对偏移（dB / cents / 倍率），或绝对值（采样偏移等）
    value: f32,
}
```

- 不存音色库路径：换库后参数尽量按 id 复用，id 对不上就忽略（不是缓存）。
- 参数 id 是枚举/常量，跨版本稳定；UI 显示名走 i18n。
- 默认值 = 0 偏移；"重置" = 删除该 override，而不是写 0。

### 2.4 生效点

| 参数类型 | 生效方式 |
|---|---|
| Brightness / Resonance | `voice_params::filter_coeffs`（实时） |
| cutoff / resonance | `voice_params::filter_coeffs`（已有实时路径） |
| volume / pan | 通道 DSP（已有）或 voice 增益 |
| attack / release | 已有 `env_frames_for` + CC72/73 传播路径 |
| tune / keytrack / veltrack | `VoiceParams::from_key_info`（note_on） |
| ampeg 其他阶段 | `VoiceParams::from_key_info`（note_on） |
| 采样/循环 | 重载 keymap |

---

## 三、和 BASSMIDI 对齐的现状（2026-11-13 实测）

用 BASSMIDI macOS dylib + 合成 SF2（白噪声/单音）逐档位测量：

| 项 | 状态 |
|---|---|
| CC74 曲线 | ✅ 已逐值对齐（±4800 cents，8 kHz 可听 cap，SF2 范围钳制） |
| CC71 曲线 | ✅ Q=0 预设误差 <0.5 dB；Q=12 dB 预设正方向极端 127 差约 3 dB |
| CC71<64 | ✅ 与 BASSMIDI 一致：不把 Q 压到库值以下 |
| 力度→cutoff | ✅ 已抵消 xsynth 默认调制（BASSMIDI 不应用） |
| exclusiveClass / bank LSB / CC120/121/123 | ✅ |
| 力度→音量曲线 | 🟡 我们的动态范围偏小 3~4 dB，整体响度高约 6~7 dB（库相关） |
| CC7/CC11 曲线 | 🟡 我们用 `(v/127)^2`，BASSMIDI 用 SF2 concave 衰减 |
| CC72/73/75 包络时间 | 🟡 CC72/73 有；CC75（decay）缺失 |
| CC1 Mod Wheel / Vibrato | ❌ |
| CC91/93 Reverb/Chorus | ❌（等 GM2 效果） |
| Aftertouch | ❌（暂缓） |
| Note-off velocity | ✅ 语义一致（BASSMIDI 也不支持） |

**结论**：不引入 BASSMIDI 作为后端。它闭源、授权按商业分发收费、无法接入
我们的混音台/GPU/1 亿音符编辑；只适合作为离线 A/B 参考。继续按上表逐项对齐
yinhe-synth 即可，剩余最大项是力度曲线、CC7/11 曲线、GM2 效果。
