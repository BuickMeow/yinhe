# spec-yinhe-dsp：内置效果器、CC 模块化与 GM2 全链

> 状态：设计稿（待评审）
> 范围：阶段 A（已落地）= xsynth 通道级 DSP 向 yinhe-dsp 模块迁移（CC 模块化）+ 统一参数模型 + 通道处理回归音源（见 §4.1）；阶段 B = GM2 Reverb/Chorus + master 轨 + SysEx 全链
> 关联：`spec-xsynth-integration.md`（效果器链预留架构）、`docs/GMLevel2.xml`（参数词典）

---

## 一、背景与目标

yinhe 已有效果器链（`yinhe-mixer` 的 `InsertProcessor`，每通道/bus/master 一条链），但目前只能挂外部 CLAP/VST3 插件，没有任何内置 DSP。本规格定义新 crate **yinhe-dsp**：内置效果器模块，像插件一样自由拼搭。

**长期路线（本节是最重要的方向约定）**：

```
阶段 A（已落地）：xsynth 通道级 DSP 迁移为 yinhe-dsp 模块，
   统一参数模型 AutomationTarget 落地 + 通道处理回归音源（见 §4.1）
   ↓  ChannelGain/Pan/Filter 作为内置音源的通道处理段（ChannelDspChain，见 §5）
阶段 B：GM2 效果器（Reverb/Chorus）与 SysEx 全链
   ↓  xsynth 0.4 完全没有这两者，必须新做
阶段 C：XG/GS 效果器（同一模块体系扩展）
   ↓
远期：xsynth 精简为 SF2/SFZ 采样加载器 + voice 级参数（ADSR/pitch/sustain）
```

**优先级**：先把 CCS 模块化（DSP 接管）做扎实，再启动 GM2 效果器，GM2 完成后才启动 XG/GS。

**CC 子模块化**：每一个 CC 或一组 CC 对应一个可挂在效果器链上的子模块，可与其他 VST/CLAP 效果自由组合、任意排列；逐步把 DSP 从 xsynth 迁到 yinhe-dsp，最终 xsynth 只保留"一定要对音源动刀子"的 voice 级处理（ADSR、音高、延音等）。

核心诉求（用户原话归纳）：

1. yinhe-dsp 首先是**音频 DSP 模块**；SysEx 不进底层模型，底层用**自动化参数**表达。
2. 导入 MIDI 时：CC/PB/RPN/NRPN 与效果 SysEx → 统一自动化参数；导出 MIDI 时反向还原（第三方插件参数不导出）。
3. 效果器像模块一样自由拼搭，纯手动挂载，不自动创建实例。
4. 参数事件按**设备 + 参数 id 精确寻址**（统一参数模型），不再广播给所有同型实例。
5. 参数面板手动调参 = 写自动化 lane（lane 是唯一真相）。
6. 阶段 B 能力：AM 回放驱动 DSP、CC91/93 打通 send、导出生成 SysEx。
7. 顺带支持 **master 轨**（挂全局 CC，导出时展开到所有 MIDI 通道）。
8. 阶段 A 能力（已落地）：CC 模块接管 xsynth 的通道级处理，用户逐通道迁移试听；统一参数模型（§4.1）落地为后续一切自动化数据的底座（回放路由由 target 类型决定，D15）。

### 决策记录（已确认）

| # | 决策 | 结论 |
|---|---|---|
| D1 | yinhe-dsp 职责 | 音频 DSP；SysEx 仅在导入/导出层与自动化参数互转 |
| D2 | 第一批效果范围 | 只做 GM2 Reverb/Chorus（含全部 7 个参数） |
| D3 | 效果器拓扑 | 纯手动拼搭，导入不自动挂效果器 |
| D4 | 参数事件寻址 | 参数身份 = 设备 + 参数 id（精确寻址，§4.1）；MIDI CC 保留为低层事件，回放交给音源侧（内置=通道处理段；插件=透传，D15） |
| D5 | 调参行为 | 写 lane（在播放头 tick），lane 是唯一真相（lane 存归一化值，UI 换算显示原始值） |
| D6 | SysEx 全链 | 效果参数解析+生成；开关类消息（GM1/GM2 System On 等）丢弃 |
| D7 | 未识别 SysEx | 丢弃（与现状一致） |
| D8 | 算法基准 | 参数语义对即可，不要求逐样本复刻硬件 |
| D9 | 复用 UI 体系 | `PluginInstance` 新增 Builtin 变体，复用参数面板/旁通/回收流程 |
| D10 | GPU 绕过混音台 | 已完成（GPU 已接入混音台，见 §5.7） |
| D11 | master 轨 | 本规格一起做（新增 `TrackKind::Master`） |
| D12 | CC91/93 目标 bus | 自动识别（bus 链上挂 Gm2Reverb/Gm2Chorus 者） |
| D13 | 效果器优先级 | GM2 完成后才启动 XG/GS |
| D14 | DSP 迁移方向 | yinhe-dsp 的 Gain/Pan/Filter 作为**内置音源通道处理段**（`ChannelDspChain`），在合成器输出后、insert 链前处理；xsynth/GpuSynth 保持纯音源（无通道 DSP），`channel_set.rs` 硬切断保留 |
| D15 | 接管语义 | CC 归音源：内置音源通道的 CC7/10/11/71/74 由通道处理段消费；插件音源通道 CC 原样透传插件（自行响应）；外挂 insert 效果器不接收 CC（无广播/无双发）；不改 xsynth 源码 |
| D16 | 参数粒度 | 按功能组：Gain(Volume/Expression)、Pan、Filter(Cutoff/Resonance)，绑定见 §4.1；CC8 不采用 |
| D17 | CPU DSP 依赖 | yinhe-dsp 不依赖 wgpu/yinhe-synth；GPU 与 CPU 实现独立 |
| D18 | GPU/CPU 分工 | GPU 只做高并发 voice 发声；效果器链保持 CPU（数据量小、串行依赖、需与 CLAP/VST3 混合） |
| D19 | CC 补齐策略 | 先迁移 xsynth 已有通道级 CC；通道级缺失（94、标准 Balance）顺带补；voice 级缺失（Vibrato/Portamento 等）不承诺，属远期 |
| D20 | 参数模型 | `AutomationTarget::Param { device: ParamDevice, id: u32, name: String }` 统一"设备参数"（u32 id 对齐 VST3/CLAP）；`CC`/`Rpn`/`Nrpn` 保留为无设备归属的低层变体；`Tempo` 是唯一不归一化的量（§4.1） |
| D21 | 值域归一化 | 自动化值一律归一化 0..1（除 Tempo）：CC、RPN0/2 除 127，PB、RPN1、NRPN 除 16383；导入归一化，回放/导出边界取整还原（无损可逆）；音符 velocity/gate 本次不归一化（保持整数） |
| D22 | 设备寻址 | `ParamDevice::{ChannelInstrument, PluginInstrument, ChannelDsp}` 均只带通道号：不用槽位序号、不用实例 uuid，导入数据先归属，挂/删/重排效果器不影响自动化；第三方插入效果器实例寻址（uuid）将来以 `Insert { target, instance }` 扩展 |
| D23 | 工程格式 | `.yin` 容器版本 6 → 7（`AutomationTarget` 序列化变更），不兼容 v6：旧档不迁移，重新导入 MIDI |

---

## 二、现状摘要

### 2.1 效果器链（yinhe-mixer）

