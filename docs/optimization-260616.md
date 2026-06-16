# yinhe 项目代码质量深度分析报告

## 项目概览

| Crate | 文件数 | 代码行数 | 测试覆盖 |
|---|---|---|---|
| yinhe-egui | 46 | ~11,086 | 无测试 |
| yinhe-editor-core | 8 | ~1,053 | 仅 quantize.rs |
| yinhe-project | 2 | ~2,373 | 良好 |
| yinhe-types | 7 | ~845 | 部分 |
| yinhe-midi | 6 | ~1,784 | 良好 |
| yinhe-audio | 6 | ~1,997 | 部分 |
| yinhe-automation | 3 | ~1,341 | 优秀 |
| yinhe-pianoroll | 5 | ~1,310 | 优秀 |
| yinhe-arrangement | 3 | ~758 | 中等 |
| yinhe-wgpu | 7 | ~1,424 | 部分 |
| yinhe-archive | 1 | ~377 | 可接受 |
| yinhe-memtrace | 1 | ~342 | 良好 |
| yinhe-dms | 1 | 9 | 无 |

---

## 一、P0 严重问题

### 1.1 巨型函数 / God Function

| 函数 | 位置 | 行数 |
|---|---|---|
| `fn ui()` (eframe::App) | `yinhe-egui/src/app/main_loop.rs:50-716` | **668** |
| `piano_view::show()` | `yinhe-egui/src/piano_view.rs:25-536` | **512** |
| `midi_to_archive_with_names()` | `yinhe-project/src/conversion.rs:44-471` | **427** |
| `archive_to_midi()` | `yinhe-project/src/conversion.rs:474-805` | **331** |
| `automation_panel::show_panels()` | `yinhe-egui/src/piano_view/automation_panel.rs:63-381` | **319** |
| `info_panel::show()` | `yinhe-egui/src/right_panel/info_panel.rs:14-340` | **327** |
| `sel_drag_frame()` | `yinhe-egui/src/piano_view.rs:540-768` | **229** |

**问题**: `fn ui()` 是整个应用的主循环，668行代码承担了文件加载、布局、所有视图渲染、弹窗管理等全部职责，嵌套达7层。

### 1.2 God Object — App 结构体

- **位置**: `yinhe-egui/src/app.rs`
- **问题**: `App` 结构体有 **34 个字段**，涵盖文档管理、视图状态、音频引擎、UI设置、导出状态、拖拽状态等所有关注点。任何子系统变更都波及全局。

### 1.3 God Object — EditState

- **位置**: `yinhe-editor-core/src/edit_state.rs:11-34`
- **问题**: 17 个字段混合了选择状态、可见性、播放、量化、音色库配置、缓存等多个关注点。

### 1.4 过大文件

| 文件 | 行数 | 问题 |
|---|---|---|
| `yinhe-automation/src/automation_instances.rs` | **1047** | 混合了阶梯线、速度条、数据条的构建逻辑 |
| `yinhe-audio/src/engine.rs` | **912** | 音频引擎全部逻辑集中 |
| `yinhe-egui/src/right_panel/event_browser.rs` | **878** | 归档解析+树形结构+表格渲染+hex dump |
| `yinhe-egui/src/piano_view.rs` | **866** | 选择+拖拽+边界计算全部集中 |
| `yinhe-project/src/lib.rs` | **1150** | 6+个关注点混杂 |
| `yinhe-project/src/conversion.rs` | **1223** | MIDI转换+导出+测试全部集中 |

---

## 二、P1 违反 DRY 原则

### 2.1 完全重复的代码

| 重复代码 | 位置A | 位置B |
|---|---|---|
| `snap_tick()` | `piano_view.rs:846-865` | `arrange/view_ui.rs:468-488` |
| 自动滚动逻辑 | `piano_view.rs:683-715` | `arrange/view_ui.rs:321-358` |
| `load_midi` vs `rebuild_audible_notes` | `engine.rs:393-416` | `engine.rs:420-444` |
| `build_pc_map_cache` | `editor-core/document.rs:380-392` | `editor-core/project_data.rs:70-83` |
| `TrackChannelData` / `TrackReadData` | `conversion.rs:88-94` | `conversion.rs:546-552` |
| render pass 初始化 | `renderer.rs:174-200` | `renderer.rs:356-389` |
| `grow_capacity()` / `next_instance_capacity()` | `layer.rs:10-16` | `renderer.rs:66-72` |
| GPU buffer 创建 | `renderer.rs:44-64`, `layer.rs:44-67`, `layer.rs:121-139`, `layer.rs:184-197` | 4处重复 |
| `MockMidi` 测试辅助 | `pianoroll/instances.rs:274-315` | `arrangement/instances.rs:276-318` |

