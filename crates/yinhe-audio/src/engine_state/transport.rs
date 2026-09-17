//! 传输与位置：暂停/恢复/seek、轨道掩码切换。

use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;
use yinhe_mixer::PluginEvent;
use yinhe_types::KEY_COUNT;

use crate::audio_model::AudibleNote;
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

    pub(crate) fn seek_to(&mut self, sample: u64) {
        // GPU 模式下 ChannelSet 不参与渲染，重置/清音是死路径
        //（GPU 的状态重置由事件表在 seek_pos 重建完成）。
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
        // insert 效果器（delay 尾音/envelope 等）随 seek 清空内部状态
        self.mixer.reset_inserts();
        // 内置音源通道处理段随 seek 清空内部状态（filter 历史等）
        for chain in &mut self.channel_dsp {
            chain.reset();
        }
        // 乐器实例随 seek 清空内部状态（尾音/envelope/挂音）与事件累积。
        for inst in self.instruments.iter_mut().flatten() {
            inst.processor.reset();
            inst.events.clear();
        }
        // 插件预览的 NoteOff 调度随 seek 作废（reset 已清插件挂音）。
        self.plugin_previews.clear();

        self.sample_position = sample;
        self.current_tick = self.sample_to_tick(sample);
        self.note_cursor = [0; KEY_COUNT];
        self.cc_cursor = 0;
        self.active_notes.clear();

        self.cc_cursor = self
            .cc_events
            .partition_point(|cc| cc.tick < self.current_tick);
        // 自 seek 点起重新打点：之前 dispatch 的控制器全部作废，chase 恢复全量生效
        //（跳过掩码为空，应用 chase 时不跳过任何控制器）。
        self.dispatched_skip = ChaseSkip::default();

        // Reset note cursors to the correct position based on pre-built audible_notes.
        // 桶内 start_tick 严格升序，partition_point 谓词单调，结果正确（修 P0-2）。
        let tick = self.current_tick;
        for key in 0..KEY_COUNT {
            let notes = self.audible_notes[key].as_slice();
            let cursor = notes.partition_point(|n| n.start_tick < tick);
            self.note_cursor[key] = cursor;

            // 扫描 seek 点之前开始、seek 点之后才结束的所有音符，全部重启（修 P2-10）。
            // 桶按 start_tick 升序，但 end_tick 不保证有序，必须线性扫 [..cursor]。
            // 黑乐谱叠层场景下 cursor 前通常有几十个跨点音符，O(cursor) 完全可接受。
            // 先收集再逐条重启（避免借用冲突，`restart_note` 需 &mut self）。
            let mut to_restart: Vec<AudibleNote> = Vec::new();
            for n in &notes[..cursor] {
                if n.end_tick > tick {
                    to_restart.push(*n);
                }
            }
            for n in to_restart {
                self.restart_note(key, &n);
            }
        }

        // 方案 B：chase（恢复 CC/PitchBend/RPN 等控制器值）移到 worker 线程异步计算。
        // renderer 在 seek_to 返回后发 PrepareChase，worker 算完回传 ChaseResult，
        // 由 apply_chase_result 应用。期间 channel state 是 ResetControl 后的初始值，
        // 渲染短暂静音 —— 比 renderer 线程同步阻塞几十万次 ChannelState::apply 更好。

        // GPU 后端位置跳变：置位同步标志，渲染线程渲染前统一重建事件表 + seek。
        #[cfg(feature = "gpu")]
        {
            self.gpu_backend_dirty = true;
        }
    }
}