- `InsertProcessor`（`crates/yinhe-mixer/src/graph.rs:21`）：纯音频口 `process(&mut self, left, right)`，无事件入参、无 position 入参；参数靠处理器内部的 `ParamQueue`（`param_queue.rs`，归一化 0..1，`Arc` 跨线程，latest-wins）。
- 挂载点：`InsertTarget::{Channel, Audio, Bus, Master}`（`crates/yinhe-audio/src/spawn.rs:16`）；命令 `AudioCommand::InsertAdd/Remove/Replace`。
- 持久化：`InsertRef { plugin_path, plugin_id, name, format: PluginFormat::{Clap,Vst3}, bypassed, state }`（`crates/yinhe-mixer/src/params.rs:69`）；`.yin` 混音段 `MIXER_SECTION_VERSION = 5`（`crates/yinhe-yin/src/io.rs`）。
- 旁通：`Arc<AtomicBool>`（与渲染线程共享，零命令往返）。
- 现有内置 DSP 只有限幅器（`yinhe-synth/limiter.rs`），不属链。

### 2.2 合成器（yinhe-synth / yinhe-audio）

- 音频引擎用 `ChannelSet`（`crates/yinhe-audio/src/channel_set.rs`）自建"分通道渲染版 ChannelGroup"：每 MIDI 通道一个 xsynth `VoiceChannel`，输出 planar 进混音台通道缓冲。
- **xsynth-core 0.4 只有 `filter.rs` / `limiter.rs` 两个 effect，没有 reverb/chorus**——GM2 效果必须由 yinhe-dsp 新做。
- xsynth 的 CC 处理分两层（`channel/mod.rs:154-190` `apply_channel_effects`）：
  - **通道级音频处理**（可迁移）：CC7/11 增益、CC8/10 等功率声像、CC71/74 通道 biquad 低通。
  - **voice 级**（不可迁移）：ADSR（CC72/73/75）、Sustain 等事件层、PitchBend/Tuning、Portamento、Vibrato、Bank/PC。
- GPU 合成器路径（`GpuSynth`）：voice 级 CC 状态机与 per-voice 滤波（`gpu_synth.rs`、`synth/filter.rs`）；已按 §5.6 精简并接入混音台（§5.7），通道级 CC 走 CPU 效果器链。

### 2.3 自动化与 MIDI 管线

> 统一参数模型（§4.1）已落地：CC/PB/RPN/NRPN 的存储、路由与导出以该节为准；本节其余内容为改造前现状（行号可能已漂移）。

- lane 模型：`AutomationTarget`（`crates/yinhe-types/src/automation.rs:373`）已按统一参数模型（§4.1）落地；`AutomationLane` 存于 `TrackData.automation_lanes`，Tempo 存于 `ConductorData.tempo`。
- 导入 `crates/yinhe-midi/src/parser.rs`：SysEx 在 `:563`（每轨）与 `:265`（conductor pass）被丢弃；RPN/NRPN 解析流程可作参考。
- 导出 `crates/yinhe-midi/src/writer.rs`：不写 SysEx；`push_lane_event`（`:362`）对 `Tempo` 与第三方插件参数静默跳过；`sort_by_key` 稳定，同 tick 顺序 = push 顺序。
- 回放 `crates/yinhe-audio/src/audio_model.rs`：`flatten_automation_to_cc_events`（`:302`）把 lane 展平为 `SortedCC`；dispatch 在 `engine_render.rs:112`，`ChannelInstrument` 参数仅路由到乐器插件。
- CC91/93 现为普通 CC（`channel.rs:413` 存入通用 cc_values，随事件发 xsynth；xsynth 无 reverb/chorus 故实际无效果），与混音台 bus send 无关联。

### 2.4 测试数据现状

扫描 `/Users/jieneng/Music/MIDIs`（235 个文件，95 含 SysEx）：GM1 System On 40 次、Master Volume 3 次、Roland GS 2 次；**无 GM2 参数 SysEx 样本**。验收需自造用例（手工字节或 Domino 导出）。

---

## 三、GM2 编解码规范（权威依据）

依据：GM2 Global Parameter Control（CA-024）实际实现（gervill `UniversalSysExBuilder`）+ Domino `docs/GMLevel2.xml` 的 `@SYSEX` 模板，二者字节级一致。

### 3.1 消息格式

```
F0 7F <dev> 04 05 <nSlot> <nParam> <nValue> <SlotPath...> <ParamID...> <Value...> F7
```

- `<nSlot>/<nParam>/<nValue>` 为**元素个数**（非字节数）。
- `SlotPath` 每元素 2 字节大端。
- `<dev>` 解析时接受任意 7 位值（惯例 7F），生成时固定 7F。

### 3.2 参数表

| 效果 | SlotPath | ParamID | 参数 | 值域 | GM2 默认 |
|---|---|---|---|---|---|
| Reverb | `01 01` | `00` | Type | 0..8 | 4 (Large Hall) |
| Reverb | `01 01` | `01` | Time | 0..127 | 64 |
| Chorus | `01 02` | `00` | Type | 0..5 | 2 (Chorus 3) |
| Chorus | `01 02` | `01` | Mod Rate | 0..127 | 3 |
| Chorus | `01 02` | `02` | Mod Depth | 0..127 | 19 |
| Chorus | `01 02` | `03` | Feedback | 0..127 | 8 |
| Chorus | `01 02` | `04` | Send to Reverb | 0..127 | 0 |

- Reverb Type：0=Small Room, 1=Medium Room, 2=Large Room, 3=Medium Hall, 4=Large Hall, 8=Plate（5/6/7 保留）。
- Chorus Type：0..3=Chorus 1..4, 4=FB Chorus, 5=Flanger。

### 3.3 字节示例

```
Reverb Type=4:   F0 7F 7F 04 05 01 01 01 01 01 00 04 F7
Reverb Time=64:  F0 7F 7F 04 05 01 01 01 01 01 01 40 F7
Chorus Type=2:   F0 7F 7F 04 05 01 01 01 01 02 00 02 F7
Chorus Rate=3:   F0 7F 7F 04 05 01 01 01 01 02 01 03 F7
```

### 3.4 丢弃的消息（第一批）

`F0 7E xx 09 01/02/03 F7`（GM1/GM2 System On/Off）、`F0 7F xx 04 01..04`（Master Volume/Balance/Fine/Coarse Tuning）、XG/GS 消息、其余未识别 SysEx。

> midly 的 `TrackEventKind::SysEx(data)`：data 不含 `F0`，含末尾 `F7`；写出时 midly 自动补 `F0` 与长度（`event.rs:170`）。

---

## 四、详细设计

### 4.1 统一参数模型（阶段 A 起）

**目标**：所有可自动化参数共用一套身份与值域。MIDI（CC/PB/RPN/NRPN）与 SysEx 只是导入/导出边界的表示；参数身份 = **设备 + 参数 id**（u32 与 VST3/CLAP 对齐），回放路由由 target 类型决定（不再按 CC 号白名单硬截胡）。

```rust
// crates/yinhe-types/src/automation.rs
pub enum ParamDevice {
    /// 通道内置 XSynth 参数（id 见 XSYNTH_PARAMS）。
    ChannelInstrument { channel: u8 },
    /// 通道乐器插件（VST3/CLAP）参数（id 为插件原生 ParamID/clap_id）。
    PluginInstrument { channel: u8 },
    /// 通道内置 DSP（ChannelGain / ChannelPan / ChannelFilter，id 见 CHANNEL_DSP_PARAMS）。
    ChannelDsp { channel: u8 },
    // 将来扩展：Insert { target, instance } —— 第三方插入效果器的实例寻址（uuid）。
}

pub enum AutomationTarget {
    /// 统一"设备参数"：设备 + 设备内 u32 参数 id + 参数显示名。
    Param { device: ParamDevice, id: u32, name: String },
    /// 无设备归属的原始 CC（无内置绑定、用户自建）。
    CC { controller: u8 },
    /// 未映射的标准 RPN / NRPN。
    Rpn { parameter: u16 },
    Nrpn { parameter: u16 },
    /// BPM 时间轴（唯一不归一化的量）。
    Tempo,
}
```

**设备寻址约定**：

