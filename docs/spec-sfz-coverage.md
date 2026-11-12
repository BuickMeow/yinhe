# SFZ 语义覆盖：从「塑料感」到专业音色

> 状态：执行中（2026-11-12）
> 关联：`spec-yinhe-synth.md`（音源层架构）、`spec-keymap-memory.md`（展开内存）、`plan-audio.md`（M0 四点插值）
> 范围：只改音源层（`yinhe-synth`）。不改 `.yin` 容器、不做 `.sfy`、不改 Flutter/egui 布局。

## 一、问题陈述

用户的主观反馈是「MIDI 听起来塑料」。排查后确认：**不是精度不够，也不是格式太老，而是音色库里决定「像不像真乐器」的那几层语义被引擎整段丢弃了。**

旗舰测试库 `Starry Studio Grand v2.7` 的实测证据：

| 库里写了什么 | 位置 | 现状 |
|---|---|---|
| `xfin_locc14/15/20` + `set_cc14=127 / 15=85 / 20=100` | `Presets/A_Standard/Studio Grand - Standard.sfz:10-21,281,878` | 未识别，三层（琴键/锤子/踏板）恒定叠响，CC 调节完全无效 |
| `trigger=release` + `on_locc64=0/127` | `Presets/Extensions/Pedal/Pedal 1` | 未识别，踏板释放声全部丢失 |
| `pitch_random=182`（cents） | 同上 :25 | 未识别，每个音相位完全相同，重复段机感重 |
| `amplfo_depth=-0.8 / amplfo_freq=0.500195` | 每个 `<group>` | 未识别，长音没有呼吸 |
| `loop_mode=one_shot` | 同上 :23 / `Pedal` master | 未识别，踏板噪声被 note_off 硬切 |
| `group` / `off_by` | 通用 SFZ 鼓组 | 未识别，断音互斥失效 |
| `seq_length/seq_position` | 通用 round-robin 库 | 未识别，RR 退化为只用第 1 层 |
| 4 点插值 | — | `Interpolation::default() == Nearest`（`yinhe-types/src/synth.rs:41`），高移调时沙粒感 |

根因有两个，都在 `yinhe-synth`：

1. **解析层**：`sf_parser/sf_parser/sfz.rs` 直接消费 `xsynth-soundfonts 0.4` 的 `RegionParams`，而它的 `parse_sfz_opcode`（`sfz/parse.rs:223`）只认 39 个 opcode，其余静默丢弃。上面这些字段一个都读不到。
2. **运行层**：`KeyInfo` 是加载期 `(key, vel)` 快照，只承载静态参数，没有任何「按 CC / 按键开关 / 按轮次 / 按释放时机」的选择信息。

因此优先级不是「发明新格式」，而是**把 SFZ 已经规定、库作者已经在用、我们却丢掉的那部分语义补回来**。

## 二、取舍（明确不做/暂缓）

先划清边界，避免把「专业」误解成「把所有 ARIA opcode 都实现」：

- **暂不做 `pitch_keytrack`**：Starry 的 `<master>` 写了 `pitch_keytrack=0`，但每个 key region 都靠 `key - pitch_keycenter` 变调；照字面实现会让整架钢琴变成单音。需要用参考播放器 A/B 后再定。
- **暂不做 `sustain_sw`**：SFZ v2 里它是开关状态而非「禁用踏板」，语义与现状冲突，先保留忽略。
- **暂不做 `fillfo_*`（滤波 LFO）**：biquad 系数是加载期烘焙的，运行期改系数要动 GPU voice 状态写回；Starry 只给了 `fillfo_freq` 没给 `fillfo_depth`，当前无实际影响。
- **暂不做 `effect1`（ARIA 混响发送）**：等 GM2 Reverb 落地（`spec-yinhe-dsp.md` 阶段 B）。
- **不做 `.sfy`**：格式的前提是引擎先能播对 SFZ；否则只是「更好看的错误播放器」。
- **不做脚本层**：先做声明式选择器；确认表达不了再加事件级脚本（确定性、有预算、可关）。

## 三、本轮实现（已落地）

> 实现状态：CPU / GPU 双后端均已完成并通过测试；`Starry Studio Grand` 实测
> 5250 region 中 `xfin` 5250、`trigger=release` 2、`on_cc` 2、`one_shot` 642、
> `pitch_random` 5250、`amplfo` 4608 全部进入引擎，缺失采样 0。

### 3.1 自有 SFZ 解析器（`yinhe-synth/src/sfz/`）

替换 `xsynth-soundfonts` 的 SFZ 解析（SF2 仍走 xsynth，二进制解析没问题）。理由：

- 需要 `xfin_*` / `trigger` / `seq_*` / `sw_*` / `off_by` / LFO 等 xsynth 不解析的字段；
- 顺带解掉 SFZ 路径的 LGPL 依赖；
- 诊断（`sf_diagnostics`）改为直接读解析器的「未识别 opcode」，不再维护一份手写白名单。

结构（不用 `mod.rs`）：

```
src/sfz.rs            # 公共入口 parse() / 数据结构再导出
src/sfz/types.rs      # SfzRegion / LayerExtra / XfGate / CcGate / Lfo / Trigger
src/sfz/lexer.rs      # 行级词法：#include / #define / 标签 / opcode=value
src/sfz/opcodes.rs    # opcode → 类型化字段（含编号后缀 xfin_locc14）
src/sfz/region.rs     # 层级继承 control<global<master<group<region>
```

