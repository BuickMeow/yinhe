# 方案 C：AP 与 LC 并列的通道模型

本文是设计提案，不代表当前实现。现有文档已经使用「目标 A / 目标 B」，
为避免和历史命名冲突，本文暂称**方案 C（AP / LC 并列）**。如果确认要叫
「方案 B」，需要先给旧的目标 B 改名，否则讨论时一定会串。

一句话：

> 默认每条 Track 都是普通 DAW 通道（独立 LC）；AP 编组是可选的共享 MIDI
> 源。AP 和 LC 是并列的通道类型，不是上下级。导出时把独立 LC 自动分配到
> 剩余的 MIDI 通道。

---

## 1. 术语

| 术语 | 含义 |
|---|---|
| Track | 内容容器：音符、音频片段、轨道级自动化 |
| LC | Logical Channel，普通 DAW 通道；拥有独立音频路径、推子、insert、send、automation、freeze |
| AP 通道 / AP 编组 | A01..P16 的 MIDI 源编组；拥有共享 Rack、共享乐器、共享通道级自动化、共享输出 LC |
| AP Rack | 一个 AP 编组里的共享乐器链 |
| 私有 Rack | 一条独立 Track/LC 自己的乐器链 |
| Owner | 自动化/CC 的归属者：TrackLc、ApRack、ApLc、Bus、Master |

AP 不是“父通道”，LC 不是“子通道”。它们是 Track 音频源的两种并列选择：

```text
Track 1 ──> LC 100（默认，独立）
Track 2 ──> LC 101（默认，独立）
Track 3 ──> AP A01 ──> LC 200（共享输出）
Track 4 ──> AP A01 ──┘
```

AP 编组也可以只有一条 LC，多输出插件则一个输出一条 LC。AP 的输出 LC
和普通 Track 的 LC 使用同一种 `MixerNode`，没有两套通道条实现。

---

## 2. 和当前目标 B 的关系

当前目标 B 的链路：

```text
Track -> MIDI 源通道/Rack（固定）-> LC -> Bus/Master
```

Track 的 MIDI 通道决定 Rack，LC 是 Rack 的下游。默认所有轨道共享 256 条
MIDI 通道 Rack。

方案 C 的链路：

```text
Track ──> 私有 Rack -> 自己的 LC ──> Bus/Master     （默认）
   └────> AP 编组/共享 Rack -> AP 的 LC ──> Bus/Master（手动链接）
```

变化只在「Track 的音频源归谁」：

- 默认 Track 有自己的私有 Rack + LC；
- 只有显式链接到 A01 时，Track 才加入 A01 的共享 Rack 和共享自动化；
- AP 和 LC 都可以直接进 Bus/Master，二者没有父子关系。

这不是回滚。可以复用的部分：

- `MixerNode` / `LogicalChannelId` / `RackParams` / `FrozenAudio`；
- LC 的 gain/pan/M/S/insert/send/automation/freeze；
- Bus 嵌套、Bus 自动化、Master；
- CLAP/VST3 多输出与按输出 LC；
- `ChannelLayout` 的 dense 通道、每轨 LC、多输出 LC；
- 自动化 lane 的编辑、undo、AR 子行、锚点面板。

需要改的是：

- `TrackData` 的音频目标从「隐式等于 MIDI 通道」变成显式的
  `TrackAudioTarget`；
- 默认不再有 256 条常驻 MIDI 通道 Rack/LC，改为按需创建 AP 和 Track LC；
- 自动化 owner 从「MIDI 通道」改为「TrackLc / ApRack / ApLc」；
- 导出时增加「独立 LC → 剩余 MIDI 通道」的分配器；
- MIDI 导入时自动建立 AP 编组。

---

## 3. 核心数据模型（草案）

### 3.1 Track 的音频目标

```rust
struct TrackData {
    // 原有内容字段...
    audio_target: TrackAudioTarget,
}

enum TrackAudioTarget {
    /// 默认：普通 DAW 通道，拥有私有 Rack + 自己的 LC。
    OwnLc {
        lc: LogicalChannelId,
        rack: RackId,
    },
    /// 手动链接到 AP 编组：MIDI 进共享 Rack，音频走 AP 的 LC。
    Ap {
        ap: ApGroupId,
        /// 该轨在 AP 多输出里的输出索引（单输出 = 0）。
        output: u16,
    },
}
```

