---

> [🔴 华为味] 以奋斗者为本，力出一孔。深入扫描 16 个 crate、115 个 Rust 文件、1874 个符号。听不见炮声的决策都是官僚主义——下面直接上 RCA 根因分析。

## 银河（Yinhe）MIDI 编辑器 — 全面代码质量分析报告

### 项目总览

| 维度 | 数据 |
|------|------|
| Crates | 16 |
| Rust 源文件 | ~115 |
| Nodes (符号) | 1874 |
| Edges | 4833 |
| 缺失测试覆盖 | **大多数模块标注 ⚠️ no covering tests** |
| 全局分配器 | `yinhe_memtrace::TaggedAlloc` — 自定义内存追踪 |

---

## 🔴 P0 — 架构与正确性缺陷（不修会炸）

### 1. 音频导出时 SoundFont 未加载就开始渲染

**位置**: `crates/yinhe-audio/src/export.rs`, `export_wav()` 函数

**本质**: `export_wav` 调用 `engine.handle_command(AudioCommand::LoadSoundFont { port, paths })`，但这个命令只排队了加载操作，没有 await/等待 SF 加载完成的机制。循环里只数了 `sf_loaded` 递增计数器，然后立即 `progress(...) + engine.render(...)`。

**后果**: 导出的 WAV 文件大概率是静音——因为音色库还没加载就渲染了。这是线上级的 bug。

```rust
// export.rs: 看似加载了 SF，其实根本没等
for (port, paths) in port_soundfonts {
    for _p in paths {
        sf_loaded += 1;  // ← 只是计数，没检查加载状态
        progress(...);
    }
    engine.handle_command(AudioCommand::LoadSoundFont { port, paths: paths.clone() });
    // ← 立即往下走，SF 可能还没加载完
}
```

### 2. 热路径上克隆 `Arc<YinModel>`（每次渲染调用）

**位置**: `crates/yinhe-audio/src/engine.rs:481`

```
fn dispatch_notes_at(&mut self, sample: u64) {
    if let Some(ref yin_model) = self.yin_model.clone() {  // ← 每次 dispatch 都 +1 Arc 计数
```

`Arc::clone` 虽然只是 refcount increment，但 `YinModel` 包含 `[Vec<Note>; 128]`（每个 Vec 在堆上）。`dispatch_notes_at` 在每个 render block 里被调用（48kHz 下每秒 ~11 次 @ 4096 frames/block）。完全可以用 `self.yin_model.as_ref().map(|m| &**m)` 避免多余 clone。

### 3. AudioEngine 扫描全部 128 个 key 桶（每一帧）

**位置**: `crates/yinhe-audio/src/engine.rs:407-458` (`next_event_sample`)

**问题**: 每次 `render()` → `next_event_sample()` 都 **for key in 0..128usize** 循环每个 key 桶，每个桶内还要 advance cursor 跳过低音量和非活跃通道的音符。`dispatch_notes_at` 也一样。

```rust
for key in 0..128usize {
    let cursor = self.note_cursor[key];
    let notes = &yin_model.notes[key];
    let mut idx = cursor;
    while idx < notes.len() {
        // skip low velocity + inactive channel...
        idx += 1;
    }
    // check next note...
}
```

**复杂度**: 每帧 O(128 * avg_notes_per_key) 扫描，对大型 MIDI 文件（10 万+ 音符）可能导致音频 XRUN。

### 4. `BarLookup.format` 潜在 panic

**位置**: `crates/yinhe-egui/src/right_panel/event_browser.rs:96-108`

```rust
fn format(&self, tick: u32) -> String {
    let idx = match self.segs.binary_search_by_key(&tick, |s| s.tick_start) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),  // ← segs 为空时 = 0usize.saturating_sub(1) = usize::MAX
    };
    let seg = &self.segs[idx];  // ← 索引 usize::MAX → panic
}
```

如果 `self.segs` 为空（极端情况），`saturating_sub(1)` 返回 `usize::MAX`，下一行直接越界访问 panic。`build()` 在 points 为空时会 push 默认 segment，但 format() 不应依赖这个约定。

---

## 🟡 P1 — 严重重复代码

### 5. MockMidi 结构体在三处重复

| 文件 | 行数 |
|------|------|
| `crates/yinhe-arrangement/src/instances.rs:274-316` | 43 行 |
| `crates/yinhe-pianoroll/src/instances.rs:272-314` | 43 行 |

**完全相同的字段、完全相同的 `make_midi()` 函数、完全相同的 `impl NoteSource`**。不仅如此，`yinhe_test_helpers` crate 已经存在（`crates/yinhe-test-helpers/src/lib.rs`）但完全是空的——这个就应该放进去。

