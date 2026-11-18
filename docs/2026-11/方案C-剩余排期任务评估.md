# 方案 C 剩余排期任务评估

> 日期：2026-11-18
> 对应文档：《方案C-返工检讨与修复方案.md》8.6 节「未开始」
> 范围：仅评估**重构/整理**类剩余项，不包含新功能与已知 bug。

---

## 0. 结论先说

1. 剩余项全部是**代码整理**，不做也不影响当前功能与正确性。
2. 真正值得做的只有 3~4 项，合计约 **3~4 人日**；全部做完约 **4.5~6 人日**（含回归）。
3. 收益最高的是 `spawn.rs`（当前全仓最大生产文件）与 `arrange.rs`；`params.rs`、`graph.rs` 不建议为了行数而拆。
4. 这些文件都不是「函数级屎山」：除 `arrange::show` 267 行外，已经没有超过 200 行的函数；问题是**单文件承担了多类职责**（文件级上帝对象）。

---

## 1. 剩余任务总表

数据来自 2026-11-18 静态扫描；「生产代码」= 非空、非行首注释、排除 `#[cfg(test)]` 块。

| 任务 | 文件 | 总行 | 生产代码 | 函数数 | 最长函数 | 建议 | 风险 | 工作量 |
|---|---|---:|---:|---:|---|---|---|---:|
| 拆 `spawn.rs` | `crates/yinhe-audio/src/spawn.rs` | 1802 | 1273 | 62 | `spawn_worker` 207 | 拆 5 个文件 | 中 | 1.5~2 天 |
| 拆 `arrange.rs` | `crates/yinhe-egui/src/arrange.rs` | 1371 | 1186 | 15 | `show` 267 | 拆 4 个文件 | 低~中 | 0.5~1 天 |
| 拆 `document.rs` | `crates/yinhe-editor-core/src/document.rs` | 1012 | 788 | 52 | `from_model` 93 | 拆 3 个文件 | 低 | 0.5~1 天 |
| 拆 `mix.rs` | `crates/yinhe-egui/src/mix.rs` | 1070 | 808 | 37 | `draw_audio_and_track_strips` 70 | 拆 3 个文件 | 低 | 0.5 天 |
| 拆 `graph.rs` | `crates/yinhe-mixer/src/graph.rs` | 1103 | 831 | 73 | `refresh_pdc` 78 | 可选，收益低 | 中 | 0.5 天 |
| 拆 `params.rs` | `crates/yinhe-mixer/src/params.rs` | 937 | 725 | 76 | 均 < 50 | **不建议** | — | 0 |
| 余下 ~100 行函数 | 多处 | — | — | — | 102 / 84 / 78 | 可选 | 低 | 0.5 天 |

> 估算前提：熟悉本仓库的 1 名开发者，包含拆分后的编译、`cargo clippy --tests`、相关 crate 测试与一次手工冒烟；不包含新增测试。

---

## 2. 逐文件评估

### 2.1 `spawn.rs` —— 当前最大的「文件级上帝对象」

**数据**

- 总 1802 行 / 生产 1273 行 / 62 个函数 / 测试文件 338 行。
- `AudioCommand` 枚举本身 182 行、34 个变体；`impl AudioCommand` 56 行。
- 大函数：`spawn_worker` 207、`spawn_cpal_audio` 161、`compute_chase_states` 106、`build_output_stream` 100、`init_renderer` 94。

**代码怎么样**

- 内聚低：同一个文件里同时有「命令协议 + 句柄 + worker 线程 + chase 计算 + 设备枚举 + 采样率/缓冲协商 + 引擎初始化 + cpal 流构建」。
- 但函数本身都不长，职责边界清楚，没有深层嵌套；属于「按类型堆在一起」，不是「逻辑纠缠」。
- 真正的风险在音频线程/初始化顺序，而不是函数复杂度；拆分本身机械，但必须做播放/导出/切设备回归。

**建议拆法**

| 新文件 | 内容 | 约行数 |
|---|---|---:|
| `spawn/command.rs` | `AudioCommand` + `CmdMeta` + `meta/is_reliable/kind` | 260 |
| `spawn/handle.rs` | `AudioHandle`、`CpalAudioHandle`、`Drop` | 220 |
| `spawn/worker.rs` | `WorkerCmd/WorkerResult`、`spawn_worker`、`compute_chase_states` | 380 |
| `spawn/device.rs` | 设备枚举、采样率/缓冲协商、合成器后端选择 | 220 |
| `spawn/stream.rs` | `init_engine`、`init_renderer`、`build_output_stream`、`start_output_stream`、`spawn_cpal_audio` | 500 |

