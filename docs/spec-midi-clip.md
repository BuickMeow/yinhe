# MIDI Riff（片段复用与重复识别）Spec

状态：草案 v1（2026-09-23）

## 背景

黑乐谱工程动辄上亿音符，其中大量内容是同一段音符画的重复（每拍 / 两拍 / 四拍 / 整段复现）。
现状下复制粘贴是逐音符深拷贝并分配新全局 id（`crates/yinhe-editor-core/src/document/selection.rs:96`），
重复内容在内存、`.yin` 存档、GPU 缓冲、音频事件表中被重复付账。

本 spec 定义「Riff 定义 + 实例」的引用式复用模型，并配套「重复识别 + 导入自动折叠」。
Riff 即 DAW 语境下的 MIDI Clip，本 spec 统一用「Riff」称呼。

## 目标与非目标

目标：

- 同一段内容被复用时只存一份（定义），复制/粘贴/移动实例为 O(1)；
- 工程「全部归 Riff」：不存在游离于 Riff 之外的音符；
- 严格精确的重复识别，导入 MIDI 后自动折叠，结果可一键展开还原；
- 保持 1 亿音符规模下的视口查询、播放、编辑性能不退化。

非目标（本 spec 范围外）：

- 变调复用（实例不带 `key_offset`，移调直接改写音符）；
- 编辑同步所有实例（编辑语义为「编辑即脱离」，定义编辑留作后续扩展）；
- 近似/容错重复识别（只做严格精确匹配）；
- 跨轨整体复用的「实例组」概念（跨轨音符画按每轨各自实例表示，实例组后续再议）。

## 术语

| 术语 | 含义 |
|------|------|
| RiffDef | Riff 定义：一段可复用的单轨内容（音符 + 自动化），局部坐标 |
| RiffInstance | Riff 实例：定义在时间轴/轨道上的一个放置，持有 `Arc<RiffDef>` |
| 内联实例 | 只有一个实例引用的 RiffDef（非重复内容），等价于就地存储 |
| 局部 ID | 音符在 RiffDef 内的唯一 id（u32，从 1 起） |
| NoteRef | 音符引用 = `(instance_id, local_id)` |
| 编辑即脱离 | 编辑共享定义时 CoW 出新定义，仅当前实例受影响 |

## 数据模型

### 总览

```
YinModel
├── conductor: Arc<ConductorData>          // tempo / time_sig / key_sig / 全局歌词和弦，仍在工程级
├── tracks: Vec<Arc<TrackData>>
│     TrackData
│     └── riff_instances: Vec<RiffInstance>    // 按 start_tick 排序，时间轴上可放多个
├── next_riff_def_id: u32
├── next_riff_instance_id: u32
└── 派生统计（note_count / tick_length / track_note_count ...）  // 由实例汇总维护
```

不再有全局 `YinModel.notes: Box<[Arc<NoteBucket>; 256]>`（`crates/yinhe-core/src/model.rs:265`），
key 桶下沉到 RiffDef 内部。

### RiffDef

```rust
/// Riff 定义：单轨内容 + 自动化，局部坐标。
pub struct RiffDef {
    /// 全曲唯一定义 id，持久化与 UI 引用用；发号器分配，不复用。
    pub id: u32,
    /// 显示名（可为空）。
    pub name: String,
    /// 256 个局部 key 桶，桶内按 start_offset 排序；空桶共享同一实例，无数据开销。
    pub buckets: Box<[Arc<NoteBucket>; KEY_COUNT]>,
    /// 自动化 lane（tick 为相对坐标）。
    pub automation: Vec<LocalLane>,
    /// 内容跨度 = 最大 end_offset（实例相交判定用）。
    pub tick_len: u64,
    pub note_count: u64,
    /// 最长音符长度（视口查询左扩，语义同现有 max_note_len）。
    pub max_note_len: u32,
    /// 局部 id 发号器（1 起，0 保留）。
    pub next_local_id: u32,
    /// 定义内实际使用到的 key 位图（增量刷新时 bump 对应 key revision）。
    pub used_keys: [u64; 4],
}

/// 局部音符。key 由桶下标隐含，track 由实例决定，故都不存字段。
pub struct LocalNote {
    pub id: u32,           // riff 内唯一
    pub start_offset: u32, // 相对 riff 起点
    pub end_offset: u32,
    pub velocity: u8,
}

pub struct LocalLane {
    pub target: AutomationTarget,
    pub events: Vec<AutomationEvent>, // tick 相对
}
```

