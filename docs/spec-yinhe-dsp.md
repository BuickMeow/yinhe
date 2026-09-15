# spec-yinhe-dsp：内置效果器与 GM2 效果全链

> 状态：设计稿（待评审）
> 范围：第一批 = GM2 Reverb/Chorus + master 轨 + SysEx 全链 + 混音台接入
> 关联：`spec-xsynth-integration.md`（效果器链预留架构）、`docs/GMLevel2.xml`（参数词典）

---

## 一、背景与目标

yinhe 已有效果器链（`yinhe-mixer` 的 `InsertProcessor`，每通道/bus/master 一条链），但目前只能挂外部 CLAP/VST3 插件，没有任何内置 DSP。本规格定义新 crate **yinhe-dsp**：内置效果器模块，像插件一样自由拼搭；第一批只做 **GM2 Reverb / GM2 Chorus**，但打通"MIDI 文件 SysEx 参数 ↔ 工程自动化参数 ↔ DSP"全链，为后续 XG/GS 效果打地基。

核心诉求（用户原话归纳）：

1. yinhe-dsp 首先是**音频 DSP 模块**；SysEx 不进底层模型，底层用**自动化参数**表达。
2. 导入 MIDI 时：效果参数 SysEx → 自动化参数；导出 MIDI 时：自动化参数 → SysEx。
3. 效果器像模块一样自由拼搭，纯手动挂载，不自动创建实例。
4. 效果参数事件**广播**给工程内所有同型效果器实例。
5. 参数面板手动调参 = 写自动化 lane（lane 是唯一真相）。
6. 第一批能力：AM 回放驱动 DSP、CC91/93 打通 send、导出生成 SysEx。
7. 顺带支持 **master 轨**（挂全局 CC，导出时展开到所有 MIDI 通道）。

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

---

## 二、现状摘要

### 2.1 效果器链（yinhe-mixer）

- `InsertProcessor`（`crates/yinhe-mixer/src/graph.rs:21`）：纯音频口 `process(&mut self, left, right)`，无事件入参、无 position 入参；参数靠处理器内部的 `ParamQueue`（`param_queue.rs`，归一化 0..1，`Arc` 跨线程，latest-wins）。
- 挂载点：`InsertTarget::{Channel, Audio, Bus, Master}`（`crates/yinhe-audio/src/spawn.rs:16`）；命令 `AudioCommand::InsertAdd/Remove/Replace`。
- 持久化：`InsertRef { plugin_path, plugin_id, name, format: PluginFormat::{Clap,Vst3}, bypassed, state }`（`crates/yinhe-mixer/src/params.rs:69`）；`.yin` 混音段 `MIXER_SECTION_VERSION = 5`（`crates/yinhe-yin/src/io.rs`）。
- 旁通：`Arc<AtomicBool>`（与渲染线程共享，零命令往返）。
- 现有内置 DSP 只有限幅器（`yinhe-synth/limiter.rs`），不属链。

### 2.2 自动化与 MIDI 管线

- lane 模型：`AutomationTarget`（`crates/yinhe-types/src/automation.rs:137`）现有 6 变体；`AutomationLane` 存于 `TrackData.automation_lanes`，Tempo 存于 `ConductorData.tempo`。
- 导入 `crates/yinhe-midi/src/parser.rs`：SysEx 在 `:563`（每轨）与 `:265`（conductor pass）被丢弃；RPN/NRPN 解析流程可作参考。
- 导出 `crates/yinhe-midi/src/writer.rs`：不写 SysEx；`push_lane_event`（`:362`）对 `Tempo | PluginParam` 静默跳过；`sort_by_key` 稳定，同 tick 顺序 = push 顺序。
- 回放 `crates/yinhe-audio/src/audio_model.rs`：`flatten_automation_to_cc_events`（`:302`）把 lane 展平为 `SortedCC`；dispatch 在 `engine_render.rs:112`，`PluginParam` 仅路由到乐器插件。
- CC91/93 现为普通 CC（`channel.rs:413` 存入通用 cc_values，随事件发 xsynth），与混音台 bus send 无关联。

### 2.3 测试数据现状

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
  Cargo.toml            # 依赖：yinhe-mixer（InsertProcessor）、serde；不依赖 audio/egui
  src/lib.rs
  src/gm2.rs            # Gm2EffectUnit 相关常量/值域/名称（与 yinhe-types 共享或转发）
  src/gm2/reverb.rs     # Gm2Reverb
  src/gm2/chorus.rs     # Gm2Chorus
  src/dsp/delay.rs      # 延迟线
  src/dsp/comb.rs       # 梳状/全通（混响用）
  src/dsp/lfo.rs        # 正弦 LFO
  src/dsp/onepole.rs    # 单极低通（平滑/阻尼）
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

## 五、改动清单（按 crate）

### yinhe-types
- [ ] `automation.rs`：`Gm2EffectUnit`、`AutomationTarget::Gm2Effect`、4 个方法分支、`param` 常量表、单测。
- [ ] `lib.rs`：导出新类型。