- **不用槽位序号、不用实例 uuid**：三个 `ParamDevice` 变体都只带通道号。导入的 MIDI 数据在挂效果器之前就已有归属（CC/PB/RPN 天生属于通道），挂/删/重排效果器不影响既有 lane。
- 通道 CC 的**回放交给音源侧**：内置音源通道由通道处理段消费 CC7/10/11/71/74（合成器输出后、insert 链前）；插件音源通道原样透传。外挂 effect 链与 CC 完全解耦。
- 内置与第三方分开：`ChannelInstrument` 是内置 XSynth（id 见下表），`PluginInstrument` 是第三方乐器插件（id 为插件自身的 ParamID/clap_id，不在内置表中）。
- 第三方插入效果器的实例级参数（需要 uuid 区分）不在本期；将来新增 `ParamDevice::Insert { target, instance }` 扩展。

**值域归一化（除 Tempo）**：

| 原始 | 位宽 | 存储值 | 边界还原 |
|---|---|---|---|
| CC | 7-bit（0..127） | `v / 127.0` | `round(v × 127)` |
| Pitch Bend | 14-bit（0..16383） | `v / 16383.0` | `round(v × 16383)` |
| RPN 0 / RPN 2 | 7-bit（0..127） | `v / 127.0` | `round(v × 127)` |
| RPN 1 / NRPN | 14-bit（0..16383） | `v / 16383.0` | `round(v × 16383)` |
| Tempo | BPM（f32 原值） | 不归一化 | — |

- 无损可逆：整数 → 归一化 f32 → `round(×max)` 必回原值（f32 对 ≤16383 的整数精确）。
- Curve 插值在归一化空间进行，取整只发生在边界（回放投递/导出），曲线不会因取整变阶梯。
- UI 显示与输入换算回原始值（如 Volume 存 100/127、界面显示 `100`）；换算走绑定表给出的上限。
- 音符 **velocity/gate 本次不归一化**，保持整数（不属于自动化值域改造范围）。

**边界职责**：

| 环节 | 职责 |
|---|---|
| 存储（内存 / `.yin`） | `Param`/`CC`/`Rpn`/`Nrpn` 事件值一律归一化 0..1；Tempo 存 bpm；velocity/gate 整数 |
| 导入 | MIDI CC → 保留低层 `CC` 事件；PB/RPN 0/1/2 → `Param{ChannelInstrument}`；其他 RPN/NRPN 保留低层变体；统一归一化后写入 lane（§4.7） |
| 回放 | CC 归音源侧（内置=通道处理段；插件=透传）；边界还原整数后投递（§4.5） |
| 导出 | `Param` → 绑定的 MIDI 消息（还原整数）；未映射变体原样写出；第三方插件参数**不导出**（§4.8） |
| UI（面板/事件编辑器） | 显示与输入按原始值换算；写回 lane 时归一化（§4.10） |

**导入映射（已落地）**：

| MIDI 来源 | 目标 |
|---|---|
| 所有 CC | 低层 `CC{controller}`（值归一化；回放交给音源侧：内置音源=通道处理段，插件=透传） |
| Pitch Bend、RPN 0/1/2 | `Param{ChannelInstrument, 对应 id}` |
| 其他 RPN / NRPN | 保留 `Rpn`/`Nrpn` 低层变体 |
| Tempo | 不变（`ConductorData.tempo`，BPM 原值） |

- 值域统一归一化 0..1（Tempo 除外）；引擎 flatten 边界按绑定还原整数（无损）。
- **回放（CC 归音源）**：内置音源通道（未挂插件乐器）→ CC7/10/11/71/74 由通道处理段（`ChannelDspChain`）消费，其余 CC 走合成器；插件音源通道 → CC 原样透传插件。**两层互斥，无广播、无双发**；外挂 insert 效果器不接收 CC。
- **chase/seek 回填**：按 `DSP_CHANNEL_CCS` 把 `ChannelState` 当前值写回内置音源通道处理段（已 dispatch 的 CC 跳过，避免旧值覆盖新值）。
- 为什么 CC 不转设备参数：CC 是事件（谁订阅谁消费）；设备参数（`Param`）是无 MIDI 信道参数（GM2 SysEx、插件原生参数）的载体。不做"CC ↔ 参数"身份转换，避免"转回来才能透传"的往返。
- 为什么 CC 不转设备参数：CC 是事件（音源侧消费）；设备参数（`Param`）是无 MIDI 信道参数（GM2 SysEx、插件原生参数）的载体。

**内置参数绑定表**（权威唯一，`yinhe-types` 的 `XSYNTH_PARAMS`/`CHANNEL_DSP_PARAMS`；`yinhe-dsp` 的 `EffectParamInfo` 与其一致性由测试锁定）：

XSynth（`ParamDevice::ChannelInstrument`）：

| id | 参数 | MIDI 绑定 | 值域 | 默认/中心 |
|---|---|---|---|---|
| 0 | Sustain | CC64 | 0..127 | 0 |
| 1 | Release | CC72 | 0..127 | 64 |
| 2 | Attack | CC73 | 0..127 | 64 |
| 3 | Pitch Bend | PB | 0..16383 | 8192（中心） |
| 4 | PB Sensitivity | RPN0 | 0..127 | 2（半音） |
| 5 | Fine Tune | RPN1 | 0..16383 | 8192（中心） |
| 6 | Coarse Tune | RPN2 | 0..127 | 64（中心，0 半音） |

ChannelDsp（`ParamDevice::ChannelDsp`）：

| id | 参数 | MIDI 绑定 | 值域 | 默认/中心 |
|---|---|---|---|---|
| 0 | Volume | CC7 | 0..127 | 127 |
| 1 | Expression | CC11 | 0..127 | 127 |
| 2 | Pan | CC10 | 0..127 | 64（中心） |
| 3 | Cutoff | CC74 | 0..127 | 64（中心） |
| 4 | Resonance | CC71 | 0..127 | 64（中心） |

**与阶段 B（GM2）的关系**：GM2 效果参数同样遵循"设备 + 参数 id + 归一化值"，但它是 bus/master 级设备，其 `ParamDevice` 变体随阶段 B 定义（路由统一由 target 类型决定）；SysEx 编解码（§3）与导入/导出流程（§4.7/§4.8）不变。

### 4.2 GM2 效果参数（阶段 B，纳入统一模型）

GM2 效果参数不再有独立 target：按统一参数模型（§4.1）存为 `Param { device: <bus/master 内置效果器设备，阶段 B 定义>, id }`，`id` 用 GM2 ParamID（0=Type、1=Time/Rate、…，见 §4.11）。`Gm2EffectUnit { Reverb, Chorus }` 保留为设备语义/UI 概念（选择器、CC91/93 目标识别）。

约定：

- 归一化分母统一按 7-bit（127）；各参数**显示上限**不同（Reverb Type 8、Chorus Type 5，其余 127），UI 换算用绑定表上限。
- `default_value()`：见 §3.2 默认列（归一化后）。
- `display_name()`：`"GM2 Reverb Type"`、`"GM2 Chorus Mod Depth"` 等（i18n 走现有文案体系）。
- `default_shape()`：Step（离散参数；连续量的 Curve 由用户自选）；无 14-bit、无中线。
- 新增参数必须补齐的穷举 match 清单（编译器强制）：
  `yinhe-types/automation.rs`（绑定表 + max/default/shape/display）、`yinhe-midi/writer.rs:370`、`yinhe-audio/audio_model.rs:442`、`yinhe-editor-core/clipboard_file.rs:245`（+`read_target` tag）、`yinhe-wgpu/automation/prepare.rs:34`、`yinhe-egui/right_panel/event_browser/tree.rs:313`、`detail.rs:1193`、`yinhe-midi/examples/cc_stat.rs`。

### 4.3 master 轨（`TrackKind::Master`）

**为什么需要 master 轨（CC 层级澄清）**：