- `NoteBucket` 的分块、排序、range 查询逻辑原样复用（`crates/yinhe-types/src/note_bucket.rs:26`），
  仅把元素类型从全局 `Note` 换成 `LocalNote`（去掉全曲 id / 绝对 track 语义）；
- 每个定义固定 256 × 8B = 2KB 指针开销（空桶共享），因此在任何路径上都要避免定义碎片化：
  非重复的连续区域合并成一个内联实例（见「导入流程」）；
- 定义是不可变对象，所有修改经 CoW 产出新 `RiffDef`。

### RiffInstance

```rust
pub struct RiffInstance {
    /// 全曲唯一实例 id（发号器分配，不复用）。
    pub id: u32,
    /// 运行时直接持有定义；持久化时按 def.id 引用。
    pub def: Arc<RiffDef>,
    /// 实例起点（绝对 tick）。
    pub start_tick: u32,
    /// 实例所在轨道下标；单轨定义，实例始终挂在 track_base 的 TrackData 上。
    pub track_base: u16,
}
```

- 实例占据区间 `[start_tick, start_tick + def.tick_len)`；
- 一个定义可被任意多个实例引用，可跨时间、跨轨道；
- 定义无任何实例引用且 undo 栈不再持有时，`Arc` 自然释放；不需要定义池。
  （保存时遍历实例按 `def.id` 去重收集即可。）

### 单轨定义的理由

- 跨轨音符画由「每轨一个定义 + 同时放置多个实例」表示，查询路径简单（实例只属于一个轨道）；
- 「同一声部复制到不同轨」仍共享同一定义，只是 `track_base` 不同；
- 将来若需要整组复制的「实例组」，在编辑器层做复合操作即可，不改模型。

## 身份模型

| 对象 | 作用域 | 说明 |
|------|--------|------|
| `RiffInstance.id` | 全曲唯一 | 选择、undo、命中、hidden/ghost 的锚点 |
| `RiffDef.id` | 全曲唯一 | 持久化、UI、去重收集 |
| `LocalNote.id` | 定义内唯一 | CoW 编辑时旧音符保留原 ID，新音符发新号 |
| 音符引用 | `NoteRef = (instance_id, local_id)` | 取代裸 `note.id` |

要求：

- 全曲唯一的两个 id 都**不复用**（删除后不回收），避免悬垂引用；
- 局部 ID **不落盘**，加载时按桶顺序重新分配（与现状 note id 不落盘一致，`crates/yinhe-yin/src/io.rs:632`）；
- 选区的全选走实例级 O(1) 表示，仅部分选择才物化每实例的局部 ID 位图（局部 ID 连续，位图页利用率高，无 u64 稀疏问题）。

## 实例放置与轨道

- `TrackData` 删除 `notes` 中转字段与 `automation_lanes`；新增 `riff_instances`；
- 实例列表按 `start_tick` 排序，插入/移动时维持有序（列表短于音符，成本可忽略）；
- 轨道增删/移动时，只需 remap 所有实例的 `track_base`（O(实例数)），
  取代现在对所有音符改 `track` 的全量遍历（`crates/yinhe-editor-core/src/document/track_ops.rs:98`）；
- 跨轨拖动实例 = 从原 track 的列表摘除、改 `track_base`、插入目标 track 列表。

## 查询与展开

所有消费者按「实例 → 定义 → 局部桶」路径读取，不物化全量副本。

### 视口查询（PR / AR / AM）

```
给定 track 与 tick 区间 [lo, hi)：
  1. 在 track 的 riff_instances 上二分，找与 [lo, hi) 相交的实例
  2. 对每个实例：
     local_lo = lo.saturating_sub(inst.start_tick)
     local_hi = hi.saturating_sub(inst.start_tick)
     遍历需要的 key：
       def.used_keys 位图快速排除不含该 key 的定义
       def.buckets[key].range(local_lo, local_hi)   // 与现状同一套 range
     输出时绝对 tick = inst.start_tick + local_offset，绝对 track = inst.track_base
```

