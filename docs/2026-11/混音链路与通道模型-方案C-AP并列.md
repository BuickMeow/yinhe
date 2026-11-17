# 方案 C：AP 与 LC 并列的通道模型

本文是设计提案，不代表当前实现。现有文档已经使用「目标 A / 目标 B」，
为避免和历史命名冲突，本文暂称**方案 C（AP / LC 并列）**。如果确认要叫
「方案 B」，需要先给旧的目标 B 改名，否则讨论时一定会串。

一句话：

> 默认每条 Track 都是普通 DAW 通道（独立 LC）；AP 编组是可选的共享 MIDI
> 源。AP 展开出的 **AP 通道** 和 LC 是并列的音频通道类型，不是上下级。
> 导出时把独立 LC 自动分配到剩余的 MIDI 通道。

---

## 0. 最新决策（覆盖下文旧描述）

这一节是后来确认的决策，和下文冲突时以本节为准：

1. **LC ID 空间改为 `u16`（0..65535）**，不再用 `u32 + TRACK_LC_BASE=1_000_000`。
   LC ID 不再编码“MIDI/音频源通道号”，而是独立的通道身份。v14 旧 ID 在
   v15 迁移时重新分配，不能保留原数值。
2. **取消独立 Bus 类型**。所有 LC/AP 通道都可以接收其他通道的发送，
   “Bus”只是一种使用角色，不是第二种通道。Master 仍是唯一终点的特殊通道。
3. **默认通道输出就是 Master**。“直通 Master”不是一种特殊音轨类型；
   任何通道把 `output` 设为 Master 即可。若需要防止误路由，加 `routing_lock`
   标记，而不是新建“直通轨”。
4. **freeze 按音频通道分配**：普通 Track 的 LC、AP 的 AP 通道。
   FL Studio 的 freeze 也是按 Mixer Track（音频通道）冻结，不是按 Playlist
   Track/Pattern；这恰好支持“按通道冻结”，但普通 Track 与 LC 是 1:1，
   所以表现上仍是“单轨冻结”。
5. **多输出插件是可选高级能力**，不是方案 C 的 MVP。默认多条 LC 各自挂一个
   LC Rack 实例；大采样器/鼓机需要省内存时才用单实例多输出。

## 1. 术语

| 术语 | 含义 |
|---|---|
| Track | 内容容器：音符、音频片段、轨道级自动化 |
| LC | Logical Channel，普通 DAW 通道；拥有独立音频路径、推子、insert、send、automation、freeze |
| AP 编组 | A01..P16 的 MIDI 源编组：共享 Rack、共享 MIDI 身份；本身不是音频通道 |
| AP 通道 | AP 编组展开出的音频通道（`MixerNode`）；单输出 1 条，多输出 N 条；与 LC 并列 |
| AP Rack | 一个 AP 编组里的共享乐器链 |
| LC Rack | 一条独立 Track/LC 自己的乐器链（原“私有 Rack”） |
| Owner | 自动化/CC 的归属者：TrackLc、ApRack、ApChannel、Channel、Master |

AP 不是“父通道”，LC 不是“子通道”。它们是 Track 音频源的两种并列选择：

```text
Track 1 ──> LC 100（默认，独立）
Track 2 ──> LC 101（默认，独立）
Track 3 ──> AP A01 通道 200（共享）
Track 4 ──> AP A01 通道 200
```

关键区分：

- **AP 编组**是共享 Rack + MIDI 身份，本身不是通道；
- **AP 通道**是 AP 的音频输出通道，和普通 LC 一样是 `MixerNode`；
- 单输出 AP 展开 1 条 AP 通道，多输出 AP 展开 N 条 AP 通道；
- Track 只指向一条音频通道：普通 LC 或某条 AP 通道。

所以链路永远是：

```text
Track -> Channel（LC 或 AP 通道）-> 任意 Channel ... -> Master
```

不存在 `AP -> LC` 的额外上下级。

---

## 2. 先把 A / B / C 说清楚

A 和 B 不是用户提出的两套方案，而是旧文档里的历史编号：

| 名称 | 是什么 | 核心链路 | 状态 |
|---|---|---|---|
| 目标 A | 当时的实现快照 / 产品约束 | Track -> MIDI 通道条（共享乐器/推子/insert/发送）-> Bus/Master | 旧实现 |
| 目标 B | 上一轮重构的目标模型 | Track -> Rack（按 MIDI 通道共享）-> LC（Rack 输出的一条音频流）-> Bus/Master | 已在 v10..v14 落地 |
| 方案 C | 用户这次提出的新组织方式 | Track -> 自己的 LC Rack + LC（默认），或 Track -> AP 编组/共享 Rack -> AP 通道 | 本文设计提案 |

三者的关键区别：

| 问题 | 目标 A | 目标 B | 方案 C |
|---|---|---|---|
| 默认混音单位 | MIDI 通道条 | Rack 的输出 LC | 每条 Track 的独立 LC |
| 同 MIDI 通道的多条轨 | 共享全部混音参数 | 共享 Rack 和 LC | 默认互不影响；只有链接 AP 才共享 |
| 乐器归谁 | MIDI 通道 | Rack（按 MIDI 通道） | LC Rack（Track）/ 共享 Rack（AP） |
| 轨道有独立推子吗 | 没有 | 只有 per-track/multi-output LC 有 | 默认有 |
| MIDI 通道的角色 | 混音身份 | Rack 的触发源 | 可选的 AP 编组 / 导出通道 |
| 自动化 owner | MIDI 通道 | Rack / LC | TrackLc / ApRack / ApChannel |
| 导出 | 轨道 channel 直接写出 | 同左 | 独立 LC 自动分配剩余 MIDI 通道 |
| 主要取舍 | 省资源，牺牲每轨独立 | 分出音频通道，但默认仍按 MIDI 通道共享 | 默认像普通 DAW，代价是每轨 LC/LC Rack 成本 |

