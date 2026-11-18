# 方案 C 返工检讨与修复方案

> 本文是对 2026-11-17 ~ 18 这一轮「方案 C MVP」实现的自我检讨。
> 结构：先承认问题，再逐条给出根因、涉及 commit、修复方案；最后单独检讨
> `yinhe-audio` 与今晚所有相关 crate 的代码质量问题。

---

## 0. 一句话结论

**我把「实现功能」当成了「完成任务」，没有按用户口径逐条核对，也没有在交付前
自己按用户的视角走一遍 UI。** 7 个问题里有 5 个是「没按口径做 / 多做了」，
2 个是「做了但用户体验不对」。这不是理解偏差，是流程缺失。

---

## 1. 七个问题的逐条检讨

### 1.1 MIX 通道被选中时没有高亮

**现象**：在 MIX 里点选通道后，看不到「已选中」的视觉反馈。

**用户明确要求（补充）**：选中时**整个通道条背景**要像音轨面板（AR 轨道行）一样
被点亮，不是只加一圈描边。AR 的 `render::draw_row_background(..., selected, ...)`
就是背景点亮，MIX 必须使用同一套观感。

**根因**：

- `crates/yinhe-egui/src/mix/strip/controls.rs::strip_frame` 的签名里根本没有
  `selected` 参数，所有通道条一律画同样的背景 + 顶部色带。
- MIX 的选中状态实际存在 `doc.edit.track_selected`（音轨索引集合），但通道条
  渲染时从未读取它。
- AR 的音轨行有 `render::draw_row_background(..., selected, ...)`，MIX 没有对应
  实现。我把「AR 有」当成了「MIX 也应该有」，但没有验证。

**涉及 commit**：不是某一个 commit 引入的，是 MIX 通道条从拆分
（`1542347d`）以来一直缺失。

**修复方案**：

1. `strip_frame` 增加 `selected: bool` 参数；选中时**整条背景提亮**（与 AR
   `draw_row_background` 的 selected 使用同一套 `theme` 颜色，不要另造色值）。
   顶部色带保持原样（它是通道身份色，不参与选中表达）。
2. 选中判定抽成纯函数 `strip_selected(doc, MixerChannelId) -> bool`：
   - MIDI 通道：`track_selected` 中存在任一该通道的 MIDI 轨；
   - 音频通道：存在 `audio_channel == ch` 的音轨；
   - 每轨 LC：`track_selected` 包含该轨；
   - Master：跟随 Conductor 选中态（或不高亮，按 AR Master 行一致）。
3. 点击通道条（色带或空白区）时同步 `track_selected`，保证「点了就亮」。
4. 回归测试：构造 2 条轨、选中其中一条，断言对应通道条 `selected == true`，
   且背景色与 AR 选中行一致（比较 `Color32` 常量，不比较像素）。

---

### 1.2 旋钮上不该写「A01 -inf」，只写 OFF 或实际 dB

**现象**：SEND 聚焦旋钮上方显示 `A01`，下方显示 `-inf dB`，信息重复且碍眼。

**根因**：

- `crates/yinhe-egui/src/mix/strip.rs::focus_send_info` 返回
  `label = channel_id_label(target, ...)`（即 `A01`/`LC01`/`AP01`）。
- `focus_send_block` 把 `label` 和 `send_db_label(amount)` 两行都画出来。
- 我当时的想法是「让用户知道发送目标是谁」，但用户既然已经在通道条上点了
  SEND 聚焦，目标就在上下文里，再写一遍是冗余。**这是典型的信息噪声，不是功能。**

**涉及 commit**：`3d84b95e feat(MIX): SEND 改为聚焦目标与单 dB 旋钮`（我引入
的 label），`7a1aece0` 删 Bus 时保留了这段。

**修复方案**：

1. `focus_send_block` 删掉 `label` 参数与那一行文本，只保留：
   - `amount <= 0` → `OFF`
   - 否则 → `-12.3 dB`（保留一位小数）
2. `send_db_label` 统一负责 `OFF` / dB 文案，避免「-inf」和「OFF」两种表达。
3. 如果确实需要提示目标，放到 hover tooltip（`hint.mix.send_target`），
   而不是常驻文本。
4. 同步删除 `focus_send_info` 的 `label` 返回值，减少一个无用的 String 分配。

---

### 1.3 AR+CH 按钮是我自作主张加的

**现象**：模式栏出现 `AR+CH` 开关，用户从未要求。

**根因**：

- 用户口径 #7 是「ARRANGE 与 MIX AR 可**同时显示**」，我把它理解成「需要一个
  手动开关」，于是加了 `show_both_ars` 布尔值和 `AR+CH` 按钮。
- 正确理解应该是：**两者本来就该同时显示**（比如 ARRANGE 左侧轨道 + 右侧通道
  AR），不需要用户手动切换；或者按可用宽度自动决定，而不是加一个全局开关。
- 我违反了「不擅自加控件」的红线，也没有在交付说明里主动提这个新增按钮。

**涉及 commit**：`60b448a3 feat(ui): ARRANGE 与 MIX 通道 AR 可同时显示`（我加
的）。

**用户明确要求（补充）**：口径 #7「ARRANGE 与 MIX AR 可同时显示」**完全忽略**，
这个功能**整体回滚**，不是「换个方式实现」。用户从未要求过「同时显示」的按钮、
状态或布局。

**修复方案**：

1. 回滚 `60b448a3` 的全部改动：
   - 删除 `show_both_ars` 状态字段（`app.rs` / `mix.rs` 的布局结构）；
   - 删除 `mode_bar.rs` 里的 `AR+CH` 按钮；
   - 删除 ARRANGE 视图里「左右分栏 60/40 + 选择同步」的共存逻辑；
   - 恢复 `60b448a3` 之前的单一 AR 布局。
2. 用 `git show 60b448a3 --stat` 核对完整文件清单，逐文件回滚；不要手工「修补」。
3. 回归测试：ARRANGE 视图下只有轨道 AR，不再有通道 AR 分栏；MIX 视图仍是
   通道 AR。删除对应布局测试（如果有）。

---

### 1.3.1 口径 #7 已作废

> 用户 2026-11-18 明确：**#7「ARRANGE 与 MIX AR 可同时显示」完全忽略，所有
> 功能完全回滚。** 本文档后续所有涉及 #7 的表述一律以本条为准。16 条口径中
> #7 标记为「作废」。

---

### 1.4 选了 LC 还留着端口/通道选项

**现象**：音轨属性里，「端口 / 通道」两列始终显示，即使音频目标是 LC。

**根因**：

- `crates/yinhe-egui/src/right_panel/info_panel/track.rs` 把「端口/通道」和
  「音频目标（LC / AP / ---）」拆成两个独立面板行，端口/通道**无条件渲染**。
