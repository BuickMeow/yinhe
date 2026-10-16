# 音符批量填缝 / 框选合并 Spec

状态：已实现（2026-10-16），待用户验证。核心语义已与用户确认，见文末「决策记录」

## 背景

黑乐谱素材（人工画谱 / MIDI 导入）里常见两类问题：

1. **断续 / 叠音**：音符之间留 1~几 tick 空隙，听感断奏；同轨道内又可能有起始不同但时间重叠的叠音（导入或编辑产生），听感混浊。
2. **碎片**：同 key 首尾严格相接的一串小音符（被切分/量化打碎），需要一个长音符继续编辑。

现状只有单音符手动拖边缘，框选后没有批量手段。本 spec 定义在 transport bar 的**选择动作区**新增两个批量操作，与既有「移调 / 翻转 / 变速 / 人性化」按钮同级。

## UI 入口

`crates/yinhe-egui/src/widgets/selection_actions.rs` 的 `SELECT_BAR_BUTTONS` 从 9 个扩到 11 个（追加末尾）：

- 填缝：`ICON_COMPARE_ARROWS`（⇄ 双向箭头，拉长/贴合）
- 合并：`ICON_LINE_STYLE`（线条样式）

行为与既有选择动作完全一致：

- 常驻显示；`selection_available == false`（无 PR/AR/通道框选、无自动化锚点选中）时灰显不可点；
- 悬停写入讲解行（`hint.pr.sel_fill_gaps` / `hint.pr.sel_merge_notes`）；
- 只对**音符**生效：AR 框选 / 通道框选 / 自动化锚点选中时点击为空操作（同「移调」「垂直翻转」现状）；
- 点击后经 `PianoViewEvent::SelectionAction` → `App::handle_selection_action` 分发；
- 同步加入编辑菜单（`EditAction::FillGaps` / `MergeNotes`，支持图钉；macOS 原生菜单经 `EDIT_GROUPS` 自动跟进）。

窄窗口风险：11 个按钮宽约 238px（18px 图标 + 4px 间距），需在 1280 宽下目视确认不挤到右侧 PR 控制组。

## 操作 1：批量填缝 `fill_gaps_selected`

### 语义

按**轨道**分组，各轨道独立处理（同一轨道内跨 key 共享一个「起音网格」；不同轨道互不参考）。

设某轨道被选中音符的起始 tick 升序去重为 `s₁ < s₂ < … < sₖ`：

- 起始 = `sᵢ`（i < k）的音符：`end_tick → sᵢ₊₁`；**只改终点**，起点 / key / 轨道 / 力度 / id 全不动；
- 起始 = `sₖ`（最后一个起音）的音符：保持各自原终点；
- 只有终点真正发生变化的音符进 undo delta；无任何变化返回 `None`（不产生 undo 项）。

结果性质：

- 同一起始的音符终点必然相同；
- 不存在「起始不同却时间重叠」——`sᵢ` 的音符止于 `sᵢ₊₁`，不可能压到更晚起点的音符上；
- 从 `s₁` 到最后一个起音组自身终点之间，每个 tick 都有音符在响（选框最后一个起音之后的空白不填充）。

### 边界

| 情况 | 行为 |
|---|---|
| 只有一个不同起点 | 无修改，返回 `None` |
| 音符原终点恰为 `sᵢ₊₁` | 该音符不变（不进 delta） |
| 长音符跨过后面的起点 | 被剪短到下一个起点（终点贴合） |
| 同起点、终点不同 | 统一到 `sᵢ₊₁`（最后一个起音组除外） |
| 最后一个起音组内同起点、终点不同 | 保持各自原终点（决策 3） |
| 选框外音符 | 不参与：既不被修改，也不作为贴合目标 |
| 属性筛选（`SelectionFilter`） | 生效：被筛掉的音符视同未选中，不参与、不作为下一个起点 |
| 多轨道框选 | 各轨独立，不互相参考 |
| 多 key 同一轨道 | 共享起音网格（琶音会被拉平成柱式，这是既定语义） |

### 算法与性能

1. 收集选中音符（经 `accepts_note` 判定）并按轨道分组，每轨按 `(start_tick)` 排序；
2. 一趟扫描得到去重起点序列，再写回各音符新终点（按 id 定位修改）。

复杂度 O(N log N)（key 桶内本身有序，可退化为 128 路归并 O(N log 128)）；内存峰值 ≈ 选中音符数 × 12B。

原子性：只改 `end_tick`，不换 key 桶、不增删音符。
Undo = 单个 `UndoAction::Notes(NoteDelta { before, after })`（无操作式 undo 的对称性前提问题）。
选区跟随：起点未动，选框与成员位图无需变更。

## 操作 2：框选合并 `merge_adjacent_selected`

### 语义

- 分组：**同一轨道 + 同一 key**（不同轨道即使同 key 也不合并）；
- 相连：前一个音符 `end_tick == 后一个音符 start_tick`（严格相等）；1 tick 空隙或 1 tick 重叠都断开；
- 链上所有音符都必须被框选（框外音符不算链的下一环；被筛选排除的音符同样断开链）；
- 合并：链条压缩成一个音符，保留**链首**音符的 id / 力度 / 轨道，`end_tick` = 链尾音符的 end，删除链内其余音符；
- 链长 ≥ 2 才合并；同一次操作内所有链合并打包成**一个** undo 项。

### 边界

