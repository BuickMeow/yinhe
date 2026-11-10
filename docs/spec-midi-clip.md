# MIDI Clip Spec v1

状态：定稿 v1（2026-11-10）

前置依赖：`docs/spec-global-event-rows.md`（AR 专属全局事件小行 + 纯时间标尺）
先完成，Clip v1 的 AR 网格/行布局以该 spec 为准；PR 不显示事件行。

## 背景

黑乐谱工程动辄上亿音符，音符本身仍按 key 桶扁平存储，`.yin` 的轨段列式 + zstd
已经有很好的压缩效果。旧 `midi-clip` 分支的引用式模型（`ClipDef + ClipInstance`、
局部 id、展开索引、编辑即脱离）在运行时引入了大量间接层，收益没有落到 GPU、音频和
存档消费端，已废弃。

v1 的 Clip 重新定位为：

> **轨道上的命名时间区间，以及区间内音符的操作单位。**
>
> **逻辑上是容器，物理上是透镜**：音符仍然存放在原有的全局 `NoteBucket` 中，
> Clip 只保存区间元数据，不保存音符副本，不引入局部坐标和局部 id。

v1 不做存储去重、不做重复识别、不做跨 Clip 共享定义。`.yin` 的压缩已经足够，
复杂度应该花在编辑和视图上，而不是再建一套音符真源。

## 目标

- 每个音符都属于且只属于一个 Clip（v1 只针对音符）。
- 新工程/导入后每轨先有一个整轨 Clip，从该轨第一个音符 start 到最后一个音符 end。
- PR 工具栏提供 `[CLIP | SONG]` 两种时间范围模式，轨道显隐与原来完全一致。
- SONG 模式保留现有整曲/多轨编辑能力。
- CLIP 模式只允许编辑当前激活 Clip 的音符，其他轨音符灰显只读，点击直接忽略。
- SONG 模式下在空白处写音符时自动扩展左侧 Clip；与右侧 Clip 重叠时自动合并。
- 1 亿音符规模下，视口查询、编辑、播放、保存性能相对当前 master 不退化。

## 非目标（v1 范围外）

- Clip Loop / 拉长重复 / 修改一个音符影响所有轮次。
- Clip 级 PC（Bank / Sub-Bank / Program）；PC 仍是独立轨道事件。
- 自动化跟随 Clip；v1 的 Clip 操作完全不碰 automation lane。
- Linked Clip / 跨 Clip 共享定义 / 重复识别。
- MPE（每音符复音表情）。
- 旧 `.yin` 工程兼容与迁移。
- 全局事件行的实现；只作为前置依赖，见独立 spec。
- 改变轨道显隐、锁定、独奏等现有轨道语义。

## 术语

| 术语 | 含义 |
|------|------|
| Clip | 单轨上的命名时间区间，v1 只承载音符归属 |
| Content Range | Clip 的时间区间 `[start_tick, end_tick)`，用于音符 start 归属 |
| Ownership | 音符归属：同轨且 `note.start_tick ∈ [start_tick, end_tick)` |
| Overhang | 音符 gate 越过 Clip 右边界；归属仍看 start，不截断 |
| Active Clip | CLIP 模式下当前唯一可编辑的 Clip |
| Left Clip | 空白处写音符时，同轨时间上位于其左侧、最近的那个 Clip |
| SONG | 整曲/多轨时间范围模式，现有 PR 行为 |
| CLIP | 激活 Clip 时间范围模式，单 Clip 可编辑，其他轨只读 |

## 数据模型

```rust
pub struct Clip {
    /// 全曲唯一，发号器分配，不复用。
    pub id: u32,
    /// 显示名；默认 "Clip {id}"。
    pub name: String,
    /// 区间起点（绝对 tick）。
    pub start_tick: u32,
    /// 区间终点（绝对 tick），必须大于 start_tick。
    pub end_tick: u32,
}
```

`TrackData` 新增：

```rust
pub clips: Vec<Clip>, // 按 start_tick 升序、互不重叠
```

`YinModel` 新增：

```rust
pub next_clip_id: u32,
```

`YinModel.notes`、`Note`、`NoteBucket`、`NoteSource` 契约均不改动。
Clip 不携带任何音符数据，也不改变音符的全局 id 语义。

### 不变量

