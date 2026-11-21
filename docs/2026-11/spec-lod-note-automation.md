# LOD 吞音符 / 自动化规则

> 日期：2026-11-21
> 范围：解释 8 档 LOD 在 PR、AR、AM 三个视图里分别如何聚合，尤其是「一系列音符最后留下哪一个」。
> 结论先行：PR 音符 LOD 不是「选一个代表音符」，而是按 `(key, block)` 输出一个包络矩形；AM 力度条 LOD 才是「选一个代表音符」（gate 最长者）；AM 自动化曲线没有 LOD。
> 关联：`docs/2026-11/plan-note-storage-and-hot-paths.md`（M0 实测与优化计划）。

## 0. 一张表

| 视图 / 层 | 聚合单位 | 保留字段 | 被吞掉的 | 代码 |
|---|---|---|---|---|
| PR 音符 LOD（桌面 GPU cull / CPU fallback） | `(key, block)`，**跨 track 合并** | `[min_start, max_end)`、最大 velocity、主导 track | 同块内所有具体音符；非主导 track 的颜色 | `yinhe-wgpu/src/pianoroll/summary.rs` |
| AR 音符 LOD | `(key, block, track)`，**保留 track** | 每 track 的 `[min_start, max_end)`、最大 velocity | 同 track 同块内除 min/max 外的音符；然后再按 ≤1px 间隔合并 | `yinhe-core/src/lod.rs` + `yinhe-wgpu/src/arrangement/instances.rs` |
| AM 力度条 LOD | 每个 `block`，**全局不分 key** | 一个代表音符的真实 `[start, end)`、velocity、track | 同块内除代表外的所有音符 | `yinhe-wgpu/src/automation/velocity_summary.rs` |
| AM 自动化曲线（CC/PB/RPN/Tempo） | 无 LOD | 视口内全部事件 | 无（只有视口裁剪） | `yinhe-wgpu/src/automation/data_lines.rs` |
| AR 原始层（未走摘要时） | `(key, track)` 相邻间隔 ≤1px | `[min_start, max_end)`、最大 velocity | 被合并的短音符 | `yinhe-wgpu/src/arrangement/instances.rs` |

## 1. 档位与选择

```text
SUMMARY_BLOCK_TICKS = [262144, 65536, 16384, 4096, 1024, 256, 64, 16]
SUMMARY_MAX_PX = 4.0
```

选择规则：`block_ticks * ppu <= 4px` 时启用摘要；从大到小取第一个满足的档位。`ppu` 是 pixels per tick（每 tick 多少像素）。

| 档位 | block_ticks | 生效 ppu 区间 | 块在屏幕上的宽度 |
|---:|---:|---|---:|
| 0 | 262144 | `0 < ppu <= 0.00001526` | ≤4px |
| 1 | 65536 | `0.00001526 < ppu <= 0.00006104` | ≤4px |
| 2 | 16384 | `0.00006104 < ppu <= 0.00024414` | ≤4px |
| 3 | 4096 | `0.00024414 < ppu <= 0.00097656` | ≤4px |
| 4 | 1024 | `0.00097656 < ppu <= 0.00390625` | ≤4px |
| 5 | 256 | `0.00390625 < ppu <= 0.015625` | ≤4px |
| 6 | 64 | `0.015625 < ppu <= 0.0625` | ≤4px |
| 7 | 16 | `0.0625 < ppu <= 0.25` | ≤4px |
| 原始层 | — | `ppu > 0.25` | 块宽 >4px |

关键点：**档位是连续的公式，不是行为阈值**。块宽在屏幕上不超过 4px 时，块内空隙肉眼不可见，所以聚合在视觉上等价于「把音符吸到块粒度」。

## 2. PR 音符 LOD：一个矩形，不是代表音符

### 2.1 算法

`build_key_summary(key, notes, block_ticks)`：

1. `notes` 已按 `start_tick` 升序。
2. 按 `block = start_tick / block_ticks` 分组。
3. 每个 block 内累加：
   - `min_s = min(start_tick)`
   - `max_e = max(end_tick)`
   - `max_vel = max(velocity)`
   - `counts[track] += 1`
4. 输出一个 `NoteInstance`：
   - `start_tick = min_s`
   - `end_tick = max_e`
   - `velocity = max_vel`
   - `track = dominant_track(counts)`（出现次数最多；并列取 track 索引最小）
5. 每个 `(key, block)` 一个段，输出按 `start_tick` 升序。

所以它保留的是**包络**，不是某一个音符：

- 左边界来自块内**最早开始**的音符；
- 右边界来自块内**最晚结束**的音符；
- 力度来自块内**最响**的音符；
- 颜色来自块内**出现次数最多**的 track；
- 其余音符的个体信息全部被吞掉。

### 2.2 一系列音符的例子

key 60、track 0，7 个音符：

```text
A: [0, 10)
B: [100, 110)
C: [300, 800)     gate 500
D: [600, 610)
E: [900, 1200)    gate 300
F: [1100, 1110)
G: [2000, 2010)
```

