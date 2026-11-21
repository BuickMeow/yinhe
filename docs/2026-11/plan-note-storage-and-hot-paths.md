# 音符数据结构与单音符热路径修复规划

> 日期：2026-11-21
> 范围：只做规划，不改代码。承接上一轮讨论「起点+长度 vs 起点+终点」和「1 亿音符下真正卡手的地方」。
> 关联：`docs/2026-07/optimization-260721.md`（早期 16B 结论与全量驻留问题）、`crates/yinhe-yin/src/lib.rs`（保存/加载峰值）。
> 目标：1 亿音符（极端 1.64 亿）可编辑；任意位置随机编辑的代价只与本块有关，不随整桶音符数退化。
> 约束：不引入缓存、不引入阈值分级、不为了省 1~2 字节破坏 `&Note` 访问模型。

## 0. 结论与第二轮决策

> 2026-11-21 第二轮已定：
> 1. **单 key 没有独立上限**，总量上限来自 id / GPU offset 的 u32；10M/key 只是压力基准，不是产品上限。见附录 D。
> 2. **摘要 8 档必须常驻**，不接受「首次进入某档再准备」。缩放是高频操作，省这部分显存没有意义；只优化构建/上传的瞬时峰值。见 3.3。
> 3. **undo 压缩不保证性能**，必须先 bench 再决定；不预设压缩方案。见 3.6。
> 4. **先做 bench，再开始修**。M0 是硬门禁，bench 结果评审后才进 M1。见第 5 节。
> 5. **不是所有措施都双端有效**：模型/热路径/批量/undo/save 是双端共享；GPU cull 的 8 档摘要和全量重建是桌面端专用；移动端另有 `NoteLayerCache` 常驻桶、`LayerSlot` 高水位等问题。见 3.9。
> 6. **M0 基准已完成**（Apple M5 / release）：10M 单 key 完整铅笔路径约 195~200 ms/帧，`rebuild_dirty` 占 80%~90%；列式 undo delta 7.87B/音符，10M 解码 109ms。见第 5 节。

1. **保留 `start_tick + end_tick`，`Note` 维持 16B。** 换成「起点+长度」内存完全一样，性能差异被内存带宽淹没；现有 `max_ends`、`range_including_long`、GPU `NoteInstance` 都以 end 为中心，改 gate 只增加溢出和 `gate = 0` 两个边界。
2. **P0 是第 4 节的单音符热路径**：按 id 线性扫整桶、每次 `update_note_by_id` 无条件 `sort()`、单点删除后全桶重算 `max_ends`、单点编辑后 `rebuild_dirty()` 重扫整桶。这是 1 亿音符下真正会卡手的地方，优先级高于省 4B/音符。该层在 `yinhe-types` / `yinhe-core`，桌面和移动端同时受益。
3. **P1 是第 3 节的内存副本**，但先测量再改：GPU cull 的 8 档摘要（桌面专用）、全量重建时 raw + summaries 同时驻留（桌面专用）、`insert_batch_sorted` 的 2× 峰值（双端）、undo delta（双端）、移动端 `NoteLayerCache` 常驻桶（移动端更需要）。没有测量数据之前不动结构。
4. **明确不做**：去掉 `id`（12B）、`repr(packed)`（15B）、track 改 u8、u24 tick、块内 delta/varint、SoA 重写、start+gate 重构、摘要按需加载。理由见 3.8 与 3.3。

---

## 1. 音符表示清单

项目里同时存在多种音符表示，不是重复造轮子，而是每一层的自然索引不同：

- **模型层按 key 分桶**，所以 `Note` 不存 key，key 就是桶号；
- **导入层按 track 分组**，所以 `NoteEvent` 不存 track，track 就是所属 `TrackData`；
- **GPU 是扁平 buffer**，所以 `NoteInstance` 必须把 key 和 track 都打包进实例；
- **磁盘是列式流**，所以 key/track 分开存，终点存 gate；
- **剪贴板/Domino 是跨进程格式**，所以用最小公共字段，通常没有 id。

### 1.1 全景

```text
MIDI 文件
  │  midly 事件流
  ▼
NoteEvent（per-track，16B：id + start + end + key + vel，track 隐式）
  │  YinModel::load_track_notes / NoteLoader
  ▼
Note（模型唯一权威存储，256 个 key 桶，16B：id + start + end + vel + track，key 隐式）
  ├── build_key_notes_in_range ──► NoteInstance（12B，key+track+vel 打包）──► GPU 原始层
  ├── build_key_summary ────────► NoteSummary（16B/段）──► NoteInstance ──► GPU LOD 层
  ├── MIDI writer ──────────────► NoteOn + NoteOff（start/end 两端）
  ├── Domino 剪贴板 ────────────► DominoNote（start + gate + key + vel，track 隐式）
  ├── 跨实例剪贴板文件 ─────────► 12B 条目（start + end + vel + key + track，无 id）
  ├── NoteDelta（undo）─────────► Note 全量副本 + key
  └── .yin data 段 ─────────────► 5 列 SoA：start delta / key / vel / gate / id delta
```

### 1.2 表示对照表

| 表示 | 位置 | 字段 | 大小 | key | track | 终点语义 | 生命周期 / 用途 |
|---|---|---|---|---|---|---|---|
| `yinhe_types::Note` | `crates/yinhe-types/src/note.rs` | id, start, end, vel, track | 16B | 隐式（桶号） | 显式 u16 | end | 运行时唯一权威存储 |
| `NoteBucket` | `crates/yinhe-types/src/note_bucket.rs` | `Arc<Vec<Note>>` × 块 + starts + max_ends | 16B/音符 + 元数据 | 隐式（外层数组下标） | 在 Note 内 | end | 每 key 一个，65536 音符/块，块间按 start 有序 |
| `yinhe_core::NoteEvent` | `crates/yinhe-core/src/events.rs` | id, start, end, key, vel | 16B | 显式 u8 | 隐式（所属 `TrackData.notes`） | end | 仅导入期存在，运行时 `TrackData.notes` 为空 |
| `NoteLoader` | `crates/yinhe-core/src/model_stats.rs` | `[Vec<Note>; 256]` + 统计 | 16B/音符 | 显式（数组下标） | 在 Note 内 | end | 加载/导入暂存，`finish` 时逐桶排序分块并释放 |
| `.yin` data 段 | `crates/yinhe-yin/src/data_section.rs` | 5 个列流 | varint/zstd | key 列 | 轨段表 | **gate** | 持久化，16MB 分帧 zstd |
| `yinhe_wgpu::NoteInstance` | `crates/yinhe-wgpu/src/vertex.rs` | start, end, packed | 12B | 显式 u9（打包） | 显式 u16（打包） | end | GPU 原始层 / LOD 层 / AR 层 / 可见窗口缓存 |
| `NoteSummary` | `crates/yinhe-types/src/note_summary.rs` | block, start, end, key, track, vel | 16B/段 | 显式 | 显式 | end | CPU 侧 LOD 摘要，每 (key, block, track) 一段 |
| `DominoNote` | `crates/yinhe-editor-core/src/domino_clipboard.rs` | start, gate, key, vel | 12B | 显式 | 隐式（`Vec<Vec<_>>` 下标） | **gate** | Domino 剪贴板，无 id |
| 跨实例剪贴板条目 | `crates/yinhe-editor-core/src/clipboard_file.rs` | start, end, vel, key, track | 12B（定长） | 显式 | 显式 | end | 系统剪贴板引用文件，无 id |
| `VelocityBlock` | `crates/yinhe-wgpu/src/automation/velocity_summary.rs` | start, end, vel, track | 12B | 无（AM 力度条不区音高，每块只留代表音符） | 显式 | end | AM 力度条摘要（CPU） |
| `VelocityBarInstance` | `crates/yinhe-wgpu/src/vertex.rs` | tick, length, packed, reserved | 16B | 无 | 显式（打包） | **length** | AM 力度条 GPU 实例 |
| `yinhe_midi::ActiveNote` | `crates/yinhe-midi/src/parser.rs` | key, velocity, start, global_channel | 8B | 显式 | 无（用 global_channel） | 无（等 NoteOff） | 解析期 NoteOn/NoteOff 配对栈 |
| `yinhe_audio::ActiveNote` | `crates/yinhe-audio/src/audio_model/event.rs` | key, dense, clap_channel, is_instrument, end, track | 16B | 显式 | 显式 | end | 播放中发声音符，NoteOff 调度堆元素 |
| `NoteDelta` | `crates/yinhe-editor-core/src/history.rs` | before/after: `Vec<(Note, u8)>` | 20B/音符/侧（16B Note + key + pad） | 在元组内 | 在 Note 内 | end | undo/redo 的副本制路径 |
| `NoteBitset` | `crates/yinhe-types/src/note_bitset.rs` | id 位图 | 1 bit/id + 页 | 无 | 无 | 无 | 选区成员 / 锚点成员 |
| UI / 渲染临时结构 | egui / mobile / wgpu | 视场景而定 | 8~16B | 视场景 | 视场景 | end | `NoteRef`、`CollectedNote`、`HitNote`、`NotePreview`、`PendingRecordingNote`、事件列表行、AR 合并元组 `(start, end, vel)`、`NoteLayerCache` 的可见窗口 |

