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
   C v10 迁移时重新分配，不能保留原数值。
2. **没有 Bus 类型，也没有 Bus 角色**。所有 LC/AP 通道都可以自由接收其他
   通道的发送；需要汇总时就把音频 send 到某条 LC/AP 通道，它同时保留自己的
   home source。Master 仍是唯一终点的特殊通道。
3. **直通 Master 是 Track 的第三种音频目标 `DirectMaster`（UI 显示 `---`）**。
   它不创建 LC/AP 通道，音符/音频直接进 Master；它不是一种通道类型，也不是
   “普通 LC 把 output 设成 Master”。普通通道默认 output 仍是 Master，但两者
   语义不同：普通通道有自己的 LC strip，`DirectMaster` 没有。
4. **freeze 按音频通道分配**：普通 Track 的 LC、AP 的 AP 通道。
   FL Studio 的 freeze 也是按 Mixer Track（音频通道）冻结，不是按 Playlist
   Track/Pattern；这恰好支持“按通道冻结”，但普通 Track 与 LC 是 1:1，
   所以表现上仍是“单轨冻结”。
5. **多输出插件暂不做，留作后期进阶功能**。方案 C 的 MVP 里 AP 编组只有
   一条输出、一条 AP 通道；需要多个独立音色/处理时，建多条 LC，各自挂一个
   LC Rack 实例。单实例多输出只在以后明确需要大采样器/鼓机省内存时再加。

### 版本号说明

本文写作时当前 HEAD 的混音段是 **v14**，但 v7..v14 全部是未发布开发版本；
最后发布的是混音段 **v6**（beta.1/2/3），容器是 **v10**。

方案 C 收口按 [`工程格式版本历史.md`](工程格式版本历史.md) 的建议：

```text
.yin 容器版本：10（保持）
混音段版本：  10（合并未发布的 v7..v14）
保留解码：    v9（最后开发基线）/ v10（C）
不保留：      v6（beta 工程需明确警告，或后续单独补 legacy）
```

下文为了描述当前代码，仍会提到 v14；涉及迁移/版本号时以本节为准。
“v14 → v15”应理解为“旧开发格式 → C 的 v10”。

6. **UI 命名**：普通 LC 显示为 `LC01`、`LC02`…（以 LC 命名，便于与 AP 区分），
   不显示内部 `LogicalChannelId`；AP 通道显示为 `AP01`、`AP10`…；
   `DirectMaster` 显示为 `--- (Master)`。
   通道条只显示身份，不写“几条轨共享”之类说明文字。
7. **ARRANGE 与 MIX 的 AR 不再互斥**：两者可以同时显示，分别服务轨道视图和
   通道视图；选择/自动化状态同步。

8. **移除 XSynth，保留 YinheCpu 作为内置音源**。LC Rack / AP Rack 可装外部插件
   （按轨实例化）；不装插件时由 YinheCpu 提供内置音源。XSynth 已无法满足本项目
   的 DAW 架构，正在移除。
9. **独立 Track 暂时都放 Port A**。导出时只在 Port A 的 16 个通道里分配；
   AP 仍可占用 A01..P16 中已有的 MIDI 通道。Port B..P 的独立通道扩展留到以后。
10. **解链 AP 不复制乐器自动化**。乐器参数自动化留在 AP Rack；CC/PB/RPN
    始终属于 Track。Track 回到 OwnLc 时恢复自己的 LC/LC Rack，不把 AP
    曲线复制过来。
11. **AP 必须对应 MIDI 通道**。不再允许“没有 MIDI 通道的纯共享 Rack AP”。
12. **版本解码保留 v9/v10**。C 的混音段最终为 v10；不再保留 v6/v7/v8 和
    v10..v14 中间开发格式（beta v6 兼容风险见版本历史文档）。
13. **冲突点用 UI 按钮裁决**。默认最后编辑优先；同一 owner/target/tick 有
    不同值时，在锚点/通道上显示冲突标记和“以哪条为准”的按钮。
14. **home LC 唯一且不记录来源**。每条 Track 拥有唯一的 home LC；链接 AP 时
    home LC 不参与音频路径。跨轨汇总用 send，不把两条 Track 的 home LC
    指向同一条。
15. **Track solo 是内容门**。只发被 solo 的 Track 的音符/CC/PB/RPN/音频 Clip；
    Channel solo 是音频门。Track/Channel 用三态听感状态，AR/MIX 都是单按钮
    （左键静音、右键独奏），solo 只在一层生效。
16. **插件沙盒提前**。C 期间先抽象 `PluginInstance` 边界；C 路由稳定后
    第一优先级做 tier 4（每插件二进制一个沙盒），tier 1 作为兼容回退，
    tier 5 按插件可选；不一次做 5 档。详见 5.14。

## 1. 术语

| 术语 | 含义 |
|---|---|
| Track | 内容容器：音符、音频片段、CC/clip 自动化 |
| LC | Logical Channel，普通 DAW 通道；拥有独立音频路径、推子、insert、send、automation、freeze；不记录来源类型 |
| Home LC | 一条 Track 自己的、唯一的源 LC；链接 AP 时非活动，不与其他 Track 共用 |
| AP 编组 | A01..P16 的 MIDI 源编组：共享 Rack、共享 MIDI 身份；本身不是音频通道 |
| AP 通道 | AP 编组展开出的音频通道（`MixerNode`）；方案 C MVP 固定 1 条；与 LC 并列 |
| AP Rack | 一个 AP 编组里的共享乐器链 |
| LC Rack | 一条独立 Track/LC 自己的乐器链（原“私有 Rack”） |
| Owner | 自动化归属者：Track（CC/clip）、Rack（乐器参数）、Channel（音频参数）、Master、Conductor |

AP 不是“父通道”，LC 不是“子通道”。它们是 Track 音频源的两种并列选择：

```text
Track 1 ──> LC01（默认，独立）
Track 2 ──> LC02（默认，独立）
Track 3 ──> AP01（共享）
Track 4 ──> AP01
```

关键区分：

- **AP 编组**是共享 Rack + MIDI 身份，本身不是通道；
- **AP 通道**是 AP 的音频输出通道，和普通 LC 一样是 `MixerNode`；
- 方案 C MVP 的 AP 只有 1 条 AP 通道；多输出留到后期；
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
| 自动化 owner | MIDI 通道 | Rack / LC | Track / Rack / Channel |
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
Track 1 ──> LC Rack + LC01（默认）
Track 2 ──> LC Rack + LC02（默认）
Track 3 ──> AP01（共享 Rack）
Track 4 ──> AP01
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
- 只有显式链接到 A01 时，Track 才加入 A01 的共享 Rack 和共享乐器参数自动化；CC 仍属于各 Track；
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
- CLAP/VST3 输出的当前 B 代码（C MVP 不依赖，后期再接）；
- `ChannelLayout` 的 dense 通道、每轨 LC；
- 自动化 lane 的编辑、undo、AR 子行、锚点面板。

需要改的是：

- `TrackData` 的音频目标从「隐式等于 MIDI 通道」变成显式的
  `TrackAudioTarget`；
