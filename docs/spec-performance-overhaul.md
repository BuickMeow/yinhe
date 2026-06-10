# 性能重构 Spec（4000 万音符 MIDI 卡顿治理）

## 背景

播放 ~4000 万音符的黑 MIDI 时：
- 同屏音符很少（低潮段）也跑不满 60 fps；
- 怀疑大量「该缓存却没缓存、该跳却没跳」的浪费；
- 力度为 1 的画图音轨在音频侧应当被整批跳过。

本 spec 列出按收益排序的优化项，作为分步重构指南。每项独立可验证，建议**逐项实施 → 测试 → 提交**，不要一次性堆改。

---

## 优先级总表

| 编号 | 项目 | 预期收益 | 工作量 | 风险 | 状态 |
|------|------|----------|--------|------|------|
| A3 | 把 scroll_x 平移做到 shader uniform | 极高 | 中 | 中（跨视图） | 待做 |
| A1 | 让 static 缓存在播放时真正生效 | 极高 | 小 | 低（依赖 A3） | 待做 |
| B1+B2 | audio 扁平化 + 预算 sample | 极高 | 中 | 中（mute 重建） | 待做 |
| A2 | cursor 独立 instance buffer | 高 | 小 | 低 | 待做 |
| A5 | piano roll 小音符合并 | 高 | 中 | 低 | 待做 |
| B3 | velocity lane 懒加载 | 高 | 小 | 低 | 待做 |
| A4 | 删除一整屏的 padding 扩展 | 中 | 小 | 低 | 待做 |
| A8 | 取消 is_playing 强制重提交 | 中 | 小 | 中（依赖 A1） | 待做 |
| A6 | track_buckets 复用 | 低 | 小 | 低 | 待做 |
| A9 | 小音符跳过抗锯齿和边框 | 低 | 小 | 低 | 待做 |
| A7 | 删除 qos::guarded | 极低 | 小 | 中（需确认动机） | 待评估 |
| C1 | Note 结构瘦身 | 低 | 中 | 高（工程文件格式） | 待评估 |

依赖关系：

```
A3 ──┬─→ A1 ──→ A8
     └─→ A2
A5 (独立)
A4 (独立)
A6 (独立)
A9 (独立)
B1+B2 (独立)
B3 (独立)
```

**推荐落地顺序**：A4 → A6 → A7 评估 → A3（仅 pianoroll）→ A1 → A2 → A8 → A3 推广到 arrangement/automation → A5 → A9 → B3 → B1+B2 → C1 评估。

理由：A4/A6 风险最低先做，立刻有可量化收益；A3 是结构性改造但收益最大，所以排第三批；B1+B2 涉及 audio 路径，等渲染稳定后再动。

---

## A 渲染热路径

### A3 用 shader uniform 平移视图

**位置**：
- `crates/yinhe-wgpu/src/shader.wgsl`
- `crates/yinhe-pianoroll/src/instances.rs`
- `crates/yinhe-arrangement/src/instances.rs`

**现状**：CPU 端把世界坐标转屏幕坐标再写进 instance：

```rust
let x_offset = kb_w - view.base.scroll_x;
let nx = x_offset + note.start_tick as f32 * ppu;  // 屏幕像素
```

scroll_x 改一次，所有 instance 都要重算和重传。

**改造方案**：

1. instance.x / instance.w 只存世界坐标：

   ```rust
   instance.x = note.start_tick as f32;             // tick
   instance.w = (note.end_tick - note.start_tick) as f32;  // tick width
   ```

2. shader 做平移、缩放、最小宽度：

   ```wgsl
   let screen_x = (instance.x - u.scroll_x) * u.pixels_per_tick + u.keyboard_width;
   let screen_w = max(instance.w * u.pixels_per_tick, 2.0);
   ```

3. **屏幕坐标 instance（keyboard / background / cursor 线）** 不应被 scroll 影响：
   - 在 `NoteInstance.tag` 给一个 bit 做 `coord_space` 标记。
   - 现 tag 在 grid 上存 tick、在音符上存 selection (0/1)。新规则：
     - bit 31: coord_space (0=world, 1=screen)
     - bit 0..30: 原语义（tick 或 sel flag）
   - shader 用 `select()` 切换公式。