- 查询成本 = O(相交实例数 × log + 可见音符数)，与全曲实例总数无关；
- PR 的 key 行渲染按「可见实例集合」逐个取桶，AR 按轨道行取，均复用现有构建器结构；
- `NoteSource` 契约（`crates/yinhe-types/src/source.rs:8`）需要扩展实例访问能力或新增 trait，
  具体接口在实施时定（候选：`fn instances(&self, track: u16) -> &[RiffInstance]` + `fn def(&self, id) -> ...`）。

### 统计

- `note_count` / `tick_length` / `max_note_len`：由实例汇总维护，
  实例增删改时增量更新：`note_count += def.note_count`（或反向减去）；
- `track_note_count` / `track_audible_count`：按 track 汇总实例；
- 定义级统计（`note_count` / `tick_len` / `max_note_len` / `used_keys`）是定义属性，随 CoW 重算，
  不是缓存。

### 增量刷新协议

- 现有 `note_revisions[key]`（`crates/yinhe-core/src/model_stats.rs:84`）保留，
  但 bump 集合改为：
  - 移动/删除/新增实例 → bump 该定义 `used_keys` 内的 key；
  - 定义 CoW → bump 该定义 `used_keys` 内的 key；
- GPU 的 per-key 增量上传（`crates/yinhe-egui/src/piano_view/gpu_upload.rs:442`）与音频 dirty 桶判定
  （`crates/yinhe-audio/src/spawn.rs:767`）消费协议不变。

### 音频

- 第一阶段：`prepare_model` 遍历实例展开 `AudibleNote`（`crates/yinhe-audio/src/prepare_model.rs:75`），
  事件表结构与调度器不变；
- 后续优化：调度器直接按实例迭代，避免展开事件表（可选）。

### MIDI 导出与保存

- 导出：`writer.rs` 前加一层「实例展开」为虚拟音符流（`crates/yinhe-midi/src/writer.rs:71`），
  writer 本体不改；
- 保存：定义只写一份（见「持久化」），文件体积按去重后大小。

## 编辑语义

核心规则一条：**编辑定义内容 = 若定义被多个实例共享则 CoW 出新定义给当前实例，否则原地修改**。
用 `Arc::make_mut` 语义表达，无阈值。

| 操作 | 行为 |
|------|------|
| 复制/粘贴实例 | 新建 `RiffInstance` 引用同一 `Arc<RiffDef>`，O(1) |
| 移动/删除实例 | 实例列表操作，O(实例数) |
| 编辑实例内音符（含移调） | `Arc::make_mut(def)`：共享时 CoW，单引用时原地；局部 ID 保留 |
| 整体移调实例 | 视为编辑：改写定义内所有音符 key，引用计数 1 时原地改 |
| 编辑所有实例 | 后续扩展：改定义一次即可（当前不做） |

- 移调不做 `key_offset`，实例结构保持最简；
- CoW 的复制成本受 `NoteBucket` 块级 `Arc` 共享保护（只深拷贝触达的 64K 音符块）；
- 内联实例（引用计数 1）的编辑零复制；
- 轨道结构变化 remap 实例，见「实例放置与轨道」。

## 选区与撤销

### 选区

```rust
pub struct RiffSelection {
    /// 整实例选择（track, instance_id）
    pub instances: HashSet<(u16, u32)>,
    /// 实例内部分选择（instance_id -> 局部 ID 位图）
    pub members: HashMap<u32, NoteBitset>,
}
```

- 全选实例 O(1)，不物化音符位图；
- `NoteBitset`（`crates/yinhe-core/src/note_bitset.rs:16`）改为按局部 ID 使用；
- hidden / ghost 的身份从空间三元组 `(track, start_tick, key)`
  （`crates/yinhe-egui/src/piano_view/drag/types.rs:12`）改为 `NoteRef`。

### 撤销

新增 undo 动作（模式照抄 `UndoAction::AudioClips`，`crates/yinhe-editor-core/src/history.rs:196`）：

```rust
/// 实例列表快照（移动/复制/删除/折叠）。
RiffInstances { track_idx: u16, before: Vec<RiffInstance>, after: Vec<RiffInstance> },

/// 定义交换（编辑即脱离）。旧 Arc 由 undo 栈持有，保证数据存活。
RiffDefSwap { instance_id: u32, before: Arc<RiffDef>, after: Arc<RiffDef> },
```

