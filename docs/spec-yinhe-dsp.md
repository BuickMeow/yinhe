# spec-yinhe-dsp：内置效果器、CC 模块化与 GM2 全链

> 状态：设计稿（待评审）
> 范围：阶段 A（当前，已落地基础设施）= xsynth 通道级 DSP 向 yinhe-dsp 模块迁移（CC 模块化）；阶段 B = GM2 Reverb/Chorus + master 轨 + SysEx 全链
> 关联：`spec-xsynth-integration.md`（效果器链预留架构）、`docs/GMLevel2.xml`（参数词典）

---

## 一、背景与目标

yinhe 已有效果器链（`yinhe-mixer` 的 `InsertProcessor`，每通道/bus/master 一条链），但目前只能挂外部 CLAP/VST3 插件，没有任何内置 DSP。本规格定义新 crate **yinhe-dsp**：内置效果器模块，像插件一样自由拼搭。

**长期路线（本节是最重要的方向约定）**：

```
阶段 A（当前，已完成基础设施）：xsynth 通道级 DSP 迁移为 yinhe-dsp CC 模块
   ↓  ChannelGain/Pan/Filter 已实现，CC 直发 DSP 链（见 §5）
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
2. 导入 MIDI 时：效果参数 SysEx → 自动化参数；导出 MIDI 时：自动化参数 → SysEx。
3. 效果器像模块一样自由拼搭，纯手动挂载，不自动创建实例。
4. 效果参数事件**广播**给工程内所有同型效果器实例。
5. 参数面板手动调参 = 写自动化 lane（lane 是唯一真相）。
6. 阶段 B 能力：AM 回放驱动 DSP、CC91/93 打通 send、导出生成 SysEx。
7. 顺带支持 **master 轨**（挂全局 CC，导出时展开到所有 MIDI 通道）。
8. 阶段 A 能力（已落地）：CC 模块接管 xsynth 的通道级处理，用户逐通道迁移试听。

### 决策记录（已确认）

| # | 决策 | 结论 |
|---|---|---|
| D1 | yinhe-dsp 职责 | 音频 DSP；SysEx 仅在导入/导出层与自动化参数互转 |
| D2 | 第一批效果范围 | 只做 GM2 Reverb/Chorus（含全部 7 个参数） |
| D3 | 效果器拓扑 | 纯手动拼搭，导入不自动挂效果器 |
| D4 | 参数事件寻址 | 广播给所有同型实例 |
| D5 | 调参行为 | 写 lane（在播放头 tick），lane 是唯一真相 |
| D6 | SysEx 全链 | 效果参数解析+生成；开关类消息（GM1/GM2 System On 等）丢弃 |
| D7 | 未识别 SysEx | 丢弃（与现状一致） |
| D8 | 算法基准 | 参数语义对即可，不要求逐样本复刻硬件 |
| D9 | 复用 UI 体系 | `PluginInstance` 新增 Builtin 变体，复用参数面板/旁通/回收流程 |
| D10 | GPU 绕过混音台 | 后续单独任务，本规格只留记录 |
| D11 | master 轨 | 本规格一起做（新增 `TrackKind::Master`） |
| D12 | CC91/93 目标 bus | 自动识别（bus 链上挂 Gm2Reverb/Gm2Chorus 者） |
| D13 | 效果器优先级 | GM2 完成后才启动 XG/GS |
| D14 | DSP 迁移方向 | xsynth 通道级 DSP 逐步迁至 yinhe-dsp CC 模块，最终 xsynth 只剩采样播放 + voice 级参数 |
| D15 | 接管语义 | 白名单 CC（7/10/11/71/74）直接进 DSP 链（广播给模块），不再下发合成器/乐器插件；不挂模块则该 CC 丢弃，无回退；不改 xsynth 源码 |
| D16 | CC 模块粒度 | 按功能组：Gain(7/11)、Pan(10)、Filter(71/74)；CC8 不采用 |
| D17 | CPU DSP 依赖 | yinhe-dsp 不依赖 wgpu/yinhe-synth；GPU 与 CPU 实现独立 |
| D18 | GPU/CPU 分工 | GPU 只做高并发 voice 发声；效果器链保持 CPU（数据量小、串行依赖、需与 CLAP/VST3 混合） |
| D19 | CC 补齐策略 | 先迁移 xsynth 已有通道级 CC；通道级缺失（94、标准 Balance）顺带补；voice 级缺失（Vibrato/Portamento 等）不承诺，属远期 |

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
- GPU 合成器路径（`GpuSynth`）有自己的 CC 状态机与滤波（`gpu_synth.rs`、`synth/filter.rs`），绕过混音台。

### 2.3 自动化与 MIDI 管线

- lane 模型：`AutomationTarget`（`crates/yinhe-types/src/automation.rs:137`）现有 6 变体；`AutomationLane` 存于 `TrackData.automation_lanes`，Tempo 存于 `ConductorData.tempo`。
- 导入 `crates/yinhe-midi/src/parser.rs`：SysEx 在 `:563`（每轨）与 `:265`（conductor pass）被丢弃；RPN/NRPN 解析流程可作参考。
- 导出 `crates/yinhe-midi/src/writer.rs`：不写 SysEx；`push_lane_event`（`:362`）对 `Tempo | PluginParam` 静默跳过；`sort_by_key` 稳定，同 tick 顺序 = push 顺序。
- 回放 `crates/yinhe-audio/src/audio_model.rs`：`flatten_automation_to_cc_events`（`:302`）把 lane 展平为 `SortedCC`；dispatch 在 `engine_render.rs:112`，`PluginParam` 仅路由到乐器插件。
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

### 4.1 数据模型：`AutomationTarget` 新变体

```rust
// crates/yinhe-types/src/automation.rs
pub enum Gm2EffectUnit { Reverb, Chorus }