继承规则对齐 SFZ 规范与 xsynth 旧实现（`control=1 global=2 master=3 group=4 region=5`），`<region>` 结束时落到一条 `SfzRegion`。采样路径 = `文件目录 / default_path / sample`，`\` 归一为 `/`，缺失采样记入 `missing` 而不是静默丢弃（供 lint 报告）。

### 3.2 `LayerExtra` 侧表（内存红线）

门控参数是**每 region 一份**，不是每 `(key, vel)` 一份。直接塞进 `KeyInfo` 会让 GeneralUser 的 670 万条快照再涨 ~375MB。

方案：`KeyMapEntry` 持有 `Arc<[LayerExtra]>`，`KeyInfo` 只存 `extra: u32` 索引（`u32::MAX` = 无门控）。`KeyInfo` 只 +4B；SF2 条目的侧表为空。

选择函数返回 `Selected { infos: &[KeyInfo], extras: &[LayerExtra] }`，仍零分配；门控在 `note_on` 逐层判定（`GateCtx`）。

### 3.3 运行期门控（note_on 时求值）

| 语义 | 实现 |
|---|---|
| `xfin_loccN/hiccN`、`xfout_*` | 线性增益（`(cc-lo)/(hi-lo)`，`xfout` 取反）。CC 值来自新增的 `ChannelState.cc: [u8;128]` |
| `<control> set_ccN=v` | 加载音色库时写入通道初始 CC（否则 piano 在 CC14=0 下会被静音） |
| `on_loccN/on_hiccN` | 二进制区间门（踏板噪声按 CC64 选层） |
| `sw_last` / `sw_lokey-hikey` | 通道记录 `sw_last`；开关区内的键被吞掉（不发声） |
| `seq_length/seq_position` | 每 `(channel, key)` 轮次计数器，`pos == counter % len + 1` 命中 |
| `lorand/hirand` | 确定性随机（note 序号哈希），离线导出与回放一致 |
| `pitch_random` | 确定性 cents 抖动 |
| `group` / `off_by` | 新音触发时按 `off_by` 掐掉同组旧音（仅 `off_by != 0` 时扫描，避免热路径全扫） |
| `loop_mode=one_shot` | 忽略 note_off/damper，播到采样结束 |
| `amplfo_*` / `pitchlfo_*` | 每 voice 相位累加；幅度 LFO 乘增益，音高 LFO 乘播放速率 |

`xfin/xfout` 只在 note_on 时求值（CC 变化不追溯已发声 voice），与多数播放器一致，也避免 GPU 逐块回写。

### 3.4 `trigger=release`

释放采样必须在「音符结束」时创建 voice，而引擎的事件表**不生成 NoteOff**（由 `end_sample` 替代）。做法：

- CPU：块渲染前预扫描 `end_sample ∈ [block_start, block_end)` 的 voice，按组去重后以 `start_offset = end_sample - block_start` 创建 release 层；踏板松开（CC64 下降）时为所有 `held_by_damper` 的音补触发。
- GPU：复用 `schedule.rs` 已有的到期释放扫描，在同一位置以 `trigger=Release` 调 `note_on(block_frame = 释放帧)`。

release 层沿用 note_on 的力度；没有力度信息时用组内主 voice 的力度。

### 3.5 插值：新增 4 点 Hermite

`Interpolation` 增加 `Cubic`（CPU + GPU/WGSL 同步），设置页可选项。默认仍为 `Linear`（黑乐谱 1.4 万 voice 下 +2 次读采样，需实测后再决定是否改默认）。

4 点取帧在循环内要对邻居做回绕（否则每个 loop 周期在接缝处出现一次咔哒）。

## 四、验证

1. 单元测试：词法/继承/include/define/缺失采样；编号 opcode；每个门控的命中与不命中；`off_by` 掐音；`one_shot` 忽略 note_off；确定性随机。
2. 真实库回归：`Starry Studio Grand`（5250 region）解析后，`xfin`/`trigger`/`pitch_random`/`amplfo` 全部落在 `LayerExtra`，`missing` 为空。
   复现：`cargo run --release -p yinhe-synth --example sfz_report -- <file.sfz>`。
3. 端到端渲染：`cargo run --release -p yinhe-synth --example sfz_render -- <file.sfz> out.wav`。
4. 交叉对比：CPU vs GPU 逐样本（`tests/parity.rs`）在 1.66% 峰值内一致；含新语义的音色库跳过与 xsynth 的对比（xsynth 丢弃这些 opcode，参照解必然失真），语义由 `tests/sfz_semantics.rs` 逐项断言。
5. `cargo test -p yinhe-synth`、`cargo clippy -p yinhe-synth --all-targets`、`cargo fmt`。

## 五、后续（不在本轮）

- 参考播放器 null test 工具（`cargo xtask ab`）。
- 力度层交叉淡化、note-off velocity、循环 crossfade。
- 多麦克风位置（`mic=`）——真正的专业能力，需要容器与混音支持。
- `.sfy`（ZIP + SFZ 派生文本 + 多 preset + 许可/来源 + lint），等引擎正确后再做。
