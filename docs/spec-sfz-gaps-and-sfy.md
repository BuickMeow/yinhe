# SFZ/SF2 覆盖缺口、许可与 .sfy 方向评估

> 状态：评估稿（2026-11-12），**不含代码改动**。
> 前置：`spec-sfz-coverage.md`（本轮已落地的门控/释放采样/RR/LFO/四点插值）。
> 目的：回答四个问题——(1) 还缺什么参数 (2) LGPL 现状 (3) `.sfy` 的两个新用途能不能做
> (4) 「播放结束后跳转，准备工作变久」的归因。

---

## 零、先说结论

| 问题 | 结论 |
|---|---|
| 还缺哪些参数 | 缺口集中在**第二包络（fileg/pitcheg/ModEnv）**、**力度曲线**、**CC 调制（`*_cc`）**、**SF2 的 `exclusiveClass` / ModEnv / Vibrato**、**循环 crossfade**、**多麦克风/流式播放**。其中 `exclusiveClass` 与滤波器阶数降级是**纯 bug 级缺口**，优先级最高。 |
| LGPL | **现状有风险**：`xsynth-core` / `xsynth-soundfonts` 是 LGPL-3.0，静态链接 + MIT/Apache 声明 + 无 NOTICE = 声明不成立；iOS 上架与 LGPL-3.0 基本冲突。建议「先补声明、再摘依赖」。 |
| `.sfy` (a) 手机单文件 | **值得做，但真正的瓶颈不是打包而是内存**：340MB wav 解码后 ≈1.4GB。必须按「原始字节 + offset 表 + 懒加载流式」设计，否则单文件化不解决手机问题。 |
| `.sfy` (b) 调音源固有参数 | **能做到，且已完成约 70%**。缺的是「参数声明模型 + 相对偏移语义 + 工程级覆盖层」，以及把 biquad 从加载期烘焙改为 note_on 计算。**建议作为 .sfy v1 的核心**，比容器/脚本更优先。 |
| 播放准备变久 | **大概率是本轮功能变化的副作用**（释放层让尾音 voice 变多），实测 CPU 每块 +20%。需要一次定向确认；若确认，用「尾音阶段不新建释放层 + 更短静音判定」解决，不必回退功能。 |

---

## 一、参数缺口清单

只列**会改变听感**的。分四类：已解析未生效 / 未解析 / SF2 规范缺失 / 跨格式缺失。

### 1.1 SFZ：解析到了但没生效

| opcode | 听感影响 | 未做的原因 | 实现成本 |
|---|---|---|---|
| `pitchlfo_depth` / `pitchlfo_freq` | 音高颤动（弦乐、人声、管乐的“活气”） | 需与弯音/pitch envelope 共用同一条换速路径（`set_speed`），否则破坏 CPU/GPU 逐样本一致 | 中（要统一变速入口） |
| `fillfo_depth` / `fillfo_freq` | 滤波 LFO，音色的缓慢呼吸 | biquad 系数是加载期烘焙，运行期改系数要动 GPU voice 状态写回 | 中 |
| `effect1` | ARIA 混响发送量（Starry 每个 group 都写 37） | 等 GM2 Reverb（`spec-yinhe-dsp.md` 阶段 B） | 依赖 dsp |
| `pitch_keytrack` / `transpose` | 整体移调、键位音高跟踪 | Starry 的 `<master>` 写 `pitch_keytrack=0`，照字面实现会把整架钢琴压成单音；需先用参考播放器 A/B | 小（但要先定语义） |
| `sustain_sw` | 延音踏板开关语义 | 与现有 CC64 标准行为冲突 | 小 |
| `sw_default` / `sw_label` / `sw_down` / `sw_up` | keyswitch 的默认值与上下键变体 | 只做了 `sw_last` + 区间吞键 | 小 |

### 1.2 SFZ：根本没解析

按收益排序：

1. **`fileg_*` / `pitcheg_*` / `eg01..eg08`（第二滤波包络、音高包络）**
   管弦乐、铜管、弦乐的核心表现力来源。当前引擎只有一条振幅包络。
2. **`velocity_*` / `amp_velcurve_N`（力度曲线形状）**
   比“力度层数”更决定专业钢琴的 pp→ff 手感。现在只有 `amp_veltrack` 一条线性/二次曲线。
3. **`*_cc`（`cutoff_cc`、`volume_cc`、`pan_cc`、`position_cc`、`width_cc`）与 `controller_*`**
   **这条与第 3 问 (b) 直接相关**——它就是 SFZ 官方的“用户拧旋钮改变音色”机制。