pub enum AutomationTarget {
    // ...现有 6 个变体...
    /// GM2 效果参数（Global Parameter Control SysEx，见 spec-yinhe-dsp）。
    /// value 为 GM2 原生值（0..127，Type 为枚举值），无 14-bit、无中线。
    Gm2Effect { unit: Gm2EffectUnit, param: u8 },
}
```

约定：

- `param` 用 ParamID 常量（0=Type, 1=Time/Rate, ...），不把显示名编进 target（避免 `PluginParam { name }` 污染 Eq/Ord/Hash 的既有隐患）。
- `max_value()`：Reverb Type → 8，Chorus Type → 5，其余 → 127。
- `default_value()`：见 §3.2 默认列。
- `display_name()`：`"GM2 Reverb Type"`、`"GM2 Chorus Mod Depth"` 等（i18n 走现有文案体系）。
- `default_shape()`：Step；`is_14bit()`：false；`has_center_line()`：false。
- 新增变体必须补齐的穷举 match 清单（编译器强制）：
  `yinhe-types/automation.rs`（max/default/shape/display 4 处）、`yinhe-midi/writer.rs:370`、`yinhe-audio/audio_model.rs:442`、`yinhe-editor-core/clipboard_file.rs:245`（+`read_target` tag）、`yinhe-wgpu/automation/prepare.rs:34`、`yinhe-egui/right_panel/event_browser/tree.rs:313`、`detail.rs:1193`、`yinhe-midi/examples/cc_stat.rs`。

### 4.2 master 轨（`TrackKind::Master`）

**为什么需要 master 轨（CC 层级澄清）**：

- MIDI 1.0 的 128 个 CC **全部是通道级**，标准中不存在"master CC"；设备级/全局参数走 SysEx（GM2 效果参数、Master Volume/Tuning 等）。
- 混响是两层组合：**CC91 是通道级送量**（每通道送多少进混响），**混响参数（Type/Time）是设备级**（唯一一台）。本设计分别对应 §4.5（CC91 → 通道 bus send）与本节 Gm2Effect lane。
- master 轨承载的 CC **不是标准语义**，而是"用户想让所有通道统一收到同一 CC"的工程内便利工具，回放广播、导出展开；是否使用完全由用户决定。

**模型约定**：

- `TrackKind` 增加 `Master`；`TrackData.port/channel = 0/0`（不占 MIDI 通道命名空间）；`notes`、`program_change`、`audio_clips` 恒空；只允许 `automation_lanes`（CC/PB/RPN/NRPN/Gm2Effect）。
- 工程内**至多一条**，追加在 `tracks` **末尾**（避免 conductor 式插入 0 引发的全量索引重映射；master 轨加入后 `lane.track` 天然正确）。
- 加载/导入后自动 ensure（类似 conductor），不提供手动新建/删除入口。
- **Tempo 仍归 conductor**；master 不承载 Tempo。

**命名冲突处理**：conductor 轨现在 badge 显示 `"Master"`（`arrange/track_panel.rs:349`），需改名（建议 `"Conductor"`），新轨显示 `"Master"`；混音台右侧总线条已是"主输出"，三者在文案上区分。

**播放语义**：

- master 的 MIDI lane（CC/PB/RPN/NRPN）在 `flatten_automation_to_cc_events` 中**复制到所有激活的 MIDI 通道**（每个通道生成一份 `SortedCC`），使回放等价于导出展开。
- master 的 `Gm2Effect` lane **不做通道复制**，走效果参数广播（§4.4）。
- `engine_state.rs:42` 的 `skip_track` 不能把 master 跳过（该轨无音符属正常）。
- master lane 的 AM M/S 试听按现有 `(track, target)` 掩码工作。

**导出语义**：

- master 轨**不生成自己的 SMF track**。
- 其 MIDI lane 展开到**本次实际写出的 MIDI 轨**（`strip_empty_tracks` 判定之后）的通道；同一 `(port, channel)` 重复时只注入第一条轨。
- 同 tick 顺序：**master 先 push、目标轨自身 lane 后 push** → 通道自身事件覆盖 master（"master 是全局基准，通道可覆盖"）。
- master 的 `Gm2Effect` lane 生成 SysEx，**只写一次**，写到第一条实际写出的 MIDI 轨。

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

### 4.3 yinhe-dsp crate

```
crates/yinhe-dsp/
  Cargo.toml            # 依赖：yinhe-mixer（InsertProcessor）、yinhe-types、serde；不依赖 audio/egui
  src/lib.rs
  src/gm2.rs            # Gm2EffectUnit 相关常量/值域/名称（与 yinhe-types 共享或转发）
  src/gm2/reverb.rs     # Gm2Reverb（阶段 B）
  src/gm2/chorus.rs     # Gm2Chorus（阶段 B）
  src/cc/gain.rs        # ChannelGain  （阶段 A，CC7/11，已实现）
  src/cc/pan.rs         # ChannelPan   （阶段 A，CC10，已实现）
  src/cc/filter.rs      # ChannelFilter（阶段 A，CC71/74，已实现）
  src/dsp/delay.rs      # 延迟线
  src/dsp/comb.rs       # 梳状/全通（混响用）
  src/dsp/lfo.rs        # 正弦 LFO
  src/dsp/onepole.rs    # 单极低通（平滑/阻尼）
  src/dsp/biquad.rs     # biquad 系数/状态（阶段 A；与 yinhe-synth 存在复用点，见 §10-5）