**收益**：
- 滚动、Continuous follow：从「每帧重建几十万 instance + 上传几十 MB」 → 「每帧写 32 字节 uniform」。
- 缩放（ppu 变化）：world instance 也无需重建；唯一例外是 A5 合并的音符——ppu 跨越像素阈值时需重建（用 `(ppu * 100.0) as u32` 做 hash bucket 即可）。

**风险**：
- shader 双坐标系分支，要小心 grid 线（本来就用 tick 算 x，归到 world space）。
- 涉及三个视图。**只先做 pianoroll**，跑通后再推广。

**验证**：
1. 滚动 4000 万音符 MIDI，帧率应接近 vsync 上限；
2. Continuous follow 模式下播放，CPU 占用应大幅下降；
3. keyboard 不跟 scroll 移动；cursor 线仍跟 tick；
4. 选区命中、点击转 tick 等 CPU 逻辑保持正确（这些走 `view.x_to_tick`，无需动）。

---

### A1 让 static 缓存在播放时真正生效

**位置**：
- `crates/yinhe-egui/src/piano_view.rs:182-187`
- `crates/yinhe-pianoroll/src/instances.rs:206-212`

**现状**：

```rust
// piano_view.rs:182
if *cursor_tick != *last_cursor_tick {
    view.base.dirty = true;       // 播放时 cursor 每帧都变 → 每帧 dirty
}
```

加上 `build_static_instances` 内部用 cursor_tick 算 active_keys（键盘高亮），强迫 static 阶段每帧重做。

**改造方案**：

1. 删掉 `piano_view.rs:182-184` 的 dirty 触发。dirty 只该由滚动、缩放、选择改变触发。
2. 把 active_keys 检测从 `build_static_instances` 抽出：
   - 新增 `build_active_keys(midi, view, cursor_tick) -> ([bool; 128], [[f32;3]; 128])`，每帧调用，O(可见 key 数)。
   - keyboard instance 不再属于 static cache，每帧重建（128 个 instance 不是事）。
3. `build_static_instances` 去掉 `cursor_tick` 参数。

**前置依赖**：先做 A3。否则 Continuous follow 下 scroll_x 仍每帧变，viewport_hash 仍变，缓存仍失效。

**验证**：
1. 暂停时拖动滚动条，60 fps 流畅；
2. 播放时（FollowMode::None），加 tracing 日志验证 `build_static_instances` 不再每帧调用；
3. keyboard 高亮仍正确跟 cursor 实时刷新。

---

### A2 cursor 用独立的 instance buffer

**位置**：`crates/yinhe-wgpu/src/renderer.rs:166-173`

**现状**：

```rust
let mut combined = std::mem::take(&mut self.instance_scratch);
combined.extend_from_slice(&self.static_instance_cache);  // memcpy 几十 MB
build_cursor(&mut combined);
self.upload_instances(&combined);                          // 全量重传 GPU
```

**改造方案**：

1. `PianorollRenderer` 拆两组 buffer：
   - `static_buffers: Vec<InstanceBufferSlot>`（音符、grid、keyboard）
   - `dynamic_buffer: InstanceBufferSlot`（cursor 线，容量固定 16 个 instance 就够）
2. `prepare_with_static_cache`：
   - viewport 变化时：rebuild + 上传 static_buffers；
   - 每帧：write dynamic_buffer（几十字节）。
3. `draw()` 里两段 set_vertex_buffer + draw。

**前置依赖**：可独立做。与 A3 配合收益翻倍（A3 后 static 几乎永不重传，A2 让 cursor 也不触发整块重传）。

**验证**：wgpu profiler 看 `queue.write_buffer` 调用大小，cursor 帧应只有 32~256 字节。

---

### A4 删除一整屏的 padding 扩展

