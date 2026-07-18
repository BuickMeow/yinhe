# yinhe 代码审查报告

> 审查对象：yinhe 黑乐谱编辑器（基于公开仓库 `ff2fbb0`）  
> 审查日期：2026-07-16  
> 规模：19 个 crate，约 34.8k 行

---

## 0. 总体印象

工程纪律整体尚可：全项目 `unwrap()` 仅 43 处（其中 20 处在 archive crate），`unsafe` 集中在 ring buffer / 平台层 / wgpu hal，crate 划分清晰，UI crate 最大文件仅 964 行。测试覆盖存在（`engine_tests.rs` 772 行 + 独立 `yinhe-tests`）。

但地基有三颗炸弹：**音符身份系统、全局分配器、音频-编辑耦合**。全部是难发现、难定位、难修的类型，且直接解释了实测的"1000W 音符爆音+减速"现象。

---

## P0 —— 数据损坏 / 确定性故障

### 1. 音符身份系统崩溃，undo 连带误删（最严重）

撤销系统用 `(track, start_tick, dup_index)` 三元组作为音符唯一身份：

```rust
// history.rs:311-323
let mut remove_by_key: HashMap<u8, HashSet<(u16, u32, u8)>> = ...;
bucket.retain(|n| !to_remove.contains(&(n.track, n.start_tick, n.dup_index)));
```

但这个身份在三处都不保证唯一：

- **加载时**：`dup_index: u8`，`saturating_add` 到 255 饱和。黑乐谱同 key 同 tick 叠数百个音符是常态 → 全部共享 `dup_index=255`
- **移动音符**：移动后一律写死 `dup_index: 0`，叠层移动一次全部同身份
- **批量插入**：`insert_batch` 不重新分配 dup_index，粘贴副本与原件同身份

后果：`apply_note_delta` 的 `retain` 会把所有身份相同的音符一起删掉。对叠层密集的曲子，撤销一次移动/删除 = 悄悄删掉成百上千个音符，且 redo 无法恢复。这是编辑器**数据损坏级 bug**，对黑乐谱场景是确定性触发。

**建议**：给每个音符发全局唯一 `u64` id（加载时和编辑时统一发号），undo/redo/拖动快照一律按 id 匹配，dup_index 仅作显示用途。

### 2. seek 的二分查找 predicate 非单调

`engine_state.rs:164-170` 的 `partition_point` 要求 predicate 单调，但 vel≤1 的装饰音穿插在桶内时，predicate 序列变成 `false true false…`，二分前置条件被破坏。返回位置无意义 → seek 后游标错位 → 丢音或重复触发。桶内 vel≤1 音符越多，错得越离谱。

### 3. renderer 线程无超时自旋等 CPAL 确认

每次 Play / Seek / Stop / 模型应用都会执行一段无上限自旋等待：

```rust
while self.state.reset_ack.load(Ordering::Acquire) != generation
    && !self.shutdown.load(Ordering::Relaxed)
{
    thread::sleep(WAKE_SLEEP);   // 无超时，无上限
}
```

ack 依赖 CPAL 回调跑一圈，但 CPAL 不保证回调持续触发：设备拔出、蓝牙断连、系统睡眠、流内部错误后回调可能永久停止。一旦停止 → renderer 线程**永久卡死** → 音频全灭，命令通道静默堆积，UI 无任何错误提示。必须重启进程。

**建议**：消费者侧自己已经监听 generation 并清 ring，生产者根本不用等 ack；要等活动线程确认也应加超时+降级。

---

## P1 —— 严重性能缺陷

### 5. memtrace 全局分配器：全程序性能税 + 内存膨胀

`yinhe-memtrace` 给全局分配器包了一层做分类统计：

- **废掉原地扩容**：mimalloc/jemalloc 的原地 realloc 优化全部失效，所有 `Vec` 增长退化为 分配+拷贝+释放。对千万级音符的桶 Vec、egui 每帧的海量小 Vec，这是持续的拷贝税；
- **原子计数 cacheline 乒乓**：每次 alloc/dealloc 都对 7 个共享 `AtomicIsize`（同一 cacheline）做 `fetch_add/sub`，多线程下持续乒乓；
- **每分配 +24B 头**，小分配内存膨胀明显。

讽刺的是 mimalloc 自带 `mi_stats` 可以拿分类统计外的几乎所有指标，零成本。

### 6. release 用 `opt-level = "z"` + `panic = "abort"`