说明：

- `OwnLc` 是默认值；新建 MIDI 轨/音频轨时创建或复用自己的 LC。
- `Ap` 不拥有私有 LC；它的音频路径就是 AP 的输出 LC。
- 从 AP 解链回 `OwnLc` 时保留原来的私有 LC/私有 Rack 状态，
  避免解链后所有设置丢失。
- 音频轨（Audio Track）通常只用 `OwnLc`，不参与 AP 编组。

### 3.2 AP 编组

```rust
struct ApGroup {
    id: ApGroupId,
    /// 对应 MIDI 通道；None = 用户手工建的无 MIDI 编组（可选）。
    midi: Option<(u8 /*port*/, u8 /*channel*/)>,
    rack: RackParams,
    /// 单输出 = 1 条；多输出插件 = 每个输出一条。
    output_lcs: Vec<LogicalChannelId>,
    /// AP 级自动化（CC / 乐器参数 / AP LC 的 gain/pan/send/insert）。
    automation: Vec<AutomationLane>,
    enabled: bool,
}
```

`ApGroupId` 建议用稳定 `u32`，不要直接用 `(port, channel)` 做身份。
MIDI 通道只是 AP 的一个属性，用户可以把 A01 编组改名、解绑或重建，
不应导致自动化/路由串线。

### 3.3 MixerParams

```rust
struct MixerParams {
    /// 所有 LC：普通 Track LC + AP 输出 LC + 每轨 LC。
    lcs: Vec<MixerNode>,
    /// AP 编组；只有用到的通道才创建。
    aps: Vec<ApGroup>,
    /// 私有 Rack；Track 解链后仍保留。
    racks: Vec<RackParams>,
    buses: Vec<MixerNode>,
    master: MixerNode,
}
```

和当前 v14 的 `channels[0..256]` 不同：不再保证 256 条 MIDI 通道条常驻。
`lcs` 按需增长，空工程可以只有 Master + Conductor。

---

## 4. AP 与 LC 的职责边界

| 项目 | 普通 LC | AP 编组 |
|---|---|---|
| 默认存在 | 是，每条 Track 一条 | 否，手动链接/导入时创建 |
| 乐器/Rack | 私有 Rack | 共享 AP Rack |
| MIDI 输入 | 只属于本 Track | 所有链接到该 AP 的 Track |
| gain/pan/M/S | 本 LC | AP 输出 LC |
| insert | 本 LC | AP 输出 LC |
| send | 本 LC | AP 输出 LC |
| CC/PB/RPN | 本 Track 的 LC owner | AP owner |
| 乐器参数自动化 | 私有 Rack owner | AP Rack owner |
| freeze | 本 LC | AP 输出 LC |
| 多输出插件 | 可以是一轨一条 LC | 每个输出一条 LC |
| 导出 MIDI | 分配剩余通道 | 固定/绑定的 MIDI 通道 |

关键规则：

1. **AP 是一条共享音频流**；链接到 AP 的 Track 不能再假装有独立推子。
2. **AP 冻结的是整组输出**；链接的 Track 不能单独冻结自己的 AP 部分。
3. **AP 的 automation 是组级 owner**；多个 Track 的同名自动化在 AP 上合并。
4. **普通 LC 是完整独立通道**；默认行为与成熟 DAW 的普通音轨一致。

---

## 5. 自动化与普通 CC 的合并

### 5.1 两类自动化

```text
Track 级：velocity、note expression、track-only 参数
Channel 级：CC、Pitch Bend、RPN/NRPN、乐器参数、LC gain/pan/send/insert
```

Track 级始终属于 Track；Channel 级属于当前音频目标的 owner：

- `OwnLc` -> `TrackLc(lc)`
- `Ap` -> `ApRack(ap)` + `ApLc(ap)`
- Bus/Master 不变

### 5.2 同名自动化允许存在，但渲染只认 owner

