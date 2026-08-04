# GPU Cull 音符丢失问题调研记录

> 状态：**未解决**。已定位到渲染层（`multi_draw_indirect` + 大 `first_instance`），根因待确认。
> 本文件供新会话快速接续，包含全部已确认事实、复现方式、关键数据和下一步实验。

## 1. 用户报告的问题

- GPU Cull 模式下，**每个 key 只能显示前几千/几百个音符，后面的全部丢失**，每个 key 的丢失范围不一样
- 从歌曲开头能看到，滚动/播放到歌曲一小半后开始丢失
- 小 MIDI 正常；**大于 ~10MB 的 MIDI**（音符数几万到上亿）出现丢失
- 1.64 亿音符的 start.mid 只显示前面一点，后面一大片丢失
- **CPU 构建模式（非 cull）显示完全正常**
- 只显示单轨时同样中断，且**中断位置可复现、因 key 而异**
- 铅笔工具能探测到"本该显示却显示不出来"的音符（数据存在、位置正确）
- 后半段帧率卡顿严重
- 与拖拽/编辑无关：打开后直接滚动就出现

## 2. 环境

- 测试 MIDI：`/Users/jieneng/Music/MIDIs/`
  - `start.mid`（1.2GB，1.64 亿音符，309 万 ticks，799 轨）——主要复现文件
  - `Night Voyager.mid`（146MB，1890 万音符，174 万 ticks，130 轨）——像素验证用
  - `test.mid`（76MB，998 万音符，6.2 万 ticks——每 tick 162 音符，极度密集）
- 视口参数（真实用户值，`cull_start_mid_sequence` 中用过）：
  - width=1376, height=419, key_height=3.2734375, keyboard_width=60, ppu=0.026372144
- 测试环境：headless wgpu（`headless_device()`），测试位于 `crates/yinhe-wgpu/src/cull.rs` 的 `mod tests`

## 3. 代码架构（相关文件）

- `crates/yinhe-wgpu/src/cull.rs`：CullState（per-key buffer、bucket/chunk 索引、dispatch、draw）
- `crates/yinhe-wgpu/src/cull.wgsl`：compute shader（AABB 裁剪 + 前缀和 + 写入 visible buffer/draw_args）
- `crates/yinhe-wgpu/src/renderer.rs`：InstanceRenderer（draw_with_cull → dispatch_cull + draw_visible_notes）
- `crates/yinhe-wgpu/src/shader.wgsl`：vs_main_note（顶点位置计算）
- `crates/yinhe-egui/src/piano_view/gpu_upload.rs`：上传状态机（全量/增量）
- `crates/yinhe-egui/src/piano_view.rs`：show() 中的 cull 路径

## 4. 已提交的修改（调研过程中的修复，均保留）

| commit | 内容 |
|---|---|
| `f419fbb` | 添加 3 个精确复现测试（合成大 key、真实大 MIDI 相对视口、多帧交互序列）——全部通过 |
| `de3e82b` | rescale（PPQ 缩放）替换 model 后未 invalidate cull 状态 → 已修复 |
| `0e07750` | **KeyBucketIndex 从 bucket 级(4096 音符)改为 chunk 级(256 音符)+ 块级(64 chunks) prefix/suffix 双向索引**，dispatch 从前缀 `[0, c_hi)` 改为区间 `[c_lo, c_hi)` |

⚠️ **`0e07750` 的区间 dispatch 引入了新问题（见第 7 节）——这是当前最重要的线索。**

未提交改动（工作区）：
- `cull.rs`：多个诊断测试 + 像素读回测试 + 最小复现测试
- `renderer.rs`：`cull` 字段改 pub(crate)、`last_diag_bar`、`cull_diag_bar`（YIN_CULL_DIAG=1 时每小节打印 CPU vs GPU）
- `piano_view.rs`：paint 后调用 `cull_diag_bar`

## 5. 已确认的事实链（按验证顺序）

### 5.1 draw_args 计数正确（GPU ≥ CPU）

`cull_vs_cpu_build_path_per_bar`：start.mid 全曲 403 小节（每小节一个视口），GPU cull 的
`instance_count` 总和 **≥** CPU 构建路径（`build_notes`），**0 个 key 少于 CPU**。
GPU 多出的部分 = 键盘区域下（x∈[0,60px]）的音符（shader 左边界比 CPU 宽 kb_w/ppu ticks）。

