调查完成。我已通读 yinhe-audio crate 全部 13 个源文件（含 engine_tests.rs 全部 915 行），并交叉验证了 yinhe-core 的 tempo_map 实现和 Cargo.toml 的 feature 配置。以下是完整报告，所有结论都有具体文件和行号支撑。

---

# yinhe-audio 代码质量与性能调查报告

## 0. 总体架构速览

线程模型（先回答第 3 点，因为它是理解其余发现的基础）：

- **cpal 回调线程**（`spawn.rs:487-511`）：只做 `ring_consumer.pop_into(data)` + 3 个 atomic store。无分配、无锁、无 IO。这是唯一的硬实时线程，**实时安全性合格**。
- **audio-renderer 线程**（`audio_renderer.rs:83-95`）：事件循环 = 收 UI 命令 → 收 worker 结果 → 渲染 512 帧 chunk 推入 SPSC ring。空闲时 `thread::sleep(1ms)`（`audio_renderer.rs:92`）。
- **audio-worker 线程**（`spawn.rs:241-370`）：做重活——`PrepareModel`/`PrepareNotes`/`PrepareChase`/加载音色库，并做同类型命令合并。
- 通信：UI→renderer 用 `crossbeam bounded(16)`（满则丢弃+warn，不阻塞 UI，`spawn.rs:103-113`）；renderer→worker 用 unbounded；worker→renderer 用 `bounded(1)`；renderer→cpal 用无锁 SPSC ring（`audio_ring.rs`，2 的幂 + `wrapping_sub` 序号，reset 用 generation 计数避免竞态，`audio_renderer.rs:392-401`）。共享状态全部走 `AtomicU64/AtomicBool`（`RendererSharedState`，`audio_renderer.rs:18-27`）。**没有发现 Mutex/RwLock 出现在任何音频回调路径上**（全 crate 仅 export.rs 的 UI 进度 Mutex 和 soundfont.rs 的全局缓存 RwLock）。

---

## 1. 实时安全性

**cpal 回调路径：干净。** 无分配、无锁、无 println、无 unwrap。

renderer 线程（软实时，ring 解耦，但卡顿会导致 ring 抽空爆音）上的问题：

| # | 位置 | 严重程度 | 问题 |
|---|------|---------|------|
| 1.1 | `engine_render.rs:121` `self.active_notes.push(...)`；`:137` `self.ended_notes.push(*an)` | Minor | 每次 dispatch 都可能触发 Vec 增长分配。Vec 容量只增不减所以是摊还的，但黑乐谱首次进入高密度段时会有一串 realloc。可用 `reserve` 预分配。 |
| 1.2 | `audio_renderer.rs:288,291` `eprintln!` | Minor | 仅 GPU 初始化一次性路径，不在 per-frame 路径。可接受但生产代码建议换 tracing。 |
| 1.3 | `export.rs:282-285, 313, 351, 438` `eprintln!` | Minor | 生产导出路径里的调试计时打印，应改 `tracing::info!` 或删除。 |
| 1.4 | `engine_state.rs:92-99` `apply_chase_result` | Minor | 每次 seek 后在 renderer 线程一次性突发发送最多 256 通道 × ~20 条事件到 xsynth，黑乐谱快速拖拽 seek 时是 renderer 上的突发负载。 |
| 1.5 | unwrap/expect | — | **生产代码零 unwrap()/expect()**，全部在测试里（audio_model.rs 测试 9 处 expect、engine_tests.rs:654 unwrap）。`soundfont.rs:15,37,49` 的 `unwrap_or_else(\|e\| e.into_inner())` 是锁中毒恢复，处理得当。 |

---

## 2. 性能问题

### 2.1 【Major】`dispatch_and_find_next` 每个事件边界全量扫 128 桶 + 全量扫 active_notes — 高密度段 O(k×(128+V))

`engine_render.rs:33-57` 的渲染循环：block 内每个事件边界调用一次 `dispatch_and_find_next`，而每次调用都：
- 循环全部 128 个 key 桶（`:91`），即使绝大部分桶是空的；
- `active_notes.retain(...)` 全量遍历活跃音符（`:135-145`）。

设一个 512 帧 block 内有 k 个事件边界、V 个活跃 voice，单 block 成本 = **O(k·(128+V))**。黑乐谱 10ms 内数千个 note-on + 数千活跃 voice 时，这是典型的 O(n²) 行为。`active_notes` 的 NoteOff 检测本质是"按 end_sample 求 min"，用按 end_sample 排序的堆/桶可以把每次调用降到 O(log V)。

### 2.2 【Major】`seek_to` 对 seek 点前所有音符做线性扫描 — 1 亿音符目标下不可行

`engine_state.rs:198-227`：对每个 key 桶线性扫 `notes[..cursor]` 找跨 seek 点的音符。总成本 = **seek 点之前的全部音符数**。注释（`:196-197`）说"cursor 前通常有几十个跨点音符，O(cursor) 完全可接受"——这个假设对普通 MIDI 成立，但对 1 亿音符黑乐谱、seek 到曲子 90% 位置时就是 ~9000 万次迭代，且发生在 renderer 线程，ring 会抽空。另外 `apply_prepared_model`（`engine_state.rs:56`）每次 ReloadNotes 都会调 `seek_to(current)`，即**每次自动化编辑/undo/redo 都附带一次全前缀扫描**。需要按 end_sample 有序的结构（如每桶维护 end_sample 排序索引）把查询降到 O(log n + 跨点数)。

### 2.3 【Major 疑似 bug】export_wav 逐文件加载音色库会互相覆盖，只保留最后一个

`export.rs:119-131`：为了报进度，循环里每次只传 `paths[i..i+1]` 发 `LoadSoundFont`。而 `engine_state.rs:137-148` → `soundfont.rs:83` `self.port_sfs[port as usize] = soundfonts;` 是**整体替换**。一个 port 配多个音色库（分层）时，导出结果只剩最后一个音色库，与实时播放（`audio_renderer.rs:126-135` 一次性传整个 `paths`）声音不一致。如果"每 port 多 SF 分层"是支持的功能，这是正确性 bug；如果不支持，这个逐文件循环就是多余转换。

### 2.4 【Minor】voice 数量控制依赖 xsynth 内部，引擎侧无上限

引擎侧唯一的 voice 控制是 per-key layer count（`engine.rs:152-159` SetLayerCount）和 `fade_out_killing: true`（`engine.rs:75`）。`active_notes` 无上限增长。黑乐谱数十万同时发声时，实际渲染压力全丢给 xsynth 的 voice stealing，yinhe-audio 自己不做任何全局 voice cap 或提前剔除不可闻 voice 的策略——结合 2.1，V 越大 renderer 越慢，形成正反馈。

### 2.5 【Minor】`flatten_automation_to_cc_events` 的 Linear/Curve 展开量

`audio_model.rs:146-154`：density=1 时每 tick 一个中间事件。480 PPQ 下一条 10 秒的 Linear 自动化 lane 就展开成 ~数千条 CC，全部进 `cc_events` 参与排序（`:176` sort O(E log E)）和 chase 线性扫描（`spawn.rs:375-388`，每次 seek O(E)）。这是设计上的 trade-off（用空间换 renderer 简单），worker 线程做所以可接受，但 cc_events 规模会随自动化密度爆炸，值得你知道。

---

## 3. 线程模型与共享状态

见第 0 节。补充发现：

| # | 位置 | 严重程度 | 问题 |
|---|------|---------|------|
| 3.1 | `spawn.rs:246` worker 结果通道 `bounded(1)` | Minor | worker `result_tx.send` 在队列满时阻塞。当前 renderer 每轮循环都 drain，不会死锁，但这是隐式背压耦合，注释里没说明。 |
| 3.2 | `audio_renderer.rs:84-93` 忙等 + `sleep(1ms)` 轮询 | Minor | renderer 无工作时 1ms 粒度轮询，空闲时白白唤醒 ~1000 次/秒。可用阻塞 recv + 超时。不是正确性问题。 |
| 3.3 | reset_generation 机制（`audio_renderer.rs:23-26`、`spawn.rs:489-494`） | — | 设计良好：producer 不直接 clear ring，用 generation 让 cpal 回调自己 clear，避免了 read/write 指针竞态，注释也写清了原因。竞态方面未发现问题。 |
| 3.4 | `soundfont.rs:10-11` 全局 `RwLock<HashMap>` 音色库缓存 | Minor | 只在加载时（worker/renderer 非 per-frame）访问，不在回调路径，无实时性问题。但按你的规则 4（不喜欢缓存）：这是一个全局缓存 + `sweep_unused` 手动 GC（`soundfont.rs:14-17`），是否保留由你决定。 |

---

## 4. export.rs 与实时路径的重复

重复相当多，是违反 DRY 最明显的地方：

| 位置 | 严重程度 | 说明 |
|------|---------|------|
| `audio_renderer.rs:327-357` `build_gpu_synth_events` vs `export.rs:324-350` | **Major** | 两处几乎逐行相同的 GPU 事件构建逻辑（128 桶遍历→skip_track→dense_for→push on/off 事件→sort）。但数据源不同：renderer 用 `engine.audible_notes`（已预转 sample），export 用 `model.notes` + 现场 `tick_to_sample`——正是你规则 9 说的 A→B→C 多余转换：export 本可直接调 `build_audible_notes` 复用同一份代码。 |
| `export.rs:150-162` vs `export.rs:375-387` | Minor | WavSpec 构建在 export_wav / export_wav_gpu 里重复。 |
| `export.rs:219-274` vs `export.rs:410-432` | Minor | Phase 2 余韵衰减循环 CPU/GPU 两份。 |
| `export.rs:86-87` vs `audio_renderer.rs:13-15` vs `spawn.rs:17` | Minor | `STEREO_CHANNELS`/`RENDER_CHUNK_FRAMES` 常量在 3-4 个文件各自定义（且 export 用 1024、renderer 用 512，不一致）。 |
| `engine_state.rs:15-36` `load_model` vs `prepare_model.rs:13-33` `prepare_model` | Minor | 同一份"AudioModel + cc_events + audible_notes + duration"组装逻辑写两遍（同步版给 export/测试，异步版给 worker）。`duration_samples` 计算公式出现在三处（`engine_state.rs:24`、`prepare_model.rs:22`、`prepare_model.rs:44-45`）。 |
| `engine.rs:183-188` `UpdateNotes` 分支退化为 `load_model` | Minor | 注释自承是"测试路径"的退化 fallback：全量重建 cc_events。如果异步 `PreparedNotes` 路径已验证可靠，这个同步 fallback 就是你规则 1 想删的那类东西（export 走的是 LoadModel，不依赖它）。 |

---

## 5. channel.rs vs channel_layout.rs

**职责不同，无重复，命名容易混淆但内容是干净的：**

- `channel_layout.rs`（499 行，其中测试 ~340 行）：源通道 0..256 → xsynth 压缩通道的不可变映射（`active_mask` + `channel_map`），以及 `differs_from_counts` flip 检测（决定何时 teardown 重建引擎）。测试覆盖很好。
- `channel.rs`（313 行，其中测试 ~95 行）：单通道控制器状态快照 `ChannelState`，专供 seek 后 chase 恢复（volume/pan/program/RPN 解析/cc_values[128]）。RPN resolve 逻辑（`:122-144`）有完整单测。

唯一可挑剔的：`ChannelState::default()` 的默认值与 xsynth 内部默认值是**手工对齐**的（`channel.rs:6-10` 注释自承），xsynth 升级改默认值时会静默失配——这是跨 crate 的隐式耦合，没有测试能守住。

---

## 6. 错误处理

**生产代码零 unwrap/expect/panic**，符合你的规则 17。全部 panic 路径在测试中：
- `audio_model.rs:346,347,392,393,435,436,490,491,492`（9 处 expect，测试）
- `engine_tests.rs:654`（ignored 测试里的 unwrap）

错误处理做得好的地方：`spawn_worker` 返回 `Result` 而非 expect（`spawn.rs:239-241`）；`AudioHandle::send` 通道满/断开只 warn 不 panic（`spawn.rs:103-113`）；cpal 流错误用 atomic 标志通知 UI（`spawn.rs:512-515`）。

---

## 7. engine_tests.rs 测试覆盖评估

覆盖了的：dispatch 精确采样点（:76）、vel≤1 过滤（:102,:180）、tempo 变速下 per-key 游标（:198）、Play/Pause/Stop/Seek 命令（:280-305）、ReloadNotes 重建 cc_events 回归（:443）、layout flip → teardown 重建的 3 个 Document 级集成测试（:715,:760,:803,:864）、并行配置 benchmark（:562）。

