# Key Map 内存优化调研

> 记录 key map（SoundFont/SFZ 展开）内存膨胀的根因、实测数据、以及各种优化路子的评估。
> 相关背景：黑乐谱编辑器需支撑 1 亿音符，音色库展开内存曾达 GB 级。

## 一、背景与问题

音色库（SF2/SFZ）用 **region** 描述范围：`keyrange`（覆盖哪些键）+ `velrange`（覆盖哪些力度）。
`crates/yinhe-synth/src/sf_parser` 采用**加载期展开**：把 `keyrange × velrange` 的每个
`(key, vel)` 组合预计算成一条 `KeyInfo`（最终合成参数快照），播放时 `note_on` **零公式计算**、
直接查表（见 `sf_parser.rs` 顶部注释）。

代价是内存按 `键数 × 力度值数` 爆炸：

- GeneralUser-GS.sf2：**6,715,044 条 KeyInfo**。
- 若 `KeyInfo` 为 136 B/条 → key map 结构约 **915 MB**（见实测）。
- 叠加音色库样本重采样缓冲、以及预览曾额外加载的 xsynth 版音色库，整体可到 GB 级。

`KeyInfo` 定义：`crates/yinhe-synth/src/sf_parser.rs:40`；展开循环：`sf_parser/sf2.rs`、`sf_parser/sfz.rs`；
唯一参数展开点（CPU/GPU 共用）：`crates/yinhe-synth/src/voice_params.rs:61`。

## 二、实测数据（基线）

### 2.1 解析/结构内存（`keymap_size` example，48kHz，两库）

| 指标 | B 前 | B 后（当前） |
|---|---|---|
| `size_of::<KeyInfo>()` | 136 B | **120 B** |
| KeyInfo 条数 | 6,715,044 | 6,715,044 |
| key map 结构内存 | 915.2 MB | **805.8 MB** |
| 样本数据（去重） | 544.0 MB | 544.0 MB |
| 合计 | 1459 MB | 1352 MB |

复现：
```sh
cargo run --release -p yinhe-synth --example keymap_size -- \
  "…/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz" \
  "…/generaluser-GS.sf2"
```

注：Starry SFZ 展开本身只有 16384 条 KeyInfo（~1.9MB），它的解析峰值（~461MB）来自**样本重采样缓冲**，不是 KeyInfo。

### 2.2 曾实测过的改动效果（参考）

| 改动 | 效果 | 状态 |
|---|---|---|
| **F** 预览跟随主引擎 + yinhe 后端跳过 xsynth 音色库 | xsynth 版 `SoundFont` 1250.8 MB → **0** | 已保留（`9fd71e05`、`3a54c2c2`） |
| **B** KeyInfo 瘦身 | KeyInfo 136 → 120 B（-12%） | 已保留（`c25f7d3e`） |
| 合并力度维度（每 region 一条，取中值；原实验） | 解析峰值 1.06GB → 461MB；GeneralUser 956 → 188MB；GUI 常驻堆 4428 → 1020MB（含 F） | **已回退**（`c09ff1b2`，力度曲线变平，见 §5） |

## 三、本质：调制参数的可分离性

逐 `(key, vel)` 变化的量，几乎都能写成 **key 项 + vel 项（加性）** 或 **key 项 × vel 项（乘性）**：

- `speed_mult = 2^((key-root)·scale/12 + tune/1200)` → key 项 + 常量（cents 域加性）
- `volume = (vel 曲线) × 10^(key_track/20)` → key 项 × vel 项（dB 域加性）
- `pan = clamp(base + vel·pan_veltrack + key·pan_keytrack)` → 加性可分离
- `cutoff = cutoff_t · 2^((vel·fil_veltrack + key·fil_keytrack)/1200)` → cents 域加性
- `ampeg_release = base + vel·vel2release` → 加性

而 region 常量（`sample_data`、`loop_*`、`offset`、`stop`、`ampeg_*`、`resonance`、`filter_type`…）
对该 region 的所有 `(key, vel)` **完全相同**。

结论：670 万条 KeyInfo 里**一大半字节是逐条重复的 region 常量**，真正随 `(key,vel)` 变的只有几个标量。
这是所有优化的切入点。

## 四、方案清单

评估维度：内存收益 / note_on 开销 / 是否无损 / 实现量 / 与项目偏好（无缓存、无阈值、无 vendor）契合度。

### A. 结构化分离：region 常量上提