- 定义级 CoW 让「编辑即脱离」的 undo 只需交换 `Arc` 指针；
- 复合操作（如识别折叠）用 `Composite` 聚合，一次撤销；
- 导入本身不是编辑操作，不产生 undo；导入后的折叠提供「展开全部 Riff」命令还原。

## 持久化

- 容器版本 bump 到 v8（`crates/yinhe-yin/src/lib.rs:56`，不等即拒绝，沿用项目策略）；
- 实例列表放 `mapping.json`（自描述、可容错加字段，参照 `audio_clips`，
  `crates/yinhe-yin/src/mapping.rs:33`）：每轨 `riff_instances: Vec<{ id, def_id, start_tick }>`；
- 定义放 data 段：每个定义一条记录
  `{ id, name, tick_len, note_count, next_local_id, note_streams, automation }`，
  音符流复用现有列式编码 `NoteStreams { delta, key, vel, gate }`
  （`crates/yinhe-yin/src/io.rs:319`），按定义内 256 桶归并；`local_track` 已在定义外，无需编码 track；
- 加载：先读定义表建 `id -> Arc<RiffDef>`，再按实例列表重建并排序；
- 局部 ID 不落盘；`next_local_id` 落盘或加载时重算均可（实施时取更简者）。

## 重复识别

### 原则

- 严格精确匹配：`(相对 start, key, velocity, gate)` 完全一致才视为重复；
- 单轨内检测；
- 无内部阈值：不设「最少重复次数/最短长度」门槛，识别结果按节省音符数由 UI 呈现；
- 纯确定性算法，同一输入结果稳定。

### 指纹

- 网格：十六分音符（`ppq / 4`）为最小单元，单元号 `u = tick / grid`；
- 单轨内，每个非空单元生成指纹 = 该单元内所有音符 `(start_in_unit, key, velocity, gate)` 排序后的哈希；
  音符按 `start_tick` 归属单元，跨单元长音符由 `gate` 编码，归属规则自洽；
- 连续相同指纹（含空单元）做 run-length 压缩后再进入检测，压缩后的序列远短于单元数。

### 检测

```
1. 对压缩后的指纹序列，用哈希表记录每个指纹的出现位置；
2. 相邻出现位置的差值都是候选周期 P；
3. 对每个候选 (起点, P) 用滚动哈希沿序列 O(1) 扩展最长重复 run，
   已确认区间做覆盖标记避免重复扫描；
4. 对每个极大 run 做逐音符精确校验（防哈希碰撞），并验证边界；
5. 最简表示：同一区间取最小周期（能整除则合并），重叠 run 合并/剪裁，
   嵌套重复按「先小周期、后大周期」排序输出。
```

输出：

```rust
pub struct RepeatGroup {
    pub track: u16,
    pub start_tick: u32,
    pub period_ticks: u32, // 一个周期的长度
    pub repeat_count: u32,
}
```

复杂度：预期 O(单元数 + 重复总长度)，最坏情况经 run-length 压缩后仍可控；
指纹计算 O(总音符数) 可按 track 并行。

### 自校验

折叠每组后，把新结构反向展开，与原音符多重集做哈希比对；
不一致则放弃该组折叠（宁可不省，不可改坏数据）。

## 导入流程

```
1. MIDI 解析（现有两遍扫描，crates/yinhe-midi/src/parser.rs:52）得到平铺音符合集
2. 构造初始结构：每轨的全部音符放入一个内联实例（单实例、单引用）
3. 单轨重复识别，得到 RepeatGroup 列表
4. 按 group 切分轨道时间线：
     重复区间        -> 提取一个周期的音符生成共享定义 + repeat_count 个实例
     其余连续区间    -> 各自合并为内联实例
   （定义可进一步按内容去重共享，非必须）
5. 自校验：展开回溯比对，通过后提交
6. 完成报告：识别 N 组重复，节省 X 音符
```

- 解析、识别、折叠三个阶段均有进度回调、可取消；
- 目标：1 亿音符总时长 < 10 秒，超预算时的降级策略（先加载、后台折叠）在实施时定；
- 菜单提供「识别重复」（手动重跑）与「展开全部 Riff」（还原为内联实例）。

