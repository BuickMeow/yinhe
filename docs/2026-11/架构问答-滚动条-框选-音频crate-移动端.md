# 架构问答：滚动条 / 框选 / 音频 crate / 移动端分层

> 日期：2026-11-18
> 背景：代码规模盘点后确认的几个边界问题。
> 前提：**移动端保留**，所有建议都是为了更易维护，不是砍移动端。

---

## 0. 结论速览

| 问题 | 结论 |
|---|---|
| 滚动条要不要单独拆出来？ | 它**已经在** `widgets/scrollbar`，不属于 PR/AR；不需要再拆 crate |
| PR/AR 框选有没有合适抽出？ | **数据模型与选区操作已共享**；手势/几何按视图保留，强行统一会是坏抽象 |
| 音频要不要再拆 crates？ | **现在不需要**；现有 audio/mixer/synth/dsp/meter 划分合理。只有出现「独立设备层消费者 / WASM / 编译时间瓶颈」才拆 |
| Flutter 哪些放 Dart / 抽通用 crate？ | 存储与纯 UI 状态可移 Dart；`scene`/`edit` 应抽成平台无关的 `yinhe-mobile-core`；音频会话抽共享 `yinhe-audio::session` |

---

## 1. 滚动条：已经独立，不需要拆 crate

### 现状

| 文件 | 生产代码 | 职责 |
|---|---:|---|
| `widgets/scrollbar/horizontal.rs` | 219 | 通用横向滚动条 + 密度预览 |
| `widgets/scrollbar/vertical.rs` | 286 | 通用纵向滚动条（也用于行高缩放） |
| `widgets/scrollbar/density.rs` | 189 | 密度预览数据 |
| `widgets/scrollbar/drag_warp.rs` | 74 | 拖拽边缘自动滚动 |
| `widgets/scrollbar.rs` | 29 | 常量、`double_clicked_in`、re-export |
| **合计** | **~797** | 通用控件层 |

使用方：PR（`piano_view/scrollbar.rs` 281 行，负责时间轴+音高轴组合、角落缩放）、AR（`arrange.rs`）、通道 AR（`channel_arrange.rs`）、PR 自动化面板（`automation_panel.rs`）。

### 评价

- 它**不隶属于 PR 或 AR**：底层 API 接收 `scroll_x/scroll_y`、`pixels_per_tick`、`Orientation`，与具体视图无关。
- PR 的 `piano_view/scrollbar.rs` 是「组合层」：把共享横/竖条摆到时间轴和音高轴上，并处理角落缩放。这是正确的分层。
- 只有 egui 使用它；Flutter 端的滚动条是 Dart 原生绘制。拆成独立 crate 只会多一条依赖边。
- 代码内聚好、测试 1,114 行（覆盖率高于多数 UI 模块）。

### 决策

**保持现状。** 如果未来需要给第四个视图复用，最多把 `drag_warp` 并入 `widgets::auto_scroll`；不需要新 crate。

---

## 2. 框选：模型/操作已共享，手势层不应强行统一

### 已经共享的部分

| 文件 | 生产代码 | 作用 |
|---|---:|---|
| `yinhe-core/selection.rs` | 381 | `Selection`：矩形 + 成员位图 + 属性筛选；1 亿音符全选用 1 个 rect |
| `yinhe-editor-core/document/selection.rs` | 236 | 选区的文档操作（物化、扩展、删除等） |
| `yinhe-editor-core/edit_state/selection_ops.rs` | 148 | 编辑态选区操作 |
| `yinhe-egui/widgets/selection_actions.rs` | 244 | 浮动工具条动作（删除/复制/移调/翻转/人性化…） |
| `yinhe-egui/selection/drag.rs` + `draw.rs` | 386 | PR 专用：音符拖动/缩放的方向感知坐标与绘制 |
| `yinhe-egui/piano_view/marquee.rs` | 258 | PR 框选状态机（量化、3px 阈值、自动滚动） |

### 没有共享、且不建议共享的部分

- **PR 框选**：像素 ↔ (tick, key)，支持横/纵瀑布流，键盘宽度、量化吸附、选框浮动条、自动滚动、拖动预览。
- **AR 框选**：tick × 轨道行，支持选框内整体移动、左右缩放、riff 填充、音频片段命中、幽灵音符；与行布局/轨道隐藏/文件夹强耦合。
- **通道 AR / PR 自动化面板**：又各自有锚点/通道语义。
- **Flutter**：触屏版 `edit/*`，阈值、命中外扩、长按、双指取消都与鼠标不同。