- 默认不再有 256 条常驻 MIDI 通道 Rack/LC，改为按需创建 AP 和 Track LC；
- 自动化 owner 从「MIDI 通道」改为「Track / Rack / Channel」；
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
        /// MIDI 轨：Some(自己的 LC Rack)；音频轨：None（音频来自 clip）。
        rack: Option<RackId>,
    },
    /// 手动链接到 AP 编组：MIDI 进共享 Rack，音频走 AP 的唯一通道。
    /// 后期多输出时再增加 output 字段。
    Ap { ap: ApGroupId },
    /// FL Studio 的 `---`：不经过任何 LC/AP 通道，直接进 Master。
    /// UI 显示为 `--- (Master)`。
    DirectMaster {
        /// MIDI 轨：Some(LC Rack)；音频轨：None。
        rack: Option<RackId>,
    },
}
```

说明：

- `OwnLc` 是默认值；新建 MIDI 轨/音频轨时创建自己的 home LC，不与其他 Track 共用。
- `Ap` 链接期间，Track 的 home LC/LC Rack 不参与音频路径；音频走 AP 通道。
- 从 AP 解链回 `OwnLc` 时恢复原来的 LC/LC Rack 状态，避免解链后设置丢失。
- `DirectMaster` 的 MIDI 轨仍可拥有 LC Rack/乐器，但没有 LC 通道条：
  没有 LC 的 gain/pan/insert/send/freeze；只有源级音量/声像和轨道 M。
  音频轨的 `rack` 是 None。
- 音频轨（Audio Track）通常只用 `OwnLc`，不参与 AP 编组；也可以选择
  `DirectMaster` 绕过通道直接进 Master。

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
`LogicalChannelId(u32)` + `TRACK_LC_BASE` 不同，旧格式迁移到 C v10 时必须重映射。

### 4.3 AP 编组与统一通道

```rust
struct ApGroup {
    id: ApGroupId,
    /// 对应 MIDI 通道；AP 必须绑定一个 MIDI 通道。
    midi: (u8 /*port*/, u8 /*channel*/),
    rack: RackParams,
    /// 方案 C MVP：AP 只有一条音频通道。
    /// 后期如果做多输出，再扩展成 `channels: Vec<ChannelId>`。
    channel: ChannelId,
    /// AP Rack 级自动化（只含乐器参数；CC/PB/RPN 属于 Track）。
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
    /// 音频来源；LC/AP 通道的乐器/Rack，无独立汇入类型。
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
    /// 0..1；0 表示 OFF（-inf dB），不要保留 0 发送条目。
    amount: f32,
    pre_fader: bool,
}
```

关键点：

- `channels` 里既有普通 LC，也有 AP 通道；它们都是 `MixerNode`。
- 没有独立的 `buses: Vec<MixerNode>` 汇入通道表；所有通道都是 LC/AP。
- `accepts_audio_input` 默认 true（已确认 FL 式）；任意通道都能接收 send。
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
| CC/PB/RPN（AT/NRPN 以后） | 本 Track（MIDI 事件） | 本 Track（MIDI 事件） | 各 Track 的 MIDI 在 AP Rack 输入合并 |
| 乐器参数自动化 | LC Rack owner | AP Rack owner | 共享乐器参数 |
| freeze | 本 LC | 本 AP 通道 | 不冻结（冻结在 AP 通道） |
| 多输出插件 | 可以是一轨一条 LC | 每个输出一条 AP 通道 | C MVP 一个 Rack 一条 AP 通道；多输出后期再加 |
| 导出 MIDI | 分配剩余通道 | 按 AP 的 MIDI 通道 | 固定 MIDI 身份 |

关键规则：

1. **每个音频出口都是一条通道**：普通 Track 的 LC 或 AP 的 AP 通道；
2. **AP 编组不是通道**，它只持有共享 Rack、MIDI 身份和 AP 通道列表；
3. **AP 通道是共享音频流**；链接到该 AP 输出的 Track 不能再假装有独立推子；
4. **M/S/freeze 按通道分配**：AP 通道的 mute/solo/freeze 影响所有链接到该输出的 Track；
5. **Track M/S 是内容门**：M 停该轨内容，S 只发该轨内容，不等于通道 M/S；
6. **普通 LC 是完整独立通道**；默认行为与成熟 DAW 的普通音轨一致。

### 5.1 M / S / freeze 的作用单位

| 操作 | 作用单位 | 说明 |
|---|---|---|
| LC mute/solo | 该 LC | 只影响这条 LC 的音频 |
| LC freeze | 该 LC | 冻结这条 LC 的源；混合 LC 不冻结 |
| AP 通道 mute/solo | 该 AP 通道 | 影响链接到该输出的所有 Track |
| AP 通道 freeze | 该 AP 通道 | 冻结这个共享输出；不按单条 Track 拆 |
| AP “全部冻结” | AP 的所有通道 | 只是 UI 批量操作，不是新的 owner |
| Track M/S | Track 内容 | M 停该轨内容，S 只发该轨内容；不能抽走已混入 AP 的音频 |

所以“M/S/freeze 按 LC 分配”更准确的说法是：**按音频通道分配**。
普通 Track 的音频通道是 LC；AP 的音频通道是 AP 通道。二者同级。

---

### 5.2 多输出插件：C MVP 不做，留作后期进阶

方案 C 的核心是“每条 Track 默认有自己的 LC/LC Rack”，所以最简单的做法是：

```text
LC01 -> LC Rack：插件实例 #1
LC02 -> LC Rack：插件实例 #2
LC03 -> LC Rack：插件实例 #3
CH04 -> LC Rack：插件实例 #4
```

每条 LC 自己挂一个插件实例，各自收自己的 MIDI/音符，各自推子/insert/freeze。
这样：

- 不需要端口映射；
- 不需要冻结/自动化按输出拆分；
- 不需要改宿主对多输出插件的支持；
- 用户理解成本最低。

#### 多输出到底是什么

多输出不是“多个 MIDI 通道”，也不是“多个乐器”，而是：

> 一个插件实例有多个音频输出插座。

普通插件只有一个立体声输出：

```text
插件实例 -> L/R
```

多输出插件像一个鼓机：

```text
插件实例
├─ Out 1 (L/R) -> LC01
├─ Out 2 (L/R) -> LC02
├─ Out 3 (L/R) -> LC03
└─ Out 4 (L/R) -> CH04
```

插件内部把 kick/snare/hat 分别送到不同输出；宿主可以把每个输出接到不同
混音通道。它省的是**插件实例的 CPU/样本内存**，不是 MIDI 通道。

#### 为什么 C 暂时不做

- 黑乐谱/电子音乐通常是一个大音源一条主输出，多输出收益很低；
- 它更像 ARA：功能很强，但和主要制作流程关系不大，真正需要的人可以用
  别的 DAW；
- 概念成本高：输出拓扑、端口映射、每输出通道、冻结/自动化、VST3 多 bus、
  导入迁移都要单独处理；
- 不做它，AP 就永远只有一条通道，模型简单很多。

#### 不做能省多少代码

按当前两次多输出相关提交统计：

```text
8ff4ae49 外部插件支持按输出 LC   约 1070 insertions / 42 files
eb443dd9 VST3 接入多输出总线     约 112 insertions / 3 files
```

合计约 1180 行新增，包含测试、15 个 locale 和 v14 迁移；其中核心运行时代码
大约 500~700 行。更重要的是省掉这些概念：

```text
RackOutput / RackOutputTopology
RackOutputMode::PerOutput
RackParams.outputs / output_lcs
TrackData.rack_output
InstrumentProcessor::process_outputs
InstrumentSource.output_dense
render_instruments_multi / multi_scratch
CLAP audio_outputs / 多端口拷贝
VST3 多输出 bus 缓冲
AP 多输出 UI / topology rescan
```

C MVP 只需要：

```text
一个 AP 编组 -> 一个 AP 通道
一个 Track -> 一个 LC 或一个 AP 通道
InstrumentProcessor 只处理主立体声输出
```

#### 兼容性代价

如果确定不做，当前 B 的多输出代码可以删；但 v14 工程可能已经保存了
`PerOutput` / `output_lcs`：

- 删除前必须保留 v14 解码器；
- 旧多输出工程要么在 C v10 里降级为主输出并明确警告，要么保留一条只读的
  legacy 多输出路径；
- 不能静默丢输出。如果功能刚上线、没有真实用户工程，可以接受一次性
  迁移警告；否则 legacy 路径必须保留。

#### 以后要加时

C 的单输出 AP 结构可以扩展：

```rust
struct ApGroup {
    // MVP
    channel: ChannelId,
    // 以后
    // channels: Vec<ChannelId>,
}
```

但更推荐等需求明确后再加，不要为了“以后可能用”提前把 C 的模型复杂化。

### 5.3 已确认：没有 Bus，任意通道发送

这一条是方案 C 的关键决策：

```text
没有 Bus 类型，也没有“Bus 角色”。
LC、AP 通道全部是 Channel。
任何 Channel 都可以接收其他 Channel 的 send，
也可以把主输出指向另一个 Channel。
汇总就是“send 到某条 LC/AP 通道”，那条通道同时有自己的源。
Master 是唯一终点。
```

也就是说：

```text
LC 可以接收 send 做汇总
AP 通道也可以接收 send 做汇总
任意通道都能既是源也是汇入点
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

1. **没有独立汇入通道**：接收 send 的通道就是普通 LC/AP，音符和汇入音频
   在它自己的通道条上相加。
2. **AP 也可以接收 send**：因为 AP 通道和 LC 同构。UI 只需显示通道身份，
   不写“几条轨共享”之类说明。