## 性能预算

| 操作 | 复杂度 |
|------|--------|
| 复制/粘贴实例 | O(1) |
| 移动/删除实例 | O(实例数) |
| 视口查询 | O(相交实例数 × log + 可见音符数) |
| 编辑即脱离 | O(触达块)，单引用时 O(定义内音符数)（原地） |
| 轨道增删移 remap | O(实例数) |
| 导入解析 + 识别 + 折叠 | 目标 < 10s / 1 亿音符 |
| 内存 | 16B × 去重后音符数 + 2KB/定义 + ~32B/实例 |

## 分阶段实施

### 第 1 阶段：模型与主链路（不可分割）

- `yinhe-types` / `yinhe-core`：`RiffDef` / `RiffInstance` / `LocalNote`、身份、定义统计、
  实例列表、选区、track remap；
- `yinhe-editor-core`：实例编辑命令（复制/粘贴/移动/删除/编辑即脱离）、undo、剪贴板；
- `yinhe-wgpu` + `yinhe-egui`：PR / AR 渲染改为实例展开路径；
- `yinhe-audio`：实例展开；
- `yinhe-yin`：v8 存取；
- `yinhe-midi`：导出展开；
- 自动化一并进入定义（若需缩小首批范围可后置，但会出现两个并存来源，不推荐）；
- 验收：打开 / 显示 / 播放 / 编辑 / 保存 / 加载 / 导出 / undo / 选区全部可用，
  折叠-展开往返哈希一致。

### 第 2 阶段：重复识别与自动折叠

- 识别引擎（纯算法，可先于第 1 阶段并行开发与测试）；
- 导入流程接线：自动折叠 + 自校验 + 报告 + 展开全部；
- 「识别重复」手动入口。

### 第 3 阶段：优化与外围

- GPU cull 按实例化（省显存；当前全量 cull buffer 仍按展开音符占用）；
- 音频调度按实例迭代（省事件表内存）；
- Android 跟随（`crates/yinhe-android/src/pr_view.rs` / `ar_view.rs`）；
- 定义编辑模式（同步所有实例）；
- 识别结果预览 UI。

## 待定决策

1. 自动化是否与音符同批迁移（本 spec 按同批写；后置需接受并存期）。
2. 旧 `.yin`（v7 及以下）是拒绝还是迁移为本模型（迁移成本低、体验好，但与项目「版本不等即拒绝」策略冲突）。
3. 局部 ID 是否落盘（当前按不落盘）。
4. UI 与操作（尚未设计）：
   - Riff 的创建入口（从选区创建？自动命名？）；
   - AR / PR 中 Riff 实例的显示方式（块、边框、颜色）；
   - 「展开全部 Riff」与「识别重复」的菜单位置；
   - 导入报告的形式；
   - 选区对整实例与实例内音符的交互（单击选实例、双击进内部？）。
5. 导入自动折叠是否提供开关（当前按默认开启）。

## 测试策略

- 单元测试：
  - `RiffDef` CoW 语义（共享时复制、单引用时原地、局部 ID 保留）；
  - 实例列表排序与二分相交查询；
  - track remap；
  - 识别算法（构造周期样本、变体、空段、嵌套周期、哈希碰撞校验）；
- 回归测试：
  - 折叠 → 展开往返，音符多重集哈希一致；
  - 保存 → 加载往返，结构一致（局部 ID 不比较）；
  - 编辑即脱离后，其他实例内容不变；
  - undo / redo 在所有新动作上对称；
- 性能测试（遵守 120 秒 rule，用轻量 MIDI 或合成数据）：
  - 千万级合成音符的导入折叠耗时外推；
  - 大量实例（10 万级）的视口查询与统计维护。

## 风险

1. 主链路一次性迁移，PR / AR / 播放 / 导出 / 编辑全部受影响，回归范围大；
2. 大定义的 CoW 与 undo 内存峰值（块级共享缓解，但需实测）；
3. 定义碎片化会放大 2KB/定义的元数据开销（靠合并非重复区间缓解）；
4. 识别误报（自校验兜底，最坏是不折叠）；
5. GPU 显存不省，直到第 3 阶段实例化；
6. Android 端滞后期间两端行为不一致。
