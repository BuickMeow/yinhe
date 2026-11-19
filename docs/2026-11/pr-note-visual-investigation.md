# PR 音符视觉优化调研：力度填充、拖动把手与音名标签

> 2026-11。针对“音符上还能加哪些适合快速工作的视觉信息”做的代码事实调研。
> 结论先行：力度填充和 hover 把手几乎不花 GPU 成本；音名标签真正的难点不是画字，
> 而是 LOD、遮挡和八度口径。前两个可以直接做，第三个按“文字放得下才画”的连续
> 规则做，不要一开始就上 glyph atlas。
>
> 2026-11-19 更新：三种力度显示方式已实现（`f463f5f7`）；把手 wgpu/egui
> 对比见 5.2/5.3；音名标签暂缓。

## 1. 结论

| 功能 | 推荐落点 | 增量几何 | 增量成本 | 主要风险 | 建议 |
| --- | --- | --- | --- | --- | --- |
| 力度填充 | wgpu fragment shader | 0 三角形，0 draw call | 1~2 个 flat varying，几 ALU；fill rate 不变 | 纵向瀑布流的方向；选中态覆盖力度 | P0，直接替换旧的整块颜色淡化 |
| hover 把手/描边 | hover 用 egui；选中描边用 wgpu fragment | hover 约 10~20 顶点；选中 0 三角形 | O(1) / 几 ALU | 6px 边缘热区在短音符上过大；纵向光标方向 | P0/P1 |
| 音名标签（hover/选中/放大后） | egui overlay | 每个标签 1 个 TextShape | O(标签数) 的 CPU layout/tessellation | 遮挡、八度口径、字体缩放 | P1，先做 hover/选中 |
| 所有可见音符常驻音名 | wgpu glyph atlas | 新纹理管线、采样器、UV、字体图集 | 高 | 新管线、Android GLES、字体栈重复 | P2，暂不做 |

一句话：第 1 个是纯 shader 改动；第 2 个只要限定在 hover/选中态，也不需要新几何；
第 3 个先决定产品目标，再决定用 egui 还是 atlas。

### 性能量级

下表是“相对现有音符绘制”的增量，不含现有成本：

| 功能 | 三角形 | draw call | CPU | GPU | 预期 |
| --- | --- | --- | --- | --- | --- |
| 力度填充 | 0 | 0 | 0 | 每像素几次 ALU | 不可测 |
| hover 把手/outline | 约 10~20 顶点/帧 | 0（egui 合并） | O(1) | 0 | 不可测 |
| 选中 outline | 0 | 0 | 0 | 每像素几次 ALU | 不可测 |
| 音名 100~1000 个 | 每个字形约 2 个三角形 | 最多 1~2 个（字体图集批次） | layout/tessellation O(N) | 纹理采样 O(N) | 需要实测，通常可接受 |
| 音名 10000 个以上 | 约 2 万~10 万三角形 | 最多 1~2 个（字体图集批次） | CPU 先成为瓶颈 | 采样量仍不大 | 风险高，必须 LOD/遮挡 |

所以“性能”不是前两个功能的阻碍；第三个功能的瓶颈在 CPU 文本 layout 和遮挡判断，
不在 GPU 三角形数量。

## 2. 当前实现事实

### 2.1 wgpu 和 egui 的分工

- PR 的 wgpu 离屏纹理里只有音符层（以及 ghost/erase 预览）。背景、调式色带、
  网格、键盘、选框、播放头、标尺、滚动条都由 egui 画。
  - 绘制顺序见 `crates/yinhe-egui/src/piano_view/overlay.rs:116-167`：
    背景 → 色带 → 网格 → wgpu 纹理 → 键盘 → 播放头 → 选框 → 把手。
  - `build_render_job` 的注释也明确写了“Grid lines 已迁移到 egui，wgpu 只负责
    notes 层”（`crates/yinhe-wgpu/src/pianoroll/prepare.rs:122`）。
- wgpu 纹理通过 `egui_renderer.register_native_texture` 注册，再由 egui 当图片
  合成，不在同一个 render pass 里。
- 默认路径是 CPU 构建 `NoteLayerCache` + 异步渲染线程，`use_gpu_cull` 默认
  `false`（`crates/yinhe-editor-core/src/audio_settings.rs:247`）。GPU cull 是
  可选路径，两条路径共用同一套音符 shader 和 pipeline。

### 2.2 音符 instance 和绘制

- `NoteInstance` 是 12 字节 = 3×u32，没有 padding：
  - `start_tick`、`end_tick`、`packed`；
  - `packed` = key(u9) | track(u16) | velocity(u7)，位域已经放满。
  - 定义见 `crates/yinhe-wgpu/src/vertex.rs:116-144`。
- velocity 已经在 instance 里，不需要额外塞字段。选中/ghost 借用了两个 tick 的
  bit31（`vertex.rs:146-152`）。
- 一个音符是一个 quad：4 顶点 + 共享 index buffer `[0,1,2, 1,3,2]`，6 个索引。
  - 顶点布局见 `crates/yinhe-wgpu/src/pipeline.rs:322-389`；
  - 每 4M 实例一个 chunk，`draw_indexed` 一次画一个 chunk
    （`crates/yinhe-wgpu/src/layer.rs:7-10`）。
- 音符 pipeline 关闭了混合（`blend: None`），因为音符是硬边、不透明、无 MSAA
  （`pipeline.rs:322`）。fragment 输出的 alpha 不参与混合，视觉上只有“不透明”
  或“discard”两种结果。
