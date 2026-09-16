use std::cmp::Reverse;

use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::SynthEvent;
use yinhe_mixer::PluginEvent;

use crate::audio_model::ActiveNote;
use crate::engine::AudioEngine;

/// Number of output channels (stereo).
const STEREO_CHANNELS: usize = 2;

impl AudioEngine {
    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 || !self.playing {
            output.fill(0.0);
            return;
        }

        // GPU 路径：GpuSynth 渲染到混音台 planar 通道缓冲，之后与 CPU 路径
        // 共用插件乐器 / 音频轨 / mixer.process（insert 效果器、总线、推子全部生效）。
        // 块长变化（导出用 1024、实时 512）：与 CPU 路径同一逻辑。
        #[cfg(feature = "gpu")]
        if self.gpu_synth.is_some() {
            if self.mixer.frames() != frames {
                let strips = self.dense_strip_params();
                let count = self.mixer.channel_count();
                self.mixer.resize(count, frames, &strips);
                self.channel_set.resize_scratches(frames);
            }
            let block_start_sample = self.sample_position;
            let block_end_sample = block_start_sample + frames as u64;
            let block_end_tick = self.sample_to_tick(block_end_sample);
            self.block_start_sample = block_start_sample;

            // GPU 渲染（覆盖写 dense 0..MAX_CHANNELS，其余清零）。
            if let Some(synth) = self.gpu_synth.as_mut() {
                synth.render_to_mixer(self.mixer.buffers_mut());
            }
            // 插件乐器事件：按块推进 tick（GPU 模式下非插件通道不喂 xsynth）。
            self.dispatch_block_events(block_end_tick);
            self.dispatch_plugin_previews(block_start_sample, frames);
            self.render_instruments(block_start_sample, frames);
            self.render_audio_tracks(block_start_sample, frames);

            let (master_l, master_r) = self.mixer.process();
            for (i, chunk) in output.chunks_exact_mut(STEREO_CHANNELS).enumerate() {
                chunk[0] = master_l[i];
                chunk[1] = master_r[i];
            }
            self.sample_position = block_end_sample;
            self.current_tick = block_end_tick;
            return;
        }

        // 块长变化（导出用 1024、实时 512）：mixer 缓冲与通道暂存按实际块长
        // 重建一次。引擎生命周期内块长固定，之后不再进入此分支。
        if self.mixer.frames() != frames {
            let strips = self.dense_strip_params();
            let count = self.mixer.channel_count();
            self.mixer.resize(count, frames, &strips);
            self.channel_set.resize_scratches(frames);
        }

        // CPU 路径：xsynth 逐段分发+渲染。事件比较全在 tick 域
        // （dispatch 基准 = current_tick，块边界 = sample→tick 反查），
        // 只有"渲染段边界"才转一次 sample（每块事件数量级）。
        // 与旧路径的差异：各通道渲染进混音台的 planar 通道缓冲（而非直接
        // 混成立体声），块末由 mixer 统一做增益/声像/mute/solo/insert。
        let block_start_sample = self.sample_position;
        let block_end_sample = block_start_sample + frames as u64;
        let block_end_tick = self.sample_to_tick(block_end_sample);
        self.block_start_sample = block_start_sample;
        let mut rendered_until_sample = block_start_sample;
        let mut rendered_until_tick = self.current_tick;
        let mut offset_frames = 0usize;

        while rendered_until_tick < block_end_tick {
            // 单次 dispatch + find_next：候选是下一个未处理事件的 tick
            //（严格 > rendered_until_tick，循环必然推进）。
            let next_tick = self
                .dispatch_and_find_next(rendered_until_tick, block_end_tick)
                .unwrap_or(block_end_tick)
                .min(block_end_tick);
            // 块末边界直接对齐 block_end_sample；否则 tick→sample 得段边界。
            // 极快 tempo 下多个 tick 可能映射同一 sample（零长段）：
            // 不渲染、只推进 tick 继续 dispatch，事件不丢不重。
            let next_sample = if next_tick >= block_end_tick {
                block_end_sample
            } else {
                self.tick_to_sample(next_tick)
            };
            let segment_frames = (next_sample - rendered_until_sample) as usize;
            if segment_frames > 0 {
                self.channel_set.render_segment(
                    self.mixer.buffers_mut(),
                    offset_frames,
                    segment_frames,
                );
                rendered_until_sample = next_sample;
                offset_frames += segment_frames;
            }
            rendered_until_tick = next_tick;
        }

