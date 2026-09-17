//! 插件集成：乐器实例的渲染、预览调度、空闲驱动与参数 flush。

use yinhe_mixer::PluginEvent;

use crate::engine::AudioEngine;

impl AudioEngine {
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
    pub(super) fn dispatch_plugin_previews(&mut self, block_start: u64, frames: usize) {
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

    /// 暂停/停止时把待发插件参数送达（静音块）。renderer 每轮轮询调用；
    /// 无参数变化时开销可忽略（每个处理器一次空队列检查）。
    pub(crate) fn flush_pending_plugin_params(&mut self) {
        let position = self.sample_position;
        for slot in self.instruments.iter_mut().flatten() {
            slot.processor.flush_pending_params(position);
        }
        self.mixer.flush_pending_insert_params(position);
        for chain in &mut self.channel_dsp {
            chain.flush_pending_params(position);
        }
    }

    /// 把本块累积的乐器事件喂给各乐器实例，输出写进对应乐器 dense 通道。
    /// 在 xsynth 段渲染之后、mixer.process() 之前调用。
    pub(super) fn render_instruments(&mut self, block_start_sample: u64, frames: usize) {
        let n = self.instruments.len();
        for dense in 0..n {
            let Some(slot) = &mut self.instruments[dense] else {
                continue;
            };
            // take 出的 Vec 处理完归还，保留容量（避免每块重新分配）。
            let mut events = std::mem::take(&mut slot.events);
            if let Some(cb) = self.mixer.channel_buffers_mut(dense) {
                let f = frames.min(cb.left.len()).min(cb.right.len());
                // 乐器不做隐式分段（携带事件与时间戳）：引擎块长必须 ≤ 插件
                // 激活能力（见 MAX_ENGINE_BLOCK_FRAMES 契约），越界是宿主 bug。
                debug_assert!(
                    f <= slot.processor.max_block_frames(),
                    "乐器块长 {f} 超过插件能力 {}",
                    slot.processor.max_block_frames()
                );
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
}