1. 同轨 `clips` 按 `start_tick` 升序，任意两个区间不重叠。
2. 每个 `Clip.start_tick < Clip.end_tick`；空 Clip（无音符）合法。
3. 该轨每个音符的 `start_tick` 恰好落在一个 Clip 内；不存在游离音符。
4. Clip 归属只看 `note.start_tick`；note gate 可以越过 `end_tick`，不截断、不改归属。
5. `Clip` 不改变音符的 key、力度、gate、全局 id 或 track 字段。
6. Clip 区间不因音符 gate 越界而自动变成重叠；边界只约束 start。

### Content Range 与显示块

- 初始创建（导入/新工程）时，`end_tick = 该轨最后一个音符的 end_tick`。
- 之后 `end_tick` 可由拆分、resize、自动扩展改变。
- 拆分可以让 `end_tick` 落在某个长音符的 gate 中间；该音符仍归左 Clip，
  右 Clip 从切点开始，两边归属区间不重叠。
- AR/PR 的 Clip 块按 `[start_tick, end_tick)` 绘制；gate 越过块边界时，
  音符本身照常绘制，必要时在块外显示为“尾音”。

## 归属与归一化

### 归属判定

```text
clip_at(track, tick):
  在该轨 clips 上二分，找 start_tick <= tick < end_tick 的唯一 Clip
```

- 查询是 O(log Clip 数)，不建立任何每帧索引。
- 一个音符的归属只依赖 `(track, note.start_tick)`，与 key、gate 无关。

### SONG 模式写出归一化

画音符、粘贴、拖动音符等所有写出操作，在提交前统一走一次归一化。
归一化按轨道批量执行，不逐音符做 O(Clips) 扫描。

对每个被写入/移动的音符：

1. `start_tick` 已落在某 Clip 内 → 归该 Clip。
2. 落在空白处 → 找同轨左侧最近的 Clip，扩展其 `end_tick` 覆盖音符 end。
   - 若音符 end 已超出原 `end_tick`，扩展 `end_tick = max(old_end, note.end_tick)`。
3. 左侧没有任何 Clip（例如落在第一个 Clip 之前，或轨道原本为空）→ 新建 Clip：
   `start = note.start_tick`，`end = max(note.end_tick, note.start_tick + 1)`。
4. 扩展或新建后与右侧 Clip 重叠（`new_end > right.start`）→ 触发合并。
5. 音符 start 小于当前 Clip start 的情况不允许出现：左 Clip 已覆盖其 start；
   若真的出现，说明数据损坏，按“新建 Clip + 合并”兜底。

批量实现建议：

- 按轨道收集待写入音符，按 start 排序；
- 一次扫描定位受影响的 Clip 区间，计算扩展/新建/合并后的边界；
- 只对受影响的 Clip 做 Vec 插入/删除，不重建整轨 Clip 列表；
- 复杂度 O(写入音符数 × log Clips + 受影响 Clip 数)。

### 移动/粘贴音符后的归一化

- 移动音符可能让它离开原 Clip、进入另一个 Clip 或空白处；
- 提交移动后同样执行一次归一化；
- 原 Clip 若仍有音符，保持原边界；若音符全部移出，Clip 可以保留为空区间
  （v1 建议保留，避免用户丢失区间书签；用户可手动删除空 Clip）。

## 创建与导入

### 新工程/导入 MIDI

- 每条非空音符轨道生成一个 Clip：
  `start = min(note.start_tick)`，`end = max(note.end_tick)`。
- 空轨不生成 Clip。
- 只有 PC/自动化、没有音符的轨道不生成 Clip（v1 Clip 只包含音符）。
- 导入不做自动切分，不做重复识别，不做折叠。
- 音符存储路径不变；Clip 只是元数据，不复制、不重排音符。

### 手动创建空 Clip

- 从当前播放头或选区创建，长度取当前量化单位（用户可以再 resize）。
- 空 Clip 合法，可作为区间书签或后续画音符的容器。
- 新建 Clip 与已有 Clip 重叠时不允许创建，或走合并规则；v1 建议拒绝并提示。

## 编辑操作

所有操作都只处理音符和 Clip 边界；PC、自动化事件完全不动。

### 拆分 Split

- 输入切点 `T`，要求 `clip.start < T < clip.end`。
- 左 Clip：`[start, T)`，保留原 id 和名字。
- 右 Clip：`[T, end)`，分配新 id，名字沿用原名（用户可重命名）。
- 音符按 start 归属：
  - `note.start < T` → 左；
  - `note.start >= T` → 右。