- 音频目标的选项也只有三个固定项：`LC`、`当前 AP`、`---`，AP 不能选别的 AP，
  LC 不能选别的 LC。
- 用户的要求是**层级化**的：
  - 先选输出类型：Master / LC 通道 / AP 通道；
  - 选 AP 后才出现「端口 A + 通道 01」来选择具体 AP；
  - 选 LC 时端口/通道没有意义，应该隐藏（或固定为 Port A 的自动值）。

**涉及 commit**：`a3f18506`（LC 身份独立 u16）之后逐步形成，`26deddb4` 把
CH 改成 LC 时没有重构这一块。

**修复方案**：

1. 重排「音轨」面板：
   ```
   名称
   颜色
   ── MIDI 源（仅 AP / 独立轨需要）──
   端口  [A ▾]   通道 [01 ▾]      ← 仅当输出类型 = AP，或轨是独立 MIDI 轨且需要指定源通道时显示
   ── 音频输出 ──
   类型  [Master / LC / AP ▾]
   LC    [LC03 ▾]                 ← 类型 = LC 时显示（列出所有 LC）
   AP    [A01 ▾]                  ← 类型 = AP 时显示（列出所有 AP，显示为 A01/B02…）
   ```
2. 数据模型：
   - `TrackAudioTarget::OwnLc { lc, rack }` 已经支持指定任意 LC；
   - `TrackAudioTarget::Ap { ap }` 已经支持指定任意 AP；
   - 需要新增「列出所有 LC / 所有 AP」的查询，以及链接到**别的** LC 的文档
     方法（当前 `link_track_to_own_lc` 只创建新 LC）。
3. 端口/通道的显示条件：
   - 输出类型 = AP：显示，且修改端口/通道会同步 `ApGroupId`；
   - 输出类型 = LC：隐藏（LC 是独立音频通道，不需要 MIDI 源）；
   - 输出类型 = Master：隐藏。
4. 独立 MIDI 轨（OwnLc）的 MIDI 源通道仍由 `TrackData.port/channel` 决定，
   但按口径 #9 只在 Port A 分配，所以 UI 上可以只显示「A + 通道」或完全不显示，
   由自动分配决定。

---

### 1.5 M 按钮被做成了「M 和 S 两个按钮」，且 MIX 里 Solo 时显示 S

**现象**：

- AR 的自动化子行有**两个独立按钮** M 和 S（`track_panel.rs:302-319`）；
- MIX 的 M/S 是单按钮，但 `Audibility::Soloed` 时按钮文字变成 `S`
  （`controls.rs::ms_column`）。

**根因**：

- 用户口径 #15 明确是「**单按钮**左键静音右键独奏」。我在 AM 子行上照抄了旧
  Bus 行的双按钮实现（`dd818ae6`），没有统一到单按钮。
- MIX 的 `ms_column` 虽然是一个按钮，但用「文字切换 M/S」来表达三态，用户看到
  的是「右键后按钮变成 S」，这就是他说的「右键还有个 S」。
- FL 的语义是：**按钮本身代表一个开关，亮 = 开、灭 = 关**；不是用文字变化表达
  状态。用户要的是这个。

**涉及 commit**：`dd818ae6 feat(arrange): AR 自动化 lane 支持 M/S 试听`（双按钮）、
`1542347d`（MIX 单按钮三态文字）。

**修复方案**：

1. 抽一个统一的 `audibility_button(ui, &mut Audibility, size) -> bool`：
   - 按钮**始终显示 `M`**，不显示 `S`；
   - 左键：`Normal ↔ Muted`；
   - 右键：`Normal ↔ Soloed`；
   - 亮起颜色：Muted = `mute_active()`，Soloed = `solo_active()`；
   - 三态互斥（同一个 `Audibility` 枚举保证）。
2. AM 子行的双按钮换成这一个按钮；`AmMsState` 的 `mute/solo` 两 bool 改为
   `Audibility` 或保持内部两 bool 但 UI 单按钮。
3. 删除 `hint.arrange.am_mute` / `am_solo` 中不再需要的文案，合并为
   `hint.audibility`（左键静音、右键独奏）。
4. 回归测试：三态切换顺序 `Normal → Muted → Normal`、`Normal → Soloed → Normal`，
   并断言同一时刻只有一个状态。

---

### 1.6 MIX 显示 AP01，应该是 A01

**现象**：MIX 通道条标题显示 `AP01`，用户认为应该显示 `A01`（端口+通道）。

**根因**：

- `crates/yinhe-egui/src/mix.rs::channel_title` 对 AP 通道返回
  `ap_label(global)` = `format!("AP{:02}", global + 1)`。
- 这个命名是 `4996cb6f feat(MIX): AP 通道显示 AP01/AP10` 引入的，当时只考虑了
  「与普通 LC 区分」，没有考虑用户对 MIDI 端口/通道的直觉：AP 本质就是一个
  MIDI 源通道，显示 `A01`/`B02` 才和导入导出、外部设备一致。
- 同理，`ap_label` 在音轨属性的输出选项里也显示 `AP01`，用户同样不接受。

**涉及 commit**：`4996cb6f`（我引入的）。

**修复方案**：

1. `ap_label(global)` 改为 `midi_label(global / 16, global % 16)`（即 `A01`）；
2. 如果需要在 UI 上区分「这是 AP 而不是独立 LC」，用图标/副标题表达，
   **不要改主标题**；
3. 全局搜索 `AP{:02}` / `ap_label` 的所有显示点：MIX 通道条、音轨属性输出选项、
   设备链语境标签、事件浏览器，全部统一为 `A01` 格式；
4. 如果保留 `AP` 前缀，只能出现在 tooltip 或说明文字里。

---

### 1.7 改完 LC 通道后 AR 不生效

**现象**：在音轨属性里把输出改成 LC（或改 LC 的归属）后，AR 里看不到变化。

**根因（已定位到两处）**：

1. **`show_track_info` 的返回值语义被复用**：
   - `show_track_fields` 的返回值注释是「端口/通道是否改变」，但
     `routing_changed`（音频目标改变）也被塞进同一个 bool；
   - 上层 `right_panel.rs` 把它当作 `port_changed`，触发的是「端口/通道变更」
     路径，而不是「轨道结构/音频布局变更」路径；
   - `routing_changed` 分支里只设了 `routing_changed = true`，**没有调用**
     `doc.edit.track_cache.rebuild_info(&doc.data)` / `rebuild_pc_map`，
     而端口/通道分支调用了。
2. **音频布局没有重建**：`link_track_to_own_lc` / `link_track_to_ap` 只
   `bump_revision` + `mark_mixer_dirty`，但 `ChannelLayout` 是在引擎创建时冻结
   的。新增/删除 LC 后必须让引擎重建（`needs_audio_rebuild`），否则：
   - AR 的 MIX 通道 AR 看不到新 LC；
   - 播放时新 LC 没有对应 dense 通道。

