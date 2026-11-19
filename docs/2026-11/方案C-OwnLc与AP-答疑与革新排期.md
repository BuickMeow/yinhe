# 方案 C：OwnLc 与 AP 答疑、革新排期

> 日期：2026-11-18
> 背景：围绕「yinhe 的音频流是不是已经是 C 计划」这一轮讨论，用户提出 7
> 个问题/决策：OwnLc 到底是什么、每轨独立 Rack 值不值得做、AP 为什么单输出、
> AP MVP 有什么影响、LC 轨道的通道是否应不可选、MIX 里 A11 的混乱、
> Master 声明与 MIX 通道 AR 暂停、LC 显示为轨道号、删除旧类型。
> 本文回答问题并记录已确认的方向与排期；设计全文见
> 《混音链路与通道模型-方案C-AP并列.md》，本文描述**当前实现**。

---

## 0. 结论速览

| 问题 | 结论 |
|---|---|
| OwnLc 是什么 | Track 的私有 home LC：一轨一条音频通道，推子/insert/send/freeze 全在这条通道上 |
| 每轨完全独立 Rack 对象值得做吗 | **现在不做**。触发条件：用户真的要「同一 MIDI 通道的两条 OwnLc 轨挂不同乐器」 |
| AP 为什么只有一条输出 | C 决策 #5：多输出插件暂不做；`ApChannelId.output` 已预留扩展位 |
| AP MVP 有什么影响 | AP 通道目前挂在共享 MIDI LC 上，不是独立 `MixerNode`；单 AP 单输出；对现有用法不阻塞 |
| LC 轨道的通道应不可选吗 | **同意**。home LC 唯一是既定决策，跨轨选 LC 是 B 的汇总遗留，以后汇总走 send |
| A10 后建 LC，MIX 仍显示 A11 | `ChannelLayout` 仍为活跃 MIDI 源通道开 dense，MIX 按 dense 列条 → 出现多余的空 A11；修法见 §6 |
| LC 显示 | LC 条直接显示**轨道号**（如 004），与 A01 同级；音轨面板不再显示 LC01 |
| 旧类型 | 全面清理 `MixerChannelId` 等过渡类型，排期见 §9 |

---

## 1. OwnLc 到底是什么

一句话：**OwnLc = 这条轨自己的音频通道**。

```text
Track（音符/CC/音频片段）
  -> TrackAudioTarget::OwnLc { lc, rack }
       lc   = 这条轨的 home LC（一轨一条，独立推子/insert/send/freeze）
       rack = 这条轨所在 MIDI 通道的 Rack 配置
  -> Master（或 send 到另一条通道）
```

与 AP 的对比：

| | OwnLc | Ap |
|---|---|---|
| 通道归属 | 本轨私有 | 共享（链接到该 AP 的所有轨） |
| 乐器 | 本轨实例（外部插件）/ 内置音源按轨拆分 | 共享一个 Rack 实例 |
| 推子/insert/send/freeze | 每轨独立 | 所有链接轨共享 |
| MIDI 身份 | 仍是自己的 port/channel（导出用） | AP 的 MIDI 通道 |
| 语义 | 普通 DAW 的“音轨” | Cubase 式“乐器机架 + 输出通道” |

当前实现里的两个过渡痕迹：

1. **Rack 配置仍按 MIDI 通道**（`RackId::new(global_channel)`）：
   同一 MIDI 通道的多条 OwnLc 轨共享一份 `RackParams` 配置，但运行时
   每条轨一个乐器实例（`Document::instrument_hosts`）。也就是说“每轨一个
   实例”已经有了，“每轨一份**配置**”还没有（见 §2）。
2. **DirectMaster 已暂停**（本次落地）：不再提供把轨道输出设为 Master
   的入口，类型保留给存量工程。

---

## 2. 每轨一个完全独立的 Rack 对象，现在值得做吗

**现状**（`crates/yinhe-editor-core/src/document/rack_edit.rs` 注释）：

- `RackId` 按 MIDI 通道分配；
- 同通道多条 OwnLc 轨 → 共享一份 `RackParams`（乐器链配置）；
- 引擎按 `instrument_hosts` 为每条 OwnLc 轨建独立插件实例，输出写各自
  home LC；
- 内置音源用 `RackOutputMode::PerTrack` 按轨拆分音频。

**要“每轨一份配置”需要改什么**：

1. `RackId` 增加 per-track 分配区间（现在是 `u16` = MIDI 通道）；
2. 工程格式：`racks` 表的 id 语义变化 + 旧工程迁移；
3. 设备链 UI 按轨展示/编辑（现在按通道）；
4. 乐器参数自动化的 owner 从“通道 Rack”变成“轨 Rack”，冲突规则重写；
5. AP 语义要保持不受影响（AP 共享 Rack 仍然是通道级）。

**收益场景只有一个**：同一 MIDI 通道的两条 OwnLc 轨要挂**不同**乐器。
这个场景目前可以用“把其中一条轨挪到别的 MIDI 通道”绕开。

