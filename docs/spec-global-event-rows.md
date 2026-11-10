# 全局事件行（Pro Tools 风格）Spec

状态：草案 v1（2026-11-10）

关系：本 spec 是 `docs/spec-midi-clip.md`（Clip v1）的前置阶段。
AR 与横向 PR 都显示事件行；PR 额外固定一条不可选的 Clip 边界行。
先完成事件行与纯时间标尺，再做 Clip v1 的网格布局。

## 背景

当前全局事件在 UI 上比较分散：

- Marker 的 pill 绘制、命中、重命名、拖动直接写在 `widgets/time_ruler.rs`；
- Tempo 是 Conductor 的自动化 lane，不属于事件行；
- 拍号参与标尺标签与网格计算；
- 调号、歌词、和弦主要通过时间码、事件浏览器或轨道数据展示；
- 每种事件都有自己的入口、命中逻辑和编辑路径。

数据层其实已经分开了：

- `ConductorData`：`tempo` / `time_sig` / `key_sig` / `markers` / `lyrics` / `chord`；
- `TrackData`：per-track `lyrics` / `chord`。

所以问题主要是 UI 与交互层：标尺承担了过多职责，事件类型各自特判。
Pro Tools 的做法是把 Marker / Meter / Key / Chords / Lyrics 放到独立的 Ruler 行里，
时间标尺只负责时间；Tempo 在 Live/Pro Tools 中是自动化，yinhe 也按自动化处理，
不进入事件行。本 spec 沿用这个思路，但做成通用行系统，而不是为每种事件再写一套。

## 目标

- 时间标尺只负责：时间、小节/拍、播放头、缩放/滚动、点击定位。
- AR 网格上方新增独立小行：
  - Marker（标签）
  - Meter（拍号）
  - Key（调号）
  - Chord（和弦）
  - Lyrics（歌词）
- 默认不显示任何事件行；AR 左上角的 `Events ▾` 下拉菜单选择显示哪些。
- 所有行共用一套布局、绘制、命中、编辑、undo 接口。
- AR 与横向 PR 共用事件行；PR 默认额外显示 Clip 边界行。
- 时间标尺统一为 20px；所有标尺内容垂直居中；PR 标尺不再显示 Marker。
- 事件密集时只绘制可见范围，标签自动稀疏化，不拖慢画面。
- 为 Clip v1 提供稳定的 AR 网格/行布局，避免 Clip 和标尺重构互相牵连。

## 非目标

- 不改变事件数据模型；不改 `ConductorData` / `TrackData` 的字段语义。
- 不把 Tempo / CC/PB/RPN/NRPN 自动化塞进全局事件行；它们仍是轨道自动化 lane。
- 纵向 PR 本期不接入事件行，保持现状。
- 不做 Clip 的创建/编辑；Clip 是后续阶段。
- 不做新的 MIDI 事件类型。
- 不改变播放、导出逻辑；行的编辑仍落到现有文档 API。
- 不要求 v1 支持所有行的自由拖拽排序；先固定默认顺序。

## 数据来源

| 行 | 数据 | 事件类型 | 说明 |
|----|------|----------|------|
| Marker | `ConductorData.markers` | `MarkerEvent { tick, text }` | 标签/标记 |
| Meter | `ConductorData.time_sig` | `TimeSigEvent` | 拍号变化 |
| Key | `ConductorData.key_sig` | `KeySigEvent` | 调号变化 |
| Chord | `ConductorData.chord` | `ChordEvent` | 全局和弦；per-track 和弦不进全局行 |
| Lyrics | `ConductorData.lyrics` | `LyricsEvent` | track 0 歌词；per-track 歌词不进全局行 |

明确的 v1 边界：

- **全局事件行只展示 Conductor 级事件。**
- `TrackData.lyrics` / `TrackData.chord` 保持现有展示路径，不混入全局行；
  后续如需 per-track 事件行，另做“轨道级事件行”，复用同一行框架。
- Program Change 是 per-track 离散事件，v1 不做全局行；后续可做轨道级行。
- 轨道自动化（CC/PB/插件参数）仍由 AR 的自动化行 / PR 自动化面板负责。

## 行模型

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventRowKind {
    Marker,
    Meter,
    Key,
    Chord,
    Lyrics,
}

/// 会话态：行的显示/高度/顺序。不参与 undo，不写入事件数据。
pub struct EventRowsState {
    pub rows: Vec<EventRowState>,
}