- 默认缩放：`pixels_per_tick = 0.15`，`key_height = 12.0`
  （`crates/yinhe-types/src/piano_roll_view.rs:72-83`）；256 键，调式折叠时行高
  比例 `THIN_RATIO = 0.18`（`crates/yinhe-types/src/fold.rs:13`）。
- 默认缩放下 LOD 已经启用：`select_summary_level(0.15)` 返回 16 tick 的摘要档
  （`crates/yinhe-wgpu/src/pianoroll/summary.rs:29-44`）。摘要段不是单音符，
  velocity 取块内最大值，track 取块内主导轨。

### 2.3 力度、边框和选中态的现状

- 力度目前只影响填充颜色的明度（`crates/yinhe-wgpu/src/shader.wgsl:555-567`）：
  - 暗色主题：`vel=0` 向黑混 50%，`vel=127` 是轨道原色；
  - 亮色主题：向白混；
  - 选中：向与背景对比最强的一极混 70%，优先级高于力度。
- 边框：
  - PR 的边框宽度 = `max(0.05 * key_height, min_border_width)`
    （`shader.wgsl:569-582`）；
  - 有描边时边框颜色 = 轨道色 × 0.4（`shader.wgsl:721-730`）；无描边时直接用
    填充色；
  - `note_outline` 默认 `true`，`min_border_width` 默认 `0.0`
    （`audio_settings.rs:231,240`）。
- `fs_main` 已经有 uv、`half_size`、`border_width`，并用
  `composite_border_fill` 合成“边框 + 填充”（`shader.wgsl:700-751`）。
  所以“把内部分成已填充/未填充两块”不需要新几何，只需要多一个
  `fill_ratio` 和几行 SDF 比较。
- 注意 `VertexOutput` 同时服务 decor、音符和力度条三套 pipeline
  （`shader.wgsl:267-276`，构造函数在 `vs_main`、`note_geometry`、
  `vs_main_velocity`）。新增字段时，非音符路径要写默认值，否则 decor 的
  行为会变。

### 2.4 交互和命中测试

- `hit_test_note` 是“按 key 分桶 + 每桶按 start_tick 二分”：
  - 只查当前 key；查询窗口是 1 tick；
  - 左边界按全曲 `max_note_len` 外扩，长音符多时候选集会被放大；
  - 边缘判定是固定 `EDGE_THRESHOLD_PX = 6.0`；
  - 代码在 `crates/yinhe-egui/src/piano_view/drag/hit.rs:55-102`。
- hover 目前只改光标，不画音符高亮：`update_hover_cursor` 拿到
  `hit_test_note` 的结果后只调用 `set_cursor_icon`，然后丢弃
  （`crates/yinhe-egui/src/piano_view/interaction.rs:257-338`）。
- 音符没有可见把手；现有把手是选框右侧的三个 egui 方块
  （`crates/yinhe-egui/src/selection/draw.rs:116-142`）。
- 纵向瀑布流下，音符边缘的光标仍然是 `ResizeWest/ResizeEast`，没有改成
  `ResizeVertical`（`interaction.rs:297-301`）。这是现有的一处不一致。

### 2.5 文本和字体

- PR 里没有任何逐音符文本。egui 只画键盘 C 标签、标尺、事件行、
  Automation 标签和拖拽 tooltip。
- 键盘 C 标签是显式字号，不跟随 `font_scale`
  （`crates/yinhe-egui/src/piano_view/keyboard.rs:289-296`）。
- 项目没有 `fontdue`、`cosmic-text`、`swash`、`rusttype`、`glyphon`。
  egui 0.37 的文本栈是 HarfRust + Skrifa + vello_cpu，字体选择用 fontique +
  egui_system_font_provider（`crates/yinhe-egui/src/fonts.rs`）。
- `crates/yinhe-wgpu` 里没有任何纹理创建/采样路径，只有 uniform 和 storage
  buffer。要自己做 glyph atlas，等于新开一条文本管线。
- `yinhe_types::key_name()` 目前是 `octave = key / 12`，即 MIDI 60 = `C5`
  （`crates/yinhe-types/src/lib.rs:92-98`），而且全仓没有调用点。桌面键盘和
  Flutter 都是 `key / 12 - 1`，即 MIDI 60 = `C4`。上音符标签前必须先统一
  这个口径，否则键盘栏和音符会差一个八度。

## 3. 成熟 DAW 的做法

| DAW | 力度 | 音名 | 交互 |
| --- | --- | --- | --- |
| Logic Pro | View > Note Labels 可在音符内部显示音高和力度；论坛反馈标签只在音符足够长、放大到一定程度时出现 | 同左 | 边缘拖动，光标变化 |
| Cubase | 力度可在控制器/颜色中查看 | Key Editor 的音符名随缩放显示 | 边缘拖动 |
| Studio One | 力度以钢琴卷帘底部的竖条显示；Alt+拖动音符可直接改力度；音符颜色可选 Velocity | 键盘标签/鼓映射 | Alt+拖动直接改力度 |

参考：

- Logic Pro：<https://support.apple.com/guide/logicpro/view-note-labels-lgcp1dd1322a/mac>
- Cubase Key Editor Note Display：<https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/midi_editors/midi_editors_key_editor_note_display_r.html>
- Studio One：<https://www.soundonsound.com/techniques/studio-one-music-editor-tricks>