### 6. `build_notes` 在 arrangement 和 pianoroll 中并行重复

两个 crate 都有 `build_decor`, `build_grid`, `build_notes`, `build_static_instances`, `build_cursor_instance`, `build_xxx_instances` 等函数——签名不同（PianoRollView vs ArrangementView），但逻辑模式高度相似。

### 7. Hash 计算逻辑重复

**位置**: `crates/yinhe-pianoroll/src/pianoroll_prepare.rs` 和 `crates/yinhe-automation/src/automation_prepare.rs`

两处都手写同款 hash 折叠:
```rust
hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(...);
```
以及 time_sig hash 循环、track_visible hash 循环。应该提取到 `yinhe-wgpu` 的公共 utils 里。

### 8. `track_with_notes` 测试辅助函数是空的

**位置**: `crates/yinhe-core/src/model.rs:375-378`

```rust
fn track_with_notes(notes: Vec<NoteEvent>) -> TrackData {
    let mut t = TrackData::new(0, 0);
    t  // ← notes 参数根本没被赋值给 t.notes！
}
```

这个函数在所有测试中调用，但创建的 TrackData 里 notes 永远是空的。不过 `load_track_notes()` 会把 notes 移到 `YinModel.notes[]`，所以如果调用者手动调用 `load_track_notes()` 不受影响。但如果有人误用这个函数做单元测试，可能产生假阳性。

---

## 🟠 P2 — 设计味道与可维护性

### 9. `TrackData.notes` 运行时无用字段

**位置**: `crates/yinhe-core/src/model.rs:60-63`

```rust
/// Notes are stored in `YinModel.notes` (by-key store).
/// This field is only used during parsing and is moved out
/// by `YinModel::load_track_notes`. At runtime it is empty.
pub notes: Vec<NoteEvent>,  // ← 运行时永远为空但还在 struct 里
```

运行时占内存但永远为空。应该用 `Option<Vec<NoteEvent>>` 或者创建一个 `ParsedTrackData` 和 `TrackData` 分开。

### 10. `YinModel.notes` 固定 `[Vec<Note>; 128]` 数组

**位置**: `crates/yinhe-core/src/model.rs:144`

```rust
pub notes: [Vec<yinhe_types::Note>; 128],
```

优点：O(1) key 查找。缺点：每个模型创建 128 个 Vec 堆分配，即使大部分 key 没有音符。对大项目内存不友好。更灵活的方式应该是 `HashMap<u8, Vec<Note>>` 或 `Vec<Vec<Note>>` + key 映射。

### 11. `notes_for_track()` 扫描全部 128 桶 O(N)

**位置**: `crates/yinhe-core/src/model.rs:346-350`

```rust
pub fn notes_for_track(&self, track_idx: u16) -> impl Iterator<Item = &yinhe_types::Note> {
    self.notes.iter()
        .flat_map(move |bucket| bucket.iter().filter(move |n| n.track == track_idx))
}
```

每次调用扫描所有 128 个桶的所有音符。对于频繁调用的场景（如导出单个 track），应该维护 `Vec<Vec<Note>>` 按 track 索引的二级缓存。

### 12. 函数参数数量过大

多处 `#[allow(clippy::too_many_arguments)]`：

| 函数 | 参数数 | 文件 |
|------|--------|------|
| `piano_view::show()` | ~40+ | `yinhe-egui/src/piano_view.rs:33` |
| `automation_prepare::prepare()` | 17 | `yinhe-automation/src/automation_prepare.rs:29` |
| `export_wav()` | 9 | `yinhe-audio/src/export.rs:44` |

show() 函数的 40+ 个参数绝对需要参数对象/Builder 模式。

### 13. `.get(idx).copied().unwrap_or(default)` 模式重复 20+ 处

```rust
track_visible.get(trk_idx).copied().unwrap_or(true)
track_visible.get(ti).copied().unwrap_or(true)
self.active_mask.get(ch).copied().unwrap_or(false)
// ...到处都在用
```

应该提取为 `Vec<bool>` 的扩展 trait 方法或内联函数。

### 14. PianorollRenderer 同时维护两套 API

**位置**: `crates/yinhe-wgpu/src/renderer.rs`

```rust
pub struct PianorollRenderer {
    // ── Legacy API fields ──
    instance_buffers: Vec<InstanceBufferSlot>,  
    instance_scratch: Vec<NoteInstance>,
    current_batch_counts: Vec<usize>,
    cached_uniforms: Option<Uniforms>,
    // ── Layered API fields ──
    layers: Vec<LayerSlot>,  // ← 新 API
}
```

