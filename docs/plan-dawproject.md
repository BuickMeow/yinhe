# DAWproject 支持规划

> 目标：让 yinhe 能与其他 DAW（Bitwig、Studio One 等）交换工程。
>
> 关联：`docs/plan-audio.md`（Warp / 音频功能路线）。DAWproject 的 `Warps` 与本项目规划的 `WarpMap` 语义一致，后者落地后可直接对齐。
>
> 约束前提：1 亿音符工程、桌面 + 移动端、不引入缓存、不 vendor、不做无意义中间模型。

---

## 0. 结论摘要

1. **DAWproject 值得支持，但它是交换格式，不是工程格式。** 普通工程可以高保真往返；1 亿音符工程只能范围 / 分轨导出。
2. **先做导出，后做导入。** 导出是结构已知的单向映射，能最快产生真实价值；导入要面对所有 DAW 的写法差异，是开放集合。
3. **不要使用 `dawproject` crate 的生产路径。** 它整棵 DOM 反序列化，内存随音符数爆炸。生产用 `quick-xml` + `zip` 流式读写，crate 只放 dev-dependency 做 conformance 测试。
4. **不要建中间 DOM。** 从 `YinModel` 直接吐 XML，避免 A→B→C 的多余转换。
5. **五类必然有损**：MIDI Port、RPN/NRPN 原生语义、Clip 内贝塞尔自动化、AudioClip.gain / reversed、Lyrics / Chord / KeySignature。
6. **音频轨用 `Clips timeUnit="seconds"`**，直接对齐 yinhe「固定秒数、不随 BPM 变」的语义；不要用 beats + Warps 强行模拟。

---

## 1. 规范概述

来源：`https://github.com/bitwig/dawproject`

| 项 | 内容 |
|---|---|
| 版本 | 1.0，已稳定 |
| 扩展名 | `.dawproject` |
| 容器 | ZIP |
| 内容 | `project.xml`、`metadata.xml`、音频文件、插件状态文件 |
| 编码 | UTF-8 |
| 时间 | beats 为主，可与 seconds 混用 |

能力范围（官方对照表）：

| 类别 | 支持 |
|---|---|
| Audio | Clip、fade、crossfade、amplitude、pan、time warping、transpose |
| Notes | Note、per-note expression |
| Automation | Tempo、拍号、MIDI messages、volume / pan / mute / sends、插件参数、内置设备参数 |
| Plug-ins | VST2 / VST3 / AU / CLAP + 完整状态 |
| Built-in Devices | 通用 EQ / Compressor / Gate / Limiter |
| Clip Launcher | Clips / Scenes |

### 1.1 关键元素

```
Project
├── Application
├── Transport        (Tempo, TimeSignature)
├── Structure
│   └── Track*       (contentType, color, name)
│       └── Channel  (role, destination, Volume/Pan/Mute/Sends/Devices)
├── Arrangement
│   ├── TempoAutomation / TimeSignatureAutomation
│   ├── Markers
│   └── Lanes (timeUnit)
│       └── Lanes track="id"
│           ├── Clips
│           │   └── Clip (time, duration, playStart, loop, fades)
│           │       ├── Notes / Note (time, duration, channel, key, vel)
│           │       └── Warps / Audio / File
│           └── Points (automation)
└── Scenes
```

### 1.2 对 yinhe 最有用的两个细节

1. **`Clip.fadeInTime` 允许负值**，负值表示 crossfade，起点自动前移 `time - abs(fadeInTime)`。未来做自动交叉淡化可直接映射。
2. **`Warps` 是分段线性 beats↔seconds 映射**，两个点即可表示固定速度音频。与 `plan-audio.md` 的 `WarpMap` 基本 1:1。

### 1.3 Rust 生态

`dawproject` crate（0.12，MIT OR Apache-2.0，quick-xml + zip + serde，MSRV 1.88）。只建议用于测试校验，不进入生产路径。

---

## 2. yinhe 现状

