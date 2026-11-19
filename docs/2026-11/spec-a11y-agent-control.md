# yinhe 无障碍与 Agent 控制：审计、修复与边界

日期：2026-11-19
范围：桌面端（`crates/yinhe-egui`）、移动端（`flutter/` + `crates/yinhe-flutter` + `crates/yinhe-mobile-core`）、程序化控制入口。

本文分三部分：

1. 现状审计：哪些地方对读屏、键盘、辅助技术、外部 Agent 不可见/不可操作。
2. 本轮已实施：低风险、不会让 1 亿音符性能下降的修复。
3. 暂缓项与性能红线：为什么不做，什么时候能做。

---

## 一、结论摘要

| 维度 | 桌面端 | 移动端 |
|---|---|---|
| 读屏：外壳 | 部分可用（标准控件 + 少量自绘名称） | 部分可用（Flutter 框架默认语义） |
| 读屏：编辑器内容 | 几乎为零 | 零 |
| 键盘：外壳 | 部分可用（Tab/Space/Enter） | 几乎没有 |
| 键盘：音符编辑 | 零（只能鼠标/触控） | 零 |
| 视觉适配 | 部分（主题多、缩放半覆盖） | 部分（系统字号被截断、画布不跟随） |
| Agent/自动化 | 极弱（只有打开文件 + 插件扫描） | 极弱（无 resource-id、无深链） |

核心判断：

- 桌面端 AccessKit 通道已经打开（`eframe` 默认 feature），但 `piano_view/`、`arrange/`、`mix/`、`right_panel/` 四个核心模块的直接无障碍注册数为 **0**。
- 移动端 `Semantics` 为 **0 处**，编辑器主体是一张 Rust/wgpu 纹理，读屏只能看到 Flutter 外壳。
- 现有自动化入口只有「启动参数打开文件」和 `--scan-plugins`；没有状态查询、命令调用、截图、稳定 automation id。
- **音符不适合逐音符建 AX 节点**。1 亿音符场景下，正确路径是「外壳走 AX，内容走文本化事件列表 + 命令/状态接口」。

---

## 二、桌面端审计

### 2.1 已经具备

- `eframe 0.37` 未关闭默认 feature，`accesskit` 已启用；AccessKit 在读屏/AX 客户端请求初始树时才懒启用。
- 标准 egui 控件（Button、TextEdit、DragValue、Checkbox、ComboBox）自带节点。
- 自绘控件有一层 helper：`crates/yinhe-egui/src/widgets/a11y.rs`。
- 走带时间码的 5 个 `TextEdit` 有可读名称（`widgets/timecode.rs:206-303`）。
- egui 对聚焦的可点击控件支持 Space/Enter；Escape 清焦点。
- `yinhe-editor-core/src/shortcuts.rs` 有约 67 个可配动作，支持一个动作多快捷键。

### 2.2 覆盖缺口

| 模块 | 直接注册 | 问题 |
|---|---|---|
| 设置/对话框 | 有 | 标准控件为主，相对最好；但原有 a11y 测试没有真正断言 |
| 走带/菜单/时间码 | 有 | 名称基本齐；开关类缺少 selected/checked |
| 标题栏 | 无 | 文档 tab 是手算矩形 + painter；非 macOS 窗口按钮只画不注册 |
| 钢琴卷帘 | 0 | 画布 `Sense::hover()`（`piano_view.rs:106-108`），连焦点都没有；音符/选区/播放头/标尺/事件行不可读 |
| AR 走带 | 0 | 整个轨道面板只有一个无名交互区（`arrange/track_panel.rs:132`）；行名、M/S、可见/锁定全是 painter |
| MIX | 0 | 推子、声像、SEND、M/S、通道选中全部无名；推子无键盘 |
| 事件浏览器 | 0 | 每个 cell 一个无名可聚焦交互；音符页 100 行 × 9 列，最多约 900 个 Tab 停靠点 |
| 设备链 | 少量 | 图标按钮 label 是 Material 私有码位；旋钮空名；旁通只靠颜色 |
| Toast/导出/错误 | 少量 | 只有标题 Label；无 live region、无 ProgressIndicator、无 AlertDialog |
| 表计 | 少量 | `Role::Meter` 但没有电平数值 |