4. **`lfo01..lfo08` + `target`（SFZ2 通用 LFO 数组）**
   现在只有 `amplfo_*` / `pitchlfo_*` 两个固定槽位，无法指向 cutoff / pan。
5. **`loop_crossfade`** 长音循环接缝淡化。缺它，长音有驻波式“嗡嗡”。
6. **`xfin_lokey/hikey/lovel/hivel`、`xfout_*` 键位/力度版** 交叉淡化（非 CC 版）。
7. **`delay` / `start` / `end` / `count` / `direction=reverse`** 采样区间变体与反向播放。
8. **`rt_dead` / `off_mode` / `bend_up` / `bend_down` / `note_off_velocity` / `note_polyphony` / `group_label` / `polyphony`**。

另有一处**静默降级**：`parse_filter_kind` 把 `lpf_4p` / `lpf_6p` / `hpf_4p` 等全部映射成 2 极点。写 4 极点的库听起来会明显“软”一截，且没有任何提示。**这应当至少在 lint 里报出来。**

### 1.3 SF2：规范级缺失

| 项 | 影响 | 现状 |
|---|---|---|
| **`exclusiveClass`（exclusive class）** | **GM 鼓组闭镲/开镲不互掐**（最容易听出来的 bug） | `Sf2Region.exclusive_class` 有字段，我们完全没接。SFZ 侧已用 `off_by` 做了 |
| **ModEnv → 滤波器**（`modEnvToFilterFc`） | 铜管/弦乐的“咬字”，SF2 第二大包络 | xsynth 0.4 未实现 |
| **VibLFO / ModLFO** | SF2 的两个 LFO | xsynth 0.4 未实现 |
| **`sm24`（24-bit）** | 高解析库动态余量 | xsynth 读了 sm24，但实际精度接近 16-bit |
| **`keynumToVolEnvHold/Decay`** | 不同音区的包络时长差异 | 字段存在，未消费 |

### 1.4 跨格式（两种格式都没有，但专业库必需）

- **note-off velocity**：抬键力度（钢琴释放声的强弱）。
- **自定义音阶 / 调律**：Scala `.scl` + `.kbm`（微分音、历史调律）。SF2 的 `scaleTuning` 是粗糙替代。
- **多麦克风位置**：`close` / `room` / `ambient` 独立增益、延迟、相位。**这是 Kontakt 级“专业感”的最大来源，两种格式都没有 → 只有 .sfy 能承载。**
- **磁盘流 / 按需解码**：现在全部解码成 f32 常驻（Starry 340MB wav ≈ 1.4GB RAM）。手机必死。

### 1.5 已支持、但值得记一笔的

`xfin/xfout`（CC 线性门）、`on_locc`、`sw_*`、`seq_*`、`lorand/hirand`、`pitch_random`、`off_by`、`one_shot`、`trigger=release`、`amplfo_*`、`<control> set_ccN`、`Cubic` 插值、`<group>/<master>` 继承、`#include`/`#define`。

---

## 二、LGPL 现状评估

### 2.1 事实

- `xsynth-core 0.4`、`xsynth-soundfonts 0.4` 的 `Cargo.toml`：`license = "LGPL-3.0"`。
- 依赖位置：
  - `yinhe-audio`：`xsynth-core` + `xsynth-soundfonts`（**静态链接进最终产物**）。
  - `yinhe-synth`：`xsynth-soundfonts`（`resample_vec`、`FilterType`）+ `xsynth-core`（dev / parity 测试）。
- 项目 `LICENSE`：`MIT OR Apache-2.0`。仓库内**没有** LGPL 全文、没有 NOTICE、没有 xsynth 版权声明。

### 2.2 结论：三处不满足

LGPL-3.0 在静态链接场景要求：

1. **显著声明** + 附 LGPL-3.0 全文；
2. 允许用户**替换/重新链接**该库（动态链接，或提供可重链接的目标文件）；
3. 允许为调试**反向工程**。

现状 1、2、3 都没做。另有两个现实风险：

- **MIT/Apache 双许可 + 静态链接 LGPL**：不能把含 LGPL 的整体以 MIT 分发，许可声明自相矛盾。
- **iOS App Store 与 LGPL-3.0 基本不兼容**（anti-tivoization vs DRM）。

### 2.3 处置建议

**短期（低成本，立即做）**

