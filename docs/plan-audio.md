# 音频功能规划（Audio Roadmap）

> 目标：把 yinhe 从「能播放 / 录音 / 混音」推进到「对标 Ableton Live 的专业音频制作」。
>
> 约束前提：
> - 支撑 1 亿音符工程的编辑与播放，音频线程预算极其有限；
> - 桌面 + Android / HarmonyOS，渲染走 wgpu，UI 走 egui 0.37；
> - 不引入缓存、不引入阈值式分支、不 vendor 第三方库；
> - 一份能力只保留一份实现，不做 A→B→C 的多余中转。

---

## 0. 结论摘要

1. **现状不是「音频为 0」**。解码、重采样、波形、AudioClip 编辑、录音、mixer（strip/bus/send/insert/PDC/meter）、导出全部已存在；**M0 与 M1 后端已于 2026-11-11 落地**，缺的是 warp marker UI / 独立变调、Clip 包络与自带效果器产品层。
2. **两个真正的洞（M0/M1 后已填上底座）**：
   - `AudioClip` 已增加 beat 主时基（`start_beat` + `WarpMap`），BPM 变化会跟随；
   - 播放取帧已换 4 点 Catmull-Rom，但独立变调（pitch-preserving）仍未做。
3. **Warp 是第一优先级**，它同时是 AudioClip、Simpler、Sampler、Drum Rack 的共同底座。
4. **Sampler 该做，但不是起点**。顺序应为：Warp 引擎 → AudioClip 使用 → Simpler 使用 → Sampler / Drum Rack。
5. **内置音频效果器基本空白**（当前只有 3 个「伪 CC」效果），这是「专业 DAW」体感差距最大的一块。

---

## 1. 现状盘点

| 能力 | 现状 | 代码入口 |
|---|---|---|
| 音频解码 | symphonia，wav / mp3 / flac / ogg / m4a | `yinhe-audio/src/audio_source.rs::decode_audio` |
| 重采样 | rubato，导入 / 录音后统一到引擎采样率 | `audio_source.rs::resample_channel` |
| 波形峰值 | 多级 bucket `WavePeaks` | `audio_source.rs::WavePeaks` |
| AudioClip 模型 | 秒域 `start / offset / duration / gain / fade / reversed` | `yinhe-core/src/model.rs::AudioClip` |
| Clip 编辑 | add / move / trim / split / duplicate / delete / normalize / reverse / fade | `yinhe-editor-core/src/document/audio_clip_edit.rs` |
| 音频轨 UI | 波形、fade 手柄、拖动复制、右键菜单 | `yinhe-egui/src/arrange/view_ui/audio_edit.rs` |
| 录音 | 输入设备选择、监听直通、延迟补偿、WAV 入库并建 Clip | `yinhe-egui/src/app/audio_recording.rs` |
| 混音 | strip(gain/pan/mute/solo)、bus、send(pre/post fader)、insert 链、PDC、meter | `yinhe-mixer/src/graph.rs`、`params.rs` |
| 第三方效果器 | VST3 / CLAP insert | `yinhe-vst3`、`yinhe-clap` |
| 内置效果器 | 仅 ChannelGain / ChannelPan / ChannelFilter，且为伪 CC 实现 | `yinhe-dsp/src/registry.rs::BuiltinEffectKind` |
| 合成 / 采样 | SF2 + SFZ 采样库，CPU + 自研 GPU 双后端 | `yinhe-synth/src/sf_parser.rs` |
| 导出 | 离线渲染 + limiter + 进度 | `yinhe-audio/src/export.rs` |

### 1.1 播放路径关键事实

`yinhe-audio/src/engine_render/render.rs` 的音频分支逐帧执行：

- tempo 剪切（`cut_clip_times`）；
- 线性 fade（`clip_fades` 预计算，`audio_model.rs:854`）；
- reverse（翻转 `src_sec`）；
- 取帧 `let src_frame = (src_sec * sr).floor();`（`render.rs:380`）。