| block_ticks | 输出段 | 说明 |
|---:|---|---|
| 16（档 7） | 7 段，基本逐个保留 | 每 16 tick 一块，A/B/C/D/E/F/G 多数落在不同块 |
| 256（档 5） | 6 段 | A+B 合为 `[0,110)`；C `[300,800)`；D `[600,610)`；E `[900,1200)`；F `[1100,1110)`；G `[2000,2010)` |
| 1024（档 4） | 2 段 | block 0：A/B/C/D/E → `[0,1200)`；block 1：F/G → `[1100,2010)` |
| 262144（档 0） | 1 段 | 全部 → `[0,2010)` |

注意 block 1024 时的两个段在 `[1100,1200)` 区间重叠：C/E 作为长音符属于 block 0，但它们的 end 伸进了 block 1。PR LOD 按 `start_tick` 分块，不按覆盖区间切分，所以段之间允许重叠。GPU 会按顺序画两个矩形，后画的可能覆盖前画的颜色。

### 2.3 长音符与视口

- 一个长音符只在它的 `start_tick` 所在块出现一次，但它的 `end_tick` 可以跨很多块。后续块不会重复它。
- `build_key_summary_in_range` 在可见窗口内构建时，会保留 `end_tick >= tick_start` 的长音符（即使它的 start 早于视口左边界）。它的 `start_tick` 可能小于视口起点，由 GPU 裁剪；这样不会在视口边缘漏掉长音符。
- 反过来，一个音符如果 start 在视口右侧，就不会被包含；这与原始层的右边界 `start < tick_end` 一致。

### 2.4 主导 track、力度、哑音

- `dominant_track` 按数量投票；如果 block 内 track 0 有 3 个音符、track 1 有 2 个，段颜色是 track 0；即使 track 1 的音符更长或更响，颜色也不会选它。
- 力度取最大值：一个哑音（velocity 1）和一个响音同块时，段是响的；哑音被吞掉。只有整块都是哑音时，段才是哑音。
- 所以「哪个音符留下了」要分字段回答：
  - x 起点：最早开始的音符；
  - 宽度：最晚结束的音符；
  - 力度：最响的音符；
  - 颜色：数量最多的 track；
  - 通常不是同一个音符。

### 2.5 跨 track 合并的代价

PR 的一个段只发一个 `NoteInstance`，`packed` 里只能放一个 track。跨 track 合并后：

- 其他 track 的颜色在这个块里消失；
- 选中/隐藏如果按 track 过滤，摘要层只能按主导 track 或整段处理，无法精确到单个音符；
- PR 路径可以在聚合前逐音符过滤 hidden（`build_key_summary_in_range` 和 GPU cull 的 raw 上传都会先按 hidden 过滤），所以不需要像 AR 那样整体回退原始层。

如果未来希望保留每个 track 的颜色，就要改成每 `(key, block, track)` 一段，段数会乘上有音符的 track 数；这是视觉与内存的取舍，不是当前实现。

## 3. AR 音符 LOD：保留 track，再做子像素合并

AR 不走 PR 的 `build_key_summary`，而是：

1. `YinModel::key_summary(key, block_ticks)`（`yinhe-core/src/lod.rs`）生成 `KeySummary`：
   - 每个 `(block, track)` 一段；
   - 段内 `start_tick` 取最小、`end_tick` 取最大、`velocity` 取最大；
   - `max_note_len` 记录该 key 最长音符，用于把查询左边界外扩。
2. `push_summary` 把每段按 track 送进 `TrackMerges`；
3. `TrackMerges` 再按「同 track 相邻段间隔 ≤1px」合并，保留 `min_start / max_end / max_velocity`。

与 PR 的区别：

- **track 保留**：不同 track 不会被合成一个颜色；每 track 一段。
- **块内仍是包络**：同一 track 同一块内，多个音符还是变成 `[min_start, max_end)`，不是代表音符。
- **额外 gap 合并**：即使 block 相同，相邻段如果间隔 ≤1px 还会继续合并；所以段数可能比 `KeySummary.segments` 更少。
- **hidden 回退**：AR 在 `hidden_notes` 非空时直接走原始层，避免「摘要段包含被隐藏音符但无法从包络里减掉」的问题。

## 4. AM 力度条 LOD：真正的「代表音符」

`VelocitySummary::build`（`yinhe-wgpu/src/automation/velocity_summary.rs`）：

- 8 档，`block_ticks` 同上；
- 每个 block **全局**（不分 key）；
- 过滤 `velocity > 1` 和 `track_visible`；
- 每个 block 只留一个代表：
  1. `gate = end_tick - start_tick` 最长者；
  2. gate 相同，取 velocity 大者；
  3. 还相同，取 `(velocity << 16) | track` 大者（即 track 大者）。
- 输出 `VelocityBlock { start_tick, end_tick, velocity, track }`，用的是代表音符的**真实区间**，不是块边界。

所以 AM 力度条是「吞掉除最长音符外的所有音符」，而不是包络：