- MIDI 1.0 的 128 个 CC **全部是通道级**，标准中不存在"master CC"；设备级/全局参数走 SysEx（GM2 效果参数、Master Volume/Tuning 等）。
- 混响是两层组合：**CC91 是通道级送量**（每通道送多少进混响），**混响参数（Type/Time）是设备级**（唯一一台）。本设计分别对应 §4.6（CC91 → 通道 bus send）与 §4.2 的 GM2 参数 lane。
- master 轨承载的 CC **不是标准语义**，而是"用户想让所有通道统一收到同一 CC"的工程内便利工具，回放广播、导出展开；是否使用完全由用户决定。

**模型约定**：

- `TrackKind` 增加 `Master`；`TrackData.port/channel = 0/0`（不占 MIDI 通道命名空间）；`notes`、`program_change`、`audio_clips` 恒空；只允许 `automation_lanes`（MIDI 层参数 + GM2 效果参数）。
- 工程内**至多一条**，追加在 `tracks` **末尾**（避免 conductor 式插入 0 引发的全量索引重映射；master 轨加入后 `lane.track` 天然正确）。
- 加载/导入后自动 ensure（类似 conductor），不提供手动新建/删除入口。
- **Tempo 仍归 conductor**；master 不承载 Tempo。

**命名冲突处理**：conductor 轨现在 badge 显示 `"Master"`（`arrange/track_panel.rs:349`），需改名（建议 `"Conductor"`），新轨显示 `"Master"`；混音台右侧总线条已是"主输出"，三者在文案上区分。

**播放语义**：

- master 的 MIDI lane（CC/PB/RPN/NRPN）在 `flatten_automation_to_cc_events` 中**复制到所有激活的 MIDI 通道**（每个通道生成一份 `SortedCC`），使回放等价于导出展开。
- master 的 GM2 参数 lane **不做通道复制**，按设备精确寻址（§4.2、§4.5）。
- `engine_state.rs:42` 的 `skip_track` 不能把 master 跳过（该轨无音符属正常）。
- master lane 的 AM M/S 试听按现有 `(track, target)` 掩码工作。

**导出语义**：

- master 轨**不生成自己的 SMF track**。
- 其 MIDI lane 展开到**本次实际写出的 MIDI 轨**（`strip_empty_tracks` 判定之后）的通道；同一 `(port, channel)` 重复时只注入第一条轨。
- 同 tick 顺序：**master 先 push、目标轨自身 lane 后 push** → 通道自身事件覆盖 master（"master 是全局基准，通道可覆盖"）。
- master 的 GM2 参数 lane 生成 SysEx，**只写一次**，写到第一条实际写出的 MIDI 轨。

**UI 要点**：

- AR：轨道条显示 `MASTER` badge/专用色；可展开 AM 子行（现有 `build_row_layout` 只特判 conductor 不展开，master 默认走普通展开路径即可）；不显示 M/S；不允许拖动排序/删除。
- PR：master 不能作为音符写入目标（与 conductor 一并用 helper 拦截：`note_edit`、`pencil`、粘贴、移动、MIDI 实时输入回退）。
- MIDI 输入实时写轨回退（`edit_state.rs:98`）需跳过 master。
- 事件浏览器：master 不混入 Port/Channel 树，加独立节点（或与 conductor 并列）。
- "添加自动化" picker：master 的 `plugin_instrument_of` 特判为空（不显示插件设备），只列 `AUTOMATION_TARGETS`。
- 混音台通道条聚合（`mix.rs:458`）排除 master，避免幽灵 A01。

**引擎侧静默点（必须人工处理）**：

- `yinhe-audio/channel_layout.rs:59,190`：`!= Audio` → 改为 `matches!(kind, Midi)`，否则 master 激活幽灵 A01。
- `yinhe-editor-core/track_ops.rs:56`（`used_channels`）：按 kind 过滤，Master/Audio 不参与。
- `yinhe-egui/app/audio.rs:699`：master 提前 return。
- `yinhe-egui/arrange/track_panel.rs:348`：badge 分支。
- `yinhe-yin/mapping.rs`：serde 自动携带 kind；旧版本读含 Master 的存档会 `unknown variant`（接受的限制）。

### 4.4 yinhe-dsp crate

```
crates/yinhe-dsp/
  Cargo.toml            # 依赖：yinhe-mixer（InsertProcessor）、yinhe-types、serde；不依赖 audio/egui
  src/lib.rs
  src/gm2.rs            # Gm2EffectUnit 设备语义/参数 id/值域（与 yinhe-types 绑定表同步）
  src/gm2/reverb.rs     # Gm2Reverb（阶段 B）
  src/gm2/chorus.rs     # Gm2Chorus（阶段 B）
  src/cc/gain.rs        # ChannelGain  （阶段 A，CC7/11，已实现）
  src/cc/pan.rs         # ChannelPan   （阶段 A，CC10，已实现）
  src/cc/filter.rs      # ChannelFilter（阶段 A，CC71/74，已实现）
  src/dsp/delay.rs      # 延迟线
  src/dsp/comb.rs       # 梳状/全通（混响用）
  src/dsp/lfo.rs        # 正弦 LFO
  src/dsp/onepole.rs    # 单极低通（平滑/阻尼）
  src/dsp/biquad.rs     # biquad 系数/状态（阶段 A；与 yinhe-synth 存在复用点，见 §10-6）
```

**DSP 设计（参数语义对齐，音质自研）**：

- `Gm2Reverb`：Freeverb 类结构（8 comb + 4 allpass，左右微失谐）。Type 映射预延迟/衰减/阻尼组合；Time(0..127) 映射反馈系数（0.36s..9s 对数语义）。干湿比固定为"全湿输出由 send 量控制"还是内置 dry/wet？
  - 采用 **insert 全湿**（输出即湿声，干声由 send 量决定）会需要并联结构；现有链是串联，**改为内置固定 dry/wet 比例**（如 dry 0.7 / wet 1.0），语义上近似 GM2 音源（send 越大混响越多，因为 send 是额外叠加）。
  - 结论：Gm2Reverb 输出 = dry + wet（wet 由 CC91/bus send 在下游控制总量），实现简单且听感可用。
- `Gm2Chorus`：2 条调制延迟线（左右反相 LFO）+ 反馈。Type 映射预设（Chorus1-4 不同 Rate/Depth/相位，FB Chorus 加反馈，Flanger 短延迟+高反馈）；Mod Rate/Depth/Feedback 直接映射。
- `Send to Reverb` 参数：**第一批只存 lane 与导出，不参与回放**（需要辅助发送路径，后续再做）。
- 参数接收：
  - 实时调参：内部 `Arc<ParamQueue>`（参数 id 0..4 对应 ParamID），process 开头 drain。
  - 内置效果参数：由 `yinhe-audio` 按设备定位目标链后投递（精确寻址，§4.1/§4.5）；**不广播**。
  - 参数平滑：块内一阶平滑（参考 `gpu_synth.rs` 的 `ValueLerp`），避免 zipper。
- 实时约束：缓冲构造时分配；`process` 零分配/零锁/不 panic；`reset()` 清空延迟线。

**API 扩展（yinhe-mixer，零内部依赖原则不变）**：

```rust
// graph.rs（在现有 handled_ccs/apply_cc 之上）
impl MixerGraph {
    /// 查找首个挂有指定 effect 的 bus（CC91/93 自动识别用）。
    pub fn find_bus_with_effect(&self, effect: u16) -> Option<usize>;
    // 旧 broadcast_effect_param / effect_id 已删除（D4 修订）：
    // 内置效果参数按（目标链 + 参数 id）精确投递，投递方法随阶段 B 定稿。
}
```

> `yinhe-mixer` 保持零内部依赖（现仅 serde）；param 用裸数字，语义由 `yinhe-audio` + `yinhe-dsp` 约定。

### 4.5 回放：AM 事件 → DSP

- `audio_model.rs::emit_automation_event`（flatten 边界，全链路唯一整数还原点）：
  - `Param { device: ChannelInstrument }` → 按 MIDI 绑定还原为原始整数事件（CC/PB/RPN）。
  - `Param { device: PluginInstrument { .. } }` → 占位事件 + `plugin_param`（值保持归一化，dispatch 走 `PluginEvent::ParamValue`）。
  - `CC`/`Rpn`/`Nrpn` → 低层事件还原。
