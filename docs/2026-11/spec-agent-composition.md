# LLM 作曲与电脑操控：yinhe Agent 前期方案

日期：2026-11-19
状态：前期计划（本轮不实现协议，只定义方向与最小闭环）
关联文档：`docs/2026-11/spec-a11y-agent-control.md`

---

## 一、先想清楚：LLM 控制 DAW 的三种方式

| 方式 | 原理 | 优点 | 缺点 | 适合 |
|---|---|---|---|---|
| A. 文件平面 | LLM 生成 MIDI/工程文件，yinhe 导入/导出 | 最简单、可离线、可复现 | 无法实时反馈；导入/导出仍要 UI 或 CLI | 批量作曲、CI、离线生成 |
| B. 结构化命令平面 | LLM 调 yinhe 的命令/状态接口（本方案重点） | 快、准、可验证、token 可控 | 需要新增协议和命令注册表 | 实时作曲、编辑、混音、Agent |
| C. 电脑操控平面 | 截图 + 鼠标/键盘 + OS 无障碍树 | 不用改应用 | 慢、脆、贵、无法验证；1 亿音符画布不可枚举 | 兜底、演示、旧版本 |

**推荐顺序：B > A > C。** C 只作为「没有 API 时的兜底」，不要作为主路径。

原因：

- 截图 + 坐标点击对 100 套主题、深浅色、DPI、字号缩放都敏感；OCR 对 10px 自绘文字不可靠。
- 画布里的音符不是 DOM/AX 节点，电脑操控看不到「第 5 小节有哪些音符」。
- 没有状态查询，Agent 无法判断「我到底改没改成功」。
- 1 亿音符场景下，任何「逐音符视觉识别」都是灾难。

---

## 二、目标：一个 token 友好的 Agent 协议（YAP）

工作名：**YAP（Yinhe Agent Protocol）**。它不是新文件格式，而是一层 JSON 命令/状态协议，可以跑在：

- 本地 NDJSON socket（桌面/移动调试）；
- CLI 子命令（离线）；
- 未来的 MCP/OSC 适配器。

### 2.1 设计原则

1. **按区间查询，不返回全曲。** 所有音符查询必须带 `track` + `from_tick`/`to_tick` + `limit`。
2. **命令有稳定 ID。** 复用/补齐 `yinhe-editor-core::shortcuts` 的动作 ID，外部不按本地化 label 调用。
3. **每次修改返回 revision。** Agent 用 `revision`/`doc_id` 做乐观并发和验证。
4. **异步任务返回 job_id。** 保存、导出、冻结等继续走后台线程，不阻塞 UI。
5. **默认 dry-run。** 批量修改先 `validate`，再 `apply`；`apply` 支持 `undo_group`。
6. **不返回音符坐标。** 用 tick/key/gate/velocity；屏幕坐标只用于截图标注。
7. **错误可分类。** `invalid_argument` / `not_found` / `conflict` / `busy` / `internal`。

### 2.2 最小命令集

| 类别 | 命令 | 说明 |
|---|---|---|
| 元信息 | `ping` / `protocol_version` / `list_commands` | Agent 自描述 |
| 文档 | `state` / `open` / `save` / `save_as` / `close` | 文档生命周期 |
| 工程 | `project_summary` | BPM、PPQ、轨道、长度、音符总数 |
| 轨道 | `list_tracks` / `add_track` / `update_track` | 名称、端口/通道、颜色、静音、solo |
| 音符 | `query_notes` / `apply_notes` / `update_notes` / `delete_notes` | 按区间/ID 操作 |
| 选择 | `get_selection` / `set_selection` / `select_all` / `clear_selection` | 区间 + 音符 ID |
| 走带 | `play` / `pause` / `stop` / `seek` / `set_loop` | tick 单位 |
| 批量 | `quantize` / `humanize` / `transpose` / `fill_gaps` / `dedupe` / `length_scale` | 复用现有 EditAction |
| 混音 | `list_channels` / `set_gain` / `set_pan` / `set_send` / `set_mute` / `set_solo` | 复用 MixerParams |
| 插件 | `list_plugin_params` / `get_plugin_param` / `set_plugin_param` | 复用 CLAP/VST3 参数接口 |
| 校验 | `validate` / `undo` / `redo` | 修改前校验、可回滚 |
| 任务 | `job_status` / `job_cancel` | 异步进度 |
| 视觉 | `screenshot` | 返回 PNG 路径或 base64 |
| 导出 | `export_midi` / `export_audio` | 复用现有导出 |