所以：

- A 是「当时已经存在的东西」，不是用户提的计划；
- B 是旧文档提出的重构目标，也不是用户这次说的计划；
- C 才是用户这次描述的「默认独立 LC、AP 可选编组」。

### 2.1 A 和 B 的具体差别

**目标 A：**

```text
Track 1 ─┐
Track 2 ─┼─> MIDI 通道 A01（乐器 + 推子 + insert + 发送 + CC 自动化）
Track 3 ─┘
```

- Track 的 `port/channel` 直接决定混音条；
- 同一 MIDI 通道的所有轨道共享乐器、推子、insert、发送、自动化；
- 没有独立的 LC 概念，也没有 Rack/LC 分层；
- 冻结/静音作用于整个 MIDI 通道；
- 优点是 256 条固定通道，资源可控；缺点是每轨没有独立混音。

**目标 B：**

```text
Track 1 ─┐
Track 2 ─┼─> Rack A01（乐器/样本/CC + 乐器自动化）─> LC（gain/pan/insert/send/freeze）─> Bus/Master
Track 3 ─┘
```

- Rack 只管乐器资源，不装推子/insert；
- LC 才装 gain/pan/M/S/insert/send/automation/freeze；
- 默认仍按 MIDI 通道共享：A01 的 Rack 输出一条 LC，所有 A01 轨道共享；
- 只有 Rack 选择 `PerTrack` 或外部插件多输出时，才会出现每轨/每输出 LC；
- 优点是音频通道条从 MIDI 通道解耦，可做 LC 级自动化/冻结/多输出；
- 缺点是默认仍不是“每条轨一个通道”，共享关系由 MIDI 通道决定。

**方案 C：**

```text
Track 1 ──> LC Rack + LC 100（默认）
Track 2 ──> LC Rack + LC 101（默认）
Track 3 ──> AP A01 通道 200（共享 Rack）
Track 4 ──> AP A01 通道 200
```

- 默认每条 Track 有自己的 LC（外部插件时也有自己的 LC Rack）；
- 只有显式链接 A01 时才进入共享 AP；
- AP 的音频出口就是 AP 通道，它和普通 LC 并列；
- 不存在 `AP -> LC` 的额外层级；
- 导出时独立 LC 自动占剩余 MIDI 通道。

因此 C 不是 B 的回滚：B 把「乐器资源」和「音频通道」拆开了，C 继续沿用这个拆分，
只是把「默认音频通道归谁」从 MIDI 通道改成 Track。

## 3. 和当前目标 B 的关系

当前目标 B 的链路：

```text
Track -> MIDI 源通道/Rack（固定）-> LC -> Bus/Master
```

Track 的 MIDI 通道决定 Rack，LC 是 Rack 的下游。默认所有轨道共享 256 条
MIDI 通道 Rack。

方案 C 的链路：

```text
Track ──> LC Rack -> 自己的 LC ──> Bus/Master       （默认）
   └────> AP 共享 Rack -> AP 通道 ──> Bus/Master       （手动链接）
```

变化只在「Track 的音频源归谁」：

- 默认 Track 有自己的 LC Rack + LC；
- 只有显式链接到 A01 时，Track 才加入 A01 的共享 Rack 和共享自动化；
- AP 通道和 LC 都可以直接进 Master 或另一条通道，二者没有父子关系；
- AP 编组只是共享 Rack 和 MIDI 身份的组织单位，不是音频通道。

### 3.1 和 Cubase / Studio One 的类比

方案 C 不是单纯的「Studio One 改回 Cubase」，而是两者的混合：

| DAW | 模型 | 和方案 C 的关系 |
|---|---|---|
| Cubase | MIDI Track、Instrument Rack、Audio/Group/FX Channel 分开；多输出乐器展开成多条音频通道；文件夹只做组织 | AP 编组接近 Instrument Rack，AP 通道接近多输出乐器展开出的音频通道 |
| Studio One | Track 自带乐器/通道条；多乐器可共享；Bus/FX Channel 做汇总 | 默认的 `OwnLc + LC Rack` 接近 Studio One 的 Track |
| 方案 C | 默认 Track 独立 LC；需要共享乐器时链接 AP | 默认像 Studio One，链接后像 Cubase 的 Instrument Rack |

所以：

- 如果绝大多数 Track 都是 `OwnLc`，它更像 Studio One；
- 如果大量 Track 链接到 A01..P16，它更像 Cubase 的「MIDI 轨 + 乐器机架」；
- AP 通道和 Cubase 的乐器输出通道一样，是真正的音频通道，不是虚拟容器。

真正的区别是：Cubase 的 Instrument Rack 通常先创建机架再让 MIDI 轨指向它；
方案 C 允许先有普通 Track，再一键「发送到 A01 编组」，迁移成本更低。

这不是回滚。可以复用的部分：