### 1.3 为什么 key / track 有时存有时不存

- **`Note` 不存 key**：`YinModel.notes` 是 `Box<[Arc<NoteBucket>; KEY_COUNT]>`，桶号就是 key。存一份 key 会多 1B，而且任何跨 key 移动都要同时改桶和字段。
- **`NoteEvent` 不存 track**：导入时每个 `TrackData` 持有一个 `Vec<NoteEvent>`，track 由外层决定。这是导入期临时结构，省 2B/音符。
- **`NoteInstance` 同时存 key 和 track**：GPU 的 `all_notes` 是 per-key 连续 buffer，但顶点着色器要知道 key 算 y、track 算颜色/显隐；`packed` 一个 u32 装下 key(9) + track(16) + vel(7)。这是整个仓库里最紧凑的运行时表示。
- **GPU 的 tick 实际是 31 位**：`NoteInstance` 把 `start_tick` / `end_tick` 的 bit31 分别借给选中标记和 ghost 标记（`SELECTED_BIT` / `GHOST_BIT`，`TICK_MASK = 0x7FFF_FFFF`）。模型层是 u32，但渲染层超过 2^31 tick 会与标记位冲突；480 PPQ、120 BPM 下 2^31 tick 仍有约 26 天，实际不是瓶颈，但做超长曲目时要记住这条边界。
- **`DominoNote` 不存 track**：Domino 剪贴板是 `tracks: Vec<Vec<DominoNote>>`，track 是外层下标；格式本身也没有 id。
- **`NoteSummary` 同时存 key 和 track**：摘要段按 `(key, block, track)` 聚合，key 不再有桶可隐含，必须显式存。
- **音频线程不存音符副本**：播放 dispatch 用 `NoteBucket::peek/advance` 的 `NoteCursor` 顺序消费模型；这也是块用 `Arc<Vec<Note>>` 的原因——编辑只复制触达块，音频线程继续读旧快照。

结论：这些不是同义重复，而是「谁拥有索引，谁就不存字段」。真正需要统一的只有热路径的寻址方式，不是字段布局。

### 1.4 16B 的构成与规模

```text
id        4
start     4
end       4
velocity  1
padding   1   ← track 需要 2 字节对齐
track     2
           = 16B
```

- 1 亿音符 = 1.6GB；1.64 亿 = 2.62GB。
- 16B 的记录密度是 4 个/64B cache line；Vec 分配通常 16B 对齐，顺序扫描不会额外跨线。
- `track` 不能改 u8：`MAX_TRACKS = 65536`（`crates/yinhe-wgpu/src/vertex.rs`），u16 的完整定义域都在用。即便改 u8，4+4+4+1+1=14 对齐后仍是 16B，不省。
- 唯一能干净省下的是去掉 `id`（-4B，25%），但 id 是选区 / `HiddenNotes` 的 `NoteBitset` 索引、undo 的身份、跨轨移动的依据；黑乐谱同 key 同 tick 有大量叠音，`(track, start, key)` 不能唯一标识。这是功能换内存，不在本次范围。
- 总量硬顶是 u32::MAX，单 key 没有独立的更低限制；详见附录 D。

---

## 2. 起点+长度 vs 起点+终点：结论是不改

| | `start + end` | `start + gate` |
|---|---|---|
| 内存 | 2 × u32 = 8B | 完全相同 |
| 结构体 | 15B → 对齐 16B | 完全相同 |
| 读取终点 | 直接读 | `start + gate`，一条加法 |
| 移动音符 | 改 start + end 两个字段 | 只改 start |
| 不变量 | `end >= start` 天然成立 | 要防 `gate = 0` 和 `start + gate` 溢出 u32 |
| 排序 / 二分 | 相同 | 相同 |
| 与现有裁剪的契合 | `max_ends` 直接用 end | 每次重算 max_end 都要加一次 |

性能上：

- 顺序扫描是 16B/音符的内存带宽瓶颈，多一条加法完全被掩盖；
- 你的裁剪路径 `range_including_long`、块级 `max_ends`、GPU `NoteInstance` 都已经以 end 为中心；
- 磁盘和 Domino 已经用 gate，说明「需要 gate 的地方」已经各自用 gate，不需要把模型层也改成 gate。

所以：**模型层保留 end；磁盘、Domino、力度条 GPU 实例继续用 gate；MIDI 导出本来就是 NoteOn/NoteOff 两端事件。** 这是当前代码已经做对的分层，不建议为了「统一语义」做全仓重构。

---

## 3. 内存副本治理（上一轮第 3 点）

### 3.1 账本

以 1.64 亿音符（start.mid）为例，数量级如下。没有标注「实测」的都是按结构体大小推算，实施前用 `yinhe_memtrace::Snapshot` 复核。这张表包含双端共享项和桌面 GPU cull 专用项，具体平台适用性见 3.9。

| 项目 | 表示 | 单位成本 | 1 亿 | 1.64 亿 | 性质 | 处理 |
|---|---|---|---|---|---|---|
| CPU 模型 | `Note` | 16B/音符 | 1.60GB | 2.62GB | 常驻 | 不动 |
| GPU 原始层 all_notes | `NoteInstance` | 12B/音符 | 1.20GB | 1.97GB | 常驻 | 不动 |
| GPU 原始层 visible 索引 | `u32` 索引 | 4B/槽，按 256 向上取整 | 0.40GB | 0.66GB | 常驻 | 不动 |
| GPU LOD 摘要层 | `NoteInstance` + 索引 | 12B/段 + 4B/槽 | 待测 | 待测 | 常驻 | S3-2 |
| 全量重建的 raw 缓冲 | `Vec<NoteInstance>` | 12B/音符 | 1.20GB | 1.97GB | 瞬时 | S3-3 |
| 全量重建的摘要缓冲 | 8 档 `Vec<NoteInstance>` | 12B × Σ段 | 待测 | 待测 | 瞬时 | S3-3 |
| UI 可见窗口 | per-key + flat + 窗口外桶 | 12B × 窗口段数 × 2 + 窗口外残留 | 随窗口 | 随窗口 | 常驻 | 3.7 / 3.9 |
| `insert_batch_sorted` | 旧块 + 新块 | ≈ 2× 桶 | 按桶 | 按桶 | 瞬时 | S3-4 |
| `iter_mut` / PPQ 缩放 | 所有块深拷贝 | ≈ 2× 模型 | 3.2GB | 5.24GB | 瞬时 | 3.7 |
| 单点 start 修改触发 `sort()` | 旧块 + flat + 新块 | ≈ 3× 桶 | 按桶 | 按桶 | 瞬时 | 由 P0 消除 |
| undo `NoteDelta` | `Vec<(Note, u8)>` | 20B/音符/侧 | 2.00GB（单侧） | 3.28GB（单侧） | 常驻历史 | S3-5 |
| 保存峰值 | — | — | — | 3.9GB（已有注释） | 瞬时 | 不动 |
| 加载峰值 | — | — | — | 2.7GB（已有注释） | 瞬时 | 不动 |