强行抽一个「通用框选状态机」需要把这些差异全部参数化，结果会比现在更难读。当前共享边界是：
- **选区数据 + 文档操作 + 工具条动作** 共享；
- **输入状态机 + 视图几何** 各视图自持。

### 可选的小抽取

如果未来出现第四个视图，可以抽一个很小的 `widgets::marquee`，只包含：

1. 「按下 → 超过 3px 才算拖动」的阈值状态；
2. 选框矩形绘制（虚线/填充/角标）；
3. `pointer_over_popup` 的统一守卫。

预计只省 50~100 行，**当前不值得做**。

---

## 3. 音频：现有 crate 划分合理，暂不再拆

### 现状

| crate | 生产代码 | 职责 |
|---|---:|---|
| `yinhe-audio` | 8,958 | 引擎、事件模型、设备/线程、导出、冻结、素材解码 |
| `yinhe-mixer` | 1,901 | 混音图、PDC、发送、参数修复 |
| `yinhe-synth` | 4,643 | CPU 合成器 + SFZ 解析 |
| `yinhe-dsp` | 339 | CC/滤波器等小 DSP |
| `yinhe-meter` | 560 | 电平/频谱/声谱分析 |
| `yinhe-clap` / `yinhe-vst3` | 4,218 | 插件宿主 |
| `yinhe-midi-io` | 167 | MIDI 设备 IO |

### 为什么不再拆 `yinhe-audio`

`engine` 与 `spawn` 是**双向引用**：

- `engine.rs` 使用 `spawn::{AudioCommand, AmMsMap, InsertTarget}`；
- `spawn.rs` 使用 `engine::{AudioEngine, ENGINE_BLOCK_FRAMES}`；
- `audio_renderer` 同时依赖两者。

真要拆成 `audio-engine` + `audio-device`，必须再引入第三个 `audio-protocol` crate 放命令/句柄/预览参数，并把 `AudioEngine` 的 `pub(crate)` 方法提升为公共 API。三个 crate、9k 行，收益主要是「隔离 cpal/symphonia 依赖」，但 UI 端本来就要用引擎，隔离不掉。

**结论：现在不拆。** 只有满足以下任一条件才值得：

1. 需要在不带 cpal 的目标（WASM/无音频服务器）编译引擎；
2. Android/OHOS 与桌面设备层出现明显分叉，需要独立 crate 做条件编译；
3. `cargo build -p yinhe-audio` 成为迭代瓶颈（目前依赖编译占大头，拆 crate 帮助有限）。

### 如果未来要拆，推荐边界

| 新 crate | 内容 | 约行数 |
|---|---|---:|
| `yinhe-audio-protocol` | `AudioCommand`、`WorkerCmd/Result`、`InsertTarget`、预览参数、`BufferSize` | ~500 |
| `yinhe-audio-engine` | engine/engine_state/engine_render/audio_model/channel_layout/channel_dsp | ~5,500 |
| `yinhe-audio-device` | spawn/audio_renderer/audio_out/audio_ring/export/freeze | ~3,500 |

`yinhe-audio-source`（解码 323 行）不建议单独拆：它依赖 symphonia/rubato，但 UI 本来就要链接引擎。

---

## 4. 移动端：哪些放 Dart，哪些抽通用 crate

### 4.1 现状：已经复用得很充分

`yinhe-flutter` 已经依赖并使用：

- `yinhe-types` / `yinhe-core`（模型）
- `yinhe-editor-core`（文档、撤销、量化、导入导出）
- `yinhe-wgpu`（GPU 渲染）
- `yinhe-audio`（引擎/设备/导出）
- `yinhe-theme`、`yinhe-midi`、`yinhe-yin`、`yinhe-archive`

所以**不存在“移动端另写一套引擎”**的问题。真正的 14,299 行生产代码分布是：

| 层 | 内容 | 约行数 |
|---|---|---:|
| 场景投影 | `scene.rs` + `scene/*`（JSON 给 Dart） | 5,513 |
| 触屏编辑 | `edit.rs` + `edit/*`（手势/几何/预览/提交） | 3,189 |
| FFI 接口 | `dart_ffi/*` | 1,299 |
| 音频桥 | `audio.rs`（会话/音色库/导出/设置） | 931 |
| 工程 IO / 后台任务 | `scene/file.rs`、`doc_job`、`autosave`、`midi_import/export`、`library`、`soundfont_archive` | 1,788 |
| 平台/纹理 | `android`、`ios`、`ohos`、`lib.rs`、`render` | ~1,500 |
| 辅助 | `selection_materializer`、`inertia`、`velocity_cache`、`theme_colors`、`midi_names` | ~350 |