**取帧无插值**，这是后续所有变速 / 变调需求的硬阻塞点。

### 1.2 混音路径关键事实

- 通道命名空间：256 个 MIDI/乐器通道 + 独立音频通道（`channel_layout.rs`）；
- 多条音频轨可共享同一 `audio_channel` → 共享 strip / insert 链；
- send 支持 pre / post fader，bus 可自发送（带回馈延迟块）；
- 已有 PDC（插件延迟补偿）与 per-channel / master meter。

---

## 2. 差距分析

### P0 — 音频轨的专业性底座

| 功能 | 说明 | 可复用资产 |
|---|---|---|
| **Warp / 时间伸缩 + Tempo Follow** | Ableton 的核心。Clip 跟随 BPM，warp marker 分段映射 | 无，需新建 `WarpMap` |
| **高质量重采样** | 4 点 cubic/Hermite 起步，理想 sinc | 替换 `render.rs` 取帧 |
| **变调 / 变速分离** | Re-Pitch（变调即变速）与 Tones（独立变调） | 需 stretch 算法 |
| **Clip 内包络** | Volume / Pan / Transpose 的 clip-local 自动化 | 现有 AM 为 track / device 级 |
| **Fade 曲线 + 自动 crossfade** | 现在只有线性 fade，相邻 Clip 不自动交叉淡化 | 扩展 `effective_fades` |
| **自带音频效果器 6 件套** | Utility、EQ、Compressor、Gate、Limiter、Filter | biquad / limiter / smooth 已有零件 |
| **侧链路由** | Compressor sidechain input | `yinhe-mixer` 的 send/bus 结构接近 |
| **录音 punch / count-in / 循环 take** | 目前是「REC 一路录到底」 | `audio_recording.rs` |

### P1 — 工作流闭环

- Simpler（单样本采样器）
- Drum Rack / Slice
- Take lanes / Comping
- Consolidate / Bounce in place
- Warp modes：Beats / Tones / Texture / Complex / Re-Pitch
- 音频 → MIDI（melody / drums）

### P2 — 差异化

- Spectral / pitch 编辑
- Beat Repeat、音频片段级 Glitch 效果
- 分轨 / stem / 循环区间导出
- 外部时钟同步

### P3 — 暂不碰

- 完整调制矩阵、脚本化设备
- 视频同步

---

## 3. 核心设计

### 3.1 双时基与 WarpMap

当前 `AudioClip` 全部字段是秒；`Clip` / `NoteEvent` 是 tick。这不是 bug，是明确取舍，但也是 warp 无法实现的原因。

规划：

```rust
pub struct AudioClip {
    // 现有字段全部保留（旧工程兼容）
    pub start_seconds: f64,
    pub offset_seconds: f64,
    pub duration_seconds: f64,
    pub gain: f32,
    pub fade_in_seconds: f64,
    pub fade_out_seconds: f64,
    pub reversed: bool,

    /// None = 秒域原样播放（现有行为）；Some = warp 生效，beat 域为主时基。
    #[serde(default)]
    pub warp: Option<WarpMap>,
}

pub struct WarpMap {
    /// 按 beat 升序的 warp marker，首尾必须覆盖 clip 区间。
    pub markers: Vec<WarpMarker>,
}

pub struct WarpMarker {
    /// clip 内 beat 位置（相对 clip 起点）。
    pub beat: f64,
    /// 素材内采样偏移（秒）。
    pub sample_seconds: f64,
}
```

约束：

- marker 之间是**分段线性**映射，查询用二分查找，O(log n)；
- `WarpMap` 是数据结构，不是缓存，符合「不引入缓存」原则；
- 不在 warp 内部做「按算法自动切换」的阈值分支。