典型「读屏听到什么」：

- 混音推子：`click_and_drag` + 无 `widget_info`（`mix/strip/controls.rs:468-516`）→ 未知控件。
- 旋钮：`WidgetInfo::slider(enabled, value, "")`（`widgets/knob.rs:86`）→ 有数值、没有名字。
- 自绘滑杆：`unnamed_input(Role::Slider)`（`widgets/slider.rs:36`）→ 无名、无值、方向键无效。
- 静音圆点：`Sense::click` + 纯颜色（`widgets/audibility.rs:23-32`）→ 无名、无 checked，且只靠颜色。
- 模式栏 ARRANGE/MIX/EDIT：只有 `Role::Button`，没有 selected。

### 2.3 键盘、焦点、播报

**焦点环**：只有 knob、slider、switch、combo、menu、numeric_input、text_input、dialog_buttons 等少数自绘件有。`flat_button*`、`hover_button*`、stepper、radio、audibility、滚动条、时间标尺、事件行、`rows::list_row`、MIX/设备卡/表计/标题栏 tab 都没有。

**Tab 顺序**：按绘制顺序注册。主循环里标题栏 → 走带栏 → 底部模式栏 → 设备链 → 表计 → 主内容 → 右栏（`app/main_loop.rs:508,658,808,863-870`），Tab 会从顶部跳到屏幕底部再跳回中部。

**焦点管理**：全仓没有 `set_focus`/`focus_lock`；`request_focus` 只有 4 处。打开错误弹窗/普通对话框不会把焦点送进去。

**hover-only 信息不可达**：大量 `hover_popup`/`hover` 提示只画 tooltip，不进 AX 树；图层面板的 lock/眼睛只在 hover 时才创建 `ui.interact`（`right_panel/info_panel/layers.rs:196-201,254-259`），键盘永远到不了。

**全局快捷键的焦点门槛用错了 API**：

- `app/actions.rs:52` 用 `ui.ctx().egui_wants_keyboard_input()` 判断「是否在输入文本」。
- egui 0.37 里这个 API 的含义是 **任意控件有焦点**（`Context::egui_wants_keyboard_input` = `memory.focused().is_some()`）。
- 后果：Tab 到任意按钮/表格 cell 后，Cmd+Z、Cmd+S 等全部失效，直到按 Escape 或重新点击空白。
- 同一个错误影响 `right_panel/event_browser/table.rs:384`：表格 cell 获得焦点后 Delete 被误判为文本输入。
- 同一个变量还用于 macOS 原生菜单加速键暂停（`app/main_loop.rs:167-170`）；VoiceOver 导航到任何节点时，AccessKit 的 Focus action 会设置 `focused_widget`，于是原生菜单加速键也被暂停。

**自定义 Switch 的键盘行为可能失效**：`widgets/switch.rs:110` 先 `resp.clicked()` 翻转一次，`:112-117` 又手动翻转一次；egui 对聚焦可点击控件的 Space/Enter 会合成 `FAKE_PRIMARY_CLICKED`，`Response::clicked()` 包含它。按源码推导是「翻转两次 = 不翻转」。

**自绘 Slider/Knob 不处理 AccessKit 动作**：没有 `Increment`/`Decrement`/`SetValue` 处理，读屏的增减手势无效。

**没有 live region**：全仓没有 `set_live`、`Role::Alert`、`Role::ProgressIndicator`。导出进度、Toast、错误不会主动播报；播放位置也不会。

### 2.4 现有 a11y 测试是假绿

`dialogs/settings.rs:499-567` 的注释写「kittest 默认会在有输入控件缺少名称时 panic」，但：

- `egui_kittest` 的 `check_accessibility()` 只在 `Harness::run()`/`try_run_impl` 和 snapshot 前调用；
- 仓库里 23 处 `harness.step()`，0 处 `harness.run()`/`check_accessibility()`；
- `step()` 只渲染，不触发检查。

所以设置页、欢迎页、对话框按钮这些「a11y 测试」目前只证明「能渲染」，不证明「有名称」。这是本轮最先修的地方。