先修 P0 的另一个理由：单点 `ResizeLeft` 一旦让桶失序，`sort()` 会走「flat 全量拷贝 + 排序 + 重建块」的路径，瞬时内存约 3× 桶，10M 音符的桶就是 ~480MB，而且每个拖拽帧都来一次。P0 修好后这条峰值自然消失。

### 3.2 S3-1：过期注释与尺寸断言（P1，约 0.5 天）

这些注释还停留在 `NoteInstance` 16B、visible 层存完整实例的年代，容易误导后续优化：

- `crates/yinhe-wgpu/src/cull.rs` 顶部：`all_notes + visible_notes ≈ 2 × total notes × 16B`，100M ≈ 3.2GB。实际 visible 层已经改成 4B 索引，原始层合计约 16B/音符（12B + 4B），不是 32B。
- `crates/yinhe-wgpu/src/render_thread.rs`：`Pre-built note layer data (16B NoteInstance)`，实际 12B。
- `crates/yinhe-wgpu/src/arrangement/instances.rs`：`Output is 16B NoteInstance`，实际 12B。
- `crates/yinhe-wgpu/src/resource.rs` 的 `GpuBudget` 注释：`100M 音符全量 all_notes + visible_notes ≈ 3.2GB`，实际约 1.6GB（不含摘要）。
- `crates/yinhe-wgpu/src/pianoroll/instances.rs` 的 `build_all_notes` 注释：`峰值只有最终缓冲一份（约 2.6GB）`，按 12B 算应是约 1.97GB（按 16B 才是 2.6GB）。

同时补尺寸断言（部分已存在）：

```rust
const _: () = assert!(std::mem::size_of::<Note>() == 16);
const _: () = assert!(std::mem::size_of::<NoteEvent>() == 16);
const _: () = assert!(std::mem::size_of::<NoteSummary>() == 16);
const _: () = assert!(std::mem::size_of::<NoteInstance>() == 12);
```

验证方式：手工加载 start.mid / Broken World，用 `yinhe_memtrace::Snapshot::capture()` 在「加载完成、首帧上传完成、全量重建完成」三个点记录 `total_mb()` 和 `gpu_mb()`，把实测值写回本文档。没有数据之前不改结构。

### 3.3 S3-2：GPU 摘要 8 档全量常驻（P1，桌面 GPU cull 专用）

现状：

- `SUMMARY_BLOCK_TICKS = [262144, 65536, 16384, 4096, 1024, 256, 64, 16]`；
- `build_summaries` 一次构建 8 档，`upload_summary_all` 把 8 档全部上传；
- `CullState.summary_levels` 常驻 8 档，每档 12B/段 + 4B/槽 + draw args；
- 只有一档会被当前 ppu 选中，其余 7 档在显存里等待。

**硬约束（已定）：8 档全部常驻，不做「首次进入某档再准备」。** 缩放是高频操作，切档卡一帧不可接受；因此不做按需上传、不做逐档淘汰，也不在缩放时后台构建。下面只优化「构建/上传过程」和「常驻结构本身」，不动常驻策略。

先做低风险开关，再测量：

1. **先修一个已确认的浪费（低风险，桌面端）**：桌面 LOD 开关关闭时，`upload_all_with_summaries` / `start_rebuild` / `upload_key_with_summaries` 仍然会构建并上传 8 档摘要，`lod_enabled` 只影响渲染时选不选摘要层。把该开关传进这三条路径，关闭 LOD 时直接跳过摘要构建/上传。这是纯省内存，不改变默认行为。移动端固定 `lod_enabled: true`，不受这条影响。
2. 在 `upload_summary_all` 里按档打印段数、字节数（`tracing::info!` 或 memtrace 统计）。
3. 用 start.mid（1.64 亿）和 Broken World（4444 万）各跑一次，记录 8 档合计。
4. 如果合计占原始层比例很小，关闭此项，不值得为它增加复杂度。
5. 如果合计很大，只允许在「仍然全量常驻」的前提下做：
   - **A. 逐档构建/上传，上传后立即释放 CPU `Vec`**：GPU 仍然 8 档全在，只是构建过程串行，削掉「8 档 CPU 缓冲同时存在」的瞬时峰值。这条与常驻要求不冲突。
   - **B. 降低每档段数**：摘要按 `(key, block, track)` 聚合，段数与「块数 × 有音符的轨数」成正比。可以减少块数档位、或在同一 block 内合并更多轨——这是视觉决策，必须先看效果。
   - **C. 降低档位数量**：也是视觉决策，必须先看「4 ticks/px 附近没有 16 档会怎样」，不擅自删档。
   - **D. 压缩摘要实例本身**：`NoteInstance` 已经是 12B（start/end/packed），没有 id；再压只能减少字段或位宽，收益小、风险大，不建议。

边界：摘要层不带选中位、不做选中轨道排序；切档失败必须能回退原始层，不能黑屏。验证：在 start.mid 上连续缩放 8 个档位，记录每档首次进入的帧耗时（应无额外准备）和显存。

### 3.4 S3-3：全量重建时 raw + summaries 同时驻留（P1，桌面 GPU cull 专用）

现状（仅桌面 `use_gpu_cull = true` 路径；移动端 Flutter 不走 `build_all_notes` / `CullRebuild`）：

- `CullRebuild::Building` 在后台线程 `build_all_notes` 产出全量 `Vec<NoteInstance>`（约 1.97GB @ 1.64 亿）；
- `BuildResult` 同时持有 `summaries: Vec<(Vec<NoteInstance>, offsets)>`，8 档全在；
- UI 线程按每帧 4 个 key 上传，上传完成前整份载荷都活着。

`build_all_notes` 已经做过一次优化（去掉 per-key 中间 Vec，峰值从约 5GB 降到一份最终缓冲），不要回退。剩余峰值来自「raw + 8 档摘要同时存在」。

分两步，**都保持 8 档全量常驻**：

1. **先做逐档（低风险）**：`BuildResult` 只保留 raw + offsets；摘要改为上传状态机里逐档构建、逐档上传、每档上传后释放 CPU `Vec`。GPU 上仍然是 8 档全量常驻，只是构建/上传串行，不在缩放时按需准备。
2. **再评估逐 key 流式（高成本）**：raw 也按 key 构建/上传，后台线程一次只持有一个 key 的实例（最坏单 key 10M = 120MB），上传完即释放。同样不改变 GPU 常驻内容。需要处理「构建期间模型又改了」的失效逻辑（现有 revision 检查可复用），以及 UI 分帧节奏。

验收：全量重建期间 RSS 峰值下降；UI 不卡顿；track 显隐切换期间旧 buffer + track_mask 双重过滤仍然正确；缩放切档不出现额外等待。

### 3.5 S3-4：`insert_batch_sorted` 的 2× 峰值（P1，约 1 天）

现状（`NoteBucket::insert_batch_sorted`）：

- 旧 `self.chunks` 一直活着；
- 新 `result` 逐块 `Arc::new(part.to_vec())` 分配；
- 最后才 `self.chunks = result`，所以峰值 ≈ 旧桶 + 新桶 ≈ 2×。

改法（不引入阈值，始终流式归并）：

1. `let old_chunks = std::mem::take(&mut self.chunks);`
2. 逐块消费 `old_chunks`，归并当前块与对应的新音符切片，产出新块后**立即 drop 旧块**；
3. 新块写进 `result`，旧块随迭代释放；refcount=1 时峰值从 2× 降到约 1× 桶 + 单块临时，而不是整桶两份；
4. `starts` / `max_ends` 同步重建。

注意：`Arc<Vec<Note>>` 如果还被音频线程持有，旧块不会真的释放，这时峰值仍是 2×；这是 COW 的固有代价，文档里写清楚，不做额外优化。

验证：新增 bench，在 1M 音符/key 的桶里批量插入 10 万音符，用 memtrace 记录峰值；对比改前/改后。功能测试复用现有 `insert_batch_merges_across_chunks`。