**已落地（M1 后端，2026-11-11）**：`yinhe-core/src/audio_warp.rs` 定义
`WarpMap` / `WarpMarker`；`AudioClip` 增加 `start_beat: Option<f64>` 与
`warp: Option<WarpMap>`。自动 warp 为两端点映射（varispeed），
`rebuild_tempo_map` 会按 beat 锚点重算秒域时间；旧工程在
`ProjectData::new` 加载时自动迁移。

### 3.2 AudioClipRenderer

`engine_render/render.rs` 的音频分支已经是一个长循环，warp 进入后会变成第二个上帝函数。拆分：

```rust
struct AudioClipRenderer {
    // 当前 clip 的播放状态
    cursor: Cursor,          // 时间轴位置（秒 / beat 二选一）
    resample_phase: f64,     // 分数相位，供 cubic 取帧
    fade: FadeState,         // 当前 fade 增益
    warp_segment: usize,     // 当前处于哪一段 marker
}
```

职责：

- 由主循环按 block 调度；
- 内部只做「时间映射 → 取帧 → 增益」；
- seek / reverse / warp 变化时通过 `reset(seek_position)` 重建状态；
- CPU / GPU 两条渲染路径共用同一份时间映射逻辑（GPU 只做 voice 合成，音频 clip 仍在 CPU 侧混入）。

**已落地（M0，2026-11-11）**：音频片段渲染拆到
`crates/yinhe-audio/src/engine_render/audio_clip.rs` 的 `AudioClipRenderer`；
`render.rs` 只负责调度。当前按块无状态渲染，seek / 暂停 / 导出天然正确。

### 3.3 重采样质量

第一步（低成本、无新依赖）：

- 最近邻换 4 点 Catmull-Rom / Hermite；
- 相位用 `f64` 累积，避免长时间播放漂移；
- 边界（首尾样本）做 clamp。

**已落地（M0）**：第一步的 4 点 Catmull-Rom 已替换最近邻取帧；
相位按 `f64` 计算并在首尾 clamp。

第二步（warp / 变调时）：

- 固定比率拉伸可用 `rubato`（已在依赖中）；
- 高质量独立变调优先评估 `signalsmith-stretch`（MIT，可 crates.io 引入，不 vendor）；
- **禁止**引入 GPL 的 Rubber Band 作为默认路径。

### 3.4 音频效果器参数模型

`BuiltinEffectKind` 的 `EffectParamInfo` 把参数伪装成 MIDI CC，对 ChannelGain/Pan/Filter 成立，但对 EQ / Compressor / Reverb 会立刻失控。

新增独立模型：

```rust
pub struct AudioEffectParam {
    pub id: u32,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: ParamUnit,
}

pub enum AutomationTarget {
    // ...现有变体
    AudioEffect { strip: StripId, slot: u8, param: u32 },
}
```

要点：

- 音频效果器参数走归一化 0..1，与现有 AM lane 一致；
- 持久化只存 effect id + 参数 id，不存伪 CC；
- 不新建平行的插件 registry：先扩展现有 `BuiltinEffectKind` 的参数模型，
  逐步迁移；对外行为保持兼容。

**架构决策（2026-11-11）**：**不新建 `plugins/` 或 `yinhe-plugin` crate**。
内置效果器继续由 `yinhe-dsp` 概括：按设备拆模块（`reverb.rs` / `delay.rs` /
`eq.rs` / `dynamics.rs`），共用 `delay_line` / `biquad` / `oversample` 等积木；
`BuiltinEffectKind` 作为唯一注册表。UI 在 `yinhe-egui` 做一次通用 device rack
+ 参数面板，设备专属可视化用 `match` 分支，不把 egui 依赖带进 DSP crate。
只有某个设备真的巨大或需要重依赖（如 convolution/FFT）时才单独拆 crate。

### 3.5 Clip 包络与 Fade