**结论：现在不做。** 触发条件 = 用户实际提出“同通道两条 OwnLc 轨要不同
乐器”。在此之前 `RackId` 保持通道级，避免格式迁移和自动化 owner 的连锁改。

---

## 3. AP 为什么只有一条输出通道

C 决策 #5（多输出暂不做）的三个理由：

1. **AP 的本职是共享 MIDI 源**：黑乐谱的主场景是“很多轨共用一个乐器”，
   一条音频输出就够；需要多个音色时建多条 AP（A01/A02…）更直观；
2. **多输出的真实成本在宿主侧**：插件多输出插座 → 每输出一条 LC →
   freeze/自动化/导出都要按输出拆分，PDC 也要按输出对齐；
3. **先验证主链路**：C 的核心是“一轨一通道”，多输出是锦上添花。

代码里已经留了扩展位：`ApChannelId { group, output }`，MVP 固定
`output = 0`；将来做多输出时把 `ApGroup.channel` 从单个 LC 变成
`Vec<LogicalChannelId>`，`MixerParams` 里 AP 通道变成独立 `MixerNode`。

---

## 4. AP MVP 的实际影响（阶段 6 后）

| 影响点 | 表现 | 阻塞吗 |
|---|---|---|
| AP 通道是独立节点 | `ChannelId::Ap` 拥有自己的 `MixerNode`，`ApGroup` 不再指向共享 MIDI LC；引擎把 AP 映射到其 MIDI 源 dense | 不阻塞 |
| 单 AP 单输出 | 多输出插件不能从一个 AP 拆多路 | 不阻塞（§3） |
| freeze/insert/自动化 | 都挂在 AP 节点上，身份清晰 | 不阻塞 |
| 发送/主输出 | `ChannelId::Ap` 与 LC 一样参与发送图与 PDC | 不阻塞 |
| 删除 AP | 编组、节点、指向它的 send 一起清理 | 不阻塞 |

一句话：**AP 已是独立音频通道，剩余差异只有“单输出”和“Rack 配置仍按
MIDI 通道”**；前者是产品决策，后者见 §2。

---

## 5. LC 轨道的通道应不可选（已确认）

同意。方案 C 决策 #14 已经写明：“home LC 唯一且不记录来源；跨轨汇总用
send，不把两条 Track 的 home LC 指向同一条。”

现状还保留着 B 的遗留入口：

- 属性面板 `show_lc_picker`：可以把 MIDI 轨指到**任意已有 LC**；
- `Document::link_track_to_lc`：跨轨链接到已有 LC。

计划（§9 阶段 2）：

- OwnLc 的 LC 不可选，属性面板只显示本轨 LC（轨道号），不再提供下拉；
- 删除 `link_track_to_lc` 与 `show_lc_picker` 的跨轨部分；
- 需要“多轨进同一通道”时用 send 或 AP，而不是改 home LC。

---

## 6. “A10 之后创建 LC，MIX 里却还是 A11”的原因与修法

**原因（两层）**：

1. `ChannelLayout` 仍然为每个**活跃 MIDI 源通道**开 dense 通道
   （`channels_for_model`：有音符/控制事件的通道就算活跃）；
2. MIX 的通道条按 dense 列出（`mix/view.rs::draw_channel_strips` 遍历
   `active` 列表）。

于是一条 OwnLc 轨（MIDI 通道 A11、音频走自己的 LC）会在 MIX 里产生**两条**
条目：一条空的 A11（源通道 dense，没人听它）+ 一条 LC 条。这就是
“A10 之后创建 LC，MIX 仍然显示 A11”的混乱来源。

**修法（两步走）**：

1. **UI 过滤**：MIX 只列“音频目标通道”——AP 正在使用的 MIDI 通道 +
   OwnLc 的 LC + 音频 LC；被 OwnLc 取代的源通道不显示。
   现状：通道条早已只列 AP（`9829d8be`）；MIX 图层面板也已按音频目标
   列行（`ea104237`），OwnLc 轨显示「001 Track1」而不是 A11 分组。
2. **引擎侧（可选优化，未做）**：某 MIDI 通道上的轨**全部**是 OwnLc 时，
   理论上可不给该源通道开 dense；但 AP 节点仍映射到 MIDI 源 dense，
   且 `ChannelLayout` 按活跃通道统一分配，暂不为省一路静音处理增加分支。
   用户可见的 A11 混乱已由 UI 过滤消除。

---

## 7. LC 通道显示为轨道号（已确认）

现状：`mix/labels.rs` 已经把每轨 LC 条显示为所属轨道号（如 `004`），
与 `A01` 同级；音轨面板不再显示 `LC01`。

确认后的规则：

- **LC 条 = 轨道号**（一轨一通道已经确认，通道号没有独立存在的必要）；
- 属性面板里 OwnLc 不再显示 LC 选择器（配合 §5）；
- 后续交互方向：把想要的轨道**添加到 MIDI 通道**（保留 home LC，另投
  AP）而不是**更换为 MIDI 通道**——轨道身份不变，共享乐器是追加关系。