### 2.5 视觉与输入

- 字体缩放覆盖不全：标尺、力度表、事件行、模式栏提示、钢琴键标签、时间码大量直接用常量或裸数字。
- 最小字号没有下限：10px × 0.75 字体 × 0.75 UI ≈ 5.6px；egui 自带 Cmd+/- 还能绕过 0.75。
- 两套缩放状态：设置里的 `ui_scale` 和 egui `zoom_factor` 共用，Cmd+0 会重置设置。
- 对比度：默认暗色 `text_label` 约 4.44:1（正文 AA 不达标），`tick_label` 约 1.8:1；部分预设更差。
- 色盲：M/S、solo、旁通、模式选中只靠颜色；轨道颜色没有第二编码。
- 减少动态效果：没有。开关缓动、跟随平滑、翻页、电平表、移动端惯性/长按圆环都关不掉。
- 桌面不跟随系统明暗；移动端反而有「浅色/深色/跟随系统」。
- 实时 MIDI 只处理 NoteOn/NoteOff，CC64 延音踏板、CC1、PitchBend 被丢弃（`app/midi_input.rs:147-179`）。

---

## 三、移动端审计

### 3.1 架构

- Flutter 壳负责顶栏、侧栏、对话框、设置、事件列表。
- 编辑器主体是一张 Rust/wgpu 纹理：`GestureDetector → Listener → Texture`（`flutter/lib/app/shell_build.dart:405-431`）。
- 标尺、播放头、网格、轨道名、力度条由 Flutter `CustomPainter` 画；音符/键盘/自动化曲线在纹理里。
- 三个平台壳只做 Surface/尺寸/文件选择，没有任何无障碍节点桥接。

### 3.2 语义现状

- `flutter/lib` 全目录 `Semantics`/`SemanticsProperties`/`semanticLabel`/`semanticsBuilder`/`SemanticsService` 为 0。
- 外壳靠 Flutter 框架默认语义：`IconButton(tooltip:)`、`ListTile(Text)`、`AlertDialog`。
- 编辑器内容完全不可见：画布只是一个 `Texture` 叶子节点。
- `CustomPainter.semanticsBuilder` 全部为 null。
- 事件列表（`flutter/lib/dialogs/event_list_dialog.dart`）是唯一文本化编辑面，用 `ListTile`，相对可读；但入口只有「长按 AR 轨道 → 菜单 → 事件列表」，而长按需要先能在画布上定位轨道。

### 3.3 危险点：TalkBack 双击画布可能误建音符

`GestureDetector` 有 `onTapUp` 和 `onLongPressStart`（`shell_build.dart:407-413`），没有 `excludeFromSemantics`。Flutter 的默认语义委托只要存在 TapGestureRecognizer 就会生成 tap action，并以**控件中心**合成 `onTapUp`。而 `_handleTapUp` 在钢琴卷帘空白处会 `rust.tapNote(...)`（`shell_input.dart:123-150`）。

所以读屏用户「双击画布」很可能不是安全地无事发生，而是在画布中心写一个音符。这是静态推导，真机开 TalkBack 需要再验证一次。

### 3.4 手势、键盘、尺寸

- 进入钢琴卷帘：只能双击 AR 轨道内容；没有 AR→PR 的按钮。
- 缩放：只有双指捏合；没有 +/- 按钮。
- 框选：长按 500ms 后拖动，或选框工具。
- 插入事件：长按 → 浮动菜单；无顶栏入口。
- 外接键盘：`Shortcuts`/`LogicalKeyboardKey`/`KeyEvent` 全为 0；只有 Ctrl+滚轮缩放。
- 外接鼠标：右键被当左键，没有 hover、没有右键菜单。
- 触控目标：顶栏 40×40，EVENT 行高 18，M/S 18~26，画布菜单按钮 `Size.zero` + `shrinkWrap`，自动化手柄命中 4px。
- 字体：顶栏 `maxScaleFactor: 1.2`；画布自绘文字完全不读 `MediaQuery.textScaler`。

### 3.5 移动端自动化