共同点：

1. 力度是一等公民，不是装饰。要么有专门的力度条，要么在音符里能直接读到。
2. 音名是 LOD 信息，不是常驻信息。放大到文字放得下才显示，缩小后自然消失。
3. 直接操作优先。能拖音符边缘、能 Alt+拖改力度，就少去一次面板。

对本项目的启示：

- 力度填充是 Logic 的“音符内显示力度”和 Studio One 的“力度竖条”的混合，
  适合“扫一眼就知道哪里轻哪里重”的快速工作流。
- 音名不应该用“可见音符数 < 1 万”这种硬阈值，而应该用“这段文字放不放得下”
  的几何条件，缩放越大自然显示越多。
- 把手应该是 hover 态的临时引导，不是每个音符上的常驻装饰。

## 4. 方案一：力度填充

### 4.1 目标

把“力度 = 整块颜色深浅”改成“力度 = 填充高度 + 未填充区域”：

- 力度 127：填充整个音符内部；
- 力度 64：填充大约一半高度；
- 力度 1：只显示极细的一条填充，未填充部分仍然保留可见的暗色轮廓；
- 选中态暂时保留整块高对比填充；低力度选中音符会变成一条短高亮条，观感待确认，
  后续可改成 selected outline（见 4.4）。

### 4.2 为什么几乎免费

- 还是一个 quad，顶点数、索引数、draw call、instance 数都不变。
- 还是一个 `NoteInstance`，不需要扩到 16 字节，也不需要额外的 per-note buffer。
- 填充率完全不变：同一个矩形内的像素数量不变，只是像素颜色由“整块按力度混色”
  变成“按 uv.y 分界取两个颜色”。
- 唯一新增的是 `VertexOutput` 里一个 flat varying（`fill_ratio`；每顶点
  4 字节），以及 fragment 里几次比较/插值。对 GPU 来说可以忽略。

### 4.3 shader 改法（当前实现）

- `VertexOutput` 只增加一个 flat varying：`fill_ratio`。
- `note_geometry`：
  - 横向 PR 且模式不是深浅颜色时，`fill_ratio = vel / 127`，填充色用稳定轨道色；
  - 纵向 PR、AR、深浅颜色模式、decor，`fill_ratio = 1`，走原来的整块逻辑。
- `fs_main` 用 `note_fill_alpha(inner_a, inner_half, p, fill_ratio, mode)` 切分
  内部矩形：
  - 模式 0：`p.y` 从底部向上；
  - 模式 1：`p.x` 从左向右；
  - 模式 2：直接返回 `inner_a`，不做几何切分。
- 几何填充模式下 `border_color` 固定为 `base_color * 0.4`；这样
  `note_outline=false` 时未填充体仍然可见，不会退化成“整块同色”。
- 非音符路径（decor、力度条）写 `fill_ratio = 1`，行为不变。
- 纵向 PR 不套几何填充：纵向音符的长边是时间长度，按长边填充会把力度和 gate
  混在一起。纵向保持原来的整块颜色淡化。

几个要点：

- 填充按去掉描边后的内部区域计算，所以力度 64 略低于整格的 50%。
- `fill_ratio=0` 时内部全是未填充体，力度 0 的音符仍有可见轮廓。
- 填充边缘用 `smoothstep(-0.5, 0.5, ...)` 保留约 1px 过渡；音符外缘仍是硬边，
  不改变“相邻音符不漏底”的约束。
- 选中态当前仍用 `SELECTED_CONTRAST` 覆盖填充色；低力度选中音符的观感需要
  实机确认，后续再决定是否改成 selected outline。

### 4.4 颜色建议

现在 `border_color = fill_color * 0.4`，而 `fill_color` 又被力度淡化过。改成
填充高度后，如果继续沿用这个公式，低力度音符的未填充区域会非常暗。建议：

- 已填充部分：直接用轨道色，不再按力度淡化。力度只由高度表达，颜色保持稳定。
- 未填充部分：`mix(轨道色, 背景方向, 0.55~0.7)`，或者轨道色 × 0.45。
  暗色主题向黑混、亮色主题向白混，和现有 `dark_theme` 的方向一致。
- 外圈和未填充部分共用同一个 `border_color`，第一版不做“外圈更深”的第二层。
  这样 shader 里仍然只有一组 fill/border 颜色，改动最小。
- 选中态当前仍是 `SELECTED_CONTRAST` 整块高对比填充；后续建议改成“填充保留
  轨道色 + 高对比边框”，这样选中和力度不再互相覆盖。选中 outline 要独立于
  `note_outline`：用户关掉描边时，选中态仍然应该有至少 1px 的轮廓。
- `MAX_FILL_FADE` 不能删：模式 2（深浅颜色）还要用它。模式 0/1 只是不再走
  这条颜色淡化路径，避免“高度 + 颜色”双重编码。

### 4.5 成本

- 0 额外三角形、0 额外 draw call、0 额外 instance。
- +1 个 flat varying；fragment 多几次比较和一次 smoothstep。
- fill rate 不变，瓶颈仍然是 ROP/带宽，不是这几条 ALU。
- 风险不是性能，而是像素密度：默认 `key_height=12px` 时，力度 1 和力度 8
  的填充高度差不到 1px，看不出来。这是缩放问题，不是算法问题；放大后自然可读。

### 4.6 验证

