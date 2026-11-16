# 移动端新手教程（Onboarding）Spec

状态：草案 v1（2026-11-16）

适用端：Flutter 移动端（Android / iOS / OHOS），不改桌面端。

相关代码：

- 启动流程：`flutter/lib/app/shell_core.dart` 的 `_init()`（482 行）
- 主壳与侧栏：`flutter/lib/app/shell_build.dart`（227~247 行、815 行）
- 侧栏菜单：`flutter/lib/widgets/sidebar.dart`
- 设置页：`flutter/lib/widgets/settings_view.dart`
- 设置持久化：`crates/yinhe-flutter/src/scene/settings.rs`
- 多指手势：`flutter/lib/core/multi_finger_gesture.dart`

## 1. 背景

移动端现在的启动路径是：进入空白 AR → 自动打开侧栏，让用户选文件或设置。
对第一次接触 DAW 的用户，这条路径有三个问题：

1. 侧栏一上来就是「新建 / 打开 / 导入 MIDI / 导出 / 音色库」十几个选项，没有先后顺序。
2. 画布的核心操作大量依赖手势：双指缩放、长按框选、双指轻点撤销、三指轻点重做。这些在界面上没有可见入口，新用户不知道。
3. 黑乐谱用户年龄偏小（10~18 岁多），对「音轨 / 钢琴卷帘 / 量化 / 自动化 / gate」这套术语没有耐心，需要的是「先做出声音和画面」。

所以教程不是功能说明书，而是一段 60~90 秒的「第一次成功体验」：

> 新建 → 加一条轨 → 画几个音符 → 播放 → 保存。

## 2. 目标与非目标

### 目标

- 首次启动自动出现，用户可以不看，也可以以后不再自动出现。
- 6~7 页内讲完「界面在哪、怎么画、手势怎么用、怎么保存」。
- 教会 4 个隐藏手势：双指捏合缩放、双指拖动平移、长按框选、双指轻点撤销。
- 菜单栏（AR 左侧抽屉）随时可以重新打开教程。
- 教程关闭后不留下任何状态，不改变用户的工程内容。
- 纯本地，不联网、不统计、不登录。

### 非目标

- 不做乐理教学（音阶、和弦、配器）。
- 不做强制任务检测（不要求用户必须真的画出音符才能进入下一页）。
- 不做视频播放器，不引入图片/视频资源包；示意图全部 Flutter 自绘，跟随主题和语言。
- 不做桌面端教程。
- v1 不做「教程练习工程」和「成就系统」。

## 3. 用户画像与设计原则

典型用户：10~18 岁，手机原生长大，玩黑乐谱/音游，愿意动手试，但耐心只有几十秒；看到术语会直接退出；喜欢「点一下就有反馈」。

因此：

1. **每页只教一件事。** 一页一个标题、两句话、一个示意图、一个主按钮。
2. **先做，后解释。** 先说「点这里」，再补一句为什么；不先讲概念。
3. **不用黑话。** 正文不出现 PR / AR / AM / gate / tick / 量化 / 自动化。必须出现时用「音符网格」「音符长度」替代。
4. **不吓人。** 明确告诉用户「画错可以撤销」「工程会自动备份」「随时可以退出教程」。
5. **短。** 正文每页不超过 40 个汉字；整段教程 90 秒内可读完。
6. **可跳、可回、可重看。** 右上角始终有关闭，底部进度点可点，菜单里永远能找到教程。
7. **不自动出声。** 教程不自动播放音乐；只有用户点「试听」才播放。

## 4. 功能清单与优先级

| 优先级 | 功能 | 说明 |
|---|---|---|
| P0 | 首次启动自动教程 | 全屏 7 页，默认开启，可关闭 |
| P0 | 「不再自动显示」 | 教程底部固定入口；关闭后写设置 |
| P0 | 侧栏「新手教程」入口 | 设置上方，随时重看，不影响自动弹出开关 |
| P0 | 设置页「启动时显示教程」开关 | 可重新开启自动弹出 |
| P1 | 空白工程引导卡 | 没有音轨时，画布中央给「新建音轨 / 打开工程 / 导入 MIDI」三个入口 |
| P1 | 无声诊断提示 | 播放但没声音时提示「先选一个音色库」，避免新手以为软件坏了 |
| P1 | 手势速查页 | 菜单 → 新手教程 → 手势，一屏看完所有手势 |
| P2 | 交互练习场 | 教程内嵌一个玩具网格，让用户安全地画/拖/缩放 |
| P2 | 示例工程 / 模板 | 「8 轨黑乐谱模板」「钢琴模板」，一键开始 |
| P2 | 完成清单 | 画 8 个音符 / 播放一次 / 保存一次，本地打勾，不联网 |