### 3.6 S3-5：undo `NoteDelta` 的整份副本（P2，bench 决定是否升 P1）

现状：删除/复制/回退路径把 `(Note, key)` 全量拷进 `before`/`after`。`(Note, u8)` 是 20B/音符，删除全选 1 亿音符 = 2.0GB 常驻在 undo 栈里。

**不能保证压缩后性能跟得上。** 压缩是有 CPU 代价的，尤其是 undo 要把上亿音符解码回 `Vec<Note>`；必须先用 bench 拿到真实数字，再决定是否改。

#### 3.6.1 先做 bench，不先实现

数据规模：1M / 10M / 100M 音符，单 key 密集分布；桌面和移动端各测一次（移动端 CPU 更慢）。

对比三组：

| 方案 | 内存 | 编码 | 解码 |
|---|---|---|---|
| 现状 `Vec<(Note, u8)>` | 20B/音符 | 只是拷贝，无编码 | 无解码，直接读 |
| 列式 delta（不复 zstd） | 约 6~10B/音符 | start delta varint + gate varint + key/vel/track/id delta | 重建 `Vec<Note>` |
| 列式 delta + zstd | 约 3~6B/音符 | 编码 + zstd | zstd + 重建 |

记录：编码耗时、解码耗时、峰值内存、最终字节数。特别注意 100M 音符的解码时间——如果 undo 一次要 2~3 秒，即便省 1.5GB，交互上也可能不可接受；如果 300ms 内，则很划算。

#### 3.6.2 如果要改，优先「始终列式」而不是「大才压缩」

「超过 N 个音符才压缩」会引入阈值，而且两套表示会长期并存。更干净的做法是：

- `NoteDelta` 内部始终用列式 delta（小编辑的编码/解码是微秒级，开销可忽略）；
- 提供 `iter()` 解码迭代器，避免一次性物化整个 `Vec<Note>`；
- 撤销/重做按 key 流式恢复，不要求一次分配上亿音符的 `Vec`。

这样没有大小分支，也没有「小 delta 用旧路径、大 delta 用新路径」的双实现。代价是 `NoteDelta` API 变化，所有生成端/消费端都要迁移；这必须在 bench 证明值得后再做。

#### 3.6.3 其他选项

1. **历史预算**：按字节数淘汰最老的 undo entry（产品决策，需要你拍板）。
2. **不动**：如果 bench 显示 100M 全选删除极少发生，且 2.0GB 只在极端操作后短暂存在，可以把优先级排在 P0 和移动端 `NoteLayerCache` 之后。

#### 3.6.4 M0 实测（2026-11-21，Apple M5 / release）

| 方案 | 1M B/音符 | 10M B/音符 | 1M 编码 / 解码 | 10M 编码 / 解码 |
|---|---|---|---|---|
| `Vec<(Note,u8)>` 拷贝 | 20.00 | 20.00 | 5.2 ms / — | 53.3 ms / — |
| 列式 delta | 7.87 | 7.87 | 11.8 / 12.3 ms | 86.6 / 109.3 ms |
| 列式 + zstd3 | 4.37 | 3.99 | 154.7 / 28.0 ms | 832.2 / 179.0 ms |

结论：

- 列式 delta 省 61% 内存，10M 编解码约 0.1 s；外推 100M 约 0.9 s 编码 / 1.1 s 解码，对一次性 undo 可能可接受。
- zstd3 再省到 3.99 B/音符，但 10M 编码 832 ms，外推 100M 约 8 s；**编辑提交时不可接受**。如果一定要 zstd，只能放到后台或按需压缩，那会引入“什么时候压”的阈值/调度复杂度，不建议。
- 所以如果做压缩，方向是「列式 delta，不叠 zstd」。100M 的 1 秒解码是否可接受，还需要你拍板；如果不可接受，就保持现状或只做历史预算。

本次建议：**列式 delta 可以进入候选，但先补 100M 编解码 bench，再决定是否实现。** 不实现 zstd3 路径。如果移动端 4~6GB 设备实测会在 undo 时 OOM，再把它提到 P1。

### 3.7 S3-6：接受 / 不修 / 移动端专项

双端共有、暂时接受：

- **`iter_mut` + PPQ 缩放 / 轨号 remap**：所有音符都变，且音频线程持有旧快照时必须 COW，2× 峰值不可避免。PPQ 缩放是低频操作，接受。
- **`sort()` 的 3× 峰值**：单点路径由 P0 消除；批量操作（如导入后全量排序）仍可能触发，但导入路径已经走 `NoteLoader::finish` 的逐桶排序，峰值受控。
- **保存 3.9GB / 加载 2.7GB**：已有专文记录，不在本次范围；移动端 4~6GB 设备上这是比音符结构更现实的风险，见 3.9。
- **`NoteBucket::len()` 的 O(块数) 求和**：模型层已有 `bucket_note_count` 缓存，热路径不直接调；不改。

移动端 / CPU 渲染路径需要重点看的两个高水位问题：

- **`NoteLayerCache` 的窗口外 per-key 桶不会释放**（`crates/yinhe-wgpu/src/note_cache.rs`）：`buckets: [Option<KeyBucket>; KEY_COUNT]` 只在当前 `key_lo..=key_hi` 范围内更新，窗口外的 key 保留上一次的 `Vec<NoteInstance>`。垂直滚动浏览所有 key 后，最多 256 个桶都会留着各自的窗口实例；换文档才整体重置。这是移动端（和桌面 CPU fallback）最可能累积的 UI 内存，不是「2× 当前窗口」那么小。
  - 先测量：在 start.mid 上高倍率垂直滚完 256 个 key，记录 `NoteLayerCache` 占用的 RSS。
  - 如果确实大，再做「只保留当前 key 窗口 + 少量预取」或「窗口外桶按 LRU/容量释放」。这是渲染缓存策略，需要你拍板是否接受重新进入某个 key 时重建。
- **`LayerSlot` 的单个 buffer chunk / CPU scratch 只涨不缩**（`crates/yinhe-wgpu/src/layer.rs`）：`BufferChunk::ensure_capacity` 扩容后不回收，`scratch.clear()` 保留容量（超过 `MAX_PER_CHUNK` 的额外 chunk 会回收，但单块容量不会）。一次大窗口（或一次全曲 zoom-out）后，这一层会一直占着峰值容量，直到 renderer 重建。移动端 4~6GB 值得加一个「长期小于容量 1/4 时缩容」或「失焦/切文档时释放」的策略；先测量峰值，再决定是否做。

这两项都不在音符模型层，但对移动端内存的实际影响可能比 GPU 摘要 8 档更大。

### 3.8 明确不做

| 方案 | 能省 | 不做的原因 |
|---|---|---|
| 去掉 `id` → 12B | 25%（1 亿省 400MB） | 选区 / `HiddenNotes` 的 `NoteBitset`、undo 身份、同 key 同 tick 叠音消歧都靠 id；功能换内存 |
| `repr(packed)` → 15B | 1B/音符 | 非对齐 u32 访问、引用字段麻烦，顺序扫描跨 cache line，反而更慢 |
| `track` 改 u8 | 0 | `MAX_TRACKS = 65536`；对齐后仍 16B |
| u24 tick | 2B/音符 | 高 PPQ 黑乐谱会溢出；拆字节访问更慢；GPU 还用 bit31 做标记 |
| 块内 base + delta/varint | 4~8B/音符 | 随机编辑要重编码整个块，且需要「短 gate / 长 gate」的 escape 阈值，违背约束 |
| SoA（start/end 单独数组） | 峰值不变 | 只对「只读 tick 的过滤」有 cache 收益；本项目裁剪已搬到 GPU，CPU 侧收益小，`&Note` API 全面重写 |
| start+gate 重构 | 0 | 见第 2 节 |

### 3.9 桌面端 / 移动端适用性（重要）

**结论：不是所有措施都双端有效。** 共享 crate 里的改动双端都吃到；桌面 GPU cull 专用的改动移动端根本不会执行。