- `MixerNode` / `LogicalChannelId` / `RackParams` / `FrozenAudio`；
- LC 的 gain/pan/M/S/insert/send/automation/freeze；
- 通道发送/嵌套、通道自动化、Master；
- CLAP/VST3 多输出与按输出 LC；
- `ChannelLayout` 的 dense 通道、每轨 LC、多输出 LC；
- 自动化 lane 的编辑、undo、AR 子行、锚点面板。

需要改的是：

- `TrackData` 的音频目标从「隐式等于 MIDI 通道」变成显式的
  `TrackAudioTarget`；
- 默认不再有 256 条常驻 MIDI 通道 Rack/LC，改为按需创建 AP 和 Track LC；
- 自动化 owner 从「MIDI 通道」改为「TrackLc / ApRack / ApChannel」；
- 导出时增加「独立 LC → 剩余 MIDI 通道」的分配器；
- MIDI 导入时自动建立 AP 编组。

---

## 4. 核心数据模型（草案）

### 4.1 Track 的音频目标

```rust
struct TrackData {
    // 原有内容字段...
    audio_target: TrackAudioTarget,
}

enum TrackAudioTarget {
    /// 默认：普通 DAW 通道，拥有 LC Rack + 自己的 LC。
    OwnLc {
        lc: LogicalChannelId,
        rack: RackId,
    },
    /// 手动链接到 AP 编组：MIDI 进共享 Rack，音频走 AP 的某条通道。
    Ap {
        ap: ApGroupId,
        /// 该轨在 AP 多输出里的输出索引（单输出 = 0）。
        /// 实际音频通道 = ap.channels[output]。
        output: u16,
    },
}
```

说明：

- `OwnLc` 是默认值；新建 MIDI 轨/音频轨时创建或复用自己的 LC。
- `Ap` 不拥有私有 LC；它的音频路径就是 AP 编组里的某条 AP 通道。
- 从 AP 解链回 `OwnLc` 时保留原来的 LC/LC Rack 状态，
  避免解链后所有设置丢失。
- 音频轨（Audio Track）通常只用 `OwnLc`，不参与 AP 编组。

### 4.2 通道身份

```rust
/// LC 的稳定身份；u16 空间 0..=65535。
struct LogicalChannelId(pub u16);

/// AP 编组的稳定身份；当前上限 256，u16 预留扩展。
struct ApGroupId(pub u16);

/// AP 展开出的某一条音频通道。
struct ApChannelId {
    group: ApGroupId,
    output: u16,
}

/// 所有可发送目标的统一身份。
enum ChannelId {
    Lc(LogicalChannelId),
    Ap(ApChannelId),
    Master,
}
```

LC ID 是**独立命名空间**，不再编码“MIDI 源通道/音频通道号”。这和 v14 的
`LogicalChannelId(u32)` + `TRACK_LC_BASE` 不同，v14 迁移时必须重映射。

### 4.3 AP 编组与统一通道

```rust
struct ApGroup {
    id: ApGroupId,
    /// 对应 MIDI 通道；None = 用户手工建的无 MIDI 编组（可选）。
    midi: Option<(u8 /*port*/, u8 /*channel*/)>,
    rack: RackParams,
    /// AP 展开出的音频通道：单输出 = 1 条，多输出 = 每个输出一条。
    /// 这些是并列的 `MixerNode`，不是 AP 的子节点。
    channels: Vec<ChannelId>,
    /// AP Rack 级自动化（CC / 乐器参数）。
    /// 每条 AP 通道自己的 gain/pan/send/insert 自动化仍在 `MixerNode.automation`。
    rack_automation: Vec<AutomationLane>,
    enabled: bool,
}

struct MixerParams {
    /// 所有音频通道：普通 Track LC + AP 通道。
    channels: Vec<MixerNode>,
    /// AP 编组；只有用到的通道才创建。
    aps: Vec<ApGroup>,
    /// LC Rack；Track 解链后仍保留。
    racks: Vec<RackParams>,
    /// Master 是唯一特殊通道；也可以视作最后一个固定 ChannelId。
    master: MixerNode,
}

struct MixerNode {
    id: ChannelId,
    /// 音频来源；None = 纯汇入通道（Bus 角色）。
    source: Option<ChannelSource>,
    strip: StripParams,
    inserts: Vec<InsertRef>,
    sends: Vec<SendParams>,
    /// None = Master；Some = 送到另一条通道。
    output: Option<ChannelId>,
    /// 默认 true：任意通道都可以接收其他通道的 send。
    accepts_audio_input: bool,
    automation: Vec<AutomationLane>,
    freeze: Option<FrozenAudio>,
}

enum ChannelSource {
    LcRack(RackId),
    ApRack(ApGroupId),
    AudioInput,
}

struct SendParams {
    target: ChannelId,
    amount: f32,
    pre_fader: bool,
}
```

关键点：

- `channels` 里既有普通 LC，也有 AP 通道；它们都是 `MixerNode`。
- 没有独立的 `buses: Vec<MixerNode>`；“Bus”是 `source = None` 的通道角色。
- `accepts_audio_input` 默认 true（已确认 FL 式）；任意通道都能当 Bus。
- `sends`/`output` 的目标是 `ChannelId`，不再限制为 BusId。
- `master` 不接受音符/乐器，也不能作为 send 源，只作为终点。

### 4.4 设备链视图不是 Rack

当前代码里有两个容易混淆的东西：

- `RackParams`：持久化数据，存乐器链；
- 设备链视图：UI 把「乐器链 + LC 的 insert/send」合在一起展示。