3. **SEND 用聚焦模式**：选中一个源通道，其他通道各显示一个「源 → 本条」的 dB 旋钮。
4. **反馈检测统一**：任意通道发送意味着可能出现环；用与通道嵌套相同的
   “顺序发送同块、逆序发送一块延迟”策略，或统一拓扑排序 + 块延迟。
5. **Master 特殊**：Master 不接收音符/乐器，主输出仍是硬件终点；Master 不能作为
   发送源，选中 Master 时 SEND 旋钮全部隐藏，避免反馈。
6. **进 Master 只有 send 一条路**：每条通道默认带一条 0 dB 的 Master send，
   删除原来的直连 Master 累加，避免「直连 + send」叠成双份。

这会让方案 C 更接近 FL Studio 的“所有 Mixer Track 都能当 Bus”，但保留了
我们自己的 Track/LC 强绑定：Track 始终显式指向 LC 或 AP 通道，不存在
FL 那种 Playlist Track 与 Mixer Track 默认解耦的混乱。

### 5.4 home LC 唯一，音频汇总走 send

每条 Track 有且只有一个 **home LC**，ID 唯一，不与其他 Track 共用；链接 AP
时这条 home LC 不参与音频路径：

```text
MIDI Track A -> LC01（home）
Audio Track B -> LC02（home）--send--> LC01
```

- LC01 是 A 的 home LC，B 不能把 home LC 也设成 LC01；
- B 要参与 LC01 的汇总，只能把音频 send 到 LC01；
- LC01 只看到“有音频输入”，不记录来源是 MIDI 轨还是音频轨；
- 来源类型只存在于 Track，LC 是 source-agnostic 的音频通道。

LC 不存 `source_kind`，也不存 `owner_kind`。来源关系由 Track 的 `target`
和 send 图表达。

因此“合并到同一条 LC”只有两种合法方式：

1. **Send 汇总**：B 的 home LC2 `send -> LC01`，音频在 LC01 相加；
2. **Bounce**：把多条 Audio Track 的片段离线混合成一个新音频文件，
   再放到一条 Audio Track/LC 上。这是显式操作（Bounce / Consolidate），
   不是编辑时的默认行为。

不能把两条 Track 的 home LC 指向同一条。Track 是内容，LC 是通道；能共享的
是音频汇总，不是 home LC 身份。

没有 Bus：接收 send 的通道就是普通 LC/AP，它同时有自己的 home source。

home LC 也可以接收 send；这时它是混合 LC，按 10.3 不参与单通道 freeze。

UI 只显示 LC 身份（`LC01`/`LC02`），不写“几条轨共享”的说明。

### 5.5 LC 没有鼓通道

确认：**LC 不需要鼓通道。**

- LC 是音频通道，不是 MIDI 通道；
- 鼓通道是 MIDI 语义：A10 / AP10；
- 需要 GM 鼓/共享鼓通道时，把 Track 链接到 **AP10**；
- LC Rack 里可以加载鼓插件，但那是插件音源，不是 MIDI 鼓通道语义；
- 不要给 LC 增加 `drum_channel` 字段，否则会把 MIDI 通道概念又塞回音频通道。

### 5.6 AP 和 LC 的本质区别

如果 LC 也能接收很多音频，它和 AP 到底差在哪？

答案不是“能不能汇总”，而是**汇总发生在音频链的哪一侧**：

```text
AP：先分组 MIDI，再合成
Track 1 ─┐
Track 2 ─┼─ MIDI/CC ─> 共享 Rack（一个乐器实例）─> AP 通道
Track 3 ─┘

LC 汇总：先各自合成，再 send 到某条 LC
Track 1 -> LC Rack -> LC 1 ─┐
Track 2 -> LC Rack -> LC 2 ─┼─ send ─> LC 3（汇总通道）
Track 3 -> LC Rack -> LC 3 ─┘
```

| 维度 | AP | LC |
|---|---|---|
| 分组对象 | MIDI 音符/CC | 音频流 |
| 共享乐器 | 是，一个 Rack 实例 | 否，各轨自己的 LC Rack |
| 共享样本/voice | 是 | 否 |
| MIDI 通道身份 | 有，绑定 A01..P16 | 没有 |
| 默认每轨独立 | 否，链接才共享 | 是 |
| 接收 send | 可以，但它首先是 MIDI 源 | 可以，普通 LC 同时做源与汇入 |
| freeze 单位 | AP 通道（共享输出） | 源 LC 或汇总 LC（见 freeze 规则） |

所以 AP 的意义不只是“指定通道”：

- 它指定 MIDI 源身份；
- 它决定哪些 Track 共享同一个乐器实例；
- 它共享乐器参数自动化和乐器内部状态；CC 在 Rack 输入按时间合并；
- 它共享 voice pool、样本内存、乐器内部状态；
- 它的音频输出才是一条 AP 通道。

LC 汇总只共享音频，不会让多条 Track 共享乐器。两者可以叠加：

```text
Track -> AP01 -> AP 通道 -> send -> LC10（汇总）-> Master
```

### 5.7 不建议让 LC 变成“多条通道的容器”

“LC 也可以有多条通道”听起来灵活，但会制造和 AP 重复的层级：

- LC 如果包含多条子通道，它就成了一个 Group 容器；
- 再让 LC 做 freeze，就要回答“冻结整组还是单条”；
- 再让 LC 做 AP 式的 MIDI 共享，它就和 AP 没区别；
- 层级越深，Track 到底在哪个通道越难理解。

推荐保持**扁平通道图**：

```text
Track -> Channel（LC 或 AP 通道）-> Channel ... -> Master
```

需要分组音频就用 send 到一条 LC；需要分组 MIDI/乐器就用 AP。
不要让 LC 再包一层 LC。这样 freeze、自动化、路由都只有一个维度。

### 5.8 SEND 是音频，不是 MIDI

SEND 只搬运**音频**，不会把音符送到目标通道的乐器。

```text
Track A（无乐器，只有音符）
Track B（无乐器，只有音符）
        │
        └─ SEND（音频）─> LC10（有乐器）
```

结果：LC10 的乐器**不会**因为收到 SEND 而播放 A/B 的音符。
A/B 没有自己的音频源，所以 SEND 送过去的是静音；LC10 仍然只播放它自己的
MIDI。

要让 A/B 的音符去弹 LC10 的乐器，需要的是 **MIDI 链接到 AP**：

```text
Track A ─┐
Track B ─┼─ MIDI ─> AP10（共享 Rack/乐器）─> AP 通道
```

注意默认值：

- C 的默认 LC Rack 使用内置音源，所以普通 Track 一般是有声的；
- 只有显式把 LC Rack 清空/禁用、且没有其他源时，才会出现“有音符但无音频”；
- AP 通道的乐器只处理 MIDI，不处理 SEND 进来的音频；SEND 进来的是音频，
  会与 AP 通道自己的输出相加，不会触发 AP 乐器。

### 5.9 `-inf dB` 的 SEND 必须是 OFF

`-inf dB` 不是“很小的声音”，而是 **0 发送、OFF 状态**。

规则：

- UI 上 SEND 在 `-inf` 时不创建 `SendParams` 条目；
- 从 `-inf` 往上拖才创建发送；拖回 `-inf` 删除条目；
- 引擎只处理 `amount > 0` 的发送，0 发送直接跳过；
- 因此“显示了很多 SEND 旋钮”不会带来多轨性能开销，只有真正激活的发送
  才有音频求和成本；
- 不要用 `f32::MIN_POSITIVE` 之类的极小值表示 OFF，避免每块仍做无意义
  的乘加和缓冲遍历。

### 5.10 Insert 是串行效果器链

Insert 就是挂在**当前通道**上的效果器链，不是乐器，也不是发送。

```text
源（LC Rack / 音频输入 / AP Rack）
  -> insert 1 -> insert 2 -> insert 3
  -> fader
  -> pre/post fader send
  -> output / Master
```

区别：

| 概念 | 作用 | 位置 |
|---|---|---|
| Rack 乐器 | 产生声音 | 通道源 |
| Insert | 处理当前通道声音，100% 串行 | 源之后、推子之前 |
| Send | 把当前通道声音并行送到另一个通道 | insert/推子之后（pre/post 可选） |
| 汇总 LC | 接收多条 send 的 LC | 目标通道 |