- 同一 MIDI、同一缩放，暗色/亮色主题各截一张图：力度 1、32、64、100、127。
- 三种模式各看一遍：底部向上、从左向右、深浅颜色。
- 检查低力度音符在暗色背景下仍然可见；检查相邻音符之间没有漏底细线。
- 横向 PR、纵向 PR、AR 各看一遍；纵向保持深浅颜色，AR 保持原样。
- 性能：默认 CPU 路径和 GPU cull 路径各跑一次，记录帧时间；预期没有可见差异。

### 4.7 设置项与三种模式（2026-11-19 落地，`f463f5f7`）

设置 → 外观 → 渲染 → “力度显示”，三选一，存
`AudioSettings.velocity_fill_mode`，编码进 `Uniforms.velocity_fill_mode`：

| 编码 | 设置项 | 横向 PR | 纵向 PR |
| --- | --- | --- | --- |
| 0 | 底部向上填充（默认） | 从底部向上 | 保持深浅颜色 |
| 1 | 从左向右填充 | 从左侧向右 | 保持深浅颜色 |
| 2 | 深浅颜色 | 最初的整块颜色淡化 | 同左 |

实现细节：

- `Uniforms` 复用原来的 `fold_pad0` 位置放 `velocity_fill_mode`，总大小仍是
  144 字节，不增加 uniform 带宽。
- 模式只改 uniform，不会让音符层缓存失效；切换设置后下一帧直接生效。
- 旧设置缺字段时按 0（底部向上）加载。
- Flutter 调用 `build_render_job` 时传 2，移动端保持最初的颜色淡化；本次未改
  移动端 UI。
- 模式 0/1 只对横向 PR 生效；纵向 PR 保持模式 2 的行为，原因见 4.3。

## 5. 方案二：拖动把手 / 交互引导

### 5.1 目标

让“哪里能拖、拖了会怎样”可见，但不增加常驻视觉噪音。

### 5.2 wgpu 和 egui 的三种落点

把手不是一个“必须由某个管线画”的东西，先把它拆成三种实现：

| 方案 | 实现在哪 | 每帧增量 | 与音符对齐 | 遮挡表现 | 实现复杂度 | 适合 |
| --- | --- | --- | --- | --- | --- | --- |
| egui overlay | UI 线程，音符纹理之后 | 约 3 个 shape / 30 顶点 / 1 次 egui 批次 | 静态视图完全对齐；滚动/缩放时可能比异步纹理早一帧 | 画在所有音符之上 | 低 | hover 把手首选 |
| wgpu 音符 shader | 音符 quad 内部 SDF | 每个顶点多 4 次比较（4 顶点/实例，100 万实例约 1600 万次/帧） | 与音符同 pass、同纹理 | 被覆盖时随音符一起被盖住 | 中高：Uniforms 布局、shader/cull 同步、移动端验证 | 选中态常驻 outline 可复用 selected 位；不适合为 hover 单独做 |
| wgpu 屏幕空间 overlay pass | 音符之后的小 quad 层 | 2 个 quad / 1 次 draw | 与音符同纹理 | 画在所有音符之上，和 egui 一样 | 中：新 LayerKind 或后置 pass，四条 draw path 都要改 | 如果一定要 wgpu 且要画复杂形状 |

说明：这里比较的是三条实现路径的代码成本和对现有渲染链的影响，不是两套已实现
版本的实测帧时间。egui 的 3 个 shape 与现有选框把手同量级；wgpu 的两条路径的
增量都远小于音符本身，真正的差别是会改到哪些公共结构。

重点：

- 三种方案的增量都远小于现有音符绘制，差别在架构和维护，不在 GPU 性能。
- wgpu 音符 shader 看起来最“原生”，但要给 `Uniforms` 加 hover 字段。
  `Uniforms` 目前 144 字节且和 `cull.wgsl` 共用布局，改一处要同步两套 shader，
  还要过 Android GLES 校验。为 1 个 hover 音符改公共 uniform，不划算。
- wgpu 屏幕空间 overlay 看起来只是“2 个 quad”，但 PR 现在没有“音符之后”的
  layer：`draw_static_layers` 的 decor 在音符之前，而且 decor 的 `vs_main`
  在 `mode == 1 && vel > 0` 时会把实例当成 PR 音符解释。要干净地画，得新增
  `LayerKind::Overlay` 或后置 pass，legacy / cull 摘要 / cull 间接 / Adreno
  四条路径都要照顾。
- egui 的成本是 3 个 shape。现有的选框把手（`selection/draw.rs`）已经每帧画
  3 个 9×9 方块和描边，没有性能问题；hover 再多 2 个 grip 属于同一量级。
- 视觉一致性也是 egui 占优：它可以直接复用选框把手的 `contrast_fg`、`app_bg`
  和圆角风格；wgpu 音符 pipeline 是硬边、无混合，复刻半透明/圆角要么走 SDF，
  要么走 decor，而 decor 在 PR 模式又有上面说的语义冲突。

### 5.3 结论：hover 用 egui，选中 outline 才考虑 wgpu

- **hover 把手用 egui**。O(1)、即时响应、复用选框把手的视觉和主题色、不改
  Uniforms/shader/renderer；这是收益/成本最好的选择。
- **选中音符的常驻 outline 用 wgpu**。它可能是几千个音符，逐个用 egui 画 shape
  会变成 O(选中数)；而 `NoteInstance` 已经有 selected 位，shader 里 0 额外几何
  就能画。