- `engine_render.rs::dispatch_and_find_next`（CC 归音源）：
  - 内置音源通道（未挂插件乐器）：CC7/10/11/71/74 → 通道处理段（`ChannelDspChain::apply_cc`，在合成器输出后、insert 链前处理）；其余 CC 走合成器。
  - 插件音源通道：CC 原样透传插件（转 MIDI 字节），插件自行响应。
  - 插件参数（`Param{PluginInstrument}` 的占位事件）→ `PluginEvent::ParamValue`。
  - GM2 参数 → 按设备定位目标链后精确投递（阶段 B）。
- GPU：合成器输出同样经过通道处理段（`render_channel_dsp`，CPU 侧统一）；`to_gpu_control_event` 过滤 DSP 白名单 CC（GpuSynth 无通道处理，与 `channel_set` 硬切断对齐）。
- 投递粒度：块级（512 帧 ≈ 11.6ms @44.1k），参数变化稀疏，可接受。
- seek/chase：`compute_chase_states` 统一 `ChannelState::apply`（含 DSP 专有字段），`apply_chase_result` 把当前值写回内置音源通道处理段（已 dispatch 的 CC 跳过）。

### 4.6 回放：CC91/93 → bus send

- CC91/93 不在内置绑定表内，仍以原始 `CC` lane 存储（归一化值，§4.1）；dispatch 收到 CC91/CC93 时（仅当该通道存在对应事件）：
  1. 照常把 CC 发给合成器/插件（保持现状，兼容）。
  2. `mixer.find_bus_with_effect(GM2_REVERB/EFFECT_CHORUS)` 找到目标 bus。
  3. 设置该通道对目标 bus 的 send 量：`amount = value / 127.0`（线性；GM2 语义为线性发送量）。
- 若找不到目标 bus，仅执行第 1 步。
- send 更新需由 `MixerGraph` 提供 `set_send_amount(channel_dense, bus, amount)`（若现有 sends 结构没有对应条目则不创建，需按 bus 配置补条目；具体实现编码时定）。

### 4.7 导入：MIDI/SysEx → lane

- **MIDI 映射**（已落地）：所有 CC 保留低层 `CC` 事件；PB → `Param{ChannelInstrument, PITCH_BEND}`；RPN 0/1/2 → `Param{ChannelInstrument, 对应 id}`；其他 RPN/NRPN 保留原始变体。原 MSB/LSB 组装逻辑不变，组装出的整数统一归一化后写入 lane；归属通道取事件所在通道。
- 新增 `crates/yinhe-midi/src/gm2.rs`：
  - `parse_sysex(data: &[u8]) -> Option<(Gm2EffectUnit, u8 param, u8 value)>`（前缀/长度/值域校验）。
  - `build_sysex(unit, param, value) -> [u8; 12]`（不含 F0，含 F7，midly 直接可写）。
- `parser.rs`：
  - `:563`（每轨）与 `:265`（conductor pass）新增 `TrackEventKind::SysEx` 匹配；命中 GM2 参数 → `auto_events.push((AutomationTarget::Param { device, id }, AutomationEvent { tick, value: value / 127.0, shape: Step }))`；其余 SysEx 丢弃。
  - 归属轨：**存 master 轨**（TODO 待确认，见 §9-1）；实现为"导入后统一挂到 master 轨 lane"，与 SysEx 在文件中的物理轨无关。
  - 排序/分组复用现有 `auto_events.sort + group_automation_events`（`parser.rs:632/640`）。
- `ensure_master_track`：导入流程末尾确保 master 轨存在（追加末尾）。

### 4.8 导出：lane → MIDI/SysEx + master 展开

`writer.rs` 改动：

1. **MIDI 映射**：`Param` → 绑定的 MIDI 消息（边界还原整数，§4.1）；`CC`/`Rpn`/`Nrpn` 原样写出；第三方插件参数（无内置绑定）跳过（与旧 `PluginParam` 行为一致）。
2. `write_with_options`：先按 `strip_empty_tracks` 确定实际写出的 MIDI 轨集合；收集去重 `(port, channel)`；汇总 master lanes；master 轨跳过自身 SMF track 生成。
3. 抽公共 `push_lanes`（供普通轨与 master 展开复用，自动获得 Curve 插值/rpn_full 行为）。
4. 每个目标轨：notes 之后、自身 lanes 之前注入 master MIDI lanes（通道自身覆盖 master）。
5. `push_lane_event` 新增 GM2 参数分支：生成 SysEx（借用字节用现有 `leak_bytes` 模式），只注入到第一条实际写出的 MIDI 轨（去重防止重复）。
6. `MidiExportOptions` 不加开关（Master 轨存在即展开）。

### 4.9 混音台与 UI 接入

**持久化**：

- `PluginFormat` 新增 `Builtin`；内置效果器 `InsertRef { plugin_path: 空, plugin_id: "gm2_reverb"/"gm2_chorus", format: Builtin, state: None, .. }`。
- `MIXER_SECTION_VERSION` 升 6（新增 Builtin 变体）；旧版混音段的解码分支随容器拒绝策略一并处理（见下条）。
- `.yin` 容器版本 6 → 7（`yinhe-yin/src/lib.rs::VERSION`）：`AutomationTarget` 序列化变更，旧档直接拒绝加载（D23），不提供迁移。
- 参数不存 state（lane 是唯一真相，§4.10）。

**UI 流程**（复用 `PluginInstance` 体系）：

- `PluginInstance` 新增 `Builtin` 变体，实现 `name/id/param_list/get_param_value/value_to_text/save_state/load_state/param_queue` 接口（参数面板与保存流程整体复用）。
- `MixerRack::load_plugin` 增加"无路径内置加载"分支；`activate_slot` 直接构造 `Gm2Reverb/Gm2Chorus`（内部创建 `Arc<ParamQueue>` + `Arc<AtomicBool>` bypass）；`on_returns` 增加内置类型的 downcast 分支（否则 `sent` 卡死）。
- `plugin_picker`：顶部新增"内置效果器"分组（固定两项：GM2 Reverb、GM2 Chorus），与扫描插件并列。
- 内置效果器**没有原生 GUI**（`toggle_gui` 对内置禁用或打开参数面板）。

### 4.10 参数面板与 lane 编辑

- 参数面板显示 7 个 GM2 参数的当前值：取自 **master 轨 lane 在播放头 tick 的值**（`value_at`），无事件时用 GM2 默认值。
- 显示/输入按 §4.1 换算：lane 存归一化值，面板显示原始值（`round(v × 上限)`，如 Volume=100、Reverb Type=4）；拖动结果反向归一化后写回。
- 拖动滑块 =
  1. `Document::add_automation_event`（master 轨，当前播放头 tick，归一化值）；
  2. 通过现有 undo 体系记录；
  3. 触发重 flatten，引擎块级生效。
- Type 参数用下拉（枚举值显示名称），其余为原始值域滑块（0..127 / 0..8 / 0..5）。
- 内置 DSP 模块（ChannelDsp）参数面板同理：lane 目标为 `Param { device: ChannelDsp { channel }, id }`，显示 Volume/Expression/Pan/Cutoff/Resonance 原始值。
- 也可通过现有 PR/AR 自动化面板直接编辑 lane（两条编辑路径共用模型）。

### 4.11 GM2 参数 id 约定

| 效果（设备） | param id | 参数 |
|---|---|---|
| GM2 Reverb | 0 | Type |
| GM2 Reverb | 1 | Time |
| GM2 Chorus | 0 | Type |
| GM2 Chorus | 1 | Mod Rate |
| GM2 Chorus | 2 | Mod Depth |
| GM2 Chorus | 3 | Feedback |
| GM2 Chorus | 4 | Send to Reverb |