### 2.3 消息示例

**查询工程摘要**

```json
{"id":1,"op":"project_summary"}
{"id":1,"ok":true,"doc_id":3,"revision":128,"ppq":960,
 "bpm":174,"bar_count":64,"note_count":18432,
 "tracks":[{"uuid":"...","index":0,"name":"Drums","kind":"midi","port":0,"channel":9,
            "muted":false,"soloed":false,"note_count":4096}]}
```

**按区间查音符（分页）**

```json
{"id":2,"op":"query_notes","track_uuid":"...","from_tick":0,"to_tick":7680,"limit":200,
 "cursor":null}
{"id":2,"ok":true,"revision":128,"next_cursor":"7680:42",
 "notes":[[0,36,120,100],[0,42,120,96],[480,38,120,104]]}
```

音符数组约定：`[start_tick, key, gate, velocity]`，key 用 MIDI 0-127，gate/velocity 用 0-127/0-1 二选一（协议里固定一种，建议 gate 用 tick，velocity 用 0-127）。

**应用音符（替换区间，带校验）**

```json
{"id":3,"op":"apply_notes","track_uuid":"...","mode":"replace_range",
 "from_tick":0,"to_tick":7680,"undo_group":"intro-drums",
 "notes":[[0,36,120,100],[240,36,120,96],[480,38,120,104]]}
{"id":3,"ok":true,"doc_id":3,"revision":129,"changed":12,"undo_label":"Agent: intro drums"}
```

**批量人性化**

```json
{"id":4,"op":"invoke","cmd":"humanize","args":{"track_uuid":"...","from_tick":0,"to_tick":7680,
 "timing":0.08,"velocity":0.06,"seed":42}}
```

**截图**

```json
{"id":5,"op":"screenshot","view":"pianoroll","track_uuid":"..."}
{"id":5,"ok":true,"path":"/tmp/yinhe-0005.png","width":1600,"height":900}
```

### 2.4 token 预算

| 内容 | 建议上限 | 说明 |
|---|---|---|
| `project_summary` | 1–4 KB | 只含轨道和统计，不含音符 |
| `query_notes` 单次 | 200–500 条 | 再大就分页 |
| `state` | 2–8 KB | 选择/走带/任务，不含音符 |
| 截图 | 1 张/步 | 只在需要视觉确认时 |
| 错误 | 1–2 KB | 带 `code`/`message`/`hint` |

不要给 LLM 喂「全曲 1.8 万音符 JSON」。它既贵又没用；LLM 需要的是「第 1-8 小节，鼓轨，每小节 4 个底鼓 + 8 个踩镲」这种摘要 + 可选的局部细节。

---

## 三、LLM 作曲工作流

### 3.1 五步闭环

```
inspect → plan → generate → apply → verify
   ↑                                    │
   └────────── revise ←─────────────────┘
```

1. **inspect**：`project_summary` + `list_tracks` + `query_notes`（只看要改的区间）。
2. **plan**：确定 BPM、拍号、调式、曲式、轨道分工、每段密度。
3. **generate**：生成 compact note 数组（或 MIDI 文件）。
4. **apply**：`validate` → `apply_notes`（带 `undo_group`）。
5. **verify**：`state` 看 revision/note_count；必要时 `screenshot`；听感由用户判断。

### 3.2 推荐的「作曲上下文包」

给 LLM 的系统提示里固定这些信息，避免每轮重复：