设备链视图本身不是 Rack，也不是新的存储层。方案 C 里建议：

- LC 的设备链标题显示「LC 设备链」，内容 = LC Rack 的乐器 + LC 的 insert；
- AP 的设备链标题显示「AP 设备链」，内容 = 共享 AP Rack 的乐器 + AP 通道的 insert；
- 不再把 UI 上的设备链直接叫成“Rack”，避免把视图、持久化数据和共享关系混为一谈。

## 5. AP 与 LC 的职责边界

| 项目 | 普通 LC | AP 通道 | AP Rack / 编组 |
|---|---|---|---|
| 默认存在 | 是，每条 Track 一条 | 否，链接/导入 AP 时创建 | 否，按需创建 |
| 乐器/Rack | LC Rack（每轨自己的） | 共享 AP Rack | 共享乐器链 |
| MIDI 输入 | 只属于本 Track | 所有链接到该 AP 的 Track | 接收 AP 的 MIDI |
| gain/pan/M/S | 本 LC | 本 AP 通道 | 不装推子/insert |
| insert | 本 LC | 本 AP 通道 | 不装推子/insert |
| send | 本 LC | 本 AP 通道 | 不装推子/insert |
| CC/PB/RPN | 本 Track 的 LC owner | AP Rack owner | 共享 MIDI 自动化 |
| 乐器参数自动化 | LC Rack owner | AP Rack owner | 共享乐器参数 |
| freeze | 本 LC | 本 AP 通道 | 不冻结（冻结在 AP 通道） |
| 多输出插件 | 可以是一轨一条 LC | 每个输出一条 AP 通道 | 一个 Rack 展开多条通道 |
| 导出 MIDI | 分配剩余通道 | 按 AP 的 MIDI 通道 | 固定 MIDI 身份 |

关键规则：

1. **每个音频出口都是一条通道**：普通 Track 的 LC 或 AP 的 AP 通道；
2. **AP 编组不是通道**，它只持有共享 Rack、MIDI 身份和 AP 通道列表；
3. **AP 通道是共享音频流**；链接到该 AP 输出的 Track 不能再假装有独立推子；
4. **M/S/freeze 按通道分配**：AP 通道的 mute/solo/freeze 影响所有链接到该输出的 Track；
5. **Track M/S 是内容门**：只停该轨后续 MIDI，不等于通道 M/S；
6. **普通 LC 是完整独立通道**；默认行为与成熟 DAW 的普通音轨一致。

### 5.1 M / S / freeze 的作用单位

| 操作 | 作用单位 | 说明 |
|---|---|---|
| LC mute/solo | 该 LC | 只影响这条 Track 的音频 |
| LC freeze | 该 LC | 冻结这条 Track 的音频流 |
| AP 通道 mute/solo | 该 AP 通道 | 影响链接到该输出的所有 Track |
| AP 通道 freeze | 该 AP 通道 | 冻结这个共享输出；不按单条 Track 拆 |
| AP “全部冻结” | AP 的所有通道 | 只是 UI 批量操作，不是新的 owner |
| Track M/S | Track 内容 | 只停该轨 MIDI，不能抽走已混入 AP 的音频 |

所以“M/S/freeze 按 LC 分配”更准确的说法是：**按音频通道分配**。
普通 Track 的音频通道是 LC；AP 的音频通道是 AP 通道。二者同级。

---

### 5.2 多输出插件不是方案 C 的必需项

方案 C 的核心是“每条 Track 默认有自己的 LC/LC Rack”，所以最简单的多输出方案是：

```text
LC 1 -> LC Rack：插件实例 #1
LC 2 -> LC Rack：插件实例 #2
LC 3 -> LC Rack：插件实例 #3
LC 4 -> LC Rack：插件实例 #4
```

每条 LC 自己挂一个插件实例，各自收自己的 MIDI/音符，各自推子/insert/freeze。
这样：

- 不需要端口映射；
- 不需要冻结/自动化按输出拆分；
- 不需要改宿主对多输出插件的支持；
- 用户理解成本最低。

代价是插件实例数量增加。对轻量插件没问题；对 Kontakt 这类大采样器，
重复实例会重复样本内存。这时才需要可选的“单实例多输出”模式：

| 场景 | 推荐 |
|---|---|
| 普通合成器/效果器 | 多 LC，多实例 |
| 鼓机/采样器，样本库很大 | 可选多输出，一个实例拆多条 AP/LC 通道 |
| 插件本身是多声部但只有单输出 | 只能共享一条通道，或开多个实例 |
| 插件输出端口很多 | 默认不用；高级用户可手动映射 |

结论：

- **方案 C 的 MVP 不需要专门适配多输出插件**；
- 当前目标 B 已实现的多输出可以作为**可选高级能力**保留；
- 默认工作流应该是“多条 LC 手动挂插件 #1/#2/#3/#4”，而不是强迫用户理解
  output port 映射；
- 只有大采样器/鼓机确实需要省内存时，才引导用户使用多输出。

### 5.3 已确认：取消 Bus 类型，任意通道发送

这一条是方案 C 的关键决策：

```text
不再有独立的 Bus 类型。
LC、AP 通道、纯汇入通道全部是 Channel。
任何 Channel 都可以接收其他 Channel 的 send，
也可以把主输出指向另一个 Channel。
Master 是唯一终点。
```

也就是说：

```text
LC 可以当 Bus
AP 通道可以当 Bus
没有乐器的纯汇入通道也可以当 Bus
```