| 能力 | 现状 | 代码入口 |
|---|---|---|
| 工程模型 | `YinModel`，per-key 音符桶 | `yinhe-core/src/model.rs` |
| MIDI 通道 | 16 port × 16 channel = 256 | `TrackData.port / channel` |
| MIDI Clip | tick 区间，非重叠 | `yinhe-core/src/clip.rs` |
| 音频 Clip | 秒域，gain / fade / reverse | `yinhe-core/src/model.rs::AudioClip` |
| 音频素材 | 原始文件字节 + 名称 + 时长 | `yinhe-core/src/model.rs::AudioSource` |
| 混音 | strip / bus / send / insert / PDC | `yinhe-mixer` |
| 自动化 | Param / CC / PB / RPN / NRPN / Tempo，Step / Curve | `yinhe-types/src/automation.rs` |
| 设备 | SF2 / SFZ、VST3、CLAP、内置 3 效果器 | `yinhe-audio`、`yinhe-synth`、`yinhe-dsp` |
| 全局事件 | Tempo / 拍号 / Marker / Lyrics / Chord / KeySig | `ConductorData`、`TrackData` |
| 归档 | `.yin`，bincode + zstd | `yinhe-archive` |

---

## 3. 映射表

### 3.1 工程 / Transport

| yinhe | DAWproject | 保真度 |
|---|---|---|
| `ProjectMeta.name` | `metadata.xml/Title` | 保真 |
| `artist` | `Artist` | 保真 |
| `album` | `Album` | 保真 |
| `album_artist` | `OriginalArtist` | 保真 |
| `genre` | `Genre` | 保真 |
| `year` | `Year` | 保真 |
| `copyright` | `Copyright` | 保真 |
| `description` | `Comment` | 保真 |
| `conductor.tempo`（tick, bpm） | `TempoAutomation`，RealPoint `time=tick/ppq`、`value=bpm`、`unit=bpm`、`interpolation=linear` | 保真 |
| `default_bpm` | `Transport/Tempo value` | 保真 |
| `conductor.time_sig` | `TimeSignatureAutomation` | 保真 |
| `conductor.markers` | `Arrangement/Markers` | 保真 |
| `meta.ppq` | 无对应字段 | 丢失（导入时另定 PPQ） |

### 3.2 Track / Channel

| yinhe | DAWproject | 说明 |
|---|---|---|
| `TrackKind::Midi` | `Track contentType="notes"` | — |
| `TrackKind::Audio` | `Track contentType="audio"` | — |
| `TrackKind::Master` | `Track contentType="audio notes"` + `Channel role="master"` | — |
| `name` / `color` | `Track/@name` / `@color` | `[f32;4]` → `#rrggbb` |
| `muted` | `Channel/Mute` | — |
| `soloed` | `Channel/@solo` | — |
| `StripParams.gain` | `Channel/Volume`（unit=linear） | — |
| `StripParams.pan` | `Channel/Pan`（unit=normalized，0.5 居中） | 需换算 |
| bus | `Track contentType="audio"` + `Channel role="effect"` | — |
| `SendParams{pre_fader, bus, gain}` | `Send type=pre|post destination Volume` | 保真 |
| 共享 `audio_channel` | 转 submix 组 | 有损，见 4.3 |
| 插件 PDC / meter | 无对应 | 派生信息，可丢 |

### 3.3 音符 / MIDI Clip

| yinhe | DAWproject |
|---|---|
| `Clip {start_tick, end_tick}` | `Clip time=start_tick/ppq duration=len/ppq contentTimeUnit=beats` |
| `NoteEvent {start_tick, end_tick, key, velocity}` | `Note time=(start-clip_start)/ppq duration=gate/ppq key vel=velocity/127` |
| `TrackData.channel` | `Note/@channel`（0..15） |
| `port`（A..P） | 无对应，丢失 |
| `dup_index` | 无对应，丢失 |
| 音符在所有 Clip 之外 | 导出时合成「补洞 Clip」 |
| Clip 重叠 | yinhe 不允许；导入时拆分 / 合并 |