**位置**：
- `crates/yinhe-pianoroll/src/instances.rs:140-142`
- `crates/yinhe-arrangement/src/instances.rs:109-111`
- `crates/yinhe-automation/src/automation_instances.rs:105-107`

**现状**：可见区前后各扩展一整个屏幕宽度的 tick，用来处理穿越屏幕的长音符。

**改造**：

- **piano roll / arrangement**：scan_index 的 `cumulative_max_end` 已处理穿越音符（`seek_first_note` 走 binary search）。直接：

  ```rust
  let pad_start = tick_start;
  let pad_end = tick_end;
  ```

- **automation**：保留「前一个 event」用于画左侧延伸线：

  ```rust
  let events = lane.events_in_range(tick_start, tick_end);
  // 另外查找 tick_start 之前最近的一个 event 用作起点
  ```

**收益**：CPU 端音符循环区间从 3× 屏宽降为 1× 屏宽。

**验证**：滚动到各种位置截图对比，穿越屏幕的长音符不能消失。

---

### A5 piano roll 小音符合并

**位置**：`crates/yinhe-pianoroll/src/instances.rs:166-213`

**现状**：每个音符都 push 一个 instance，即使屏幕上小于 1 像素宽。

**改造**：抄 `yinhe-arrangement/src/instances.rs:138-209` 的 merge 模式，按 (key, track) 合并相邻音符。

```rust
let merge_gap_ticks = (1.0 / ppu).ceil() as u32;
// 按 track 分桶，桶内按 start_tick 升序（已经是）
for notes_in_track in track_buckets {
    let mut s = notes_in_track[0].start;
    let mut e = notes_in_track[0].end;
    let mut v = notes_in_track[0].velocity;
    let mut sel = is_selected(&notes_in_track[0]);
    for n in &notes_in_track[1..] {
        let cur_sel = is_selected(n);
        // 选中状态不一致或有任一被选中 → 不合并，保证选中视觉
        if n.start <= e + merge_gap_ticks && !sel && !cur_sel {
            e = e.max(n.end);
            v = v.max(n.velocity);
        } else {
            flush(s, e, v, sel);
            s = n.start; e = n.end; v = n.velocity; sel = cur_sel;
        }
    }
    flush(s, e, v, sel);
}
```

**决定**：
- gap 阈值用 1 像素（沿用 arrangement）。
- 选中音符不参与合并，保证选中状态可见。

**收益**：zoom-out 看全曲时 instance 数可降两到三个数量级。

**风险**：和 A3 有交叉——A3 后 instance 的 w 是 tick width，shader 决定屏幕宽。merge 决策依赖于「同一像素列」，所以 merge 需要在 prepare 阶段以**当前 ppu** 作为输入；ppu 跨阈值变化时需要重建。可以在 viewport_hash 里加上 `(ppu * 100.0) as u32`。

**验证**：
1. 不同缩放级别下截图对比合并前后；
2. 选中一段音符，缩放到极远，选中边框仍清晰可见。

---

### A6 arrangement track_buckets 复用

**位置**：`crates/yinhe-arrangement/src/instances.rs:171`

**现状**：

```rust
let mut track_buckets: Vec<Vec<(u32, u32, u8)>> = vec![Vec::new(); num_tracks];
```

128 个 key 并行，每 key 分配 num_tracks 个 Vec。track=200 时是 25600 次 `Vec::new()`/帧。

**改造方案**（推荐方案 B，最简单）：

用 `thread_local!` 缓存 `Vec<Vec<(u32, u32, u8)>>`，每帧 clear 后复用。注意要在 rayon worker 里使用，所以需要：

```rust
thread_local! {
    static TRACK_BUCKETS: RefCell<Vec<Vec<(u32, u32, u8)>>> = RefCell::new(Vec::new());
}

// 在 par_iter 闭包内：
TRACK_BUCKETS.with(|cell| {
    let mut buckets = cell.borrow_mut();
    if buckets.len() < num_tracks {
        buckets.resize_with(num_tracks, Vec::new);
    }
    for b in buckets.iter_mut().take(num_tracks) { b.clear(); }
    // ... 使用 buckets，结尾再 clear 不需要，下次进来会 clear
});
```