```json
{
  "ppq": 960,
  "bar_ticks": 3840,
  "beat_ticks": 960,
  "key": "D minor",
  "scale": [62,64,65,67,69,70,72],
  "chords": {"Dm":[62,65,69],"Bb":[58,62,65],"F":[65,69,72],"C":[60,64,67]},
  "drum_map": {"kick":36,"snare":38,"clap":39,"hat_closed":42,"hat_open":46},
  "tracks": [
    {"name":"Drums","channel":9,"range":[36,49]},
    {"name":"Bass","channel":0,"range":[28,48]},
    {"name":"Chords","channel":1,"range":[48,72]},
    {"name":"Lead","channel":2,"range":[60,84]}
  ],
  "constraints": {
    "max_notes_per_apply": 2000,
    "avoid_overlap_same_key": true,
    "velocity_range":[40,120],
    "swing": 0.0
  }
}
```

### 3.3 生成格式：两种都支持

**A. 稀疏数组（推荐给 LLM）**

```json
{"track":"Drums","from_tick":0,"to_tick":15360,
 "notes":[[0,36,120,100],[960,38,120,96],[1920,36,120,100],[2880,38,120,96]]}
```

优点：token 少、易校验、易 diff。
缺点：没有「和弦/音阶」语义，LLM 要自己算。

**B. 紧凑 DSL（后续可加）**

```text
track Drums bars 1-4
  kick 1,3 | snare 2,4 | hat 8ths
track Bass bars 1-4
  root D2 | rhythm 1,1.5,2.5,3.5 | gate 0.4
```

优点：更接近音乐语言，token 更少。
缺点：需要解析器和错误提示；第一版不建议做。

**C. MIDI 文件**

LLM 生成 `.mid`（用 mido/pretty_midi），yinhe `--import-midi` 或 UI 导入。
适合：复杂编曲、需要现成 MIDI 工具链、离线批量。

### 3.4 一个 8 小节示例（概念）

用户：「写一个 174 BPM 的 D 小调暗黑电子 loop，鼓、贝斯、和弦、琶音各一轨，8 小节，第 8 小节加 fill。」

Agent 的动作序列：

1. `project_summary` → 确认 PPQ=960，BPM=174，空工程。
2. `add_track` × 4（Drums/Bass/Chords/Arp，通道 9/0/1/2）。
3. `apply_notes` Drums：4 小节底鼓/军鼓/踩镲，第 8 小节 16 分音符 fill。
4. `apply_notes` Bass：Dm-Bb-F-C 根音，八分音符，gate 0.45。
5. `apply_notes` Chords：每小节 2 个和弦，gate 1.5 拍，力度 70-90。
6. `apply_notes` Arp：16 分音符，D 小调音阶，上行/下行，力度随小节递增。
7. `humanize` 鼓（timing 0.03，velocity 0.05，seed 42）。
8. `quantize` 贝斯到 1/16（可选）。
9. `state` 验证 note_count/revision。
10. `screenshot` 或让用户试听。

这个流程不需要 LLM 看到全曲，也不需要点任何像素。

---

## 四、电脑操控：什么时候需要，怎么做

### 4.1 不推荐作为主路径

「截图 + 点击」适合演示，不适合生产：

- 画布是 GPU 纹理，OCR 读不到音符。
- 10px 文字、100 套主题、DPI 缩放 → 坐标和识别都不稳。
- 没有状态，Agent 无法确认是否成功。
- 1 亿音符场景下，截图看到的只是视口，不是工程。

### 4.2 推荐的分层

| 层 | 用途 | 优先级 |
|---|---|---|
| 结构化命令/状态（YAP） | 所有编辑、查询、导出 | P0 |
| 文件平面（MIDI/.yin） | 离线生成、批量、CI | P0 |
| OS 无障碍树（AX/UIA/AT-SPI） | 找菜单/按钮、读屏兼容、旧版兜底 | P1 |
| 截图 + 坐标点击 | 演示、无 API 时的兜底 | P2 |

### 4.3 AX 层的正确用法

- 给所有命令控件/菜单项/Tab 加 `author_id`（= `CommandId`），macOS 映射 `AXIdentifier`，Windows UIA 映射 `AutomationId`。
- 不要依赖本地化 label；label 只给人看。
- 画布只暴露「画布」节点 + 自定义动作（读选择、读播放头、打开事件列表），不暴露逐音符节点。
- 读屏/Agent 找控件用 ID；找内容用 `query_notes`。