模型上允许多条 Track 各自保留一条同名 lane（方便用户编辑/迁移），
但渲染和导出前做一次 **owner 解析**：

```text
(owner, target, tick) -> 最终值
```

推荐策略：

1. 完全相同 `(owner, target)` 的 lane 归为一组；
2. 同一 tick 值相同 -> 直接合并，无冲突；
3. 同一 tick 值不同：
   - 记录 `edit_epoch`（最后一次用户编辑的序号）；
   - 渲染/导出取 `edit_epoch` 最大的点；
   - UI 在冲突点显示标记，并允许用户选择「以哪条轨为准」；
4. 合并结果只写一次到引擎/导出，不会出现同一通道同一 tick 写两遍 CC。

这样「同通道允许多条相同自动化」不会变成不确定的 MIDI 事件顺序。
如果以后想支持“层叠/叠加”语义（例如多条 CC 曲线相加），需要明确哪些
target 可叠加；默认不做，避免黑乐谱里 CC 互相覆盖。

### 5.3 自动化和普通 CC 排在一起

存储层继续使用 `AutomationLane`，不区分「CC 区」和「AM 区」。
Track 的 lane 列表按 target 统一排序显示：

```text
CC 007 Volume
CC 010 Pan
Pitch Bend
Rack Cutoff
LC Gain
```

导出时按 owner 解析后统一写入对应 MIDI 通道；不再有“自动化一套、CC 一套”
的两张表。

---

## 6. MIDI 导入

导入时自动编组：

1. 解析每个 MIDI track 的 `(port, channel)`；
2. 对每个实际出现的 `(port, channel)` 创建一个 AP 编组；
3. 同一 `(port, channel)` 的多个 MIDI track 全部链接到同一个 AP；
4. MIDI track 的名称/颜色保留在 Track 上；
5. 空 MIDI track 不创建 AP，保持默认 `OwnLc`；
6. 导入的 CC/Program Change/乐器参数自动落到 AP owner。

这样导入后：

- 同一 MIDI 通道的轨道保持原来的共享乐器/共享 CC 语义；
- 不同 MIDI 通道仍然是不同 AP；
- 用户可以把 AP 里的某条 Track 解链，变成独立 LC，不影响其他轨道。

当前解析器已经按 `port << 4 | channel` 建轨，并在 `group_automation_events`
里按通道归组；方案 C 主要改“导入后建立 AP 并链接”，不需要重写 MIDI 解析。

---

## 7. MIDI 导出与通道分配

### 7.1 分配顺序

导出前建立通道表：

1. 已链接 MIDI 的 AP 编组占用其固定通道；
2. 独立 `OwnLc` Track 按轨道顺序分配剩余 `(port, channel)`；
3. 每个独立 Track 只拿一个通道；AP 的所有 Track 共用一个通道；
4. 多输出 AP 仍只占一个 MIDI 通道（MIDI 输入是一个通道，多输出只是音频）。

伪代码：

```text
used = { ap.midi for ap in aps if ap.midi.is_some() }
pool = all (port, channel) not in used

for track in tracks:
    if track.audio_target is Ap:
        channel = ap.midi
    else:
        channel = pool.next()
        if channel is None:
            report "MIDI 通道不足"
```

### 7.2 通道不足

三种策略，导出设置里选：

| 模式 | 行为 |
|---|---|
| `Fail` | 直接报错，列出需要的通道数 |
| `ReuseSafe` | 只复用没有 CC/自动化冲突的通道；有冲突则报错 |
| `Force` | 强制复用，导出后提示哪些轨道共享了通道 |

默认 `Fail`，避免黑乐谱导出后 CC 互相覆盖。

### 7.3 端口

当前导出只写低 4 位通道，没有 `MidiPort` meta。方案 C 第一阶段：

- 只使用 Port A 的 16 个通道；
- 需要超过 16 条独立通道时，再补 `MidiPort` meta，扩展到 A..P 共 256 个。

注意：不是所有 DAW 都保留 MIDI port，扩展端口前要先在导出选项里明确提示。

### 7.4 自动化写入

导出时对每个 owner 只写一次：