- 没有 `resource-id`；没有 `SemanticsProperties.identifier`。
- 没有 AccessibilityService。
- 没有 deep link/URL scheme/`ACTION_VIEW`；Android 只有 MAIN/LAUNCHER。
- 没有 `integration_test`/`enableFlutterDriverExtension`/Appium。
- `uiautomator dump` 只能看到 Flutter 语义树里的外壳控件；画布是单个无标签纹理。
- 调试入口只有 `cargo xtask emulator`。

---

## 四、Agent / 电脑自动控制审计

### 4.1 现有入口

| 入口 | 能力 | 边界 |
|---|---|---|
| 启动参数传文件 | `app.rs:534-546` | 只能打开，不能控制；无 `--help`/`--version` |
| `--scan-plugins` | 子进程 headless 扫描单个插件，stdout JSON | 只有插件元数据，没有参数/工程状态 |
| macOS 文件关联 | `application:openFiles:` | 只能把文件路由进已运行实例；Windows/Linux 二次启动是新进程 |
| `.yin` | 前两段 `project.json`/`mapping.json` 是 JSON；`data` 段是 postcard+zstd | 外部能读元数据，读不了音符/自动化 |
| 剪贴板 | `yinhe-clip/1/<id>` + 临时 `.yhcl` | 可注入音符/自动化，但依赖 UI 焦点和粘贴 |
| 移动端 FFI | 约 190 个 C 函数 + JSON 场景读取 | 只有 Flutter 进程内可用 |
| 快捷键表 | 约 67 个动作 | 可枚举、可持久化；部分菜单命令没有 action id |

### 4.2 完全缺失

- 通用 CLI：`--headless`、`--inspect`、`--batch`、`--export`、`--print-state`、`--list-commands`、`--screenshot`、`--version`、`--help`。
- IPC：Unix socket、TCP、命名管道、HTTP、WebSocket、JSON-RPC、OSC、MCP。
- 运行中状态 dump：工程、选择、播放、视图、撤销栈、任务进度。
- 稳定 automation id：没有 `set_author_id`/`set_html_id`。
- 窗口标题固定为 "Yinhe"，多文档无法区分。
- 单实例转发、截图、插件参数外部化、MIDI remote/OSC/MIDI learn。
- 命令 ID 不完整：`InsertAction::action_id()` 返回空串（`chrome/insert_actions.rs:75-77`）；`PlayMenuAction` 的 record/step/tap/automation write/loop/loop selection 返回空串（`transport_bar_actions.rs:402-407`）；`ViewAction` 全部返回空串（`:490-492`）；`FileAction` 的 ImportMidi/ExportMidi7z/Welcome/About 也有空串（`:88-92`）。

### 4.3 为什么纯 AX 驱动不了核心编辑

- 音符不是 widget，是 GPU 绘制数据；核心目录没有直接 `widget_info`。
- 1 亿音符不可能全量建 AX 节点；只建视口内节点也要和滚动/缩放/虚拟化耦合。
- 没有 `author_id`，外部只能按本地化 label 找控件。
- 画布是 10px 级密集自绘文字，OCR/模板匹配在 100 套主题/深浅色/DPI 下都不稳定。
- 没有状态导出，Agent 无法验证「操作真的生效」。

---

## 五、性能红线（本轮必须遵守）

以下方案会让 1 亿音符场景变慢或吃内存，**本轮不做**：