        // 补齐剩余帧（浮点/块对齐：tick_to_sample(block_end_tick) 可能略小于
        // block_end_sample，剩余段无事件）。
        let remaining = block_end_sample - rendered_until_sample;
        if remaining > 0 {
            self.channel_set.render_segment(
                self.mixer.buffers_mut(),
                offset_frames,
                remaining as usize,
            );
        }

        // 乐器插件：把每块累积的事件喂给各自实例，输出写进对应乐器 dense 通道。
        self.dispatch_plugin_previews(block_start_sample, frames);
        self.render_instruments(block_start_sample, frames);

        // 音频片段回放：按绝对时间直接混进对应音频 dense 通道（无状态）。
        self.render_audio_tracks(block_start_sample, frames);

        // 混音：insert → 增益/声像斜坡 → mute/solo → master，然后交错输出。
        let (master_l, master_r) = self.mixer.process();
        for (i, chunk) in output.chunks_exact_mut(STEREO_CHANNELS).enumerate() {
            chunk[0] = master_l[i];
            chunk[1] = master_r[i];
        }

        self.sample_position = block_end_sample;
        self.current_tick = block_end_tick;
    }

    /// GPU 合成器是否启用（无 `gpu` feature 时恒 false）。
    /// dispatch 用它决定是否把事件喂给 xsynth（GPU 自管事件列表）。
    #[inline]
    fn gpu_synth_active(&self) -> bool {
        #[cfg(feature = "gpu")]
        {
            self.gpu_synth.is_some()
        }
        #[cfg(not(feature = "gpu"))]
        {
            false
        }
    }

    /// GPU 路径：把一整块的事件派发到插件乐器并推进 `current_tick`。
    /// CPU 路径在分段渲染循环里逐段 dispatch；GPU 由合成器自管音符/CC，
    /// 这里只需把插件事件推完（`dispatch_and_find_next` 内部在 GPU 模式下
    /// 跳过 xsynth 发送）。
    #[cfg(feature = "gpu")]
    fn dispatch_block_events(&mut self, block_end_tick: u32) {
        let mut t = self.current_tick;
        while t < block_end_tick {
            t = self
                .dispatch_and_find_next(t, block_end_tick)
                .unwrap_or(block_end_tick)
                .min(block_end_tick);
        }
    }

    /// 合并了原来 `next_event_sample`、`dispatch_cc_until`、`dispatch_notes_at`
    /// 三个函数的职责，KEY_COUNT 桶只扫描一次。所有比较都在 tick 域，无需转换。
    ///
    pub(crate) fn dispatch_and_find_next(&mut self, tick: u32, block_end_tick: u32) -> Option<u32> {
        let mut next: Option<u32> = None;

        // ── CC 事件 ──
        while self.cc_cursor < self.cc_events.len() && self.cc_events[self.cc_cursor].tick <= tick {
            let cc = &self.cc_events[self.cc_cursor];
            // mute 的音轨跳过其自动化事件（CC/PB/RPN/NRPN/PC）；
            // AM M/S 动态掩码再跳过被旁通的 lane（PC 事件 lane==哨兵，天然不跳过）。
            let track_skipped = self
                .skip_track
                .get(cc.track as usize)
                .copied()
                .unwrap_or(false);
            let lane_skipped = self
                .am_lane_skip
                .get(cc.track as usize)
                .and_then(|v| v.get(cc.lane as usize))
                .copied()
                .unwrap_or(false);
            if !track_skipped && !lane_skipped {
                if let Some(pp) = cc.plugin_param {
                    // 插件参数自动化 → 该 MIDI 通道插件实例的 ParamValue。
                    if let Some(dense) = self.channel_plugin_dense(pp.channel) {
                        let time = self
                            .tick_to_sample(cc.tick)
                            .saturating_sub(self.block_start_sample)
                            as u32;
                        if let Some(Some(slot)) = self.instruments.get_mut(dense) {
                            slot.events.push(PluginEvent::ParamValue {
                                time,
                                param_id: pp.param_id,
                                value: f64::from(pp.value),
                            });
                        }
                    }
                } else {
                    // CC 广播：先广播给该通道 insert 链上订阅的效果器（内置
                    // DSP 模块经 `handled_ccs` 接管），再走常规路径（挂插件则
                    // 转 MIDI 喂插件，否则进合成器）。两路都发：谁订阅谁消费；
                    // 同时挂插件与效果器时的双重处理由用户挂载选择决定。
                    if let Some((cc_num, cc_value)) = raw_cc(&cc.event) {
                        let dense = self.channel_layout.dense_for(cc.channel as usize);
                        if dense != u32::MAX {
                            self.mixer
                                .broadcast_channel_cc(dense as usize, cc_num, cc_value);
                            // 实际发送 → 打点（chase 应用时跳过，避免旧值覆盖新值）。
                            self.dispatched_skip.mark(&cc.event, cc.channel as usize);
                        }
                    }
                    if let Some(dense) = self.channel_plugin_dense(cc.channel as u8) {
                        // 该 MIDI 通道挂了插件 → CC/PB/RPN/PC 转原始 MIDI 字节喂实例；
                        // 否则走 xsynth。
                        // 先算 frame offset（只读），再取可变实例引用，避免整机借用冲突。
                        let time = self
                            .tick_to_sample(cc.tick)
                            .saturating_sub(self.block_start_sample)
                            as u32;
                        if let Some(data) = cc_to_midi(&cc.event, cc.channel as u8)
                            && let Some(Some(slot)) = self.instruments.get_mut(dense)
                        {
                            slot.events.push(PluginEvent::Midi { time, data });
                            self.dispatched_skip.mark(&cc.event, cc.channel as usize);
                        }
                    } else {
                        let dense = self.channel_layout.dense_for(cc.channel as usize);
                        if dense != u32::MAX {
                            // GPU 合成器路径：事件由 GpuSynth 自己的事件列表管理，
                            // 不喂 xsynth（避免缓存无界增长）。
                            if !self.gpu_synth_active() {
                                self.channel_set.send_event(SynthEvent::Channel(
                                    dense,
                                    ChannelEvent::Audio(cc.event),
                                ));
                            }
                            self.dispatched_skip.mark(&cc.event, cc.channel as usize);
                        }
                    }
                }
            }
            self.cc_cursor += 1;
        }
        if self.cc_cursor < self.cc_events.len() {
            let cc_tick = self.cc_events[self.cc_cursor].tick;
            if cc_tick < block_end_tick {
                next = Some(next.map_or(cc_tick, |t| t.min(cc_tick)));
            }
        }

        // ── NoteOn + 找下一个 NoteOn 边界（单次 KEY_COUNT 桶扫描）──
        // audible_notes 桶内 start_tick 升序（模型桶有序，无需 sort），
        // 桶里只有 vel>1 的音符，无需运行时过滤。
        for key in 0..yinhe_types::KEY_COUNT {
            let notes = self.audible_notes[key].as_slice();
            let mut cursor = self.note_cursor[key];

            while cursor < notes.len() {
                let note = &notes[cursor];
                if note.start_tick > tick {
                    // 该桶下一个待处理音符 → 记录为边界候选
                    if note.start_tick < block_end_tick {
                        next = Some(next.map_or(note.start_tick, |t| t.min(note.start_tick)));
                    }
                    break;
                }
                // start_tick ≤ tick → dispatch NoteOn
                let track = note.track as usize;
                let ch = self
                    .model
                    .as_ref()
                    .map(|m| m.track_channel(track))
                    .unwrap_or(0);
                if !self.skip_track.get(track).copied().unwrap_or(false) {
                    let dense = self.channel_layout.dense_for(ch as usize);
                    if dense != u32::MAX {
                        let has_plugin = self
                            .instruments
                            .get(dense as usize)
                            .is_some_and(|s| s.is_some());
                        if has_plugin {
                            // 该通道挂了插件乐器：音符喂实例（插件通道 = MIDI 通道低 4 位）。
                            // 先算 frame offset 与 CLAP 通道（只读），再取可变实例引用。
                            let time = self
                                .tick_to_sample(note.start_tick)
                                .saturating_sub(self.block_start_sample)
                                as u32;
                            let clap_ch = ch & 0x0F;
                            if let Some(Some(slot)) = self.instruments.get_mut(dense as usize) {
                                slot.events.push(PluginEvent::NoteOn {
                                    time,
                                    channel: clap_ch,
                                    key: key as u8,
                                    velocity: note.velocity as f64 / 127.0,
                                });
                                self.active_notes.push(Reverse(ActiveNote {
                                    key: key as u8,
                                    dense,
                                    clap_channel: clap_ch,
                                    is_instrument: true,
                                    end_tick: note.end_tick,
                                    track: track as u16,
                                }));
                            }
                        } else if !self.gpu_synth_active() {
                            // GPU 路径：音符由 GpuSynth 事件列表处理（不喂 xsynth）。
                            self.channel_set.send_event(SynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                                    key: key as u8,
                                    vel: note.velocity,
                                }),
                            ));
                            self.active_notes.push(Reverse(ActiveNote {
                                key: key as u8,
                                dense,
                                clap_channel: 0,
                                is_instrument: false,
                                end_tick: note.end_tick,
                                track: track as u16,
                            }));
                        }
                    }
                }
                cursor += 1;
            }
            self.note_cursor[key] = cursor;
        }

        // ── NoteOff + 找下一个 NoteOff 边界（min-heap 逐个 pop）──
        // 堆顶 = end_tick 最小的活跃音符。
        // ended 个音符每个 O(log V) pop，未结束的堆顶 O(1) peek 得下一边界。
        // 之前是 Vec::retain 全扫 O(V_active)，高密度段 V 大时被多次调用形成 O(k×V) 正反馈。
        self.ended_notes.clear();
        while let Some(Reverse(an)) = self.active_notes.peek() {
            if an.end_tick > tick {
                break;
            }
            self.ended_notes.push(*an);
            self.active_notes.pop();
        }
        // peek 堆顶（最早结束的未结束音符）作为下一 NoteOff 边界候选
        if let Some(Reverse(an)) = self.active_notes.peek()
            && an.end_tick < block_end_tick
        {
            next = Some(next.map_or(an.end_tick, |t| t.min(an.end_tick)));
        }
        for an in &self.ended_notes {
            if an.is_instrument {
                // 乐器音符 NoteOff → 喂乐器实例（含挂音恢复）。
                let time = self
                    .tick_to_sample(an.end_tick)
                    .saturating_sub(self.block_start_sample) as u32;
                if let Some(Some(slot)) = self.instruments.get_mut(an.dense as usize) {
                    slot.events.push(PluginEvent::NoteOff {
                        time,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            } else {
                let dense = an.dense;
                if dense != u32::MAX && !self.gpu_synth_active() {
                    // GPU 路径：音符由 GpuSynth 事件列表处理（不喂 xsynth）。
                    self.channel_set.send_event(SynthEvent::Channel(
                        dense,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                    ));
                }
            }
        }

        next
    }

    /// MIDI 通道 `channel` 上已挂载插件乐器时的 dense 索引；
    /// 通道未激活或使用默认 XSynth（未挂插件）时返回 None。
    pub(crate) fn channel_plugin_dense(&self, channel: u8) -> Option<usize> {
        let dense = self.channel_layout.dense_for(channel as usize);
        if dense == u32::MAX {
            return None;
        }
        self.instruments
            .get(dense as usize)
            .is_some_and(|s| s.is_some())
            .then_some(dense as usize)
    }

    /// 是否存在已安装的乐器处理器（渲染器判据：停止状态也需要空闲渲染）。
    pub(crate) fn has_instruments(&self) -> bool {
        self.instruments.iter().any(|s| s.is_some())
    }

    /// 插件预览：登记音符，NoteOn/NoteOff 由 `dispatch_plugin_previews` 按块调度。
    /// 组替换语义与 xsynth 预览一致：清掉该通道旧的**待触发**音符；
    /// 已触发的保留（快速拖动时每个音都能听到）。组内按 target_tick 错开触发。
    pub(crate) fn preview_instrument_notes(
        &mut self,
        channel: u8,
        notes: Vec<crate::spawn::InstrumentPreviewNote>,
        exclusive: bool,
    ) {
        let Some(dense) = self.channel_plugin_dense(channel) else {
            return;
        };
        if exclusive {
            // 拖动替换：旧组该通道的全部预览音（含已响）停掉，只保留当前组。
            self.preview_instrument_stop(Some(channel), None);
        }
        self.plugin_previews
            .retain(|p| !(p.dense == dense && !p.triggered));
        if notes.is_empty() {
            return;
        }
        // 位置换算：模型未加载（无 tempo map）时全部立即触发。
        let has_tempo = self.yin_model.is_some();
        let min_tick = notes.iter().map(|n| n.target_tick).min().unwrap_or(0);
        let base_sample = self.tick_to_sample(min_tick);
        let base_on = self.sample_position;
        for n in notes {
            let on_sample = if has_tempo {
                base_on
                    + self
                        .tick_to_sample(n.target_tick)
                        .saturating_sub(base_sample)
            } else {
                base_on
            };
            let off_sample = if n.duration_ticks > 0 && has_tempo {
                let start = self.tick_to_sample(n.target_tick);
                let end = self.tick_to_sample(n.target_tick.saturating_add(n.duration_ticks));
                Some(on_sample + end.saturating_sub(start))
            } else {
                None
            };
            self.plugin_previews.push(crate::engine::PluginPreviewNote {
                dense,
                midi_channel: n.midi_channel,
                key: n.key,
                velocity: n.velocity,
                on_sample,
                off_sample,
                triggered: false,
            });
        }
    }

    /// 停止插件预览：清除匹配的待触发音符 + 对已触发音符发 NoteOff。
    /// `channel` 限定 MIDI 通道（None = 全部），`key` 限定单个键（None = 全部）。
    pub(crate) fn preview_instrument_stop(&mut self, channel: Option<u8>, key: Option<u8>) {
        if self.plugin_previews.is_empty() {
            return;
        }
        let target_dense = channel.and_then(|ch| self.channel_plugin_dense(ch));
        let mut i = 0;
        while i < self.plugin_previews.len() {
            let p = &self.plugin_previews[i];
            let matches = target_dense.map(|d| d == p.dense).unwrap_or(true)
                && key.map(|k| k == p.key).unwrap_or(true);
            if !matches {
                i += 1;
                continue;
            }
            let p = self.plugin_previews.swap_remove(i);
            if p.triggered
                && let Some(slot) = self.instruments.get_mut(p.dense).and_then(|s| s.as_mut())
            {
                slot.events.push(PluginEvent::NoteOff {
                    time: 0,
                    channel: p.midi_channel,
                    key: p.key,
                    velocity: 0.0,
                });
            }
        }
    }

    /// 每块调度插件预览的 NoteOn/NoteOff（在 `render_instruments` 之前调用）。
    fn dispatch_plugin_previews(&mut self, block_start: u64, frames: usize) {
        if self.plugin_previews.is_empty() {
            return;
        }
        let block_end = block_start + frames as u64;
        let mut i = 0;
        while i < self.plugin_previews.len() {
            let (dense, midi_channel, key, velocity, on_sample, off_sample, triggered) = {
                let p = &self.plugin_previews[i];
                (
                    p.dense,
                    p.midi_channel,
                    p.key,
                    p.velocity,
                    p.on_sample,
                    p.off_sample,
                    p.triggered,
                )
            };
            if !triggered {
                if on_sample > block_end {
                    i += 1;
                    continue;
                }
                let time = on_sample.saturating_sub(block_start).min(frames as u64) as u32;
                if let Some(slot) = self.instruments.get_mut(dense).and_then(|s| s.as_mut()) {
                    slot.events.push(PluginEvent::NoteOn {
                        time,
                        channel: midi_channel,
                        key,
                        velocity: f64::from(velocity.min(127)) / 127.0,
                    });
                }
                self.plugin_previews[i].triggered = true;
                i += 1;
                continue;
            }
            let Some(off) = off_sample else {
                i += 1;
                continue;
            };
            if off > block_end {
                i += 1;
                continue;
            }
            let time = off.saturating_sub(block_start).min(frames as u64) as u32;
            if let Some(slot) = self.instruments.get_mut(dense).and_then(|s| s.as_mut()) {
                slot.events.push(PluginEvent::NoteOff {
                    time,
                    channel: midi_channel,
                    key,
                    velocity: 0.0,
                });
            }
            self.plugin_previews.swap_remove(i);
        }
    }

    /// 空闲渲染（停止/暂停）：不推进走带、不派发音符，只驱动乐器插件
    ///（GUI 键盘、插件预览、插件尾音）与混音输出。
    /// 存在已安装乐器时由渲染器持续调用（成熟 DAW 语义：乐器插件始终在跑）。
    pub(crate) fn render_idle(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 {
            output.fill(0.0);
            return;
        }
        // 块长变化（导出用 1024、实时 512）：与 render 同一逻辑。
        if self.mixer.frames() != frames {
            let strips = self.dense_strip_params();
            let count = self.mixer.channel_count();
            self.mixer.resize(count, frames, &strips);
            self.channel_set.resize_scratches(frames);
        }
        let block_start = self.sample_position;
        self.block_start_sample = block_start;
        self.mixer.clear_channel_buffers();
        self.dispatch_plugin_previews(block_start, frames);
        self.render_instruments(block_start, frames);
        let (master_l, master_r) = self.mixer.process();
        for (i, chunk) in output.chunks_exact_mut(STEREO_CHANNELS).enumerate() {
            chunk[0] = master_l[i];
            chunk[1] = master_r[i];
        }
    }

    /// 暂停/停止时把待发插件参数送达（静音块）。renderer 每轮轮询调用；
    /// 无参数变化时开销可忽略（每个处理器一次空队列检查）。
    pub(crate) fn flush_pending_plugin_params(&mut self) {
        let position = self.sample_position;
        for slot in self.instruments.iter_mut().flatten() {
            slot.processor.flush_pending_params(position);
        }
        self.mixer.flush_pending_insert_params(position);
    }

    /// 把本块累积的乐器事件喂给各乐器实例，输出写进对应乐器 dense 通道。
    /// 在 xsynth 段渲染之后、mixer.process() 之前调用。
    fn render_instruments(&mut self, block_start_sample: u64, frames: usize) {
        let n = self.instruments.len();
        for dense in 0..n {
            let Some(slot) = &mut self.instruments[dense] else {
                continue;
            };
            // take 出的 Vec 处理完归还，保留容量（避免每块重新分配）。
            let mut events = std::mem::take(&mut slot.events);
            if let Some(cb) = self.mixer.channel_buffers_mut(dense) {
                let f = frames.min(cb.left.len()).min(cb.right.len());
                slot.processor.process(
                    &events,
                    &mut cb.left[..f],
                    &mut cb.right[..f],
                    block_start_sample,
                );
            }
            events.clear();
            slot.events = events;
        }
    }

    /// 音频轨片段回放：按块内**绝对时间**把每条音频轨的片段混进对应音频
    /// dense 通道缓冲。完全无状态（seek/暂停/导出天然正确），PDC 由混音台
    /// 统一补偿。音频通道是覆盖写（没有 xsynth/插件替它清零）。
    fn render_audio_tracks(&mut self, block_start_sample: u64, frames: usize) {
        if self.channel_layout.audio_channels().is_empty() {
            return;
        }
        let Some(model) = self.yin_model.clone() else {
            return;
        };
        for &ach in self.channel_layout.audio_channels() {
            let dense = self.channel_layout.audio_dense_for(ach) as usize;
            if let Some(cb) = self.mixer.channel_buffers_mut(dense) {
                let n = frames.min(cb.left.len()).min(cb.right.len());
                cb.left[..n].fill(0.0);
                cb.right[..n].fill(0.0);
            }
        }
        let sr = self.sample_rate as f64;
        let block_start = block_start_sample as i64;
        let block_end = block_start + frames as i64;
        for (track_idx, track) in model.tracks.iter().enumerate() {
            if track.kind != yinhe_core::TrackKind::Audio || track.audio_clips.is_empty() {
                continue;
            }
            if self.skip_track.get(track_idx).copied().unwrap_or(false) {
                continue;
            }
            let Some(ach) = track.audio_channel else {
                continue;
            };
            let dense = self.channel_layout.audio_dense_for(ach);
            if dense == u32::MAX {
                continue;
            }
            let dense = dense as usize;
            for (ci, clip) in track.audio_clips.iter().enumerate() {
                let Some(pcm) = self.audio_sources.get(&clip.source) else {
                    continue;
                };
                if pcm.frames == 0 || clip.duration_seconds <= 0.0 {
                    continue;
                }
                let clip_start = (clip.start_seconds * sr).round() as i64;
                let clip_end = (clip.end_seconds() * sr).round() as i64;
                let from = clip_start.max(block_start);
                let to = clip_end.min(block_end);
                if to <= from {
                    continue;
                }
                let (fade_in, fade_out) =
                    crate::audio_model::effective_fades(&track.audio_clips, ci);
                let Some(cb) = self.mixer.channel_buffers_mut(dense) else {
                    continue;
                };
                for g in from..to {
                    let n = (g - block_start) as usize;
                    if n >= frames {
                        break;
                    }
                    // 片段内已播时长（秒）；淡入淡出与素材偏移都以它为准。
                    let p = g as f64 / sr - clip.start_seconds;
                    let mut gain = clip.gain;
                    if fade_in > 0.0 && p < fade_in {
                        gain *= (p / fade_in).clamp(0.0, 1.0) as f32;
                    }
                    let remain = clip.duration_seconds - p;
                    if fade_out > 0.0 && remain < fade_out {
                        gain *= (remain / fade_out).clamp(0.0, 1.0) as f32;
                    }
                    let src_sec = if clip.reversed {
                        clip.offset_seconds + (clip.duration_seconds - p)
                    } else {
                        clip.offset_seconds + p
                    };
                    let src_frame = (src_sec * sr).floor();
                    if src_frame < 0.0 {
                        continue;
                    }
                    let si = src_frame as usize;
                    if si >= pcm.frames {
                        continue;
                    }
                    cb.left[n] += pcm.left[si] * gain;
                    cb.right[n] += pcm.right[si] * gain;
                }
            }
        }
    }
}

/// 提取原始 CC（号 + 值）；非 CC 事件返回 None。
pub(crate) fn raw_cc(event: &ChannelAudioEvent) -> Option<(u8, u8)> {
    match event {
        ChannelAudioEvent::Control(ControlEvent::Raw(cc, value)) => Some((*cc, *value)),
        _ => None,
    }
}

/// 把 xsynth 风格的通道事件转成原始 MIDI 报文（CC/弯音/ProgramChange），
/// status 字节带上音轨的 MIDI 通道（0..15）。仅用于乐器轨的自动化路由。
fn cc_to_midi(event: &ChannelAudioEvent, channel: u8) -> Option<[u8; 3]> {
    let ch = 0x0F & channel;
    match event {
        ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)) => Some([0xB0 | ch, *cc, *val]),
        ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => {
            // v ∈ [-1, 1] → 14 bit 弯音值
            let raw = ((v.clamp(-1.0, 1.0) + 1.0) * 8191.5) as u32;
            Some([0xE0 | ch, (raw & 0x7F) as u8, ((raw >> 7) & 0x7F) as u8])
        }
        ChannelAudioEvent::ProgramChange(p) => Some([0xC0 | ch, *p, 0]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xsynth_core::channel::ControlEvent;

    #[test]
    fn cc_to_midi_status_uses_channel_nibble() {
        let msg = cc_to_midi(&ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)), 0x0A).unwrap();
        // 0xB0 | 通道低 4 位（0x0A）= 0xBA
        assert_eq!(msg, [0xBA, 7, 100]);
    }

    #[test]
    fn cc_to_midi_pitchbend_14bit() {
        let msg = cc_to_midi(
            &ChannelAudioEvent::Control(ControlEvent::PitchBendValue(0.0)),
            0,
        )
        .unwrap();
        assert_eq!(msg[0] & 0xF0, 0xE0);
        let raw = ((msg[2] as u32) << 7) | (msg[1] as u32);
        assert!(
            (8190..=8192).contains(&raw),
            "中部弯音值应在中点附近, got {raw}"
        );
    }

    #[test]
    fn cc_to_midi_program_change() {
        let msg = cc_to_midi(&ChannelAudioEvent::ProgramChange(42), 3).unwrap();
        assert_eq!(msg, [0xC3, 42, 0]);
    }

    #[test]
    fn cc_to_midi_unhandled_returns_none() {
        // NoteOn 类事件不是控制器/ProgramChange，不应转 MIDI 报文。
        let r = cc_to_midi(&ChannelAudioEvent::NoteOn { key: 60, vel: 100 }, 0);
        assert!(r.is_none());
    }
}