### 4.2 更适合放在 Dart 的部分

**原则：Dart 负责“表现与平台存储”，Rust 负责“模型与编辑语义”。**

| 可移 Dart | Rust 行数 | 理由 |
|---|---:|---|
| 最近文件列表 `scene/recent.rs` | 261 | 纯持久化 + 文件存在性；Dart 有 `path_provider`/`shared_preferences` |
| 应用私有目录/设置的持久化 `scene/settings.rs`（部分） | ~250 | 平台存储是 Dart 的强项；Rust 只接收 JSON |
| 主题派生色 `theme_colors.rs` | 31 | 只把 GPU 主题转成 Dart 颜色；可由 Dart 根据主题包计算 |
| 项目元数据弹窗 `scene/project.rs`（部分） | ~243 | 只是只读 JSON；可让 Dart 直接读 `project.json` |
| 纯 UI 状态（当前 tab、滚动位置、对话框开关、跟随/暂停开关） | — | 不应在 Rust 再镜像一份 |

**不要移 Dart：**

- `scene/events.rs`（标记/拍号/调号/和弦/歌词）：需要模型遍历与时间轴换算；
- `scene/automation.rs`：自动化目标枚举与取值；
- `scene/view.rs`：视口/覆盖层命中数据；
- `edit/*`：编辑提交必须走 `editor-core` 的 undo/模型 API；
- `audio.rs`：音频会话状态必须与引擎同线程语义；
- `midi_import/export`、`soundfont_archive`：已有 Rust 解析器，移 Dart 会重复实现。

### 4.3 更适合抽成通用 crate / 模块的部分

#### A. `yinhe-mobile-core`（平台无关的移动编辑核心）

把以下内容从 `yinhe-flutter` 移出，不包含 JNI/NAPI/objc/纹理：

- `scene.rs` + `scene/*`（5,513）
- `edit.rs` + `edit/*`（3,189）
- `selection_materializer`、`inertia`、`velocity_cache`、`theme_colors`（~350）
- `render/upload` 等与 surface 无关的上传准备（~400）
- `dart_ffi` 中纯数据转换部分（可选）

预计 **9,000~9,500 行**。剩下的 `yinhe-flutter` 只保留：

- `android.rs` / `ios.rs` / `ohos_ffi.rs` / `lib.rs` / 纹理与 surface
- `dart_ffi` 的 `#[no_mangle]` 导出层
- `audio.rs` 的平台胶水（见 B）

**收益**：

- 可在桌面 `cargo test -p yinhe-mobile-core` 跑场景/手势/几何测试（现在 Flutter 只有 180 行测试）。
- 平台 crate 不再携带全部编辑逻辑，CI 可不装 NDK 就测核心。
- FFI ABI 保持不变（导出符号仍在 `yinhe-flutter`）。

**风险**：纯搬迁，但 `scene`/`edit` 目前大量使用 `crate::` 路径，需要一轮机械调整；建议先搬 `scene`，再搬 `edit`。

#### B. `yinhe-audio::session`（桌面/移动共享音频会话）

桌面 `app/audio.rs`（906 行）与移动 `audio.rs`（931 行）重复了：

- 惰性 spawn / 重建判断
- soundfont 下发与去重
- loop / skip / max voices / ignore velocity 下发
- 播放位置平滑
- 导出 WAV 进度

建议在 `yinhe-audio` 内新增 `pub mod session`：

- `AudioSession`：持有 `CpalAudioHandle` + `ChannelLayout` + 已发送的 soundfont 列表 + 设置缓存；
- 方法：`ensure(layout, settings)`、`rebuild`、`send_soundfonts`、`sync_loop`、`sync_skip`、`set_max_voices`、`export`、`smooth_position`；
- 桌面与移动都调用它；插件机架/insert 生命周期仍留在桌面 `App`。

预计统一 **500~700 行**行为，减少两端漂移。风险中等（音频线程/设备），需要播放/导出/切设备回归。

#### C. 工程 IO / 自动保存共享