统一结构：

```rust
struct MixerNode {
    id: ChannelId,
    source: Option<ChannelSource>, // None = 纯汇入通道
    accepts_audio_input: bool,     // 默认 true
    sends: Vec<SendParams>,        // 目标可以是任意 Channel
    output: Option<ChannelId>,     // None = Master
    // strip / inserts / automation / freeze...
}
```

这样带来的效果：

1. **“Bus”只是角色**：`source = None` 的通道就是传统 Bus；有乐器的 LC 接收 send
   时就是 FL Studio 式混合通道（音符 + 汇入音频相加）。
2. **AP 也可以接收 send**：因为 AP 通道和 LC 同构，不再有“AP 不能当 Bus”的
   特殊规则。但 UI 要标明它同时是“共享源”和“汇入点”。
3. **SEND 用聚焦模式**：选中一个目标通道，其他通道各显示一个 dB 旋钮；
   不再为每条 Bus 固定显示旋钮。
4. **反馈检测统一**：任意通道发送意味着可能出现环；用与通道嵌套相同的
   “顺序发送同块、逆序发送一块延迟”策略，或统一拓扑排序 + 块延迟。
5. **Master 特殊**：Master 不接收音符/乐器，主输出仍是硬件终点；SEND 聚焦 UI 按需求显示 Master，但 Master 作为发送源默认禁用，避免反馈。

这会让方案 C 更接近 FL Studio 的“所有 Mixer Track 都能当 Bus”，但保留了
我们自己的 Track/LC 强绑定：Track 始终显式指向 LC 或 AP 通道，不存在
FL 那种 Playlist Track 与 Mixer Track 默认解耦的混乱。

## 6. 自动化与普通 CC 的合并

### 6.1 两类自动化

```text
Track 级：velocity、note expression、track-only 参数
Channel 级：CC、Pitch Bend、RPN/NRPN、乐器参数、LC gain/pan/send/insert
```

Track 级始终属于 Track；Channel 级属于当前音频目标的 owner：

- `OwnLc` -> `TrackLc(lc)`
- `Ap` -> `ApRack(ap)` + `ApChannel(ap, output)`
- 任意通道（原 Bus 角色）-> `Channel(id)`
- Master 不变

### 6.2 同名自动化允许存在，但渲染只认 owner

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

### 6.3 自动化和普通 CC 排在一起

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

## 7. MIDI 导入

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

## 8. MIDI 导出与通道分配

### 8.1 分配顺序

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

### 8.2 通道不足

三种策略，导出设置里选：

| 模式 | 行为 |
|---|---|
| `Fail` | 直接报错，列出需要的通道数 |
| `ReuseSafe` | 只复用没有 CC/自动化冲突的通道；有冲突则报错 |
| `Force` | 强制复用，导出后提示哪些轨道共享了通道 |

默认 `Fail`，避免黑乐谱导出后 CC 互相覆盖。

### 8.3 端口

当前导出只写低 4 位通道，没有 `MidiPort` meta。方案 C 第一阶段：

- 只使用 Port A 的 16 个通道；
- 需要超过 16 条独立通道时，再补 `MidiPort` meta，扩展到 A..P 共 256 个。

注意：不是所有 DAW 都保留 MIDI port，扩展端口前要先在导出选项里明确提示。

### 8.4 自动化写入

导出时对每个 owner 只写一次：

- AP owner 的 CC/PB/RPN 写到 AP 的 MIDI 通道；
- 独立 LC 的 CC 写到分配到的通道；
- 乐器参数自动化不写成 MIDI（如果插件参数不是 CC），保持为工程内设置；
- 同名 lane 先按第 5 节的 owner 解析合并。

---

## 9. UI / 交互

### 9.1 Track 头

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

### 9.2 快速链接

- 拖 Track 到 AP 编组头/AP 行即可链接；
- 右键 Track ->「链接到 MIDI 通道」；
- 选中多条 Track 后一次性链接到同一个 AP；
- 解链后回到原来的 `OwnLc`，不复制 AP 的自动化（避免二次污染）。

### 9.3 MIX 视图

- 普通 LC 按 Track 顺序显示；
- AP 通道单独显示为共享通道，条带上标明「A01 · 4 条轨共享」；
- 链接到 AP 的 Track 不再显示独立 LC 条，只显示它对应的 AP 通道；
- AP 多输出时每条 AP 通道各一条 strip；
- AP 通道的 freeze/mute/solo 明确提示作用范围；
- 通道面板可以像音轨面板一样选中，选中条用淡强调色，不加边框；
- 通道选择和音轨选择联动：选中一条 Track 时，它对应的 LC/AP 通道也选中；
- 多选时 SEND focus 仍只取一个目标（最后一次点击的通道）。

### 9.3.1 SEND 聚焦模式（按截图确认）

混音台不再为每条 Bus 显示一个固定旋钮，而是改成“选中目标，其他通道发送给它”：

```text
选中 A01
├─ A01 自己：不显示 SEND
├─ A02：显示 SEND → A01
├─ A10：显示 SEND → A01
└─ Master：显示 SEND → A01（特殊，见下）
```

规则：

1. **选中的通道是 SEND 目标**。该通道自己的条上不显示 SEND（不能发给自己）。
2. 其他所有可发送通道显示一个 SEND 旋钮，目标就是当前选中通道。
3. 切换选中的通道，所有 SEND 旋钮的目标一起切换；每个旋钮显示的是
   源通道到新目标的发送量，不是“所有 Bus 的旋钮”。