### 3.4 自动化

| yinhe `AutomationTarget` | DAWproject |
|---|---|
| `CC {controller}` | `expression=channelController channel controller` |
| `PitchBend` | `expression=pitchBend channel` |
| `ProgramChange` | `expression=programChange channel`（IntegerPoint） |
| `Param { ChannelInstrument }` | `parameter=<设备参数 id>`，同时写设备 `Parameters` |
| `Param { PluginInstrument }` | `parameter=<VST3 ParamID / CLAP id>` |
| `Rpn` / `Nrpn` | 无原生对应，展开为 CC 序列（101/100/6/38） |
| `Tempo` | `Arrangement/TempoAutomation` |

曲线：yinhe `Step` → `hold`；`Curve{贝塞尔}` → `linear`（有损）。

### 3.5 音频 Clip

| yinhe `AudioClip` | DAWproject |
|---|---|
| `start_seconds` / `duration_seconds` | `Clip time` / `duration`，音频轨用 `Clips timeUnit="seconds"` |
| `offset_seconds` | `Clip playStart`（contentTimeUnit=seconds） |
| `fade_in_seconds` / `fade_out_seconds` | `fadeInTime` / `fadeOutTime` + `fadeTimeUnit="seconds"` |
| `gain` | 规范无 clip gain 属性，降级丢失或烘焙 |
| `reversed` | 规范无 reverse，预生成反向 WAV 或丢失 |
| `AudioSource.data` | `audio/{uuid}.{ext}` |
| `AudioSource.name` | `Audio/@name` 或文件名 |

**音频轨统一用 `Clips timeUnit="seconds"`。** 理由：yinhe 的 AudioClip 就是「固定秒数、不随 BPM 变」，用 seconds lane 零转换保住语义；用 beats + Warps 会被其他 DAW 当作跟随 tempo 拉伸。

### 3.6 设备 / 插件

| yinhe | DAWproject |
|---|---|
| SF2 / SFZ 通道乐器 | `BuiltinDevice deviceID="yinhe.soundfont"` + `State` 存音色路径 JSON |
| VST3 insert | `Vst3Plugin deviceID pluginVersion State` |
| CLAP insert | `ClapPlugin deviceID State` |
| `BuiltinEffectKind::{Gain,Pan,Filter}` | `BuiltinDevice`（不要硬套 Equalizer） |
| `InsertRef.state` | `<State path="plugins/...">` |

首版建议只导出 deviceID + 参数，不导出 VST3 state：规范期望 `.vstpreset`，而 yinhe 存的是 component + controller 原始字节，跨 DAW 未必可读。

### 3.7 无法映射

- MIDI Port（256 通道模型）
- RPN / NRPN 原生 u16 语义
- Lyrics / Chord / KeySignature
- Clip Launcher / Scene（yinhe 没有）
- 共享 `audio_channel` 的「多轨一条 strip」
- `SegmentShape::Curve`
- `AudioClip.gain` / `reversed`

---

## 4. 结构性冲突

### 4.1 1 亿音符 vs XML

一个 `<Note .../>` 约 90–120 字节，1 亿音符约 **10 GB XML**。这不是优化能解决的。

对策：

- 禁止整棵 DOM；
- 流式写入；
- 提供时间范围 / 选中 Clip / 选中轨道导出；
- 导出对话框显示预计大小，全量导出允许但必须警告，禁止静默截断。

### 4.2 256 通道 vs 4-bit channel

`Note/@channel` 只有 0–15，port 必然丢失。导入时 port 归 0。

不建议发明私有属性；可把 port 写进 `Track/@comment` 作为人工线索，但不作为可靠往返依据。

### 4.3 通道中心 vs 轨道中心

yinhe 的乐器与 mixer strip 按 MIDI channel 组织，DAWproject 按 Track / Channel 组织。

