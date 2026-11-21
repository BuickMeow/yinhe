# 音符数据结构与单音符热路径修复规划

> 日期：2026-11-21
> 范围：只做规划，不改代码。承接上一轮讨论「起点+长度 vs 起点+终点」和「1 亿音符下真正卡手的地方」。
> 关联：`docs/2026-07/optimization-260721.md`（早期 16B 结论与全量驻留问题）、`crates/yinhe-yin/src/lib.rs`（保存/加载峰值）。
> 目标：1 亿音符（极端 1.64 亿）可编辑；任意位置随机编辑的代价只与本块有关，不随整桶音符数退化。
> 约束：不引入缓存、不引入阈值分级、不为了省 1~2 字节破坏 `&Note` 访问模型。

## 0. 结论

1. **保留 `start_tick + end_tick`，`Note` 维持 16B。** 换成「起点+长度」内存完全一样，性能差异被内存带宽淹没；现有 `max_ends`、`range_including_long`、GPU `NoteInstance` 都以 end 为中心，改 gate 只增加溢出和 `gate = 0` 两个边界。
2. **P0 是第 4 节的单音符热路径**：按 id 线性扫整桶、每次 `update_note_by_id` 无条件 `sort()`、单点删除后全桶重算 `max_ends`。这是 1 亿音符下真正会卡手的地方，优先级高于省 4B/音符。
3. **P1 是第 3 节的内存副本**，但先测量再改：LOD 关闭时仍构建 8 档摘要、GPU 摘要 8 档全量常驻、全量重建时 raw + summaries 同时驻留、`insert_batch_sorted` 的 2× 峰值、undo delta。没有测量数据之前不动结构。
4. **明确不做**：去掉 `id`（12B）、`repr(packed)`（15B）、track 改 u8、u24 tick、块内 delta/varint、SoA 重写、start+gate 重构。理由见 3.8。

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

以 1.64 亿音符（start.mid）为例，数量级如下。没有标注「实测」的都是按结构体大小推算，实施前用 `yinhe_memtrace::Snapshot` 复核。

| 项目 | 表示 | 单位成本 | 1 亿 | 1.64 亿 | 性质 | 处理 |
|---|---|---|---|---|---|---|
| CPU 模型 | `Note` | 16B/音符 | 1.60GB | 2.62GB | 常驻 | 不动 |
| GPU 原始层 all_notes | `NoteInstance` | 12B/音符 | 1.20GB | 1.97GB | 常驻 | 不动 |
| GPU 原始层 visible 索引 | `u32` 索引 | 4B/槽，按 256 向上取整 | 0.40GB | 0.66GB | 常驻 | 不动 |
| GPU LOD 摘要层 | `NoteInstance` + 索引 | 12B/段 + 4B/槽 | 待测 | 待测 | 常驻 | S3-2 |
| 全量重建的 raw 缓冲 | `Vec<NoteInstance>` | 12B/音符 | 1.20GB | 1.97GB | 瞬时 | S3-3 |
| 全量重建的摘要缓冲 | 8 档 `Vec<NoteInstance>` | 12B × Σ段 | 待测 | 待测 | 瞬时 | S3-3 |
| UI 可见窗口 | per-key + flat | 2 × 12B × 窗口段数 | 随窗口 | 随窗口 | 常驻 | 3.7 |
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

### 3.3 S3-2：GPU 摘要 8 档全量常驻（P1，先测量）

现状：

- `SUMMARY_BLOCK_TICKS = [262144, 65536, 16384, 4096, 1024, 256, 64, 16]`；
- `build_summaries` 一次构建 8 档，`upload_summary_all` 把 8 档全部上传；
- `CullState.summary_levels` 常驻 8 档，每档 12B/段 + 4B/槽 + draw args；
- 只有一档会被当前 ppu 选中，其余 7 档在显存里等待。

先做低风险开关，再测量：

1. **先修一个已确认的浪费（低风险）**：桌面 LOD 开关关闭时，`upload_all_with_summaries` / `start_rebuild` / `upload_key_with_summaries` 仍然会构建并上传 8 档摘要，`lod_enabled` 只影响渲染时选不选摘要层。把该开关传进这三条路径，关闭 LOD 时直接跳过摘要构建/上传。这是纯省内存，不改变默认行为。
2. 在 `upload_summary_all` 里按档打印段数、字节数（`tracing::info!` 或 memtrace 统计）。
3. 用 start.mid（1.64 亿）和 Broken World（4444 万）各跑一次，记录 8 档合计。
4. 如果合计 < 原始层 30%，其余项关闭，不值得为它增加复杂度。
5. 如果合计很大，按以下顺序做：
   - **A. 逐档构建/上传，上传后立即释放 CPU `Vec`**：至少削掉「8 档 CPU 缓冲同时存在」的瞬时峰值，GPU 常驻不变。
   - **B. 按需常驻**：只保留当前 ppu 选中的档，切档时后台构建/上传；未就绪期间优先回退到更粗的已上传档，没有则走原始层。注意现有 `summary_level_ready` 的 `(best..len).find` 是向**更细**档回退，和“省显存”的目标相反，需要改成 coarser-first 或显式保留一档更细兜底。
   - **C. 减少档位数量**：这是视觉决策，必须先看「4 ticks/px 附近没有 16 档会怎样」，不擅自删档。