```toml
[profile.release]
opt-level = "z"     # 按体积优化 —— 对实时合成/渲染是错误选择
panic = "abort"     # 任何 panic = 无栈直接死
```

一个拼极限性能的黑乐谱渲染器，发布版按体积优化。-Oz 相对 -O2/3 在数值密集代码上慢 10-30% 不稀奇，直接作用于 xsynth 的 sample 内层循环和 GPU 事件构建。配合 `panic="abort"`，P0 那些边界情况一旦触发 panic，用户拿到的就是无声崩溃，连 backtrace 都没有。

### 7. device_lost 一次置位，永不恢复

wgpu device lost 在 Windows 重负载 TDR 下不算罕见。一旦触发：钢琴卷帘永久停更，只有一行日志，无对话框、无重建、无自救。用户只能自己悟出要重启。

---

## P2 —— 设计缺陷

### 8. 框选拖动按"当前框内内容"移动（已知 bug 的代码实锤）

- `batch_ops.rs:25-59` 按调用时刻的 selection rect 现查音符；
- `document.rs:826` 每帧移动后 `selection.offset(...)` 让框跟随；
- 下一帧再按新框位置重新查询 → 框扫到哪里，哪里的音符被卷进来。

正确做法：拖动开始时快照命中音符身份集，拖动期间只操作快照集。但目前没有可靠身份可快照（P0-1），两个 bug 互相卡死。

### 9. 编辑-音频耦合过紧：每次编辑全量重备 + 全量 chase

每次编辑触发 `reload_notes` → worker 全量 `flatten_automation_to_cc_events`（density 展开后 CC 事件可达几十万条）→ renderer `apply_prepared_model` → `seek_to(current)` → `inject_chase` 从头线性扫 CC 到当前位置。

后果：自动化多的曲子，每拖一个音符，renderer 线程都要做几十万次 `ChannelState::apply`，期间停止渲染 → ring 耗尽 → 填零爆音。编辑与播放体验互相伤害。

**建议**：编辑只改音符时不动 CC 链；chase 用检查点索引（每 N tick 缓存一次 256 通道状态快照），seek 从最近检查点起放。

### 10. seek 只重启一个跨点音符，叠层丢音

seek 时只检查 `notes[cursor-1]` 一个音符是否跨越 seek 点。同一 key 有多个重叠音符同时跨越时（黑乐谱叠层常态），只有排序后最后一个被重启，其余全部丢失。

### 11. 命令通道 unbounded + 每次 Seek 自旋清空

`unbounded::<AudioCommand>()`：疯狂拖进度条时 Seek 命令无限堆积，每个 Seek 又触发 P0-3 的自旋等待 → 卡顿被放大且延迟越来越长。应有界 + 拖拽期间合并 Seek（只保留最新）。

### 12. GPU 事件构建的内存尖峰

每次模型应用把全部音符展开成 2N 个 `SynthEvent` 再排序。1000W 音符 → 2000W 事件 × 16B ≈ 320MB 临时分配 + O(n log n) 排序，叠加 memtrace 的 +24B/alloc 和拷贝式 realloc，加载大曲子时内存尖峰可观。

---

## P3 —— 小毛病

| 位置 | 问题 |
|---|---|
| `history.rs:431` | undo 栈满用 `Vec::remove(0)`，O(n) 搬移，应 `VecDeque` |
| `spawn.rs:361` | cpal 流错误仅 `eprintln!` |
| `spawn.rs:65` | `AudioHandle::send` 吞掉断开错误 |
| `spawn.rs:263`、`audio_renderer.rs:407` | 线程 spawn 失败 `.expect()`，release 下无栈 abort |
| `audio_ring.rs:92-95` | producer 侧 `clear()` 与 consumer 并发存在逻辑竞态 |
| `engine_render.rs:55` | 死代码残留 |
| `export.rs:90` | 尾部硬上限 30s，长 release 音色库会被截断 |

---

## 值得肯定的部分

- AudioRing 的 SPSC 实现本身正确（单调计数器避免满/空歧义）；
- worker 线程模型合理：`PrepareModel` 有命令合并，连续编辑只保留最新；
- undo 用 delta 而非快照，`rebuild_dirty` 只排脏桶且利用 Arc 共享避免深拷贝；
- 文件加载全异步，UI 不阻塞；
- CPAL 回调只做 ring 消费，重活在 renderer 线程——架构方向对。

---

*本报告基于公开仓库代码的静态分析。*