**涉及 commit**：`a3f18506` 之后引入 `routing_changed` 时没有区分返回值语义；
`26deddb4` 改 LC 命名时没有回归这条路径。

**修复方案**：

1. `show_track_fields` 的返回值拆成两个：
   ```rust
   struct TrackInfoOut {
       /// 端口/通道改变：需要 rebuild_info + rebuild_pc_map + ensure_rack_track_lcs。
       port_changed: bool,
       /// 音频目标改变：需要重建 ChannelLayout（needs_audio_rebuild）。
       routing_changed: bool,
   }
   ```
2. `routing_changed` 分支补齐：
   - `doc.edit.track_cache.rebuild_info(&doc.data)`；
   - `doc.sync_track_lc_racks(idx)`（确保 UI 侧每轨 LC 机架同步）；
   - 返回 `structural_changed = true` 让 `views.rs` 走 `teardown_audio()`。
3. 在 `views.rs` 里把 `structural_changed` 与 `needs_audio_rebuild` 合并，
   下一帧由 `rebuild_audio_if_needed` 重建引擎。
4. 回归测试：
   - `link_track_to_own_lc` 后 `needs_audio_rebuild` 为真；
   - `track_cache.info` 与模型一致。

---

## 2. 为什么会出现这些问题（流程反思）

1. **没有按用户口径逐条验收**。16 条口径我写进了文档，但实现完没有逐条对着
   UI 点一遍。第 3、5、6 条都是口径里有明确描述、我实现时改了口径。
2. **批量脚本删除没有逐行审核**。删 Bus 时用脚本批量删测试，误删了 5 个纯通道
   测试、误删了 `controls.rs` 文件末尾所有函数（后来从 HEAD 恢复）。这是危险的
   操作方式，应该先 `git diff --stat` 检查删除行数是否符合预期。
3. **「能编译 + 测试绿」被当成交付标准**。第 1、2、7 条都是测试覆盖不到、但
   用户一眼能看到的问题。UI 功能必须自己按用户视角走一遍。
4. **擅自加控件**。`AR+CH` 是典型：需求是「可同时显示」，我加了「开关」。
5. **信息冗余当成「帮助用户」**。旋钮上的 `A01 -inf` 是我想多了。
6. **把旧实现照抄到新地方**。AM 子行的双 M/S 是从旧 Bus 行抄的，没有统一到
   新口径。

---

## 3. `yinhe-audio` 专项反思

> 这一轮 `yinhe-audio` 改动量最大（XSynth 移除、Bus 移除、每轨插件实例）。
> 以下是具体的代码质量问题，不是泛泛而谈。

### 3.1 `ensure_cpu_synth` 用 `expect` 掩盖不可能分支

```rust
self.cpu_synth.as_mut().expect("cpu synth just ensured")
```

虽然逻辑上刚 `Some` 过，但 `expect` 在生产路径上是 panic 源。AGENTS 红线是
「生产环境不要 unwrap」，`expect` 同样会终止进程。应改为：

```rust
match self.cpu_synth.as_mut() {
    Some(synth) => synth,
    None => unreachable!("ensure_cpu_synth 刚写入 Some"),
}
```

或返回 `Option<&mut>`，调用方 `if let Some`。**更好的做法**：把
`cpu_synth: Option<CpuSynth>` 改成 `CpuSynth`（XSynth 已删，没有「不存在」的
语义），从类型上消除这个分支。

### 3.2 `SyncSends` 命令与渲染语义脱节

- `AudioCommand::SyncSends { sends, audio_sends, track_sends }` 更新的是
  `MixerNode.sends`（旧的 `SendParams` 列表）；
- 但混音图实际只消费 `MixerParams::channel_sends`（`ResolvedChannelSend`）；
- `engine_mixer.rs::sync_sends_config` 写入 `node.sends` 后，渲染图读不到，
  等于**写了个没人用的字段**。

这是我在删 Bus 时把 `SyncBusConfig` 改名成 `SyncSends` 留下的半成品。
**修复**：删除 `SyncSends` 命令与 `sync_sends_config`，发送只走
`set_channel_send` + 全量 `set_mixer_params`；同时删除
`midi_sends_snapshot` / `audio_sends_snapshot`（只剩测试用）。

### 3.3 `instruments` 按 dense 全量遍历

`render_instruments` 每块遍历 `0..instruments.len()`（所有 dense，含音频通道、
AP 通道、DirectMaster），大部分是 `None`。大型工程 dense 可能上百，每块做一次
全扫是浪费。应维护一个「有乐器链的 dense 列表」，在 `set_instrument_slot`
增删时更新，渲染只遍历这个列表。

### 3.4 `set_instrument_slot` 静默丢弃不支持的目标

```rust
InsertTarget::Audio(_) | InsertTarget::Master => u32::MAX,
```

音频轨和 Master 装乐器是非法操作，但当前只是静默退回 processor，没有日志。
应改为 `tracing::warn!` 或直接 `debug_assert!`，让调用方尽早发现。

### 3.5 `ChannelOutput` 只剩一个变体

删 Bus 后：

```rust
pub enum ChannelOutput {
    Channel(usize),
}
```

这是**死抽象**：`Option<ChannelOutput>` 等价于 `Option<usize>`。应删除
`ChannelOutput`，`set_channel_output(channel, Option<usize>)` 直接用下标，
`refresh_pdc` / `process_channels` 的 match 也简化。

### 3.6 `instrument_output_dense` 是恒等函数

```rust
pub fn instrument_output_dense(&self, source_dense: usize) -> u32 {
    source_dense as u32
}
```

OwnLc 每轨实例后不再需要重定向。应删除该方法，调用方直接用 `dense`。

### 3.7 `dispatch` 每次音符都查 `track_lc_dense`

```rust
let own_lc_dense = self.channel_layout.track_lc_dense(track);
```

`track_lc_dense` 是 `Vec` 索引，O(1)，但每音符一次仍可避免：`ChannelLayout`
可以在构建时生成 `plugin_host_dense: Vec<u32>`（按 track 索引），
`dispatch` 直接查表。当前量级可接受，但应在注释里写明「每音符一次 O(1) 查表」。

### 3.8 `engine_render.rs` 的 `refresh_channel_mask` 也全扫 `instruments`

```rust
for (dense, slot) in self.instruments.iter().enumerate() {
    if slot.is_some() { ... }
}
```

与 3.3 同一问题：应只遍历有链的 dense。

### 3.9 `audio_renderer/commands.rs` 的 `SyncSends` 分支是空转发

```rust
AudioCommand::SyncSends { ... } => {
    self.engine.handle_command(AudioCommand::SyncSends { ... });
}
```