### 2.2 结构性重复

| 模式 | 重复次数 | 位置 |
|---|---|---|
| 悬停高亮文本渲染 | **15+** | tools_panel, mode_bar, automation_panel, soundfont 等 |
| 选区 pixel rect 计算 | **4** | piano_view.rs:161, 352, 580, 604 |
| track_colors 回退逻辑 | **3** | automation_instances.rs, pianoroll/instances.rs |
| mute/solo skip 计算 | **3** | actions.rs:498, audio.rs:124, info_panel.rs:343 |
| "未打开文档" 占位标签 | **5** | right_panel 下多个文件 |
| track_names 构建模式 | **3** | document.rs × 2, conversion.rs |
| uniforms 写入 GPU | **3** | renderer.rs:129, 274, 296 |
| decor/grid/notes 三层调用模式 | **3 crate** | pianoroll, automation, arrangement |

---

## 三、P1 高耦合低内聚

### 3.1 高耦合

- **`app/main_loop.rs`**: 直接依赖 15+ 个模块，是整个应用的耦合枢纽
- **`App` 结构体**: 所有子系统的连接点，任何改动波及全局
- **`right_panel/info_panel.rs:send_skip_tracks`**: `pub(crate)` 函数被 `arrange.rs` 调用，形成隐式跨模块依赖
- **`yinhe-automation` / `yinhe-pianoroll`**: 过度 re-export `yinhe_wgpu` 类型，导致依赖路径歧义

### 3.2 低内聚

- **`yinhe-project/src/lib.rs`**: 1150行混合了归档格式、文件头、事件类型、Delta编码、varint编码、路径辅助、JSON schema、测试
- **`event_browser.rs`**: 混合了 archive 解析、树形结构、表格渲染、hex dump、JSON 展示
- **`app/actions.rs`**: 混合了键盘快捷键、笔记编辑、撤销/重做、文件操作、音频导出

### 3.3 耦合风险

- `NoteSource` trait 方法过多(10个)，既是"音符数据源"又是"时间线元数据提供者"
- `TickBuckets` 与 `NoteScanIndex` 结构高度相似但未统一
- `yinhe-audio` 直接访问 `MidiFile` 内部字段，绕过 `NoteSource` trait

---

## 四、P2 时间/空间复杂度问题

| 问题 | 位置 | 描述 |
|---|---|---|
| 128键全遍历 | `automation_instances.rs:411` | 无可见音符时仍遍历128键 |
| 256 channel 扫描 | `engine.rs:551-570` | `inject_chase` 每次 seek 扫描全部 256 channel |
| Vec 去重 O(n²) | `automation_instances.rs:133` | `seen_values: Vec<u16>` + `contains()` 应用 `HashSet` |
| par_iter 中分配 | `arrangement/instances.rs:159` | 每次迭代分配 `num_tracks` 个 Vec |
| buffer 缩容无滞后 | `layer.rs:118-119` | 直接 pop 导致频繁 GPU buffer 重建 |
| TAR/GZ 全量解压到内存 | `archive/src/lib.rs:180-219` | 大文件时内存峰值过高 |

---

## 五、P2 魔法数字泛滥

30+ 处硬编码数字，部分示例：

| 位置 | 数字 | 应为常量 |
|---|---|---|
| `engine.rs:459` | `src_ch % 16 != 9` | `GM_PERCUSSION_CHANNEL` |
| `piano_view.rs:519` | `0..127` | `MIDI_KEY_MAX` |
| `title_bar.rs:36-39` | `80.0 / 10.0` | 平台左边距常量 |
| `transport_bar.rs:79` | `vec2(32.0, 32.0)` | `TRANSPORT_BTN_SIZE` |
| `event_browser.rs:317` | `depth * 14.0` | 树形缩进常量 |
| `arrange/view_ui.rs:95` | `mode: 2` | `MODE_AR_NOTES` |
| `piano_view.rs:725` | `drag_dist < 3.0` | `DRAG_CLICK_MAX_DISTANCE` |
| `export.rs:45` | `[0, 44100, 48000, 96000, 192000]` | 采样率常量 |