**风险**：thread_local 在 rayon worker 中需要确保不被嵌套使用（同一线程的同一帧只有一次进入）。当前结构是单层 par_iter，安全。

**验证**：用 `cargo instruments` 或 dhat 看堆分配数量，每帧 vec 分配应降到接近 0。

---

### A7 删除 qos::guarded

**位置**：`crates/yinhe-egui/src/widgets/qos.rs`，调用点：`piano_view.rs:190,208`、`arrange/view_ui.rs:112,140`

**现状**：每帧 4 次 `pthread_set_qos_class_self_np` 系统调用，意图让 audio 实时线程优先。

**疑问点（需用户确认）**：
- 当初引入这段代码是因为发现什么实际问题（比如播放破音）吗？
- 如果是为了 audio 顺畅，cpal 的 audio callback 已经跑在 macOS Real-time 线程类（高于 USER_INTERACTIVE），UI 线程的 QoS 切换其实没用。

**改造方案**（如果用户确认无具体动机）：
- 直接删 `qos::guarded` 调用，保留函数定义以备后用。
- 或者：在 `main.rs` 一次性把主线程设成 USER_INTERACTIVE，不再 per-frame 切。

**风险**：如果用户之前确实观察到破音，删完会复现。建议保留一个 git revert 路径。

**验证**：用 4000 万音符 MIDI 满载播放 5 分钟，无破音、无音频卡顿。

---

### A8 取消 is_playing 强制重提交

**位置**：`crates/yinhe-egui/src/arrange/view_ui.rs:139`

**现状**：

```rust
let content_changed = gpu_updated || is_playing;  // 播放时总是 true
```

**改造方案**：

```rust
let content_changed = gpu_updated;
```

**前置依赖**：必须先做 A1（让 cursor 变化时也算 gpu_updated，因为 cursor instance 真的变了），否则改完播放时画面会冻屏。

**验证**：播放时 cursor 仍然流畅移动；wgpu profiler 看每帧 `submit` 次数应只在真正有变化时增加。

---

### A9 小音符跳过抗锯齿和边框

**位置**：`crates/yinhe-pianoroll/src/instances.rs:192-203`

**现状**：所有音符无差别 push border_w 和 rounding。

**改造方案**：CPU 端按屏幕尺寸条件性置零：

```rust
let screen_w = (note.end_tick - note.start_tick) as f32 * ppu;
let screen_h = kh;
let (border_w, rounding) = if screen_w.min(screen_h) < 3.0 {
    (0.0, 0.0)  // 触发 shader fast-path 且不画边框
} else {
    (0.1 * screen_w.min(screen_h), NOTE_ROUNDING * screen_w.min(screen_h))
};
```

shader fast-path 里 `radius < 0.5` 已自动跳 SDF；现在让 border_w=0 时也跳掉抗锯齿的 smoothstep 边界采样。

**收益**：黑 MIDI 同屏几十万音符 fragment 工作量下降。

**风险**：和 A3/A5 有交叉。A3 后这个判断要挪到 shader 里做（因为 instance.w 是 tick width）。可以等 A3 完成后再做。

---

## B 音频热路径

### B1 + B2 audio 事件扁平化 + 预计算 sample 位置

**位置**：`crates/yinhe-audio/src/engine.rs:151-213`，`crates/yinhe-audio/src/engine.rs:226-297`

**现状**：
- `render()` 每次回调对 128 个 key 各持一个 cursor 推进；
- 每次 cursor 推进都要 `if note.velocity <= 1 { continue; }` 跳过画图音符；
- 每个有效音符都要做 2 次 `tick_to_seconds`（O(log segments)）；
- 维护 `active_notes` Vec 跟 NoteOff 时机。

**核心问题**：vel≤1 的音符在 4000 万级别下，光是 cursor 推进 + continue 就是 O(N) 的常驻开销，且分散到每次 audio 回调里，是 black MIDI 卡顿的潜在主因之一。