边界：摘要层不带选中位、不做选中轨道排序；切档失败必须能回退原始层，不能黑屏。验证：在 start.mid 上连续缩放 8 个档位，记录每档首次进入的帧耗时和显存。

### 3.4 S3-3：全量重建时 raw + summaries 同时驻留（P1）

现状：

- `CullRebuild::Building` 在后台线程 `build_all_notes` 产出全量 `Vec<NoteInstance>`（约 1.97GB @ 1.64 亿）；
- `BuildResult` 同时持有 `summaries: Vec<(Vec<NoteInstance>, offsets)>`，8 档全在；
- UI 线程按每帧 4 个 key 上传，上传完成前整份载荷都活着。

`build_all_notes` 已经做过一次优化（去掉 per-key 中间 Vec，峰值从约 5GB 降到一份最终缓冲），不要回退。剩余峰值来自「raw + 8 档摘要同时存在」。

分两步：

1. **先做逐档（低风险）**：`BuildResult` 只保留 raw + offsets；摘要改为上传状态机里逐档构建、逐档上传、每档上传后释放。raw 不变。
2. **再评估逐 key 流式（高成本）**：raw 也按 key 构建/上传，后台线程一次只持有一个 key 的实例（最坏单 key 10M = 120MB），上传完即释放。需要处理「构建期间模型又改了」的失效逻辑（现有 revision 检查可复用），以及 UI 分帧节奏。

验收：全量重建期间 RSS 峰值下降；UI 不卡顿；track 显隐切换期间旧 buffer + track_mask 双重过滤仍然正确。

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

### 3.6 S3-5：undo `NoteDelta` 的整份副本（P2，先测量）

现状：删除/复制/回退路径把 `(Note, key)` 全量拷进 `before`/`after`。`(Note, u8)` 是 20B/音符，删除全选 1 亿音符 = 2.0GB 常驻在 undo 栈里。

可选方案：

1. **列式压缩 delta**：按 key 存 start delta varint、gate varint、vel、track、id delta，典型 6~8B/音符，省 50%~60%；undo 时解码。代价是编解码复杂度和一次额外扫描。
2. **历史预算**：按字节数淘汰最老的 undo entry（产品决策，可能需要你拍板）。
3. **不动**：先测量实际 undo 栈峰值。如果全选删除很少发生，1.6GB 只在极端操作后短暂存在。

本次建议：只做测量和记录，不实现压缩。除非实测显示 undo 是继模型/GPU 之后的第三大常驻项。

### 3.7 S3-6：接受 / 不修

- **`iter_mut` + PPQ 缩放 / 轨号 remap**：所有音符都变，且音频线程持有旧快照时必须 COW，2× 峰值不可避免。PPQ 缩放是低频操作，接受。
- **`sort()` 的 3× 峰值**：单点路径由 P0 消除；批量操作（如导入后全量排序）仍可能触发，但导入路径已经走 `NoteLoader::finish` 的逐桶排序，峰值受控。
- **保存 3.9GB / 加载 2.7GB**：已有专文记录，不在本次范围。
- **UI `NoteLayerCache` 的 per-key + flat 两份**：正常缩放下窗口很小；极端缩小走 LOD 摘要。先观察，不为它改结构。
- **`NoteBucket::len()` 的 O(块数) 求和**：模型层已有 `bucket_note_count` 缓存，热路径不直接调；不改。

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
| 批量选区 `update_selected_in_place` / `remove_by_ids` | 逐桶一次扫描 | O(N_key) | 可接受 |
| 历史 apply 的 `remove_by_ids` / `update_matching` | 逐桶一次扫描 | O(N_key) | 可接受 |

关键细节：