本文档详细设计 P0；P1/P2 只给方向，不展开实现。

## 5. 触发规则与持久化

### 5.1 一个字段就够

在 `settings.txt` 增加一个字段：

```text
show_tutorial=1
```

- `1`：下次启动自动弹出教程。
- `0`：不自动弹；菜单里仍可手动打开。
- 文件里没有这个字段（老用户升级）：按 `1` 处理，让老用户也看一次。
- 默认值：`1`。

不需要 `tutorial_seen`、不需要版本号、不需要「每天一次」这类阈值。任何一次主动关闭（完成 / 跳过 / 不再显示）都写入 `0`；用户在设置页手动打开开关时写回 `1`。

### 5.2 启动时机

现在 `_init()` 在恢复自动保存后，用 `addPostFrameCallback` 自动打开侧栏（`shell_core.dart` 495~500 行）。改成二选一：

```text
纹理就绪
  → 恢复自动保存（可能弹恢复对话框）
  → show_tutorial == 1 ?
      是：弹教程（不打开侧栏）
      否：打开侧栏（保持现状）
```

规则：

- 自动保存恢复对话框优先于教程，避免两层浮层叠加。
- 教程打开前，如果播放中则暂停（`rust.transport(1)`），并确保抽屉已关闭。
- 教程从菜单打开时，不读也不写 `show_tutorial`，不自动暂停播放以外的任何状态。
- 教程关闭后：如果是首次启动（来源 = 自动），且当前是空工程，打开侧栏，让用户直接开始；如果来源 = 菜单，回到原界面，不打开侧栏。

### 5.3 关闭路径

| 操作 | 行为 | 写入 `show_tutorial` |
|---|---|---|
| 最后一页「开始创作」 | 关闭教程，空工程时打开侧栏 | 0 |
| 右上角关闭 | 关闭教程 | 0 |
| 底部「不再自动显示」 | 关闭教程 | 0 |
| 菜单里看完教程 | 关闭教程 | 不写 |
| 设置页开关 | 只改开关 | 1 / 0 |

上表的写入只针对「启动自动弹出」的教程；从菜单打开的教程，无论怎么关闭都不写 `show_tutorial`。

这样默认只会自动弹一次；用户想每次都看，可以在设置里打开。跳过和「不再显示」行为一致，但文案分别是「关闭」和「不再自动显示」，后者更明确，满足「我不想再看到它」的心理预期。

### 5.4 返回键

- 教程第 2 页及以后：系统返回 = 上一页。
- 教程第 1 页：系统返回 = 关闭教程（写入 0）。
- 教程打开时，Shell 的 `PopScope` 不应抢事件；教程用独立路由处理返回。

## 6. 入口设计

### 6.1 侧栏

`flutter/lib/widgets/sidebar.dart` 在「设置」上方加一项：

```text
[school_outlined] 新手教程
[settings_outlined] 设置
```

- 点击后先收抽屉，再打开教程第 1 页（`_runFromSidebar` 已有这个模式）。
- 不写 `show_tutorial`，不影响下次启动。
- 图标用 Material Icons，不加边框、不加小字说明。

### 6.2 设置页

`settings_view.dart` 的「编辑」页最后加一个开关：

```text
启动时显示教程
每次打开应用时自动进入新手教程
```

（副标题是设置项说明，允许；但不要在真实控件下面加操作教学小字。）

打开开关写 `show_tutorial=1`，关闭写 `0`。开关只影响「启动是否自动弹」，不影响菜单入口。

### 6.3 空白工程引导（P1）

不是教程的一部分，但和教程衔接：

- 当前工程没有任何音轨时，AR 画布中央显示三个大按钮：新建音轨 / 打开工程 / 导入 MIDI。
- 用户加第一条轨后自动消失，不写任何「已读」状态。
- 不加边框，用控件底色区分；不要写「点击这里开始」这类小字。

## 7. 页面流程与文案

### 7.1 流程图

```text
[1 欢迎]
   ├─ 开始教程 → [2 界面] → [3 加轨] → [4 画音符] → [5 手势] → [6 撤销] → [7 保存]
   └─ 直接创作 → 关闭教程，打开侧栏

任意页：右上角关闭 / 底部「不再自动显示」/ 系统返回
```

### 7.2 每页内容

#### 第 1 页：欢迎

- 标题：三分钟，做出你的第一段黑乐谱
- 正文：不用学乐理，跟着点就行。画错了可以撤销，工程也会自动备份。
- 主按钮：开始
- 次按钮：直接开始创作
- 底部：不再自动显示
- 示意：一条从左到右生长的音符带，最后变成播放头扫过。