- 导出：每个通道的乐器挂到该通道第一条 Track，其余留空并注释；
- 共享 `audio_channel`：导出 `role="submix"` 的 Track，原音频轨各建 Channel，`destination` 指向 submix；
- 导入：同通道多乐器按 yinhe 多乐器链处理；无 clip 的 submix Track 还原为 bus。

有损但语义可解释，必须写进 UI 提示。

### 4.4 秒域 AudioClip vs beats 主时基

见 3.5。秒域 lane 是正解。`WarpMap` 落地后再增加「beats + Warps」模式。

### 4.5 Clip 语义

DAWproject 的 Clip 有 `loopStart/loopEnd/playStart/playStop`，yinhe 的 MIDI Clip 只是音符归属区间，不支持 loop。

- 导入带 loop 的 Clip 必须展开成重复音符，否则静默丢内容；
- 展开前预估音符数并让用户确认（用户决策，不是算法阈值）；
- 嵌套 Lanes（group/folder）必须扁平化，分组名并入 Track 名。

---

## 5. 技术方案

### 5.1 不使用 `dawproject` crate 的生产路径

| 方案 | 优点 | 问题 |
|---|---|---|
| `dawproject` 0.12 | 上手快、XSD 覆盖全 | DOM 内存随音符数爆炸 |
| 自研 streaming | 内存恒定、可控 | 工作量略大 |

- 生产：`quick-xml` + `zip` 流式读写；
- 测试：`dawproject` crate 作为 dev-dependency，解析导出文件做 conformance 校验。

### 5.2 新 crate：`crates/yinhe-dawproject`

```
crates/yinhe-dawproject/
├── lib.rs        // export_project / import_project
├── write.rs      // YinModel → project.xml（流式）
├── read.rs       // project.xml → YinModel（流式状态机）
├── media.rs      // zip 读写、音频打包、安全校验
├── map.rs        // tick ↔ beat ↔ second、颜色、id 分配
└── error.rs
```

依赖：`yinhe-core`、`yinhe-types`、`yinhe-audio`（probe）、`yinhe-mixer`、`quick-xml`、`zip`、`thiserror`。

不放进 `yinhe-editor-core`（编辑语义层不应知道交换格式），也不放 `yinhe-midi-io`（只做 SMF）。

### 5.3 导出流程

```
1. 建 ZipWriter，先写 metadata.xml
2. project.xml：Application → Transport → Structure
3. Structure：逐 Track 写 Track + Channel + Devices
   - 先建 IdMap（track/channel/bus/master/device/clip）
   - 共享 audio_channel → submix
4. Arrangement：
   - TempoAutomation / TimeSignatureAutomation / Markers
   - 逐 Track 写 Lanes：
     MIDI 轨：Clips(beats) → Clip → Notes
     音频轨：Clips(seconds) → Clip → Audio + File
5. 写音频文件与插件 state
6. 关闭 project.xml，完成 zip
```

关键点：

- **id 必须先规划**：`destination`、`parameter` 是前后向引用，不能边写边猜；
- **音符遍历**：对每个 Clip 区间，在 128 个 key bucket 各做一次 `partition_point`，复杂度 `O(128 × clips × log N + N)`；
- **补洞 Clip**：先算 Clip 与音符覆盖范围的差集，未覆盖音符生成合成 Clip；
- **音频文件**：原样写 `AudioSource.data`，不解码不转码；扩展名取 `name` 后缀，否则 `probe_audio_info`；`sampleRate` / `channels` 用 probe；
- **进度**：复用 `export.rs` 的 `ExportProgress` 模式，后台线程 + 原子进度。

### 5.4 导入流程

用 `quick-xml` 拉式解析 + 显式状态栈，不建 DOM。

难点：

1. **音符批量写入**
   按 key 收集到临时 `Vec<NoteEvent>`，每满一个 chunk 排序后批量合并进 `NoteBucket`，复用 `load_track_notes` 的批量路径，避免逐条 `insert` 的 O(N²)。

2. **PPQ**
   DAWproject 没有 PPQ。导入用当前工程 `meta.ppq`（默认 480），`tick = round(beat * ppq)`，并在导入报告中说明可能存在的量化误差。