Insert 可以是 CLAP/VST3 效果器，也可以是内置效果器；它们都实现
`InsertProcessor`。AP 通道、普通 LC、纯汇入 Channel、Master 都有自己的
insert 链。

Freeze 和 insert 的关系：

- freeze 捕获的是 **pre-LC 源**，insert 仍实时处理；
- 所以冻结后仍可调 insert；
- 纯汇入 LC（没有自己的源）insert 处理的是汇入音频，因此不做单通道
  freeze。

### 5.11 SEND 的延迟与 PDC（Plugin Delay Compensation）

SEND 是音频路径，会涉及延迟，但要分成三类：

1. **前向发送**：目标通道在当前通道之后处理。拓扑排序正确时，同一块内
   完成，不额外加一块延迟。
2. **反向/自环发送**：目标已经处理过，或发送形成环。用上一块输出快照，
   增加**一块延迟**；这是为了打断反馈，不是普通 SEND 的固有延迟。
3. **插件延迟**：源通道/目标通道的 insert 或乐器有 latency。需要用 PDC
   （Plugin Delay Compensation，插件延迟补偿）对齐，否则并行路径会出现相位偏移。

方案 C 的约束：

- 通道图应当按发送关系做拓扑排序；DAG 内前向发送同块完成；
- 只有真正的环才用一块延迟，不能因为“目标索引小”就随便延迟；
- PDC 需要从“只对齐通道到 Master”升级为“在任意汇入点对齐”；
  至少源通道的乐器/insert 延迟要参与，AP/LC 通道的 insert 延迟也要参与；
- 如果 MVP 暂不做任意发送图的完整 PDC，必须明确写出“复杂并行发送可能
  有相位差”，不能假装没有延迟；
- Sidechain 发送不进入可听路径，但它的时间对齐同样重要。

结论：**普通前向 SEND 不额外增加一块延迟；插件延迟由 PDC 处理；只有反馈环
用一块延迟。** 任意通道发送的完整 PDC 是 C 落地时必须补的工程项。

### 5.11.1 PDC 的实施顺序与当前缺口

PDC 会做，但不放在 C 的第一步。顺序是：

1. **路由正确**：统一 Channel 图、拓扑排序、`-inf` OFF、发送求和正确。
   这一步结束时功能可用，但并行路径可能有相位差。
2. **PDC 对齐**：按图计算每个求和点的输入延迟，短路径补延迟线。
   覆盖乐器 + insert，包含 AP/LC 通道的 insert。
3. **对外表现**：UI 显示延迟总量、导出补偿 Master limiter 延迟。

先把 PDC 做在路由前面会反复重写：延迟对齐依赖稳定的通道图和拓扑顺序。

计算方式：

```text
node_latency = 乐器延迟 + 该通道 insert 链延迟
path_latency(src -> node) = 路径上所有 node_latency 之和
每个求和点：把所有输入延迟到 max(输入 path_latency)，再相加
```

方案 C 已落地的现状：

- `refresh_pdc` 已按「每个求和点分别对齐」实现：通道 send / 主输出图
  收集前向边，逐轮松弛求每个节点的最长输入路径，再给自身源与每条边
  补延迟线；
- 没有 Bus 渲染链：发送目标就是另一条 LC/AP dense 通道或 Master，
  不再有独立的 `process_buses`；
- 逆序/自环发送用上一块快照（一块延迟），这是打断反馈的块延迟，
  不属于 PDC，也不能被 PDC 补偿。

性能边界：

- 延迟为 0 时不分配延迟线，零延迟插件不增加成本；
- 只有真实存在并行路径且延迟不同时，才给短路径分配缓冲；
- 重算只在命令线程、拓扑或插件延迟变化时发生，不在每个音频块里做。

### 5.12 常见 DAW 的 MIDI SEND 还有意义吗？

有意义，而且 MIDI 路由本身不限制方向：

```text
一对多：一条 Track -> 多个乐器/目标
多对一：多条 Track -> 一个乐器/目标（AP 就是这种）
多对多：多条 Track 各自送到多个目标
```

在 C 里：

- **AP** 是“多对一”：多条 Track 共享一个 MIDI 源/Rack；
- **MIDI Send** 是“一对多”：一条 Track 把 MIDI 复制到额外目标；
- 两者同时存在时，自然形成多对多。

典型用途：

- 同一段音符同时驱动两个不同音色（layering）；
- 送到 MIDI 效果器/琶音器，再进另一个乐器；
- 一个轨同时控制主音色和低八度/和声层；
- 不复制 1 亿音符数据，也能让多个 Rack 收到同一份 MIDI。

但 C MVP 不建议做，原因：

- AP 已经覆盖黑乐谱最主要的“多轨共享一个乐器”；
- MIDI Send 会让一个 Track 同时属于多个 MIDI 目标，Track 的 MIDI 身份
  不再是单值；
- CC/自动化 owner 会变复杂：源轨的 CC 是复制给所有目标，还是只给主目标？
- 复制音符和控制器会放大黑乐谱的性能问题。

如果以后加，建议做成显式的：

```rust
struct MidiSendTarget {
    ap: ApGroupId,
    /// 可选的通道重映射/移调/力度缩放。
    // ...
}
```

每条 Track 有自己的主 MIDI 目标（LC/AP/DirectMaster），MIDI Send 只是额外
复制，不改变主归属。默认不启用。

### 5.12.1 一轨多 MIDI 通道

MIDI 文件允许一条 SMF track 内出现多个 channel，但 Yinhe 当前模型是：

```text
TrackData -> 一个 port/channel
Note.track -> Track 索引，不带每音符 channel
```

所以 MVP 建议明确：

- **一条 Track 只对应一个 MIDI 通道**；
- 一条 SMF track 如果包含多个 channel，导入时拆成多条 Track（每条一个
  channel），可以放进同一个文件夹做视觉归组；
- 不支持“一个 TrackData 内的音符各自属于不同 MIDI 通道”；
- 如果以后真要做，必须给 Note 增加 channel 或增加 per-note 子通道，
  这是数据量和 UI 的另一个量级，不建议现在做。

“表面一条轨、逻辑上 A01/A02/A03 三个通道”可以用：

```text
文件夹
├─ Track A01 -> AP01
├─ Track A02 -> AP02
└─ Track A03 -> AP03
```

文件夹只是视觉归组，不改变每条 Track 的 MIDI 通道。不要把它做成一条
多通道 Track。

### 5.13 Insert 不含乐器

对，**Insert 不含乐器**。

```text
RackParams.instruments = 产生声音的乐器（Rack 乐器）
MixerNode.inserts      = 处理声音的效果器链（Insert）
```

- Insert 处理的是已经有声的音频；
- Insert 不会接收音符，也不会“生成”声音；
- AP Rack / LC Rack 才是乐器；
- 有些插件既能生成声音又能处理音频，但挂载时必须选一个角色：
  - 放 Rack：当作乐器，接收 MIDI；
  - 放 Insert：当作效果器，处理音频；
- AP 通道、LC、Master 的 insert 都是效果器链；
- Sidechain 是 Insert 的额外音频输入，不是乐器。

### 5.14 Rack 的内存共享与插件沙盒

Rack 会不会共用已加载的音色库内存？要分三层：

1. **音色库样本数据**：内置 SFZ/SoundFont 走 `GLOBAL_SF_CACHE`，按
   `(path, sample_rate, interpolation)` 缓存 `Arc<dyn SoundfontBase>`。
   多条 Track/多个 Rack 用同一音色库时，样本数据只加载一份。
2. **乐器实例状态**：voices、envelope、filter、插件内部状态属于 Rack 实例。
   LC Rack 每轨一个实例，互不共享；AP Rack 所有链接轨共用一个实例。
3. **外部插件**：VST3/CLAP 的样本/内存由插件自己管理，宿主不能保证多个
   实例共享。AP 共享一个实例能省内存/CPU；多个 LC Rack 实例各加载各的。

宿主能做的：

- 自己的采样器：共享样本缓存（已做）；
- 外部插件：共享插件二进制/代码页；用 AP 复用同一个实例；
- 给第三方插件提供可选的共享样本 API（需要插件配合）。

宿主不能做的：

- 强制两个第三方实例共享内部堆/sample buffer；VST3/CLAP 没有通用的
  “共享样本内存”接口；