- 长音符允许 gate 越过 `T`，不切音符、不改 gate；左右归属区间仍不重叠。
- 左右任一侧可以为空 Clip。
- 一次 split = 一条 undo。

### 合并 Merge

触发场景：

- SONG 模式覆盖/扩展后与右侧 Clip 重叠；
- 拖动 Clip 到已有 Clip 上；
- 粘贴 Clip 到已有 Clip 上；
- 用户显式“合并相邻 Clip”。

语义：

- 区间取并集：`start = min(start)`，`end = max(end)`。
- 音符取并集，不做内容去重；完全相同的音符也保留（避免隐式改变演奏语义）。
- 保留“被合并到的那个 Clip”的 id 和 name：
  - 左扩展导致重叠 → 保留左 Clip；
  - 拖动/粘贴覆盖 → 保留目标位置原有的 Clip；
  - 显式多选合并 → 保留最左侧 Clip。
- 被合并的源 Clip 删除；其 id 不复用。
- 三块以上重叠按一次合并处理，仍然只保留一个 id/name。
- 一次 merge = 一条 undo。

### 移动 Move

- 选中 Clip 后拖动，移动该 Clip 内所有音符（按 start 归属）和 Clip 边界。
- 支持跨轨移动；音符 track 字段和 Clip 所在轨道一起改变。
- 移动到目标轨后若与已有 Clip 重叠 → 合并。
- 越过 tick 0 时整体 clamp，不允许负 tick。
- PC / 自动化不移动，留在原位。UI 必须有提示，避免用户以为自动化跟随。
- 一次 move = 一条 undo（音符移动 + Clip 快照）。

### 复制 Copy

- 复制 Clip = 复制其内音符（分配新全局 note id）+ 新建 Clip（新 id，名字沿用）。
- 支持跨轨、跨时间粘贴。
- 粘贴位置与已有 Clip 重叠 → 合并。
- 不建立任何共享引用；后续编辑两份互不影响。
- 一次 copy/paste = 一条 undo。

### 删除 Delete

- 删除 Clip 内所有 start 落在区间的音符。
- 删除 Clip 本身。
- 其他 Clip 的音符若 gate 越过本区间，不删除（归属看 start）。
- 空 Clip 直接删除，不产生音符变化。
- 一次 delete = 一条 undo。

### 缩放 Resize

- 左边界：
  - 非空 Clip：`start` 不能大于该 Clip 内最小音符 start；
  - 空 Clip：可在不越过左邻居的前提下自由移动。
- 右边界：
  - 非空 Clip：`end` 不能小于该 Clip 内最大音符 start + 1（gate 可越界）；
  - 空 Clip：自由。
- 不允许与左右邻居重叠；碰到邻居边界时 clamp 停住。
- 不自动合并（resize 是边界调整，不是覆盖操作）。
- 一次 resize = 一条 undo。

### 音符编辑

- 单选/框选/拖动/缩放/改力度/改 gate 仍走现有音符编辑命令。
- 编辑后执行归一化：
  - start 未离开当前 Clip → 只可能在 `end_tick` 不足时扩展右边界；
  - start 离开当前 Clip → 按 SONG 归一化处理（左扩展/新建/合并）。
- 不改变音符全局 id，不引入局部 id。

### Undo

新增：

```rust
UndoAction::Clips {
    track_idx: u16,
    before: Vec<Clip>,
    after: Vec<Clip>,
}
```

- Clip 边界变化使用 `Clips` 快照，成本 O(Clip 数)。
- 同时有音符变化时，用 `UndoAction::Composite` 聚合为一个 undo 项。
- 拆分、合并、移动、复制、删除、resize、自动扩展、合并都必须是单条 undo。
- 合并保留 id 的规则保证选区/undo 引用稳定。

## PR：`[CLIP | SONG]`

### 工具栏

- PR 工具栏增加分段选择器：`[CLIP | SONG]`。
- `SONG`：现有整曲/多轨时间范围，轨道显隐、锁定、独奏完全沿用现状。
- `CLIP`：以当前 Active Clip 的时间范围为窗口；轨道显隐仍沿用现状。
- 没有 Active Clip 时，`CLIP` 按钮不可用（或点击后提示先选 Clip，v1 建议禁用）。
- 切换模式只改变时间范围和编辑权限，不复制模型、不重建索引。