- AP owner 的 CC/PB/RPN 写到 AP 的 MIDI 通道；
- 独立 LC 的 CC 写到分配到的通道；
- 乐器参数自动化不写成 MIDI（如果插件参数不是 CC），保持为工程内设置；
- 同名 lane 先按第 5 节的 owner 解析合并。

---

## 8. UI / 交互

### 8.1 Track 头

每轨显示当前音频目标：

```text
Track 1   [LC 1]        ← 默认
Track 2   [AP A01]      ← 已链接
Track 3   [AP A01]      ← 已链接
```

点击目标打开选择器：

- `独立通道（LC n）`
- `链接到 AP：A01 / A02 / ...`
- `新建 AP 编组…`

### 8.2 快速链接

- 拖 Track 到 AP 编组头/AP 行即可链接；
- 右键 Track ->「链接到 MIDI 通道」；
- 选中多条 Track 后一次性链接到同一个 AP；
- 解链后回到原来的 `OwnLc`，不复制 AP 的自动化（避免二次污染）。

### 8.3 MIX 视图

- 普通 LC 按 Track 顺序显示；
- AP 编组单独显示为共享通道，条带上标明「A01 · 4 条轨共享」；
- 链接到 AP 的 Track 不再显示独立 LC 条，只显示 AP 条；
- AP 多输出时显示每个输出的 LC 条；
- AP 的 freeze/mute/solo 明确提示作用范围。

### 8.4 自动化面板

- Track 只显示 track-local lane + 当前 owner 的 channel lane；
- AP owner lane 在任一链接 Track 上都能编辑，但只有一份物理数据；
- 冲突点显示标记，并在 AP 头显示「N 个自动化冲突」。

---

## 9. 冻结、静音和独奏

| 操作 | 普通 LC | AP |
|---|---|---|
| 冻结 | 冻结本 LC | 冻结 AP 输出（整组） |
| mute | 本 LC 静音 | AP 静音（所有链接轨） |
| solo | 本 LC solo | AP solo |
| Track M/S | 内容门，停止该轨后续 MIDI | 同样只能停该轨 MIDI，不能抽走已混入 AP 的音频 |

这和当前目标 B 的语义一致，只是 LC 的默认归属从“MIDI 通道”变成“Track”。

---

## 10. 迁移

### 10.1 v14 -> v15

旧工程的行为必须保持不变：

1. 旧工程里每个实际使用的 MIDI 通道 -> 创建一个 AP 编组；
2. 旧 `MixerParams.channels[ch]` -> AP 的输出 LC；
3. 旧 `racks[ch]` -> AP 的共享 Rack；
4. 旧 `TrackData.port/channel` 对应的 Track 全部链接到 AP；
5. 旧 `TrackData.lc`（per-track / multi-output）-> `OwnLc` 或 AP 多输出 LC；
6. 旧 `rack_output` -> AP 多输出索引；
7. 空通道不创建 AP，避免 256 条空编组；
8. 旧工程的冻结/自动化/insert/发送原样搬到 AP。

这样打开旧工程听到的结果和 v14 一致，只是内部从“固定 MIDI 通道条”
变成了“AP 编组”。

### 10.2 新工程

- 新建 MIDI 轨：默认 `OwnLc`，私有 Rack（或内置音源）；
- 新建音频轨：`OwnLc` + 音频输入；
- 手动链接 AP：共享 Rack；
- MIDI 导入：自动建立 AP 并链接。

### 10.3 兼容

- v15 加载器支持 v8..v15；
- 旧版本不能打开 v15，这是破坏性变更，需要在文档和发布说明中写清楚；
- 回滚方式：v15 保存前尽量保留一份 v14 兼容导出？不建议长期维护双写；
  更实际的是发布前备份工程，加载器只保证向后读。

---

## 11. 性能

默认每轨一条 LC，数量从“最多 256 条 MIDI 通道”变成“最多 Track 数”。

- 799 轨 -> 799 条 LC，`MixerGraph` 的通道缓冲约 `799 × 2 × frames × 4B`，
  在 512 帧下约 3.3 MB，可接受；
- 65536 轨满载时 LC 数量很大，必须懒分配：无音符/无音频/无自动化的空轨
  不创建 LC；