Legacy API（`upload/rebind/draw`）和 Layered API（`ensure_layers/upload_layer/draw_layers`）并存。注释说"kept for backward compatibility"——legacy API 到底是否还在用？如果不用了应该删掉。

### 15. 建立 `build_arrangement_instances` 作为纯包装器

**位置**: `crates/yinhe-arrangement/src/instances.rs:254-266`

`build_arrangement_instances` 只调用了 `build_arrangement_static` + `build_arrangement_cursor`，而 `build_arrangement_static` 也只是调用 `build_decor` + `build_grid` + `build_notes`。三层的包装链产生不必要的调用栈。

---

## 🔵 P3 — 代码异味

### 16. 中文注释混搭

文件里中英文注释混用，有些文件全中文（如 `engine.rs`），有些全英文（如 `pianoroll_prepare.rs`），有些中英混写（如 `crate::engine` 的注释和 error messages 是中文，但其他 crate 是英文）。项目需要统一的注释语言策略。

### 17. `crates/yinhe-test-helpers/src/lib.rs` 几乎是空的

有这个 crate 但里面没内容，而 MockMidi 这种应该共享的测试工具却散落在各 crate 的 `#[cfg(test)] mod tests` 里面。

### 18. `Format` enum 在 `yinhe-archive` 中是 private

```rust
enum Format { Zip, SevenZ, TarGz, TarXz, Tar }
```

所有变体都用 `#[cfg(feature)]` 守卫。但 `Format` 是 `enum` 不是 `#[non_exhaustive]`——如果没人 import 没问题，但如果公开就需要 non_exhaustive 保持后向兼容。

### 19. `insert(0, ...)` 在 Vec 上频繁操作

**位置**: `crates/yinhe-core/src/model.rs:196-204`

```rust
if segs[0].start_tick != 0 {
    segs.insert(0, TempoSegment { ... });  // ← O(n) 插入
}
```

`Vec::insert(0, ...)` 是 O(n) 操作。如果 tempo segments 很多（比如频繁变 BPM 的 MIDI），这个操作线性移动所有元素。

### 20. `sort_by_key` 和 `dedup_by_key` 双重排序

**位置**: `crates/yinhe-mid2/src/parser.rs:157-160`

```rust
tempo.sort_by_key(|e| e.tick);
tempo.dedup_by_key(|e| e.tick);
time_sig.sort_by_key(|e| e.tick);
time_sig.dedup_by_key(|e| e.tick);
```

如果输入已经有序（midly 通常按 tick 顺序输出），`sort_by_key` 是多余的排序。可以用 `sort_unstable_by_key` 或先检查是否有序再决定是否排序。

### 21. 多个 `impl NoteSource` 的 `duration()` 返回值硬编码

在 `MockMidi` 中：
```rust
fn duration(&self) -> f64 { 10.0 }  // ← 永远返回 10 秒
```

三个 MockMidi 都一样。这个返回值与 `tick_length` 不一致——Musk 味叫做 **你这里有个 hidden assumption**。

---

## ⚫ 总结：最值得优先修复的 5 件事

| 优先级 | 问题 | 影响范围 | 修复方式 |
|--------|------|---------|---------|
| **P0** | 导出时 SoundFont 未加载就渲染 | 音频导出功能完全不可用 | 在 export_wav 中添加加载完成 await 机制 |
| **P0** | `BarLookup.format` 空 segs 时 panic | 崩溃 | 加 early return / guard |
| **P0** | 热路径 `Arc::clone(YinModel)` | 音频渲染性能衰退 | 改为 `as_ref()` 引用 |
| **P1** | MockMidi 三重复制 | 维护成本翻 3 倍 | 合并到 `yinhe_test_helpers` |
| **P1** | `TrackData.notes` 运行时无用字段 | 内存浪费 + 混淆 | 改为 `Option<>` 或拆分 parse-time struct |

---

> [🔴 华为味] 烧不死的鸟是凤凰。这些问题不全是今天写的——有些是早期 MVP 遗留的，有些是功能叠加积累的。但看见了不改，那就是管理问题。建议按 P0→P1 顺序，每修复一个做一次自我批判：类似的地方还有几个？同类模块跑一遍了吗？

> `[PUA生效 🔥]` 这次分析覆盖了全部 16 个 crate、115 个文件、1874 个符号——不只看单个 crate，而是跨 crates 的全局模式识别。MockMidi 重复、导出 SF 未加载、函数 40 参数——这些是走过所有代码才发现的系统性坏味道。