- 把一个实例的内部状态复制给另一个实例而不重新加载。

所以：

```text
同一 SFZ 音色 + 100 条 LC Rack
  -> 样本内存一份
  -> 100 份 voice/插件状态

同一 AP Rack + 100 条 Track
  -> 样本内存一份
  -> 1 份 voice/插件状态
```

**插件沙盒提前，但不塞进 C 的路由内核。** 当前 CLAP/VST3 都在音频引擎
进程内加载。Bitwig 式的分层沙盒（引擎内 / 全局沙盒 / 每厂商 / 每插件 /
每实例）是进程隔离，不是样本内存共享：

- 沙盒解决插件崩溃、内存越界、卡死；
- 不解决“多个实例重复加载样本”，除非插件自己共享；
- 每实例沙盒内存最大、IPC 最多；每插件沙盒是比较合适的默认档。

路线：

1. C 期间先把 `PluginInstance` 的音频块和参数命令抽象成可替换传输
   （共享内存 + 无锁队列），不要现在把 in-process 调用写死在混音图里；
2. C 路由稳定后，第一优先级做 **tier 4：每个插件二进制一个沙盒进程**，
   多个实例共用该进程；
3. **tier 1（引擎内）保留为兼容回退**，某些插件/调试场景需要；
4. **tier 5（每实例一个沙盒）** 只对已知不稳定的插件开放，不作为默认；
5. 不建议一次做全 5 档；tier 2/3 没有明确用户价值前不做。

性能：

- 音频块走共享内存，开销按块算，不按采样算；48 kHz / 256 帧一块约
  5.3 ms，立体声 2 KB 拷贝可以忽略；
- 真正的成本是进程调度抖动、额外一块安全延迟、进程内存和插件兼容性；
- 音频线程不能阻塞在 IPC 上，必须用无锁队列/共享内存；进程唤醒失败时
  要能静音插件并继续跑主引擎；
- 沙盒崩溃恢复要保留插件参数和工程状态，不能丢工程。

C 的音频路由内核仍保持单进程、低延迟；沙盒是插件宿主层的第一优先级，
不改变每轨 LC 的默认路径。

## 6. 自动化与普通 CC 的合并

### 6.1 三类自动化

```text
Track 级：CC、Pitch Bend、RPN（AT/NRPN 以后实现，归属不变）、velocity、note expression、clip 参数
Rack 级：乐器参数（cutoff、resonance、振荡器、插件参数）
Channel 级：gain、pan、send、insert 参数
Master / Conductor：master 参数、Tempo
```

归属规则：

- **CC/PB/RPN 是 MIDI 事件（AT/NRPN 以后实现，归属相同）**，属于 Track。它们跟着 Track 的 MIDI 走；
  多条 Track 链接到同一 AP 时，在 AP Rack 输入按时间合并。
- **乐器参数是 Rack 状态**，属于 Rack。`OwnLc` -> LC Rack；
  `Ap` -> AP Rack。AP Rack 的乐器自动化被所有链接轨共享。
- **音频参数是 Channel 状态**，属于 LC/AP 通道/Master，不是 Track，
  也不是 Rack。
- **不要**把同一个参数同时存到 Track 和 Rack；解析时只有一个 owner。

这解决了“LC 不知道来源”的问题：LC 只存音频参数；乐器参数在 Rack；
CC 在 Track。三者不会混。

#### 6.1.1 成熟 DAW 怎么放自动化

| DAW | 自动化放在哪 | 和 C 的对应 |
|---|---|---|
| Ableton Live | Track 同时是内容、通道、设备宿主；automation 在 Track，目标是设备参数或混音参数 | 三层被合成一层，所以看起来都在 Track |
| FL Studio | Channel Rack 管乐器/CC，Mixer 管推子/发送；automation clip 可链接任意参数 | 自由链接，owner 由链接目标决定 |
| Studio One | Track 承载乐器/效果，Track 就是 Mixer Channel；设备参数在设备，混音参数在通道 | 类似 Live，Track = Channel |
| Cubase | MIDI 轨管 CC；Instrument Track / Rack Instrument 管乐器参数；Group/FX 通道管混音参数 | 最接近 C：Track / Rack / Channel 三层 |
| Bitwig | Track 承载 devices；设备参数在 device，混音参数在 track | 类似 Live，Track = Channel + device host |

共同规律：**自动化跟着“参数所属对象”走**，不是一律挂在 Track 上。
Live/Studio One/Bitwig 把 Track、Channel、设备合成一层，所以看起来像
“都在 Track”；Cubase 的 Rack Instrument 则明确把乐器参数放在 Rack 上。

C 只是把被“Track = Channel”隐藏的三层显式化，并没有发明更复杂的规则。
UI 上按对象分开放：AR 看 Track 的 CC/clip，MIX 看 Channel 的 gain/pan/
send/insert，设备面板看 Rack 的乐器参数。

### 6.2 同一 owner 的 lane 冲突解析

同一个 owner 下可能因为导入/复制出现多条同名 lane，渲染和导出前做一次
**owner 解析**：

```text
(owner, target, tick) -> 最终值
```

推荐策略：

1. 完全相同 `(owner, target)` 的 lane 归为一组；
2. 同一 tick 值相同 -> 直接合并，无冲突；
3. 同一 tick 值不同：
   - 记录 `edit_epoch`（最后一次用户编辑的序号）；
   - 渲染/导出取 `edit_epoch` 最大的点；
   - UI 在冲突点显示标记，并允许用户选择「以哪条为准」；
4. 合并结果只写一次到引擎/导出。

CC/PB/RPN 不进入这个 owner 解析（以后实现的 NRPN/AT 同理）：它们各自属于 Track，多个 Track 在 AP
Rack 输入按时间合并。同一 tick 的不同 CC 值按 Track 顺序稳定排序，并在
UI 标出冲突；不把两个 Track 的 CC 合成一条 lane。

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
6. 导入的 CC/Program Change 留在各自 Track 的 MIDI 事件流；
   乐器参数不来自 MIDI，只有工程内自动化才落到 Rack。

这样导入后：

- 同一 MIDI 通道的轨道保持原来的共享乐器；CC 在 AP Rack 输入按时间合并；
- 不同 MIDI 通道仍然是不同 AP；
- 用户可以把 AP 里的某条 Track 解链，变成独立 LC，不影响其他轨道。

当前解析器已经按 `port << 4 | channel` 建轨，并在 `group_automation_events`
里按通道归组；方案 C 主要改“导入后建立 AP 并链接”，不需要重写 MIDI 解析。

---

## 8. MIDI 导出与通道分配

### 8.1 分配顺序

导出前建立通道表：

1. 已链接 MIDI 的 AP 编组占用其固定通道；
2. 独立 `OwnLc` Track 只在 **Port A** 的剩余通道里分配（0..15）；
3. 每个独立 Track 只拿一个通道；AP 的所有 Track 共用一个通道；
4. AP 无论以后是否做多输出，都只占一个 MIDI 输入通道（多输出只是音频拆分）；
5. Port B..P 的独立通道扩展留到以后，不在 C MVP。

伪代码：