```

**DSP 设计（参数语义对齐，音质自研）**：

- `Gm2Reverb`：Freeverb 类结构（8 comb + 4 allpass，左右微失谐）。Type 映射预延迟/衰减/阻尼组合；Time(0..127) 映射反馈系数（0.36s..9s 对数语义）。干湿比固定为"全湿输出由 send 量控制"还是内置 dry/wet？
  - 采用 **insert 全湿**（输出即湿声，干声由 send 量决定）会需要并联结构；现有链是串联，**改为内置固定 dry/wet 比例**（如 dry 0.7 / wet 1.0），语义上近似 GM2 音源（send 越大混响越多，因为 send 是额外叠加）。
  - 结论：Gm2Reverb 输出 = dry + wet（wet 由 CC91/bus send 在下游控制总量），实现简单且听感可用。
- `Gm2Chorus`：2 条调制延迟线（左右反相 LFO）+ 反馈。Type 映射预设（Chorus1-4 不同 Rate/Depth/相位，FB Chorus 加反馈，Flanger 短延迟+高反馈）；Mod Rate/Depth/Feedback 直接映射。
- `Send to Reverb` 参数：**第一批只存 lane 与导出，不参与回放**（需要辅助发送路径，后续再做）。
- 参数接收：
  - 实时调参：内部 `Arc<ParamQueue>`（参数 id 0..4 对应 ParamID），process 开头 drain。
  - 广播事件：trait 新增默认方法（见 §4.4），按 effect id 过滤。
  - 参数平滑：块内一阶平滑（参考 `gpu_synth.rs` 的 `ValueLerp`），避免 zipper。
- 实时约束：缓冲构造时分配；`process` 零分配/零锁/不 panic；`reset()` 清空延迟线。

**trait 扩展（yinhe-mixer，零内部依赖原则不变）**：

```rust
// graph.rs
pub trait InsertProcessor: Send {
    // ...现有方法...
    /// 效果器类型标识（宿主广播参数时用于过滤）。u8/u16 由上层约定。
    fn effect_id(&self) -> Option<u16> { None }
    /// 宿主广播的效果参数：effect/param 为效果器自定义标识，value 归一化 0..1。
    fn apply_effect_param(&mut self, _effect: u16, _param: u16, _value: f32) {}
}