| 措施 | 所在层 | 桌面 | 移动 | 说明 |
|---|---|---|---|---|
| P0 单音符 `locate_at` / `update_at` / `remove_at` | `yinhe-types` / `yinhe-core` | 有效 | 有效 | 移动端 `Document` 调同一套写原语 |
| `delete_notes` / `move_clip` / `loop_clip_*` 批量化 | `yinhe-editor-core` | 有效 | 有效 | 移动端场景事件也走这些 Document API |
| `insert_batch_sorted` 流式归并 | `yinhe-types` | 有效 | 有效 | 双端共用 NoteBucket |
| undo `NoteDelta` | `yinhe-editor-core` | 有效 | **更需要** | 20B/音符，4~6GB 设备上 OOM 风险更高 |
| `Note` 16B / 不去 id | `yinhe-types` | 有效 | **更需要** | 模型是双端最大常驻项 |
| 保存 / 加载峰值 | `yinhe-yin` | 有效 | **更需要** | 移动端 4~6GB 上保存峰值可能比模型本身更危险 |
| `iter_mut` COW（PPQ 缩放 / 轨号 remap） | `yinhe-core` | 有效 | 有效 | 低频操作，2× 峰值双端都存在 |
| 音频游标不复制音符 | `yinhe-audio` | 已生效 | 已生效 | 不是待修项，这里只是说明没有隐藏副本 |
| LOD 关闭时跳过摘要构建/上传 | `yinhe-egui` | 有效 | 无效 | 移动端固定 `lod_enabled: true` |
| GPU 8 档摘要常驻 | `yinhe-wgpu` GPU cull | 有效 | 无效 | 移动端走 `NoteLayerCache` CPU 裁剪，不建 8 档 GPU 摘要 |
| 全量重建 raw + summaries | `yinhe-egui` GPU cull | 有效 | 无效 | 移动端不调 `build_all_notes` / `CullRebuild` |
| `NoteLayerCache` 窗口外桶不释放 | `yinhe-wgpu` | 有效 | **更需要** | 移动端 PR 主路径；桌面 CPU fallback 也走 |
| `LayerSlot` 高水位不缩容 | `yinhe-wgpu` | 有效 | **更需要** | 移动端 GPU/CPU 内存共享 |

移动端 4~6GB 的粗略判断：

- 1 亿音符模型 1.6GB，加上系统、音频、GPU 缓冲和 undo，6GB 设备可以跑，4GB 设备已经很紧。
- 1.64 亿音符模型 2.62GB，4GB 设备基本不可行，6GB 也要看音频/贴图占用。
- 所以移动端真正值得先做的是：P0（CPU 不卡，间接省内存峰值）、undo 压缩（bench 后）、保存峰值、`NoteLayerCache` 桶释放、`LayerSlot` 缩容；GPU 摘要 8 档那套是桌面专项，移动端不用投入。

---

## 4. 单音符热路径（上一轮第 4 点）

### 4.1 现状与复杂度

| 调用点 | 当前路径 | 复杂度 | 严重度 |
|---|---|---|---|
| 铅笔左拉伸 `note_pencil.rs` `ResizeLeft` | `update_note_by_id` → 按 id 扫桶 → 改 start → `sort()` | O(N) 查找 + O(N) `is_sorted`；失序时 O(N log N) + 约 3× 桶内存 | P0 |
| 铅笔右拉伸 / 力度 | 同上，但 start 不变 | O(N) 查找 + O(N) `is_sorted` | P0 |
| 铅笔移动 `Move` | `remove_by_id` → 按 id 扫桶 → `rebuild_max_ends` 扫全桶 | O(2N) | P0 |
| `delete_single_note` | `range` 定位 O(log) + `remove_by_id` | O(2N) | P0 |
| `delete_notes(k)` | 每个音符一次 `remove_by_id` | O(k·N) | P1 |
| `set_note_end_tick` | `note_by_id` + `update_note_by_id` + `sort()` | 3 × O(N) | P1（MIDI 录音每 NoteOff 一次） |
| `move_clip(k)` | 每个音符 `remove_by_id` + `insert_note` | O(k·N) | P1 |
| 单音符编辑后的 `rebuild_dirty()` | 扫描脏桶重建 per-track stats | O(N_key) | P0 |
| 批量选区 `update_selected_in_place` / `remove_by_ids` | 逐桶一次扫描 | O(N_key) | 可接受 |
| 历史 apply 的 `remove_by_ids` / `update_matching` | 逐桶一次扫描 | O(N_key) | 可接受 |

关键细节：

- `NoteBucket::update_by_id` 每次调用后，`YinModel::update_note_by_id` 都会 `bucket.sort()`（`crates/yinhe-core/src/note_write.rs`）。
- `sort()` 先 `is_sorted()` 全桶扫描；**一旦单点改 start 造成失序**，就走 `flat: Vec<Note> = 全桶拷贝` → `sort_by_key` → `from_sorted` 重建块。10M 的桶 = 160MB 拷贝 + 160MB 排序临时 + 160MB 新块，约 480MB/帧。
- `remove_by_id` 先 `chunks.iter().position(|c| c.iter().any(...))` 全桶找 id，删除后 `rebuild_max_ends()` 又扫全部音符。
- 这些路径的调用方通常刚刚用 `range(start, start+1).find(...)` 定位过同一个音符，等于同一个音符找了两遍。
- 单音符编辑后调用方还会 `model.rebuild_dirty()`：它按脏 key 重扫整个桶重建 `bucket_track_stats`（`crates/yinhe-core/src/model_stats.rs` 的注释也写明 O(dirty bucket size)）。所以铅笔拖拽一帧实际是「id 扫桶 + is_sorted / 全排序 + stats 扫桶」2~3 次全桶遍历，10M 桶就是每帧 320~480MB 内存流量，还没算 full sort。

### 4.2 设计原则

1. **id 是身份，不是热路径索引。** 保留 id 用于选区、undo、跨轨移动；但单音符编辑用 `(start_tick, track)` 定位，id 只在同 `(start, track)` 叠音时消歧。
2. **不新增持久 id → 位置索引。** 任何 insert/remove/sort 都会让位置失效；维护成本高、每音符多 4~8B，还不如省下来。
3. **不做全局 sort。** 单点 start 变化只搬移该音符，并刷新受影响块的 `starts` / `max_ends`。
4. **API 语义分开。** `*_at` 给热路径（要求坐标是最新的），`*_by_id` 只留给冷路径/测试。迁移完成后，如果 `*_by_id` 单点 API 没有调用方，按项目规则删除，不留旧 fallback。
5. **一次操作只 `mark_dirty` 一次。** 批量路径不逐音符 `mark_dirty`。
6. **单音符编辑不走 `rebuild_dirty()`。** 统计更新要 O(1)：用旧/新 `Note` 直接调 `bucket_track_stats` / `track_note_count` / `track_audible_count` / `note_count` 的增量，`tick_length` 用块级 `max_end_tick()`（O(块数)）；不要为改一个音符重扫整桶。

### 4.3 `NoteBucket::locate_at`：按 `(start, track)` 定位

```rust
/// 定位一个音符。id 仅在同 (start, track) 有多个音符时消歧。
/// 返回 (chunk_idx, note_idx)。
fn locate_at(&self, start: u32, track: u16, id: Option<u32>) -> Option<(usize, usize)>
```

算法：

1. `ci = chunk_index_for(start)`（最后一个 `starts <= start` 的块）；
2. **向前回退**：`while ci > 0 && chunks[ci-1].last().start_tick >= start { ci -= 1; }`；
3. 从 `ci` 向后扫描：块首 `start_tick > start` 时停止；否则在块内 `partition_point(< start)` 到 `partition_point(<= start)` 之间找 `track`（以及 id）；
4. 返回第一个命中。

必须处理一个容易漏的边界：**块切分可能把同一 start 的音符分到两个块**。例如块 0 以 start=100 结尾，块 1 以 start=100 开头；`chunk_index_for(100)` 返回块 1，但目标音符可能在块 0。上面的「向前回退」就是为此。极端黑乐谱（10M 音符全在 tick 0）会退化成扫描所有同 tick 块，但那只与同 tick 音符数有关，与整桶音符数无关。