/// GPU 合成器接入混音台的冒烟测试（需 `YINHE_TEST_SFZ` 指向 SFZ 文件）。
#[cfg(all(test, feature = "gpu"))]
mod gpu_tests {
    use std::sync::Arc;

    use yinhe_core::{ConductorData, NoteEvent, ProjectMeta, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    use crate::channel_layout::ChannelLayout;
    use crate::engine::AudioEngine;
    use crate::spawn::AudioCommand;

    /// 1 拍、单个音符的模型（120 BPM / PPQ 480）。
    fn tiny_model() -> Arc<YinModel> {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(vec![vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        }]]);
        model.rebuild();
        Arc::new(model)
    }

    /// GPU 路径经混音台渲染：不卡死、输出有限且有声音。
    #[test]
    fn gpu_engine_render_smoke() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let model = tiny_model();
        let layout = ChannelLayout::from_model(&model);
        let mut engine = AudioEngine::new(48_000, layout);
        engine.handle_command(AudioCommand::LoadModel { model });

        let mut synth = yinhe_synth::GpuSynth::new_default(48_000).expect("GpuSynth init");
        synth
            .load_dense_soundfonts(0, &[std::path::PathBuf::from(&sfz)])
            .expect("soundfont load");
        synth.finish_soundfont_load();
        // 与真实路径一致：加载事件列表（这里手动构造一个 0..100ms 的音符）。
        synth.load_events(vec![
            yinhe_synth::SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 100,
            },
            yinhe_synth::SynthEvent::NoteOff {
                sample: 4800,
                channel: 0,
                key: 60,
            },
        ]);
        engine.gpu_synth = Some(synth);

        engine.handle_command(AudioCommand::Play { from_sample: 0 });
        let mut out = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for i in 0..200 {
            engine.render(&mut out);
            assert!(out.iter().all(|v| v.is_finite()), "block {i} 输出异常");
            peak = out.iter().fold(peak, |m, v| m.max(v.abs()));
        }
        assert!(peak > 0.0, "GPU 渲染无输出（peak=0）");
    }
}