### 5.2 visible buffer 内容正确

`cull_visible_buffer_content_check`：读回 visible buffer 槽位，与 CPU 期望逐项一致
（start_tick/end_tick/packed 完全相同）。

### 5.3 ❌ 真实渲染像素远少于 CPU 期望（关键）

`cull_render_pixel_check`：用**真实渲染管线**（InstanceRenderer::draw → draw_with_cull）
画到 texture 并读回像素，对比 CPU 模式（legacy + build_notes）：

```
tick=87168  cull像素=2766   cpu像素=22836（40356 实例）
tick=435840 cull像素=10147  cpu像素=23368（56418 实例）
tick=1089600 cull像素=0     cpu像素=46904（98576 实例）
```

- legacy 路径（`pass.draw` + build_notes 上传的 layer buffer）渲染**完全正常**
- cull 路径（`multi_draw_indirect` + visible buffer 作为 vertex buffer）画出的像素只有 CPU 的 12%~43%，大量位置为 0

### 5.4 ❌ 画出的 key 几乎全是 `c_lo = 0`（关键模式）

tick=435840 的逐 key 对比（key, 像素, chunk数, c_lo, cpu实例）：

```
画出的 key 示例: (21, 35, 10, 8, 156)  ← c_lo=8 只画出 35 像素
                 (0, 84, 3, 0, 525)   ← c_lo=0 正常
                 (60, 108, 3, 0, 601) ← c_lo=0 正常
有 CPU 音符但 0 像素的 key: (22, 1047, 21, 16)  ← c_lo=16 完全没画
                           (27, 1048, 38, 33)  ← c_lo=33 完全没画
                           (34, 155, 26, 25)   ← c_lo=25 完全没画
                           (39, 157, 10, 8)    ← c_lo=8 完全没画
```

**结论：`c_lo ≠ 0` 的 key 画不出来（或极少），`c_lo = 0` 的 key 正常。**
`c_lo ≠ 0` 是 `0e07750` 区间 dispatch 的新产物（旧前缀索引 c_lo 恒为 0）。

### 5.5 ❌ 最小复现成功（决定性）

`cull_draw_c_lo_nonzero_minimal`：手工构造 key 60（2560 音符 = 10 chunks），两个视口：

```
场景A（视口 tick [0, 49942]，c_lo=0, cc=2）:
  args = [(6, 256, 0, 0), (6, 244, 0, 256)]     ← first_instance = 0, 256（小值）
  槽位 0 起音符: [0,10,...], [100,110,...]         ← 数据正确
  像素 = 2048 ✓ 画出来了

场景B（视口 tick [221497, 273755]，c_lo=8, cc=10）:
  args = [(6, 88, 0, 2048), (6, 256, 0, 2304), (6, 0, 0, 2560), (6, 0, 0, 2816)]
                                                          ← first_instance = 2048, 2304（大值）
  槽位 2048 起音符: [221600, 221610,...], [221700, 221710,...]  ← 数据正确
  像素 = 0 ✗ 画不出来
```

**数据全部正确（draw_args、visible buffer 槽位内容），但 first_instance 大值（≥2048）的
multi_draw_indirect 画不出来。** legacy 用 `pass.draw(0..6, 0..count)`（first_instance 恒为 0）所以正常。

## 6. 已排除的原因

- ❌ dispatch 范围/bucket 索引逻辑错误（单帧、多帧、全曲测试全部 GPU ≥ CPU）
- ❌ visible buffer 内容错误（逐项一致）
- ❌ draw_args 内容错误（逐项一致）
- ❌ 上传状态机（全量/增量/切轨/隐藏，多帧交互测试通过）
- ❌ f32 精度（tick < 2 亿时误差 < 0.5px，start.mid 只有 309 万 ticks）
- ❌ 数据排序/异常（CPU 模式正常）
- ❌ uniforms 时序（upload_uniforms 在 paint 前，cached_uniforms 同步）
- ❌ shader 写入越界（c_lo+wg < chunk_total 已验证）
- ❌ GPU 过载/超时（单轨 chunk 很少也中断；headless 同样复现）

## 7. 当前假设