**风险与验证**

- 风险：中。线程启动顺序、ring 清空、采样率切换、设备切换都在这条路径上。
- 手工必须覆盖：播放/暂停/继续/停止、seek、循环回绕、导出 WAV、冻结、切换输出设备、切换采样率。

**工作量：1.5~2 人日。**

---

### 2.2 `arrange.rs` —— 视图编排 + 命令落地

**数据**

- 总 1371 行 / 生产 1186 行 / 15 个函数，其中 10 个 ≥ 50 行。
- `show` 267、`draw_track_panel` 136、`draw_horizontal_scrollbar` 117、`draw_arrangement_view` 108、`apply_audio_edit_commands` 103、`draw_hover_hint` 94、`draw_event_rows_band` 69、`handle_blank_double_click` 65、`draw_ruler_band` 64、`draw_vertical_scrollbar` 63。

**代码怎么样**

- 经过本轮拆分，已经从 1030 行的 `show` 降到 267；剩余的是「视图编排 + 若干绘制/命令辅助」。
- 没有 500+ 行函数，逻辑也不深；继续拆主要是文件级整理。
- 风险点是 `arrange.rs` 没有专属单测（`arrange/view_ui` 有测试，但框选/拖拽/双击/滚动主要靠手工）。

**建议拆法**

| 新文件 | 内容 | 约行数 |
|---|---|---:|
| `arrange/view.rs` | `show` + `draw_*` + `handle_blank_double_click` | 700 |
| `arrange/edit_cmds.rs` | `apply_clip_edit_commands`、`apply_audio_edit_commands`、`clip_params` | 210 |
| `arrange/layout.rs` | `build_row_layout` + 行布局辅助 | 80 |
| `arrange/types.rs` | `ArrangeData`、`ArrangeEdit`、`ArrDragDelta`、`ArrSelRect` | 60 |

**风险与验证**

- 风险：低；拆完手工验证：框选、拖拽音符/片段、双击空 Clip、双击 Clip 标题、标尺跳转、横竖滚动、分割条拖拽/双击复位。

**工作量：0.5~1 人日。**

---

### 2.3 `document.rs` —— 方法很多，但每个都很短

**数据**

- 总 1012 行 / 生产 788 行 / 52 个函数，最长 `from_model` 93 行。
- 职责：文档生命周期与脏标记、构造/导入、字体覆盖同步、快照与 undo/redo、轨迹缓存同步、轨道/通道听感、skip mask、overrides 同步。

**代码怎么样**

- 内聚偏低，但拆分难度最低：`Document` 字段全部 `pub`，方法基本互相独立，没有借用奇技淫巧。
- 属于「简单堆叠」型文件，不是屎山。

**建议拆法**

| 新文件 | 内容 | 约行数 |
|---|---|---:|
| `document/undo.rs` | `capture_snapshot`、`push_undo`、`undo/redo`、`jump_history_to`、`restore_snapshot`、`rescale_edit_ticks` | 220 |
| `document/io.rs` | `empty`、`from_model`、`from_yin_path`、`apply_sf_overrides`、`recode_track_names` | 260 |
| `document/sync.rs` | 缓存同步、听感、skip mask、`sync_overrides_to_model` | 260 |

**风险与验证**

- 风险：低。验证：新建空工程、加载 MIDI、保存/加载、undo/redo 多次、跳转历史、切主题/Conductor 颜色、M/S。

**工作量：0.5~1 人日。**

---

### 2.4 `mix.rs` —— 状态 + 绘制 + 会话动作

**数据**

- 总 1070 行 / 生产 808 行 / 37 个函数，最长 `draw_audio_and_track_strips` 70 行。
- `MixAction` 的定义与落地已经在 `mix/actions.rs`；这里剩 `MixUiState`、`show`、通道条绘制、App 的节点写入方法、乐器机架恢复。

**代码怎么样**

- 函数短、职责相对清楚；低风险。
- 唯一需要注意的是 `MixUiState` 的字段与 `App` 的借用关系，拆文件时保持方法签名不变即可。

**建议拆法**