### 4.4 安全边界

- 控制 socket 默认关闭，只在显式 `--control` 时启用；只绑 127.0.0.1 / 用户私有 socket；启动时生成随机 token。
- 文件写入/删除、导出覆盖、关闭未保存工程等破坏性命令需要显式 `confirm: true`。
- Agent 的每次批量修改带 `undo_group`，用户可以一键撤销。
- 截图可能包含工程内容，默认只写临时目录，不上传。
- 限制单次 `apply_notes` 的音符数（例如 2000），防止 LLM 一次灌入百万音符。

---

## 五、分阶段落地

### Phase 1：离线最小闭环（部分已实现 2026-11-19）

已实现：

- `--version` / `--help`
- `--list-commands`：输出 67 个稳定 CommandId、作用域、默认快捷键
- `--inspect <file.yin>`：只读 `project.json` + `mapping.json`，输出 project/轨道摘要
- 关键命令按钮、菜单行、pinned 动作写 AccessKit `author_id`

暂未实现：

- `--export-midi <in.yin> <out.mid>` / `--import-midi <in.mid> <out.yin>`：
  需要完整加载工程；1 亿音符会付出全量解压/内存代价。当前先用现有 UI 导出；
  等流式导出/加载路径可复用后再做。
- 模式栏 / dock tab 的 `author_id`：本轮只覆盖命令按钮/菜单。

验收：Agent 不启动 GUI 就能知道工程里有什么、能列出命令表。
实测 `--inspect` 对 4.4KB 合成工程（data 段为垃圾字节）只读元数据，不触碰音符段。

### Phase 2：本地控制 socket

- `--control <path>` / `--control-port <port>`，NDJSON
- `state` / `project_summary` / `query_notes` / `apply_notes` / `invoke` / `job_status` / `screenshot`
- 主循环 poll + 独立线程，不阻塞 UI
- revision + undo_group + dry-run

验收：外部 Python 脚本能「读状态 → 写 8 小节鼓 → 读状态验证 → 截图」。

### Phase 3：MCP/OSC 适配

- MCP server 把 YAP 命令暴露成 tools
- OSC 把 transport/参数暴露给控制面
- MIDI remote / MIDI learn（可选）

### Phase 4：LLM 专用 helper

- `summary`：按小节/轨道聚合（音符数、音域、密度、和声）
- `suggest`：给出「下一段可以怎么发展」的候选，不直接改工程
- `evaluate`：检查平行五度、超音域、过密、重复、力度突变
- `diff`：两次 revision 的音符级 diff，方便 Agent 自我检查

---

## 六、给 LLM 的提示词骨架

```text
你是 yinhe 的编曲 Agent。你只能通过 YAP 命令修改工程。

规则：
1. 先 project_summary，再 query_notes；不要假设工程为空。
2. 每次 apply 最多 2000 个音符；超过就分页。
3. 所有 tick 用整数；1 拍 = PPQ，1 小节 = PPQ * 拍号分子。
4. 修改前先 validate；修改后读 state，确认 revision 增加。
5. 不要删除用户已有音符，除非用户明确要求；用 replace_range 时先 query 原区间。
6. 每次批量修改用 undo_group，命名格式 "Agent: <意图>"。
7. 听感无法验证时，明确告诉用户「已写入，建议试听」。
```

---

## 七、开放问题（需要节能降耗拍板）

1. YAP 第一版是「本地 socket」还是「CLI 子命令」优先？我建议 CLI 先做只读，socket 再做读写。
2. 音符数组的 gate 用 tick 还是拍？我建议 tick，和工程内部一致。
3. 是否允许 Agent 直接改混音/插件参数？建议先只读，写操作加 `confirm`。
4. 移动端是否要开放 YAP？建议先只做「事件列表 + 画布语义动作」，不开放网络控制。
5. 是否需要 `--import-midi`/`--export-midi` 作为第一批？如果用户常用外部 MIDI 工具链，值得。