**明显缺口**（按你的规则 18，屡次出 bug 的地方应有测试）：
1. **seek 跨点音符重启（P2-10 修复，`engine_state.rs:195-227`）没有测试**——代码里有"修 P2-10"注释说明这里出过一次 bug，但没有对应回归测试。
2. **chase generation 过期丢弃**（`audio_renderer.rs:258-264`）无测试——这是并发逻辑，最容易回归。
3. **`apply_notes_only` 的 note_cursor 重算**（`engine_state.rs:83-86`）无测试。
4. **export_wav 完全无测试**——包括上面 2.3 的疑似 SF 覆盖 bug，有测试就能抓到。
5. `bench_parallelism_configs`（`engine_tests.rs:562`）没有 `#[ignore]`，每次 `cargo test` 都渲染 3 配置 × 2 遍 × 2 秒音频，拖慢测试套件；建议转 criterion bench 或加 ignore。

---

## 8. 上帝对象评估：AudioEngine

`engine.rs:19-58` 共 **20 个字段**，混了 4 类职责：

1. 合成器包装：`channel_group`、`sf_manager`、`gpu_synth`、`channel_layout`
2. 走带状态：`sample_position`、`playing`、`duration_samples`、`pending_play_from_sample`
3. 模型数据：`yin_model`、`model`、`audible_notes`、`cc_events`、`skip_track`
4. dispatch 游标/缓冲：`note_cursor`、`cc_cursor`、`active_notes`、`ended_notes`、`chase_generation`、`automation_density`、`sample_rate`

缓解因素：方法已经按职责拆到三个文件（engine.rs 命令/构造、engine_state.rs 模型应用/seek、engine_render.rs 渲染/dispatch），字段全是 `pub(crate)` 但没有跨 crate 泄漏。还没到失控的程度，但如果要拆，自然的边界是：**`PlaybackData`（职责 3+4，每次 prepare 整体替换）** 和 **`SynthCore`（职责 1）**，走带状态留在 Engine。这样 `apply_prepared_model`/`apply_notes_only` 就是换一整个子结构，字段级替换的 8 处赋值（`engine_state.rs:40-49`）会收敛成一两行。

---

## 总结

### 屎山指数：35 / 100

注释质量高、线程架构清晰、cpal 回调干净、生产代码无 unwrap、关键 bugfix 都有注释溯源，这些把分数拉住了。扣分主要在：export 与实时路径的大面积重复、两个黑乐谱场景下的复杂度地雷、以及测试盲区恰好落在出过一次 bug 的 seek 逻辑上。

### Top 5 最严重问题

1. **【Major】export_wav 逐文件加载音色库互相覆盖**（`export.rs:119-131` + `soundfont.rs:83`）：多 SF 分层时导出只剩最后一个 SF，导出与实时播放声音不一致，且无测试能抓到。
2. **【Major】`seek_to` 对 seek 点前全部音符线性扫描**（`engine_state.rs:198-227`）：与 1 亿音符目标直接冲突；且每次 ReloadNotes（undo/redo/自动化编辑）都经 `apply_prepared_model` 附带一次。注释里的"几十个跨点音符"假设在黑乐谱下不成立。
3. **【Major】`dispatch_and_find_next` 每事件边界 O(128 + V_active)**（`engine_render.rs:33-57, 91, 135-145`）：高密度段退化为 O(k×V)，黑乐谱同时发声越多 renderer 越慢，正反馈式爆音风险。
4. **【Major】GPU 事件构建逻辑两处重复且 export 版做了多余的 tick→sample 转换**（`audio_renderer.rs:327-357` vs `export.rs:324-350`）：违反 DRY + 规则 9，应统一走 `audible_notes`。
5. **【Major】P2-10 修复（seek 跨点音符重启）无回归测试**（`engine_state.rs:195-227`）：按你的规则 18，出过 bug 的地方必须有测试；同时 chase generation、apply_notes_only、export 全链路也都是测试空白。

### 顺带提醒（规则 2/3/12）

- 复用代码你已知的两份（第 4 节表格）之外，`duration_samples` 计算一式三份（`engine_state.rs:24` / `prepare_model.rs:22` / `prepare_model.rs:44-45`），值得收敛。
- `engine.rs:183-188` 的 `UpdateNotes → load_model` 同步退化路径、以及 `engine.rs:189-191` 的同步 `LoadSoundFont` 路径，都是异步 worker 路径之外的旧式 fallback——如果异步路径已稳定，这两个分支是你可能想删的。
- 本次为纯只读调查，未运行构建，所以未能核对既有编译警告；如需我跑一遍 `cargo check -p yinhe-audio --all-features` 收集警告清单，说一声即可。
        
          
最后核实几个疑似死代码的符号使用情况。
        
            
toolName: search_by_regex
            
status: failed
          