- **做法**：`KeyMapEntry` 增加 region 表（`sample_data`/`loop`/`offset`/`stop`/`ampeg`/`resonance`/`filter_type`… ≈ 55–60 B/region）；
  `KeyInfo` 退化为 `region_idx(2–4B) + 可变标量`。
- **内存**：670 万 × ~55B ≈ **省 350 MB+**，region 数量远小于 670 万。
- **note_on**：多一次数组索引，几乎为零。
- **无损**：是。
- **实现量**：中等（构建 + 播放侧改为 `(region, info)` 两段读取）。
- **偏好**：契合（无缓存/无阈值）。

### B. 可分离/低秩分解：K×V → K+V

- **做法**：变量不存张量，存 `key_term[K]` + `vel_term[V]`（或共享 vel 曲线 LUT），note_on 在 dB/cents 域做几次乘加；clamp/powf 收尾算一次。
- **内存**：从 `K×V` 降到 `K+V`（如 60 键 × 42 力度 = 2520 条 → 102 个数），**数量级下降**。
- **note_on**：几次乘加，无三角函数。
- **无损**：对可分离调制无损；SF2 modulator 系统需单独设计（较复杂）。
- **实现量**：大。
- **偏好**：契合。

### C. 惰性 / per-preset 展开

- **做法**：只展开工程实际用到的 `(bank, preset)`，首次用到时展开一次（GM 库 287 preset，通常只用几个）。
- **内存**：可能 90%+（取决于用几个），完全无损。
- **note_on**：零额外成本（展开在加载/首次使用，不在触发路径）。
- **无损**：是。
- **实现量**：中等。
- **偏好**：本质"按需"，会被归类为缓存，但是一次性必要展开、非回收式缓存。

### D. 曲线参数化（LUT / 分段线性）

- **做法**：vel 曲线不存 127 点，存 N 锚点 + 线性插值。
- **内存**：N/127 倍。**note_on**：一次插值（乘加）。
- **取舍**：N 是精度参数，本质是曲线降采样；听感可无损，但非逐位无损，且形似"分段/阈值"。
- **实现量**：小。

### E. 量化 / 位打包

- **做法**：f32 → f16/定点，或按实际值域位打包（如 `speed_mult` 16 位）。
- **内存**：约 2–4 倍。**有损**；实现简单。

### F. 三角函数专治（biquad / pan）

- `pan_gains`：`pan∈[0,1]`，预存 256 档 `pan→(l,r)` LUT，note_on 查表，省 cos/sin。
- `bake_biquad`：cutoff 在 cents 域可分离；biquad 系数可对 log-cutoff 做 LUT。
- **目的**：若走运行期展开，用于避开最贵的三角函数。
- 实现量：小；**无损/近似无损**。

### G. note_on 侧批处理 / SIMD

- 黑乐谱同段大量 NoteOn；GPU 路径已有"完全重复 NoteOn 合批（dup+1）"（`gpu_synth/schedule.rs`）。
- CPU 侧可批量化/向量化，把每音符公式求值摊到多条（项目已有 `yinhe-dsp` SIMD 经验）。
- 收益取决于触发密度；实现量中等。

### H. 其他（不推荐或高成本）

- **稀疏/游程表示**：region 的 key/vel range 本就是区间，保留 region 表示即最小内存，但 note_on 需区间查找。
- **mmap + 按页解压**：展开结果压缩存盘、懒调页；引入 IO 与复杂性，不推荐。
- **力度分档**：0–127 量化到 N 档，只展开 N×keys；简单但有损，且属"档位/阈值"，偏好不符。

## 五、合并力度维度的损失（那次实验的回退原因）

该实验让每个 region 只留一条 `KeyInfo`，参数取 `velrange` **中值**。损失是**同一力度层内的动态被抹平**。

### 例 1：层内动态塌平

某钢琴 region：`keyrange 60-62, velrange 1-42`，`amp_veltrack=100%`。sfz 音量公式化简为
`volume = (vel/127)²`：
- 原：vel=1 → `(1/127)² ≈ 0.00006`；vel=42 → `(42/127)² ≈ 0.109`，**层内相差约 1800 倍（≈65 dB）**。
- C 后：整层用 vel=21 → `(21/127)² ≈ 0.027`。**vel=1 被抬高约 26 dB，vel=42 被压低约 6 dB**。

### 例 2：单层库力度完全失效