pub struct EventRowState {
    pub kind: EventRowKind,
    pub visible: bool,
    pub height: f32,
}
```

- v1 默认顺序：Marker → Meter → Key → Chord → Lyrics。
- 每行高度固定小行（建议 18–24 px），由主题/布局常量给出，用户可调。
- 默认全部隐藏（`visible = false`），不占用音轨区域高度。
- 行可见性由 AR / PR 各自的 `Events ▾` 下拉菜单控制，两个视图状态独立；状态是会话态，暂不落盘。
- PR 在 CLIP 模式下有一条 **Clip 边界行**，不进入 `EventRowKind`、不提供勾选；SONG 模式隐藏。
- 行顺序数据结构支持重排；v1 UI 先不提供拖拽排序。
- 行标题在左侧头部区域显示图标/名称；时间内容区与网格时间轴严格对齐。

## 布局

### 横向（AR / 通道 AR / 横向 PR）

```text
┌─────────────────────────────────────────────────────────────┐
│ 时间标尺：小节 / 拍 / 播放头（只负责时间）                  │
├──────────────┬──────────────────────────────────────────────┤
│ 行头         │ Marker 行                                    │
│ 行头         │ Tempo 行                                     │
│ 行头         │ Meter 行                                     │
│ ...          │ ...                                          │
├──────────────┼──────────────────────────────────────────────┤
│ 轨道/键盘    │ 音轨行 / 钢琴卷帘                            │
└──────────────┴──────────────────────────────────────────────┘
```

- 全局事件行固定在轨道区域上方，不随轨道垂直滚动。
- 行头宽度与 AR 轨道面板左栏一致（`left_panel_width`），保证时间内容区起点一致。
- 事件行消费的高度从轨道可视区域扣除；行折叠后轨道区域自动变高。
- 播放头贯穿标尺 + 所有行 + 轨道区。

### 与音轨行的关系

视觉上，事件行和音轨行一样是「行头 + 时间内容区」，所以会复用同一套行背景、
斑马纹、hover、选中态和播放头绘制。但**它不是音轨行**：

- 事件行不进入 `ArRowLayout`，不占用 `ArRow::Track` / `ArRow::Automation` 的行号；
- 事件行不参与 `total_rows`、`visible_track_range`、`track_offsets` 等轨道行计算；
- 事件行不随轨道面板的 `scroll_y` 垂直滚动，是独立固定区；
- 音轨面板只在顶部新增一块「事件行头部区域」，轨道列表从它下方开始，
  轨道索引与行号映射保持不变；
- 否则把事件行插进 `ArRowLayout` 会污染 `row_hit`，把事件行误判成音轨或自动化子行。

实现上应当有两个布局：

```text
EventRowsLayout   // 固定顶部：行高、可见行、行头矩形
ArRowLayout       // 其下：现有音轨/自动化行，scroll_y 只作用于它
```

PR 在 CLIP 模式下标尺下方显示 Clip 边界行（18px），其下才是可选事件行。
PR 的 `Events ▾` 放在键盘列顶部；Marker 只在事件行显示，PR 标尺不再绘制 Marker。
Clip 边界行用音轨代表色，左右边界可拖动，拖动时通过
`PianoViewEvent::ClipResize` 落到文档 undo。

### 纵向 PR（瀑布流）

纵向 PR 本期不显示事件行 / Clip 边界行，保持现状；事件行先实现横向 AR、通道 AR、横向 PR。

## 通用行接口

每行只负责“内容怎么画、事件怎么命中、怎么编辑”，公共部分统一：

```text
EventRow:
  - 背景、分隔线、hover、选中态、播放头穿越
  - 事件查询：只取可见 tick 范围（二分）
  - 标签稀疏化与 tooltip
  - 右键菜单框架 / undo 接线