4. SEND 旋钮标签显示 **dB**，不再显示 `BUS 1`：
   - 最小 `-inf dB`（0 发送）；
   - 最大 `0.0 dB`（unity，1.0）；
   - 中间按 dB 映射，不按线性 0..1 显示。
5. 因为不再需要“每个 Bus 一个旋钮”，混音台取消 Bus 横向滚动；通道本身
   仍然可以横向滚动。
6. 高级多目标发送可以放在通道属性里，不作为混音台默认 UI。
7. Master 也显示 SEND（按你的要求），但 Master 的发送是特殊 tap：
   - 默认置灰/禁用，提示“Master 发送会形成反馈”；
   - 如果以后允许，必须走与自发送相同的反馈检测/一块延迟，并提供安全开关；
   - Master 作为目标时，其他通道正常显示 SEND → Master。

### 9.3.2 SEND 聚焦的数据与交互

```rust
struct MixerUiState {
    /// 当前选中的 SEND 目标；不是持久化混音数据。
    send_focus: Option<ChannelId>,
    /// 多选通道（用于批量 mute/solo/freeze 等）。
    selected_channels: Vec<ChannelId>,
}
```

- `send_focus` 只影响 UI 显示，不改变音频；
- 真正的发送量仍在每个 `MixerNode.sends` 里（按目标 ChannelId 存）；
- 旋钮从 `-inf` 拖起来才创建发送条目，拖回 `-inf` 删除条目；
- 切换 focus 时，旋钮值从各源通道的 `sends` 里读取目标对应的 amount；
- 目标没有 `accepts_audio_input` 或会形成环时，旋钮禁用并显示原因；
- SEND 的 dB 映射：`20 * log10(amount)`，`amount = 0` 显示 `-inf`；
- 当前 B 的发送量允许 0..2（+6 dB）；C 规范为 0..1（-inf..0 dB），
  v14 迁移时 >1.0 的值要 clamp 或提示，不能静默改变听感。

### 9.4 自动化面板

- Track 只显示 track-local lane + 当前 owner 的 channel lane；
- AP owner lane 在任一链接 Track 上都能编辑，但只有一份物理数据；
- 冲突点显示标记，并在 AP 头显示「N 个自动化冲突」。

---

### 9.5 文件夹、AP、Bus 不要混为一谈

这四个东西解决的是不同问题：

| 概念 | 作用 | 是否产生音频通道 |
|---|---|---|
| Folder | 视觉组织、折叠、批量选择/移动 | 否 |
| AP 编组 | 共享 MIDI 源/乐器/Rack | 是，展开成 AP 通道 |
| 纯汇入 Channel（Bus 角色） | 音频汇总、insert、send | 是，普通 Channel |
| Send | 把音频送到另一个 Channel | 否，是路由 |

**不建议让文件夹强制共用通道。** 原因：

1. 文件夹是组织工具，用户放 100 条轨进一个文件夹可能只是为了折叠；
2. 强制共用通道会把“组织”变成“混音路由”，很难解释；
3. 发送轨比 AP 更灵活：send 可以把同一份音频送到多个 Bus，而 AP 是
   MIDI 源的共享身份，两者不能互相替代；
4. 一旦“文件夹 = 通道”，就会被迫限制文件夹层数，否则层级和通道路由
   会纠缠不清。

推荐策略：

- 文件夹默认只做视觉组织，**不限层数**；
- 可选「文件夹作为 AP 编组」开关，默认关闭；开启后文件夹头就是 AP 头，
  子轨链接到该 AP，但文件夹本身不额外产生一条通道；
- 需要音频分组时，显式把文件夹/多条轨发送到一条纯汇入 Channel（Bus 角色）；
- 需要共享乐器时，显式链接 AP。

### 9.6 通道分配要像 Domino 一样方便

默认每条 Track 独立 LC 以后，“我的音符到底在哪个通道”必须一眼可见，
而且改起来要快：

- 轨道列表增加 `MIDI Out` 列（Port + Channel），支持排序/筛选；
- 多选轨道后一次改通道，或使用「下一个空闲通道」；
- 显示每个通道的占用数（`A01 · 12 轨`）和冲突警告；
- 提供「通道地图」面板：16 个通道 × 若干轨，拖拽分配；
- AP 编组头可以一键把选中轨链接到当前 AP；
- 导出前显示「独立通道 X / 可用 16」，不足时提前提示。

“烘焙通道”不建议自动做。它应该是显式命令：

```text
Bake to AP / Bake to LC
```

作用是把一组轨的共享关系固化、清理未使用 LC、或为导出/冻结做准备；
默认编辑过程中不做烘焙，避免用户失去“这条轨到底在哪”的控制权。

100 轨但只有 16 个 MIDI 通道并不矛盾：

- 16 个 Web MIDI 通道可以对应多个 AP 编组，但每个 AP 只占一个 MIDI 通道；
- 独立 LC 数量不受 16 限制，**导出时**才需要把它们压到 16/256 个通道；
- 如果确实需要 100 条独立通道同时导出，就用 Port A..P 扩展到 256 个，
  或者接受导出通道复用并在导出前解决 CC 冲突。

关键不是“把通道烘焙掉”，而是把 **轨道身份（Track/LC）** 和
**MIDI 导出身份（port/channel）** 分开。编辑时允许很多独立 LC，
导出时再按剩余通道自动排列。

