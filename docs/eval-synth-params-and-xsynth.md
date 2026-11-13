# 音色参数化与 XSynth 依赖评估

> 状态：评估稿（2026-11-13）。对应提交 `1e0ff952`（CC71/74 归音源层 +
> CC32/120/121/123 + exclusiveClass）与 `54c50f77`（BASSMIDI 实测标定）。
> 参数目录与子窗口设计见 `spec-instrument-params.md`。

---

## 零、结论

| 问题 | 结论 |
|---|---|
| 能不能给 synth 加旋钮 | **能，而且刚解除了最大阻塞点**（cutoff 已从加载期烘焙改为 note_on 计算）。缺的是参数声明模型、相对偏移语义、工程覆盖层和 UI 四件事，不是引擎能力。 |
| 能否做到 Kontakt/Halion 级"一个库驾驭各种音乐" | **旋钮能做到 60~70%**；剩下 30% 不在旋钮，在**第二包络**、**力度曲线/交叉淡化**、**多麦克风与流式**。这三项不做，钢琴以外的音色仍然是"能响但不像"。 |
| 要不要扔掉 XSynth | **渲染层可以立刻扔**（yinhe CpuSynth/GpuSynth 已完整）；**解析层（xsynth-soundfonts）还不能立刻扔**，先补 LGPL 声明或自研 SF2 解析。建议先把默认后端切到 YinheCpu，再分两步摘依赖。 |
| Release velocity | **暂缓**。BASSMIDI 官方 implementation chart 里 Note-Off Velocity 本身就是 "no"，黑乐谱也不用。 |
| Aftertouch | **暂缓**。BASSMIDI 支持，但黑乐谱不用，收益远小于成本；Channel Pressure 可作为远期低优先级项。 |

---

## 一、音色可调（旋钮）评估

### 1.1 已经具备的地基

| 能力 | 位置 | 状态 |
|---|---|---|
| per-voice 参数唯一展开点 | `yinhe-synth/src/voice_params.rs` | 有（CPU/GPU 共享） |
| 每个 voice 独立滤波器 | `CpuVoice.flt_*` / `GpuVoiceState.flt_*` | 有 |
| 滤波器实时跟随 CC | `apply_filter_cc`（CPU）/ `apply_filter_dirty`（GPU） | 本次新增 |
| 包络时长实时跟随 CC | `env_frames_for` + `EnvUpdateCmd` | 有 |
| 音色库参数已解析 | `KeyInfo`（cutoff/resonance/amp_*/ampeg_*/pan/tune） | 有 |
| 运行时门控/轮次/release 层/LFO | `LayerExtra` + `sf_parser::evaluate` | 有 |

**本次提交解除的阻塞点**：旧实现把 cutoff 在加载期烘成 biquad 系数
（`bake_biquad`），运行期改不了；现在系数在 `note_on` 按
"库自身 cutoff + CC71/74 偏移"计算，CPU 与 GPU 走同一函数
（`voice_params::filter_coeffs`）。这正是 `spec-sfz-gaps-and-sfy.md` §3.2
标注的".sfy 调音色的真正阻塞项"。

### 1.2 还缺的四层东西

1. **参数声明模型**
   `id / 显示名 / 单位 / 范围 / 默认值 / 归一化映射 / 作用目标（哪些库字段）`。
   这是"一个库被无数人玩出花"的前提。
2. **相对偏移语义**
   用户拧"亮度"应该是给 `cutoff` 加 ±N 音分（或倍率），不是覆盖成 800 Hz。
   CC74 已经按这个语义实现（±9600 cents，64 中性），可以直接复用作"亮度"。
3. **工程级覆盖层**
   覆盖值存 `.yin`（库文件只读），可导出/分享为 `.sfypreset`。不要改库文件。
4. **UI**
   通道/乐器参数面板的旋钮，复用现有自动化 lane 体系（`ParamDevice`）。

### 1.3 建议的第一批旋钮（按收益/成本）

| 旋钮 | 映射 | 成本 | 说明 |
|---|---|---|---|
| Brightness | CC74 同款 cents 偏移（±9600） | 低（已具备） | 最直观 |
| Resonance | CC71 同款 Q 偏移（±25 dB） | 低（已具备） | 与亮度配对 |
| Volume | 线性增益（dB） | 低 | 已有 CC7 通路 |
| Pan | 等功率声像 | 低 | 已有 CC10 通路 |
| Release / Attack | 与 CC72/73 同款曲线 | 低 | 已有 CC 通路 |
| Tune | 音分偏移 | 低 | `pitch_multiplier` 已支持 |
| Velocity curve | 力度→音量映射形状 | 中 | 需要每库参数，不是单一标量 |
| Tone (EQ) | 低架/高架滤波 | 中 | 需要新增滤波器类型 |

> 前六项都能在 `voice_params` 一个点上落地；建议 `.sfy` v1 先交付这六项。

### 1.4 旋钮解决不了的部分（真正的"专业感"缺口）

1. **第二包络（`fileg_*` / `pitcheg_*` / SF2 ModEnv）**
   弦乐、铜管、管乐的"咬字"靠滤波包络。当前引擎只有一条振幅包络。
2. **力度曲线与交叉淡化（`amp_velcurve_*`、`xfin/xfout` 的 vel 版）**
   决定 pp→ff 的手感；现在的力度曲线是固定二次曲线。
3. **多麦克风与磁盘流（只可能由 `.sfy` 承载）**
   close/room/ambient 分轨、按需解码。340MB 采样解码成 f32 ≈ 1.4GB，
   手机必死；这是当前最大的专业度差距。