1. **逐音符 AX 节点**：每个音符一个 AccessKit 节点，1 亿音符直接爆炸。即使只建视口内，也会和 GPU 裁剪/LOD 重复计算。
2. **每帧全量重建语义树**：MIX/轨道/事件表都不能每帧遍历全模型。只允许 O(可见行/O(轨道数) 的注册。
3. **`--inspect` 全量加载**：不能为了输出 JSON 就 `load_yin` 解压全部音符。只读 `project.json` + `mapping.json`。
4. **状态快照包含全部音符**：Agent 状态查询必须分页/按区间，不能一次返回全曲。
5. **画布 semantical 逐元素同步**：不做「音符变化 → AX 节点增量同步」的实时链路。
6. **移动端把每个音符做成 Flutter Semantics**：纹理里的音符不做逐音符语义节点；只做画布级语义动作 + 文本化事件列表。

这些不是「永远不做」，而是「等有真实读屏用户/Agent 需求，并且有独立的可见区语义层时再做」。

---

## 六、本轮实施（2026-11-19）

### 6.1 已修复

| 提交 | 范围 | 内容 |
|---|---|---|
| `6e8abac8` | 桌面 | a11y 测试改为显式 `check_accessibility()`；补 `files` 分类；全局快捷键改用 `text_edit_focused()`；修复开关 Space 双翻转 |
| `e289cdd7` | 桌面 | Toast 标题 Polite live、进度 ProgressIndicator + numeric value；错误弹窗 AlertDialog + Assertive live + 焦点送「确定」+ Esc；命令按钮 `author_id`；模式栏 Tab + selected；flat/hover/列表行/步进器/单选/听感圆点焦点环 |
| `e531734e` | 桌面 | 通道条整卡 ListItem + selected；推子/声像/发送 Slider + 名称；dB 名称；SEND/听感 CheckBox + checked；事件表格 Cell/RowHeader 名称；15 个语言文件补 `mix.gain/pan/send/audible` |
| `f7fe7959` | 桌面 | 自绘 Slider/旋钮支持方向键、PageUp/Down、Home/End、AccessKit Increment/Decrement；Slider 写入 numeric value |
| `bca7d4d1` | 桌面 | 轨道行只读 ListItem + 名称/静音/独奏/音符数 + bounds（不新增 interact）；标题栏文档 tab、Yinhe 入口、非 macOS 窗口按钮只读节点 |
| `a42df966` | 桌面 | popup 菜单行 `author_id`；设备链图标按钮用提示覆盖 Material 码位；电源 CheckBox；右栏 dock tab + 焦点环；图层锁定/可见从 hover-only 改为常驻交互 + CheckBox |
| `7cbc4f86` | 桌面 | 模式栏 ARRANGE/MIX/EDIT 与右栏 dock tab 写稳定命令 ID |
| `958a86c0` | 桌面 | 总输出表 Meter 写电平数值；AR「添加音轨」行补名称 |
| `56ad654f` | CLI | `--inspect`（只读 project/mapping）、`--list-commands`（67 条 CommandId）、`--version`、`--help`；`yinhe-yin` 新增 `read_mapping_file` |
| `3764e3ed` | 移动 | 侧栏「事件列表」入口；画布 `Semantics + ExcludeSemantics`，移除自动 tap/longPress，改为「朗读位置与选区 / 打开事件列表 / 播放或暂停」 |
| `85ecdd3b` | 移动 | 画布 FocusNode + 外接键盘：Space 播放/暂停、Cmd/Ctrl+Z 撤销、Shift 重做、Cmd/Ctrl+S 保存、PR 下 Esc 返回 AR |

### 6.2 验证

自动化测试（全部通过）：

```text
cargo test -p yinhe-egui a11y                                 3 passed
cargo test -p yinhe-egui widgets::switch::tests               2 passed
cargo test -p yinhe-egui error_modal                          1 passed
cargo test -p yinhe-egui mix::strip                          10 passed
cargo test -p yinhe-egui arrow_key_changes_value              2 passed
cargo test -p yinhe-egui doc_tab_is_accessible_tab            1 passed
cargo test -p yinhe-egui cli::tests                           3 passed
cargo test -p yinhe-yin                                      30 passed
cargo clippy -p yinhe-egui -p yinhe-yin --all-targets        clean
flutter analyze                                              No issues
flutter test                                                 49 passed
```

CLI 手工验证：

```text
$ cargo run -p yinhe-egui -- --version
Yinhe 0.1.0
yinhe-yin format: 10

$ cargo run -p yinhe-egui -- --list-commands | ...
yinhe-cli/1 67
[{'default': ['Cmd+S'], 'id': 'save', 'label_key': 'file.save', 'scope': 'file'}]

$ cargo run -p yinhe-egui -- --inspect /tmp/yinhe-inspect-test.yin
yinhe-cli/1 {'magic': 'YINH', 'version': 10} Agent Demo 960
[{'uuid': 'u-drums', 'name': 'Drums', 'channel': 9, 'muted': False, ...},
 {'uuid': 'u-bass', 'name': 'Bass', 'channel': 9, 'muted': True, ...}]
```

`--inspect` 的测试工程只有 4.4KB，data 段是 4KB 垃圾字节；读取 project/mapping 不触碰 data，证明该入口不会解压音符。

### 6.3 暂缓项与原因

| 暂缓项 | 原因 | 何时再做 |
|---|---|---|
| 画布逐音符 AX 节点 / 移动端逐音符 Semantics | 1 亿音符下节点数和内存不可接受；只建视口内节点也要和 GPU 裁剪/LOD 耦合 | 有真实读屏用户且需要「逐音符浏览」时，单独做可见区语义层 |
| 事件浏览器键盘网格 | 现在是 100 行 × 9 列、每 cell 一个 Tab 停靠点；要改成「整表一个焦点 + 方向键移动当前 cell」需要新的网格状态机 | 下一轮桌面 a11y |
| 本地控制 socket / `apply_notes` | 需要协议、并发、权限、undo group、异步 job 设计；直接做容易把 UI 线程写阻塞 | Phase 2，见 `spec-agent-composition.md` |
| `--export-midi` / `--import-midi` | 需要完整加载工程；1 亿音符会付出全量解压/内存代价。当前先用现有 UI 导出 | 等流式导出/加载路径可复用 |
| 高对比主题、减少动态效果、最小字号 | 不涉及性能红线，但会横跨主题、全部自绘控件和移动端，改动面大 | 单独主题/a11y 迭代 |
| 实时 MIDI CC（延音踏板等） | 功能缺口，不是性能问题；需要决定 MIDI learn/remote 的产品边界 | 单独输入链路迭代 |
| 窗口标题带文档名、单实例转发 | 涉及平台窗口管理和 macOS/Windows 行为 | 与 Agent 控制 socket 一起做 |
| 轨道行节点挂到面板树 | 当前是根级只读节点（读屏可导航，但树层级不完美）；挂父节点需要 egui `accessibility_parent` 或公开 API | 下一轮 egui 语义整理 |
| 模式栏 / dock tab 的 `author_id` | 本轮只给命令按钮/菜单/pinned 动作写了 id；Tab 仍靠 label | 下一轮统一 |
| VoiceOver / TalkBack 真机走查 | 需要人工开读屏、听播报、验证焦点顺序；CI 无法替代 | 每次发布前手工 checklist |

性能证据：

- 轨道行节点只对 `visible_range` 内的行注册，O(可见行数)。
- 通道条命名只对当前可见的 MIX 通道注册，O(通道数)。
- 事件表格命名只对当前页 100 行注册，O(页大小)，不遍历全曲音符。
- `--inspect` 只读两个 JSON 段，O(元数据)，不解压 data。
- 没有任何「音符变化 → AX 节点增量同步」的实时链路。

### 6.4 下一步建议

1. 桌面事件浏览器做成「整表一个焦点 + 方向键当前 cell + Enter 编辑」的网格，消掉 900 个 Tab 停靠点。
2. 实现本地 NDJSON 控制 socket 的第一版：`state` / `project_summary` / `query_notes` / `apply_notes` / `screenshot`。
3. 主题层加高对比模式、对比度校验、系统 reduced motion。
4. 发布前用 VoiceOver（macOS）、Narrator（Windows）、Orca（Linux）、TalkBack（Android）各走一遍核心链路。

---

## 七、后续建议

1. 桌面端先做「事件列表 + 状态/命令接口 + a11y 测试」，再做画布键盘光标。
2. 移动端先做「侧栏事件列表入口 + 画布语义安全动作 + 外接键盘基础键」，再做完整 Semantics 覆盖。
3. Agent 控制先做「命令注册表 + 状态快照 + 只读 CLI」，再做本地控制 socket，最后才是 OSC/MCP。
4. 所有新控件默认带名称、角色、值、焦点环；所有新命令默认有稳定 `CommandId`。