| 移动端文件 | 行数 | 建议 |
|---|---:|---|
| `autosave.rs` | 211 | 与桌面 `app/autosave.rs` 570 行统一策略到 `editor-core::autosave`（路径/间隔由平台注入） |
| `midi_import.rs` / `midi_export.rs` | 336 | 共用 `editor-core` / `yinhe-midi` 的解析与写回，移动端只保留进度 JSON |
| `scene/file.rs` | 639 | 打开/保存/归档状态机可抽 `yinhe-project-io`；平台路径仍由 Dart 传入 |
| `library.rs` | 107 | 读 `.yin` 头部；可放 `yinhe-yin` 或 `editor-core` |
| `soundfont_archive.rs` | 341 | 与 `yinhe-archive` 合并为通用音色库归档模块 |

预计合并 **400~700 行**重复逻辑；不需要新 crate 时优先放 `yinhe-editor-core` 的 `io`/`autosave` 模块。

#### D. 不建议抽的

- `scene/*` 的 JSON 投影**不要**抽到 `yinhe-editor-core`：它是移动端渲染契约，桌面用不到，放进去会污染核心。
- `edit/*` 的手势状态**不要**放 `yinhe-editor-core`：触屏阈值/长按/双指语义是移动端专属。
- `dart_ffi` 不要拆成多个 crate：ABI 边界集中在一处更容易审计。

### 4.4 建议的最终分层

```
yinhe-types / yinhe-core / yinhe-editor-core      ← 模型与编辑语义（桌面+移动共享）
yinhe-wgpu                                        ← GPU 渲染
yinhe-audio (+ session)                           ← 引擎/设备/导出（桌面+移动共享）
yinhe-yin / yinhe-archive / yinhe-midi            ← 工程/MIDI/归档

yinhe-mobile-core                                 ← 移动场景投影 + 触屏编辑状态机 + 上传准备
yinhe-flutter                                     ← Android/iOS/OHOS 平台胶水 + FFI 导出 + 纹理
flutter/lib                                       ← Dart UI、布局、本地化、平台存储
```

### 4.5 实施进度（2026-11-18）

- ✅ 已新增 `crates/yinhe-mobile-core`（commit `bb460493`）：`scene/*`、
  `edit/*`、音频会话、`doc_job`、`autosave`、MIDI 导入导出、库、音色库归档、
  选区物化、惯性、力度缓存、主题派生色、视口尺寸。
- ✅ `yinhe-flutter` 只保留平台胶水：Android/iOS/OHOS 入口、纹理/surface、
  `render`、`dart_ffi` 导出层。
- 当前规模：`yinhe-mobile-core` 12,950 物理行（生产约 11.1k），
  `yinhe-flutter` 4,074 物理行（生产约 3.2k）。
- ⏳ 尚未做：`yinhe-audio::session`（桌面/移动共享音频会话）、
  最近文件/设置持久化移 Dart、autosave/MIDI IO 的进一步共用。
- Flutter 端测试按用户要求延后。

### 4.6 建议顺序与工作量

| 阶段 | 内容 | 收益 | 工作量 |
|---|---|---|---:|
| 1 | 把 `scene/*` 抽到 `yinhe-mobile-core` | 可桌面测试场景投影 | 0.5~1 天 |
| 2 | 把 `edit/*` 抽到 `yinhe-mobile-core` | 可桌面测试手势/几何/提交 | 1~1.5 天 |
| 3 | `yinhe-audio::session` 统一两端音频会话 | 消除音频行为漂移 | 1~1.5 天 |
| 4 | 最近文件/设置持久化移 Dart | 减少 Rust 平台 IO | 0.5~1 天 |
| 5 | autosave / midi import-export 共用 | 减少重复 | 0.5~1 天 |

**总计约 3.5~6 人日**，全部是结构整理，不改变移动端功能。

---

## 5. 需要记录的决策

1. 滚动条保持 `widgets/scrollbar`，不拆 crate。
2. 框选保持「共享选区模型 + 各视图手势」；不强行统一状态机。
3. 音频不再拆 crate，除非出现无 cpal 目标或设备层分叉。
4. 移动端保留；`yinhe-flutter` 拆为「平台胶水 + `yinhe-mobile-core`」两层。
5. 桌面/移动共享音频会话，优先做成 `yinhe-audio::session` 模块而不是新 crate。
6. Dart 负责表现与平台存储，Rust 负责模型、编辑语义、渲染投影。