### SONG 模式

- 时间范围：不限，沿用现有 PR。
- 轨道范围：沿用 `track_pianoroll_visible` / 主音轨规则。
- 音符颜色：默认使用轨道色（彩色）。
- Clip 背景条带照常显示，作为区间参考。
- 写出操作执行 SONG 归一化。

### CLIP 模式

- 时间窗口 = Active Clip 的 `[start_tick, end_tick)`。
- 水平滚动/缩放限制在该时间窗内；不允许滚出窗口。
- 轨道显隐不变；Active Clip 所属轨强制可见。
- **只有 Active Clip 的音符可选中、可编辑。**
- 其他轨在窗口内的音符：
  - 灰显、更低透明度；
  - 只读；
  - 点击/框选/双击**直接忽略**：不选中、不编辑、不弹提示、不改变光标。
- 选区框选只作用于 Active Clip 的音符；其他轨不参与。
- 写入目标固定为 Active Clip 所属轨道。
- 在窗口内画音符：写入 Active Clip；gate 超出 `end_tick` 时扩展 Clip。
- 不允许在窗口外画音符（窗口即 Clip 范围）。
- 切换 Active Clip：通过 AR 选中/双击，或 PR 面包屑；灰音符点击不切换。
- 进入方式：AR 双击 Clip 头部；退出：Esc、面包屑点击、或工具栏切回 SONG。
- 进入/退出保存并恢复 PR 的滚动与缩放，避免视角丢失。
- 坐标仍使用绝对 tick；不引入 Clip 局部坐标。标尺可以显示小节/拍，不做第二套 tick。

### 音符颜色

| 模式 | 音符颜色 |
|------|----------|
| SONG | 轨道色（彩色） |
| CLIP，Active Clip 音符 | 灰度、正常透明度 |
| CLIP，其他轨音符 | 灰度、更低透明度、只读 |
| 任意模式，选中音符 | 强调色（主题 selection 色） |

- 颜色是视图派生，不写入音符/Clip 数据。
- 灰度模式不改变力度、轨道、选择语义，只改绘制颜色。

## 持久化

### 版本

- `.yin` 容器版本 bump；旧版本文件**直接拒绝加载**，提示“不支持该工程版本”。
- 不写迁移逻辑，不为旧工程生成 Clip。
- `mapping.json` 版本同步 bump。

### Clip 元数据

- 每轨 Clip 列表放在 `mapping.json` 的 `TrackMap.clips`（自描述 JSON，
  与 `audio_clips` 同层）：

```text
TrackMap {
  uuid, name, color, ...,
  clips: [{ id, name, start_tick, end_tick }]
}
```

- 音符段（data 段）本 spec 不改；Clip 不参与音符流编码。
- 加载时按 `start_tick` 排序校验；不合法（重叠、start >= end）直接报错，
  不做静默修复。
- `next_clip_id` 加载时由 max(id)+1 重算。

## 性能

| 操作 | 复杂度 |
|------|--------|
| `clip_at` / `clips_in_range` | O(log Clips + 可见 Clips) |
| 视口查询 | 与 master 相同，Clip 不参与每帧音符查询 |
| 编辑一个音符 | 与 master 相同，不做 Clip 深拷贝 |
| SONG 归一化 | O(写入音符数 × log Clips + 受影响 Clips) |
| 移动/复制/删除 Clip | O(Clip 内音符数) |
| 保存 | 音符段不变；仅多几 KB Clip 元数据 |
| 每帧重建 | 无 |

硬性要求：

- 不引入 `clip_id` 到 `Note`。
- 不引入 Clip 展开索引、局部 id、CoW。
- 不因为 Clip 数量增加而增加每帧音符查询成本。
- GPU / 音频事件表 / MIDI 导出路径不因 Clip 元数据改变。

## 分阶段实施

### 前置阶段：AR 全局事件行

先完成 `docs/spec-global-event-rows.md`：

- AR 标尺只负责时间；
- Marker / Meter / Key / Chord / Lyrics 移入 AR 网格上方的全局事件小行；
- 默认不显示，由 AR 左上角 `Events ▾` 菜单开启；Tempo 仍视为自动化，不进入事件行；
- PR 不显示事件行，PR 标尺保持原高度；
- Clip v1 的 AR 网格布局依赖该行系统。