- **不要为 hover 在音符 shader 里加 per-note 分支**。为 1 个音符让所有可见音符
  的顶点都做 hover 比较，性价比低，而且会把 hover 状态耦合进共享的
  AR/移动端 shader。
- **如果以后确实要 wgpu hover**，选屏幕空间 overlay pass，不要把 hover 塞进
  音符几何；但先问“这一帧的对齐”是否值得新增四条 draw path。
- **先修 hit-test 的 z-order**。`hit_test_note` 现在按 `NoteBucket` 的 start_tick
  顺序返回第一个命中，不是绘制顺序的 topmost；重叠音符上可能把把手套在被盖住
  的音符上。无论谁画把手，这个问题都要先解决。

### 5.4 推荐分层

| 状态 | 画什么 | 谁画 | 成本 |
| --- | --- | --- | --- |
| hover | 1px 对比色 outline + 左右两个 2px 竖向 grip | egui overlay | 每帧约 10~20 顶点，O(1) |
| 选中 | 1px 对比色 outline，不画 grip | wgpu fragment（selected 位） | 0 额外几何，几 ALU |
| 拖动中 | 现有 ghost + tooltip | 现有路径 | 不变 |
| 所有音符常驻 | 不做 | - | 视觉噪音，且需要 per-note 状态/LOD |

理由：

- hover 只有一个音符，egui 画 outline/grip 最便宜，也不需要在 GPU 里加
  hover uniform。
- 选中可能有几千个音符，用 egui 逐个画 outline 会变成 O(选中数) 的 shape；
  用 shader 的 `selected` 位改边框颜色，仍然是 0 额外几何。
- 常驻把手在 12px 高的音符上会非常密，黑乐谱下会变成一片小方块；主流 DAW
  也更依赖光标形状和边缘热区，而不是每个音符都画把手。

### 5.5 命中区和视觉要一致

现有 `hit_test_note` 的边缘热区是固定 6px。默认 `key_height=12px` 没问题，
但 1/16 音符在默认缩放下只有约 18px 宽，左右各 6px 之后只剩 6px 可以移动；
更短的音符会完全没有“移动”区域，点击总会变成伸缩。

建议：

- 边缘热区改成 `min(6.0, 音符像素宽 * 0.3)`；
- grip 的宽度和位置跟随这个值；
- 音符窄到放不下两个 grip 时，只画 outline，不画 grip，让光标形状承担提示。

这是行为变更，需要补回归测试：短音符仍然可以移动，长音符仍然可以拖两端。

另外，shader 对 PR 音符强制最小 2px 宽（`shader.wgsl:480-485`），而
`hit_test_note` 用的是原始 tick 宽度。hover outline 和 grip 应该按渲染后的
`max(2px)` 宽度画，否则极短音符的框会比音符本身窄。

### 5.6 纵向瀑布流

- 纵向模式下时间轴在 Y，音符的“左右边缘”实际是屏幕上的上下边缘；
- 光标应该用 `ResizeVertical`，不是现在的 `ResizeWest/ResizeEast`；
- egui 的 grip 也要旋转 90 度，画成上下两个横条；
- 输入框选把手已经有 `view.is_vertical()` 的分支，可以复用同样的判断。

### 5.7 成本

- hover outline + 2 个 grip：约 10~20 个顶点，一次 egui draw call 内合并；
- 选中 outline：0 额外三角形，fragment 多一个 `selected` 分支；描边关闭时也要
  能画出选中轮廓，不能直接复用 `border_width == 0` 的退化路径；
- 不需要 GPU picking，不需要 `map_async` 读回；
- 命中测试复用现有 `hit_test_note`，不增加新的索引结构。

### 5.8 验证

- 手工：1/16、1/32、1 tick 音符；音符中部拖动、左右边缘拖动；
- 纵向 PR：光标方向、grip 方向；
- 选中 1k/10k 音符：确认 shader outline 不影响帧时间；
- 回归：短音符的边缘热区变更后，原有拖动测试全部通过。

### 5.9 把手布局、胶囊工具栏与 Riff 修复（2026-11-19）

**Riff 选框修复（`d4764672`）**

- 拖动 Riff 时选框实时覆盖“原选区 + 重复范围”，不再等松手才动。
- 提交后选框和选中集合一起扩到副本范围；继续拖 Riff 的起点正确。
- 预览只存在 `SelRectState.preview_rects`，不写正式 `rects`；undo 快照
  在清预览之后拍摄，不会把预览框写进历史。
- 桌面和移动端共用 `riff_expanded_rect` 与
  `Document::set_pr_selection_rect`，避免两端公式/选区同步漂移。

**当前三个方块的问题**

- 三个方块都在选框右缘：Riff 右下、Scale 右中、Gate 右中内缩一个量化格。
- 一行高或很窄的选框上，三者的 11px/18px 命中半径必然重叠；Gate 还会因为
  选框宽度不足一个量化格而直接隐藏。
- 九向自定义位置解决不了“选框本身很矮”的重叠，只是把重叠换个地方。

**本次落地（2026-11-19）**

- 选择动作 12 按钮回到 transport bar，外加边框分组；胶囊只保留直线/剪刀的
  确认按钮。
- 动作组图标 20px、间距 8px；移调/变速/垂直翻转的图标旋转恢复。
- 两个外框统一左右/上下都是 2px 内边距；动作组再补 3px 内衬，视觉边距与
  三联选择器一致。