（与 §3.2 ParamID 对齐；后续 XG/GS 用不同设备/参数 id 空间扩展。旧 effect_id 广播表已废弃，见 D4。）

---

## 五、CC 子模块化与 xsynth 精简路线（阶段 A，已落地）

### 5.1 事实依据：xsynth 的 CC 分层

xsynth-core 0.4 的 `VoiceChannel::apply_channel_effects`（`channel/mod.rs:154-190`）证明以下 CC 本来就是**通道级音频处理**，不是 per-voice，可无损迁移：

| CC | 参数 | xsynth 实现 | 迁移可行性 |
|---|---|---|---|
| 7 | Volume | `out *= (volume)^2`（通道输出增益） | 精确等价 |
| 11 | Expression | 与 Volume 相乘后平方 | 精确等价 |
| 10 | Pan | 通道输出等功率声像 | 精确等价 |
| 74 | Cutoff | 通道输出 `MultiChannelBiQuad` 低通（`value<64` 启用，查表映射） | 精确等价（映射表可复刻） |
| 71 | Resonance | 同上 biquad 的 Q（`value>64` 启用） | 精确等价 |

> CC8（Balance，xsynth 也当 Pan）**不纳入白名单**（用户不采用该 CC），继续走原路径。

以下 CC 必须在合成器/事件层处理，**不迁移**（用户所说"一定要对音源动刀子"）：

| CC | 参数 | 原因 |
|---|---|---|
| 72/73/75 | Release/Attack/Decay | voice 包络 |
| 64/66/67 | Sustain/Sostenuto/Soft | 事件层（note off 时机） |
| 1/76/77/78 | Modulation/Vibrato | voice 音高/LFO 调制 |
| 5/65/84 | Portamento | voice 层滑音 |
| 0/32 | Bank Select | 音色选择 |
| 120/121/123 | All Sound Off/Reset/All Notes Off | 事件层 |
| PitchBend / RPN | 调音 | voice 音高 |

> SF2/SFZ 音色自带的 filter（`cutoff`/`resonance` opcode）与 voice 包络属 voice 级，继续由 xsynth 处理；CC71/74 的通道滤波迁移后与之串联，不冲突。

### 5.2 模块清单

| 模块 | 接管 CC | 语义 |
|---|---|---|
| `ChannelGain` | 7, 11 | `(vol/128 * expr/128)^2` 通道增益，10ms 参数平滑 |
| `ChannelPan` | 10 | 等功率声像（`pan=CC10/128`，与 xsynth 一致） |
| `ChannelFilter` | 71, 74 | RBJ cookbook 低通（cutoff 复刻 xsynth FREQS 键频表，Q 复刻 `10^((v-64)/48)×1/√2`），块级系数更新 |

- 全部实现 `InsertProcessor`，可在任意通道 insert 链中与 CLAP/VST3 效果任意排列。
- CC91/93 不做成模块（它是混音台 send 语义，见 §4.6）。
- 每通道可挂多个/不挂/重复挂（重复挂会叠加处理，由用户自行负责）。
- 参数身份走统一参数模型（§4.1）：`Param { device, id }`；旧 `effect_id` 广播机制已废弃（D4 修订）。

### 5.3 分发机制（CC 归音源）

1. **模块声明与接收**（不变）：`InsertProcessor` 的
   ```rust
   /// 本处理器接管的 MIDI CC 号（默认空）。
   fn handled_ccs(&self) -> &'static [u8] { &[] }
   /// 接收被接管的 CC 值（0..127）。
   fn apply_cc(&mut self, _cc: u8, _value: u8) {}
   ```
2. **通道处理段**：`yinhe-audio` 的 `ChannelDspChain`（Gain/Pan/Filter 三个模块，复用 yinhe-dsp）作为**内置音源的一部分**，在合成器输出后、insert 链前处理 CC7/10/11/71/74。挂插件乐器的通道跳过（CC 透传插件）。
3. **两层互斥**：一个通道要么内置音源（处理段消费 CC）、要么插件音源（透传插件）——**不存在广播/双发**；外挂 insert 效果器不接收 CC。
4. **消费集合**：即 `DSP_CHANNEL_CCS`（= 模块 `handled_ccs` 并集，由 yinhe-dsp 测试锁定），不按 CC 号在 dispatch 里硬编码判断路由。
5. **chase 回填**：seek 后把 `ChannelState` 当前值写回通道处理段
   （与 xsynth 的 `skip` 语义一致，已被 dispatch 的不覆盖）。
6. **参数平滑**：Gain/Pan 内部 10ms 线性斜坡（`Smoothed`，参考 xsynth `ValueLerp`）；
   Filter 采用块级系数更新（CC 事件本身以块为粒度）。
7. **音源层参数**：`Sustain/ADSR/调音` 等属 `ChannelInstrument` 参数（§4.1），回放还原为 MIDI 后继续走 xsynth/乐器插件路径；`Portamento/Vibrato/Bank/PC` 等无内置绑定，保持原始变体。

> `DSP_CHANNEL_CCS`（[7, 10, 11, 71, 74]）用于通道处理段的消费集合、合成器硬切断（`channel_set.rs`）与模块一致性校验（`yinhe-dsp` 测试）。

### 5.4 自由组合语义与边界

- 模块顺序影响结果（如 Filter 在失真模块之后 = 对失真输出滤波；之前 = 先滤波再失真），这是用户要的自由度。
- CC 模块只对**通道 insert 链**生效；挂在 bus/master 上的模块不接收通道 CC（CC 通道级语义）。
- 通道挂了乐器插件（CLAP/VST3）时：CC 原样透传插件，通道处理段跳过（两层互斥，不会双重处理）。
- 迁移期可以让部分通道挂模块、部分通道不挂，逐个试听对比。

### 5.5 迁移路线与验收

1. 阶段 A（本阶段）已落地，不依赖 GM2；GM2 效果（阶段 B）在统一参数模型（§4.1）之上继续。
2. 按 `ChannelGain → ChannelPan → ChannelFilter` 顺序迁移（从简单到复杂）。
3. 验收：同一 MIDI 文件在"xsynth 处理"与"模块处理"两种配置下 A/B 对比，音量/声像/滤波听感一致（参数语义对即可，不要求样本级一致）。
4. 全部通道迁移完成后，xsynth 侧只剩采样播放 + voice 级事件；远期再评估 fork/替换 xsynth 为纯采样器（保持 ADSR/音高处理）。

### 5.6 yinhe-synth 精简（已完成）

GPU 合成器（`GpuSynth`）已按"只做音源层"精简：

- **移除**：CC7/10/11/71/74 的通道音量/声像/滤波处理（含 shader 内的通道渐变与
  `GpuVoiceState.ch_vol/ch_expr/ch_pan`）、通道级 CC74/71 biquad（`apply_cutoff_filter`）、
  内部 `VolumeLimiter` 限幅、`ChannelState` 的对应状态与 `ValueLerp`。
- **保留**：采样播放、7 阶段 ADSR 包络、pitch bend/RPN 调音、damper（CC64）、
  ADSR CC（72/73）、bank/program，以及**音色自带**的 per-voice biquad（SFZ/SF2 filter）。
- **限幅归属**：`VolumeLimiter` 移至 `yinhe-dsp::dsp::limiter`，由 yinhe-audio 在
  最终输出（实时渲染与两条导出路径）统一调用；GPU 导出循环补上限幅。
- **事件过滤**：`to_gpu_control_event` 丢弃 DSP 白名单 CC，不再进入 GPU 事件流；
  CPU 路径的 `ChannelSet::send_event` 同样硬切断。
- parity 测试的 `cc_plan` 只保留音源层 CC；多端口折叠回归测试改用 PitchBend。

### 5.7 GPU 与 CPU 的分工

**结论：GPU 只做高并发发声（voice），效果器链保持 CPU；GPU 输出已接入混音台（D10 完成）。**

