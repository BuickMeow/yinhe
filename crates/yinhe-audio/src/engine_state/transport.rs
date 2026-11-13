//! 传输与位置：暂停/恢复/seek、轨道掩码切换。

use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use yinhe_mixer::PluginEvent;
use yinhe_types::KEY_COUNT;

use crate::channel::ChaseSkip;
use crate::engine::AudioEngine;

impl AudioEngine {
    /// 暂停：传输停止 + 停掉乐器插件的挂音。
    ///
    /// 插件在停止状态仍被空闲渲染持续 process（GUI 键盘/尾音语义），
    /// 不杀挂音就会在暂停时一直响（连续音色如三角波尤其明显）。
    /// tick 未推进 → 音符未结束，恢复时按跨点音符重启。
    pub(crate) fn pause(&mut self) {
        self.playing = false;
        // 丢掉尚未触发的插件 NoteOn，避免暂停/停止后它们又冒出来；
        // 已经发声的音符在下面补 NoteOff，让 release 尾音正常收尾。
        for inst in self.instruments.iter_mut().flatten() {
            inst.events.clear();
        }
        let all = std::mem::take(&mut self.active_notes);
        let mut remaining = std::collections::BinaryHeap::new();
        for std::cmp::Reverse(an) in all.into_iter() {
            if an.is_instrument {
                if let Some(Some(slot)) = self.instruments.get_mut(an.dense as usize) {
                    slot.events.push(PluginEvent::NoteOff {
                        time: 0,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            } else {
                remaining.push(std::cmp::Reverse(an));
            }
        }
        self.active_notes = remaining;
        // 试听音同样停掉（暂停不该有预览在响）。
        self.preview_instrument_stop(None, None);
    }

    /// 停止（Stop）：先按暂停语义释放挂音，再回到 0；**不重置 insert /
    /// 通道 DSP / 乐器处理器**，让 delay/reverb 尾音继续自然衰减。
    pub(crate) fn stop_transport(&mut self) {
        self.pause();
        self.relocate(0, RelocateMode::Stop);
    }

    /// 恢复（Pause 后）：重启 pause 期间被杀掉的**乐器**跨点音符。
    /// xsynth 音符不受影响（暂停不杀 xsynth voice，恢复自然续响）。
    pub(crate) fn resume(&mut self) {
        self.playing = true;
        self.restart_crossing_instrument_notes();
    }

    /// 应用新的轨道 skip 掩码，按 diff 即时处理：新 mute 的轨立即停音，
    /// 新 unmute 的轨立即重启跨点音符。`self.skip_track` 同步更新为 `new`。
    pub(crate) fn apply_skip_mask(&mut self, old: &[bool], new: &[bool]) {
        self.skip_track = new.to_vec();
        let n = old.len().max(new.len());
        for i in 0..n {
            let was = old.get(i).copied().unwrap_or(false);
            let is = new.get(i).copied().unwrap_or(false);
            if was == is {
                continue;
            }
            if is {
                // 新 mute：立即停掉该轨在响音符。
                self.kill_track_notes(i as u16);
            } else {
                // 新 unmute：立即重启该轨跨点音符。
                self.restart_track_crossing_notes(i as u16);
            }
        }
    }

    /// 设置走带循环。区间无效（`end <= start`）时等价于关闭。
    ///
    /// 播放中且播放头落在区间外 → 立即 seek 到起点（只发生一次，不是每轮回绕）。
    pub(crate) fn set_loop(&mut self, enabled: bool, start_tick: u32, end_tick: u32) {
        self.loop_chase = None;
        if !enabled || end_tick <= start_tick {
            self.loop_enabled = false;
            self.loop_start_tick = 0;
            self.loop_end_tick = 0;
            return;
        }
        self.loop_enabled = true;
        self.loop_start_tick = start_tick;
        self.loop_end_tick = end_tick;
        if self.playing {
            let start = self.tick_to_sample(start_tick);
            let end = self.tick_to_sample(end_tick);
            let pos = self.sample_position;
            if pos < start || pos >= end {
                self.seek_to(start);
            }
        }
    }

    /// 循环起点的绝对 sample。
    pub(crate) fn loop_start_sample(&self) -> u64 {
        self.tick_to_sample(self.loop_start_tick)
    }

    /// 循环终点的绝对 sample。
    pub(crate) fn loop_end_sample(&self) -> u64 {
        self.tick_to_sample(self.loop_end_tick)
    }

    /// 回绕到循环起点。
    ///
    /// 与用户 seek 不同：保留 insert / 通道 DSP / 乐器处理器的内部状态
    ///（delay 尾音、滤波器历史），只清 voice、回退游标，并应用 loop 起点
    /// 的 chase 快照。GPU 只 seek 不重建事件表。
    pub(crate) fn rewind_to_loop_start(&mut self) {
        let start = self.loop_start_sample();
        let end = self.loop_end_sample();
        // 绝对位置保持连续：逻辑位置从 end 跳回 start，偏移量累加一圈长度。
        // cpal 回调据此把“已消费绝对样本”换算成逻辑播放位置。
        if end > start {
            self.loop_wrap_offset = self.loop_wrap_offset.saturating_add(end - start);
        }
        self.relocate(start, RelocateMode::Loop);
    }

    pub(crate) fn seek_to(&mut self, sample: u64) {
        self.relocate(sample, RelocateMode::Seek);
    }

    fn relocate(&mut self, sample: u64, mode: RelocateMode) {
        let is_seek = matches!(mode, RelocateMode::Seek);
        let is_loop = matches!(mode, RelocateMode::Loop);
        let is_stop = matches!(mode, RelocateMode::Stop);
        if is_seek || is_stop {
            // 用户 seek / Stop：消费位置被重新对齐到逻辑位置，偏移归零。
            self.loop_wrap_offset = 0;
        }
        // yinhe CPU 后端启用时 ChannelSet 不参与渲染，重置/清音是死路径。
        if self.cpu_synth_active() {
            self.channel_set
                .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                    ChannelAudioEvent::AllNotesOff,
                )));
            self.channel_set
                .send_event(SynthEvent::AllChannels(ChannelEvent::Audio(
                    ChannelAudioEvent::ResetControl,
                )));
        }
        // yinhe CPU 后端：清 voice/通道状态 + cursor 定位到 seek 点
        {
            if let Some(cs) = self.cpu_synth.as_mut() {
                cs.seek(sample);
            }
            self.segment_start_sample = sample;
        }
        // 复位会把每通道 bank 打回 0：重新声明鼓组/旋律模式，
        // 否则 seek/循环回绕后鼓通道变回钢琴（「鼓通道开关无效」的表象之一）。
        self.reapply_percussion();
        if is_seek {
            // insert 效果器（delay 尾音/envelope 等）随用户 seek 清空内部状态。
            self.mixer.reset_inserts();
            // 内置音源通道处理段随 seek 清空内部状态（filter 历史等）。
            for chain in &mut self.channel_dsp {
                chain.reset();
            }
            // 乐器实例随 seek 清空内部状态（尾音/envelope/挂音）与事件累积。
            for inst in self.instruments.iter_mut().flatten() {
                for p in &mut inst.chain {
                    p.processor.reset();
                    p.params.clear();
                }
                inst.events.clear();
            }
            // 插件预览的 NoteOff 调度随 seek 作废（reset 已清插件挂音）。
            self.plugin_previews.clear();
        } else if is_loop {
            // Loop wrap：只给插件乐器发 NoteOff，不 reset 处理器（保留尾音），
            // 并丢弃回绕前尚未处理的插件事件。
            let all = std::mem::take(&mut self.active_notes);
            for std::cmp::Reverse(an) in all {
                if an.is_instrument
                    && let Some(Some(slot)) = self.instruments.get_mut(an.dense as usize)
                {
                    slot.events.push(PluginEvent::NoteOff {
                        time: 0,
                        channel: an.clap_channel,
                        key: an.key,
                        velocity: 0.0,
                    });
                }
            }
            // 插件预览的挂音同样要释放，否则回绕后 NoteOn 被丢、NoteOff 永远不来。
            for p in &self.plugin_previews {
                if p.triggered
                    && let Some(Some(slot)) = self.instruments.get_mut(p.dense)
                {
                    slot.events.push(PluginEvent::NoteOff {
                        time: 0,
                        channel: p.midi_channel,
                        key: p.key,
                        velocity: 0.0,
                    });
                }
            }
            self.plugin_previews.clear();
            for inst in self.instruments.iter_mut().flatten() {
                inst.events.clear();
                for p in &mut inst.chain {
                    p.params.clear();
                }
            }
        }

        self.sample_position = sample;
        self.current_tick = self.sample_to_tick(sample);
        self.note_cursor = [yinhe_types::NoteCursor::default(); KEY_COUNT];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self
            .cc_events
            .partition_point(|cc| cc.tick < self.current_tick);
        // 自 seek 点起重新打点：之前 dispatch 的控制器全部作废，chase 恢复全量生效
        //（跳过掩码为空，应用 chase 时不跳过任何控制器）。
        self.dispatched_skip = ChaseSkip::default();

        // 游标直接定位到模型桶里第一个 start_tick >= 当前位置的音符。
        let tick = self.current_tick;
        if let Some(model) = self.yin_model.clone() {
            for key in 0..KEY_COUNT {
                self.note_cursor[key] = model.notes[key].cursor_at(tick);
            }
        } else {
            self.note_cursor = [yinhe_types::NoteCursor::default(); KEY_COUNT];
        }

        // 用户 seek：扫描 seek 点之前开始、seek 点之后才结束的音符，全部重启。
        // Loop wrap 不重启跨点音符（循环区间内的长音在小节线处切断）。
        if is_seek {
            for (key, n, end_tick) in self.crossing_notes(tick) {
                self.restart_note(key, &n, end_tick);
            }
        }

        // 方案 B：chase（恢复 CC/PitchBend/RPN 等控制器值）移到 worker 线程异步计算。
        // renderer 在 seek_to 返回后发 PrepareChase，worker 算完回传 ChaseResult，
        // 由 apply_chase_result 应用。期间 channel state 是 ResetControl 后的初始值，
        // 渲染短暂静音 —— 比 renderer 线程同步阻塞几十万次 ChannelState::apply 更好。

        if !(is_seek || is_stop) {
            // Loop wrap：应用预计算的循环起点控制状态（CC/PB/PC/RPN/插件参数）。
            let chase = self.loop_chase.take();
            if let Some(c) = chase {
                if c.generation == self.chase_generation && c.start_tick == self.loop_start_tick {
                    self.apply_chase_result(&c.states, &c.plugin_params);
                }
                self.loop_chase = Some(c);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum RelocateMode {
    Seek,
    Loop,
    Stop,
}