- transport bar 第二组是 **Gate / Scale / Riff 三联选择器**，选中态用强调色；
  两个外框都是 32px 高，与菜单按钮 `TRANSPORT_BTN_SIZE` 对齐。
- PR 选框的三个控制点删除；统一拖选框右缘（纵向为时间终点边），按三联选择器
  当前模式执行 Gate / Scale / Riff；框内拖动仍是整体移动。
- 拖动模式保存在 egui data；PR 可见时显示选择器。AR 仍保留原有控制点，
  本次未跟进。
- `draw_hover_tooltip` 从 `view_interaction.rs` 移到
  `widgets/tooltip.rs`，调用方统一走 `crate::widgets::tooltip`。

**单锚点胶囊方案（未采用，保留为后续候选）**

一个锚点放在选框右侧留白，按下展开 Gate / Scale / Riff 三个图标：

| 状态 | 行为 |
| --- | --- |
| idle | 小圆，显示上次使用的工具图标 |
| 按下 | 立即展开 Gate / Scale / Riff |
| 按住后拖动超过阈值 | 用上次工具，胶囊缩成圆，开始拖动，圆形保持到松手 |
| 松开且未拖 | 保持展开，等用户点选工具 |
| 点选工具 | 胶囊缩回圆，记住该工具；下次按下直接拖就用它 |
| 选区变化 / 点击外部 | 关闭胶囊 |

几个关键约束：

- 锚点在选区内**外侧**，不抢框内密集音符的点击。即使框内全是音符，抓锚点
  也不会误触批量 gate/scale。
- 选区贴边、没有右侧留白时，胶囊改为覆盖在选区上方并吃掉自己的命中区，或
  退到屏幕角落；不能让它把点击漏给下面的音符。
- 动画 120~180ms；圆形不跟随指针，避免挡住正在拖的音符。
- 图标 16~18px 才能看清；9px 方块里塞 Material Icons 没有意义。桌面可
  hover 展开，移动端 tap 展开。
- 胶囊只负责“选工具 + 起手”，拖动过程中的 `±tick / ±key / ±gate / span`
  仍由现有 tooltip 显示，两者不要混成一套状态。

**现有胶囊、transport 组和拖动 tooltip 分别在哪**

- `crates/yinhe-egui/src/widgets/selection_actions.rs`：既有胶囊工具，也有
  transport bar 的分组控件。
  - `show_transport_buttons`：12 个选择动作，带边框，常驻 transport bar；
    无框选时整组灰显。
  - `show_drag_mode_selector`：Gate/Scale/Riff 三联选择器。
  - `show` / `bar_rect`：仍服务直线/剪刀的单个确认胶囊，放在锚点线 bbox
    右侧 8px。
- `overlay.rs` 不再给选择工具画浮动胶囊，也不再画三个控制点；选择动作走
  transport bar。
- 拖动选框时出现的 `±tick / ±key / ±gate / span` **不是 widget**，原先在
  `view_interaction.rs`，现在已移到 `crates/yinhe-egui/src/widgets/tooltip.rs`，
  用 `ctx.debug_painter()` 画出的非交互提示。它和胶囊不是同一套东西。

**三个想法的取舍**

1. **重排**：改为 transport bar 的三联选择器 + 选框右缘拖动，不再把三个点
   贴在选框边缘。
2. **九向自定义**：暂不做。三联选择器 + 右缘已经解决重叠和图标可读性；
   九向只会增加设置、碰撞检测和双端测试。以后真有需求，最多做 2~3 个预设。
3. **看 DAW**：FL Studio 是单个 Stretch Handle + `Alt`/`Right-Shift` 拖音符
   边缘拉伸；复制主要靠 `Ctrl+B`、`Ctrl+D`、`Shift+拖拽`。桌面 DAW 的主流是
   修饰键/工具模式，不是三个常驻方块。你的三把手已经是“移动端简化版”，重点
   是固定、可点、不重叠，而不是可配置。

**建议优先级**

- 已完成：选择动作回归 transport bar（带边框）+ Gate/Scale/Riff 三联选择器
  + 删除 PR 控制点 + 右缘拖动 + `widgets/tooltip` 抽取。
- P1：AR 的 Riff/Scale 控制点是否跟进同一套选择器；移动端未跟进。
- P2：如仍有需求，做 2~3 个布局预设，不做每个把手九向。

## 6. 方案三：音名标签

### 6.1 先决定产品目标

- 目标 A：只在 hover、选中、或放大到文字放得下时显示音名。推荐。
- 目标 B：所有可见音符常驻显示音名。不推荐，尤其黑乐谱。

这两个目标的实现成本差一个数量级：

- A 用 egui overlay 就够，不需要新 GPU 管线；
- B 需要 wgpu glyph atlas、纹理 bind group、采样器、glyph 图集和 LOD，
  还要在 Android GLES 上验证。

### 6.2 目标 A：egui overlay

**插入位置**

必须在 wgpu 纹理之后、键盘之前：

- `crates/yinhe-egui/src/piano_view/overlay.rs:116-130` 画纹理；
- `overlay.rs:132-167` 画键盘；
- 标签 pass 放在这两段之间，用 `music_rect` 做 clip。

不能放在 `piano_view.rs` 里 `overlay::draw_overlays` 之前，否则会被音符纹理盖住。

**数据源**