- `NoteBucket::update_by_id` 每次调用后，`YinModel::update_note_by_id` 都会 `bucket.sort()`（`crates/yinhe-core/src/note_write.rs`）。
- `sort()` 先 `is_sorted()` 全桶扫描；**一旦单点改 start 造成失序**，就走 `flat: Vec<Note> = 全桶拷贝` → `sort_by_key` → `from_sorted` 重建块。10M 的桶 = 160MB 拷贝 + 160MB 排序临时 + 160MB 新块，约 480MB/帧。
- `remove_by_id` 先 `chunks.iter().position(|c| c.iter().any(...))` 全桶找 id，删除后 `rebuild_max_ends()` 又扫全部音符。
- 这些路径的调用方通常刚刚用 `range(start, start+1).find(...)` 定位过同一个音符，等于同一个音符找了两遍。

### 4.2 设计原则

1. **id 是身份，不是热路径索引。** 保留 id 用于选区、undo、跨轨移动；但单音符编辑用 `(start_tick, track)` 定位，id 只在同 `(start, track)` 叠音时消歧。
2. **不新增持久 id → 位置索引。** 任何 insert/remove/sort 都会让位置失效；维护成本高、每音符多 4~8B，还不如省下来。
3. **不做全局 sort。** 单点 start 变化只搬移该音符，并刷新受影响块的 `starts` / `max_ends`。
4. **API 语义分开。** `*_at` 给热路径（要求坐标是最新的），`*_by_id` 只留给冷路径/测试。迁移完成后，如果 `*_by_id` 单点 API 没有调用方，按项目规则删除，不留旧 fallback。
5. **一次操作只 `mark_dirty` 一次。** 批量路径不逐音符 `mark_dirty`。

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
| `remove_by_id` vs `remove_at` | 1M / 10M | 量化查找 + `rebuild_max_ends` |
| `move_by_id` vs `move_at` | 1M / 10M | 量化移动 |
| `resize_left`（会触发失序） | 1M | 单独跑，避免 10M 下 full sort 超时 |

- 每个 case 先跑 100~1000 次取平均，`black_box` 防止优化掉。
- 10M 音符/key 初始化约 160MB + 排序，单次 bench 控制在 120 秒内；如果超时，降到 5M 或减少迭代。
- 目标：10M 桶的单点 update/remove/move **≤ 100µs**（1MB 块扫描 + memmove 的量级），不再出现毫秒级到秒级的抖动。

### 4.9 验收标准

- 铅笔拖动/拉伸在 10M 音符/key 的桶上不出现逐帧全桶扫描。
- 单点删除/单点 end 修改不再触发 `rebuild_max_ends` 全桶扫描。
- 单点 start 修改不再触发 `sort()` / flat 全量拷贝。
- 未触达块的 `Arc` 指针保持不变（有测试）。
- `cargo test -p yinhe-types -p yinhe-core -p yinhe-editor-core` 全绿。
- `cargo clippy -p yinhe-types -p yinhe-core -p yinhe-editor-core` 无新增警告。
- 手工：start.mid 上拖动/拉伸/擦除；Broken World 上重复验证；移动端同一套 core 操作抽测（不专门改移动端 UI）。

---

## 5. 执行顺序

| 阶段 | 内容 | 依赖 | 预估 |
|---|---|---|---|
| M0 | 补 bench + 尺寸断言 + 过期注释修正；记录现状数字 | 无 | 0.5~1 天 |
| M1 | `NoteBucket::locate_at` + `update_at` + `remove_at` + 单元测试/对拍 | M0 | 1~2 天 |
| M2 | 模型层 `*_at` API；迁移铅笔/力度/单删/`set_note_end_tick`；删除无条件 sort | M1 | 1 天 |
| M3 | `delete_notes` / `move_clip` / `loop_clip_*` 批量化；COW 范围测试 | M2 | 1~2 天 |
| M4 | 跑 bench 对比，确认目标；清理旧单点 by-id API | M3 | 0.5 天 |
| M5 | S3-1 注释/断言；关闭 LOD 时跳过摘要构建/上传；S3-4 `insert_batch_sorted` 流式归并 | 可与 M1~M4 并行 | 1~1.5 天 |
| M6 | S3-2/S3-3 测量；只有数据支持时才做摘要懒上传 / 逐 key 重建 | M5 | 先测后估 |
| M7 | S3-5 undo delta 测量；决定是否压缩 | 全局测量后 | 待定 |

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

## 7. 需要你拍板的问题

1. 单 key 1000 万音符是真实上限，还是只会到 100~300 万？这决定 P0 的目标是 100µs 还是 1ms。
2. GPU 摘要 8 档是否都要常驻？如果 16 tick 档在显存里占大头，能否接受「首次进入某缩放档时后台准备一帧」？
3. undo 全选删除 1 亿音符的 2.0GB 副本，是否值得做列式压缩？还是接受它只在极端操作后短暂存在？
4. 是否同意先只做 P0（M0~M4），内存结构等实测数据出来再排期？

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