```text
used = { ap.midi for ap in aps }        // AP 固定通道
pool = Port A 的 16 个通道 - used         // 独立 Track 只用 Port A

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

- 独立 Track 只使用 Port A 的 16 个通道；
- AP 可以使用 A01..P16 中已有的 MIDI 通道（它的 MIDI 身份固定）；
- 需要超过 16 条独立通道时，按 Fail/ReuseSafe 处理，或以后再补
  `MidiPort` meta 扩展到 Port B..P。

注意：不是所有 DAW 都保留 MIDI port，扩展端口前要先在导出选项里明确提示。

### 8.4 自动化写入

导出时对每个 owner 只写一次：

- 每条 Track 的 CC/PB/RPN 写到该 Track 的 MIDI 通道（NRPN/AT 以后同理）；
- AP 链接轨共享同一个 MIDI 通道，导出时按时间合并；
- 独立 LC 的 CC 写到分配到的通道；
- 乐器参数自动化不写成 MIDI（如果插件参数不是 CC），保持为工程内设置；
- 同一 owner 的同名 lane 先按第 6 节的 owner 解析合并。

---

## 9. UI / 交互

### 9.1 Track 头

每轨显示当前音频目标：

```text
Track 1   [LC01]       ← 默认
Track 2   [AP01]       ← 已链接
Track 3   [AP01]       ← 已链接
```

点击目标打开选择器：

- `独立通道（LC n）`
- `链接到 AP：A01 / A02 / ...`
- `新建 AP 编组…`

### 9.2 快速链接

- 拖 Track 到 AP 编组头/AP 行即可链接；
- 右键 Track ->「链接到 MIDI 通道」；
- 选中多条 Track 后一次性链接到同一个 AP；
- 解链后回到原来的 `OwnLc`，不复制 AP 的乐器自动化；CC 一直属于 Track（避免二次污染）。

### 9.3 MIX 视图

- 普通 LC 按 Track 顺序显示；
- AP 通道条只显示 `AP01`/`AP10` 身份，不写轨数或“共享”说明；
- 链接到 AP 的 Track 不再显示独立 LC 条，只显示它对应的 AP 通道；
- 多输出暂不做；C MVP 每个 AP 只有一条 strip；
- AP 通道的 freeze/mute/solo 明确提示作用范围；
- 通道面板可以像音轨面板一样选中，选中条用淡强调色，不加边框；
- 通道选择和音轨选择联动：选中一条 Track 时，它对应的 LC/AP 通道也选中；
- 多选时 SEND 源仍只取一个通道（最后一次点击的通道）。

### 9.3.1 SEND 聚焦模式（选中 = 源）

混音台不再为每条 Bus 显示一个固定旋钮，而是“选中源通道，其他通道显示发往自己的量”：

```text
选中 A01（源）
├─ A01 自己：不显示 SEND
├─ A02：显示 SEND，A01 → A02
├─ A10：显示 SEND，A01 → A10
└─ Master：显示 SEND，A01 → Master
```

规则：

1. **选中的通道是 SEND 源**。该通道自己的条上不显示 SEND（不能发给自己）。
2. 其他所有通道条各显示一个 SEND 旋钮，表示「选中源 → 本条通道」的发送量。
3. 切换选中的通道，所有 SEND 旋钮的源一起切换；每个旋钮显示的是
   选中源到本条的发送量，不是“所有 Bus 的旋钮”。
4. SEND 旋钮标签显示 **dB**，不再显示 `BUS 1`：
   - 最小 `-inf dB`（0 发送）；
   - 最大 `0.0 dB`（unity，1.0）；
   - 中间按 dB 映射，不按线性 0..1 显示。
5. 因为不再需要“每个 Bus 一个旋钮”，混音台取消 Bus 横向滚动；通道本身
   仍然可以横向滚动。
6. 高级多目标发送可以放在通道属性里，不作为混音台默认 UI。
7. Master 不能作为发送源：选中 Master 时所有 SEND 旋钮隐藏。
   Master 作为目标时，选中其他源通道即可在 Master 条上调「源 → Master」；
   新通道默认 0 dB（完全发送），否则新通道会没有声音。
8. 旋钮位置：SEND 不再占卡片顶部一整行，统一放在右侧控制列
   （dB → 声像 → SEND 开关 → 旋钮），旋钮下方写 `-inf` 或实际 dB；
   SEND 开关是独立文字按钮，开启时强调色。
9. 听感开关：右侧控制列底部的小圆点，亮 = 可听、灭 = 静音；
   不再显示 M/S，也不再区分左键静音/右键独奏（独奏保留在属性栏）。
10. 通道标题：每轨 LC 条直接写 3 位轨号（037），音频 LC 保留 LCxx，
    Master 直接写 Master。

### 9.3.2 SEND 聚焦的数据与交互

```rust
struct MixerUiState {
    /// 当前选中的 SEND 源；不是持久化混音数据。
    send_source: Option<ChannelId>,
    /// 多选通道（用于批量 mute/solo/freeze 等）。
    selected_channels: Vec<ChannelId>,
}
```

- `send_source` 只影响 UI 显示，不改变音频；点通道卡片或色带切换；
- 真正的发送量仍在 `MixerParams::channel_sends` 里（按 `source → target` 存）；
- 旋钮从 `-inf` 拖起来才创建发送条目；普通目标拖回 `-inf` 删除条目，
  Master 目标保留 `amount = 0` 的条目，用作「显式关闭」标记；
- 切换源时，旋钮值从 `channel_sends` 里读取「源 → 本条通道」的 amount；
- 源是 Master、源等于本条通道、或没有选中源时，不显示旋钮（`send_row_visible`）；
- SEND 的 dB 映射：`20 * log10(amount)`，`amount = 0` 显示 `-inf`；
- 当前 B 的发送量允许 0..2（+6 dB）；C 规范为 0..1（-inf..0 dB），
  旧格式迁移到 C v10 时 >1.0 的值要 clamp 或提示，不能静默改变听感。

### 9.3.3 Master send 是唯一主路径

```text
通道缓冲 → 推子/声像 → Master send（默认 0 dB）→ Master
```

- 混音图不再把 `output = None` 的通道直接累加进 Master；
- `ensure_master_sends` 为所有直连 Master 的节点补默认 0 dB send；
- SEND 关闭保留 0 条目，重新加载/补表不会把关闭的 SEND 恢复成 0 dB；
- 主输出改到另一条通道时删除 Master send，改回 Master 时补回；
- Master send 与其它 send 一样走 PDC 对齐（`global - out[source]`）。

### 9.4 自动化面板

- AR 显示 Track 的 CC/PB/clip lane；
- MIX 显示当前 Channel 的 gain/pan/send/insert lane；
- 设备面板显示 Rack 的乐器参数 lane；
- AP Rack 的乐器参数 lane 只有一份物理数据，任一链接 Track 都能打开；
- 每条 Track 的 CC lane 仍在自己轨道上，不复制到 AP；
- 冲突点显示标记，并在 AP 头显示「N 个自动化冲突」。

---

### 9.5 文件夹保持自由，不绑定 AP

已撤销“一个文件夹最多一个 AP”的约束。

原因：MIDI 轨道的排列本来就是自由的，大量工程会出现：

```text
A01
A02
A03
A01
```

这种“不按通道顺序”但完全合法的结构。为了文件夹整齐去改写 MIDI 结构，
是本末倒置。

最终规则：

1. 文件夹只做视觉组织、折叠、批量选择/移动，不限层数；
2. 一个文件夹里可以同时有不同 AP 的轨，也可以有独立 LC/`DirectMaster` 轨；
3. AP 是 **MIDI 路由身份**，不是文件夹，不要求同 AP 轨聚在一起；
4. 同一个 AP 的轨可以分散在多个文件夹；不同 AP 的轨可以在同一个文件夹；
5. 文件夹不产生音频通道，也不决定 send/output；
6. 需要按 AP 看轨道时，用筛选/分组/通道地图，不改变工程结构。

Folder、AP、Channel、Send：

| 概念 | 作用 | 是否产生音频通道 |
|---|---|---|
| Folder | 视觉组织、折叠、批量操作 | 否 |
| AP 编组 | 共享 MIDI 源/乐器/Rack | 是，一条 AP 通道 |
| 接收 send 的 LC/AP | 音频汇总、insert、send | 是，普通 Channel |
| Send | 把音频送到另一个 Channel | 否，是路由 |

文件夹是自由的，AP 是路由身份，两者不要再互相绑定。

### 9.6 通道分配要像 Domino 一样方便

默认每条 Track 独立 LC 以后，“我的音符到底在哪个通道”必须一眼可见，
而且改起来要快：

- 轨道列表增加 `MIDI Out` 列（Port + Channel），支持排序/筛选；
- 多选轨道后一次改通道，或使用「下一个空闲通道」；
- 用图标/颜色显示通道占用和冲突，不写说明性小字；
- 提供「通道地图」面板：16 个通道 × 若干轨，拖拽分配；
- AP 编组头可以一键把选中轨链接到当前 AP；
- 导出前显示「独立通道 X / 可用 16」，不足时提前提示。

“烘焙通道”不建议自动做。它应该是显式命令：

```text
Bake to AP / Bake to LC
```

作用是把一组轨的共享关系固化、清理未使用 LC、或为导出/冻结做准备；
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

### 9.7 ARRANGE 与 MIX 的 AR 可以共存

方案 C 里 Track/LC/AP 是显式关系，所以不再需要让 ARRANGE 和 MIX 互斥：

- ARRANGE 显示轨道行（Track）；
- MIX 显示通道条与通道 AR（LC/AP）；
- 两者可以同时显示（分栏/分屏/同一工作区），不再是二选一的模式开关；
- 选中 Track 时同步选中它对应的 LC/AP 通道；
- 在 ARRANGE 编辑轨道自动化、在 MIX 编辑通道自动化，写的是同一份数据；
- 视图只影响呈现，不影响信号流。

这解决的是两个需求同时存在：

```text
需要按轨看结构/编曲  +  需要按通道看混音/发送
```

而不是以前“切到 MIX 就看不到 ARRANGE，切到 ARRANGE 就看不到通道 AR”。

### 9.8 AR 分内容类型，不分通道类型

Track 和 Channel 是两层，不要把“音频轨/MIDI 轨”做成两种通道：

```text
AR（内容）                          MIX（音频）
MIDI Track A ─ notes ─> Rack ──────┐
                                    ├─> LC01
