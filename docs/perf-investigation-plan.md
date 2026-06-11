# 50fps 瓶颈排查计划

## 背景
Cursor 解耦后，症状：

| 场景 | real_fps | cpu/frame | upload | rb |
|---|---|---|---|---|
| 小 MIDI (33 万) 播放+不动 | 61 | 0.01ms | 0 | 0/60 |
| 大 MIDI (4400 万) 播放+不动 | 51 | 0.03ms | 0 | 0/60 |
| 大 MIDI 播放+滚动 | 40-52 | 0.4ms | ~0.05 | ~60/60 |

CPU 不是瓶颈。Cursor 解耦后 upload=0 在不动时。但大 MIDI 仍卡 51fps，小 MIDI 稳定 61fps。

## A — paint() GPU timeline 分段计时
在 `render_ctx.paint()` 中细拆：

- `encoder.create_command_encoder()` 耗时
- `renderer.draw()` 耗时
- `encoder.finish()` + `queue.submit()` 耗时
- 识别是否有任何段在大 MIDI 时明显变慢

## B — audio mixer 负载与线程调度
大 MIDI 有 4800 channels。audio callback 是否超 budget？

- 在 audio engine 的 render callback 入口/出口加 timestamp trace
- 监控 `SUBMITTED / DROPPED / buffered` heartbeat 在大/小 MIDI 加载前后的跳变
- 确认 audio 线程是否阻塞主线 repaint 调度

## C — display 刷新率确认
```bash
system_profiler SPDisplaysDataType
```
- 确认显示输出是 60Hz。51fps 可能是 60Hz vsync 不稳定，也可能是 120Hz ProMotion cap。

## D — 同一 MIDI 在不同负载下的 fps 对比
- 加载小 MIDI，播放 + 不动 → real_fps=61
- 加载大 MIDI，播放 + 不动 → real_fps=51
- 但从大 MIDI 切回小 MIDI（不重启程序），小 MIDI 是 61 还是 51？
  - 如果切回小 MIDI 恢复 61 → 瓶颈与 MIDI 体量正相关（GPU upload/buffer 大小 vs vsync? inst_max=89 vs 89 一致所以不是 instance count 问题）
  - 如果切回小 MIDI 仍是 51 → 程序整体变慢了（memory allocator / wgpu resource leak / 某个后台线程卡了）

## E — 不同 MIDI 体量下 real_fps 的 GPU 差异分析
- 大/小 MIDI 播放时不动的 inst_max 都是 ~89-548（visible 范围内的都很少）
- 但大 MIDI kb cache 占用、tick_length 更大。扫描/visible_tick_range 查找不同
- 加 `tick_at_time` 函数在 4400 万音符上的查找性能 profile