若一个采样覆盖 `velrange 1-127`（很多简化 GM 库/某些钢琴 preset），C 后整个键只有一条参数 →
**敲轻敲重完全一样，力度信息丢失**。

### 例 3：力度控制音色的乐器

- 弦乐/pad：`fil_veltrack` 被固定在中点 → 重力度不再打开滤波。
- `ampeg_vel2release`：层内释放时间统一。
- `pan_veltrack`：层内声像固定。
- SF2 modulator 系统中随 vel 变的所有量（甚至微调）全部塌成中点。

### 边界

- **层与层之间不受影响**（不同 region / 不同采样）。
- **力度层越多损失越小**（8 层 → 8 级台阶，仍丢 127 级连续性）；**层越少（尤其 1 层）损失越大**。
- 黑乐谱（重密度、轻细腻动态）损失相对可忍；独奏钢琴/细腻乐段明显。

## 六、运行期展开的性能损耗在哪

运行期展开 = 不预存 `(key,vel)→KeyInfo`，`note_on` 时按 region + `(key,vel)` 现算 `VoiceParams`。

- **损耗集中在 note_on 热路径，不在渲染**。已 spawn 的 voice 混音完全不变。
- 会挪到 note_on 的贵操作：
  1. `bake_biquad → biquad_coeffs`（`sf_parser.rs`）：RBJ 含 `sin`/`cos`。
  2. `pan_gains`：`cos`/`sin`。
  3. `2.0f32.powf` / `10.0f32.powf`：指数函数。
  4. SF2 `region.note_params(key, vel)`：遍历 modulator 系统，最重。
  5. region 查找：当前 `map[key]` O(1)，运行期展开可能要扫 region（需另建索引）。
- **黑乐谱放大**：峰值每秒几十万~上百万 note_on；且同一 `(key,vel)` 会被重复触发成千上万次，
  展开式"算一次查 N 次" vs 运行期"每次重算"。
- **单线程 dispatch**：`CpuSynth::render_range` 按事件 sample 顺序 `dispatch_event`，计算串行压渲染线程。
- **违背原设计目标**：`sf_parser.rs` 注释明确"note_on 时零公式计算"。
- 量级（估算）：每 note_on 多算 ~2 次三角 + 几次 powf + 分支 ≈ 几十~几百 ns；100 万 note_on/s × 100ns ≈ 0.1 核。

变体：按 key 惰性展开 + 缓存（对"只用几个 preset"收益大，但黑乐谱会全展开，且本质缓存）；
把 vel 曲线存系数、note_on 轻量求值（最接近"无损 + 省内存 + 低开销"，但实现复杂）。

## 七、推荐排序

1. **C2（per-preset 惰性）**——最高性价比、无损、零 note_on 成本。
2. **A（region 常量上提）**——无损、note_on 几乎零成本、对所有 preset 通用。
3. **B（可分离分解）**——A 之后若还要压，最大一步，也能解决"运行期展开怕慢"。
4. **F（trig LUT）**——若走运行期展开，必要前置。

`A + B` 合起来，理论可把 key map 从"K×V 条完整结构"打到"K+V 个标量 + 一张小 region 表"，
接近 C 的降幅而**无损**——既不牺牲音质、又不大增 note_on。

## 八、相关代码位置

| 内容 | 位置 |
|---|---|
| `KeyInfo` 定义 / `select_key_info` | `crates/yinhe-synth/src/sf_parser.rs:40`、`:234` |
| SF2 展开 | `crates/yinhe-synth/src/sf_parser/sf2.rs` |
| SFZ 展开 | `crates/yinhe-synth/src/sf_parser/sfz.rs` |
| 参数展开（CPU/GPU 共用） | `crates/yinhe-synth/src/voice_params.rs:61` |
| CPU synth（dispatch / render） | `crates/yinhe-synth/src/cpu_synth.rs`、`cpu_synth/voice.rs`、`cpu_synth/events.rs` |
| GPU note_on 构建 / 采样上传 | `crates/yinhe-synth/src/gpu_synth/schedule.rs`、`gpu_synth/upload.rs` |
| key map 进程级缓存 | `crates/yinhe-synth/src/sf_cache.rs` |
| 内存基准 example | `crates/yinhe-synth/examples/keymap_size.rs` |
| 预览引擎（F） | `crates/yinhe-audio/src/preview_engine.rs`、`preview_engine/yinhe.rs` |
| worker 音色加载（F） | `crates/yinhe-audio/src/spawn.rs` |