---

## 8. 已落地与暂停项

**暂停（`0dee4f41`）**：把轨道输出声明为 Master——MIDI 轨 / 音频轨的
输出下拉不再提供 Master；存量 DirectMaster 轨仍显示 Master 以便切回 LC。
回归测试：`output_kind_hides_master_unless_legacy_direct`、
`audio_output_hides_master_unless_legacy_direct`。

**升级为删除（`042ba014`）**：MIX 的通道 AR 视图整模块删除（约 3.9k 行，
用户决策：两套走带维护成本太高，后续会补统一的新方案）。通道分组是纯数据，
迁至 `info_panel/layers_groups` 供 MIX 图层面板继续使用；MIX 主区域交给底部
面板占满。ARRANGE 的轨道 AR 保留。

**音轨面板徽标（`d924c8f5`）**：OwnLc 轨不再显示 LC01（普通 DAW 轨道观感），
AP 轨仍显示 A01；`lc_labels` 传递链删除。

**AP 独立节点（`f104e3cb`）**：`ChannelId::Ap` 拥有自己的 `MixerNode`，
删除 `MixerChannelId` / `MixerNodeId`，`MixerRack` 改为单一 `ChannelId` 槽位表。

---

## 9. 旧类型清理排期（全面革新）

目标：`MixerChannelId`（Midi/Audio/TrackLc/Master）等过渡查询键全部退场，
只保留 C 的统一身份 `ChannelId::{Lc, Ap, Master}`。

| 阶段 | 内容 | 状态 |
|---|---|---|
| 1 | MIX 只列音频目标通道 | ✅ `ea104237`（图层面板按音频目标列行；通道条早已只列 AP——`9829d8be`） |
| 2 | OwnLc 通道不可选 | ✅ `afaa3f61`（删 `link_track_to_lc` + LC 选择器） |
| 3 | `MixerChannelId` → `ChannelId` 全面替换 | ✅ `f104e3cb`（删 `MixerChannelId`/`MixerNodeId`；`AutomationTarget`/`InsertTarget`/`DockContext` 全部 ChannelId；v9 混音段迁移器删除） |
| 4 | `DirectMaster` 类型删除 | ✅ `2dee7542`（类型、UI、隐藏 dense 全部删除） |
| 5 | `ChannelSource::Track(u16::MAX)` 哨兵 | ✅ `88979687`：改为"创建即绑定轨道"（`add_track_lc(track)`），哨兵不再落盘 |
| 6 | AP 通道变独立 `MixerNode` | ✅ `f104e3cb`（`ChannelId::Ap` 自有节点，`ApGroup.channel` 删除；音频仍映射到 MIDI 源 dense） |
| 7 | v9 Bus 残留文案/注释清理 | ✅ `9a81c19c` |

另两个本轮修复：

- `3705cce5`：创建文件夹后 automation lane 的 `track` 未同步，曲线沿用旧轨道色。
- `f104e3cb`：SEND 聚焦目标失效时回退首条可发送通道，旋钮不再消失。
- `f0bf651d`：SEND 改为「选中 = 源」，其他条显示「选中源 → 本条」；Master 不能作为源，选中 Master 时全部旋钮隐藏。
- `bc9cc1e3`：听感开关改为小圆点（亮 = 可听，灭 = 静音），删除 M/S 按钮。
- `587aa9f0`：每轨 LC 条标题改为轨号（037），Master 文案统一。
- `174b62dc`：SEND 移到右侧控制列，新增 SEND 开关，`OFF` 改为 `-inf`。
- `10525db2`：电平表改为推子后取样，音轨表读混音通道表。
- `3d82c606`：删除直连 Master，默认 0 dB Master send；SEND 关闭保留 0 条目。

顺序理由：先动用户可见的显示（阶段 1–2），再做身份替换（3），最后才动
音频图（6）；每阶段独立 commit + 回归测试，任何一步都可单独回滚。

---

## 10. 关键代码索引

| 主题 | 位置 |
|---|---|
| `TrackAudioTarget::{OwnLc, Ap, DirectMaster}` | `crates/yinhe-core/src/model.rs` |
| `ChannelId / ChannelSource / ApChannelId` | `crates/yinhe-types/src/mixer_node.rs` |
| `MixerNode / ApGroup` | `crates/yinhe-mixer/src/node.rs` |
| AP 过渡（共享 LC 指针） | `crates/yinhe-mixer/src/params.rs` `ensure_ap` |
| 每轨乐器实例 | `crates/yinhe-editor-core/src/document/rack_edit.rs` `instrument_hosts` |
| MIX 通道条按 dense 列出 | `crates/yinhe-egui/src/mix/view.rs` `draw_channel_strips` |
| 输出类型下拉（Master 已暂停） | `crates/yinhe-egui/src/right_panel/info_panel/track.rs` |
| MIX 通道 AR 开关 | `crates/yinhe-egui/src/app/layout.rs` `MIX_CHANNEL_AR_ENABLED` |