**根因假设：`multi_draw_indirect` 的 `first_instance` 大值时绘制失败。**

- 场景A（first_instance = 0/256）画得出来；场景B（first_instance = 2048/2304）画不出来
- 数据层全部正确，唯一差异就是 first_instance 的值
- wgpu/Metal 对 indirect draw 的实例访问（`instanceStart * stride`）超出某内部限制时
  可能丢弃整个 draw（即使按 buffer 大小计算并未越界——vis buffer 30720 字节，
  first_instance=2304 → 字节 27648，+256 实例 × 12 = 30720 恰好到末尾）
- 已知 wgpu 的间接绘制在部分驱动上有对齐/大小检查的坑

**备选假设：** vertex buffer 绑定的 size 与 indirect draw 实例范围的交互（正好等于 buffer
末尾的访问被丢弃）；或 headless 与 Metal 共享此行为（用户真实设备同样丢失，符合）。

## 8. 下一步实验建议（新会话）

1. **验证 first_instance 边界**：在最小复现测试里加场景 C——把可见音符 CPU 拷贝到槽位 0，
   手工改 draw_args 的 first_instance=0，重画（绕过 dispatch，直接构造 RenderPass 调
   `draw_visible_notes`）。若画出 → 100% 确认 first_instance 大值是根因。
2. **尝试修复方案 A（推荐）**：draw 时把 vertex buffer 绑定为
   `vis_buf.slice(c_lo*256*12..)`（偏移到 c_lo 起点），args 的 first_instance 改为
   `wg*256`（相对小值，从 0 开始）。需要 CullState 存每 key 本帧 c_lo
   （dispatch_cull 里已有，存到数组即可）。shader 写 args 时 first_instance 也改为 `wg*256`。
3. **尝试修复方案 B**：放弃 `c_lo` 非零（区间 dispatch 改为"只 dispatch 视口附近 + 前缀"的
   折中），或 c_lo 永远为 0 但 shader 端用 `chunk = wg`（回到前缀，接受性能损失）。
4. 修复后跑 `cull_draw_c_lo_nonzero_minimal`（场景B 应 >200 像素）+ 全曲像素对比
   （cull 像素应接近 cpu 像素的 ~90%+，差异仅为键盘边界）。
5. 用 `YIN_CULL_DIAG=1 ./target/release/yinhe-egui` 在真实环境滚动到丢失位置，
   对比每小节的 cpu/gpu 计数（`[cull-diag]` 日志，已在代码中实现，未提交）。

## 9. 相关测试清单（cull.rs mod tests）

| 测试 | 用途 | 状态 |
|---|---|---|
| `cull_mid_song_exact_per_key` | 合成 5 万音符/key × 4 视口，GPU vs f32 镜像 | 通过 |
| `cull_real_large_midi_relative_viewport` | 4 个大 MIDI × 25/50/75% 视口，GPU vs f32 镜像 | 通过 |
| `cull_multi_frame_interaction_sequence` | 滚动/skip/编辑/切轨/隐藏多帧状态机 | 通过 |
| `cull_vs_cpu_build_path_per_bar` | start.mid 全曲每小节 CPU 真实路径 vs GPU | 通过（GPU ≥ CPU） |
| `cull_visible_buffer_content_check` | visible buffer 内容逐项验证 | 通过（内容正确） |
| `cull_render_pixel_check` | 真实渲染像素 vs CPU 像素（**暴露问题**） | **失败（cull << cpu）** |
| `cull_draw_c_lo_nonzero_minimal` | 最小复现（**决定性证据**） | **失败（c_lo≠0 画不出）** |
| `cull_single_track_fine_scroll_interrupt_detect` | 单轨精细滚动中断检测 | 通过（无中断，track0 空轨需换轨） |

## 10. 用户附带报告的其他现象（与本问题可能无关，待确认）

- 框选时 info panel 音符数计数"透过其他音轨"（PR 选区是 tick×key 矩形语义，包含所有轨道，
  可能是设计如此；若期望单轨模式只选该轨需改 selection 逻辑）
- 后半段卡顿（dispatch 巨大：歌曲末尾每帧 64 万 chunks = 1.64 亿线程；区间索引已缓解
  无长音符的 key，但黑乐谱开头长音符仍会把 c_lo 拉到 0 → 改进有限）