| 维度 | voice 渲染 | 效果器链 |
|---|---|---|
| 数据规模 | 音符数 × 每 voice 独立 | 通道级混合后音频（512 帧 × 2ch × N 通道），小几个数量级 |
| 并行性 | 海量独立任务，GPU 理想场景 | 链内串行依赖，GPU 并行优势有限 |
| 与外部插件混合 | 无关 | 链里有 CLAP/VST3 就必须 CPU，无法 GPU/CPU 混合链 |
| 状态/参数 | 每 voice 独立状态 | 延迟线/滤波状态/参数平滑，GPU 化需 CPU↔GPU 同步，复杂且延迟高 |

**GPU 接入混音台（已完成）**：

- `GpuSynth::render_to_mixer` 把 per-channel 输出去交错写入混音台 planar 通道缓冲
  （覆盖写 dense 0..32，其余清零）；之后与 CPU 路径共用
  `render_instruments` / `render_audio_tracks` / `mixer.process()`（insert 效果器、
  总线、推子、PDC 全部生效）。
- 通道上限：GPU 合成器支持 32 个 dense 槽位（2 个 MIDI 端口）；
  `load_dense_soundfonts` 越界返回错误（不再 `% 32` 折叠），事件构建过滤 dense ≥ 32。
- 插件乐器通道的音符/CC 由 CPU dispatch 喂插件，不进 GPU 事件列表；
  GPU 模式下 dispatch 不再向 xsynth 发送事件。
- `render_idle` 的 GPU 提前返回删除：空闲统一走混音台（插件尾音/GUI 键盘正常）。
- 导出统一：删除独立的 `export_wav_gpu` 路径，导出复用实时引擎
  （GPU 模式导出同样经过混音台与效果器链）。
- 限幅统一在 `audio_renderer` 最终输出（`yinhe_dsp::dsp::limiter`）。

- 若未来实测效果器链成为瓶颈，再评估"纯内置链全 GPU"。
- CPU 侧优化空间（按需再做）：静音/空通道跳过效果器处理、通道间并行（rayon）。

### 5.8 GM2 CC 覆盖差距与补齐路线

xsynth-core 0.4 实际处理的 CC：`0, 6, 7, 8, 10, 11, 38, 64, 71, 72, 73, 74, 100, 101, 120, 121, 123`（+RPN 0/1/2）。GM2 要求但缺失的部分：

| 类别 | 缺失 CC | 归属与计划 |
|---|---|---|
| 通道级 send | 91/93（Reverb/Chorus Send）、94（Variation Send） | 91/93 属阶段 B（§4.6）；94 随 XG/GS（阶段 C） |
| 通道级 | 标准 CC8 Balance（xsynth 把 8 当 Pan） | 不采用（用户决定，D16） |
| voice 级 | 1 Modulation、5/65/84 Portamento、66 Sostenuto、67 Soft、75 Decay、76/77/78 Vibrato、88 High-Res Velocity、96/97 Data Inc/Dec、98/99 NRPN | 远期（随 xsynth 精简/替换评估） |
| 音色选择 | 32 Bank LSB | xsynth/采样器侧 |

- 迁移期原则：**先迁移 xsynth 已有能力（不回归），通道级缺失可顺带补齐，voice 级缺失不承诺**。
- 两套合成实现的 CC 覆盖也不完全一致（`gpu_synth.rs` 与 xsynth），迁移与测试以 xsynth（CPU）为基准。

### 5.9 GPU DSP 参考点

- `crates/yinhe-synth/src/synth/filter.rs::biquad_coeffs`：CPU biquad 系数计算，`ChannelFilter` 可参考/复用（复用方式见 §10-6；两选项都不引入 wgpu 依赖）。
- `crates/yinhe-synth/src/gpu_synth.rs::ValueLerp`：10ms 参数平滑实现（私有类型，按模式自实现），`ChannelGain/Pan/Filter` 参考。
- GPU 合成器（`GpuSynth`）已接入混音台（§5.7），其事件流按 §5.6 丢弃 DSP 白名单 CC，DSP 模块照常生效；CPU/GPU 的 voice 级差异属已知限制。

---

## 六、改动清单（按 crate）

### yinhe-types
- [x] `automation.rs`：统一参数模型（§4.1）——`AutomationTarget::{Param, CC, Rpn, Nrpn, Tempo}`、`ParamDevice`、内置参数绑定表、归一化换算 helper、方法分支与单测。
- [x] `lib.rs`：导出新类型（`ParamDevice`/`MidiBinding`/`BuiltinParamInfo`/两张绑定表与查询函数）。

### yinhe-core
- [ ] `model.rs`：`TrackKind::Master`、`ensure_master_track` 辅助；`TrackData::is_master()`。
- [ ] 非音符轨判定 helper（conductor + master 统一）。

### yinhe-mixer
- [ ] `graph.rs`：`find_bus_with_effect`、`set_send_amount`（内置效果参数按目标链 + 参数 id 投递，不广播）。
- [x] `graph.rs`（阶段 A）：`handled_ccs`/`apply_cc` 默认方法与 `broadcast_channel_cc`。
- [x] `params.rs`：`PluginFormat::Builtin`（阶段 A 已落地，内置效果器机架在用）。

### yinhe-dsp（新）
- [ ] crate 骨架 + `Gm2Reverb`/`Gm2Chorus` + DSP 基础件 + 单测。
- [x] `registry.rs`：`EffectParamInfo` 与 `yinhe-types` 绑定表一致性测试（名称/cc 绑定/默认值，`params_match_types_binding_table`）。
- [x] 阶段 A：`ChannelGain`/`ChannelPan`/`ChannelFilter` 已实现；biquad 各持一份（见 §10-6）；`Smoothed` 按 `ValueLerp` 模式自实现。

### yinhe-midi
- [x] `parser.rs`：CC 保留低层事件 + PB/RPN 0/1/2 ↔ `Param`（已落地）+ `writer.rs` `Param` 还原（含 roundtrip 测试）。
- [ ] `gm2.rs`（解析/生成）、`parser.rs` SysEx 分支、`writer.rs` master 展开 + SysEx 还原。

### yinhe-audio
- [x] `audio_model.rs`：`Param` 边界还原/占位（阶段 A′）。
- [ ] `audio_model.rs`：master 复制（阶段 B）。
- [x] `engine_render.rs`：dispatch CC 归音源（内置=通道处理段；插件=透传，阶段 A′）。
- [ ] `engine_render.rs`：GM2 精确投递、CC91/93（阶段 B）。
- [x] `engine_state.rs`：chase 回填写回通道处理段（`DSP_CHANNEL_CCS` + skip 语义）。
- [ ] `engine_state.rs`：`skip_track` 不跳 master（阶段 B）。
- [ ] `channel_layout.rs`：kind 过滤修正。
- [ ] `engine_mixer.rs`/`spawn.rs`：内置效果器实例的回收识别（如需）。
- [x] 阶段 A：`channel.rs` chase 事件流不再发这些 CC；`preview_engine` 统一 `apply`；GPU 过滤保持白名单对齐 `channel_set` 硬切断。
- [x] 通道处理回归音源（A′ 收尾）：`ChannelDspChain`（复用 yinhe-dsp）在合成器输出后、insert 链前处理；删除 `broadcast_channel_cc`/`channel_subscribed_ccs` 与外挂订阅。

### yinhe-editor-core
- [x] `clipboard_file.rs`：target 二进制 tag 重做（Param 三 device/CC/Rpn/Nrpn/Tempo，往返测试）。
- [ ] `track_ops.rs`：`used_channels` 过滤；master 保护（不可删/移）。
- [ ] 文档编辑：master 轨 lane 编辑允许、音符操作拒绝。