不要用 `NoteLayerCache` 的实例列表：

- LOD 开启时它是摘要段，不是单音符；
- GPU cull 路径下根本没有这份 CPU 列表。

正确做法是从模型直接查可见窗口：

```rust
for key in view.visible_cross_range() {
    for note in midi.key_notes_in_range(key, tick_lo, tick_hi) {
        // 过滤 end_tick <= tick_lo 的长音符候选
        // 投影成像素矩形
    }
}
```

`key_notes_in_range` 会按 `max_note_len` 外扩左边界，所以候选里可能包含起点在
视口左边的长音符；用 `end_tick > tick_lo` 再过滤一次即可。

投影复用 hit-test 的 `tick_to_main_px_dir` / `key_to_cross_px` / `orient_rect`，
不要另写一套像素公式；否则标签、hover outline 和实际命中区会错位。

**显示条件用几何适配，不用硬阈值**

```
let galley = painter.layout_no_wrap(label, font_id, color);
if galley.rect.width() + 2 * pad <= note_px_width
    && galley.rect.height() + 2 * pad <= row_px
{
    painter.galley(...);
}
```

这样缩放越大，能放下的标签越多，是连续变化；不需要“可见音符 < 1 万”这种
魔法阈值。默认缩放下 LOD 是 16 tick 摘要，本来就不该画逐音符标签；当缩放
大到 `lod_block().is_none()` 时，再按文字是否放得下来决定。

**遮挡问题**

标签画在整个音符纹理之后，会盖在所有音符上。如果同一 key 上有重叠音符，
被覆盖的音符标签会浮在上面，看起来像标签贴错了位置。

第一版建议只画 hover/选中音符的标签，O(1)~O(选中数)，没有遮挡歧义。选中态也
走同一套几何适配，不要无条件给所有选中音符画字；选中 1 万个长音符时，标签
数量仍然是 O(选中数)，需要实测。

如果要做“放大后全量音名”，需要按绘制顺序做一次区间覆盖判断，只给标签区域
没有被后画音符盖住的音符画字。这个 pass 可以复用 `NoteInstance` 的排序规则
（`start_tick` 升、`end_tick` 降、velocity 升、track 升），但 GPU cull 路径
拿不到 CPU 排序结果，需要在模型层重新扫一遍。

**字体和颜色**

- 用比例字体，不要等宽字体；字号跟随行高连续缩放，并保证可读下限。
- 标签颜色按轨道色亮度选黑/白，而不是固定 `contrast_fg()`；轨道色有深有浅，
  固定颜色在某些轨道上会看不清。
- 标签要跟随 `font_scale`；键盘 C 标签目前是显式字号，不跟随，做标签时不要
  复制这个行为。
- 文字内容先只做音名（`C4`、`C#4`）。如果宽度够，可以再显示力度，例如
  `C4 · 100`；但这会让标签宽度翻倍，建议作为后续可选。

**成本**

- 每个标签一个 `TextShape`；egui 的 galley 有缓存，但每个 shape 仍会产生顶点。
- 几百到一两千个标签通常在可接受范围；上万个需要实测，尤其是 4K/高 DPI 下。
- 真正先爆的往往不是 GPU，而是 CPU layout/tessellation 和遮挡判断。
- 建议在 `piano_view/perf.rs` 里临时加 `label_count` 和 tessellation 时间，
  测完再决定是否保留。

### 6.3 目标 B：wgpu glyph atlas（暂不做）

如果以后确实要给所有可见音符常驻音名，才需要这条路径：

- `crates/yinhe-wgpu` 目前没有任何纹理创建/采样路径，需要新增：
  - R8 glyph atlas + sampler + bind group；
  - 每个 glyph 的 UV/quad，或者把 glyph index 塞进 instance；
  - 一个新的文本 pipeline 和 draw call；
- 字体栅格化可以用依赖树里已有的 `skrifa`（epaint 同源），不要同时引入
  `fontdue`/`cosmic-text` 两套字体栈；
- Android GLES 3.20 的 shader 翻译和纹理格式要一起验证；
- 需要 LOD：缩小后标签没有意义，必须跟现有 `SUMMARY_BLOCK_TICKS` 联动。

这条路不是不能做，而是第一版不值得。先把“hover/选中/放大后”的标签做好，
如果用户确实需要全量标签，再评估 atlas。

### 6.4 八度口径必须先统一

- `yinhe_types::key_name()` 现在是 C5=60，且没有调用点；
- 桌面键盘 `keyboard.rs` 是 C4=60；
- Flutter 代码也是 C4=60（注释里写的“桌面键盘用 C5”与实际桌面代码不符）。

上音符标签前，把 `key_name()` 改成 `key / 12 - 1`（C4=60），并让键盘、音符
标签、Flutter 共用同一个 helper。否则同一个音符在键盘栏写 C4、在音符上写 C5。

### 6.5 验证

- 手工：C4/C#4、128/256 键、调式折叠、横向/纵向、亮/暗主题、字体缩放。
- 遮挡：同一 key 上重叠 2~3 个音符，确认标签不会浮在覆盖它的音符上。
- 性能：100/500/1000/5000 个可见标签各测一次帧时间和 tessellation 时间。
- 回归：`key_name()` 改动后，键盘标签和 Flutter 显示一致。

## 7. 和对话里几条判断的差异