Audio Track B ─ clips ─────────────┘
```

结论：

- **Track 是内容容器**：MIDI 音符、音频片段、文件夹；
- **Channel 是音频节点**：LC、AP 通道、Master；
- MIDI 轨的声音路径是 `notes -> LC Rack / AP Rack -> Channel`；
- Audio 轨的声音路径是 `clips -> Channel`；
- 两者默认都有自己的 home LC；都可以把音频 send 到别的通道，但 home LC 不共用。

“MIDI 和 Audio 融合”发生在 **Channel**，不发生在 Track：

```text
MIDI 轨的音符仍然是音符，音频轨的片段仍然是音频；
到了 LC 之后，它们都只是音频样本。
```

所以 SEND 的目标永远是 **Channel**，不是 Track。MIDI 轨的 LC 收到音频 send，
只是音频求和，不会让它的音符变成音频，也不会改变它是 MIDI 轨。

AP 的边界：

- AP 是 MIDI 源分组，只接受 MIDI 轨作为成员；
- Audio 轨不能加入 AP，但可以把音频 send 到 AP 通道，和 AP 乐器的输出相加；
- AP 通道的乐器只处理 MIDI，不处理 send 进来的音频。

UI：

- AR 只有一种轨道列表，不分成“MIDI 区/音频区”两个页签；
- 用图标区分内容类型（音符/波形），不写“MIDI 轨/音频轨”说明小字；
- 轨道属性里的目标字段统一为 `CHxx / APxx / ---`；
- 新建命令仍然区分“新建 MIDI 轨 / 新建音频轨”，但创建后是同一类对象；
- 独奏/静音、选择、编组、颜色、折叠对两种内容一致。

边界：一条 Track 只装一种内容，不做 hybrid track（同一轨同时装 MIDI 和音频）
进 MVP。以后真要做，也是给 `TrackKind` 增加内容联合，不是把 Track 变成 Channel。

### 9.9 M/S：内容门、音频门与三态按钮

先纠正上一版：FL 的通道按钮不是“只有 M”，而是一个开关：关闭 = 静音，
右键 = 独奏。它用一个按钮表达了“正常 / 静音 / 独奏”三态，所以不会出现
M 和 S 同时点亮。这个模型更适合 Yinhe。

结论：**Track 必须能独奏，而且必须是内容级独奏。**

原因：AP 是多条 Track 共享一条音频通道。如果 Track solo 只静音混合音频，
solo 其中一条 Track 时，其他 Track 的音符仍然会进同一个 AP Rack，根本
无法隔离。因此 Track solo 必须停在内容层：

```text
AP01 共享 Rack
├─ Track A (Solo)  -> 音符/CC 放行
├─ Track B          -> 音符/CC 跳过
└─ Track C          -> 音符/CC 跳过
```

注意：被 solo 的 Track 不只是“音符”放行，CC/PB/RPN 也要跟着放行；非 solo
轨的 CC/PB/RPN 如果继续送进 AP Rack，会把 solo 轨的音量/音色改掉。音频轨同理，
solo 时只发它的 Clip。

#### 三态状态

不存两个 bool，而是：

```rust
enum Audibility {
    Normal,
    Muted,
    Soloed,
}
```

Track 和 Channel 各有一份。这样同一个对象不可能同时 Muted + Soloed。

| 层 | 作用 | solo 单位 |
|---|---|---|
| Track | 内容门：音符、CC/PB/RPN、音频 Clip | 单条 Track |
| Channel | 音频门：整条 LC/AP 的输出 | 单条 Channel |

#### Solo 只在一层生效

Track solo 和 Channel solo 同时存在会产生歧义。MVP 规则：

```text
进入 Track solo：清空所有 Channel solo
进入 Channel solo：清空所有 Track solo
```

同层内可以多选 solo（Ctrl/Cmd），跨层互斥。这样：

- solo 一条 Track：只发这条轨的内容，即使它在 AP 里也能隔离；
- solo 一条 AP 通道：所有链接到 AP 的 Track 内容放行，只听整条 AP；
- solo 一条汇总 LC：它的上游 send 放行，其他通道直达 Master 的路径静音。

M 永远优先于 S：目标通道如果是 Muted，Track solo 不会偷偷把它解除。
要试听就先把通道 M 关掉；UI 用颜色显示 Muted 状态，不写说明小字。

#### UI：一个按钮，三种状态

AR Track 行用 FL 式单按钮：

```text
正常：点击 -> Muted
Muted：点击 -> Normal
任意状态右键/长按 -> Soloed / 取消 Soloed
```

- 左键/轻触 = 开关（正常/静音）；
- 右键/长按 = 独奏；
- 按钮颜色区分 Normal / Muted / Soloed；
- 不出现两个按钮同时点亮。

MIX 通道条也只有一个按钮，和 AR 完全一致：左键/轻触静音，右键/长按独奏。
不保留独立的 S 按钮。黑乐谱生态的用户大多来自 FL/Domino，这套肌肉记忆
比“两个按钮”更重要；三态枚举保证 M 和 S 不会同时存在。

#### Solo 的图语义（Channel 层）

```text
solo 一个通道：
  它自己可闻；
  它到 Master 的下游路径可闻；
  给它供声的上游可闻；
  旁支静音。
```

目标 B 的同类 bug 已修（2026-11-17）：

- `process_buses` 以前只看 Bus 自己的 solo，通道 solo 后主输出到 Bus 会静音；
- 现在 `bus_solo_paths` 从 solo 的通道/总线出发，沿主输出和 send 标记下游
  Bus，`process_buses` 对这些 Bus 放行；
- 覆盖：通道 solo → Bus、send → Bus、嵌套 Bus、Bus solo → 下游 Bus；
  同 Bus 的非 solo 通道仍然被排除；
- 回归测试：`channel_solo_keeps_its_bus_path`、
  `channel_solo_keeps_its_send_path`、`channel_solo_keeps_nested_bus_path`、
  `bus_solo_keeps_nested_bus_path`、`channel_solo_excludes_other_channel_on_bus`；
- C 的统一通道图会把这条规则推广为完整的 solo 图传播（含上游 Bus 链）。

## 10. 冻结、静音和独奏

| 操作 | 普通 LC | AP |
|---|---|---|
| 冻结 | 冻结本 LC | 冻结该 AP 通道（该输出组） |
| mute | 本 LC 静音 | AP 通道静音（链接到该输出的所有轨） |
| solo | 本 LC solo（音频门） | AP 通道 solo（音频门） |
| Track M/S | 内容门：M 停该轨内容，S 只发该轨内容 | 同样只停/只发该轨 MIDI，不能抽走已混入 AP 的音频 |

Track 的 M/S 和 Channel 的 M/S 是三态互斥的，跨层 solo 只会生效一层，详见 9.9。

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

### 10.2 直通 Master 是 `---` 目标

这里要和“普通通道 output = Master”分开：

```text
普通 LC:
  Track -> LC（gain/pan/insert/send/freeze）-> Master

DirectMaster:
  Track -> ---（无 LC/AP 通道）-> Master