```

按 kind 分派到具体实现：

- Marker：旗标 + 文本 pill；复用现有 marker 重命名/拖动/删除逻辑。
- Meter：事件块 + “4/4 / 3/4”标签。
- Key：事件块 + “C maj / A min”标签。
- Chord：事件块 + 和弦文本。
- Lyrics：文本片段，密度高时只画短横线/省略号，hover 显示全文。

禁止：

- 在 `time_ruler.rs` 里继续为每种事件加 `if kind == ...` 分支；
- 把行状态塞进 `YinModel`；
- 把行布局写进每个 view 各自实现，导致 AR/PR 漂移。

## 交互

### 公共

- 点击行空白处：与点击标尺一致，定位播放头。
- 点击事件：选中该事件；拖动改变 tick；双击编辑文本/值；右键菜单。
- 右键空白处：在播放头处新增该行事件。
- 拖动时与网格吸附；按住修饰键可临时关闭吸附（沿用现有约定）。
- 所有增删改走现有 document API，并生成对应 undo：
  - Marker：`UndoAction::EventList`
  - Meter/Key/Lyrics/Chord：对应的事件列表 undo
- 行内选中状态不跨行多选；v1 不做跨行框选。

### 各行的最小编辑能力

| 行 | 新增 | 编辑 | 删除 | 移动 |
|----|------|------|------|------|
| Marker | 播放头处新增 | 重命名 | 是 | 拖动 tick |
| Meter | 播放头处新增 | 改分子/分母 | 是 | 拖动 tick |
| Key | 播放头处新增 | 改调/音阶 | 是 | 拖动 tick |
| Chord | 播放头处新增 | 改和弦文本 | 是 | 拖动 tick |
| Lyrics | 播放头处新增 | 改歌词文本 | 是 | 拖动 tick |

- 同 tick 多事件：沿用各自数据结构的现有语义（如 Tempo lane 的 upsert）。
- 行事件密集时，命中优先级：事件 > 空白；重叠事件按绘制顺序/最近命中。

## 渲染与性能

- 每行每帧只查询可见 tick 范围：

```text
lo = view.tick_at_content_left()
hi = view.tick_at_content_right()
events = events_in_range(lo, hi)
```

- 查询必须二分，不允许全量扫描。
- 标签绘制：按像素间距稀疏化，放不下就不画文本，只画标记；hover 出 tooltip。
- 行背景/分割线/播放头用同一套 painter，避免每个 row 重复。
- 行数通常 < 10，每行可见事件通常很少；对音符 GPU 渲染无影响。
- 事件行不包含 Tempo；Tempo 的密集自动化仍由 AR 自动化行负责。

## 代码触点（计划）

| 位置 | 变化 |
|------|------|
| `crates/yinhe-egui/src/widgets/event_rows/` | 新增通用行框架 + 各 kind 实现（按文件拆分，不使用 `mod.rs`） |
| `crates/yinhe-egui/src/widgets/time_ruler.rs` | 移除 Marker 绘制/命中/重命名；只保留时间标尺、播放头、缩放/滚动 |
| `crates/yinhe-egui/src/arrange/` | 顶部接入全局事件行，轨道区域下移 |
| `crates/yinhe-egui/src/piano_view/` | 顶部接入全局事件行，键盘/网格区域下移 |
| `crates/yinhe-egui/src/app/` | AR 左上角 `Events ▾` 显示入口；行状态接入 `EditState` |
| `crates/yinhe-editor-core/` | 沿用现有事件编辑 API；如缺批量接口，补在对应事件模块 |

注意：

- `time_ruler.rs` 目前已经很长，本 spec 的目标之一是让它变短，而不是继续加分支。
- 行框架应保持高内聚：布局、绘制、命中、编辑各一层；不要把六种事件塞进一个上帝函数。
- 事件行状态不进入 `YinModel`，不参与 undo；这是视图状态。

## 持久化

- v1：行可见性、行高、顺序为**会话态**，随文档 `EditState` 存活，不落盘。
- 事件数据照旧由 `.yin` 的 conductor/tracks 段保存。
- 后续如需保存行布局，放入项目视图状态或应用设置；不要写进音符数据段。

## 与 Clip v1 的关系

- Clip v1 的 AR 布局依赖“标尺 + 全局事件行 + 轨道区”的固定结构。
- 本 spec 完成后，Clip v1 不再需要改标尺/行布局，只需要在轨道区加 Clip 块与
  `[CLIP | SONG]` 时间范围。
- 实施顺序：

```text
全局事件行框架 + Marker 迁移
→ Meter / Key 行
→ Chord / Lyrics 行
→ 横向 PR 事件行 + Clip 边界行
→ Clip v1
```

## 测试

- 时间标尺不再绘制/命中 Marker；Marker 行仍支持点击、拖动、重命名、删除。
- 每行事件查询只返回可见范围；缩放/滚动后位置正确。
- 行头与内容区起点对齐；AR/PR 左栏宽度变化时不漂移。
- Tempo/Meter/Key/Lyrics/Chord 的新增/编辑/删除/移动均进入正确 undo。
- 密集事件下标签稀疏化、hover tooltip 正常。
- 行折叠/展开后轨道区域高度正确，播放头贯穿所有行。
- 横向/纵向 PR 行布局（若实现）不写死方向；未实现时纵向模式有明确 fallback。
- 性能：每帧不全量扫描事件；Clip v1 前先跑一次 1 亿音符工程的滚动基线。

## 开放问题（实施前定）

1. 全局事件行的高度是固定 18/20/24px 还是允许用户逐行调整？
2. 行顺序是否允许拖拽重排，还是固定默认顺序即可？
3. Marker 行的“标签”是否需要区分 Section / Cue / Marker 子类型？
4. per-track 歌词/和弦是否未来需要轨道级事件行？