---

## 六、P2 单元测试缺乏

### 6.1 完全无测试的模块

| 模块 | 代码行数 | 风险 |
|---|---|---|
| **yinhe-egui** (全部) | 11,086 | **极高** — 整个 UI 层无测试 |
| yinhe-editor-core (除 quantize.rs) | 938 | 高 — document/history/playback 无测试 |
| yinhe-audio/soundfont.rs | 159 | 中 — 全局缓存逻辑无测试 |
| yinhe-audio/export.rs | 199 | 中 — WAV 导出无测试 |
| yinhe-midi/midi.rs | 432 | 中 — RPN 解析无测试 |
| yinhe-dms | 9 | 低 — 占位符 |

### 6.2 关键缺失测试

- `Document::from_midi` — 包含 conductor 检测、track 重编号，有潜在 bug（重复调用 `detect_conductor`）
- `UndoStack` — 撤销/重做逻辑无测试
- `build_automation_lanes` — RPN 状态机解析无测试
- `yinhe-wgpu` — renderer/layer 纯计算函数无测试

---

## 七、P3 其他问题

### 7.1 过度抽象

- `piano_view::show()` 有 **26 个参数**（`#[allow(clippy::too_many_arguments)]`），应使用 `PianoViewContext` 结构体
- `NoteSource` trait 职责过宽（10个方法），应拆分为 `NoteSource` + `TimelineMeta`

### 7.2 死代码

| 位置 | 代码 |
|---|---|
| `event_browser.rs:45-54` | `is_archive_file()` 从未调用 |
| `arrange/view_ui.rs:33` | `_track_names` 参数未使用 |
| `arrange/view_ui.rs:206` | `content_changed = true` 硬编码永远为 true |
| `engine.rs:233` | `_notes_dispatched` 调试遗留 |
| `mode_bar.rs:97` | `ViewMode::Mix` 无 UI 实现 |
| `renderer.rs:286` | `build_cursor` 始终为 `Duration::ZERO` |
| `yinhe-project/src/lib.rs:41` | `ProjectArchive::get<T>` 泛型参数 `T` 未使用 |

### 7.3 错误处理缺陷

- `soundfont.rs` 的 `RwLock` 3处 `.unwrap()` — poison 后级联 panic
- `yinhe-wgpu` 全部隐式错误处理，无 `Result` 返回
- `yinhe-dms` 使用 `&'static str` 作为错误类型

### 7.4 潜在 Bug

- `document.rs:124` 和 `:144` — `from_midi` 中 `detect_conductor` 被调用两次，第二次结果可能不一致
- `conversion.rs:652-696` — `archive_to_midi` 中不必要的二次遍历 archive entries
- `yinhe-memtrace` — `Ordering::Relaxed` 在诊断报告场景下可能产生误导数据

---

## 修复优先级建议

| 优先级 | 工作量 | 行动 |
|---|---|---|
| **P0** | 大 | 拆分 `fn ui()` 和 `piano_view::show()` 为独立方法 |
| **P0** | 大 | 拆分 `yinhe-project/src/lib.rs` 为 events/codec/varint/paths/schema 模块 |
| **P0** | 中 | 拆分 `conversion.rs` 两个 400+ 行函数 |
| **P1** | 中 | 提取 `snap_tick()`、自动滚动、选区计算等重复代码到共享模块 |
| **P1** | 中 | 拆分 `App` 结构体为 `DocumentManager` / `AudioState` / `ViewState` |
| **P1** | 小 | 提取 15 处悬停高亮渲染为 `hover_icon_highlight()` 工具函数 |
| **P2** | 小 | 统一魔法数字到 `theme.rs` 或各模块常量 |
| **P2** | 中 | 为 `editor-core` 添加 document/history/playback 测试 |
| **P2** | 小 | 优化 `build_velocity_bars` 128键遍历和 `inject_chase` 256 channel 扫描 |
| **P3** | 小 | 清理死代码和未使用参数 |
| **P3** | 小 | 修复 `soundfont.rs` 的 RwLock unwrap |