impl MixerGraph {
    /// 向所有 insert（通道/bus/master）广播效果参数。
    pub fn broadcast_effect_param(&mut self, effect: u16, param: u16, value: f32);
    /// 查找首个挂有指定 effect_id 的 bus（CC91/93 自动识别用）。
    pub fn find_bus_with_effect(&self, effect: u16) -> Option<usize>;
}
```

> `yinhe-mixer` 保持零内部依赖（现仅 serde）；effect/param 用裸数字，语义由 `yinhe-audio` + `yinhe-dsp` 约定。

### 4.4 回放：AM 事件 → DSP

- `audio_model.rs::emit_automation_event` 新增分支：`Gm2Effect` → 生成占位 `SortedCC`，带 `gm2_param: Option<(u8 unit, u8 param, f32 value)>`（仿 `plugin_param` 字段），channel 用哨兵（master 轨无通道）。
- `engine_render.rs::dispatch_and_find_next` 新增分支：`gm2_param` → `self.mixer.broadcast_effect_param(effect, param, value/127.0)`。
- 广播粒度：块级（512 帧 ≈ 11.6ms @44.1k），参数变化稀疏，可接受。
- seek/chase：`compute_chase_states` 需要回填 GM2 参数的当前值（仿 `PluginParam` 分支），使 seek 后参数正确。

### 4.5 回放：CC91/93 → bus send

- dispatch 收到 CC91/CC93 时（仅当该通道存在对应事件）：
  1. 照常把 CC 发给合成器/插件（保持现状，兼容）。
  2. `mixer.find_bus_with_effect(GM2_REVERB/EFFECT_CHORUS)` 找到目标 bus。
  3. 设置该通道对目标 bus 的 send 量：`amount = value / 127.0`（线性；GM2 语义为线性发送量）。
- 若找不到目标 bus，仅执行第 1 步。
- send 更新需由 `MixerGraph` 提供 `set_send_amount(channel_dense, bus, amount)`（若现有 sends 结构没有对应条目则不创建，需按 bus 配置补条目；具体实现编码时定）。

### 4.6 导入：SysEx → lane

- 新增 `crates/yinhe-midi/src/gm2.rs`：
  - `parse_sysex(data: &[u8]) -> Option<(Gm2EffectUnit, u8 param, u8 value)>`（前缀/长度/值域校验）。
  - `build_sysex(unit, param, value) -> [u8; 12]`（不含 F0，含 F7，midly 直接可写）。
- `parser.rs`：
  - `:563`（每轨）与 `:265`（conductor pass）新增 `TrackEventKind::SysEx` 匹配；命中 GM2 参数 → `auto_events.push((AutomationTarget::Gm2Effect{..}, AutomationEvent{ tick, value, Step }))`；其余 SysEx 丢弃。
  - 归属轨：**存 master 轨**（TODO 待确认，见 §9-1）；实现为"导入后统一挂到 master 轨 lane"，与 SysEx 在文件中的物理轨无关。
  - 排序/分组复用现有 `auto_events.sort + group_automation_events`（`parser.rs:632/640`）。
- `ensure_master_track`：导入流程末尾确保 master 轨存在（追加末尾）。

### 4.7 导出：lane → SysEx + master 展开

`writer.rs` 改动：

1. `write_with_options`：先按 `strip_empty_tracks` 确定实际写出的 MIDI 轨集合；收集去重 `(port, channel)`；汇总 master lanes；master 轨跳过自身 SMF track 生成。
2. 抽公共 `push_lanes`（供普通轨与 master 展开复用，自动获得 Curve 插值/rpn_full 行为）。
3. 每个目标轨：notes 之后、自身 lanes 之前注入 master MIDI lanes（通道自身覆盖 master）。
4. `push_lane_event` 新增 `Gm2Effect` 分支：生成 SysEx（借用字节用现有 `leak_bytes` 模式），只注入到第一条实际写出的 MIDI 轨（去重防止重复）。
5. `MidiExportOptions` 不加开关（Master 轨存在即展开）。

### 4.8 混音台与 UI 接入

**持久化**：

- `PluginFormat` 新增 `Builtin`；内置效果器 `InsertRef { plugin_path: 空, plugin_id: "gm2_reverb"/"gm2_chorus", format: Builtin, state: None, .. }`。
- `MIXER_SECTION_VERSION` 升 6，保留旧版本迁移结构（旧版本无 Builtin，直接兼容读取；新增变体不破坏旧档）。
- 参数不存 state（lane 是唯一真相，§4.9）。

**UI 流程**（复用 `PluginInstance` 体系）：

- `PluginInstance` 新增 `Builtin` 变体，实现 `name/id/param_list/get_param_value/value_to_text/save_state/load_state/param_queue` 接口（参数面板与保存流程整体复用）。
- `MixerRack::load_plugin` 增加"无路径内置加载"分支；`activate_slot` 直接构造 `Gm2Reverb/Gm2Chorus`（内部创建 `Arc<ParamQueue>` + `Arc<AtomicBool>` bypass）；`on_returns` 增加内置类型的 downcast 分支（否则 `sent` 卡死）。
- `plugin_picker`：顶部新增"内置效果器"分组（固定两项：GM2 Reverb、GM2 Chorus），与扫描插件并列。
- 内置效果器**没有原生 GUI**（`toggle_gui` 对内置禁用或打开参数面板）。

### 4.9 参数面板与 lane 编辑

- 参数面板显示 7 个 GM2 参数的当前值：取自 **master 轨 lane 在播放头 tick 的值**（`value_at`），无事件时用 GM2 默认值。
- 拖动滑块 =
  1. `Document::add_automation_event`（master 轨，当前播放头 tick，值）；
  2. 通过现有 undo 体系记录；
  3. 触发重 flatten，引擎块级生效。
- Type 参数用下拉（枚举值显示名称），其余为 0..127 滑块。
- 也可通过现有 PR/AR 自动化面板直接编辑 lane（两条编辑路径共用模型）。

### 4.10 参数与 effect_id 约定

| effect_id | 效果器 | param 含义 |
|---|---|---|
| 0 | GM2 Reverb | 0=Type, 1=Time |
| 1 | GM2 Chorus | 0=Type, 1=Rate, 2=Depth, 3=Feedback, 4=SendToReverb |

（与 GM2 ParamID 对齐，后续 XG/GS 用不同 effect_id 空间扩展。）

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
- CC91/93 不做成模块（它是混音台 send 语义，见 §4.5）。
- 每通道可挂多个/不挂/重复挂（重复挂会叠加处理，由用户自行负责）。
- `effect_id` 广播标识属 GM2 效果参数阶段（阶段一），本阶段未引入。

### 5.3 分发机制（直发，无回退）

1. **模块声明与接收**：`InsertProcessor` 新增
   ```rust
   /// 本处理器接管的 MIDI CC 号（默认空）。挂在通道链上时生效。
   fn handled_ccs(&self) -> &'static [u8] { &[] }
   /// 接收被接管的 CC 值（0..127）。
   fn apply_cc(&mut self, _cc: u8, _value: u8) {}
   ```
2. **固定白名单**：`yinhe_dsp::cc::DSP_CHANNEL_CCS = [7, 10, 11, 71, 74]`，
   dispatch 对白名单内的 CC **直接** `MixerGraph::broadcast_channel_cc`，
   **不再下发合成器/乐器插件**（无"没挂模块就回退 xsynth"的兜底；不挂模块则该 CC 丢弃）。
3. **广播过滤**：`broadcast_channel_cc` 遍历通道 insert 链，只调用 `handled_ccs` 命中的模块。
4. **chase 回填**：seek 后，白名单 CC 的 chase 状态值广播给模块
   （与 xsynth 的 `skip` 语义一致，已被 dispatch 的不覆盖）。
5. **参数平滑**：Gain/Pan 内部 10ms 线性斜坡（`Smoothed`，参考 xsynth `ValueLerp`）；
   Filter 采用块级系数更新（CC 事件本身以块为粒度）。
6. **音源层 CC**：`ADSR/Sustain/Portamento/Vibrato/Bank/PC/RPN` 等继续走 xsynth/乐器插件路径。

### 5.4 自由组合语义与边界

- 模块顺序影响结果（如 Filter 在失真模块之后 = 对失真输出滤波；之前 = 先滤波再失真），这是用户要的自由度。
- CC 模块只对**通道 insert 链**生效；挂在 bus/master 上的模块不接收通道 CC（CC 通道级语义）。
- 通道挂了乐器插件（CLAP/VST3）时同样适用：白名单 CC **完全丢弃**（不转发、不改插件），插件当作该 CC 不存在（D15）。
- 迁移期可以让部分通道挂模块、部分通道不挂，逐个试听对比。

### 5.5 迁移路线与验收

1. 完成阶段一（GM2 Reverb/Chorus）后再启动本阶段——先验证 yinhe-dsp 模块基础设施。
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
| 通道级 send | 91/93（Reverb/Chorus Send）、94（Variation Send） | 91/93 属阶段 B（§4.5）；94 随 XG/GS（阶段 C） |
| 通道级 | 标准 CC8 Balance（xsynth 把 8 当 Pan） | 不采用（用户决定，D16） |
| voice 级 | 1 Modulation、5/65/84 Portamento、66 Sostenuto、67 Soft、75 Decay、76/77/78 Vibrato、88 High-Res Velocity、96/97 Data Inc/Dec、98/99 NRPN | 远期（随 xsynth 精简/替换评估） |
| 音色选择 | 32 Bank LSB | xsynth/采样器侧 |

- 迁移期原则：**先迁移 xsynth 已有能力（不回归），通道级缺失可顺带补齐，voice 级缺失不承诺**。
- 两套合成实现的 CC 覆盖也不完全一致（`gpu_synth.rs` 与 xsynth），迁移与测试以 xsynth（CPU）为基准。

### 5.9 GPU DSP 参考点

- `crates/yinhe-synth/src/synth/filter.rs::biquad_coeffs`：CPU biquad 系数计算，`ChannelFilter` 可参考/复用（复用方式见 §10-5；两选项都不引入 wgpu 依赖）。
- `crates/yinhe-synth/src/gpu_synth.rs::ValueLerp`：10ms 参数平滑实现（私有类型，按模式自实现），`ChannelGain/Pan/Filter` 参考。
- GPU 合成器路径（`GpuSynth`）绕过混音台，CC 模块不会生效（D10 后续任务）；迁移期 CPU/GPU 行为会有差异，属已知限制。

---

## 六、改动清单（按 crate）

### yinhe-types
- [ ] `automation.rs`：`Gm2EffectUnit`、`AutomationTarget::Gm2Effect`、4 个方法分支、`param` 常量表、单测。
- [ ] `lib.rs`：导出新类型。

### yinhe-core
- [ ] `model.rs`：`TrackKind::Master`、`ensure_master_track` 辅助；`TrackData::is_master()`。
- [ ] 非音符轨判定 helper（conductor + master 统一）。

### yinhe-mixer
- [ ] `graph.rs`：`effect_id`、`apply_effect_param` 默认方法；`broadcast_effect_param`、`find_bus_with_effect`、`set_send_amount`。
- [x] `graph.rs`（阶段 A）：`handled_ccs`/`apply_cc` 默认方法与 `broadcast_channel_cc`。
- [ ] `params.rs`：`PluginFormat::Builtin`。

### yinhe-dsp（新）
- [ ] crate 骨架 + `Gm2Reverb`/`Gm2Chorus` + DSP 基础件 + 单测。
- [x] 阶段 A：`ChannelGain`/`ChannelPan`/`ChannelFilter` 已实现；biquad 各持一份（见 §10-5）；`Smoothed` 按 `ValueLerp` 模式自实现。

### yinhe-midi
- [ ] `gm2.rs`（解析/生成）、`parser.rs` SysEx 分支、`writer.rs` master 展开 + SysEx 写出、roundtrip 测试。

### yinhe-audio
- [ ] `audio_model.rs`：flatten 新分支（master 复制 + Gm2 占位）。
- [ ] `engine_render.rs`：dispatch 新分支（广播 + CC91/93）。
- [ ] `engine_state.rs`：chase 回填 GM2 参数；`skip_track` 不跳 master。
- [ ] `channel_layout.rs`：kind 过滤修正。
- [ ] `engine_mixer.rs`/`spawn.rs`：内置效果器实例的回收识别（如需）。
- [x] 阶段 A：`engine_render.rs` dispatch 白名单直发 DSP 链；`engine_state.rs` chase 回填；`channel.rs` chase 事件流不再发这些 CC。

### yinhe-editor-core
- [ ] `track_ops.rs`：`used_channels` 过滤；master 保护（不可删/移）。
- [ ] 文档编辑：master 轨 lane 编辑允许、音符操作拒绝。

### yinhe-egui
- [ ] `mix/`：`PluginInstance::Builtin`、picker 内置分组、activate/on_returns 分支、参数面板。
- [ ] `arrange/`：master badge/颜色、AM 展开、右键菜单保护、拖拽保护。
- [ ] `piano_view/`：master 不可写音符。
- [ ] `chrome/dock_bar.rs`：match 新分支。
- [ ] `event_browser`：master 节点/排除。
- [ ] `app/audio.rs`、`app/plugin_automation.rs` 等静默点。
- [ ] 自动化面板常量与显示适配。

### yinhe-yin
- [ ] mapping/io：随 serde 自动；确认 `.yin` 往返测试。

### yinhe-wgpu
- [ ] `automation/prepare.rs`：`target_hash` 新分支（防撞车）。

---

## 七、测试计划

| 层 | 测试 |
|---|---|
| yinhe-types | 新变体方法（值域/默认值/显示名）、lane 唯一性 |
| yinhe-midi | SysEx 解析/生成单测（7 参数 × 边界值 + 非 GM2 丢弃）；roundtrip：构造含 SysEx 的 SMF → 模型 → 导出 → 字节比对；master 展开（2 通道 + master CC，断言两通道都有且同 tick 通道自身覆盖）；空轨 strip 后的展开集合 |
| yinhe-audio | flatten：master CC 复制到所有激活通道；Gm2 占位事件；dispatch 广播到 mixer（用测试用 InsertProcessor 记录）；CC91/93 设 send；seek chase 回填 |
| yinhe-audio（阶段 A） | dispatch 白名单 CC 不发 xsynth、广播给模块；chase 回填；`events_to_send` 不再包含 DSP CC |
| yinhe-dsp | 各效果器：参数生效、无 NaN/爆音、reset 清尾、块长/采样率无关性 |
| yinhe-dsp（阶段 A） | Gain/Pan/Filter 单测（默认值、CC 生效、平滑、稳定性）；与 xsynth `apply_channel_effects` 的 A/B 听感对比（手工） |
| yinhe-editor-core | master 轨保护（删除/移动/音符）、剪贴板序列化 tag |
| yinhe-yin | `.yin` 往返（含 master 轨与 Gm2 lane） |
| 手工验证 | 挂模块后 A/B 听感对比；阶段 B：自造 GM2 MIDI → 导入 → 挂 Gm2Reverb 到 bus → 听感；导出 → 外部工具确认 SysEx |

---

## 八、实施顺序

**阶段 A：CC 模块化（已完成）**

1. ~~P1 yinhe-dsp crate + `ChannelGain`~~ ✅
2. ~~P2 混音台接入：`PluginFormat::Builtin` + `PluginInstance::Builtin` + `BuiltinInsert` + picker~~ ✅
3. ~~P3 分发机制：白名单直发 + `broadcast_channel_cc` + chase 回填~~ ✅
4. ~~P4 `ChannelPan` / `ChannelFilter`~~ ✅
5. 后续（P5）：手工 A/B 验收、按需补 dispatch 单测、参数面板对内置模块的说明文案。

**阶段 B：GM2 效果与 SysEx 全链（下一步）**

6. **P6 模型层**：`AutomationTarget::Gm2Effect` + `TrackKind::Master` + 所有穷举 match 跟进。
7. **P7 GM2 DSP**：`Gm2Reverb`/`Gm2Chorus` + DSP 单测。
8. **P8 SysEx 全链**：导入解析 + 导出生成 + master 轨 lane 存放 + 导出展开。
9. **P9 回放驱动**：`effect_id` 广播（`broadcast_effect_param`）+ CC91/93 send + chase。
10. **P10 master 轨 UI 完善**：AR/PR/事件浏览器/保护逻辑。
11. **P11 验收**：真实/自造 MIDI 文件端到端 + release 构建。

**阶段 C（远期）**：XG/GS 效果扩展；xsynth 精简评估（替换为纯采样器）。

每个阶段完成后跑 `cargo fmt`、涉及 crate 的 `clippy`/`test`，并按 AGENTS.md 分步 commit。

---

## 九、后续与已知限制

1. **GPU 合成器**：已接入混音台（见 §5.7）。32 通道上限保留（2 端口）；超出通道在 GPU 模式下静音。
2. **Send to Reverb**：第一批只存与导出，回放不生效。
3. **XG/GS 效果**：阶段 C 按同一模式扩展（effect_id 空间、SysEx 前缀不同）。
4. **MIDI 效果器**（琶音器等）：需要新的 MIDI 事件链，当前 `InsertProcessor` 设计未覆盖。
5. **效果器参数自动化对插件 insert**：本规格打通了内置效果的 AM；外部插件 insert 的 AM 仍未支持。
6. **master 轨与混音台主输出**：无直接关联；master 轨 CC 不影响 `MixerParams.master.gain`。
7. **xsynth 源码不可改**：它是 crates.io 依赖，迁移只能通过"不发送对应 CC"绕过其通道处理；远期精简需评估 fork 或自研采样器。

---

## 十、待确认项

1. **GM2 参数 lane 存放**：本稿改为 **master 轨**（原 D5 调研时选择"跟随来源轨"，但当时尚无 master 轨）。若坚持跟随来源轨，导出 SysEx 的写出位置需按来源轨处理。
2. **导出时 SysEx 写在哪条轨**：本稿为"第一条实际写出的 MIDI 轨"。
3. **master 轨是否允许用户删除**：本稿为"自动 ensure、不可删除"。
4. **conductor 改名**：本稿建议 conductor badge 从 `"Master"` 改为 `"Conductor"`。
5. ~~CC 模块与 xsynth 的 biquad 复用~~（已定：各自一份）：
   `yinhe-dsp::dsp::biquad`（ChannelFilter 的通道低通）与
   `yinhe-synth::synth::filter`（per-voice 音色 filter，音源层保留）
   用途已分离，各持约 30 行 RBJ 系数实现，注释互指；都不引入 wgpu 依赖。