| 情况 | 行为 |
|---|---|
| 无任何可合并链 | 返回 `None` |
| 同 key 不同轨道 | 不合并 |
| 3 个以上音符长链 | 一次压成一个（`A[0,10)+B[10,20)+C[20,30) → A[0,30)`） |
| 同一次点击存在多条链 | 全部合并，一个 undo 项 |
| 合并后选区 | 不动（成员位图里被删 id 悬空但无害，`accepts_note` 不会误命中） |

### 算法与性能

1. 逐 (轨道, key) 桶单趟扫描（桶内已按 start 排序），找出所有最大相邻链；
2. 链首原地改 `end_tick`，其余移入 undo `before` 并从桶中删除。

复杂度 O(N)；内存 = 被合并掉的音符数（进 delta）。
Undo：`apply_note_delta` 按 id 移除/插入；链首 id 复用为合并音符 id，undo/redo 对称（先 remove `after` 再 insert `before`，同一 id 可安全重放）。

## 代码触点清单（实现记录）

| 文件 | 改动 |
|---|---|
| `crates/yinhe-editor-core/src/document/note_edit/cleanup.rs` | `fill_gaps_selected` / `merge_adjacent_selected` + 助手 `apply_end_ticks` / `record_merge_chain`（note_edit 已按主题拆分，见下） |
| `crates/yinhe-editor-core/src/document/note_edit/tests/humanize_cleanup.rs` | 11 个单元测试 |
| `crates/yinhe-editor-core/src/shortcuts.rs` | 新增 `ACTION_FILL_GAPS` / `ACTION_MERGE_NOTES`（默认无快捷键） |
| `crates/yinhe-editor-core/src/audio_settings.rs` | `PINNED_EDIT_ACTIONS_LEN` 18 → 20 |
| `crates/yinhe-egui/src/widgets/selection_actions.rs` | `SelectionAction` 枚举 + `SELECT_BAR_BUTTONS`（9 → 11）+ `action_hint` + 唯一性冒烟测试 |
| `crates/yinhe-egui/src/chrome/transport_bar_actions.rs` | `EditAction` 新增两项（图钉索引 18/19）+ `EDIT_GROUPS` 新分组；「人性化」图标改 `ICON_PERSON` |
| `crates/yinhe-egui/src/app/actions.rs` | `handle_edit_action` / `handle_selection_action` 分发 + 两个 `with_undo` 包装函数 |
| `crates/yinhe-egui/src/app/main_loop.rs`、`platform/mod.rs`、`platform/macos/menu.rs` | macOS 原生菜单跟进（MenuAction + 分发） |
| `crates/yinhe-egui/src/shortcuts.rs` | `ACTION_FILL_GAPS` / `ACTION_MERGE_NOTES` 的 i18n label 映射 |
| `crates/yinhe-egui/locales/*.yml`（15 个） | `menu.` / `undo.` / `hint.pr.sel_` 共 6 个 key 全语言补齐 |

注意：`i18n_tests::all_locales_have_same_keys` 会校验 15 个语言文件 key 完全一致，新增 key 必须全语言补齐。

验证：`cargo test -p yinhe-editor-core`（325 通过）、`cargo test -p yinhe-egui`（331 通过）、`cargo clippy` 两个 crate 零 warning。

## 测试计划（已落地）

单元测试（`note_edit.rs`）：

1. 填缝：断续音符逐个终点贴合下一个起点；
2. 填缝：重叠时剪短前者、同起点统一终点（`A[0,300) B[100,200) C[300,400)` → `A[0,100) B[100,300) C[300,400)`）；
3. 填缝：最后一个起音保持原终点；
4. 填缝：按轨道分组，轨道之间不互相贴合；
5. 填缝：框外音符不作为目标、不被修改；
6. 填缝：属性筛选生效（被筛掉的音符不参与）；
7. 填缝：单起点返回 `None`；undo/redo 往返恢复；
8. 填缝：同轨道多 key 共享起音网格。

合并：

9. 合并：首尾链压成一个，保留链首 id / 力度，终点 = 链尾终点；
10. 合并：1 tick 空隙 / 1 tick 重叠都断开；
11. 合并：链上音符未全部框选 → 不合并（被筛选排除的音符断开链）；
12. 合并：同 key 不同轨道不合并；
13. 合并：多条链一次完成、单 undo 项回滚；
14. 合并：无可合并链返回 `None`。

UI 侧：`SELECT_BAR_BUTTONS` 数量与 action 唯一性冒烟测试。

## 决策记录

1. 填缝**按轨道分组**（同轨道跨 key 共享起音网格；用户选择，非按 (轨道,key)）；
2. 重叠对齐 = **终点贴合**（起点全不动，终点 = 下一个不同起点；类似 Cubase Legato 工具）；
3. 最后一个起音**保持原终点**，选框外音符不参与；
4. 合并保留**最早（链首）音符**的 id / 力度 / 轨道；同轨道 + 同 key、严格 `end == next.start`、链上全部被框选；
5. 两个操作**同时加入编辑菜单**（支持图钉）；图标：填缝 `ICON_COMPARE_ARROWS`、合并 `ICON_LINE_STYLE`；「人性化」图标改 `ICON_PERSON`。

## 可选后续

- 100M 级选中时的流式实现优化（当前按选区物化 + 逐桶 `update_matching`，按需再做）；
- 若需要：为两个动作设置默认快捷键（快捷键设置页目前也只列了部分动作）。