渲染器层没有做任何处理，只是转发给引擎。删命令时一并删除。

### 3.10 `spawn.rs` 的 `MixerMeters` 删了 `buses` 但保留 `orphan` 概念

`MixerRack` 的 `orphan` 字段原本服务 Bus 删除；现在 Bus 没了，`orphan` 只剩
「引擎 teardown 退回的槽位」一种用途，注释仍写着「总线删除时的孤儿槽位」。
应更新注释或评估是否需要独立字段。

---

## 4. 今晚所有 commit 相关 crate 的代码质量问题

> 范围：`699c8fe5`..`2b206244`，主要涉及 `yinhe-types`、`yinhe-mixer`、
> `yinhe-audio`、`yinhe-editor-core`、`yinhe-egui`、`yinhe-yin`、`yinhe-core`。

### 4.1 误删与恢复（流程问题，但造成了实际风险）

| commit | 问题 | 后果 |
|---|---|---|
| `7a1aece0` | 批量脚本删除 `graph.rs` 测试时，误删 5 个纯通道发送测试 | 发送核心行为一度失去测试覆盖，后来补回 |
| `7a1aece0` | `params.rs` 的 `midi_sends_snapshot` / `audio_sends_snapshot` 被连带删除 | 编译失败后才发现，后来恢复 |
| `7a1aece0` | `controls.rs` 删 `self_send_toggle` 时截断文件末尾 | 误删 `strip_controls` / `master_controls` 等，后来从 HEAD 恢复 |

**教训**：批量脚本删除必须 `git diff --stat` 核对删除行数，且删除后先
`cargo check` 再继续下一步。

### 4.2 `yinhe-mixer`

1. **`SendParams` 与 `ChannelSend` 两套发送并存**：
   - `MixerNode.sends: Vec<SendParams>`（旧，节点内嵌）
   - `MixerParams.channel_sends: Vec<ChannelSend>`（新，统一表）
   - 两者都表达「源 → 目标」，冗余。应删除 `SendParams`，统一到 `ChannelSend`。
2. **`sanitize_automation` 的 `resolved` 表每帧重建**：只在 `ensure_len` 调用，
   可接受，但 `HashMap` 分配可以改为按需查找。更重要的是**它之前漏了 TrackLc**，
   导致自动化被静默删除——这是「失败静默」的典型案例，应加日志。
3. **`MixerParams::ensure_len` 做了 12 件事**：`ensure_racks` / `ensure_channels` /
   `ensure_aps` / `ensure_channel_sends` / `ensure_send_amounts` /
   `ensure_track_channels` / `ensure_audio_channels` / `clear_mixed_lc_freezes` /
   `ensure_master` / `ensure_instrument_uids` / `sanitize_routing` /
   `sanitize_automation`。这是上帝方法，任何一项失败都难以定位。应拆成
   `repair_*` 系列并在调试日志里逐项输出。

### 4.3 `yinhe-editor-core`

1. **`show_track_fields` 返回值语义混淆**（见 1.7）。
2. **`link_track_to_own_lc` 只能创建新 LC**：无法链接到已有 LC，导致 1.4 的
   「选 LC 通道」无法实现。应增加 `link_track_to_lc(track_idx, lc)`。
3. **`ensure_rack_track_lcs` 同时负责 5 件事**：音频目标补齐、LC 创建/删除、
   AP 同步、Rack 模式推断、revision 标记。应拆分，否则每次改一处都担心副作用。
4. **`AutomationTarget::MixerSend` 改名后注释未同步**：
   `display_max` / `default_value` 的注释仍写「到某条 Bus 的发送量」。

### 4.4 `yinhe-egui`

1. **`mix.rs` 的 `instrument_hosts` 放在 UI 层**：它读取
   `doc.data.model.tracks` 并解析 `TrackAudioTarget`，这是文档层职责。应移到
   `yinhe-editor-core`，UI 只消费结果。
2. **`InstrumentRack::slot_mut` 是 O(n²)**：每个 slot 都 `seen.contains`，
   在 slot 多时（同通道多轨 × 多插件）会退化。当前量级可接受，但应在注释里
   写明，或改用 `HashMap<(u8, u64), usize>` 索引。
3. **`sync_states_to` 双结构去重**：`written: HashSet` + `next_index: HashMap`，
   可以合并成一个 `HashMap<(u8, u64), usize>`。
4. **`auto_panel_height(bus_count)` → `auto_panel_height()` 后仍保留参数名的
   注释**（「基础布局 + 发送区按总线数扩展」），已过时。
5. **`strip.rs` 的 `focus_send_info` 返回 5 元组**，可读性差，应改成 struct。
6. **`mode_bar.rs` 的 `AR+CH` 按钮**（见 1.3）。
7. **`track_panel.rs` 的 AM 双 M/S**（见 1.5）。
8. **`channel.rs::output_selector` 的注释仍写「Master / BUS n」**，已过时。

### 4.5 `yinhe-types`

1. **`AutomationTarget::MixerSend` 的字段名 `target` 与 `node` 易混**：
   `node` 是「哪个混音节点的发送」，`target` 是「发送到哪个通道」。应改名为
   `source` / `target`。
2. **`ArRow` 删除 `Bus` 后没有简化 `track_opt` / `track` 的 `unwrap_or(0)`**：
   现在 `track_opt` 对 `Track`/`Automation` 都返回 `Some`，`track()` 可以
   直接 match，不需要 `unwrap_or(0)` 掩盖「固定行」语义。

### 4.6 `yinhe-yin`

1. **v9 的 `SendParamsV9` 保留 `bus: u16` 但不使用**，只靠 `#[allow(dead_code)]`
   压制警告。应加注释说明「仅用于反序列化布局」，或改名为 `_bus`。
2. **v9 测试的 `old.sends[3].push(...)` 与 `midi_outputs` 设置了但断言丢弃**：
   测试意图是「验证丢弃」，应加注释明确，否则后来的人会以为是遗漏。

### 4.7 `yinhe-core`

1. **`TrackAudioTarget::OwnLc.rack` 字段未真正按轨分配**：当前仍是
   `RackId::new(global_channel)`，与「每轨实例」的持久化需求不一致。
2. **`allows_automation_target` 的 `_target` 参数未使用**：可以删除参数，
   或保留为未来扩展并加注释。

---

## 5. 修复优先级与执行顺序