| 说法 | 代码事实 | 结论 |
| --- | --- | --- |
| velocity 塞进 instance 的 padding 当 u8 | `NoteInstance` 没有 padding，velocity 已经在 `packed` 的 bit25..32 | 比想象中更简单，不需要改 instance 布局 |
| 半格高亮零额外三角形 | 正确；但需要给 `VertexOutput` 加一个 flat varying，并处理 decor/AR 的默认值 | 仍然接近免费 |
| 把手是 SDF 区域，不额外加三角形 | 选中态可以；hover 用 egui 更简单；所有音符常驻把手仍然需要 LOD | 不要给全量音符画把手 |
| 1000 万可见音符 = +2000 万 quad | 1000 万是总音符量级，不是可见量；默认缩放走 16 tick LOD 摘要 | 不要按全曲总量估算每帧 quad |
| 音名必须自己搭 glyph atlas | 只在“所有音符常驻音名”时成立；hover/选中/放大适配用 egui 够 | 先决定目标 A 还是 B |
| egui text 几万个就废 | 方向对；几百到几千通常可接受，上万个要实测，且遮挡判断可能先成为瓶颈 | 关键不是字体，是 LOD 和遮挡 |
| 第 3 条从一开始就按 atlas 设计 | 如果目标是 B，正确；如果目标是 A，会把简单问题做复杂 | 先用 egui 验证工作流 |

## 8. 建议实施顺序

### P0：力度填充（已完成，`f463f5f7`）

- 只改 `shader.wgsl` 和 `VertexOutput` 的三个构造函数；
- 横向 PR 启用，AR 和纵向 PR 保持原行为；
- 选中态改成边框高亮，保留力度高度；
- 验证后删掉 PR 的旧颜色淡化路径。

### P0：hover outline + grip + 纵向光标

- 让 `update_hover_cursor` 返回命中的音符信息（rect、mode、track、tick、key），
  不要重复跑 hit-test；
- 在 `overlay.rs` 纹理之后画 outline 和两个 grip；
- 顺带修纵向 PR 的 `ResizeVertical` 光标；
- 短音符的边缘热区改成随宽度收缩，并补回归测试。

### P1：选中 outline

- 复用 4.3 已经加的 `selected`，不另起一套状态；
- `fs_main` 把 `border_color` 换成对比色，边框宽度不变或 +1px；
- 不给所有选中音符画 grip。

### P1：hover / 选中音名

- 统一 `key_name()` 为 C4=60；
- 在 `overlay.rs` 里画 hover/选中音符的标签；
- 用 galley 尺寸做适配，不用固定数量阈值；
- 记录标签数和 tessellation 时间。

### P2：放大后全量音名

- 先做 egui + 几何适配 + 遮挡判断；
- 实测 100/500/1000/5000 标签的帧时间；
- 只有在实测不达标、且产品确实需要全量标签时，才做 glyph atlas。

### P2：Alt+拖动音符改力度

- 这是 Studio One 式的直接操作，和本次三个视觉功能独立；
- 现有力度编辑只在 Automation 的 Velocity 模式或数值面板里；
- 如果要加，建议单独做交互设计，不要和把手/标签混在一个 commit。

## 9. 待确认

1. 力度填充是否只做横向 PR？纵向瀑布流保持旧颜色，还是也做填充？
2. 选中态是否保留力度填充（用 outline 表示选中），还是继续用整块高对比色？
3. 音名标签的目标是 A（hover/选中/放大后）还是 B（所有可见音符常驻）？
   这决定用 egui 还是 glyph atlas。
4. 是否接受把音符边缘热区从固定 6px 改成随音符宽度收缩？
5. 八度口径统一为 C4=60（标准 MIDI）是否同意？

## 10. 关键文件

| 内容 | 路径 |
| --- | --- |
| NoteInstance 定义 | `crates/yinhe-wgpu/src/vertex.rs:116-210` |
| 音符 vertex/fragment | `crates/yinhe-wgpu/src/shader.wgsl:396-771` |
| `VertexOutput` / 构造函数 | `crates/yinhe-wgpu/src/shader.wgsl:267-276`、`290`、`418`、`619` |
| 音符 pipeline / blend | `crates/yinhe-wgpu/src/pipeline.rs:322-389` |
| 音符 layer / chunk / draw | `crates/yinhe-wgpu/src/layer.rs:7-170` |
| LOD 摘要选择 | `crates/yinhe-wgpu/src/pianoroll/summary.rs:29-44` |
| PR instance 构建 | `crates/yinhe-wgpu/src/pianoroll/instances.rs:65-130` |
| 可见窗口缓存 | `crates/yinhe-wgpu/src/note_cache.rs:40-58` |
| egui overlay 顺序 | `crates/yinhe-egui/src/piano_view/overlay.rs:116-167` |
| 音符 hit-test | `crates/yinhe-egui/src/piano_view/drag/hit.rs:55-102` |
| hover 光标 | `crates/yinhe-egui/src/piano_view/interaction.rs:257-338` |
| 键盘 C 标签 | `crates/yinhe-egui/src/piano_view/keyboard.rs:289-296` |
| 音名 helper | `crates/yinhe-types/src/lib.rs:92-98` |
| 渲染设置 | `crates/yinhe-egui/src/dialogs/settings/render.rs:7-72` |
| 设置默认值 | `crates/yinhe-editor-core/src/audio_settings.rs:231,240,247,256` |
| 大规模基准 | `crates/yinhe-wgpu/src/cull/bench.rs` |