复杂度：O(log 块数 + 同 start 块数 + 同 tick run 长度)。典型情况是 O(log N + 1)。

### 4.4 `update_at` / `remove_at` / 元数据刷新

```rust
/// 修改一个音符。f 可以改 end/velocity/track，也可以改 start；
/// 改 start 时只搬移该音符，不全局 sort。
fn update_at(
    &mut self,
    start: u32,
    track: u16,
    id: u32,
    f: impl FnOnce(&mut Note),
) -> Option<Note>;

/// 删除一个音符，只重算受影响块。
fn remove_at(&mut self, start: u32, track: u16, id: u32) -> Option<Note>;
```

`remove_at` 的元数据规则：

- 删除后块为空：移除该块，同时 `starts` / `max_ends` 对应项移除；
- 删除的是块首（`idx == 0`）：`starts[ci] = chunk[0].start_tick`；
- 删除的音符 `end_tick == max_ends[ci]`：重算该块的 `max_ends`（最多 65536 个音符 = 1MB 扫描，约几十微秒）；
- 否则 `max_ends[ci]` 保持不变（它仍然是一个上界，且是精确的，因为被删的不是最大者）。

`update_at` 的规则：

- 只改 end：
  - `new_end > max_ends[ci]` → 直接抬高；
  - `old_end == max_ends[ci] && new_end < old_end` → 重算该块；
  - 其他情况不动（max 仍是精确上界）。
- 只改 velocity / track：不动元数据（`track` 不是排序键）。
- 改 start：
  1. 保存 `old_start`；
  2. `f` 之后如果 `new_start != old_start`，从原块移除该音符；
  3. 原块为空则删块，否则刷新该块元数据；
  4. 调用现有 `insert_sorted(note)` 放回（可能落在同块，也可能落到相邻块；内部会处理块分裂）；
  5. 刷新目标块元数据。
- 同 `(start, track)` 叠音：`id` 必须传对；`id` 不存在时返回 `None`，不回退到「随便删一个」。

注意 `Arc::make_mut` 的借用冲突：移除/插入要分两段作用域，不能边持有 `&mut chunk` 边调 `self.insert_sorted`。这是实现细节，但决定代码能不能写干净。

### 4.5 模型层 API

在 `crates/yinhe-core/src/note_write.rs` 增加：

```rust
pub fn update_note_at(
    &mut self,
    key: u8,
    start: u32,
    track: u16,
    id: u32,
    f: impl FnOnce(&mut Note),
) -> Option<Note>;

pub fn remove_note_at(
    &mut self,
    key: u8,
    start: u32,
    track: u16,
    id: u32,
) -> Option<Note>;

/// 跨 key / 跨 tick 移动；保留 id 与 gate（end - start）。
pub fn move_note_at(
    &mut self,
    old_key: u8,
    start: u32,
    track: u16,
    id: u32,
    new_key: u8,
    new_start: u32,
) -> Option<(Note, Note)>;

/// 单音符写路径的统计增量：用 before/after 直接更新
/// bucket_note_count / bucket_track_stats / track_note_count /
/// track_audible_count / note_count / tick_length，
/// 不调用 `rebuild_dirty()`（不重扫桶）。
fn apply_note_stat_delta(&mut self, key: u8, before: Option<&Note>, after: Option<&Note>);
```

- 删除 `update_note_by_id` 里的无条件 `bucket.sort()`；改由 `update_at` 内部搬移。
- `note_by_id` 保留给冷路径/测试；迁移完成后统计引用，单点 `update_note_by_id` / `remove_note_by_id` 若无调用方则删除。
- `new_start + gate` 用 `saturating_add` 并保证 `end >= start + 1`，顺手修掉当前 `move_note` 的潜在溢出。

### 4.6 调用点迁移

| 文件 / 场景 | 迁移方式 |
|---|---|
| `document/note_pencil.rs` `ResizeRight` | `update_note_at(key, before.start_tick, before.track, before.id, 修改 end)`，不再 sort |
| `document/note_pencil.rs` `ResizeLeft` | 同上，`update_at` 内部搬移 |
| `document/note_pencil.rs` 力度修改 | `update_note_at`，不再 sort |
| `document/note_pencil.rs` `Move` | `move_note_at`，不再 `remove_by_id` |
| `document/note_edit/add_delete.rs` `delete_single_note` | 已用 `range` 定位，直接 `remove_note_at` |
| `document/note_edit/add_delete.rs` `delete_notes(k)` | 逐个 `remove_note_at`（路径擦除 k 通常很小）；不再每个音符全桶扫 |
| `document/note_edit/add_delete.rs` `set_note_end_tick` | 签名补 `track` + `start_tick`（调用方本来就有），用 `update_note_at` 一次定位 |
| `app/midi_input.rs` `PendingRecordingNote` | 增加 `start_tick`（和 `track`，如需），NoteOff 时走 `update_note_at` |
| `mobile-core/scene/events.rs` `event_update` | 事件行本身有 tick/track，改调 `set_note_end_tick_at` |
| 铅笔 / MIDI 录音 / 单删的 `model.rebuild_dirty()` | 换成 `apply_note_stat_delta`（before/after 增量），不再重扫桶 |
| `document/clip_edit.rs` `move_clip` | 按 key 分组：`remove_by_ids` 一次 + 平移 + `insert_batch_sorted` 一次 |
| `document/clip_edit.rs` `loop_clip_right/left` | 按 key 收集新增音符，`insert_batch_sorted` 一次；不再逐音符 `insert_note` |
| `batch_ops.rs` / `history/apply.rs` | 保留 Bulk API；`update_selected_in_place` 的 `sort()` 先保留（整体平移通常仍有序，`is_sorted` 早退） |

`delete_notes` 的取舍：不按 `k` 分支（避免阈值）。统一走 `remove_note_at` 逐点删除，因为它的语义是「路径擦除」，k 与鼠标路径成正比；真正的百万级删除走选区批量路径，不走这里。如果以后出现大 k 的路径擦除，再加「按 key 一次 retain」的批量版本，不事先猜。

### 4.7 正确性测试

必须覆盖：

1. **同 start 跨块切分**：构造 65537 个 start 相同的音符，分别放在两个块；第一块尾部和第二块头部各放一个目标 track/id，两个都能被 `locate_at` 命中（只测第二块会漏掉“向前回退”这个 bug）。
2. **块首/块尾删除**：删块首后 `starts` 正确；删唯一元素后块被移除；删最大 end 后 `max_ends` 精确；删非最大 end 后 `max_ends` 不变。
3. **块内 start 搬移**：ResizeLeft 后块内仍有序，`max_ends` 正确。
4. **跨块 start 搬移**：新 start 落到相邻块，原块/目标块元数据都正确。
5. **跨 key 移动**：`move_note_at` 保留 id 与 gate，旧桶少一个、新桶多一个。
6. **COW 范围**：`Arc::clone` 一个桶模拟音频快照，单点 `update_at` 后，未触达块的 `Arc::ptr_eq` 仍为 true，只有目标块变。
7. **随机对拍**：1000 次随机增删改，维护一个 `Vec<Note>` 朴素模型，逐步比较排序、`max_ends`、id 集合。
8. **回归**：铅笔左拉伸、单音符删除、力度修改各一条，放在现有 `note_edit/tests` / `note_pencil` 测试里。

### 4.8 基准

扩展现有 `crates/yinhe-core/benches/chunked_bench.rs`（它已经是手写 `Instant` 风格，不引入 criterion）：

| bench | 规模 | 目的 |
|---|---|---|
| `update_end_by_id` vs `update_at` | 1M / 10M 音符/key | 量化 `is_sorted` + 无条件 sort |
| `update_velocity` | 1M / 10M | 量化纯 O(N) 扫描 |
| `update_end + rebuild_dirty`（真实铅笔路径） | 1M / 10M | 量化 stats 重扫 |
| `remove_by_id` vs `remove_at` | 1M / 10M | 量化查找 + `rebuild_max_ends` |
| `move_by_id` vs `move_at` | 1M / 10M | 量化移动 |
| `resize_left`（会触发失序） | 1M | 单独跑，避免 10M 下 full sort 超时 |