| 优先级 | 问题 | 工作量 | 风险 |
|---|---|---|---|
| P0 | 1.5 单按钮 M/S（全 UI 统一） | 小 | 低 |
| P0 | 1.6 AP 显示 A01 | 小 | 低 |
| P0 | 1.2 旋钮只显示 OFF/dB | 小 | 低 |
| P1 | 1.1 MIX 选中高亮 | 小 | 低 |
| P1 | 1.3 删除 AR+CH 按钮 | 小 | 中（涉及布局） |
| P1 | 1.7 LC 改动 AR 生效 | 中 | 中（涉及引擎重建） |
| P2 | 1.4 音轨属性输出层级重构 | 大 | 中 |
| P2 | 3.2 删除 `SyncSends` / `SendParams` 冗余 | 中 | 中 |
| P2 | 3.5/3.6 删除 `ChannelOutput` / `instrument_output_dense` 死抽象 | 小 | 低 |
| P3 | 3.1 `ensure_cpu_synth` 去 `expect` | 小 | 低 |
| P3 | 3.3/3.8 乐器 dense 列表 | 中 | 低 |
| P3 | 4.2/4.3 上帝方法拆分 | 大 | 低 |

---

## 6. 我接下来会怎么做

1. **先修 P0/P1，每修一个就跑 UI 自检**（不是只看测试绿）。
2. **修完 1.4 之前，不再新增任何 UI 控件**。
3. **每个 commit 只做一件事**，正文写清「用户验收点」。
4. **批量删除脚本一律先 `git diff --stat`**。
5. **交付前按 16 条口径逐条打勾**，没做到的明确写「未覆盖」，不糊弄。

---

---

## 8. 代码质量专项列举（过长 / DRY / 内聚耦合 / 层级）

> 数据来自 2026-11-18 的静态扫描（脚本统计顶层函数体行数、文件行数、字段数）。
> 只列**本轮改动过或直接相关**的代码，按问题类型分组。

### 8.1 过长代码

#### 8.1.1 超长函数（>200 行）

| 函数 | 行数 | 所在文件 | 问题 |
|---|---:|---|---|
| `arrange::show` | **1030** | `crates/yinhe-egui/src/arrange.rs` | 布局、行构建、菜单、dock、音频编辑、自动化、选区、按钮绘制全在一个函数 |
| `track_panel::show` | **710** | `crates/yinhe-egui/src/arrange/track_panel.rs` | 行渲染、M/S、自动化子行、右键、拖拽、加号 |
| `interaction::handle_interactions` | **313** | `crates/yinhe-egui/src/arrange/track_panel/interaction.rs` | 双击/单击/右键/拖拽/框选/自动化 lane 命中 |
| `track::show_track_fields` | **281** | `crates/yinhe-egui/src/right_panel/info_panel/track.rs` | 名称/端口/通道/颜色/输出/LC/AP 全在一个函数 |
| `dispatch_and_find_next` | **279** | `crates/yinhe-audio/src/engine_render/dispatch.rs` | CC/NoteOn/NoteOff 三段 + 边界扫描 + 插件投递 |
| `mix::show` | **228** | `crates/yinhe-egui/src/mix.rs` | MIX 布局、通道条、Master、工具栏、扫描弹窗 |

#### 8.1.2 长函数（100–200 行）

| 函数 | 行数 | 所在文件 |
|---|---:|---|
| `automation_edit::apply_automation_edits` | 166 | `crates/yinhe-editor-core/src/document/automation_edit.rs` |
| `strip::header_context_menu` | 142 | `crates/yinhe-egui/src/mix/strip.rs` |
| `device_chain::collect_dock_state` | 140 | `crates/yinhe-egui/src/chrome/device_chain.rs` |
| `device_chain::apply_dock_actions` | 134 | `crates/yinhe-egui/src/chrome/device_chain.rs` |
| `engine_mixer::apply_mixer_automation` | 128 | `crates/yinhe-audio/src/engine_mixer.rs` |
| `device_chain::draw_dock` | 126 | `crates/yinhe-egui/src/chrome/device_chain.rs` |
| `mix::apply_action` | 122 | `crates/yinhe-egui/src/mix.rs` |
| `graph::process_channels` | 115 | `crates/yinhe-mixer/src/graph.rs` |
| `track::show_color_row` | 118 | `crates/yinhe-egui/src/right_panel/info_panel/track.rs` |
| `graph::refresh_pdc` | 81 | `crates/yinhe-mixer/src/graph.rs` |
| `plugins::render_instruments` | 86 | `crates/yinhe-audio/src/engine_render/plugins.rs` |
| `controls::focus_send_block` | 87 | `crates/yinhe-egui/src/mix/strip/controls.rs` |
| `mixer_section::into_current` | 74 | `crates/yinhe-yin/src/mixer_section.rs` |
| `automation_edit::make_doc_with_anchor`（测试辅助） | 82 | `crates/yinhe-editor-core/src/document/automation_edit.rs` |

#### 8.1.3 超大文件

| 文件 | 行数 | 备注 |
|---|---:|---|
| `crates/yinhe-audio/src/engine_tests.rs` | 3454 | 测试可以大，但应按主题拆成多个 `*_tests.rs` |
| `crates/yinhe-audio/src/spawn.rs` | 2159 | 命令定义 + worker + 音色加载 + 回调 + 测试 |
| `crates/yinhe-audio/src/audio_model.rs` | 1998 | 事件枚举 + 排序键 + 聚合 + ActiveNote + PreparedModel + AudioModel + 测试 |
| `crates/yinhe-editor-core/src/document.rs` | 1844 | Document + 导入导出 + 轨道操作 + 混音同步 + 选区 |
| `crates/yinhe-mixer/src/graph.rs` | 1657 | PDC + 渲染 + DelayLine + MasterChain + ChannelChain + 测试 |
| `crates/yinhe-mixer/src/params.rs` | 1292 | MixerParams + 12 个 ensure/sanitize + 测试 |
| `crates/yinhe-egui/src/mix.rs` | 1288 | MixState + 30+ MixAction + 绘制 + 处理 + 电平表 |
| `crates/yinhe-egui/src/arrange.rs` | 1282 | 视图编排 + 菜单 + dock + 音频编辑 |

**修复建议**：优先拆 `arrange::show` 与 `track_panel::show`（按「布局 / 行渲染 /
交互 / 菜单」四段）；`audio_model.rs` 按「事件 / 聚合 / 模型」拆模块；
`engine_tests.rs` 按功能拆文件。

---

### 8.2 违反 DRY

#### 8.2.1 `mix/strip.rs` 三个通道条函数几乎是复制粘贴

`channel_strip`（70 行）、`audio_strip`（70 行）、`track_lc_strip`（75 行）的
结构完全一致：

```
取 params → focus_send_info → strip_frame(title, color) →
  section(send_height, focus_send_block) →
  strip_controls(StripAutomationTarget::X) →
  MixAction::SetNodeStrip { node, params }
```

只有 `title`、`color`、`MixerChannelId`、`StripAutomationTarget` 四个参数不同。
**应抽一个 `node_strip(app, ui, idx, node, title, color, target, ...)`**，三个
包装函数只负责算 title/color。