**改造方案（按 key 分桶 + 预算 sample，不增加内存）**：

新增字段：

```rust
struct PreparedNoteEvent {
    start_sample: u64,
    end_sample: u64,
    channel: u8,
    velocity: u8,    // 已知 > 1
    track: u16,      // 保留供 skip_track 动态判断
}

// 替换原本基于 midi.key_notes 的运行时逻辑
prepared_key_events: [Vec<PreparedNoteEvent>; 128]
```

`load_midi` 阶段构造：

```rust
for key in 0..128 {
    for note in &midi.key_notes[key] {
        if note.velocity <= 1 { continue; }
        if !active_mask[note.channel as usize] { continue; }
        // skip_track 不在这里过滤，运行时动态可变
        let start_sample = (midi.tick_to_seconds(note.start_tick as u64) * sr) as u64;
        let end_sample = (midi.tick_to_seconds(note.end_tick as u64) * sr) as u64;
        prepared_key_events[key as usize].push(PreparedNoteEvent {
            start_sample, end_sample,
            channel: note.channel,
            velocity: note.velocity,
            track: note.track,
        });
    }
    prepared_key_events[key as usize].sort_by_key(|e| e.start_sample);
}
```

`render` 简化为：

```rust
for key in 0..128 {
    let events = &self.prepared_key_events[key];
    while self.note_cursors[key] < events.len() {
        let e = &events[self.note_cursors[key]];
        if e.start_sample >= end { break; }
        if !self.skip_track.get(e.track as usize).copied().unwrap_or(false) {
            self.channel_group.send_event(SynthEvent::Channel(
                e.channel as u32,
                ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: key as u8, vel: e.velocity }),
            ));
            self.active_notes.push(ActiveNote {
                key: key as u8,
                channel: e.channel,
                end_sample: e.end_sample,
            });
        }
        self.note_cursors[key] += 1;
    }
}
// NoteOff 部分不变
```

**收益**：
- vel≤1 在 load 阶段一次性过滤，audio render 永远碰不到（B1 解决）；
- 运行时零次 `tick_to_seconds`（B2 解决）；
- 内存几乎不变（每个 event 18 字节，原 Note 16 字节，多 2 字节但少了 start_tick/end_tick 那部分语义 → 实际相当）。

**风险**：
- mute/unmute 时**不需要重建** prepared_events，只需更新 `skip_track`，render 里照常检查（保留 track 字段就是为了这个）。
- 但 active_mask（哪些 channel 启用）变化时需要重建——这个场景很少（基本只在 load_midi 时发生一次），可以接受。
- seek 时仍能用 `partition_point` 跳到正确位置（events 按 sample 排序）。

**验证**：
1. 4000 万音符 MIDI 满载播放，无破音、无音频回调超时；
2. mute/unmute 单个 track 实时生效，无重建延迟；
3. seek 仍准确。

---

### B3 velocity lane 懒加载

**位置**：`crates/yinhe-midi/src/midi.rs:403-418`

**现状**：

```rust
for notes in key_notes.iter() {
    for note in notes {
        velocity_events.push(AutomationEvent { ... });  // 4000 万 events
    }
}
velocity_events.sort_by_key(|e| e.tick);
```

4000 万 AutomationEvent × ~16 字节 = 640 MB 内存 + O(N log N) 排序，且 99% 用户不打开 velocity 面板。

**改造方案**：

- 在 `build_automation_lanes` 里**不生成** Velocity lane。
- 在 `AutomationLane` 上加一个 `Lazy<Velocity>` 变体，或者直接在 velocity 面板的 instance 构建器里从 `key_notes` 现取：

  ```rust
  // automation_instances.rs 里检测 lane.target == Velocity
  // 时改走单独路径，从 midi.key_notes 取数据，且只取可视范围内
  ```

- 自动化面板的范围查询：128 个 key 各做一次 `partition_point(start_tick)` 找到起点，多路归并。屏幕宽度内最多几千个事件，性能没问题。

**收益**：load 时间和峰值内存都大幅下降。4000 万音符可少占 ~640 MB。