- 每个 case 先跑 100~1000 次取平均，`black_box` 防止优化掉。
- 10M 音符/key 是压力目标，不是产品硬限制（见附录 D）；初始化约 160MB + 排序，单次 bench 控制在 120 秒内，超时就降到 5M 或减少迭代。
- 目标（M0 评审时确认）：10M 桶的单点 update/remove/move **≤ 100µs**（1MB 块扫描 + memmove 的量级），不再出现毫秒级到秒级的抖动。
- M0 先测现状，M4 修完用同一套 bench 复跑；没有 before/after 数字就不算完成。

### 4.9 验收标准

- 铅笔拖动/拉伸在 10M 音符/key 的桶上不出现逐帧全桶扫描。
- 单点删除/单点 end 修改不再触发 `rebuild_max_ends` 全桶扫描。
- 单点编辑不再调用 `rebuild_dirty()` 重扫桶，统计用增量更新。
- 单点 start 修改不再触发 `sort()` / flat 全量拷贝。
- 未触达块的 `Arc` 指针保持不变（有测试）。
- `cargo test -p yinhe-types -p yinhe-core -p yinhe-editor-core` 全绿。
- `cargo clippy -p yinhe-types -p yinhe-core -p yinhe-editor-core` 无新增警告。
- 手工：start.mid 上拖动/拉伸/擦除；Broken World 上重复验证；移动端同一套 core 操作抽测（不专门改移动端 UI）。

---

## 5. 执行顺序

**M0 是硬门禁：bench / 测量结果出来并评审之前，不进 M1。**

### M0 bench / 测量清单

1. **单音符热路径**（1M / 10M 音符/key）：改 end、改 velocity、删除、移动、左拉伸；`*_by_id` vs `*_at` 各跑一遍，记录 µs/op 和峰值内存。
2. **批量操作**：`insert_batch_sorted` 往 1M 桶插入 10 万；`delete_notes` k=1/100/1000；`move_clip` k=1万/10万。
3. **undo delta**：1M / 10M / 100M，对比 `Vec<(Note,u8)>`、列式 delta、列式+zstd；记录编码/解码耗时和字节数。这是 3.6 的决策依据。
4. **内存快照**：加载完成、首帧上传完成、全量重建完成三个点，桌面 + 移动端各一份；100M 合成工程必测，start.mid 视 120 秒限制决定是否申请长时间测试。
5. **GPU 摘要段数**：桌面端 8 档各多少段、多少字节；确认是否值得继续优化。
6. **移动端 `NoteLayerCache` 桶增长**：高倍率垂直滚完 256 个 key，记录 RSS；确认窗口外桶是否是大头。
7. **移动端 `LayerSlot` 高水位**：全曲 zoom-out 再 zoom-in，记录 GPU/CPU 容量是否回落。

1~3 用于决定 P0 和 undo 是否值得做；4~7 用于决定桌面/移动端内存优化的优先级。只有数字能支撑「修完真的更快、更省」时，才进 M1。

### M0 基线记录表（2026-11-21 实测，Apple M5 / macOS 27.2.1 / release）

| 场景 | 1M/key | 10M/key | 备注 |
|---|---|---|---|
| 改 end（只写原语） | 1.9 ms | 19.4 ms | 按 id 扫桶 + is_sorted |
| 改 end + `rebuild_dirty` | 20.0 ms | 199.5 ms | 真实铅笔路径 |
| 改 velocity（只写原语） | 1.9 ms | 19.7 ms | 按 id 扫桶 + is_sorted |
| 改 velocity + `rebuild_dirty` | 20.0 ms | 195.3 ms | 真实铅笔/力度路径 |
| 移动（只写原语） | 1.7 ms | 17.8 ms | remove_by_id + insert |
| 移动 + `rebuild_dirty` | 20.0 ms | 193.8 ms | 真实移动路径 |
| 左拉伸（start 改变） | 38~44 ms | 497.5 ms | 10M：sort 289.9 + stats 207.6 |
| 删除（只写原语） | 1.7 ms | 18.6 ms | remove_by_id + rebuild max_ends |
| 删除 + `rebuild_dirty` | 20.2 ms | 147.6 ms | 真实单删路径 |
| `insert_batch_sorted` 10 万 | 16.4 ms | 64.6 ms | 真实 NoteBucket |
| undo 列式 delta（1M / 10M） | 7.87 B/音符 | 7.87 B/音符 | 编码 11.8 / 86.6 ms，解码 12.3 / 109.3 ms |
| undo 列式 + zstd3（1M / 10M） | 4.37 B/音符 | 3.99 B/音符 | 编码 154.7 / 832.2 ms，解码 28.0 / 179.0 ms |

### M0 结果解读

1. **10M 单 key 的真实铅笔路径约 195~200 ms/帧**，目标是 ≤100µs，差约 2000 倍。即使只修 id 查找和 sort，纯写原语仍有 18~20 ms，必须同时把 `rebuild_dirty` 的全桶 stats 扫描改成增量。
2. **`rebuild_dirty` 是主要成本**：10M 下一次约 150~180 ms，占完整路径的 80%~90%。它按脏 key 重扫整个桶重建 `bucket_track_stats`，与「改一个音符」不匹配。这是 P0 必须一起修的原因。
3. **左拉伸（start 改变）10M 约 497 ms**：290 ms 全桶 sort + 208 ms stats。当前 `sort()` 会做 flat 全量拷贝 + 排序 + 重建块，单帧内存峰值约 3× 桶（480MB），必须改成单音符重定位。
4. **批量插入时间可接受，但峰值内存未测**：10M 桶插 10 万音符 64.6 ms；`insert_batch_sorted` 的 2× 峰值仍需单独用 memtrace 验证。
5. **undo 压缩的初步结论**：
   - 列式 delta：7.87 B/音符（20B → -61%），10M 编码 86.6 ms、解码 109.3 ms；外推 100M 约 0.9 s / 1.1 s。
   - 列式 + zstd3：3.99 B/音符（-80%），但 10M 编码 832 ms、解码 179 ms；外推 100M 编码约 8 s，**不适合在编辑提交时做**。
   - 如果要做，优先「列式 delta，不叠 zstd」；100M 解码约 1 秒是否可接受，需要你拍板。若不可接受，就保持现状或只做历史预算。
6. 1M 的数字大约是同场景 10M 的 1/10；P0 的目标和验收应以 10M 为准。

注意：这是单 key 均匀分布的合成数据，不是 start.mid 的真实分布；真实曲目的最密集 key 可能更极端，但数量级已经足够说明问题。M4 修完后用同一命令复跑，不换数据。

| 阶段 | 内容 | 依赖 | 预估 |
|---|---|---|---|
| M0 | 补 bench + 尺寸断言 + 过期注释修正；跑 1~7，记录基线 | 无 | 1~2 天 |
| M1 | `NoteBucket::locate_at` + `update_at` + `remove_at` + 单元测试/对拍；模型层 `apply_note_stat_delta` 增量统计 | M0 通过 | 1~2 天 |
| M2 | 模型层 `*_at` API；迁移铅笔/力度/单删/`set_note_end_tick`；删除无条件 sort 和单点 `rebuild_dirty` | M1 | 1~2 天 |
| M3 | `delete_notes` / `move_clip` / `loop_clip_*` 批量化；COW 范围测试 | M2 | 1~2 天 |
| M4 | 复跑 bench 对比，确认目标；清理旧单点 by-id API | M3 | 0.5 天 |
| M5 | S3-1 注释/断言；关闭 LOD 时跳过摘要构建/上传；S3-4 `insert_batch_sorted` 流式归并 | 可与 M1~M4 并行 | 1~1.5 天 |
| M6 | S3-2/S3-3：8 档仍全量常驻，只做逐档构建/上传峰值优化和逐 key 重建评估 | M5 | 先测后估 |
| M7 | S3-5 undo delta：只有 M0 数据证明解码够快、省内存值得，才实现 | M0 数据 | 待定 |
| M8 | 移动端专项：`NoteLayerCache` 窗口外桶释放、`LayerSlot` 缩容（先测后做） | M0 数据 | 待定 |