#### 8.2.2 通道显示标签分散且命名不一致

`crates/yinhe-egui/src/mix.rs` 里同时有：

- `midi_label(port, channel)` / `channel_label(global)` / `ap_label(global)` /
  `audio_label(ch)` / `lc_label(display)` / `channel_title(mixer, ch)` /
  `channel_id_label(id, map)`。

`channel_title` 与 `channel_label` 只差「是否查 AP」，命名无法区分；`ap_label`
和 `channel_id_label` 又各自拼一次 AP 文案。**应合并到一个 `labels` 模块**，
统一入口 `track_target_label` / `mixer_channel_label`，避免 AP 显示改一处漏一处
（这正是问题 1.6 的温床）。

#### 8.2.3 乐器宿主解析有两套

- `crates/yinhe-egui/src/mix.rs::App::instrument_hosts`：解析
  `TrackAudioTarget` → `Vec<Option<LogicalChannelId>>`；
- `crates/yinhe-editor-core/src/document/rack_edit.rs::ensure_rack_track_lcs`：
  同样遍历 `TrackAudioTarget`，决定 LC 的创建/删除。

两处对「OwnLc / AP / DirectMaster」的判断必须永远保持一致，否则 UI 装的实例和
引擎期望的宿主会对不上。**应把宿主解析收进 `yinhe-editor-core`**，返回
`Vec<InstrumentHost>`，UI 与 rack 共用。

#### 8.2.4 M/S 按钮有两套实现

- `mix/strip/controls.rs::ms_column`：单按钮 + 三态文字 M/S；
- `arrange/track_panel.rs`：AM 子行 M、S **两个**按钮；普通轨又是另一个
  inline 按钮分支。

应统一到一个 `audibility_button`（见 1.5）。

#### 8.2.5 右键菜单有两套

`mix/strip.rs::header_context_menu`（142 行）与
`arrange/track_panel/interaction.rs::track_context_menu`（210 行）都在拼
「重命名 / 删除 / 自动化 / 设备链」菜单项，文案和快捷键各写一遍。

#### 8.2.6 `sync_output_routing` 与 `sync_sends_to_graph` 重复遍历

`crates/yinhe-audio/src/engine_mixer.rs` 两个函数都对每个 dense 调
`dense_to_namespace` 做一次映射；可以合并成一次遍历同时收集输出与发送。

#### 8.2.7 v9 迁移与 `ensure_len` 重复补齐

`yinhe-yin::MixerParamsV9::into_current` 手工补齐 Rack/节点/LC，随后
`decode_mixer_section` 又调 `params.ensure_len()`。前者的补齐逻辑与
`ensure_*` 系列重复，且容易漏字段。

---

### 8.3 低内聚高耦合

#### 8.3.1 `AudioEngine` 是 53 字段的上帝对象

`crates/yinhe-audio/src/engine.rs` 的 `AudioEngine` 同时持有：

- 混音图（`mixer` / `mixer_params` / `instruments` / `channel_dsp`）；
- 播放状态（`sample_position` / `current_tick` / `playing` / `active_notes`）；
- 模型与事件（`yin_model` / `cc_events` / `note_cursor`）；
- 插件回收（`insert_returns` / `instrument_returns`）；
- 预览、导出、冻结、电平表、循环偏移……

任何一处改动都要碰这个 struct。**建议按「混音 / 播放 / 插件宿主」拆成三个子
结构**（`MixerState` / `PlaybackState` / `PluginHostState`），`AudioEngine` 只做
聚合与命令分派。

#### 8.3.2 `MixerParams::ensure_len` 一次做 12 件事

`crates/yinhe-mixer/src/params.rs`：

```rust
ensure_len() →
  ensure_racks / ensure_channels / ensure_aps / ensure_channel_sends /
  ensure_send_amounts / ensure_track_channels / ensure_audio_channels /
  clear_mixed_lc_freezes / ensure_master / ensure_instrument_uids /
  sanitize_routing / sanitize_automation
```

任何一项改动都可能影响别的项（`sanitize_automation` 漏 TrackLc 就是例子）。
**应拆成独立 `repair_*` 并按依赖顺序显式调用**，每项可单测。

#### 8.3.3 `arrange::show` 1030 行：视图/数据/交互全耦合

`crates/yinhe-egui/src/arrange.rs::show` 内同时：

- 计算 `bus_ids`（已删）、`track_mixer_lanes`、`row_layout`；
- 处理 `TrackAction` 的 20+ 分支（含增删轨、文件夹、自动化、混音）;
- 打开 dock、处理右键菜单、处理音频片段编辑。

**建议拆成**：`build_arrange_data`（数据快照）、`handle_track_actions`（动作
落地）、`draw_arrange`（纯绘制）。

#### 8.3.4 `mix::show` 228 行 + `mix::apply_action` 122 行

`MixUiState` 18 个字段，`MixAction` 30+ 变体，绘制与动作处理都在 `mix.rs`。
**建议把 `MixAction` 的定义与处理移到 `mix/actions.rs`**，`show` 只负责布局。

#### 8.3.5 `audio_model.rs` 混合了事件、聚合、模型、测试

1998 行里同时定义 `ControlEvent` / `ChannelAudioEvent` / `AudioEvent` /
`SortedCC` / `PluginParamEvent` / `ActiveNote` / `PreparedModel` / `AudioModel` /
`AggKey` 及大量测试。**建议拆成 `audio_model/event.rs` / `aggregate.rs` /
`model.rs`**。

#### 8.3.6 `RendererSharedState` 是 20+ `Arc<Atomic*>` 的口袋

`crates/yinhe-audio/src/audio_renderer.rs` 的 `RendererSharedState` 把渲染线程
与 UI 线程共享的所有原子量/锁都塞进一个 struct，没有分组。删除 `bus_readings`
时就能体会到：每次增删都要改构造、所有使用点。**建议按「播放位置 / 音色库加载 /
电平表 / 欠载统计」分四个子结构**。

---

### 8.4 所属 crate / 级别有误

#### 8.4.1 `instrument_hosts` 在 UI 层解析持久化模型

`crates/yinhe-egui/src/mix.rs::App::instrument_hosts` 直接读
`doc.data.model.tracks` 并匹配 `TrackAudioTarget`。这是**文档/模型层职责**，
应在 `yinhe-editor-core`。UI 只应消费 `Vec<Option<LogicalChannelId>>`。

#### 8.4.2 `channel_id_for_ui` / `channel_id_label` 混淆「模型映射」与「显示」

`crates/yinhe-egui/src/mix.rs`：

- `channel_id_for_ui` 是 `MixerChannelId → ChannelId` 的**模型解析**，
  `yinhe-mixer` 已经有 `channel_id_for`，重复且可能不一致；
