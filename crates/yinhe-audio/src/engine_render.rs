use std::cmp::Reverse;

use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;

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
                let dense = self.channel_layout.dense_for(cc.channel as usize);
                if dense != u32::MAX {
                    self.channel_set
                        .send_event(SynthEvent::Channel(dense, ChannelEvent::Audio(cc.event)));
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
                            channel: ch as u8,
                            end_tick: note.end_tick,
                        }));
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
            let dense = self.channel_layout.dense_for(an.channel as usize);
            if dense != u32::MAX {
                self.channel_set.send_event(SynthEvent::Channel(
                    dense,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: an.key }),
                ));
            }
        }

        next
    }
}