#### 第 2 页：认识三个地方

- 标题：先记住三个地方
- 正文：左上角是菜单，顶部是播放和保存，中间是时间轴。音符都画在时间轴上。
- 按钮：下一步
- 示意：AR 界面简图，用三个强调色圆点标出「菜单」「播放/保存」「时间轴」。
- 不讲轨道面板、事件行、自动化，留到以后。

#### 第 3 页：建一条音轨

- 标题：先有一条自己的音轨
- 正文：点左上角菜单 → 新建工程，再到轨道列表末尾点 +，就有了第一条音轨。
- 按钮：下一步
- 示意：侧栏「新建工程」和轨道末尾「+」两个位置的高亮。
- 这一页只告诉路径，不强制用户真的去点。

#### 第 4 页：画第一个音符

- 标题：点一下，就是一个音符
- 正文：双击音轨进入音符网格，选铅笔，在格子上点一下；往右拖，音符会变长。
- 按钮：下一步
- 示意：先用强调色标出顶部铅笔按钮，再演示点出音符 + 向右拖长的两段动画。
- 术语对照：正文用「音符网格」，不写「钢琴卷帘」；术语说明留给「手势速查」这类独立页面，不在控件下面加小字。

#### 第 5 页：手势

- 标题：两根手指，比按钮快
- 正文：双指捏合缩放，双指拖动平移；长按半秒出现圆环，再拖就是框选。
- 按钮：下一步
- 示意：三个循环动画，每个 2.4 秒：
  1. 两个手指相向/相离，网格缩放；
  2. 两个手指同向移动，网格平移；
  3. 一根手指长按出现圆环，拖出选框。

#### 第 6 页：画错了怎么办

- 标题：画错了，双指轻点一下
- 正文：双指轻点撤销，三指轻点重做。顶部也有撤销/重做按钮。工程每 5 分钟自动备份。
- 按钮：下一步
- 示意：双指轻点 → 音符消失；三指轻点 → 音符回来。
- 这一页的目的是降低「不敢画」的心理门槛。

#### 第 7 页：保存与导出

- 标题：做完记得保存
- 正文：顶部软盘图标保存工程；菜单里可以导出音频或 MIDI，直接发给朋友。
- 主按钮：开始创作
- 底部：不再自动显示
- 示意：保存按钮 → 对勾；导出菜单 → 文件飞出。
- 完成页可以顺带展示一个三项清单（不强制）：
  - 画 8 个音符
  - 播放一次
  - 保存一次

### 7.3 英文文案（简版）

教程正文先做中英双语（`L10n.isZh ? 中文 : English`），其余 13 种语言回退英文。理由：15 份 arb 翻译会拖慢 P0，而且现有设置页已有中英双语的先例。后续再迁进 `app_*.arb`。

```text
1  Make your first black MIDI in 3 minutes
   No music theory needed. Mistakes can be undone, and your project is auto-backed up.
2  Three places to remember
   Menu at top-left, play/save at top, timeline in the middle.
3  Create your first track
   Menu → New project, then tap + at the end of the track list.
4  Tap once, get one note
   Double-tap a track, pick the pencil, tap the grid. Drag right to make it longer.
5  Two fingers are faster
   Pinch to zoom, drag with two fingers to pan, long-press half a second to marquee-select.
6  Made a mistake?
   Two-finger tap to undo, three-finger tap to redo. Your work is auto-backed up every 5 minutes.
7  Save your work
   Tap the save icon at top. Export audio or MIDI from the menu.
```

## 8. 视觉与交互规范

- 全屏覆盖，背景用 `YinheTheme.bg` 的半透明纯色，不使用 blur/backdrop filter（低端机贵）。
- 卡片：圆角 12，底色 `controlBg`，靠颜色分层，不加边框。
- 标题 22 / 正文 15 / 按钮 15；正文行高 1.4；支持系统字体放大，超出可滚动。
- 深色/浅色都跟随 `YinheTheme`，不写死颜色。
- 进度点：7 个，当前页用强调色，可点跳转；横屏时放在右侧，竖屏放在底部。
- 右上角关闭按钮 40×40，始终可见。
- 示意图全部 `CustomPaint` 或基础 Widget 拼装，不引入位图；换语言/主题不需要换图。
- 手势动画只在当前页播放，离开页面停止；动画时长 2.4 秒循环，使用 `AnimationController`，不用视频。
- 进入新页时 `HapticFeedback.selectionClick()`；关闭时 `lightImpact()`。不要每帧震动。
- 不使用 emoji；图标全部 Material Icons。
- 不锁方向；横屏左图右文，竖屏上图下文。
- 教程内不自动播放声音；「试听」按钮（如果加）必须显式点击。