**结论**：旋钮值得做，而且现在就能做；但不要指望旋钮替代上面三项。

---

## 二、XSynth 依赖评估

### 2.1 现在依赖它的两层

| 依赖 | 用途 | 位置 |
|---|---|---|
| `xsynth-core` | XSynthCpu 后端（`VoiceChannel`）、事件类型（`ChannelEvent`/`ControlEvent`/`ChannelAudioEvent`）、预览引擎、parity 测试 | `yinhe-audio`、`yinhe-synth/tests/parity.rs`、`examples/diag_bend.rs` |
| `xsynth-soundfonts` | SF2 解析（`load_soundfont`）、重采样（`resample_vec`）、`FilterType`/`LoopMode` 类型 | `yinhe-synth/src/sf_parser/sf2.rs`、`sfz/{types,opcodes}.rs` |

注意：`SynthEngine::default()` 目前是 `XSynthCpu`。也就是说：

> **本次 CC71/74 的语义修正只在 YinheCpu / YinheGpu 后端生效。
> XSynthCpu 后端仍然用它自己的通道级低通（旧的错误语义）。**

这是当前最需要拍板的一致性问题。两个可行方向：

- **A（推荐）**：默认后端切 `YinheCpu`，设置页保留 XSynthCpu 一个版本用于 A/B，
  下一版删除。
- **B**：暂时保持默认，接受两个后端声音不同；在设置页明确标注
  "XSynth 后端不支持新的 CC71/74 语义"。

### 2.2 渲染层：可以立刻扔

`CpuSynth` 与 `GpuSynth` 已完整（voice/CC/包络/滤波/门控/释放层/LFO），
且 CPU/GPU 有逐样本 parity 测试。删除 `ChannelSet`（xsynth `VoiceChannel`
包装）只是删代码 + 迁移事件类型。

代价：`ChannelEvent`/`ControlEvent`/`SynthEvent` 这些类型要从 xsynth 复制到
`yinhe-synth`，并改所有 use 点（机械改动，约几十处）。

### 2.3 解析层：不能立刻扔

`xsynth-soundfonts` 的 SF2 解析包含 preset/instrument/zone 三层继承、
generator/modulator 展开、sample 链接、24-bit、立体声拆分。这是硬骨头。

两条路：

1. **短期**：保留 `xsynth-soundfonts`，按 `spec-sfz-gaps-and-sfy.md` §2
   补 LGPL 声明（LGPL 全文 + NOTICE + 源码获取方式）。
2. **中期**：自研 SF2 解析器（只保留本项目需要的 generator/modulator），
   然后两个 xsynth crate 全部删除。

自研成本估计：解析器 800~1500 行 + 用 GeneralUser/TWGMD/Project CF 做回归；
收益是彻底摆脱 LGPL、能直接补 `exclusiveClass` 以外的字段（ModEnv、
VibLFO、24-bit 精度）、不再受 xsynth 的 39 个 SFZ opcode 白名单限制。

### 2.4 建议顺序

1. 默认后端切 `YinheCpu`（一次性，用户可切回）；
2. 迁移事件类型到 `yinhe-synth`，删除 `ChannelSet` 与 XSynthCpu 后端；
3. 补 LGPL 声明（如果 2 之前要发版）；
4. 自研 SF2 解析器，删除 `xsynth-soundfonts`；
5. 删除 parity 测试与 `diag_bend` 里的 xsynth 依赖。

---

## 三、Release velocity 与 Aftertouch

### 3.1 Note-off velocity（Release 力度）

- **BASSMIDI 不支持**：官方 MIDI implementation chart 明确写
  "Note-Off Velocity: no"。
- 黑乐谱几乎不用；当前 SFZ `trigger=release` 层已经用 `info.lovel`
  在 vel==0 时兜底（`spawn_release_layers`）。
- 若做，需要改事件模型（`SynthEvent::NoteOff` 加 velocity）并影响
  note_off 匹配、chase、导出。成本中等、收益接近零。

**结论：暂缓，不做。** 等做管弦乐库的"专业可调"时再评估。

### 3.2 Aftertouch（通道压力 / 键压力）

- BASSMIDI 支持（Channel/Poly 都 yes）。
- 黑乐谱不用；实现要进事件模型 + 运行时调制（vibrato/filter/volume），
  而且每音符事件会让 1 亿音符的存储再涨。
- 现在的 `parser.rs` 是 `_ => {}` 直接丢掉，**不会**产生错误声音，只是不支持。

**结论：暂缓。** 如果以后要做，先做 Channel Pressure（每通道一个值，
成本低），Poly Pressure 不做。

---

## 四、这次改动顺带发现、需要知道的事

1. **XSynthCpu 后端语义不一致**（见 §2.1），默认后端需要决策。
2. `ChannelState::process_control` 返回值改为 `ControlAction`，
   CPU/GPU 共用；此前 CPU 完全没有 CC120/123。
3. `CC121` 重置不再清掉 `program`（旧实现连 program 一起归零，是 bug）。
4. `KeyMapEntry.bank` 由 `u8` 改为 `u16`：此前 `preset.bank as u8`
   会截断 SF2 的 16-bit bank。
5. `KeyInfo.biquad` 删除，每条 (key,vel) 快照省 24 字节；
   GeneralUser 全量展开约省 160MB（惰性展开时收益按实际使用的 preset 计）。
6. `yinhe-dsp` 现在只剩：`cc/gain`、`cc/pan`、`dsp/limiter`、`dsp/smooth`、
   `registry`（两个内置效果器）。