**风险**：自动化面板若用户拖选 velocity 做批量编辑，需要保证查询接口完整。当前如果只是显示，没问题。

**验证**：
1. 加载 4000 万音符 MIDI，load 时间和峰值内存显著降低；
2. 打开 velocity 面板，显示正确，滚动流畅；
3. 不打开 velocity 面板，零开销。

---

## C 数据布局

### C1 Note 结构与工程文件瘦身（待评估）

**位置**：`crates/yinhe-types/src/note.rs`，工程文件相关 crate

**讨论点（需用户澄清）**：用户提到「把 Note 的上下宽高从工程文件里扔掉，能不能现算」。

需要澄清「上下宽高」具体指什么字段。当前 `Note` 是：

```rust
pub struct Note {
    pub start_tick: u32,   // 4
    pub end_tick: u32,     // 4
    pub key: u8,           // 1
    pub velocity: u8,      // 1
    pub channel: u8,       // 1
    pub track: u16,        // 2
}
// 实际 padding 后 = 16 字节
```

- `start_tick / end_tick` 是必要数据，不能算（保留）；
- `key` 在 `key_notes[key]` 分桶后理论上可以删，省 1 字节 → 4000 万 × 1 ≈ 40 MB；
- `channel` 是 audio 路径必需，不能算；
- `track` 是渲染、mute 必需，不能算。

**可能的优化**：
1. **删除 key 字段**：`key_notes[key]` 已经知道 key，迭代时传入。需要修改 NoteSource trait 接口（迭代时把 key 附带传给消费者）。Layout 改成 12 字节（含 padding 可能仍 16）。
2. **u32 end_tick 改成 u32 duration**：节省同样空间但可能让代码更清晰。
3. **工程文件（.yin）格式精简**：若 .yin 是自己定义的格式，可以只存 (start, duration, vel, ch, track) 然后按 key 分组，加载时重组。

**风险**：
- 修改 Note 是大改，影响范围横跨 midi / pianoroll / arrangement / audio / automation。
- key_notes 接口改动需要小心：所有 `for note in key_notes[key]` 的地方都要把 key 加进迭代。

**建议**：等其他优化都做完，确认 Note 结构是否真的成为瓶颈（用 dhat / Instruments 测内存）再决定。预期收益是 40-80 MB（4000 万音符）。

---

## 工具与基线

### 性能 baseline 记录

每完成一项优化，建议记录：

| 指标 | 工具 | baseline | A4 后 | A3 后 | ... |
|------|------|----------|-------|-------|------|
| Idle 帧率（暂停） | 内置 fps | | | | |
| 播放帧率（低潮段） | 内置 fps | | | | |
| 播放帧率（密集段） | 内置 fps | | | | |
| 滚动帧率 | 内置 fps | | | | |
| Load 时间 | tracing | | | | |
| 峰值 RSS | Instruments | | | | |
| 峰值 GPU 内存 | yinhe-memtrace | | | | |

### Profiling 工具建议

- **macOS**：`cargo instruments -t "Time Profiler"` 看 CPU 热点。
- **GPU**：Xcode → Debug → Capture GPU Frame (Metal)。
- **内存**：`cargo instruments -t "Allocations"` 或现有 `yinhe-memtrace`。
- **音频**：cpal 已有的 `XRunCount` 计数（如果没暴露，可加一个 atomic counter）。

### tracing 日志建议

在每个改造点加 `tracing::debug!`，记录关键事件：

```rust
tracing::debug!(
    target: "perf",
    "static rebuild: instances={}, viewport_hash={:x}",
    count, hash
);
```

跑的时候 `RUST_LOG=perf=debug cargo run --release` 即可看到。

---

## 提交粒度建议

- 每个编号一次提交（A3 单独，A1 单独），便于 bisect 性能问题。
- 提交信息格式：`perf(A4): 删除 pianoroll/arrangement padding 扩展` + 量化数据（如果有）。
- 涉及 shader 改动的（A3、A9）务必单独提交并测试三个视图。