query: PrepareTimings|instance_count\(\)|hash_f64s|build_static|render_block\(
          

        
            
toolName: search_by_regex
            
status: failed
          
query: GpuSynth::|GpuSynth\b
          
补最后两个细节：automation_panel 的返回值用途与 render_context.paint 的调用方式。
        
            
toolName: view_files
            
status: failed
          
            
filePath: /Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/piano_view/automation_panel.rs
          

        
            
toolName: view_files
            
status: failed
          
            
filePath: /Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/render_context.rs
          
调查完成，我已通读全部目标文件及其测试、并交叉验证了调用方（yinhe-core、yinhe-wgpu、yinhe-editor-core）。以下为完整报告。

---

# 代码质量与性能调查报告

## 1. yinhe-mid2（parser.rs / writer.rs / encoding.rs / error.rs）

### 解析性能特征

**事件存储**：解析产出 `Vec<NoteEvent>`（per-track），再由 `YinModel::load_track_notes`（yinhe-core/src/model_stats.rs:26）按 key 分桶到 `[Arc<Vec<Note>>; 128]`，分桶前精确预计数容量——这部分设计良好，无重复分配。`Note` 为 `#[repr(C)]` 16 字节，1 亿音符约 1.6 GB，合理。

**发现的问题：**

- **[Major] 整个事件流被完整解码两遍** — parser.rs:66 `collect_conductor(track_iter.clone())` 先扫一遍全部事件收 tempo/time-sig；parser.rs:75-76 再 clone 迭代器扫第二遍做音符解析。midly 是惰性解析器，clone 不复制数据但意味着每个字节的事件解码工作做两次。几百 MB 的黑乐谱文件，解析 CPU 时间直接翻倍。conductor 事件完全可以在单遍 parse_track 里顺带收集（tempo/time-sig 是 Meta 事件，各 track 的 parse 已在 match 它们）。
- **[Major] writer.rs 的 O(T×N) 扫描** — writer.rs:82-105 `build_track` 对每条 track 遍历全部 128 个 key 桶并 `filter(|n| n.track == track_idx)`。T 条 track、N 个音符就是 O(T×N)。**讽刺的是 yinhe-yin/io.rs:76 的注释明确写着 "Build per-track notes in a single O(N) pass instead of O(T×N)"**——同一个优化在 yin 里做了，mid2 writer 还留着旧版低效实现（符合你规则 3，建议直接按 io.rs:78-91 的模式重写）。
- **[Major] `handle_cc38` 的 rfind 是潜在 O(n²)** — parser.rs:557-560 和 579-582：每来一个 CC38 就在整个 `auto_events` Vec 里 `rfind` 找同 target 同 tick 的事件。RPN/NRPN LSB 密集的文件（如细粒度 Fine Tune 自动化）会退化成 O(n²)。修起来很简单：CC38 只需检查 Vec 尾部少量元素，或记录每个 target 最后写入的索引。
- **[Major] `ensure_conductor_track` 对全部音符做 O(n) 重写 + 逻辑重复** — parser.rs:150-154 对每个 key 桶 `Arc::make_mut` 后逐音符 `n.track += 1`，1 亿音符就是 1 亿次写（且若 Arc 被共享会触发整桶克隆）。另外 parser.rs:137 注释自认 "replicates the logic from `Document::from_model`"——同一段"插入 conductor 轨并平移索引"逻辑存在两份（规则 2，建议告诉你：两处复用代码在 parser.rs:139-163 与 yinhe-editor-core 的 Document::from_model）。
- **[Minor] NoteOff 配对的 rposition 扫描** — parser.rs:615-618 `active.iter().rposition(...)`，每个 NoteOff 扫一遍活跃音符栈。黑乐谱同 key 大量叠音时活跃栈可能很长，最坏 O(n·k)。通常 k 小，可接受；但同 key 叠万级的极端黑乐谱会慢。
- **[Minor] 悬空 NoteOn 被静默丢弃** — 文件结束未配对的 NoteOn 直接消失（active_notes 不 flush），不产生半音符也不报警。对损坏文件是数据静默丢失。
- **[Minor] `group_automation_events` 逐事件 clone** — parser.rs:484-487 排序后按 lane 分组时 `e.clone()` 逐元素复制，其实可以按区间 `drain` / 切片移动，省一遍拷贝。
- **[Minor] writer.rs:40 `Vec::new()` 无容量** — 写几百 MB 输出时反复 realloc；可预估容量。writer.rs:35 `u15::new(ppq as u16)`：ppq 是 u32，>65535 时 `as u16` 截断，>32767 时 u15 debug 下会 panic（违反你规则 17 的精神，虽是边缘情况）。
- **无逐字节 push**：文件一次性 `std::fs::read` 后切片解析，OK。

**error.rs（9 行）**：仅 `Io` + `Parse` 两个变体，简洁够用，设计无问题。encoding.rs 是 33 种编码的纯映射表，整洁。

## 2. yinhe-yin（container / io / mapping / project_meta）

设计整体清晰：3 段 length-prefixed 容器，project.json/mapping.json 明文可读，data 段 zstd+bincode。io.rs 的 per-track notes 收集已是 O(N) 单遍（io.rs:76-91），比 mid2 writer 先进。

- **[Major] 全量内存驻留，无流式** — load 路径：`fs::read` 全文件 → `zstd::decode_all` 全量解压 → `bincode::deserialize` 全量反序列化（io.rs:186-187）。峰值内存 ≈ 压缩文件 + 解压后 bincode（1 亿音符 ~1.6 GB）+ 最终模型，约 3-4 倍。save 路径同理（io.rs:118-120 先 bincode 全量再 zstd 全量）。对 1 亿音符目标，这是最大的内存瓶颈点；bincode 支持 `deserialize_from` 流式读取，zstd 也支持流式，值得考虑。
- **[Minor] unpack 对每段 `.to_vec()`** — container.rs:41-45，data 段（最大的一块）多拷贝一次。可改为借用切片。
- **[Minor] mapping.rs 的 A→B→C 双重转换**（规则 9）— mapping.rs:80 先建 `BTreeMap<(u8,u8), Vec<TrackMap>>`，再在 93-96 行转成嵌套 `BTreeMap<u8, BTreeMap<u8,...>>`。直接往嵌套 map 里插即可，省一层中间结构。
- **[Minor] save 时 clone 全部 automation/PC** — io.rs:106-107 `t.automation_lanes.clone()` / `t.program_change.clone()`，自动化极多的工程会整量复制一遍。
- mapping.rs 的职责：`mapping.json` 的音轨树（port→channel→track 元数据 + 音色库路径 + 视图状态），让 UI 不解压 data 段就能画音轨列表。设计意图良好。
- 错误处理（error.rs）变体齐全（BadMagic/BadVersion/Truncated/Json/Bincode），无 unwrap。

## 3. yinhe-types（automation.rs / view 文件）

- **automation.rs 查询效率良好**：`AutomationLane.events` 按 tick 排序 + `partition_point` 二分（automation.rs:319-323），范围查询 O(log n + k)。554 行里一半是测试，测试质量高（贝塞尔数值断言都齐全）。
- **[Minor] AutomationEvent 偏大**：`SegmentShape::Curve` 带 4 个 f32，枚举约 20 字节，整个事件约 28 字节；占绝对多数的 `Step` 事件浪费 16 字节。自动化点极多时（千万级）内存可观，可考虑 shape 分离存储——但目前自动化量级远低于音符，优先级低。
- **view_base / piano_roll_view / arrangement_view 之间基本无重复**：`TimelineViewBase` 抽取得很干净，两个视图都是薄委托。这值得肯定。
- **[Minor] arrangement_view 的 static/instance 双 API 重复** — arrangement_view.rs:63-84 `visible_track_range` 与 `visible_track_range_static`、`lane_y` 与 `lane_y_static` 是两两重复的 API（实例版只是转发）。
- **[Major/规则2] TRACK_PALETTE 完全重复定义，且 yinhe-theme 那份是死代码** — yinhe-types/src/palette.rs:4 与 yinhe-theme/src/palette.rs:4 是逐字节相同的两份 16 色调色板；而全 workspace 只有 `yinhe_types::TRACK_PALETTE` 被使用（yinhe-editor-core/document.rs:6、yinhe-wgpu/automation/data_lines.rs:3），yinhe-theme 的 re-export（lib.rs:7）无人引用。建议删 yinhe-theme 那份。
- **[Minor] hash.rs 的 hash_f64s / hash_f32s / hash_bools 三份复制粘贴**（hash.rs:32-56），循环体完全相同，可泛型化。
- **[Minor] arrangement_view.rs 没有任何测试**，而兄弟文件 view_base / piano_roll_view / automation_panel_view 都有充分测试——坐标换算这种容易出 off-by-one 的地方反而裸奔。

## 4. yinhe-archive（lib.rs 377 行）

职责：多格式压缩包读取（zip 惰性随机访问；7z/tar 打开时全量解压进内存 HashMap）。

**unwrap 澄清：20 处 unwrap 全部位于 `#[cfg(test)] mod tests`（lib.rs:310-373），测试里 unwrap 是惯例，无风险。** 生产代码 unwrap 为零。关键风险位置：

- **[Minor] lib.rs:199 `_ => unreachable!()`** — 生产代码唯一的 panic 宏。当前由 match 结构保证不可达，安全，但将来给 `Format` 加变体时会变成真炸弹；可用编译期穷尽匹配代替（把 Tar 分支从该函数分离）。
- **[Minor] Memory 变体 read_file 整量克隆** — lib.rs:107-110 `.cloned()`，tar/7z 里读一个大 MIDI 就全量复制一次。
- **[Minor] 7z/tar 打开即全量解压所有 MIDI 文件**（lib.rs:155-167、181-215），tar-xz 还要先把压缩文件整个读进内存再解压（191-194）——大压缩包内存峰值高。属于设计取舍，但值得知晓。
- **[Minor] 测试只覆盖 zip**，sevenz/tar/tar-xz 路径零测试。

## 5. yinhe-memtrace

是调试/诊断工具，设计得相当克制：

- **feature gating 正确**：`memtrace` feature 关闭时 `TaggedAlloc` 是后端分配器（jemalloc/mimalloc）的零开销薄转发（lib.rs:352-369），`with_tag` 是 no-op（lib.rs:247-250）。生产无开销。
- **perf_probe.rs 不是编译期 debug-only，而是运行时 `YIN_PERF=1` 环境变量门控**（perf_probe.rs:44-53，OnceLock 缓存）。关闭时每次调用的成本是一次 OnceLock 原子读，可忽略；`submit` 用 try_lock 不阻塞渲染线程。设计良好。
- **[Minor] 分配标签不会传播进 rayon 线程** — parser.rs:38/55 的 `with_tag` 设置的是**调用线程**的 thread_local（lib.rs:224-226），而 parse_track 在 rayon worker 线程上跑（parser.rs:84），那些线程的分配会全部计入 `Unknown`。memtrace 对 MIDI 解析的归因因此基本失效。
- lib.rs 里 unsafe 分配器代码（带 header 的 TaggedAlloc）有对应单元测试（含 feature-gated 的 round_up/user_offset 边界测试），质量可以。

## 6. yinhe-dms / yinhe-haptic / yinhe-theme / yinhe-test-helpers

- **yinhe-dms（lib.rs 共 10 行）**：纯占位符，`from_bytes` 永远返回 `Err("DMS parsing not yet implemented")`，注释"别删，以后我一定会实现的"。被 yinhe-egui 声明为依赖。**当前是死代码**，但属有意保留——建议你确认是否继续留。
- **yinhe-haptic**：macOS 私有 `MultitouchSupport.framework` MTActuator 驱动（已在 macos.rs:10-11 注释声明过不了 App Store）。槽位/边界状态机清晰，无非 Mac 残留。无测试（平台私有 API，情有可原）。macos.rs:89-95 的 0.34/0.67 强度阈值是把连续 intensity 映射到 3 个离散波形 ID，属于必要的离散化，不算你讨厌的那种阈值分支。
- **yinhe-theme**：GpuTheme + egui_colors 正常被 wgpu/egui 使用；唯一问题是上面的死 TRACK_PALETTE。
- **yinhe-test-helpers**：`MockMidi::duration()` 硬编码返回 `10.0`（lib.rs:20），与 tick_len 字段无关，是个隐形魔法常量；lib.rs:123 有一个 `.expect()`（测试辅助 crate，可接受）。`make_test_model` 与 mid2/yin 两个 roundtrip 测试里的 `build_complex_model` 结构高度相似，存在三份近似拷贝（test-helpers/lib.rs:56、mid2/tests/roundtrip.rs:70、yin/tests/roundtrip.rs:11）——可以考虑让两个 roundtrip 测试改用 test-helpers。

## 7. 测试覆盖总览

| crate | 测试情况 |
|---|---|
| yinhe-mid2 | tests/roundtrip.rs：7 个测试（最小解析、roundtrip、复杂模型、重叠音符 id、RPN 解码、port/channel prefix）。**缺：大文件压力/性能测试、fuzz、writer 独立测试** |
| yinhe-yin | tests/roundtrip.rs：15+ 测试，含 10 万音符压缩率断言、坏 magic/截断/版本拒绝、SF 持久化——**覆盖最好** |
| yinhe-archive | 3 个单元测试，仅 zip 路径 |
| yinhe-types | automation / view_base / piano_roll_view / automation_panel_view / time_format 有充分单测；**arrangement_view、hash、source、note、palette 无测试** |
| yinhe-memtrace | 有单测（部分 feature-gated） |
| yinhe-theme / yinhe-dms / yinhe-haptic / yinhe-test-helpers | **完全无测试**（theme 是纯常量可豁免，dms 是占位符，haptic 是平台私有 API） |

## 8. 生产代码 unwrap()/expect() 清单

这 9 个 crate 的 **src 生产代码中 unwrap/expect 为零**——这点做得很好。全部命中项：

- yinhe-archive/src/lib.rs:199 — `unreachable!()`（生产，低风险，见上）
- yinhe-archive/src/lib.rs:310-373 — 20 处 unwrap，全部在 `#[cfg(test)]` 内
- yinhe-test-helpers/src/lib.rs:123 — `.expect("from_model failed")`（测试辅助 crate）
- 间接风险：yinhe-mid2/src/writer.rs:35 `u15::new(ppq as u16)` 及多处 `u7::new` —— midly 的 checked 构造器在 debug 下越界 panic（writer 已做 `& 0x7F` 掩码，仅 ppq 一处未防护）

---

# 屎山指数评分

| crate | 屎山指数 | 点评 |
|---|---|---|
| yinhe-mid2 | **45** | 双遍解析、writer O(T×N)（且 yin 已有更优实现）、conductor 平移逻辑重复，是本次调查问题最集中的 crate |
| yinhe-yin | **25** | 设计清晰、错误处理规范、测试最好；扣分在全量内存驻留与少量多余拷贝 |
| yinhe-types | **20** | 基础扎实（排序+二分、TimelineViewBase 抽取干净）；扣分在 palette 重复、static 双 API、arrangement_view 无测试 |
| yinhe-archive | **30** | 生产无 unwrap 值得肯定；扣分在 tar/7z 全量入内存设计与非 zip 路径零测试 |
| yinhe-memtrace | **15** | feature 门控教科书级；仅 rayon 标签不传播一处归因失真 |
| yinhe-theme | **25** | 主体干净，但藏着一份无人使用的 TRACK_PALETTE 死代码 |
| yinhe-haptic | **20** | 小而聚焦，私有框架风险已自觉声明 |
| yinhe-test-helpers | **20** | 基本合格，硬编码 duration=10.0 与三份模型构造重复是小瑕疵 |
| yinhe-dms | **N/A（占位符）** | 死代码但有意保留，等你拍板 |

# Top 5 最严重问题

1. **[Critical] yinhe-mid2 writer.rs:82-105 — O(T×N) 音符扫描**。每条 track 全量扫 128 桶 filter；yinhe-yin/io.rs:76-91 已有 O(N) 单遍实现可照搬。多轨黑乐谱导出 MIDI 时这是数量级差距。
2. **[Major] parser.rs:66 + 75 — 事件流双遍解码**。几百 MB 文件解析 CPU 直接翻倍；conductor 收集可并入 parse_track 单遍完成。
3. **[Major] parser.rs:139-163 `ensure_conductor_track` — O(1亿) 音符逐条重写 + 与 Document::from_model 逻辑重复**（注释自己承认 replicate）。既慢又违反 DRY。
4. **[Major] parser.rs:557-560、579-582 — CC38 的 `rfind` 全 Vec 扫描**，RPN/NRPN-LSB 密集文件退化为 O(n²)。
5. **[Major] TRACK_PALETTE 双份定义**（yinhe-types/src/palette.rs:4 与 yinhe-theme/src/palette.rs:4），yinhe-theme 那份全 workspace 无人使用，属可直接删除的死代码。

另按你的规则 2 汇报两处复用代码：conductor 插入逻辑（parser.rs:139 ↔ Document::from_model）、测试模型构造（test-helpers/lib.rs:56 ↔ mid2/tests/roundtrip.rs:70 ↔ yin/tests/roundtrip.rs:11）。本次调查未执行构建，故无编译错误/警告可报；如需我跑一遍 `cargo test -p` 这些 crate 验证现状，说一声即可。
        
          
调查完成。我已通读全部 14 个重点文件，并抽查了 `dialogs/`（rescale_overlay、export、dialog_dispatch）、`piano_view/drag.rs`、`right_panel.rs`、`view_interaction.rs`、`file_loader.rs`、`poll.rs` 等，以下结论覆盖全 crate，无遗漏项。

---

# yinhe-egui 代码质量与性能调查报告

## 1. 每帧热路径性能

### [Critical] event_browser 音符详情页每帧 O(N) 全量扫描 + 排序
**文件**: `/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/right_panel/event_browser.rs:452-471`
当用户在事件浏览器选中某轨道的 "Notes" 时，`show_event_detail` **每一帧**都遍历全部 128 个 key bucket、filter 出该轨音符、push 进新 Vec、再 `sort_by_key`：
```rust
let mut notes: Vec<(yinhe_core::NoteEvent, u8, u16)> = {
    let mut v = Vec::new();
    for (key, bucket) in model.notes.iter().enumerate() {
        for n in bucket.iter().filter(|n| n.track == *track) { v.push(...) }
    }
    v
};
notes.sort_by_key(|(n, _, _)| n.start_tick);
```
表格行渲染是虚拟化的（`build_table` 用 `body.rows` 只画可见行），但**数据收集不是**。1 亿音符的工程打开这个面板会直接卡死 UI 线程（每帧全堆分配 + 排序）。同文件 CC/PB/PC 分支（489-547 行）也有每帧 collect + sort，量级小一些但也应只在 revision 变化时重建。

### [Major] 每帧克隆整条音轨的 automation lanes
**文件**: `/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/app/layout.rs:290-308`
`show_pianoroll_split` 每帧执行 `t.automation_lanes.clone()`（`.map(|t| t.automation_lanes.clone())`），把编辑轨所有 CC/PB 事件 Vec 完整深拷贝一份，只为传给 `piano_view::show` 的 `&[AutomationLane]`。高密度 CC 的黑乐谱工程下这是每帧 O（事件数） 的堆分配。应传引用或用 `Arc` 共享。

### [Major] event_browser 树每帧重建 + 字符串格式化
**文件**: `/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/right_panel/event_browser.rs:142-143, 226-403`
`BarLookup::build`、`ts_changes`（Vec 分配）、`group_tracks_by_port_channel`（BTreeMap 分配）每帧重建；`render_track_row` 对展开的每个轨道每帧遍历其 automation_lanes 构建 `cc_map` 并 `format!` 4-5 个字符串（323-329 行）。树结构只在 `doc.data.revision` 变化时才需要重建，现在指纹只用于清空 expanded_keys，没有用于跳过重建。

### [Minor] tempo 上限每帧全 lane 扫描（两处重复）
**文件**: `piano_view/automation_panel.rs:291-297` 和 `piano_view/automation_panel/interaction.rs:229-233`
`tempo_lane.events.iter().map(|e| e.value).fold(0.0, f32::max)` 每个可见面板每帧各算一遍，且两处是复制粘贴的同一段逻辑。tempo 事件多时（黑乐谱变速曲）是每帧 O(tempo 事件数）×面板数。

### [Minor] 钢琴键盘每帧 3×128 次循环 + format! 分配
**文件**: `piano_view.rs:559-651`
黑键行、白键、黑键三轮 `for key in 0u8..128` 每帧执行（有视口裁剪，属常数级），但 `format!("C{}", octave)`（621 行）每帧为每个可见 C 键分配字符串。属常数开销，可接受，但 128 键的几何完全可以交给 wgpu 装饰层一次画好。

### [Minor] AR 拖拽 ghost 每帧 128 key × range 查询
**文件**: `arrange/view_ui.rs:577-602`
AR 移动拖拽中每帧遍历 128 个 key 做 `key_notes_in_range`。PR 的对应实现（`piano_view/drag.rs:215-237`）是在拖拽开始时预计算一次 `drag_notes` 存 egui data，AR 没有沿用这个更优方案——同一问题两种实现，建议统一到 PR 的方式。

### 每帧 O(1)~O(tracks) 的正常项（无问题）
- `layout.rs:249-256` `pr_visible` 每帧按轨道数重建（O(tracks)，合理）。
- `layout.rs:274-285` `ts_events` Vec 每帧重建（拍号事件通常很少，Minor）。
- `piano_view.rs:914-921` `total_notes` 全 128 bucket 求和——仅在 `YIN_PERF=1` 时，无影响。

---

## 2. 代码重复 / 遗留旧实现

| 严重程度 | 位置 | 问题 |
|---|---|---|
| Major | `widgets/scrollbar.rs:32-183 / 202-356 / 370-508` | `show` / `show_vertical` / `show_vertical_value` 三个函数各约 150-170 行，背景/thumb 绘制、三区 interact、cursor 逻辑几乎是逐行复制，仅坐标轴和缩放语义不同。约 500 行里至少 300 行是重复。 |
| Major | `app/actions.rs:560-650` | `export_wav` 的参数列表（model/sr/port_sf/skip/path/bit_depth/layer_count/progress回调/progress/cancel）复制粘贴了 **3 次**（GPU fallback CPU、纯 CPU、`cfg(not(gpu))`），progress 回调闭包一字不差重复 3 份。 |
| Major | `app/layout.rs:412-497` | `handle_automation_edits` / `handle_note_drag` / `handle_pencil_note_drag` / `handle_arr_drag` 四个函数重复同一段 "构造 UndoEntry + push + dirty + notify" 模板，而 `actions.rs:198` 已有现成的 `with_undo` 抽象没被复用。 |
| Major | `dialogs/rescale_overlay.rs` 全文 | 文件头注释自认"参考 export::show_progress_viewport 的模式"——两个进度条 viewport 是复制粘贴（progress.lock → ProgressBar → 取消 → cancel.store）。应抽一个通用 progress viewport。 |
| Minor | `view_interaction.rs:288-356` | `snap_tick` / `snap_tick_ceil` / `snap_tick_floor` 三个函数除一行外完全相同（A→B→C 式重复，应用闭包参数合成一个）。 |
| Minor | `render_context.rs:70-154` | `new()` 与 `from_render_state()` 后半段（device_lost 回调注册 + create_target + 结构体初始化）完全重复，且这个重复直接导致了下面第 4 条的 device-lost 回调互相覆盖 bug。 |
| Minor | `app/poll.rs:12-16 vs 37-41` | quantize preset 读取块复制粘贴两次；两种 LoadResult 的 "push doc + set active + teardown_audio" 也重复。 |
| Minor | 死代码 | `app/actions.rs:277`：`FileAction::Open` 在 264 行已被提前 return 拦截，此分支永远不可达（配合 282 行的 `unreachable!()`）；`app/actions.rs:110-120`：`duplicate_selected_notes` 里 `pending_delta = action.as_ref().and_then(|_| { None })` 恒为 None，是一段没写完的死逻辑；`event_browser.rs:313` `let _child_count`、`:450` `let _t` 死变量；`arrange/view_ui.rs:41` `_track_names` 死参数；`interaction.rs:65-68` `hit_line_on_lane` 带 4 个未使用参数。 |
| Minor | `app/poll.rs:101` | `let ctx = egui::Context::default();` 造一个**假的** egui Context 传给 `execute_pending_file_action`，而后者签名是 `_ctx` 根本不用。应直接删掉这个参数。 |

---

## 3. 上帝对象

### [Major] `piano_view::show` — 950 行、约 50 个参数的上帝函数
**文件**: `piano_view.rs:38-987`。参数 38-88 行连绵 50 行，含 10 个 `Option<&mut ...>` 的 automation 相关参数（"all-or-nothing" 注释自认）。函数体内混合了：布局计算、三种工具的拖拽分发、键盘区缩放、键盘拖宽、触觉反馈、GPU cull 上传（含增量/全量回退策略）、渲染线程 job 构建、egui 键盘绘制、选框绘制、时间标尺、automation 面板、两条滚动条、perf 探针、量化按钮。`#[allow(clippy::too_many_arguments)]` 是在压制正确的警告。建议拆出 `PianoViewCtx` 结构体和独立的 cull-upload / keyboard-paint / overlay 函数。

### [Major] `App` 结构体 — 约 60 个字段
**文件**: `app.rs:29-149`。一个结构体同时持有：2 套 GPU RenderContext + 2 个 InstanceRenderer + 渲染线程句柄、per-doc automation 渲染器、多文档、文件加载、音频引擎、音频设置、导出/重缩放异步状态、右侧面板、工具、标题栏、触觉、系统监控、FPS、剪贴板、菜单栏。职责至少横跨 6 个领域。缓解因素：方法已按领域拆到 7 个 `impl App` 文件（actions/audio/layout/main_loop/poll/dialog_dispatch/rescale_state），子状态也抽了 `AudioState`/`ExportState`/`RescaleState`。但字段层面仍是单一大袋。建议按 `render` / `audio` / `async_ops` 再聚合成 3-4 个子结构。

### `render_context.rs` 的角色（健康）
只是把 eframe 共享的 wgpu device/queue + 离屏纹理 + `paint()` 封装起来，职责单一、有单元测试，是 crate 里最干净的文件之一。

---

## 4. 耦合问题

### [Major] device-lost 回调互相覆盖（代码注释自认的 bug）
**文件**: `render_context.rs:81-88, 125-132, 275-278` + `app/dialog_dispatch.rs:14`
每个 `RenderContext` 都调 `device.set_device_lost_callback`，而 wgpu 该 API 是**替换式**的。pianoroll、arrange、以及每个 automation 面板（`automation_panel.rs:118` 每次 `from_render_state`）都会覆盖前一个回调。结果只有**最后创建**的 RenderContext 的 flag 会被置位；`dialog_dispatch.rs:14` 只 OR 了 `render_ctx` 和 `arr_render_ctx` 两个——automation 面板后建，会抢走回调，导致真正的 device lost 时这两个 flag 永远为 false，对话框可能不弹。文档注释承认了问题但没修。

### [Major] UI 层直接写核心模型，绕过 editor-core
**文件**: `app/dialog_dispatch.rs:259-260, 272, 287-288`
PPQ rescale 确认流程直接 `Arc::make_mut(&mut doc.data.model)` 改 `model.meta.ppq`、调 `model.rebuild_tempo_map()`，编辑逻辑散在 UI 层而不是 editor-core 的 API 里（undo 提交靠手动调 `yinhe_editor_core::history::commit_ppq`）。同类：`app/actions.rs:166-185` `select_all` 直接操作 `documents[idx]` 内部字段并手动 bump revision。

### [Minor] UI 直接读核心数据结构内部
`event_browser.rs` 全文直接摸 `model.notes` bucket、`model.conductor.tempo.events`、`track.automation_lanes`；`layout.rs:258-285` 直接读 `model.conductor.time_sig`。只读展示尚可接受，但这意味着 core 数据结构任何重构都会波及 UI 层十几个文件。event_browser 的 Notes 收集逻辑（按 track 过滤）本质是 core 该提供的查询 API。

### 依赖方向（总体健康）
yinhe-egui → editor-core / wgpu / types / audio / yin / mid2 / haptic / archive，未发现反向依赖。`view_interaction.rs` 用 trait 统一 PR/AR 视图交互是好设计。

---

## 5. 最长函数 Top 5

| # | 函数 | 位置 | 行数 |
|---|---|---|---|
| 1 | `piano_view::show` | `piano_view.rs:38-987` | ~949 |
| 2 | `automation_panel::show_panels` | `piano_view/automation_panel.rs:133-779` | ~646 |
| 3 | `handle_automation_interaction` | `piano_view/automation_panel/interaction.rs:211-720` | ~509 |
| 4 | `arrange::view_ui::show` | `arrange/view_ui.rs:25-396` | ~371 |
| 5 | `sel_drag_frame_arrange` | `arrange/view_ui.rs:406-732` | ~326 |

（紧随其后：`App::ui` main_loop.rs:43-287 ~244 行、`show_pianoroll_split` layout.rs:164-409 ~245 行、`start_export` actions.rs:489-676 ~187 行。）嵌套最深处在 `view_ui.rs:685-721`（6 层 if-let 嵌套）和 `interaction.rs` Pencil 分支（5 层）。

---

## 6. 错误处理：生产代码 unwrap/expect/unreachable

| 文件：行 | 内容 | 风险 |
|---|---|---|
| `app/actions.rs:541, 653` | `export_progress.lock().unwrap()` | Mutex 中毒即 panic，导出线程+UI 线程各一处，违反项目规则 17 |
| `app/rescale_state.rs:132` | `.expect("pending must be Some when rx was Some")` | 状态不一致即丢工程 |
| `app/main_loop.rs:30` | `ReplaceGuard::as_mut().expect(...)` | 实际不可达，但既然是守卫就更不该 panic |
| `app/actions.rs:282` | `unreachable!()` | 上面分析过：它守护的分支里本身含一个死分支，说明该 match 结构已腐烂 |
| `app/dialog_dispatch.rs:291` | `unreachable!()` | 靠 246 行提前 return 保证，脆弱 |
| `render_context.rs:74` | `.expect("wgpu backend required")` | 启动期，可接受但可给友好报错 |
| `main.rs:45, 80` | `expect` / `unwrap` | 启动期 |
| `widgets/selection_actions.rs:170` | `bar.unwrap()` | 运行期，依赖前置 is_some 判断 |
| `dialogs/archive_picker.rs:256`、`dialogs/export.rs:125-126` | `unwrap()` | 运行期，依赖状态约定 |
| `platform/macos.rs:26, 240` | `unwrap()` | lock poison |

另有 **7 处调试 `eprintln!` 残留**（`app/actions.rs:520, 556, 558, 561, 566, 586, 609`），同文件其他地方用的是 `tracing::`，应统一。

---

## 7. 魔法数字 / 硬编码

- **缩放因子 1.1 重复 6 处以上**：`piano_view.rs:298`、`automation_panel.rs:319, 354`、`view_interaction.rs:152, 156`——应入 `theme` 常量。
- **拖拽阈值 3.0px 重复 5 处**：`view_ui.rs:335, 673, 689, 808`、`view_interaction.rs:180`。
- `piano_view.rs:109` `avail_h * 0.65`（面板最大高度占比）、`event_browser.rs:47` `split_ratio: 0.45`。
- `interaction.rs:92` 线段命中阈值 `8.0`、`:38` `ANCHOR_HIT_PX = 10.0`、`:14` `HOVER_DELAY = 0.6`、贝塞尔偏移 `*4.0`（126-129 行，虽有注释但值很魔术）。
- `transport_bar.rs:191` `DOUBLE_CLICK_MS = 400.0`（局部 const，尚可）。
- `scrollbar.rs:19-20` `PPT_MIN/MAX = 0.001/10.0` 与 `piano_view.rs:364-367` 触觉通知里的 `0.001, 10.0, 60.0` 是同一组缩放范围的**两份硬编码**，改一处忘另一处就会不一致。`view_ui.rs:386-389` 又一份 `0.001, 10.0, 16.0, 120.0`。
- `render_context.rs:41` `_ => 4` 保守 bpp fallback（合理但有注释即可）。

---

## 屎山指数：58 / 100

评分依据：模块划分清晰、注释质量高、关键纯逻辑（BarLookup、snap、texture size）有单元测试、Rev-based 增量上传设计不错——这些拉住分数。但上帝函数 + 上帝结构、scrollbar/export/snap 三大复制粘贴、event_browser 每帧 O(N)、每帧 clone automation lanes、以及 device-lost 回调这个"注释自认未修"的正确性 bug，都是实打实的屎。

## Top 5 最严重问题

1. **[Critical] event_browser Notes 详情每帧 O(N) 收集+排序**（`right_panel/event_browser.rs:452-471`）——1 亿音符目标下打开即死。收集结果应按 `doc.data.revision` 缓存重建（抱歉，我知道你不喜欢缓存，但这里正确解是把"按 track 查音符"做成 editor-core 的增量维护索引，UI 只读）。
2. **[Major] device-lost 回调替换式覆盖**（`render_context.rs:81-88/125-132`）——多 RenderContext 下只有最后注册者生效，GPU 丢失提示可能永远不弹；且 `new`/`from_render_state` 的重复代码是帮凶。应共享同一个 `Arc<AtomicBool>`。
3. **[Major] 每帧深拷贝 automation lanes**（`app/layout.rs:290-308`）+ tempo max 每帧全扫两份（`automation_panel.rs:294`、`interaction.rs:230`）——编辑轨 CC 一多，UI 帧率直线下降。
4. **[Major] `piano_view::show` 950 行 50 参数上帝函数 + `App` 60 字段上帝对象**——两者互相成全：所有状态都得穿过 App 再摊平成 50 个参数。先抽 `PianoViewCtx`/`AutomationCtx` 参数结构是性价比最高的第一步。
5. **[Major] 三大复制粘贴**：scrollbar 三胞胎（~300 行重复）、`export_wav` 参数表×3、undo-push 模板×4（有现成 `with_undo` 不用）。按你的规则 2，这些都建议合并。

### 顺带提醒（规则 2/3/12 相关）
- AR 拖拽 ghost 是"每帧 128 key 查询"的旧方案，PR 已迭代到"拖拽开始时预计算"的更优方案——同一功能两种实现，AR 那份建议删了统一到 PR 方式。
- `arrange/view_ui.rs:212` notes 层 cache key 用 `hidden_notes.len()` 而非 hash——拖拽中增删隐藏音符但数量不变时会拿到陈旧图层，潜在渲染 bug。
- `arrange/view_ui.rs:326-347` 与 `eraser_drag_frame_arrange`（749 行）读取同一个 persisted id `"eraser_drag_arr"` 两份状态，橡皮擦框选的状态读取分散两处，易腐烂。
- 生产环境 unwrap 共 15 处（清单见第 6 节），导出/重缩放路径上的那 3 处最危险。
        
          
调查完成。我已通读两个 crate 的全部源文件（yinhe-core 7 个文件、yinhe-editor-core 20 个文件），并交叉验证了关键调用链。以下是完整报告。

---

# yinhe-core / yinhe-editor-core 代码质量与性能调查报告

## 总体评价

先说结论：**整体架构不是屎山**。文件拆分合理（无超长文件，最大 435 行）、注释质量高、COW（Arc::make_mut）+ 增量 rebuild + 矩形选区的设计方向正确，明显经过多轮性能打磨。但存在 **3 个 Critical 级正确性 bug**（其中两个是撤销导致的数据丢失）、若干热路径冗余、以及核心编辑逻辑测试空白。

---

## 1. 数据结构设计

**音符存储**：`YinModel.notes: Box<[Arc<Vec<Note>>; 128]>`（按 key 分桶，每桶按 start_tick 排序）— `/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-core/src/model.rs:148`。

各操作复杂度（B = 单桶音符数，R = 选区 rect 数，K = rect 覆盖 key 数）：

| 操作 | 复杂度 | 评价 |
|---|---|---|
| 单音符插入（add_note, note_edit.rs:38-39） | partition_point O(log B) + Vec::insert O(B) memmove | 可接受 |
| 范围查询（batch_ops.rs:31-32） | O(log B + 命中数） per bucket | 好 |
| 批量删除（remove_selected） | 每 rect×key 一次 drain/retain O(B) | 好（但见 P-6） |
| 批量插入（insert_batch） | extend + mark_dirty，rebuild_dirty 统一排序 | 好 |
| rebuild() 全量 | O(N + B log B)，rayon 并行 128 桶 | 仅用于加载/轨道结构变更，合理 |
| rebuild_dirty() 增量 | O（脏桶大小） | 设计正确 |

**1 亿音符下的隐患**：by-key 分桶意味着黑乐谱中高密度音高（如 C4 桶可能有数百万音符）时，单桶 O(B) 操作依然很大，但没有 O(N²)。真正的 O(N) 全量操作出现在轨道结构变更（见 C-4）。

---

## 2. 正确性 Bug（Critical）

### C-1 [Critical] 删除轨道后 undo 无法恢复音符（数据丢失）
`/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/document/track_ops.rs:77-143`
`remove_track` 返回的 `UndoAction::TrackStructure` 只存 `tracks_before/after`（元数据）+ remap 表；被删轨道上的音符在 track_ops.rs:109 `bucket.retain(...)` 中被物理删除，且 `TrackData.notes` 运行时是空的（model.rs:66-69 注释明确说明），**undo 时这些音符彻底丢失**。对比：删除音符的 undo 有 NoteDelta.before 兜底，删除轨道没有。1 亿音符工程里撤销一次删轨 = 静默丢音符。

### C-2 [Critical] 跨轨拖拽 automation 在 clamp 到原轨时事件被静默删除
`/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/document/arrange_move.rs:121-159`
第一阶段（126 行）已把 `src_lane.events` 替换为 `remaining`（被移动事件被剔除），且 `delta_tracks != 0` 时不会放回原 lane；第二阶段 157-159 行：
```rust
if dst_track_idx == lm.src_track {
    continue; // clamped to same track, events already in source lane
}
```
注释声称"事件已在源 lane"，但**它们不在**——已被 126 行剔除。极易触发：只有 conductor + 1 轨时向下拖（raw_dst clamp 回 src）、或向 conductor 方向拖（148-153 的 skip 逻辑把它调回 src）。被拖的 automation 事件直接蒸发。

### C-3 [Critical] `rebuild_dirty` 的 tick_length 只增不减
`/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-core/src/model_stats.rs:261-277`
```rust
let mut new_tick_length = self.tick_length;  // 从当前值起步，只取 max
```
删掉全曲最后一个音符（max end_tick 所在音符）后，tick_length 永远不会收缩。连带影响：`select_all_pr/ar` 选区范围（document/selection.rs:19,52）、播放结束判定（playback.rs:78）、`total_ticks_padded`。现有测试（model.rs:560-577）只测了"删非末尾音符"的情形，未覆盖此 bug。

### C-4 [Major] TrackStructure 的 undo/redo 是全量 O(N) 深拷贝 + 无边界检查索引
`/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/history/apply.rs:67-78`
- 对全部 128 桶做 `Arc::make_mut` + retain + 重写 track，再 `model.rebuild()`。若桶与音频/GPU 线程持有的快照共享 Arc，这是一次 **O(N) 深拷贝**；1 亿音符下撤销一次"移动轨道"可能卡数秒。track_ops.rs:47-52、107-113、185-190 正向操作同样如此。
- apply.rs:71-73 `note_remap[n.track as usize]` 无边界检查：任何 track 索引越界的音符（损坏数据/bug 引入）直接 panic，违反项目规则 17（生产环境不许崩）。同样模式见 track_ops.rs:50,109,187。

### C-5 [Major] track_ops 不重映射 automation lane 的 `track` 字段
`/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/document/track_ops.rs:47-52,107-113,185-190` 只改 `note.track`；而 document.rs:153-158（from_model 插入 conductor 时）是会同步 `lane.track += 1` 的。两处行为不一致，add/remove/move track 后 `AutomationLane.track` 变成陈旧值。

---

## 3. 性能问题

### P-1 [Major] undo/redo 每次全量克隆 delta 两次
`/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/history/apply.rs:93-95`：`undo()` = `self.reversed().redo()`，`reversed()`（history.rs:101-104）克隆 before+after 两个 Vec；随后 document.rs:256 push redo 栈时又 `entry.action.reversed()` 克隆第二次。对 1000 万音符的移动操作（NoteDelta ≈ 16B×2×10⁷ = 320MB），一次 Ctrl+Z 产生 **~640MB 临时分配**。history.rs:5 注释自称"typically a few hundred bytes"——对黑乐谱大选区完全不成立。建议：redo 栈直接存原 action（undo 时按方向解释），或为大选区操作改用桶级快照。

### P-2 [Major] add_note / pencil_drag 插入后冗余全桶排序
note_edit.rs:38-40、247-250：已用 partition_point 保持有序插入，随即 `mark_dirty` → `rebuild_dirty`（model_stats.rs:254）又对整个桶 `sort_by_key` O(B log B)。黑乐谱单桶数十万音符时，铅笔每画一个音都触发一次大排序。既然插入维持了不变量，应跳过该桶排序（或直接不设 dirty 的排序语义）。这正是规则 9 说的"A→B→C 多余转换"。

### P-3 [Major] 大选区删除的双倍扫描
note_edit.rs `delete_selected`（50-70 行）：先 `collect_selected` 全扫一遍，再 `remove_selected` 又扫一遍——而后者本来就返回被删音符。第一次 collect 完全多余（规则 9）。

### P-4 [Major] `YinModel::clone` 深拷贝 128 个 HashMap
model.rs:138 derive(Clone)，`bucket_track_stats: [HashMap<u16,(u64,u64)>; 128]`（187 行）不是 Arc。任何 `Arc::make_mut(&mut model)` 在共享状态下都会深拷贝这 128 个 map + 轨道 Vec。8 个统计缓存（note_count / track_note_count / track_audible_count / channel_note_count / channel_ctrl_count / bucket_note_count / bucket_track_stats / note_revisions）靠 rebuild/rebuild_dirty 两条路径分别维护——C-3 的 bug 已经证明这种双路径维护是脆弱的。

### P-5 [Minor] 轨道结构变更后 dirty 标志残留
model_stats.rs：`rebuild()` 不清 `dirty_keys`（只有 `rebuild_dirty` 在 257 行清）。track_ops.rs:114-118 `remove_track` 先 mark 全部 128 桶 dirty 再调 `rebuild()`，标志残留 → 下一次 `rebuild_dirty` 白白重排所有桶。

### P-6 [Minor] remove_selected 慢路径全桶扫描
batch_ops.rs:48-51：track 过滤时 `retain` 扫描整个桶而非仅 [start_idx, end_idx) 区间（注释自己承认 "retain (full scan)"）。按轨道选择大桶时是 O(B) 而非 O（区间）。

### P-7 [Minor] Document::empty 不必要的整体 clone
document.rs:95 `Arc::new(model.clone())`——model 是局部变量，直接 `Arc::new(model)` 即可。另外 from_model 中 model.rs:160 `rebuild()` 之后 document.rs:176 `data.rebuild_model()` 又 rebuild 一次。

---

## 4. 上帝对象评估

**Document（document.rs:37-43）不是上帝对象**：仅 5 个字段，方法分散在 6 个文件，职责（数据/编辑态/历史）三分清晰。这是好的设计。

**真正的隐患是 `EditState`（edit_state.rs:84-114）**：20 个字段的大杂烩——选区、量化预设、播放状态、轨道可见性、音色库配置（soundfont_selected_port / project_sf）、pending_edits、三种缓存、conductor 索引、editing_track、sel_rect。音频配置与编辑状态混在一起，建议拆出 `PlaybackUi` / `SfConfig` / `TrackViewState`。

**YinModel（model.rs:139-193）**：17 个 pub 字段，核心是 notes + 8 个派生统计缓存。内聚性尚可，但所有字段 pub 意味着任何调用方都能绕过 mark_dirty/rebuild_dirty 直接改数据破坏缓存一致性（测试里就是这么干的）。建议把统计字段私有化、只暴露只读 getter。

---

## 5. 代码重复

### D-1 [Major] "移动音符"模式复制了 5 份
以下五处都是同一个模式（remove_selected → HashMap 按 key 分组构造新 Note → 构建 after vec → insert_batch → offset 选区）：
- note_edit.rs:73-119 `duplicate_selected`
- note_edit.rs:122-162 `transpose_selected`
- note_edit.rs:169-212 `move_selected_notes`
- arrange_move.rs:36-75 `move_selected_arrange` 音符段
- document/selection.rs:139-176 `paste_from_selection`

连"HashMap → after vec 的 flat_map 转换"这段代码都逐字重复了 5 次。应在 batch_ops.rs 提供一个 `move_notes(model, originals, transform) -> after` helper。

### D-2 [Major] rescale_ppq 同步/异步两版逻辑完全复制
model_stats.rs:343-399 与 413-505：音符缩放、conductor 缩放、track 缩放三段逻辑逐行复制，仅多了 progress/cancel 检查。可抽公共函数 + 可选进度回调。

### D-3 [Minor] arrange_move 内 conductor-skip 逻辑重复
arrange_move.rs:45-55（音符）与 148-156（automation）是同一段"clamp + 跳过 conductor"逻辑的两份拷贝——而正是第二份拷贝漏了 dst==src 的回放，导致 C-2。DRY 缺失直接产生了 bug。

### D-4 [Minor] automation_edit 四个方法同构
automation_edit.rs 的 add/move/delete/set_shape 都是"定位 events → clone before → 改 → clone after → tempo 则 rebuild_tempo_map → bump_revision → 包 AutomationDelta"，Tempo/非 Tempo 分支各重复 4 次。可抽 `with_lane_events(...)`。

### D-5 [Minor] track_ops 三个方法的 remap 应用循环重复
track_ops.rs:47-52、107-113、185-190 三份"遍历 128 桶 make_mut + remap note.track"；且 move_track 的 remap 表（165-180）和 editing_track 更新（200-211）是同一套映射规则写了两遍。

### 遗留 fallback / 死代码
- edit_state.rs:94 `track_pianoroll_visible_snapshot: Option<Vec<bool>>` — 全仓库除定义和 Default 外**零引用**，死字段，按规则 1 应删。
- config.rs:80 `with_fallback_path` + audio_settings.rs:13 `default_sf2_path`（"Kept for migration — no longer used directly"）——旧配置迁移 fallback，是否可删取决于你是否还要兼容老用户的 settings.json。
- document.rs:224-227 `recode_track_names` 是 TODO 空壳（只 bump_revision）。
- model_stats.rs:229-230 注释说 "Track-level stats still do a full scan... future optimization"——**注释已过时**，下面第 3 步明明做了增量更新。

---

## 6. 错误处理

两个 crate 的生产代码中 `unwrap()` 共 2 处，无 `panic!`/`expect`：
- `/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/document/note_edit.rs:85` — `selected_data.iter()...min().unwrap()`
- `/Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/document/note_edit.rs:86` — `...max().unwrap()`

两处都被 81-83 行的 `is_empty()` 提前 return 保护，实际不会崩，但按规则 17 建议改为 `let Some(...) = ... else { return None }`。

**比 unwrap 更危险的是无检查索引**（越界即 panic）：
- apply.rs:71,73 `note_remap[n.track as usize]`（undo/redo 路径，损坏数据可触发）
- track_ops.rs:50,109,111,187,188 同模式
- document/selection.rs:41 `add_rect_track(0, max_end + 1, ...)`：若 `tick_length` 截断后等于 u32::MAX，`+1` 溢出（debug 下 panic）。selection.rs:19,52 `model.tick_length as u32` 截断转换本身在 tick_length > u32::MAX 时静默错误。

---

## 7. 测试覆盖

| 模块 | 测试 | 评价 |
|---|---|---|
| yinhe-core model.rs（rebuild/rebuild_dirty/channel 计数/rescale_ppq） | ~17 个，含增量 vs 全量对比测试 | 好 |
| tempo_map.rs | 6 个 | 好 |
| **yinhe-core selection.rs（contains/offset/hash）** | **0** | 选区是所有编辑的输入，应有测试 |
| history（apply/commit + Document::undo/redo） | history/tests.rs 7 个 | 中等（未测 Composite、TrackStructure、Automation undo） |
| playback / follow / quantize / document(conductor 检测） | 有 | 好 |
| **batch_ops.rs（remove/collect/insert——所有批量编辑的根基）** | **0** | 严重缺口 |
| **note_edit / arrange_move / track_ops / automation_edit / selection ops（paste）** | **0** | 严重缺口——C-1/C-2 两个数据丢失 bug 正是活在这个无测试区 |

无 benches。对 1 亿音符目标，建议至少加 criterion 基准（单桶 100 万音符的 insert/remove_selected/rebuild_dirty）。

---

## 8. 代码坏味道

- **过长函数**：`move_selected_arrange`（arrange_move.rs:23-202，约 180 行，含音符+automation+选区三段，应拆）；`pencil_drag_note`（note_edit.rs:219-311，92 行，三个 match 臂结构雷同）；`rescale_ppq_with_progress`（model_stats.rs:413-505，93 行）。
- **深嵌套**：arrange_move.rs:88-119 达 5 层（for rect → for track → for lane → for evt → if）；track_ops.rs move_track 的 remap 闭包 4 层 if/else。
- **魔法数字**：
  - selection.rs:100-105 哈希常数 `0x9e3779b97f4a7c15` 重复 6 次（应提 const）；且 XOR 哈希对"全字段相同的 rect"会自相抵消，作为 GPU cache key 有碰撞风险。
  - follow.rs:35 `64` 小节 padding、:58 `0.2` margin、:60 `0.5` 居中系数，注释自承"Assumes 4/4"。
  - document.rs:345 conductor 颜色 `[0.94, 0.94, 0.94]`；playback.rs:37 最低速 `0.1`；progress.rs:28-53 四个硬编码中文阶段标签（UI 文案漏进 core 库）。
  - tempo_map 的拍号分母用 2 的幂表示（`denominator: 2` = 4 拍）——是设计而非魔法数，但对读者不直观。

---

## 屎山指数评分

| Crate | 评分 (0-100，越高越屎） | 说明 |
|---|---|---|
| **yinhe-core** | **30** | 数据结构选型合理、注释优秀、测试尚可；扣分点：tick_length 只增不减 bug、双路径缓存维护、rescale 复制、selection 无测试 |
| **yinhe-editor-core** | **45** | 拆分清晰、delta undo 思路正确；扣分点：2 个 undo 数据丢失 Critical、移动模式×5 复制、批量编辑/轨道操作零测试、undo 热路径双倍克隆 |

**综合：38 / 100** —— 不算屎山，是"地基良好但承重墙上有三条裂缝"。裂缝全在 undo 正确性上，而 undo 恰恰是用户敢在 1 亿音符工程里随机编辑的前提。

## Top 5 最严重问题

1. **C-1 删轨 undo 丢音符**（track_ops.rs:77-143）——TrackStructure 不存被删音符，undo 永久丢失。修复方向：TrackStructure 增加 `deleted_notes: Vec<(Note, u8)>`。
2. **C-2 跨轨拖 automation 在 clamp 回原轨时事件蒸发**（arrange_move.rs:157-159）——注释与行为相反。修复：dst==src 时把 `lm.events` 放回源 lane 再排序。
3. **C-3 rebuild_dirty 的 tick_length 只增不减**（model_stats.rs:261）——删末尾音符后选区/播放范围永久错误。修复：脏桶删除后若旧 max 消失需重算（或退化为对该场景全量 max 扫描）。
4. **C-4+P-1 undo 系统在大选区下的双重代价**（apply.rs:67-78 + apply.rs:93/document.rs:256）——TrackStructure undo 全量 O(N) 深拷贝 + 无检查索引可 panic；NoteDelta 每次 undo/redo 全量克隆两遍。黑乐谱规模下这两个叠加就是"撤销一次卡几秒 + 可能崩溃"。
5. **测试空白区恰好覆盖最高危代码**——batch_ops / note_edit / arrange_move / track_ops 零测试，而 Top 1/2 的 bug 都在这里。按规则 18，应先为 C-1/C-2/C-3 各补一个回归测试。

另外按规则 2 提醒：D-1（移动模式×5）和 D-2（rescale×2）是明确的复用代码，建议你确认后统一收编进 batch_ops / 一个共享 rescale helper；`track_pianoroll_visible_snapshot` 是死字段可以直接删。
        
          
调查完成。我已通读两个 crate 的全部源码（yinhe-wgpu 20 个文件 + 2 个 WGSL，yinhe-synth 4 个 rs + 5 个 WGSL），并跨 workspace 核实了所有公共 API 的真实调用方（yinhe-egui、piano_view.rs、arrange/view_ui.rs、yinhe-audio）。以下报告中的"死代码"结论都经过全 workspace 引用验证，无遗漏。

注：本环境没有 shell 工具，无法运行 `cargo check`，所以编译警告无法实测；但下文中标出的死代码/未用项均基于引用扫描，结论是可靠的。

---

# yinhe-wgpu 调查报告

## 1. 渲染性能

### 1.1 cull.wgsl 确实在用，PR 音符已是 GPU culling（好）
`renderer.rs:98-100` include 了 cull.wgsl，`dispatch_cull`（renderer.rs:306）每帧对 128 个 key 各 dispatch 一次 compute，结果经 `draw_indirect`（renderer.rs:347）绘制。cull.wgsl 的 Hillis-Steele 前缀和保证了 z-order 稳定，设计是合理的。CPU 端 `build_notes`（pianoroll/instances.rs:60）只在 `use_gpu_cull=false` 的 render-thread fallback 路径每帧调用（piano_view.rs:499）。

### 1.2 [Major] AR（编曲视图）完全没有走 GPU cull，且滚动时每帧全量重建
`cull.wgsl:96-102` 里 mode==2 的 AR 分支实际上是**死代码**——AR 的 `arr_renderer` 从不调用 `upload_all_notes_for_cull`，`cull.is_ready()` 恒为 false，永远走 `draw_legacy`。
更严重的是 `arrange/view_ui.rs:212` 的 notes cache key 包含 `vh = view.render_hash()`，而 `arrangement_view.rs:123-131` 的 `render_hash()` **包含 scroll_x 和 scroll_y**：

```rust
// arrangement_view.rs:124-130
crate::hash::hash_f32s(&[self.base.pixels_per_tick, self.base.scroll_x, self.base.scroll_y, ...])
```

而 `arrangement/instances.rs:113-114` 的注释明确声称 "scroll_y changes do NOT invalidate the cache (same optimization as PR notes)"——**注释与实际行为相反**。AR 音符的像素位置全部在 shader 里算（vs_main_note mode==2），cache 本应对滚动免疫，实际却每次滚动/缩放都触发 O（可见音符数） 的全量重建+上传。1 亿音符最小缩放时这是每帧数百万次迭代。

### 1.3 [Major] grid 层 cache key 用了原始 scroll_x，亚像素滚动每帧失效
grid 实例只用 `scroll_x_pos = floor(scroll_x)`（pianoroll/prepare.rs:50 经 `compute_scroll_frac`），但 grid_key 里的 `vh`（pianoroll/prepare.rs:90-97）来自 `piano_roll_view.rs:59-67` 的 `render_hash()`，包含**原始 f32 scroll_x**。scroll_mode=2（亚像素偏移）下连续滚动时 vh 每帧变化 → grid 层每帧重建+重传。grid 本身量小，但这说明 cache key 的粒度设计有系统性错误：直接拿"整个 view 的 hash"当每层 key，而没有按层的真实依赖拆分。automation 的 grid_key（automation/prepare.rs:166）同样如此。

### 1.4 [Major] automation 每帧 O（全部事件数） 哈希，空闲也跑
`automation/prepare.rs:186` 的 `hash_lanes_excluding`（66-84 行）对**所有 lane 的所有事件**做逐事件哈希（`hash_lane` 42-62 行遍历 tick+value+shape），这一步在 cache 判断之前、每帧无条件执行。黑乐谱 CC/弯音数据量巨大时，仅"算 cache key"本身就是每帧 O(N) 扫描。缓存的目的是省掉 O(N) 重建，但 key 计算本身是 O(N)，缓存收益被部分抵消（省下的只是 GPU 上传）。同理 `bars_key` 里还有 `revision`（196 行）——revision 变化已经能代表内容变化时，逐事件哈希可能是重复保险。

### 1.5 [Major] 增量上传没有增长余量，逐音符编辑会反复重建 buffer
`renderer.rs:232`：`let size = needed.max(4096);`——按**精确大小**分配。用户往某个 key 里连续加音符时，每次增量上传（`try_incremental_key_upload` → `upload_one_key`）都因 `buf.size() < needed` 重建整个 buffer + bind group + 重写全 key 数据。对比 `layer.rs:9-11` 的 `grow_capacity` 是幂增长，CullState 这里应该复用同一策略。这也违反 DRY（两处容量增长逻辑）。

### 1.6 [Major] 空闲帧也强制 GPU 重绘
`render_context.rs:333` 本来有 `do_render = needs_render || content_changed` 的跳帧机制，但两个调用方都使其失效：
- `arrange/view_ui.rs:232`：`let content_changed = true;` 硬编码
- `automation/prepare.rs:230`：`prepare` 恒返回 `true`，`automation_panel.rs:503` 的 `gpu_dirty` 因此恒真

piano_view.rs:539 注释也写明 "every frame rebuilds + uploads, so always paint"。结果是 PR/AR/automation 三个面板在完全静止时也每帧 dispatch cull + 全部 draw call + submit。

### 1.7 [Minor] grid 实例每帧在 CPU 全量构建，cache 只省了 GPU 上传
`pianoroll/prepare.rs:98-105` 在 `build_render_job` 里**无条件**构建 grid_instances，然后才由 `upload_layer` 用 cache_key 决定要不要上传。构建本身没被缓存跳过（AR 路径的闭包式写法才能跳过构建，见 view_ui.rs:199——两种写法不一致）。

### 1.8 [Minor] NoteBufferKey 用 XOR 合成哈希 + hash_hidden 重复实现
`note_buffer_key.rs:35`：`revision ^ tv_hash ^ hidden_hash`——XOR 合成有对消风险（revision +1 且 hidden_hash 变化量恰好异或抵消时 key 不变，漏更新）。且 `NoteBufferKey::new`（28-34 行）与 `hash_hidden`（49-56 行）是**逐字节复制的同一段哈希循环**；piano_view.rs:412 和 458 每帧各算一次 `hash_hidden`，同一帧算两遍。

### 1.9 [Minor] dispatch_cull 每帧 32KB 栈数组 + 全量重置
`renderer.rs:309`：`let mut reset_data = [[0u32; 64]; 128];`（32KB 栈）+ 每帧 `write_buffer`。`draw()` 在 UI 线程被调（render_context.rs:339），32KB 对 Windows 1MB 主线程栈尚可接受，但完全可以用一个常驻 staging buffer 或 compute 清零。

---

## 2. render_thread.rs 线程模型

- 模型本身健康：mpsc channel + "drain 保留最新 job"（100-104 行）+ 空时阻塞 `recv`（110 行），**无 busy-wait**；锁只短暂持有克隆 TextureView（117-121 行），锁竞争可忽略。
- **[Minor] 每帧分配**：fallback 路径下 UI 线程每帧新建 `notes_instances`/`ghost_instances` Vec（piano_view.rs:497-504），经 channel 发给渲染线程后再 `extend_from_slice` 拷贝进 scratch（render_thread.rs:147-149）——同一份数据复制两次。
- **[Major] 整条 render-thread 路径是 GPU-cull 的旧版 fallback**：只在 `use_gpu_cull=false`（settings.rs:184 的用户开关）时使用。按你的规则 1，如果 cull 路径已验证稳定，这整条约 200 行的线程 + `RenderJob`/`NoteLayerData` 协议 + piano_view.rs:489-531 的分支都应该删。它还导致双 `InstanceRenderer` 并存（app.rs:32-33），渲染线程里那个 renderer 带着完整 CullState 闲置。
- **[Minor] unwrap/expect**：render_thread.rs:117、178（`lock().unwrap()`，锁中毒即 panic）、162（`spawn.expect`）。

---

## 3. unwrap/expect/unreachable 清单（yinhe-wgpu）

| 位置 | 内容 | 严重程度 |
|---|---|---|
| renderer.rs:293 | `NonZeroU64::new(256).unwrap()`（常量，安全但可改 const） | Minor |
| renderer.rs:497, 518, 540, 562 | 4 处 `unreachable!()`（upload_*_layer 系列） | Minor |
| render_thread.rs:117, 178 | `lock().unwrap()` | Minor |
| render_thread.rs:162 | `expect("failed to spawn render thread")` | Minor |
| grid.rs:106 | `sub_beat_color.unwrap()`（有 is_some 守卫，安全） | Minor |

违反规则 17 的都是低危路径，但生产代码确实不该出现。

---

## 4. 上帝对象：InstanceRenderer

`renderer.rs:364-374` 的 `InstanceRenderer` 有 9 个字段、至少 4 类职责：
1. uniform/track_colors/selection 上传与容量管理（402-458 行）
2. 层生命周期管理（460-564 行，4 个近乎复制粘贴的 `upload_*_layer`）
3. GPU cull 状态机（CullState，72-350 行，占文件 40%）
4. 双路径 draw 分发（614-726 行）

735 行不算失控，但 **CullState 应拆到独立 `cull.rs`**（它自包含、有独立文档注释，拆出去 renderer.rs 立刻降到 ~450 行）；4 个 `upload_*_layer` 应泛化成一个按 `LayerKind` 分派的方法（现在每个 20 行 × 4，连 `unreachable!()` 都复制了 4 份）。`draw_legacy` 与 `draw_with_cull` 的 Step 1-3（decor/velocity/curve 三个循环）逐行重复（642-664 vs 693-715），应提取共用函数。

### 其他死代码（yinhe-wgpu）
- **`PrepareTimings`（renderer.rs:8-14）**：全 workspace 无任何构造点，纯死类型，lib.rs:16 还在导出。
- **`LayerSlot::instance_count`（layer.rs:156-158）**：无调用方。
- **shader.wgsl `vs_main` 的 mode==1/2 音符分支（99-123、169-201 行）**：`DrawInstance.velocity` 在 vertex.rs:57 已标注 "Reserved"，音符早已迁到 NoteInstance 管线，这段 DrawInstance 版音符处理疑似遗留死分支，建议确认后删除。

---

# yinhe-synth 调查报告

## 1. [Critical] `cpu_render_voices` 把 voice.time 推进了两次——"CPU 参考实现"是错的

`synth.rs:548-550` 在逐 voice 循环末尾已经 `voice.time += speed * active_frames` 并清零 `start_offset`；紧接着 `synth.rs:553` 又调用 `advance_voices(voices, frame_count)`，后者在 `synth.rs:61-64` **再次** `voice.time += speed * active_frames`。每块 time 翻倍推进，循环回绕位置也全错。这个函数号称 "与 GPU shader 逻辑完全对应"（488 行注释）、被 `phase15_benchmark` 当基准对比（620 行），实际输出与 GPU 不一致。它还被 lib.rs:14 导出为公共 API。要么修复要么改 `#[cfg(test)]` 并修正——目前状态是埋雷。

## 2. [Critical] SF2 加载路径：采样数据按 key 深拷贝，内存爆炸 + 去重失效

`sfz_parser.rs:218-221`：对每个 region 的 keyrange **逐 key `info.clone()`**，而 `KeyInfo.sample_data` 是 `Option<Vec<f32>>`（sfz_parser.rs:14）——Vec 深拷贝。一个跨 88 键、10MB 采样的 region 会复制出 880MB。
连锁反应：`gpu_synth.rs:97` 的去重 key 是 `format!("sf2_{:p}_{}", data.as_ptr(), data.len())`——**每个 clone 的指针都不同**，去重必然失败，同一段采样又被重复 append 进 `sample_data` 并上传 GPU。SFZ 路径（PathBuf clone + 路径字符串去重）没问题，所以用 Starry Studio Grand（SFZ）测不出来；一旦加载 SF2 就是 GB 级内存事故。修法：sample_data 改 `Arc<[f32]>`。

## 3. [Major] 采样数据三处冗余 + 重建策略浪费

- `synth.rs:179` `sample_chunks: Vec<Vec<f32>>` 把全部采样在 CPU 侧**永久保留一份**（GPU 已有一份），注释说"reused for buffer rebuilds"，但重建只发生在 voice 数/frame 数增长时，完全可以重建时只重建 voice/output 小 buffer。
- `synth.rs:300-325` `ensure_buffers`：voice_count 超过幂次或 frame_count 变大时，**连几百 MB 的 sample chunk buffers 一起销毁重建**（319-325 行），而采样数据根本没变。
- `synth.rs:9-10`：`MAX_CHUNKS=5 × CHUNK_SIZE=30M f32` → 采样上限 600MB，超过的 key 静默发声为静音/垃圾，无任何报错。

## 4. [Major] render_into 每块全同步，双 staging 设计名存实亡

`synth.rs:459-463`：每个 block 都 `map_async` + `poll(Wait)` + `recv().unwrap().unwrap()`——CPU 阻塞等 GPU 跑完。既然每块都死等，`staging: [Buffer; 2]` + `staging_idx` 双缓冲轮转（163-164、470 行）**完全没有作用**，是照搬了异步管线的设计却没实现异步。导出长曲目时 CPU-GPU 完全串行，GPU 并块流水线收益为零。

## 5. [Major] 测试 shader 死代码 + 巨型调试型测试

- `shaders/test_shader.wgsl`、`shaders/test_switch.wgsl`：**全 workspace 零引用**，纯死文件。
- `shaders/const_one.wgsl`：仅在 synth.rs:720 被 include，但创建的 `const_pipeline`（724-728 行）**从未 dispatch**，等于死引用。
- `shaders/test_read.wgsl`：仅被 synth.rs:750 的调试测试使用。
- `synth.rs:628-805` 的 `gpu_vs_cpu_correctness`：约 180 行、手工建 5 套 buffer/bind group、满篇 `eprintln!`，**没有任何一个 assert**——这不是测试，是当年调 bug 的脚手架。按规则 1/3 应整个删除（它同时是 const_one/test_read 存在的唯一理由）。

## 6. 重复逻辑与遗留 fallback

- `advance_voices`（synth.rs:59-150）与 `voice_render.wgsl` 的 `envelope_at` + 循环回绕、以及 `cpu_render_voices` 三处**同一套 7 阶段包络逻辑**（Rust 两份 + WGSL 一份）。WGSL 那套不可避免，但 Rust 两份中 `cpu_render_voices` 的包络是第三遍实现（506-524 行），且如上所述还带 bug。
- `gpu_synth.rs:94-100`（from_renderer）与 `gpu_synth.rs:227-233`（note_on）的 dedup_key 构造逻辑完全重复，改一处忘另一处即崩。
- `render_block`（synth.rs:475-485，注释自认"兼容旧接口"）：全 workspace 只有测试在用——按规则 1 可删，测试改用 `render_into`。
- `cpu_render_voices` 同理：生产零调用（仅 synth.rs:620 的 benchmark 测试），却从 lib.rs:14 导出。
- 没有发现"GPU 不可用回退 CPU"的旧 fallback——生产路径（yinhe-audio）里 CPU 合成由 xsynth 承担，是并行实现而非 fallback，这块是干净的。

## 7. 错误处理清单（yinhe-synth）

| 位置 | 内容 | 严重程度 |
|---|---|---|
| sfz_parser.rs:251, 256, 262 | WAV 逐采样 `s.unwrap()`——损坏的 WAV 文件直接 panic | **Major** |
| synth.rs:430 | `buffers.as_mut().unwrap()`——`upload_samples` 前调用 `render_into` 即 panic | Major |
| synth.rs:463 | `recv().unwrap().unwrap()`——map 失败即 panic | Major |
| synth.rs:154-174 | 3 处 `#[allow(dead_code)]`（sample_chunks、chunk_offsets_buf、pipeline_layout）——压警告而不是删字段 | Minor |

## 8. 性能细节（小）

- `gpu_synth.rs:208-210`：`advance_voices` 按单 voice slice 逐 voice 调用（函数调用 × N），可批量化；`Voice { state, key }` 的非连续布局也阻碍了整块上传（`states_buf.extend(map(|v| v.state))` 每块拷贝一次，gpu_synth.rs:202——量小，可接受）。
- `voice_render.wgsl:73-91` 每个采样读取做一次二分 + switch（每 voice-frame 2 次），chunk≤5 时开销可忽略；真正的瓶颈在第 4 条的同步等待。
- `select_key_info`（sfz_parser.rs:87-99）线性扫 layer，力度分层通常 <8 层，无问题。

---

# 屎山指数评分

## yinhe-wgpu：**45 / 100**（中等偏健康）

架构主线（per-key GPU cull、revision 增量上传、16B 实例、shader 端算像素）是清晰且高性能的，测试覆盖也不错（instances/grid/vertex 均有单测）。扣分集中在：cache key 系统性误用 render_hash（AR 滚动全量重建）、三处面板空闲帧强制重绘、renderer.rs 内 4 段复制粘贴 + CullState 未拆文件、render_thread 整条 legacy fallback、若干死类型/死分支。

**Top 5：**
1. [Major] AR 不走 GPU cull 且 notes cache key 含 scroll_x/scroll_y → 滚动时每帧全量重建（view_ui.rs:212 + arrangement_view.rs:123-131），与 instances.rs:113 的设计注释直接矛盾
2. [Major] 空闲帧强制重绘：`content_changed = true` 硬编码（view_ui.rs:232）、`prepare_automation` 恒返 true（automation/prepare.rs:230）
3. [Major] automation 每帧 O（全事件） 哈希（prepare.rs:186 hash_lanes_excluding）
4. [Major] 增量上传 buffer 精确尺寸无余量，逐音符编辑反复重建（renderer.rs:232）
5. [Major] render_thread 整条 GPU-cull 旧 fallback（render_thread.rs 全文件 + piano_view.rs:489-531）+ renderer.rs 内 4×upload_*_layer / 2×draw 路径复制粘贴

## yinhe-synth：**70 / 100**（技术债较重）

存在两个 Critical 级正确性 bug、一个名存实亡的双缓冲设计、180 行无断言的调试脚手架和 2 个零引用死文件。`GpuSynth` 高层封装（事件调度 + voice 管理 + 预分配 states_buf）本身写得不错，是 crate 里最干净的部分。

**Top 5：**
1. [Critical] SF2 采样按 key 深拷贝 + 指针去重失效，内存爆炸（sfz_parser.rs:218-221 + gpu_synth.rs:97）
2. [Critical] `cpu_render_voices` time 双重推进，"参考实现"输出错误（synth.rs:548-553）
3. [Major] `render_into` 每块阻塞同步，双 staging 形同虚设（synth.rs:459-470）
4. [Major] 采样数据 CPU/GPU 双份常驻 + 缓冲区重建时重传全部采样（synth.rs:179、300-325）；600MB 上限静默截断（synth.rs:9-10）
5. [Major] 死代码群：test_shader/test_switch.wgsl 零引用、const_one 建了不用、180 行无 assert 调试测试、render_block 旧接口、cpu_render_voices 死导出（synth.rs:475-805，shaders/ 目录）

## 横切建议（优先级排序）
1. 先修 yinhe-synth 两个 Critical（SF2 改 `Arc<[f32]>`；删或修 cpu_render_voices）——这两个会出实际事故。
2. 删 synth.rs:628-805 调试测试 + 4 个测试 shader 中的 3 个 + render_block。
3. 修 AR 的 cache key（从 notes_key 中剔除 scroll，验证 cull.wgsl mode==2 分支要么接线要么删）。
4. 决定 render_thread fallback 的去留（规则 1：cull 稳定就删）。
5. 把 CullState 拆出 renderer.rs，泛化 upload_*_layer。
        
          
# 🏔️ 屎山指数报告 — yinhe 全代码库深度调查

调查范围：16 个 crate，约 39,800 行 Rust 代码，由 5 个并行调查组逐文件通读完成。

## 📊 总体评估

- **综合屎山指数：42 / 100**（中等，地基良好但有多条承重裂缝）
- **主要问题类别**：① 撤销系统的正确性 bug ② 黑乐谱规模下的复杂度地雷 ③ 复制粘贴与遗留 fallback
- **风险等级**：**High** —— 存在 4 个会导致用户数据丢失/静默错误输出的 Critical 级 bug

各 crate 评分：

| Crate | 指数 | 一句话点评 |
|---|---|---|
| yinhe-synth | **70** | 两个 Critical bug + 死代码群，技术债最重 |
| yinhe-egui | **58** | 950 行上帝函数 + 60 字段 App + 每帧 O(N) |
| yinhe-editor-core | 45 | 拆分清晰，但 undo 有 2 个数据丢失 bug |
| yinhe-wgpu | 45 | 主线架构好，cache key 系统性误用 |
| yinhe-mid2 | 45 | 双遍解析 + writer O(T×N) |
| yinhe-audio | 35 | 回调路径干净，export 重复多 |
| yinhe-core | 30 | 数据结构选型合理 |
| yinhe-archive | 30 | 生产无 unwrap，非 zip 路径裸奔 |
| yinhe-yin | 25 | 测试最好，全量内存驻留是隐患 |
| yinhe-theme | 25 | 藏着一份死 TRACK_PALETTE |
| yinhe-types | 20 | 基础扎实 |
| yinhe-memtrace | 15 | feature 门控教科书级，全场最干净 |

**值得肯定的地方**：无 `mod.rs`、生产代码 unwrap 极少（audio/mid2/yin 等 9 个 crate 为零）、cpal 音频回调完全实时安全、per-key GPU cull + revision 增量上传的渲染主线设计优秀、注释质量普遍高。

---

## 🔴 立即处理（Critical —— 会丢数据/出错误结果）

### 1. 删除轨道后 undo 永久丢失音符
[track_ops.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/document/track_ops.rs#L77-L143)：`TrackStructure` 只存轨道元数据，被删轨道上的音符在 `bucket.retain` 中被物理删除，undo 时彻底蒸发。1 亿音符工程里撤销一次删轨 = 静默丢音符。
**修复**：`TrackStructure` 增加 `deleted_notes: Vec<(Note, u8)>`。

### 2. 跨轨拖 automation 在 clamp 回原轨时事件蒸发
[arrange_move.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/document/arrange_move.rs#L157-L159)：第一阶段已把被拖事件从源 lane 剔除，`dst == src` 时 `continue` 的注释声称"事件已在源 lane"——实际不在。极易触发（只有 conductor + 1 轨时向下拖）。

### 3. SF2 加载内存爆炸 + GPU 去重失效
[sfz_parser.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-synth/src/sfz_parser.rs#L218-L221)：SF2 region 按 key 逐 key `info.clone()`，`sample_data: Vec<f32>` 深拷贝——88 键 × 10MB 采样 = 880MB。[gpu_synth.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-synth/src/gpu_synth.rs#L97) 用指针做去重 key，每个 clone 指针不同，去重必然失败，同段采样重复上传 GPU。SFZ 路径没问题所以 Starry Studio Grand 测不出来。
**修复**：`sample_data` 改 `Arc<[f32]>`。

### 4. `cpu_render_voices` time 双重推进，"参考实现"输出错误
[synth.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-synth/src/synth.rs#L548-L553)：循环末尾推进一次 `voice.time`，随后 `advance_voices` 又推进一次。这个号称"与 GPU 完全对应"的函数被 benchmark 当基准，实际输出是错的，还从 lib.rs 导出。

### 5. `rebuild_dirty` 的 tick_length 只增不减
[model_stats.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-core/src/model_stats.rs#L261-L277)：从当前值起步只取 max。删掉全曲最后一个音符后 tick_length 永不收缩，连带选区范围、播放结束判定全部错误。

### 6. event_browser 音符详情每帧 O(N) 全量收集 + 排序
[event_browser.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/right_panel/event_browser.rs#L452-L471)：选中某轨 "Notes" 后**每帧**遍历 128 桶 filter + sort。1 亿音符工程打开此面板 UI 直接卡死（行渲染是虚拟化的，数据收集不是）。

### 7. export_wav 逐文件加载音色库互相覆盖
[export.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-audio/src/export.rs#L119-L131) + `soundfont.rs:83` 整体替换：一个 port 配多个 SF 分层时，导出只剩最后一个 SF，与实时播放声音不一致，且 export 全链路零测试。

### 8. device-lost 回调互相覆盖
[render_context.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/render_context.rs#L81-L132)：wgpu 的 `set_device_lost_callback` 是替换式的，多个 RenderContext 互相覆盖，automation 面板最后注册会"抢走"回调——GPU 丢失对话框可能永远不弹。代码注释自认了问题但没修。

---

## 🟡 短期优化（High —— 黑乐谱规模下的复杂度地雷）

| # | 问题 | 位置 | 规模影响 |
|---|---|---|---|
| 1 | `seek_to` 对 seek 点前全部音符线性扫描，且每次 ReloadNotes（undo/redo/自动化编辑）都附带一次 | [engine_state.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-audio/src/engine_state.rs#L198-L227) | seek 到 90% 位置 = ~9000 万次迭代，ring 抽空爆音 |
| 2 | `dispatch_and_find_next` 每事件边界 O(128 + V_active) | [engine_render.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-audio/src/engine_render.rs#L33-L57) | 高密度段 O(k×V)，voice 越多越慢的正反馈 |
| 3 | mid2 writer O(T×N) 扫描——**yin/yin/io.rs:76 已有 O(N) 单遍实现可照搬** | [writer.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-mid2/src/writer.rs#L82-L105) | 多轨黑乐谱导出数量级差距 |
| 4 | mid2 parser 事件流完整解码两遍 | [parser.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-mid2/src/parser.rs#L66-L76) | 几百 MB 文件解析 CPU 翻倍 |
| 5 | TrackStructure undo 全量 O(N) 深拷贝 + `note_remap[n.track]` 无边界检查可 panic | [apply.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/history/apply.rs#L67-L78) | 1 亿音符撤销一次移动轨道卡数秒 |
| 6 | undo/redo 每次全量克隆 delta 两遍（1000 万音符移动 ≈ 640MB 临时分配） | [apply.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-editor-core/src/history/apply.rs#L93-L95) + document.rs:256 | 大选区撤销内存暴涨 |
| 7 | AR 视图 notes cache key 含 scroll_x/scroll_y，滚动时每帧全量重建——与 instances.rs:113 的设计注释**直接矛盾**；AR 从不走 GPU cull | [view_ui.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/arrange/view_ui.rs#L212) + arrangement_view.rs:123-131 | 最小缩放时每帧数百万次迭代 |
| 8 | automation 每帧 O（全事件数） 哈希只为算 cache key | [prepare.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-wgpu/src/automation/prepare.rs#L186) | CC 密集时缓存收益被 key 计算抵消 |
| 9 | 每帧深拷贝编辑轨 automation lanes + tempo max 每帧全扫两份 | [layout.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/app/layout.rs#L290-L308) | CC 一多 UI 帧率直线下降 |
| 10 | `ensure_conductor_track` O(N) 逐音符重写 + 与 `Document::from_model` 逻辑重复（注释自认 replicate） | [parser.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-mid2/src/parser.rs#L139-L163) | 加载时 1 亿次写 |
| 11 | yin 工程文件全量内存驻留（峰值 ≈ 3-4 倍模型大小），bincode/zstd 均可流式 | [io.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-yin/src/io.rs#L186-L187) | 1 亿音符 ~5-6GB 峰值内存 |
| 12 | CC38 的 `rfind` 全 Vec 扫描，RPN-LSB 密集文件 O(n²) | parser.rs:557-560, 579-582 | 细粒度自动化文件解析退化 |
| 13 | add_note 有序插入后又触发全桶 sort_by_key | note_edit.rs:38-40 → model_stats.rs:254 | 铅笔每画一音触发一次大排序 |

---

## 🟢 长期改进（结构/重复/死代码）

**上帝对象/函数**：
- [piano_view.rs](file:///Users/jieneng/Documents/GitHub/yinhe/crates/yinhe-egui/src/piano_view.rs#L38) `show` — **949 行、约 50 个参数**（`#[allow(clippy::too_many_arguments)]` 压制了正确警告），先抽 `PianoViewCtx` 是性价比最高的第一步
- `App` 约 60 字段横跨 6 个领域（app.rs:29-149）；`show_panels` 646 行、`handle_automation_interaction` 509 行
- `EditState` 20 字段大杂烩（edit_state.rs:84-114）；`AudioEngine` 20 字段（engine.rs:19-58）；`InstanceRenderer` 中 CullState 应拆独立文件

**复制粘贴重灾区**（按你的规则 2 汇报）：
- "移动音符"模式逐字复制 **5 份**（note_edit ×3、arrange_move、paste）→ 应收进 batch_ops
- scrollbar 三胞胎约 300 行重复（scrollbar.rs:32-508）
- `export_wav` 参数表 + progress 闭包复制 3 份（actions.rs:560-650）；`snap_tick` 三函数只差一行
- GPU 事件构建两处 + export 版多做一遍 tick→sample 转换（audio_renderer.rs:327-357 vs export.rs:324-350）
- rescale_ppq 同步/异步两版逐行复制（model_stats.rs:343-505）
- TRACK_PALETTE 双份定义，yinhe-theme 那份**全 workspace 零引用**

**遗留 fallback / 死代码**（按你的规则 1/3，建议确认后删）：
- `render_thread.rs` 整条 GPU-cull 旧 fallback（~200 行 + 双 InstanceRenderer 并存）
- synth.rs:628-805 约 180 行**无 assert** 的调试脚手架"测试"；`test_shader.wgsl`、`test_switch.wgsl` 零引用；`const_one.wgsl` 建了 pipeline 从不 dispatch；`render_block` 旧接口、`cpu_render_voices` 死导出
- `engine.rs:183-191` UpdateNotes/LoadSoundFont 的同步退化 fallback
- `track_pianoroll_visible_snapshot` 死字段（edit_state.rs:94）；`PrepareTimings` 死类型；shader.wgsl mode==1/2 音符死分支；`yinhe-dms` 整个 crate 是占位符
- 空闲帧强制重绘：`content_changed = true` 硬编码（view_ui.rs:232）、`prepare_automation` 恒返 true

**测试空白恰好覆盖最高危代码**：batch_ops / note_edit / arrange_move / track_ops **零测试**（Top 1/2 的 bug 就活在这里）；P2-10 seek 修复无回归测试（违反你的规则 18）；export 全链路零测试；arrangement_view 坐标换算裸奔；7z/tar 路径零测试。

**其他**：export_progress 两处 `lock().unwrap()`（actions.rs:541,653）违反规则 17；7 处调试 `eprintln!` 残留；缩放因子 1.1 出现 6 处、拖拽阈值 3.0px 出现 5 处、缩放范围 `0.001/10.0` 三处各自硬编码；SF2 WAV 逐采样 `s.unwrap()`（sfz_parser.rs:251-262）损坏文件即 panic。

---

## 📈 改进路线图（建议顺序）

1. **第一批（正确性）**：修 Top 8 中的 ① ② ⑤ ⑦ ⑧，并按规则 18 各补一个回归测试 —— 全是小改动、高收益
2. **第二批（防爆雷）**：SF2 改 `Arc<[f32]>`；删除 synth 死代码群（调试测试 + 3 个测试 shader + render_block + cpu_render_voices 死导出）
3. **第三批（1 亿音符性能）**：AR cache key 剔除 scroll；seek_to 改 end_sample 有序索引；dispatch 的 NoteOff 检测改堆；mid2 writer 照搬 yin 的 O(N) 单遍；parser 单遍化
4. **第四批（结构）**：抽 `PianoViewCtx`；移动模式×5 收编 batch_ops；scrollbar 三合一；决定 render_thread fallback 去留；删 TRACK_PALETTE 死副本

需要我针对其中任何一批进入 `/spec` 制定详细修改方案，随时说。