### yinhe-core
- [ ] `model.rs`：`TrackKind::Master`、`ensure_master_track` 辅助；`TrackData::is_master()`。
- [ ] 非音符轨判定 helper（conductor + master 统一）。

### yinhe-mixer
- [ ] `graph.rs`：`effect_id`、`apply_effect_param` 默认方法；`broadcast_effect_param`、`find_bus_with_effect`、`set_send_amount`。
- [ ] `params.rs`：`PluginFormat::Builtin`。

### yinhe-dsp（新）
- [ ] crate 骨架 + `Gm2Reverb`/`Gm2Chorus` + DSP 基础件 + 单测。

### yinhe-midi
- [ ] `gm2.rs`（解析/生成）、`parser.rs` SysEx 分支、`writer.rs` master 展开 + SysEx 写出、roundtrip 测试。

### yinhe-audio
- [ ] `audio_model.rs`：flatten 新分支（master 复制 + Gm2 占位）。
- [ ] `engine_render.rs`：dispatch 新分支（广播 + CC91/93）。
- [ ] `engine_state.rs`：chase 回填 GM2 参数；`skip_track` 不跳 master。
- [ ] `channel_layout.rs`：kind 过滤修正。
- [ ] `engine_mixer.rs`/`spawn.rs`：内置效果器实例的回收识别（如需）。

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

## 六、测试计划

| 层 | 测试 |
|---|---|
| yinhe-types | 新变体方法（值域/默认值/显示名）、lane 唯一性 |
| yinhe-midi | SysEx 解析/生成单测（7 参数 × 边界值 + 非 GM2 丢弃）；roundtrip：构造含 SysEx 的 SMF → 模型 → 导出 → 字节比对；master 展开（2 通道 + master CC，断言两通道都有且同 tick 通道自身覆盖）；空轨 strip 后的展开集合 |
| yinhe-audio | flatten：master CC 复制到所有激活通道；Gm2 占位事件；dispatch 广播到 mixer（用测试用 InsertProcessor 记录）；CC91/93 设 send；seek chase 回填 |
| yinhe-dsp | 各效果器：参数生效、无 NaN/爆音、reset 清尾、块长/采样率无关性 |
| yinhe-editor-core | master 轨保护（删除/移动/音符）、剪贴板序列化 tag |
| yinhe-yin | `.yin` 往返（含 master 轨与 Gm2 lane） |
| 手工验证 | 自造 GM2 MIDI（Domino 或手写字节）→ 导入 → 挂 Gm2Reverb 到 bus → 听感；导出 → 用外部工具确认 SysEx |

---

## 七、实施顺序（建议）

1. **P0 模型层**：`AutomationTarget::Gm2Effect` + `TrackKind::Master` + 所有穷举 match 跟进（保证编译与测试绿）。
2. **P1 yinhe-dsp**：crate + 两个效果器 + DSP 单测。
3. **P2 混音台接入**：`PluginFormat::Builtin` + `PluginInstance::Builtin` + 挂载/回收/参数面板（能挂能听能保存，参数先走 ParamQueue 手动调）。
4. **P3 SysEx 全链**：导入解析 + 导出生成 + master 轨 lane 存放 + 导出展开。
5. **P4 回放驱动**：广播 + CC91/93 send + chase。
6. **P5 master 轨 UI 完善**：AR/PR/事件浏览器/保护逻辑。
7. **P6 验收**：真实/自造 MIDI 文件端到端 + release 构建。

每个阶段完成后跑 `cargo fmt`、涉及 crate 的 `clippy`/`test`，并按 AGENTS.md 分步 commit。

---

## 八、后续与已知限制

1. **GPU 合成器绕过混音台**（独立任务）：`GpuSynth::render` 输出最终交错立体声，接入 mixer 需 per-channel planar 出口、解决 `dense % 32` 上限、限幅移至 master 后、修 `render_idle`、同步两条导出路径（`export.rs::render_block` 与 `export_wav_gpu`）。
2. **Send to Reverb**：第一批只存与导出，回放不生效。
3. **XG/GS 效果**：架构就绪后按同一模式扩展（effect_id 空间、SysEx 前缀不同）。
4. **MIDI 效果器**（琶音器等）：需要新的 MIDI 事件链，当前 `InsertProcessor` 设计未覆盖。
5. **效果器参数自动化对插件 insert**：本规格打通了内置效果的 AM；外部插件 insert 的 AM 仍未支持。
6. **master 轨与混音台主输出**：无直接关联；master 轨 CC 不影响 `MixerParams.master.gain`。

---

## 九、待确认项

1. **GM2 参数 lane 存放**：本稿改为 **master 轨**（原 D5 调研时选择"跟随来源轨"，但当时尚无 master 轨）。若坚持跟随来源轨，导出 SysEx 的写出位置需按来源轨处理。
2. **导出时 SysEx 写在哪条轨**：本稿为"第一条实际写出的 MIDI 轨"。
3. **master 轨是否允许用户删除**：本稿为"自动 ensure、不可删除"。
4. **conductor 改名**：本稿建议 conductor badge 从 `"Master"` 改为 `"Conductor"`。