### yinhe-egui
- [x] `mix/`：`PluginInstance::Builtin`、picker 内置分组、activate/on_returns 分支、参数面板（阶段 A 已落地）。
- [ ] `arrange/`：master badge/颜色、AM 展开、右键菜单保护、拖拽保护。
- [ ] `piano_view/`：master 不可写音符。
- [x] `chrome/dock_bar.rs`：音源面板合并通道处理 + 音源参数（12 项，CC lane 反查）；内置效果器隐藏（picker 移除、加载迁移清理）。
- [x] `mix.rs`：删除自动挂默认 DSP 链；加载时迁移清理遗留内置 insert。
- [ ] `event_browser`：master 节点/排除。
- [x] `app/audio.rs`、`app/plugin_automation.rs` 等静默点（新变体穷尽 match 已全部跟进）。
- [x] 自动化面板常量与原始值显示适配（`to_display_value`/`from_display_value` 换算）。

### yinhe-yin
- [x] `lib.rs`/`container.rs`：`.yin` VERSION 6 → 7（旧档拒绝，D23）；mapping/io 随 serde 自动；`.yin` 往返测试已适配。

### yinhe-wgpu
- [x] `automation/prepare.rs`：`target_hash` 新分支（命名空间隔离 + 防撞车测试）。

---

## 七、测试计划

| 层 | 测试 |
|---|---|
| yinhe-types | 新变体方法（值域/默认值/显示名）、lane 唯一性；统一参数模型：绑定表完备（id 唯一、覆盖全部内置参数）、归一化全值域往返（0..127 / 0..16383 边界无损）、Tempo/velocity/gate 不归一化 |
| yinhe-midi | 导入映射：已知 CC/PB/RPN/NRPN → `Param`、未知 → 原始变体；导出还原：`Param` → MIDI 消息；SysEx 解析/生成单测（7 参数 × 边界值 + 非 GM2 丢弃）；roundtrip：构造含 SysEx 的 SMF → 模型 → 导出 → 字节比对；master 展开（2 通道 + master CC，断言两通道都有且同 tick 通道自身覆盖）；空轨 strip 后的展开集合 |
| yinhe-audio | flatten：master CC 复制到所有激活通道；`Param` 边界还原/占位事件；dispatch 按设备投递到 mixer（用测试用 InsertProcessor 记录）；CC91/93 设 send；seek chase 回填 |
| yinhe-audio（阶段 A/A′） | CC 归音源：内置通道处理段消费 CC7/10/11/71/74、插件透传；chase 回填写回处理段；`events_to_send` 不再包含 DSP CC |
| yinhe-dsp | 各效果器：参数生效、无 NaN/爆音、reset 清尾、块长/采样率无关性 |
| yinhe-dsp（阶段 A） | Gain/Pan/Filter 单测（默认值、CC 生效、平滑、稳定性）；与 xsynth `apply_channel_effects` 的 A/B 听感对比（手工） |
| yinhe-editor-core | master 轨保护（删除/移动/音符）、剪贴板序列化 tag（统一参数变体） |
| yinhe-yin | `.yin` 往返（含 master 轨与 GM2 参数 lane）；旧版本（v6）拒绝加载 |
| 手工验证 | 挂模块后 A/B 听感对比；阶段 B：自造 GM2 MIDI → 导入 → 挂 Gm2Reverb 到 bus → 听感；导出 → 外部工具确认 SysEx |

---

## 八、实施顺序

**阶段 A：CC 模块化（已完成）**

1. ~~P1 yinhe-dsp crate + `ChannelGain`~~ ✅
2. ~~P2 混音台接入：`PluginFormat::Builtin` + `PluginInstance::Builtin` + `BuiltinInsert` + picker~~ ✅
3. ~~P3 分发机制：白名单直发 + `broadcast_channel_cc` + chase 回填~~ ✅
4. ~~P4 `ChannelPan` / `ChannelFilter`~~ ✅
5. 后续（P5）：手工 A/B 验收、按需补 dispatch 单测、参数面板对内置模块的说明文案。

**阶段 A′：统一参数模型（已落地）**

6. ~~P5.1 模型层：`AutomationTarget::Param`/`ParamDevice` + 绑定表（`yinhe-types`）+ 所有穷举 match 跟进 + 单测~~ ✅
7. ~~P5.2 映射层：导入/导出 CC/PB/RPN/NRPN ↔ `Param`（无损往返）；`.yin` 升 v7（旧档拒绝）~~ ✅
8. ~~P5.3 边界与 UI：回放边界还原整数投递；参数面板/事件编辑器原始值换算；dispatch/chase 跟进；通道处理回归音源（ChannelDspChain）~~ ✅

**阶段 B：GM2 效果与 SysEx 全链（下一步）**

9. **P6 GM2 参数设备**：GM2 设备变体定义 + 参数 id 约定（§4.2/§4.11）+ `TrackKind::Master` + 所有穷举 match 跟进。
10. **P7 GM2 DSP**：`Gm2Reverb`/`Gm2Chorus` + DSP 单测。
11. **P8 SysEx 全链**：导入解析 + 导出生成 + master 轨 lane 存放 + 导出展开。
12. **P9 回放驱动**：GM2 参数精确投递 + CC91/93 send + chase。
13. **P10 master 轨 UI 完善**：AR/PR/事件浏览器/保护逻辑。
14. **P11 验收**：真实/自造 MIDI 文件端到端 + release 构建。

**阶段 C（远期）**：XG/GS 效果扩展；xsynth 精简评估（替换为纯采样器）。

每个阶段完成后跑 `cargo fmt`、涉及 crate 的 `clippy`/`test`，并按 AGENTS.md 分步 commit。

---

## 九、后续与已知限制

1. **GPU 合成器**：已接入混音台（见 §5.7）。32 通道上限保留（2 端口）；超出通道在 GPU 模式下静音。
2. **Send to Reverb**：第一批只存与导出，回放不生效。
3. **XG/GS 效果**：阶段 C 按同一模式扩展（设备/参数 id 空间、SysEx 前缀不同）。
4. **MIDI 效果器**（琶音器等）：需要新的 MIDI 事件链，当前 `InsertProcessor` 设计未覆盖。
5. **效果器参数自动化对插件 insert**：本规格打通了内置效果的 AM；外部插件 insert 的 AM 仍未支持。
6. **master 轨与混音台主输出**：无直接关联；master 轨 CC 不影响 `MixerParams.master.gain`。
7. **xsynth 源码不可改**：它是 crates.io 依赖，迁移只能通过"不发送对应 CC"绕过其通道处理；远期精简需评估 fork 或自研采样器。
8. **第三方插件参数**：走 `PluginInstrument`（可自动化），无内置 MIDI 绑定 → 不导出 MIDI；id 空间由插件自身定义（切换插件后既有 lane 的语义随之变化）。
9. **`.yin` v6 及更早**：统一参数模型落地后容器版本升 7，旧档不再读取（D23）；旧工程重新导入 MIDI。

---

## 十、待确认项

1. **GM2 参数 lane 存放**：本稿改为 **master 轨**（原 D5 调研时选择"跟随来源轨"，但当时尚无 master 轨）。若坚持跟随来源轨，导出 SysEx 的写出位置需按来源轨处理。
2. **导出时 SysEx 写在哪条轨**：本稿为"第一条实际写出的 MIDI 轨"。
3. **master 轨是否允许用户删除**：本稿为"自动 ensure、不可删除"。
4. **conductor 改名**：本稿建议 conductor badge 从 `"Master"` 改为 `"Conductor"`。
5. **GM2 参数设备**：bus/master 级内置效果器的 `ParamDevice` 变体命名（及同一效果挂多个实例时的寻址）随阶段 B 定稿；本稿只锁定"设备 + 参数 id、不广播"原则。
6. ~~CC 模块与 xsynth 的 biquad 复用~~（已定：各自一份）：
   `yinhe-dsp::dsp::biquad`（ChannelFilter 的通道低通）与
   `yinhe-synth::synth::filter`（per-voice 音色 filter，音源层保留）
   用途已分离，各持约 30 行 RBJ 系数实现，注释互指；都不引入 wgpu 依赖。