## 10. 冻结、静音和独奏

| 操作 | 普通 LC | AP |
|---|---|---|
| 冻结 | 冻结本 LC | 冻结该 AP 通道（该输出组） |
| mute | 本 LC 静音 | AP 通道静音（链接到该输出的所有轨） |
| solo | 本 LC solo | AP 通道 solo |
| Track M/S | 内容门，停止该轨后续 MIDI | 同样只能停该轨 MIDI，不能抽走已混入 AP 的音频 |

这和当前目标 B 的语义一致，只是 LC 的默认归属从“MIDI 通道”变成“Track”。

### 10.1 FL Studio 的 freeze 是按 Mixer Track

FL Studio 官方手册把这件事叫 **Recording Internal Mixer Audio (Freezing Mixer
Tracks)**：

- 非实时：Playlist 的 Consolidate / 导出时的 “Split mixer tracks”，每个
  active Mixer Track 生成一个 wav；
- 实时：在 Mixer Track 上挂 Edison，或 arm 该 Mixer Track 做 disk recording；
- 单位始终是 **Mixer Track**，不是 Playlist Track/Pattern，因为 FL 的 Playlist
  和 Mixer Track 默认是解耦的。

这正好说明方案 C 的 freeze 单位应该是**音频通道**：

- 普通 Track 的 LC 是 1:1，所以看起来就是“冻结这条轨”；
- AP 是多条 Track 共享一条 AP 通道，所以只能冻结 AP 通道，不能冻结其中
  一条 Track；
- 不要为了“每轨 freeze”去复制 AP 的音频流，那会重新引入 FL 那种
  track/channel 解耦问题。

### 10.2 直通 Master 不是特殊轨道

方案 C 里所有通道默认就是：

```text
output = Master
```

所以“直通 Master 轨道”不需要新类型，也不需要新字段。普通 LC 默认直通
Master；AP 通道默认也直通 Master；只有用户显式改 output 或加 send 才进别的
通道。

如果担心用户误路由，可以加一个 `routing_lock` 标记：

```rust
struct MixerNode {
    // ...
    routing_lock: bool, // UI 禁止改 output/send；不改变音频路径
}
```

但不要把“直通 Master”做成一种轨道类型，否则又会出现“普通 LC / 直通 LC /
AP / Bus”四套语义。

---

## 11. 迁移

### 11.1 v14 -> v15

旧工程的行为必须保持不变：

1. 旧工程里每个实际使用的 MIDI 通道 -> 创建一个 AP 编组；
2. 旧 `MixerParams.channels[ch]` -> AP 通道；
3. 旧 `racks[ch]` -> AP 的共享 Rack；
4. 旧 `TrackData.port/channel` 对应的 Track 全部链接到 AP；
5. 旧 `TrackData.lc`（per-track / multi-output）-> `OwnLc` 或 AP 多输出通道；
6. 旧 `rack_output` -> AP 多输出索引；
7. 空通道不创建 AP，避免 256 条空编组；
8. 旧工程的冻结/自动化/insert/发送原样搬到 AP；
9. 旧 `buses` 全部转成 `source = None` 的普通 Channel，保留原来的
   strip/insert/send/freeze/meter；`BusId` 只作为迁移期旧 ID 使用；
10. 旧 `sends`/`output` 的 BusId 目标映射到新 ChannelId；旧 Bus 自动化
    （`BusGain` 等）迁移到该 Channel 的 `MixerGain`/节点自动化；
11. 旧 `LogicalChannelId(u32)` 全部重映射到新的 `u16` LC 空间；
    不能保留 `TRACK_LC_BASE=1_000_000`，也不能假设 LC ID = 源通道号。

这样打开旧工程听到的结果和 v14 一致，只是内部从“固定 MIDI 通道条”
变成了“AP 编组”。

### 11.2 新工程

- 新建 MIDI 轨：默认 `OwnLc`，LC Rack（或内置音源）；
- 新建音频轨：`OwnLc` + 音频输入；
- 手动链接 AP：共享 Rack；
- MIDI 导入：自动建立 AP 并链接。

### 11.3 兼容

- v15 加载器支持 v8..v15；
- 旧版本不能打开 v15，这是破坏性变更，需要在文档和发布说明中写清楚；
- 回滚方式：v15 保存前尽量保留一份 v14 兼容导出？不建议长期维护双写；
  更实际的是发布前备份工程，加载器只保证向后读。

---

## 12. 性能

默认每轨一条 LC，数量从“最多 256 条 MIDI 通道”变成“最多 Track 数”。

- 799 轨 -> 799 条 LC，`MixerGraph` 的通道缓冲约 `799 × 2 × frames × 4B`，
  在 512 帧下约 3.3 MB，可接受；
- 65536 轨满载时 LC 数量很大，必须懒分配：无音符/无音频/无自动化的空轨
  不创建 LC；
- 默认 LC Rack 如果每条轨都加载一个外部插件，实例数会爆炸。建议：
  - 默认使用内置音源（XSynth/YinheCpu）的 per-track 输出；
  - 只有用户手动选择外部插件时才创建私有插件实例；
  - AP 编组共享一个插件实例，是省资源路径。
- 多输出 AP 的每个输出一条 AP 通道，仍然比每轨一个插件实例便宜。

这是方案 C 最大的工程风险：**默认独立通道 + 外部插件 = 实例数量增长**。
需要产品上明确默认音源策略，不能假装所有轨都能无成本挂独立插件。