- 默认私有 Rack 如果每条轨都加载一个外部插件，实例数会爆炸。建议：
  - 默认使用内置音源（XSynth/YinheCpu）的 per-track 输出；
  - 只有用户手动选择外部插件时才创建私有插件实例；
  - AP 编组共享一个插件实例，是省资源路径。
- 多输出 AP 的每个输出一条 LC，仍然比每轨一个插件实例便宜。

这是方案 C 最大的工程风险：**默认独立通道 + 外部插件 = 实例数量增长**。
需要产品上明确默认音源策略，不能假装所有轨都能无成本挂独立插件。

---

## 12. 和当前实现的可复用度

| 现有模块 | 方案 C 是否复用 | 说明 |
|---|---|---|
| `MixerNode` / `StripParams` | 复用 | LC/AP 输出/Bus/Master 共用 |
| `LogicalChannelId` | 复用 | 每轨/AP 输出都是稳定 ID |
| `RackParams` | 复用 | AP 共享 Rack、Track 私有 Rack |
| `FrozenAudio` | 复用 | 普通 LC/AP 输出均可冻结 |
| `MixerGain/Pan/Send/InsertParam` | 复用 | owner 从 MIDI 通道改为 TrackLc/Ap |
| `ChannelLayout` | 复用 | dense 通道、每轨 LC、多输出已具备 |
| CLAP/VST3 多输出 | 复用 | AP 多输出直接使用 |
| Bus 嵌套 | 复用 | AP/LC 都走同一 Bus |
| MIDI 导入/导出 | 改造 | 导入建 AP，导出分配剩余通道 |
| Track `port/channel` | 降级为属性 | 不再是混音路由的唯一依据 |

---

## 13. 分阶段实现建议

1. **数据模型 v15**：`TrackAudioTarget`、`ApGroup`、`MixerParams.lcs/aps`；
   v14 迁移为 AP。
2. **默认 OwnLc**：新建工程/新建轨分配私有 LC；空轨懒分配。
3. **AP 链接 UI**：Track 头选择器、拖拽链接、MIX 共享条。
4. **自动化 owner 解析**：同名 lane 合并、冲突标记、引擎/导出只写一次。
5. **MIDI 导入自动编组**。
6. **MIDI 导出通道分配器**：Fail/ReuseSafe/Force。
7. **冻结/多输出/多轨共享回归**：确保 AP/LC 的 freeze、multi-output、
   automation、undo 全部覆盖。
8. **端口扩展**：需要时补 `MidiPort`。

每一步都可以独立提交、独立回滚；不要一次性替换 256 通道表。

---

## 14. 需要先确认的问题

1.「AP」是否就是 A01..P16 的 MIDI 源编组？本文按这个理解写。
2. 默认私有 Rack 用内置音源还是外部插件？外部插件按轨实例化成本很高。
3. 同 owner 同 target 的冲突，采用「最后编辑优先」还是让用户手动裁决？
   本文建议最后编辑优先 + UI 标记。
4. 独立 Track 数超过 16 时，第一阶段是否只支持 Port A 16 通道？
5. 解链 AP 时，AP 上的自动化是否复制回 Track，还是保持 AP、Track 回到空
   的私有 LC？本文建议不复制，避免同一条曲线出现两份。
6. 是否允许一个 AP 编组不对应 MIDI 通道（纯共享 Rack）？本文保留
   `midi: Option<...>`，但默认 UI 不暴露。

---

## 15. 结论

- 这是一个成立的方案，而且比目标 B 更接近「普通 DAW 默认行为」。
- 它不是回滚：`MixerNode`、LC、自动化、freeze、Bus、多输出都能继续用；
  真正变的是 **Track 的默认音频源** 和 **AP/LC 的并列关系**。
- 不建议直接叫「方案 B」：仓库里已有目标 B，建议叫**方案 C（AP/LC 并列）**。
- 实现上最大的风险不是音频图，而是：
  1. 默认每轨私有 Rack 的插件实例成本；
  2. 同名自动化/CC 的 owner 解析；
  3. 导出时 MIDI 通道耗尽。
  这三点需要在动代码前先定策略。