## 9. 技术实现

### 9.1 组件拆分

新增目录 `flutter/lib/tutorial/`：

```text
tutorial/
├── tutorial_overlay.dart   全屏外壳、路由、进度点、返回键、横竖屏布局
├── tutorial_steps.dart     页面数据（标题/正文/示意图类型/按钮）
└── tutorial_art.dart       7 张自绘示意图 + 手指动画
```

`tutorial_overlay.dart` 只依赖回调和数据，不直接读写 Rust、不持有 `StudioShell` 引用：

```dart
enum TutorialSource { launch, menu }

class TutorialOverlay extends StatefulWidget {
  const TutorialOverlay({
    required this.source,
    required this.onClose,          // 关闭（完成/跳过/不再显示）
    required this.onStartCreating,  // 最后一页「开始创作」
    required this.onDisable,        // 写 show_tutorial=0 后关闭
  });
}
```

`StudioShell` 只负责：什么时候打开、打开来源、关闭后干什么。教程页面的页码、动画、返回键都在组件内部，避免 `StudioShell` 继续膨胀。

### 9.2 打开方式

用 `showGeneralDialog`（root navigator）而不是在 body 的 Stack 里加状态：

```dart
showGeneralDialog<void>(
  context: context,
  useRootNavigator: true,
  barrierDismissible: false,
  barrierColor: Colors.transparent,
  pageBuilder: (_, _, _) => TutorialOverlay(...),
);
```

原因：

- 能盖住 Scaffold 的 drawer，不依赖「打开前先关抽屉」的时序。
- 自带路由返回键处理，不和 Shell 的 `PopScope` 抢事件。
- `StudioShell` 的 ticker 用 `ModalRoute.of(context)?.isCurrent` 判断是否被遮住；教程路由压上去后会自动跳过 `renderFrame`，教程动画期间不会白烧 GPU（`studio_shell.dart` 612~614 行）。
- 关闭后 ticker 自动恢复，不需要额外恢复逻辑。

### 9.3 设置持久化

Rust 侧沿用 `UI_LANGUAGE` / `UI_THEME` 的模式，单独一个 `OnceLock<Mutex<bool>>`：

- `scene/settings.rs`
  - `SHOW_TUTORIAL: OnceLock<Mutex<bool>>`，默认 `true`。
  - `show_tutorial() -> bool` / `set_show_tutorial(bool)`。
  - `write_settings()` 写 `show_tutorial=1|0`。
  - `load_settings()` 读 `show_tutorial=`，缺省 `true`。
- `dart_ffi/settings.rs`
  - `yinhe_show_tutorial() -> i32`
  - `yinhe_set_show_tutorial(show: i32) -> i32`
- `flutter/lib/core/ffi.dart` 加两个绑定。

不放进 `AudioSettings`：教程不是音频参数，避免以后改音频 JSON 时把 UI 设置一起带进去。

### 9.4 启动改动

`flutter/lib/app/shell_core.dart`：

```text
_init()
  await _restoreAutosaveIfAny()
  addPostFrameCallback:
     if (rust.showTutorial() == 1) _openTutorial(TutorialSource.launch)
     else _scaffoldKey.currentState?.openDrawer()
```

`_openTutorial(source)`：

1. 若正在播放，`rust.transport(1)` 并同步 `_playing = false`。
2. `await showGeneralDialog(...)`；`await` 返回后如果 `!mounted` 直接返回。
3. `source == launch && tracks.isEmpty` 时打开侧栏；菜单来源不打开。
4. 关闭回调里写 `show_tutorial=0`（仅当用户点了完成/关闭/不再显示；菜单来源不写）。

### 9.5 侧栏与设置页

- `ShellBuild` 的 `Sidebar(...)` 增加 `onTutorial` 参数，回调 `() => _runFromSidebar(() => _openTutorial(TutorialSource.menu))`。
- `Sidebar` 增加 `onTutorial` 字段，在「设置」上方插入 `_item(Icons.school_outlined, L10n.isZh ? '新手教程' : 'Tutorial', onTutorial)`。
- `SettingsView` 增加 `showTutorial` 读取/写入，在「编辑」页加 `_switchCard`。设置页只调用 Rust，不直接打开教程。

### 9.6 多语言