### 阶段 1：数据层

- `Clip` 类型、`TrackData.clips`、`next_clip_id`。
- 二分查询、排序与不变量校验。
- 导入/新工程每轨生成一个整轨 Clip。
- `mapping.json` 存取、版本 bump、旧版本拒绝。
- Undo `Clips` 动作。
- 单元测试：归属、拆分、合并、移动、复制、删除、resize、undo/redo。

### 阶段 2：SONG UI

- AR 绘制 Clip 块（复用音频 Clip 的视觉参数，按轨色）。
- Clip 选中、移动、复制、删除、resize、拆分、合并。
- SONG 写出归一化（自动扩展左 Clip、重叠合并）。
- 音符颜色保持轨道色。

### 阶段 3：CLIP 模式

- PR 工具栏 `[CLIP | SONG]`。
- Active Clip 时间窗、水平滚动限制。
- 单 Clip 可编辑、其他轨灰显只读、点击忽略。
- 灰度音符绘制。
- AR 双击进入、Esc/面包屑退出。

### 阶段 4：打磨

- 重命名、右键菜单、多选合并、跨轨拖动、自动滚动。
- 移动端跟随（先保证桌面端语义稳定）。
- 后续再讨论 Loop、自动化跟随、Clip PC、Linked Clip。

## 测试与验收

### 单元/集成测试

- 同一轨任意两个 Clip 不重叠；任意音符 start 恰好属于一个 Clip。
- 导入后每轨一个 Clip，边界 = 第一音符 start / 最后音符 end。
- 空白处画音符扩展左侧 Clip；无左 Clip 新建；与右 Clip 重叠合并。
- 拆分点落在长音符 gate 内：音符不截断，归属左 Clip，右 Clip 从切点开始。
- 合并保留目标 Clip id/name，音符并集，源 Clip 删除。
- 移动/复制/删除 Clip 只动音符，PC/自动化不动。
- CLIP 模式：其他轨音符点击不选中、不编辑、不弹提示。
- CLIP 模式：框选只命中 Active Clip 音符。
- 旧 `.yin`：加载报“不支持该工程版本”，不崩溃。
- 保存/加载往返：Clip id、name、start/end、顺序一致。
- Undo/redo 在每个操作上对称。

### 性能验收

- 1 亿音符工程加载/保存相对 master 不增加可感知开销。
- PR/AR 每帧不因 Clip 总数增加而线性变慢。
- 编辑一个音符不触发 Clip 内容复制或全量重建。
- 按 AGENTS 约定只跑涉及 crate 的测试，不跑全 workspace。

## 决策记录

| 编号 | 决策 |
|------|------|
| 1 | Loop 不进第一版 |
| 2 | Clip 的 PC 可以为空，但 v1 不做 Clip PC；PC 仍是独立事件 |
| 3 | `[CLIP | SONG]` 只控制时间范围，轨道范围不变；CLIP 灰度、SONG 彩色 |
| 3a | CLIP 模式只有 Active Clip 的音符可编辑，其他轨灰显只读，点击直接忽略 |
| 4 | SONG 模式空白处写音符总是扩展左侧 Clip |
| 5 | 覆盖时自动合并音符与 Clip，保留被合并到的 Clip 的 id/name |
| 6 | 自动化暂时不跟随 Clip |
| 7 | v1 的 Clip 只包含音符 |
| 8 | 不兼容旧工程，旧 `.yin` 直接拒绝 |
| 9 | Pro Tools 式全局事件行在 Clip v1 之前完成，写入独立 spec |

## 已知取舍与风险

1. **Clip 移动不带走 PC/自动化**：这是 v1 的明确行为；UI 需要提示，否则用户会以为数据丢失。
2. **CLIP 模式点击其他轨直接忽略**：没有 toast，用户可能暂时不理解；Active Clip 切换入口必须明显
   （AR 选中、面包屑）。
3. **左侧扩展策略**：在长空白间隙里画音符会把左侧 Clip 拉得很长；这是“规则可预测”的代价，
   用户可手动拆分。
4. **合并不去重**：两块内容重叠时音符取并集，完全相同的音符也保留；避免隐式改变演奏语义。
5. **没有 Loop/自动化/PC**：v1 的 Clip 不是完整 DAW Clip，只是音符区间组织层；
   后续能力必须单独设计，不能把引用模型重新塞回来。