### 12.1 数量上限

| 对象 | 上限 | 说明 |
|---|---:|---|
| TrackData 总数 | 65536 | `Note.track: u16`，索引 0..=65535 |
| 可用内容轨 | 65535 | Master/Conductor 占 1 个轨道槽；Folder 也占槽 |
| LC ID 空间 | 65536 | `LogicalChannelId(u16)`；独立命名空间，不编码源通道号 |
| 同时活跃 LC | 当前受引擎限制 | CpuSynth 的 `MAX_CHANNELS = 256`；超过 256 的 dense 通道不会渲染 |
| AP 数量 | 256 | 当前 `global_channel: u8`，16 port × 16 channel |
| AP 的音频通道 | 每个 AP 1..N | 单输出 1 条；多输出插件按输出展开 |
| Bus 数量 | 无独立上限 | Bus 是 `source = None` 的普通 Channel；“Bus”不再是单独类型 |

几个容易混的点：

1. **Track 上限 65536 包含 Master/Conductor。** 当前 Master 同时承担 Conductor，
   所以内容轨最多 65535；如果未来 Master 和 Conductor 分成两条轨，就要减 2。
   Folder 也占 Track 槽位。
2. **LC 的 ID 上限和同时活跃上限不是一回事。** 可以有 65536 个稳定 LC ID，
   但音频引擎不可能同时为 65536 条通道分配 dense 缓冲。目标 C 需要定义
   “同时活跃 LC 预算”，空轨/静音轨懒分配。
   `u16` 只解决身份存储；不解决 65536 条同时发声。
3. **AP 上限 256 来自当前 `u8` 通道模型，不是 MIDI 文件理论的端口上限。**
   MIDI 本身是每个 port 16 个 channel，Port meta 也只有很有限的编号空间；
   65536 不是 MIDI 的标准通道数。
   SMF 可以写 Port meta，但工程和音频引擎当前只定义 16×16；提高到 4096/65536
   需要改 `global_channel` 类型、事件结构、导出/导入和 Domino 兼容，暂时没必要。
4. **CpuSynth 的 256 通道限制**是方案 C 必须先解决的工程问题：要么把内置音源
   改成按活跃轨动态通道，要么接受“同时最多 256 条 LC 发声”。不能只在文档里
   宣称支持 65536 条独立 LC。



---

## 13. 和当前实现的可复用度

| 现有模块 | 方案 C 是否复用 | 说明 |
|---|---|---|
| `MixerNode` / `StripParams` | 复用 | LC/AP 通道/纯汇入通道/Master 共用 |
| `LogicalChannelId` | 复用 | 每轨/AP 通道都是稳定 ID |
| `RackParams` | 复用 | AP 共享 Rack、Track LC Rack |
| `FrozenAudio` | 复用 | 普通 LC/AP 通道均可冻结 |
| `MixerGain/Pan/Send/InsertParam` | 复用 | owner 从 MIDI 通道改为 TrackLc/ApChannel |
| `ChannelLayout` | 复用 | dense 通道、每轨 LC、多输出已具备 |
| CLAP/VST3 多输出 | 复用 | AP 多输出展开多条 AP 通道 |
| 通道发送/嵌套 | 复用并泛化 | AP/LC/纯汇入通道都走同一发送图 |
| MIDI 导入/导出 | 改造 | 导入建 AP，导出分配剩余通道 |
| Track `port/channel` | 降级为属性 | 不再是混音路由的唯一依据 |

---

## 14. 分阶段实现建议

1. **数据模型 v15**：`TrackAudioTarget`、`ApGroup`、`MixerParams.channels/aps`；
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

## 15. 需要先确认的问题

1. 默认 LC Rack 用内置音源还是外部插件？外部插件按轨实例化成本很高。
2. 多输出插件保留为可选高级能力，还是方案 C 完全不做？
   本文建议保留但默认不暴露，默认让用户多建几条 LC 各挂一个实例。
3. LC 默认是否允许作为 Bus？已确认：允许，`accepts_audio_input` 默认 true，
   “Bus”只是 `source = None` 的通道角色。
4. 同 owner 同 target 的冲突，采用「最后编辑优先」还是让用户手动裁决？
   本文建议最后编辑优先 + UI 标记。
5. 独立 Track 数超过 16 时，第一阶段是否只支持 Port A 16 通道？
6. 解链 AP 时，AP 上的自动化是否复制回 Track，还是保持 AP、Track 回到空
   的 LC？本文建议不复制，避免同一条曲线出现两份。
7. 是否允许一个 AP 编组不对应 MIDI 通道（纯共享 Rack）？本文保留
   `midi: Option<...>`，但默认 UI 不暴露。

---

## 16. 结论

- 这是一个成立的方案，而且比目标 B 更接近「普通 DAW 默认行为」。
- 它不是回滚：`MixerNode`、LC、自动化、freeze、通道发送、多输出都能继续用；
  真正变的是 **Track 的默认音频源** 和 **AP/LC 的并列关系**。
- 不建议直接叫「方案 B」：仓库里已有目标 B，建议叫**方案 C（AP/LC 并列）**。
- 实现上最大的风险不是音频图，而是：
  1. 默认每轨 LC Rack 的插件实例成本；
  2. 同名自动化/CC 的 owner 解析；
  3. 导出时 MIDI 通道耗尽。
  这三点需要在动代码前先定策略。