- `channel_id_label` 是**显示文案**，却与模型映射放在同一文件。

应删除 `channel_id_for_ui`，统一用 `MixerParams::channel_id_for`；显示文案独立
成 `labels` 模块。

#### 8.4.3 `engine_mixer::mixer_channel_of` 在音频层引入 UI 概念

```rust
fn mixer_channel_of(id: ChannelId, mixer: &MixerParams) -> Option<MixerChannelId>
```

`MixerChannelId` 是「UI 查询键」（Midi/Audio/TrackLc/Master），音频引擎只需要
`ChannelId`。这个函数把 UI 概念带进了 `yinhe-audio`，且与
`MixerParams::channel_id_for` 反向重复。

#### 8.4.4 `AutomationTarget::display_name` 放在 `yinhe-types`

`crates/yinhe-types/src/automation.rs::display_name` 返回 UI 文案（例如
`Mixer Gain {:?}`），属于展示层。类型层只应提供 `kind` / `default_value` 等
语义，文案由 UI 的 `labels` 模块负责（也便于 i18n）。

#### 8.4.5 `SendParams` 被持久化 / 引擎 / UI 三层共享

- 持久化：`MixerNode.sends: Vec<SendParams>`；
- 引擎：`AudioCommand::SyncSends` 搬运 `Vec<Vec<SendParams>>`；
- UI：`MixAction::SetSend` 直接改 `node.sends`。

但渲染图实际只用 `MixerParams::channel_sends`。**三层共享一个已不参与渲染的
struct**，是 1.7「改了不生效」的同类风险。应删除 `SendParams` 与 `SyncSends`
（见 3.2）。

#### 8.4.6 `ChannelSource::Unspecified` 长期存在于类型层

`ChannelSource` 的 `Unspecified` 是「加载时还没推断出角色」的临时状态，却作为
公开变体长期存在，导致所有 match 都必须处理它。**应改为 `Option<ChannelSource>`
或在加载后保证补齐**，而不是留一个永久的「未知」态。

#### 8.4.7 `RackOutputMode` 在 `yinhe-core`、`RackParams` 在 `yinhe-mixer`

`TrackAudioTarget::OwnLc { rack: Option<RackId> }` 在 `yinhe-core`，而
`RackParams.output_mode: RackOutputMode` 在 `yinhe-mixer`。两边都描述同一个
「Rack 拓扑」，但类型分属两个 crate，`yinhe-core` 又不能依赖 `yinhe-mixer`
（否则循环）。**建议把 `RackId` / `RackOutputMode` 留在 `yinhe-types`**，
`yinhe-core` 与 `yinhe-mixer` 共同依赖。

#### 8.4.8 `TrackAudioTarget` 的 `rack` 字段没有按轨分配

`effective_audio_target()` 对 MIDI OwnLc 返回
`rack: Some(RackId::new(global_channel))`，即持久化仍按 MIDI 通道共享。这与
「每轨插件实例」的运行时行为不一致（运行时每轨一个实例，持久化只有一份配置）。
**要么承认「配置共享、实例每轨」，并在字段注释写清；要么给 RackId 增加按轨
区间**（`TRACK_RACK_BASE + track_index`）。

---

### 8.5 其他具体问题

1. **`ChannelOutput` 只剩一个变体**（`graph.rs`）——死抽象，删。
2. **`instrument_output_dense` 恒等函数**（`channel_layout.rs`）——删。
3. **`ensure_cpu_synth` 用 `expect`**（`engine.rs`）——改 `CpuSynth` 非 Option。
4. **`MixerRack::orphan` 注释仍写「总线删除」**——过时。
5. **`strip.rs::focus_send_info` 返回 5 元组**——改成 struct。
6. **`mix.rs::instrument_hosts` 每次分配 Vec + contains 去重**——轨数少可接受，
   但应注释复杂度。
7. **`InstrumentRack::slot_mut` O(n²)**——大工程 slot 多时退化。
8. **`sync_states_to` 双结构去重**——可合并。
9. **`ArRow::track()` 的 `unwrap_or(0)`**——删 Bus 后不再有固定行，
   `track_opt` 恒 `Some`，可以简化。
10. **`AutomationTarget::MixerSend { node, target }` 命名易混**——`node` 是源
    节点，`target` 是目标通道，建议改 `source` / `target`。

---

## 8.6 修复进度（2026-11-18）

### 已完成