- 一个短而响的音符会被一个长而轻的音符吞掉；
- 长音符的 bar 仍然显示真实长度，长度指示 gate 的语义保留；
- 同块内其他音符完全不可见。

例子（block 256）：

```text
A: [100, 110)  vel 90   gate 10
B: [120, 320)  vel 40   gate 200
C: [300, 310)  vel 70   gate 10
D: [400, 410)  vel 100  gate 10
```

- block 0（A、B）：代表是 B（gate 200），输出 `[120,320)`、vel 40；A 被吞。
- block 1（C、D）：gate 都是 10，D 的 vel 100 > C 的 70，代表是 D，输出 `[400,410)`、vel 100；C 被吞。

选档：`level_for_ppu` 先按 `select_summary_level` 选最大块；如果该档为空，向更细档找非空；都没有则回退原始力度条路径。

桌面和移动端都用这套 CPU 摘要（`set_velocity_summary_shared`），和桌面 GPU cull 的 8 档不是同一份数据。

## 5. AM 自动化曲线：没有 LOD

CC / PitchBend / RPN / NRPN / Tempo 的曲线没有摘要层：

- `data_lines.rs` 用 `lane.events_in_range(pad_start, pad_end)` 取视口内事件；
- 每个事件/段都画，不做块聚合；
- 只有视口裁剪，没有「吞音符」。

如果未来千万级自动化事件在同一视口内，才会需要考虑曲线 LOD；目前不在范围内。

## 6. 平台差异

| 路径 | 用的 LOD | 常驻档位 |
|---|---|---|
| 桌面 GPU cull | PR 音符 LOD（跨 track 包络） | 8 档全部常驻（用户已定） |
| 桌面 CPU fallback / 移动端 PR | `NoteLayerCache` 按当前 ppu 构建一个 block 宽度的摘要窗口 | 不常驻 8 档，窗口内一份 |
| 桌面 / 移动 AM | `VelocitySummary` 8 档 CPU 摘要 | 8 档常驻（CPU，量小） |
| 桌面 / 移动 AR | `KeySummary` + `TrackMerges` | 按当前 block 宽度按需构建/缓存 |
| 桌面 / 移动 AM 曲线 | 无 LOD | — |

## 7. 实测摘要段数

`yinhe-tests/examples/memory_probe.rs summary <mid|yin>`，8 档合计（all + visible + args）：

| 工程 | 音符 | 轨数 | tick_length | 8 档合计 | 原始层 | 摘要/原始 |
|---|---:|---:|---:|---:|---:|---:|
| A[BlackMidi] Wooden | 159,745 | 17 | 2,392,320 | 8.9MB | 2.5MB | 356% |
| Ouranos | 24,337,991 | 136 | 1,360,080 | 73.5MB | 375MB | 20% |
| ReptilianDarkRitual | 40,000,000 | 132 | 510,720 | 49.0MB | 616MB | 8% |
| start.mid | 164,203,965 | 799 | 3,093,150 | 437.3MB | 2530MB | 17% |

结论：

- 音符越密、tick 范围越短，摘要相对原始层越小；40M 只有 8%。
- start.mid 的 8 档摘要 437MB，原始层 2.53GB，约占 17%；桌面端可以接受，8 档常驻合理。
- start.mid 加载 + raw + 8 档摘要构建耗时 16.05s，峰值 RSS 5.34GB；移动端 4~6GB 不适合 1.64 亿工程。
- 小工程摘要可能比原始层大，但绝对值只有几 MB，不是问题。

## 8. 需要你确认的取舍

1. PR 音符 LOD 跨 track 合并，会在缩小视图里丢掉非主导 track 的颜色。你是否接受？如果要求每 track 保留颜色，段数会乘上有音符的 track 数，内存和 draw 量都会上升。
2. AM 力度条用「gate 最长」而不是「velocity 最大」，这是为了让 bar 宽度仍表示音符长度。如果更希望「最响优先」，规则要改。
3. AM 自动化曲线目前没有 LOD；如果以后自动化事件达到千万级，需要单独设计。

## 9. 代码索引

- PR 摘要：`crates/yinhe-wgpu/src/pianoroll/summary.rs`
- PR 档位：`SUMMARY_BLOCK_TICKS` / `select_summary_level`
- AR 摘要：`crates/yinhe-core/src/lod.rs`（`KeySummary` / `NoteSummary`）
- AR 合并：`crates/yinhe-wgpu/src/arrangement/instances.rs`（`push_summary` / `TrackMerges`）
- AM 力度摘要：`crates/yinhe-wgpu/src/automation/velocity_summary.rs`
- AM 力度条：`crates/yinhe-wgpu/src/automation/velocity_bars.rs`
- AM 曲线：`crates/yinhe-wgpu/src/automation/data_lines.rs`
- 可见窗口：`crates/yinhe-wgpu/src/note_cache.rs`
- 桌面档位选择 / 回退：`crates/yinhe-wgpu/src/renderer.rs`