1. 新增 `LICENSES/LGPL-3.0.txt`（全文）。
2. 新增 `NOTICE`：列 `xsynth-core` / `xsynth-soundfonts` 版本、版权、许可。
3. README/LICENSE 明确：「`crates/yinhe-audio`、`crates/yinhe-synth` 的最终产物包含 LGPL-3.0 组件，整体分发受 LGPL-3.0 约束」。
4. 桌面端可考虑把这两个库改为**动态链接**（`.dylib`/`.so`），直接满足第 2 条。

**中期（真正的解法）**

把 xsynth 彻底摘掉。剩余三块工作量可控：

| 剩余依赖 | 替代方案 | 工作量 |
|---|---|---|
| SF2 二进制解析（`xsynth_soundfonts::sf2`） | 自己写：RIFF 块遍历 + `phdr/pbag/pmod/pgen/inst/ibag/imod/igen/shdr`，输出当前 `Sf2Region` 所需字段 | 中（格式是死的，且有公开规范） |
| `resample::resample_vec` | 自己写 windowed-sinc（或换 MIT 的 `rubato`，但需评估体积/性能） | 小~中 |
| `FilterType` | 自己的枚举 | 极小 |
| `xsynth_core`（仅 parity 测试） | 删除；改用金样（golden wav）回归 | 小 |

**清洁室要求**：重写时不要照抄 xsynth 的源码结构与命名，只参考**行为**；拿它的渲染结果做对照测试是合法的（行为不是表达）。

**判断**：从**语义覆盖**看，本次已超过 xsynth 0.4（它不支持 `xfin`/`trigger`/`seq`/`pitch_random`）；但从**工程成熟度**看仍有差距（SF2 modulator 系统、边界 case 积累）。不要为了许可而仓促替换 SF2 解析——先把声明补齐，再把替换排进正常迭代。

---

## 三、.sfy 的两个新用途

用户原话：

> (a) 手机只能读 sfz 压缩包，变成 sfy 就可以轻松把 sfy 包装进内存。
> (b) 像 DAW 那样调音源的固有参数……一个音色库可以被无数音乐人玩出花。

### 3.1 (a) 单文件打包：值得做，但真瓶颈是内存

**问题真实**：SAF 的 content URI 不可随机 seek，`soundfont_archive.rs` 因此只能整包解压到私有目录，SFZ 的相对路径才成立。

**但“装进内存”不是正确目标**：

- Starry 340MB wav → 解码 f32 ≈ **1.4GB**，手机必被系统杀掉。
- 正确形态：`.sfy` 内以**原始 wav 字节（16/24-bit）+ offset 表**存储，播放时按 zone 懒加载 + 环形缓冲流式解码。这样 3GB 的库也能在手机跑，代价是一点解码 CPU。

**顺带都是真收益**：单文件 = 一个 SAF 权限项；无相对路径问题；CRC 校验；随机访问；增量更新；不依赖解压目录。

**设计要点（后续 spec 要写死的）**：

- ZIP64（>4GB 库）、WAV 用 store 模式、文本/JSON 高压缩；
- 确定性写出（固定时间戳 + 条目排序），同输入字节一致，可 CI diff；
- 采样波形峰值金字塔（mipmap）烘进容器（UI 波形 + lint 都用，不算缓存）；
- 条目级 offset 表 + 块索引，支持“改一个循环点不回写 2GB”。

### 3.2 (b) 调音源固有参数：能做到，且已完成大部分

**为什么说接近完成**：`KeyInfo` + `LayerExtra` 已经持有全部相关字段（`cutoff`/`resonance`/`amp_*`/`ampeg_*`/`pan`/`amplfo`/`xfin` 映射），而 `apply_gate` 已经是“**按音符求值**”的架构——天然适合插入一层用户参数。

**缺的三层东西（都不是引擎能力问题）**：

1. **参数声明模型**
   每个可调项需要：`id` / 显示名 / 单位 / 范围 / 默认值 / 归一化映射 / 作用目标（哪些 zone 的哪个字段）/ UI 提示与分组。
   这是“一个库被无数人玩出花”的前提，也是 `.sfy` 必须写清楚的部分。

2. **参数语义 = 相对偏移，不是绝对值**
   用户拧“亮度”应是给 `cutoff` 加 ±N 音分（或乘一个倍率），**不是**覆盖成 800Hz。否则：
   - 一调就把整个库的音色比例调坏；
   - 跨库无法复用同一套控件；
   - 无法做 A/B。