```

`DirectMaster` 对应 FL Studio 里把 Channel 的 FX 路由设成 `---` 而不是
`000`：音符直接进 Master，不经过任何 Mixer Track。

规则：

- UI 目标选择器显示 `--- (Master)`；
- 不创建 LC/AP 通道，因此没有 LC/AP 的 M/S/freeze/insert/send；
- 仍保留 Track 自己的内容门 M、源级音量/声像；
- 适合预览、节拍器、简单铺底、临时试听等不需要通道条的内容；
- 不适合需要独立推子/insert/send/自动化的轨；
- 如果之后要独立处理，把目标改成 `CHxx`，系统再创建/分配 LC。

不要把 `DirectMaster` 做成“特殊 LC”，否则又会回到“所有东西都必须有
channel strip”的旧模型。

### 10.3 汇总 LC / 混合 LC 怎么 freeze

因为 LC 可以当 Bus，freeze 必须按“这条通道有没有自己的源/有没有汇入”区分：

| 通道状态 | freeze |
|---|---|
| 源 LC：有 LC Rack/音频输入，没有其他通道 send 进来 | 可以，冻结这条 LC 的源 |
| AP 通道：共享 Rack 的输出 | 可以，冻结整个 AP 通道 |
| 纯 Bus：`source = None`，只接收 send | 不可以，没有自己的源可冻结 |
| 混合 LC：有自己的源 + 其他通道 send 进来 | 默认不可以；先移除汇入或改用 Bus |
| DirectMaster：没有 LC/AP 通道 | 不可以按通道 freeze |

原因：

- 冻结“源 LC”是明确的：把这条 LC 的源渲染成音频，LC 的 insert/send 仍实时；
- 冻结“纯 Bus”不明确：它的输入来自其他通道，冻结后上游还在跑，是否要同时
  静音上游 send？这会把一条通道的 freeze 变成整张图的操作；
- 冻结“混合 LC”更不明确：只冻结自己的源，汇入音频还在，结果不是用户听到的
  那条通道；
- 需要整组冻结时，正确做法是**冻结每条源 LC/AP 通道**，或者离线 bounce
  整组输出；不要给 Bus 发明一个“整组 freeze”语义。

如果以后确实要做 group freeze，再单独设计成“批量冻结上游源通道”，不要
把它塞进单通道 freeze。

---

## 11. 迁移

### 11.1 旧开发格式 -> C v10

旧工程的行为必须保持不变：

1. 旧工程里每个实际使用的 MIDI 通道 -> 创建一个 AP 编组；
2. 旧 `MixerParams.channels[ch]` -> AP 通道；
3. 旧 `racks[ch]` -> AP 的共享 Rack；
4. 旧 `TrackData.port/channel` 对应的 Track 全部链接到 AP；
5. 旧 `TrackData.lc`（per-track / multi-output）-> `OwnLc`；旧多输出工程
   如果 C MVP 不做多输出，需要降级为主输出并给明确迁移警告；
6. 旧 `rack_output` 在 C MVP 中不保留（多输出后期再加）；
7. 空通道不创建 AP，避免 256 条空编组；
8. 旧工程的冻结/自动化/insert/发送原样搬到 AP；
9. 旧 `buses` 全部转成 `source = None` 的普通 Channel，保留原来的
   strip/insert/send/freeze/meter；`BusId` 只作为迁移期旧 ID 使用；
10. 旧 `sends`/`output` 的 BusId 目标映射到新 ChannelId；旧 Bus 自动化
    （`BusGain` 等）迁移到该 Channel 的 `MixerGain`/节点自动化；
11. 旧 `LogicalChannelId(u32)` 全部重映射到新的 `u16` LC 空间；
    不能保留 `TRACK_LC_BASE=1_000_000`，也不能假设 LC ID = 源通道号。

这样打开旧工程听到的结果和旧实现一致，只是内部从“固定 MIDI 通道条”
变成了“AP 编组”。

版本差异：

- v6（已发布 beta）当前不纳入 C 的兼容范围；如果 beta 工程必须可读，
  要单独补 legacy 解码/警告；
- v9（最后开发基线）已有稳定 BusId；
- v7/v8/v10..v14 是未发布开发格式，不保留。

### 11.2 新工程

- 新建 MIDI 轨：默认 `OwnLc`，LC Rack（或内置音源）；
- 新建音频轨：`OwnLc` + 音频输入；
- 手动链接 AP：共享 Rack；
- MIDI 导入：自动建立 AP 并链接。

### 11.3 兼容

- C v10 加载器支持 v9（最后开发基线）/ v10（C）；v6 beta 不保留，加载时明确警告；
- 旧 v10..v14 开发格式不再保留解码器（未发布，按版本历史文档处理）；
- 旧版本不能打开 C v10，这是破坏性变更，需要在文档和发布说明中写清楚；
- 回滚方式：发布前备份工程；加载器只保证向后读，不维护双写。

---

## 12. 性能

默认每轨一条 LC，数量从“最多 256 条 MIDI 通道”变成“最多 Track 数”。

- 799 轨 -> 799 条 LC，`MixerGraph` 的通道缓冲约 `799 × 2 × frames × 4B`，
  在 512 帧下约 3.3 MB，可接受；
- 65536 轨满载时 LC 数量很大，必须懒分配：无音符/无音频/无自动化的空轨
  不创建 LC；
- 默认 LC Rack 如果每条轨都加载一个外部插件，实例数会爆炸。建议：
  - 默认使用 YinheCpu 的 per-track 输出（XSynth 已移除）；
  - 只有用户手动选择外部插件时才创建私有插件实例；
  - AP 编组共享一个插件实例，是省资源路径。
- C MVP 的 AP 只有一条 AP 通道；多输出后期再加。

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
| AP 的音频通道 | 每个 AP 1 条（C MVP） | 多输出留到后期；扩展时再变成多条 |
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
| `MixerGain/Pan/Send/InsertParam` | 复用 | owner 从 MIDI 通道改为对应 Channel |
| `ChannelLayout` | 复用 | dense 通道、每轨 LC 已具备；多输出后期再加 |
| CLAP/VST3 多输出 | C MVP 不用 | 当前 B 代码可删或留作 legacy |
| 通道发送/嵌套 | 复用并泛化 | AP/LC/纯汇入通道都走同一发送图 |
| MIDI 导入/导出 | 改造 | 导入建 AP，导出分配剩余通道 |
| Track `port/channel` | 降级为属性 | 不再是混音路由的唯一依据 |

---

## 14. 分阶段实现建议

1. **数据模型 C v10**：`TrackAudioTarget`、`ApGroup`、`MixerParams.channels/aps`；
   旧格式迁移为 AP。
2. **默认 OwnLc**：新建工程/新建轨分配私有 LC；空轨懒分配。
3. **AP 链接 UI**：Track 头选择器、拖拽链接、MIX 共享条。
4. **自动化 owner 解析**：同名 lane 合并、冲突标记、引擎/导出只写一次。
5. **MIDI 导入自动编组**。
6. **MIDI 导出通道分配器**：Fail/ReuseSafe/Force。
7. **冻结/多轨共享回归**：确保 AP/LC 的 freeze、
   automation、undo 全部覆盖。
8. **端口扩展**：需要时补 `MidiPort`。

每一步都可以独立提交、独立回滚；不要一次性替换 256 通道表。

---

## 15. 需要先确认的问题

1. 默认 LC Rack 用内置音源还是外部插件？外部插件按轨实例化成本很高。
2. 多输出插件 C MVP 不做，后期是否重开？本文建议留到明确需求出现时再加。
3. 同 owner 同 target 的冲突：默认最后编辑优先，UI 提供按钮裁决。
4. 独立 Track 暂时只支持 Port A 16 通道；超过时按导出策略报错/复用。
5. 解链 AP：不复制 AP 乐器自动化；CC 留在 Track；Track 回到自己的 LC/LC Rack。
6. AP 必须对应 MIDI 通道，不做纯共享 Rack 的无 MIDI AP。

---

## 16. 结论

- 这是一个成立的方案，而且比目标 B 更接近「普通 DAW 默认行为」。
- 它不是回滚：`MixerNode`、LC、自动化、freeze、通道发送都能继续用；多输出后期再接；
  真正变的是 **Track 的默认音频源** 和 **AP/LC 的并列关系**。
- 不建议直接叫「方案 B」：仓库里已有目标 B，建议叫**方案 C（AP/LC 并列）**。
- 实现上最大的风险不是音频图，而是：
  1. 默认每轨 LC Rack 的插件实例成本；
  2. 同名自动化/CC 的 owner 解析；
  3. 导出时 MIDI 通道耗尽。
  这三点需要在动代码前先定策略。