3. **Clip loop**
   `loopStart/loopEnd` + `duration` 展开前预估音符数，让用户确认。

4. **未知设备**
   保留 `InsertRef`（name + plugin_id + format + state），UI 显示「缺失插件」，与现有工程加载行为一致。

5. **合并策略**
   首版把 `.dawproject` 作为「打开工程」，不合并到当前文档；合并涉及重编号，后续再做。

### 5.5 安全

外部输入按不可信处理：

- zip 拒绝 `..`、绝对路径、软链接；
- 限制条目数、单文件解压大小、总解压大小；
- XML 限制嵌套深度，拒绝 DTD / 外部实体；
- 悬空引用降级，不 panic；
- 生产路径禁止 `unwrap()`。

---

## 6. 保真度分级

| 级别 | 内容 |
|---|---|
| 保真 | 轨道名 / 颜色 / 静音 / 独奏、gain / pan、send pre/post、bus、音符 key / vel / gate、tempo / 拍号 / marker、音频位置 / offset / 时长 / fade |
| 降级 | 贝塞尔自动化 → linear、pan 精度、PPQ、Clip loop 展开、group 扁平化、共享 audio_channel → submix |
| 丢失 | MIDI Port、RPN / NRPN 原生语义、Lyrics / Chord / KeySignature、AudioClip.gain、reversed、dup_index |

导出对话框直接列出「本次会丢失 X 项」。

---

## 7. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M1 导出核心 | Project / Transport / Structure / Arrangement、MIDI 轨、音符、tempo / 拍号 / marker | Bitwig 能打开，音符正确 |
| M2 导出音频与混音 | 音频轨（seconds lane）、fade、bus / send、Channel 参数、音频打包 | 位置与混音正确 |
| M3 导出设备 | SF2 / SFZ BuiltinDevice、VST3 / CLAP 引用、内置效果器、自动化 | 设备与自动化可用 |
| M4 导入 | 流式解析、音符批量、Clip / Audio / Track / Transport、未知设备降级 | 反向可用 |
| M5 往返打磨 | 真实文件 fixture、conformance 测试、错误报告、UI 提示 | 达到「可交换」质量 |

先后理由：导出结构已知、价值直接；导入是开放集合，先用导出文件实测校准规范理解，再做导入。

---

## 8. 测试计划

- **conformance**：导出文件用 `dawproject` crate 解析，断言元素与属性；
- **round-trip**：yinhe → dawproject → yinhe，对比音符数、tick、channel、velocity、track / clip；
- **边界**：空工程、无 Clip 的音符、跨 Clip 长音符、负 fade（crossfade）、音频 offset；
- **规模**：10 万 / 100 万音符导出的时间与内存上限（1 亿只做流式内存断言，不做完整文件）；
- **安全**：路径穿越、zip bomb、深嵌套 XML、悬空 id；
- **fixture**：Bitwig README 示例工程 + 自建最小工程。

---

## 9. 代码质量与风险

1. **不要建 DAWproject DOM**，否则会自然滑向「先转中间模型再写」。
2. **`map.rs` 是唯一单位换算点**，tick / beat / second、颜色、pan 只在这里换算。
3. **`write.rs` 按 Structure / Arrangement / Audio / Automation 拆函数**，避免上帝对象。
4. **`read.rs` 用显式状态栈**，不要依赖元素顺序假设。
5. **文件菜单**：`File → Export → DAWproject`，macOS 菜单栏同步。
6. **导出线程**：不能阻塞 UI，复用现有导出进度模型。
7. **不建议支持 Scenes / Clip Launcher**：yinhe 没有对应模型，做了也是空壳。

---

## 10. 参考

- 规范：`https://github.com/bitwig/dawproject`
- XML Reference：`https://github.com/bitwig/dawproject/blob/main/Reference.html`
- XSD：`Project.xsd`、`MetaData.xsd`
- Rust 移植（仅测试）：`https://crates.io/crates/dawproject`