| 项 | commit | 说明 |
|---|---|---|
| 1.5 单按钮 M/S | `d84b5609` | `widgets::audibility` 统一 4 处（AR 轨、AR AM 行、通道 AR AM 行、MIX）；按钮恒显示 M，左键静音右键独奏 |
| 1.6 AP 显示 A01 | `d84b5609` | `ap_label` 改为端口+通道；测试覆盖 A01/A10/B01 |
| 1.2 旋钮只显示 OFF/dB | `d84b5609` | 删除通道名行；`send_db_label` 输出 `OFF` 或 `+x.x dB` |
| 1.3 回滚口径 #7 | `c7762d4c` | `git revert 60b448a3`：删除 AR+CH 开关、左右分栏、选择同步 |
| 1.1 MIX 选中背景点亮 | `9aede5c6` | `strip_frame` 选中背景用 `selection.bg_fill`；单击通道条同步 `track_selected` |
| 1.7 LC 改动 AR 生效 | `db27657d` | 拆分 `port_changed`/`routing_changed`；AR 轨道行显示 LCxx/---；强制重建引擎 |
| 1.4 音轨属性输出层级 | `d689aeb0` | Master/LC/AP 三层；LC 不显示端口通道；新增 `link_track_to_lc` |
| 8.2.1 通道条抽公共函数 | `a714b047` | `node_strip` 统一三条通道条 |
| 8.2.2 统一 labels | `5becb5a8` | `mix/labels.rs`；新增 `track_target_label` |
| 8.4.1 乐器宿主移入 editor-core | 随后提交 | `Document::instrument_hosts` |
| 3.5/3.6 死抽象 | `4e6c11d0` | 删除 `ChannelOutput` / `instrument_output_dense` |
| 3.1 CpuSynth 非 Option | `3f5dab7d` | 删除 `ensure_cpu_synth` 的 `expect`、`CpuSynthSlot` |
| 3.2 SendParams/SyncSends | `c635c41a` | 删除三层共享的死发送链路（-252 行） |
| 8.5 ArRow/注释 | `61b0c3d2` | `track()` 去 `unwrap_or`；orphan 注释 |
| 8.3.4 MixAction 拆分 | `a1093c50` | `mix/actions.rs`，mix.rs 1212→1041 |
| 8.3.2 ensure_len 拆 repair_* | `79e740cd` | 12 项统一命名，顶部写明依赖顺序 |
| 8.4.2 channel_id_for_ui 移入模型层 | `32f91a86` | 删除 UI 重复的 AP 解析 |
| 8.1 arrange::show 动作拆分 | `072fae1a` | `arrange/actions.rs`，arrange.rs 1289→1131 |
| 8.4.3 mixer_channel_of 移入模型层 | `ad3face0` | 音频层不再自带 UI key 反查 |
| 8.4.8 Rack 配置/实例边界 | `ad3face0` | `OwnLc.rack` 注释写明共享配置、每轨实例 |
| 8.3.6 RendererSharedState 分组 | `531fe702` | playback / sf / underrun 三组 |
| 8.1 track_panel AM 行拆分 | `73620ba7` | `render::draw_automation_row`，759→629 |
| 8.1 track_panel 普通轨行拆分 | `3a14babf` | `render::draw_track_row`，629→474 |
| 生产路径 panic 源 | `9a0126b6` | 音频乐器链/和弦排序/归档对话框/网格线/仓储等 11 处 unwrap/expect 改为防御性处理 |
| 8.3.1 AudioEngine 拆分 | `96cbdb09` `61385170` `ce9fd960` `d8bd5066` | PluginHost/Mixer/Synth/Export/Playback/Meter 六组，53 → 9 字段 |
| 8.3.2 ensure_len repair_* | `79e740cd` | 12 项显式命名与依赖顺序 |
| 8.4.2 channel_id_for_ui | `32f91a86` | AP 解析进 MixerParams |
| 8.4.3 mixer_channel_of | `ad3face0` | 逆映射进模型层 |
| 8.4.4 display_name 移出 types | `6cf6693b` | yinhe-theme::automation_label trait |
| 8.4.6 Unspecified 删除 | `ed8cf631` | 构造必须给角色，不再有临时态 |
| 8.2.5 右键菜单统一 | `e63e7b8c` | widgets::menu::action 唯一实现 |
| 8.2.7 v9 迁移复用 ensure_lc_node | `3ba43798` | 迁移与 ensure_len 同一条路径 |
| 8.5.5~8.5.10 具体项 | `f0e88037` | FocusSend 结构、O(n²)、MixerSend source |
| 8.3.3 arrange::show 拆分 | `c9a79ccd` `67d9e472` `da2a35aa` | 标尺/事件行/空白双击/滚动条/面板/GPU 主区，1030 → 399 |
| 8.1 track_panel::show 拆分 | `e85112ae` | 占位行/色带图标到 render，425 → 290 |
| 8.1 dispatch 拆分 | `c4f416b2` | CC / NoteOn / NoteOff 三段，289 → 11 |
| 8.1 音轨属性拆分 | `10ab4462` | 349 → 27，Conductor/名称/输出各自成函数 |
| 8.1 mix::show 拆分 | `c600a794` | 228 → 63，快照/主体/通道条/Master/重绘 |

### 未开始（P3，需单独排期）

| 项 | 原因 |
|---|---|
| 8.1.3 超大文件按主题拆分 | `audio_model.rs` / `engine_tests.rs` / `spawn.rs` / `document.rs` / `graph.rs` / `params.rs` 仍是单文件，需要单独排期 |
| 8.1.2 其余 100~200 行函数 | automation_edit / device_chain / graph / show_color_row 等 |

---

## 9. 修复顺序建议（含本章新增项）

| 优先级 | 项 | 说明 |
|---|---|---|
| P0 | 1.5 单按钮 M/S | 用户口径，影响 AR/MIX 全部通道 |
| P0 | 1.6 AP 显示 A01 | 用户口径 |
| P0 | 1.2 旋钮只显示 OFF/dB | 用户口径 |
| P1 | 1.1 MIX 选中背景点亮 | 用户口径 |
| P1 | 1.3 回滚口径 #7 全部功能 | 用户明确要求整体回滚 |
| P1 | 1.7 LC 改动 AR 生效 | 功能正确性 |
| P2 | 1.4 音轨属性输出层级重构 | 需要新增 link-to-existing-LC/AP |
| P2 | 8.2.2 统一 labels 模块 | 为 1.6 提供单点修复 |
| P2 | 8.2.1 通道条抽公共函数 | 为 1.1 提供单点实现 |
| P2 | 8.4.1 乐器宿主解析移入 editor-core | 消除 UI 层模型解析 |
| P3 | 8.1 长函数拆分 | 持续重构 |
| P3 | 8.3 AudioEngine 拆分 | 大工程，单独排期 |
| P3 | 8.4 其余层级问题 | 单独排期 |

## 7. 附：本轮 commit 清单（供回溯）

```
2b206244 test(MIX): yinhe-core 自动化测试改用 MixerGain
d4bb24d5 feat(MIX): 外部插件 Rack 支持按轨拆分（OwnLc）
48df219f docs(MIX): PDC 与 freeze 口径更新为无 Bus
328bb66e feat(MIX): OwnLc 轨的外部乐器插件按轨实例化
7a1aece0 refactor(MIX): 移除 Bus 概念，发送统一走任意通道
26deddb4 feat(MIX): LC 显示名改 LCxx，导出独立轨只占 Port A
15a17088 docs(MIX): 口径更新为无 Bus、LC 命名、只保留 Yinhe 音源
6032957c build(deps): Cargo.lock 同步移除 xsynth 依赖
a63b2307 test(MIX): 修正 v9 迁移测试对独立 u16 LC ID 的断言
3a54a01c refactor(types): 内置音源参数 API 从 XSynth 命名改为 builtin_synth
c0e0d28c refactor(ui): 内置音源 UI 命名从 XSynth 改为 Yinhe
468d86d3 refactor(audio): 去掉 yinhe-audio 对 xsynth-core 的依赖
dd407cf4 refactor(audio): 移除 XSynth 引擎，播放只走 YinheCpu
699c8fe5 refactor(audio): XSynth 移除第一阶段，合成后端收敛为 YinheCpu
11551c70 feat(AM): 自动化冲突标记与以哪条为准裁决
9922826c feat(MIX): 独立 Track 的 Auto 通道分配只在 Port A
a3f18506 refactor(MIX): LC 身份改为独立 u16 并显式记录来源
be39038a feat(MIX): 旧工程共享 home LC 拆成唯一 LC 并 SEND 汇总
42c5660f refactor(MIX): 移除多输出插件 legacy 路径
6bbfd02b feat(MIX): OwnLc 外部插件音频输出到自己的 home LC
60b448a3 feat(ui): ARRANGE 与 MIX 通道 AR 可同时显示
...
4996cb6f feat(MIX): AP 通道显示 AP01/AP10
dd818ae6 feat(arrange): AR 自动化 lane 支持 M/S 试听（Mute 旁通 / 音轨内 Solo）
3d84b95e feat(MIX): SEND 改为聚焦目标与单 dB 旋钮
```
