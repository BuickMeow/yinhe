use std::cmp::Reverse;

use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::SynthEvent;
use yinhe_clap::ClapInputEvent;

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

        // GPU 路径：GpuSynth 管理自己的事件列表和 voice 状态
        //（暂不经过混音台：GPU 合成器内部直接混成立体声）
        #[cfg(feature = "gpu")]
        if let Some(ref mut synth) = self.gpu_synth {
            synth.render(output);
            self.sample_position = synth.sample_position();
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

        // CLAP 乐器：把每块累积的事件喂给各自实例，输出混入对应乐器 dense 通道。
        self.render_instruments(block_start_sample, frames);

        // 混音：insert → 增益/声像斜坡 → mute/solo → master，然后交错输出。
        let (master_l, master_r) = self.mixer.process();
        for (i, chunk) in output.chunks_exact_mut(STEREO_CHANNELS).enumerate() {
            chunk[0] = master_l[i];
            chunk[1] = master_r[i];
        }

        self.sample_position = block_end_sample;
        self.current_tick = block_end_tick;
    }

    /// 合并了原来 `next_event_sample`、`dispatch_cc_until`、`dispatch_notes_at`
    /// 三个函数的职责，KEY_COUNT 桶只扫描一次。所有比较都在 tick 域，无需转换。
    ///
    pub(crate) fn dispatch_and_find_next(&mut self, tick: u32, block_end_tick: u32) -> Option<u32> {
        let mut next: Option<u32> = None;

        // ── CC 事件 ──
        while self.cc_cursor < self.cc_events.len() && self.cc_events[self.cc_cursor].tick <= tick {
            let cc = &self.cc_events[self.cc_cursor];
            // mute 的音轨：跳过其自动化事件（CC/PB/RPN/NRPN/PC），
            // 使同 channel 上其他非 mute 轨道不受影响。
            if !self
                .skip_track
                .get(cc.track as usize)
                .copied()
                .unwrap_or(false)
            {
                // 乐器轨的自动化 → 喂对应乐器实例；否则走 xsynth。
                if let Some(inst_ch) = self
                    .model
                    .as_ref()
                    .and_then(|m| m.track_instrument(cc.track as usize))
                {
                    if let Some(dense) = self.instrument_dense(inst_ch) {
                        // 先算 frame offset（只读），再取可变实例引用，避免整机借用冲突。
                        let time = self
                            .tick_to_sample(cc.tick)
                            .saturating_sub(self.block_start_sample)
                            as u32;
                        if let Some(data) = cc_to_clap_midi(&cc.event, cc.channel as u8)
                            && let Some(Some(slot)) = self.instruments.get_mut(dense)
                        {
                            slot.events.push(ClapInputEvent::Midi { time, data });
                            // 实际发送 → 打点（chase 应用时跳过，避免旧值覆盖新值）。
                            self.dispatched_skip.mark(&cc.event, cc.channel as usize);
                        }
                    }
                } else {
                    let dense = self.channel_layout.dense_for(cc.channel as usize);
                    if dense != u32::MAX {
                        self.channel_set
                            .send_event(SynthEvent::Channel(dense, ChannelEvent::Audio(cc.event)));
                        // 实际发送 → 打点（chase 应用时跳过，避免旧值覆盖新值）。
                        self.dispatched_skip.mark(&cc.event, cc.channel as usize);
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
                    .map(|m| m.track_channel(track) as usize)
                    .unwrap_or(0);
                if !self.skip_track.get(track).copied().unwrap_or(false) {
                    if let Some(inst_ch) =
                        self.model.as_ref().and_then(|m| m.track_instrument(track))
                    {
                        // 乐器轨：音符喂 CLAP 乐器实例（CLAP 通道 = 音轨 MIDI 通道低 4 位）。
                        if let Some(dense) = self.instrument_dense(inst_ch) {
                            // 先算 frame offset 与 CLAP 通道（只读），再取可变实例引用。
                            let time = self
                                .tick_to_sample(note.start_tick)
                                .saturating_sub(self.block_start_sample)
                                as u32;
                            let clap_ch = (ch & 0x0F) as u8;
                            if let Some(Some(slot)) = self.instruments.get_mut(dense) {
                                slot.events.push(ClapInputEvent::NoteOn {
                                    time,
                                    channel: clap_ch,
                                    key: key as u8,
                                    velocity: note.velocity as f64 / 127.0,
                                });
                                self.active_notes.push(Reverse(ActiveNote {
                                    key: key as u8,
                                    dense: dense as u32,
                                    clap_channel: clap_ch,
                                    is_instrument: true,
                                    end_tick: note.end_tick,
                                }));
                            }
                        }
                    } else {
                        let dense = self.channel_layout.dense_for(ch);
                        if dense != u32::MAX {
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
                    slot.events.push(ClapInputEvent::NoteOff {
                        time,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            } else {
                let dense = an.dense;
                if dense != u32::MAX {
                    self.channel_set.send_event(SynthEvent::Channel(
                        dense,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                    ));
                }
            }
        }

        next
    }

    /// 乐器通道 → 乐器 dense 索引（无对应乐器轨 = 未激活，返回 None）。
    pub(crate) fn instrument_dense(&self, inst_ch: u16) -> Option<usize> {
        let dense = self.channel_layout.instrument_dense_for(inst_ch);
        (dense != u32::MAX).then_some(dense as usize)
    }

    /// 把本块累积的乐器事件喂给各 CLAP 实例，输出混入对应乐器 dense 通道。
    /// 在 xsynth 段渲染之后、mixer.process() 之前调用。
    fn render_instruments(&mut self, block_start_sample: u64, frames: usize) {
        let n = self.instruments.len();
        for dense in 0..n {
            let Some(slot) = &mut self.instruments[dense] else {
                continue;
            };
            let events = std::mem::take(&mut slot.events);
            let (l, r) = match slot
                .processor
                .process_instrument(&events, Some(block_start_sample))
            {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(target: "clap-instrument", "乐器处理失败，本块静音: {e}");
                    continue;
                }
            };
            let f = frames.min(l.len()).min(r.len());
            if f == 0 {
                continue;
            }
            if let Some(cb) = self.mixer.channel_buffers_mut(dense) {
                for i in 0..f {
                    cb.left[i] += l[i];
                    cb.right[i] += r[i];
                }
            }
        }
    }
}

/// 把 xsynth 风格的通道事件转成 CLAP 原始 MIDI 报文（CC/弯音/ProgramChange），
/// status 字节带上音轨的 MIDI 通道（0..15）。仅用于乐器轨的自动化路由。
fn cc_to_clap_midi(event: &ChannelAudioEvent, channel: u8) -> Option<[u8; 3]> {
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
        let msg =
            cc_to_clap_midi(&ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)), 0x0A).unwrap();
        // 0xB0 | 通道低 4 位（0x0A）= 0xBA
        assert_eq!(msg, [0xBA, 7, 100]);
    }

    #[test]
    fn cc_to_midi_pitchbend_14bit() {
        let msg = cc_to_clap_midi(
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
        let msg = cc_to_clap_midi(&ChannelAudioEvent::ProgramChange(42), 3).unwrap();
        assert_eq!(msg, [0xC3, 42, 0]);
    }

    #[test]
    fn cc_to_midi_unhandled_returns_none() {
        // NoteOn 类事件不是控制器/ProgramChange，不应转 MIDI 报文。
        let r = cc_to_clap_midi(&ChannelAudioEvent::NoteOn { key: 60, vel: 100 }, 0);
        assert!(r.is_none());
    }
}