P0（M1~M4）先合入，再碰内存结构。不要一边改音符布局一边改写路径，出问题无法定位。

---

## 6. 风险

| 风险 | 说明 | 缓解 |
|---|---|---|
| 同 start 跨块切分 | 最容易写错的边界，直接导致「找不到音符」 | 4.7 的专项测试 + 对拍 |
| `Arc` COW 语义 | 音频线程持有旧快照，编辑会复制触达块 | `ptr_eq` 测试；只刷新受影响块 |
| `max_ends` 不精确 | 影响 `tick_length` 和裁剪跳过 | 删除/收缩 max 时重算该块；测试断言精确值 |
| 旧 API 删除 | `update_note_by_id` / `remove_note_by_id` 还有测试/冷路径引用 | 先迁移，后 `rg` 审计，最后删；保留 `remove_by_ids` 批量 |
| 移动端 | 移动端共用 `yinhe-core`，签名变化会传导 | 只改 core 调用点，不改移动端 UI；`cargo test -p yinhe-mobile-core` 可选跑 |
| 撤销语义 | `NoteDelta` 的 before/after 必须与旧路径逐字一致 | 复用现有 undo 测试；新增铅笔/删除回归 |
| 基准误判 | 1M 桶的 O(N) 只有几毫秒，容易觉得「还行」 | 必须跑 10M 单 key，目标 ≤100µs |
| 摘要档位测量偏差 | 不同曲目的段数差异极大 | 用 start.mid + Broken World 两份数据，不用合成均匀分布 |

---

## 7. 已定 / 待定

已定（2026-11-21）：

1. 单 key 没有独立上限，总量受 id / GPU offset 的 u32 限制；10M/key 只作为 bench 压力目标，不作为产品硬限制。见附录 D。
2. GPU 摘要 8 档必须常驻，不做「首次进入某档再准备」；只优化构建/上传峰值。见 3.3。
3. undo 压缩必须先 bench。见 3.6。
4. 先做 bench，再开始修；M0 是硬门禁。见第 5 节。

待定：

1. bench 的验收线：10M 桶单点 update/remove/move ≤100µs 是否够用？undo 1 亿音符的列式解码可接受多少 ms（bench 外推约 1.1s）？
2. 移动端 4~6GB 的目标工程规模是多少（1 亿 / 5000 万 / 更小）？这决定 undo 压缩、`NoteLayerCache` 桶释放、保存峰值三者的优先级。
3. 保存峰值（1.64 亿约 3.9GB）是否要单独排期？移动端上它可能比音符结构更早触发 OOM。
4. 是否同意先只做 P0（M0~M4），内存结构等实测数据出来再排期。

---

## 附录 A：为什么保留 `start + end`

详见第 2 节。补充一个容易忽略的点：`Note.end_tick` 天然保证 `end >= start`，而 `gate` 需要额外保证 `gate >= 1`；在当前代码里，`end` 还承担了 `max_ends`（块级裁剪）的预计算值。改 gate 会让「裁剪」和「显示」两个路径都多一次加法，且没有内存收益。

## 附录 B：尺寸与源文件

| 类型 | 文件 | 大小 |
|---|---|---|
| `Note` | `crates/yinhe-types/src/note.rs` | 16B（4+4+4+1 + 1 pad + 2） |
| `NoteEvent` | `crates/yinhe-core/src/events.rs` | 16B（4+4+4+1+1+2 pad） |
| `NoteInstance` | `crates/yinhe-wgpu/src/vertex.rs` | 12B（已有测试断言） |
| `NoteSummary` | `crates/yinhe-types/src/note_summary.rs` | 16B |
| `DominoNote` | `crates/yinhe-editor-core/src/domino_clipboard.rs` | 12B（4+4+1+1+2 pad） |
| 剪贴板条目 | `crates/yinhe-editor-core/src/clipboard_file.rs` | 12B（定长序列化） |
| `NoteDelta` | `crates/yinhe-editor-core/src/history.rs` | 20B × 音符数 × 侧数（16B Note + key + pad） |

## 附录 C：本次代码勘察发现的注释过期

- `crates/yinhe-wgpu/src/cull.rs` 顶部：2×16B / 100M ≈ 3.2GB。
- `crates/yinhe-wgpu/src/render_thread.rs`：`16B NoteInstance`。
- `crates/yinhe-wgpu/src/arrangement/instances.rs`：`Output is 16B NoteInstance`。
- `crates/yinhe-wgpu/src/resource.rs`：`100M 音符全量 all_notes + visible_notes ≈ 3.2GB`。
- `crates/yinhe-wgpu/src/pianoroll/instances.rs`：`build_all_notes` 的 `约 2.6GB`。
- `crates/yinhe-wgpu/src/automation/velocity_summary.rs`：注释写「9 档并行」，但 `SUMMARY_BLOCK_TICKS` 实际是 8 档。

这些不影响功能，但会影响后续做内存决策的人。M0 一并修正。

## 附录 D：单 key / 单轨 / 总音符数上限

**结论：没有单独的「单 key 上限」；总量上限是 u32。10M/key 只是压力基准，不是代码限制。**

代码里的实际约束：

| 约束 | 位置 | 上限 |
|---|---|---|
| 总音符 id | `Note.id: u32`、`next_note_id: u32`（`crates/yinhe-core/src/model.rs`） | u32::MAX（0 保留） |
| GPU cull 总 offset / per-key count | `build_all_notes` 的 `[u32; KEY_COUNT + 1]`、`CullState.per_key_counts: [u32; KEY_COUNT]` | u32::MAX |
| `.yin` 轨段音符数 | `TrackSegment.count: u32`（`crates/yinhe-yin/src/data_section.rs`） | u32::MAX/轨 |
| 导入期 per-key 预分配 | `NoteLoader::new(per_key_count: [u32; KEY_COUNT])` | u32::MAX/key |
| MIDI writer 轨内索引 | `off_order: Vec<u32>`（`crates/yinhe-midi/src/writer.rs`） | u32::MAX/轨 |
| 单 key 容器 | `NoteBucket`：`Vec<Arc<Vec<Note>>>` + `usize` 索引 | 受内存限制，远大于 u32 总量 |
| 单 key 播放游标 | `NoteCursor.chunk: u32` | 2^32 块 × 65536，远大于 u32 总量 |
| 轨道数 | `Note.track: u16`、`MAX_TRACKS = 65536` | 65536 轨 |

所以：

- **总音符数**：实际硬顶是 u32::MAX（约 42.9 亿），由 id 和 GPU offset 共同决定；`note_count: u64` 只是统计字段，不代表能超过 u32。
- **单 key**：没有比 u32::MAX 更低的显式限制；`NoteBucket` 本身可以继续分块，`NoteCursor.chunk` 也够大。真正的限制是内存：10M/key = 160MB 模型，100M/key = 1.6GB 模型。
- **单轨**：受 `track: u16` 限制最多 65536 轨；单轨音符数受 u32 限制。

一个需要补的安全口子：`alloc_note_id()` 用的是 `wrapping_add(1)`，目前没有在接近 u32::MAX 时拒绝分配。用户说总量最大 u32，但代码里没有显式 guard；如果真到达上限，id 会回绕并可能与旧音符冲突。建议在导入/新增路径加一个明确的「达到 u32::MAX 就报错/停止分配」的检查，而不是依赖回绕。本次不改代码，先记录。

因此 bench 的规模选择：

- 1M/key：普通黑乐谱歌曲；
- 10M/key：压力上限，覆盖 start.mid 这类极端曲目中最密集的 key；
- 100M 总量：跨 key 总量测试，验证单点操作不随总量退化（每个 key 仍然只受本桶大小影响）。