- P0：`L10n.isZh ? 中文 : English`，与设置页现有新文案一致。
- P1：把 7 页标题/正文/按钮迁进 `app_en.arb` / `app_zh.arb`，再补 13 种语言。
- 示意图不写文字，避免翻译后布局溢出；需要标签时用 `Text` 叠加，不烘焙进图。

### 9.7 代码边界

- 教程状态（当前页、动画）只在 `TutorialOverlay` 内，Shell 不保存。
- 教程不直接调用 `rust.newProject()` / `rust.addTracks()` / 画音符；它只讲解路径，避免教程失败时污染工程。
- 教程打开期间不做任何文档修改，不触发 undo/redo。
- `TutorialOverlay` 预计 300~400 行；超过 500 行就按「外壳 / 页面 / 插图」拆，不写上帝对象。

## 10. 边界与异常

| 场景 | 处理 |
|---|---|
| 首次启动时自动保存恢复对话框出现 | 先恢复，再教程；不叠加 |
| 自动教程打开时侧栏正开着 | 先 `closeDrawer()`，再弹教程 |
| 教程打开时正在播放 | 暂停播放，不同步修改文档 |
| 横竖屏旋转 | 重新布局，页码和动画进度保留 |
| 系统字体放大 | 正文可滚动，按钮不被挤出屏幕 |
| 低端机 | 纯色遮罩 + 简单 CustomPaint；不使用 blur、不加载大图 |
| 用户从菜单重看 | 不写设置，不自动打开侧栏，不影响下次启动 |
| 空工程点「开始创作」 | 关闭教程后打开侧栏，让用户新建/打开 |
| 已有工程点「开始创作」 | 关闭教程，回到原界面 |
| 教程文案含专有名词 | 必须用用户能懂的说法；英文也不要出现 PR/AR |
| 深色模式 | 示意图颜色全部取 `YinheTheme`，不写死 |
| 无障碍 | 每页 Semantics 读出标题和正文；关闭按钮有 tooltip |

## 11. 验证

### 自动化

- Rust：`settings.txt` 缺省 `show_tutorial` 读到 `true`；写 `0` 后重启读到 `false`；写 `1` 后读到 `true`。
- Flutter widget：
  - `show_tutorial=1` 时启动出现教程第 1 页。
  - 点「直接开始创作」后教程关闭，且设置写 `0`。
  - 第 1 页按系统返回关闭；第 2 页按系统返回回第 1 页。
  - 进度点可跳到指定页；最后一页「开始创作」关闭。
  - 菜单入口能重新打开，且不把 `show_tutorial` 从 `0` 改回 `1`。
  - 横屏 / 竖屏各构建一次，无 overflow。
- 不建议为教程跑整 workspace 测试；只跑 `cargo test -p yinhe-flutter` 和 `flutter test` 相关用例。

### 手工

- Android 模拟器冷启动：首次自动弹；关闭后杀进程重开不再弹；设置里重新打开后重启会弹。
- iOS 模拟器：同样流程 + 横竖屏旋转。
- 深色/浅色、中文/英文各看一遍。
- 系统字体放大到 1.3，检查第 4、7 页不溢出。
- 教程打开时播放器确实暂停；关闭后能继续播放。
- 教程打开期间 GPU/CPU 不应持续渲染编辑器帧（ticker 被遮住）。

### 性能

- 教程翻页动画稳定 60fps（低端 Android 模拟器至少不卡顿）。
- 教程冷启动到第 1 页不超过 300ms（不含引擎初始化）。
- 不新增常驻定时器；动画只在教程路由存活时运行。

## 12. 里程碑

### M1（P0，本次）

- `show_tutorial` 持久化 + FFI。
- 教程 7 页 + 自绘示意图 + 手势动画。
- 启动自动弹出、关闭写 0、菜单入口、设置开关。
- 中英双语。
- 自动化测试 + Android/iOS 手工验证。

### M2（P1）

- 空白工程引导卡。
- 手势速查页（教程内单页，可单独从菜单进入）。
- 无声诊断提示（没音色库/没音轨/没音符）。

### M3（P2）

- 交互练习场。
- 示例工程/模板。
- 完成清单。

## 13. 待确认

1. 默认行为是否按本文档：**只自动弹一次，关闭后写 0，设置里可重新开启**。如果你希望「每次启动都弹，直到点不再显示」，把关闭时写 0 的分支只保留给「不再显示」即可。
2. 教程正文是否先中英双语，还是 P0 就要 15 种语言。
3. 是否需要「精准高亮真实按钮」（spotlight）。本文档 P0 用自绘示意图，避免为高亮改造 TopBar/Sidebar 并处理横竖屏坐标；如果你要，我可以在 M1 之后单独加。