- `fade_gain(t, fade_in, fade_out, curve)` 抽成唯一实现；
- UI 画波形、渲染混音、手柄拖动三处共用；**线性版本已落地**：
  `yinhe_core::clip_fade_gain`，渲染与 AR/通道 AR 波形幅度共用；
- 新增 `FadeCurve`（Linear / Exp / Log / SCurve），默认 Linear，不做自动切换；
- 相邻 Clip 的自动 crossfade 由编辑器在拖拽结束时生成显式 fade，而不是渲染期隐式推断。

### 3.6 Sampler / Simpler / Drum Rack

现有 `yinhe-synth` 已是采样器内核（SF2/SFZ region 展开、key/vel 映射、voice 渲染、GPU 调度）。缺的是「素材池 → 乐器」。

**Simpler（最小可用）**

```rust
pub struct SampleInstrument {
    pub source_uuid: String,
    pub root_key: u8,
    pub key_range: (u8, u8),
    pub vel_range: (u8, u8),
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_mode: LoopMode,
    pub gain: f32,
    pub pan: f32,
    pub filter: Option<FilterSettings>,
    pub amp_env: AmpEnv,
    pub warp: Option<WarpMap>,
}
```

- 播放复用现有 `InstrumentProcessor` 与 voice 渲染，只替换采样来源为 `DecodedAudio`；
- UI：素材池拖到乐器槽 → 波形 + loop 手柄 + 少量旋钮；
- 不做多 zone、不做 mod matrix。

**Sampler（完整版）**：多 zone、velocity layer、round robin、LFO、mod matrix。

**Drum Rack**：16 个 Simpler + note 映射，可与「10 号通道视为鼓」的全局设置联动。

---

## 4. 里程碑

| 里程碑 | 内容 | 状态 | 验收标准 |
|---|---|---|---|
| M0 | `fade_gain` 统一 + 取帧换 cubic + `AudioClipRenderer` 拆分 | ✅ 2026-11-11 | 现有音频用例音质不降；音频线程无额外分配 |
| M1 | `WarpMap` + tempo follow + warp marker UI | 🟡 后端完成；marker UI / 独立变调未做 | 改 BPM 后音频位置正确；seek / reverse 组合正确 |
| M2 | Clip 包络 + fade 曲线 + 自动 crossfade | ⏳ | 包络可编辑、可 undo、可持久化 |
| M3 | 音频效果器 6 件套 | ⏳ | 每个效果器有参数 UI、可 AM 自动化、可持久化 |
| M4 | Simpler | ⏳ | 素材拖入即响，loop / root key 生效 |
| M5 | Sampler / Drum Rack | ⏳ | 多 zone 映射正确，GPU 路径 parity |

---

## 5. 测试计划

- **WarpMap**：beat↔sample 映射单调性、边界不越界、marker 乱序拒绝、空 clip 拒绝；
- **Warp + reverse**：反向播放时映射仍单调，首尾样本正确；
- **Fade**：线性 / 指数 / S 曲线端点值为 0 与 1，长度超过 clip 时正确裁剪；
- **Crossfade**：相邻 clip 重叠区增益和恒为 1（等功率或等增益）；
- **Renderer 状态重建**：任意 seek 位置 reset 后输出与连续播放一致（容差内）；
- **Tempo follow（已加）**：240 BPM 下 4-beat clip 缩成 1s，结束后渲染块静音；
- **Warp / reverse（已加）**：进度 0/0.5/1 与反向映射端点正确；
- **效果器**：每个 DSP 的 bypass / 参数极值 / 自动化插值；
- **性能**：10 分钟音频 + 1 亿音符工程的播放 block 耗时上限（单独 bench，不放常规测试）。

---

## 6. 风险与屎山提示