3. **覆盖层落在工程里，库文件只读**
   覆盖值存 `.yin`（类比现有 `SfChannelOverride`），可导出/分享为独立的 `.sfypreset` 小文件。库文件保持只读 → 可校验、可分享、不会被误改。

**必须先解决的一个技术点**：`cutoff` 现在是**加载期烘焙 biquad 系数**（`bake_biquad`）。要让“亮度”实时生效，系数必须移到 **note_on 时计算**：
- CPU：每音符一次 `tan`，成本可接受；
- GPU：本来就在上传 per-voice 状态，直接带上。

不做这一步，旋钮不会生效——这是 (b) 的**真正阻塞项**，而不是格式。

**关于脚本层（Kontakt/KSP）**：不要一开始就做。专业感 80% 来自“声明式参数 + 好的默认宏”，20% 才是脚本。先做参数表 + 归一化 + UI 分组，已足以让人把一个库调出花；真的遇到“参数表达不了的条件逻辑”（如 legato 上下文判断）再上**事件级、确定性、可关**的脚本。

**建议**：(b) 应作为 **.sfy v1 的核心**，优先于容器实现——它不依赖复杂格式特性，却直接兑现“最大愿望”。

---

## 四、「播放结束后跳转，准备时间变久」的归因

### 4.1 已测数据（临时探针，已删除）

| 场景 | 结果 |
|---|---|
| 空载 seek | µs 级，不是瓶颈 |
| 88 voice 长音，CPU 每块 | **168µs**，改动前同场景 **140µs**（**+20%**） |
| 1500 voice | ≈800µs/块（未与基线对比） |
| Starry `Release/Key Default` | 128 个 region，`ampeg_release` 0.9~3.5 秒 |
| Starry `Simplicity` 系列 | 有 10 秒 release 版本 |

### 4.2 三个嫌疑（按可能性排序）

1. **释放层让尾音 voice 数上升（最大嫌疑）**
   现在每个音结束都会创建 `trigger=release` 层；Starry 的释放样本 `ampeg_release` 是 0.9~3.5s（Simplicity 到 10s）。停止/跳转后的“尾音阶段”因此活跃 voice 明显变多 → 尾音渲染更久 → 主观上“要等一下才准备好”。
   **这是功能正确带来的副作用，不是 bug。**

2. **新增的每块 O(V) 释放扫描**（CPU 侧）
   实测 +20%。GPU 侧本来就有同类扫描（`collect_block`），CPU 是新增。

3. **`one_shot` 现在忽略 note_off**
   锤子层（0.3s）、踏板层（0.64s）会完整播完，也拉长尾音阶段。

### 4.3 确认方案（需要你同意才动手）

同一 MIDI + 同一库，记录：

1. `Stop` 之后到 `tailing=false` 的墙钟时间 + 该区间峰值 voice 数；
2. 换 `Standard (No Hammer)`（释放层更少）对比；
3. 插值切回 `Nearest` 对比（排除插值影响）。

### 4.4 若确认是第 1 条，推荐解法（不回退功能）

- **尾音阶段不再新建释放层**（尾音里的释放声已经听不出层次）；
- `TAIL_SILENCE_BLOCKS` 从 4 降到 2，`TAIL_SILENCE_PEAK` 可适度放宽；
- 尾音阶段用更小的 ring 目标（已有 `preview_target_frames` 机制）。

---

## 五、建议的执行顺序

1. **补 LGPL 声明**（10 分钟，避免带风险发布）。
2. **修 SF2 `exclusiveClass`**（鼓组掐音，纯 bug，收益明确）。
3. **用新 `.app` 听一遍 Starry**，并确认第四节“准备变久”能否接受。
4. **确认 (b) 的方向**：若认同“参数表 + 相对偏移 + 工程覆盖层”，先出 `.sfy` 参数模型 spec（不含容器），可独立于打包落地。
5. `.sfy` 容器（ZIP64 + 原始字节 + offset 表 + 流式）放在参数模型之后——它要服务的正是手机内存。

---

## 六、待你决定

1. LGPL：先补声明（快），还是直接排“摘掉 xsynth”的迭代（彻底）？
2. `pitch_keytrack` / `transpose`：是否允许我先用参考播放器 A/B，再决定语义？
3. `.sfy` v1 是否以「参数模型 + 覆盖层」为核心（而非容器/脚本）？
4. 第四节的三项对比测量，是否可以现在就跑？