- `mix/view.rs`：`show`、`draw_mixer_body`、`draw_channel_strips`、`draw_audio_and_track_strips`、`draw_master_strip`。
- `mix/session.rs`：`apply_node_strip`、`apply_master`、`restore_mixer_rack`、`set_instrument_slot` 等 App 方法。
- `mix/state.rs`：`MixUiState` + 显示缓存 + 电平衰减状态。

**工作量：0.5 人日。**

---

### 2.5 `graph.rs` —— 算法密集，但函数都短

**数据**

- 总 1103 行 / 生产 831 行 / 73 个函数，最长 `refresh_pdc` 78。
- 测试文件 570 行，覆盖 PDC、延迟线、发送、主输出、冻结等。

**代码怎么样**

- 内聚中等：DelayLine / MasterChain / ChannelChain / 发送路由 / PDC 都在一个文件，但每个函数都短。
- 文件大是因为「类型 + 小函数」多，不是因为某个函数复杂。
- 拆分收益低；如果拆，建议只按 `pdc.rs` / `delay.rs` / `channel.rs` 分组。

**风险**：中（音频线程、听感回归）。
**工作量：0.5 人日；可不做。**

---

### 2.6 `params.rs` —— 不建议拆

**数据**

- 总 937 行 / 生产 725 行 / 76 个函数，**没有超过 50 行的函数**。
- 已经是 `repair_*` 扁平结构，每个修复项一个小函数。

**评价**

- 这是本轮重构的直接成果：从「`ensure_len` 一次做 12 件事」变成了显式 `repair_*` 顺序调用。
- 再拆文件只会增加跳转成本，不增加可读性。

**建议：保持现状。工作量 0。**

---

### 2.7 余下 ~100 行函数

| 函数 | 行数 | 评价 |
|---|---:|---|
| `apply_anchor_field_edit` | 102 | 输入解析 + 批量应用 + 选框跟随；可再拆「解析/应用」，收益一般 |
| `render_instruments` | 84 | 插件乐器求和 + 事件合并；与音频块契约耦合，建议不动 |
| `refresh_pdc` | 78 | PDC 拓扑计算；与 graph 拆分一起做才有意义 |
| `draw_audio_and_track_strips` | 70 | 纯 UI 列表绘制，可拆可不拆 |

都已低于文档定的 200 行阈值；**可选，0.5 人日**。

---

## 3. 建议顺序与总工作量

| 顺序 | 任务 | 理由 | 工作量 |
|---:|---|---|---:|
| 1 | `document.rs` | 风险最低、收益明确，先建立拆分节奏 | 0.5~1 天 |
| 2 | `arrange.rs` | 视图文件最大，拆完结构最清晰 | 0.5~1 天 |
| 3 | `mix.rs` | 低风险，顺手完成 | 0.5 天 |
| 4 | `spawn.rs` | 收益最大但风险最高，放最后并留足回归时间 | 1.5~2 天 |
| 5 | `graph.rs` / ~100 行函数 | 可选 | 0.5~1 天 |

**推荐做 1~4：约 3~4 人日。**
**全部做完：约 4.5~6 人日。**

---

## 4. 回归清单（每个拆分后都跑）

```text
cargo fmt --all
cargo clippy -p <crate> --tests
cargo test -p <crate>          # 下游 crate 一起
```

手工冒烟（按拆分范围选）：

- 播放 / 暂停 / 继续 / 停止 / seek / 循环回绕
- 导出 WAV、冻结 LC、解冻
- 新建/加载/保存工程、undo/redo、跳转历史
- AR：框选、拖拽、双击空 Clip、双击标题、标尺跳转、横竖滚动、分割条
- MIX：选中通道、推子/声像/M/S、发送、插入、乐器机架、设备链
- 切换输出设备/采样率（仅 `spawn.rs`）

---

## 5. 明确不建议做的事

1. **不要为了行数拆 `params.rs`**：它已经是 76 个短函数，拆文件只增加导航成本。
2. **不要独自拆 `graph.rs` 的 PDC/延迟线**：必须和听感回归一起做，收益低于风险。
3. **不要改音频线程契约**：`MAX_ENGINE_BLOCK_FRAMES`、ring 清空代次、PDC 延迟语义都不属于「整理」。
4. **不要在没有 AR 手工回归的情况下拆 `arrange.rs`**：该文件没有专属单测。
5. **不要重命名持久化字段/枚举变体**：本评估只做文件移动与函数拆分，不触碰磁盘格式。