1. **双时基是最大结构风险**。warp 引入后，「秒域」与「beat 域」必须只有一个主时基，否则编辑、导出、UI 会各算一套。
2. **render.rs 音频分支需要拆分**，否则 M1 之后会变成跨 200 行的单函数。
3. **伪 CC 效果器模型不能继续扩张**，音频效果器必须走独立参数模型。
4. **fade 曲线有三处重复**（渲染 / 波形绘制 / 拖拽手柄），M0 必须合并。
5. **`decode_audio` 的 `data.to_vec()`** 对长素材是整文件复制，可换 borrowed wrapper。
6. **`audio_channel` 共享 strip** 与 Ableton 每轨独立不同；Sampler / Drum Rack 落地前必须确定「乐器挂 track 还是挂 channel」。
7. **录音目前无 punch / take**，循环录音会产生覆盖问题，P0 内需补齐。

---

## 7. 文件索引

- `crates/yinhe-audio/src/audio_source.rs` — 解码 / 重采样 / 峰值 / WAV 编码
- `crates/yinhe-core/src/model.rs` — `AudioClip` / `TrackData` / `clip_fade_gain`
- `crates/yinhe-core/src/audio_warp.rs` — `WarpMap` / `WarpMarker`
- `crates/yinhe-audio/src/engine_render/audio_clip.rs` — `AudioClipRenderer`
- `crates/yinhe-audio/src/engine_render/render.rs` — 块调度 / 混音链
- `crates/yinhe-audio/src/audio_model.rs` — 播放模型准备、`clip_fades` 预计算
- `crates/yinhe-audio/src/export.rs` — 离线导出
- `crates/yinhe-editor-core/src/document/audio_clip_edit.rs` — Clip 编辑与 undo
- `crates/yinhe-egui/src/arrange/view_ui/audio_edit.rs` — 音频 Clip 交互
- `crates/yinhe-egui/src/app/audio_recording.rs` — 录音落地
- `crates/yinhe-mixer/src/params.rs` / `graph.rs` — 混音参数与图
- `crates/yinhe-dsp/src/registry.rs` — 内置效果器注册表
- `crates/yinhe-synth/src/sf_parser.rs` — SF2 / SFZ 采样库解析

---

## 8. 实施进度（2026-11-11）

### 已完成

1. **暂停 / 停止保留 delay/reverb 尾音**
   - 暂停/停止改为干声**输入侧** 10ms 淡出，insert / 通道 DSP 状态保留；
   - 新增 `render_tail`：只跑混音链，不推进走带、不派发音符；
   - 连续静音约 -100 dBFS × 4 块后停止渲染；
   - 用户停止按钮走 `StopTransport`，加载/导出仍用硬 `Stop` 复位。
2. **M0**
   - `yinhe_core::clip_fade_gain` 统一线性淡入淡出；
   - 音频取帧 4 点 Catmull-Rom；
   - `AudioClipRenderer` 拆分到 `engine_render/audio_clip.rs`。
3. **M1 后端**
   - `WarpMap` / `WarpMarker`、`AudioClip.start_beat` / `warp`；
   - tempo 变化按 beat 锚点重算秒域时间，素材跨度保持不变（varispeed）；
   - 新片段创建即带 beat 锚点；旧工程加载自动迁移；
   - 移动只更新起点；裁剪/拆分重建两端点映射；
   - 波形取素材位置与淡入淡出幅度和引擎共用同一套函数。

### 已知限制

- M1 目前是 **varispeed**：跟 BPM 伸缩时音高跟着变；pitch-preserving 未做。
- clip **内部**跨 tempo 自动化时，当前按整段线性映射，内部拍点会漂；
  需要按 tempo segment 分段映射（M1b）。
- Warp marker 的 UI 编辑（拖动/新增/删除/吸附）未做。
- fade 曲线、自动 crossfade（M2）未做。

### 下一步候选

1. **M1b**：tempo segment 精确映射 + warp marker UI；
2. **M3 单件**：先在 `yinhe-dsp` 做多模式 Reverb / Delay，验证尾音链路；
3. **M2**：fade 曲线 + Clip 包络 + 自动 crossfade。
