//! 事件分发：tick 域逐事件派发（CC/NoteOn/NoteOff）到合成后端/插件/DSP。

use std::cmp::Reverse;

use xsynth_core::channel::ChannelAudioEvent;
use yinhe_mixer::PluginEvent;

use crate::audio_model::ActiveNote;
use crate::engine::AudioEngine;

/// 把一个通道事件路由到当前 CPU 后端（字段级展开，避免 `&mut self` 与
/// dispatch 循环持有的 `cc_events`/模型音符不可变借用冲突）：
/// - yinhe `CpuSynth` 存在 → 转 `yinhe_synth::SynthEvent` 投递（按 sample 生效）；
/// - 否则 → xsynth `ChannelSet`。
///
/// 事件 sample 取 `segment_start_sample`（每段渲染前由 render 循环更新）。
macro_rules! route_cpu_event {
    ($engine:ident, $dense:expr, $event:expr) => {{
        if let Some(cs) = $engine.cpu_synth.as_mut() {
            let sample = $engine.segment_start_sample;
            let dense = $dense;
            match $event {
                $crate::audio_model::AudioEvent::Channel(
                    xsynth_core::channel::ChannelAudioEvent::NoteOn { key, vel },
                ) => {
                    // CPU 路径沿用显式 NoteOff（与 xsynth 同语义）；end_sample 不自行到期。
                    cs.send_event(yinhe_synth::SynthEvent::NoteOn {
                        sample,
                        channel: dense as u8,
                        key,
                        velocity: vel,
                        end_sample: u64::MAX,
                    });
                }
                $crate::audio_model::AudioEvent::Channel(
                    xsynth_core::channel::ChannelAudioEvent::NoteOff { .. },
                ) => {
                    // yinhe CPU：NoteOn 带精确 `end_sample`（音符自身 end_tick）到期
                    // 自释（含 damper 快照），显式 NoteOff 不再投递——它是 FIFO 语义
                    // （释放该 key 最老未释放 voice），被 enforce 杀掉的 voice 会让
                    // 后续 NoteOff 错位释放下一个音符（听感上音符被逐个截短）。
                }
                other => {
                    // 原生 RPN/NRPN 由 `to_backend_control_event` 映射为
                    // yinhe-synth 的一等 `ControlEvent::Rpn/Nrpn`。
                    if let Some(ev) = $crate::synth_events::to_backend_control_event(&other) {
                        cs.send_event(yinhe_synth::SynthEvent::Control {
                            sample,
                            channel: dense as u8,
                            event: ev,
                        });
                    }
                }
            }
        } else {
            // XSynth 没有 RPN 事件：0/1/2 → 高层事件，其余拆 CC 序列。
            for ev in $crate::engine_render::xsynth_events(&$event).as_slice() {
                $engine.channel_set.send_event(
                    xsynth_core::channel_group::SynthEvent::Channel(
                        $dense,
                        xsynth_core::channel::ChannelEvent::Audio(*ev),
                    ),
                );
            }
        }
    }};
}

use super::{MidiMessages, event_to_midi, raw_cc};

impl AudioEngine {
    /// 合并了原来 `next_event_sample`、`dispatch_cc_until`、`dispatch_notes_at`
    /// 三个函数的职责，KEY_COUNT 桶只扫描一次。所有比较都在 tick 域，无需转换。
    ///
    pub(crate) fn dispatch_and_find_next(&mut self, tick: u32, block_end_tick: u32) -> Option<u32> {
        let mut next: Option<u32> = None;

        // ── CC 事件 ──
        // 本段包含全部 tick' <= tick 的待处理事件（cursor 之后的全在此段）。
        // 先过滤（mute 轨 / AM M/S 旁通 lane），再对同 tick 同通道同目标取
        // max：被过滤的事件不参与取 max 也不发送；整组被过滤则整组不发。
        // 聚合只改变"发送哪条/发多少"，事件流本身保持不变（旁通切换无需重建）。
        let seg_start = self.cc_cursor;
        let seg_end = self.cc_events.partition_point(|cc| cc.tick <= tick);
        if seg_end > seg_start {
            let reps = {
                let events = &self.cc_events[seg_start..seg_end];
                let skip_track = &self.skip_track;
                let am_lane_skip = &self.am_lane_skip;
                crate::audio_model::aggregate_max_reps(events, |cc| {
                    let track_skipped = skip_track.get(cc.track as usize).copied().unwrap_or(false);
                    let lane_skipped = am_lane_skip
                        .get(cc.track as usize)
                        .and_then(|v| v.get(cc.lane as usize))
                        .copied()
                        .unwrap_or(false);
                    !track_skipped && !lane_skipped
                })
            };
            for &i in &reps {
                let cc = &self.cc_events[seg_start + i];
                if let Some(pp) = cc.plugin_param {
                    // 插件参数自动化 → 按 uid 精确投给对应乐器实例（定向，不广播）。
                    let dense = self.channel_layout.dense_for(pp.channel as usize);
                    if dense != u32::MAX {
                        let time = self
                            .tick_to_sample(cc.tick)
                            .saturating_sub(self.block_start_sample)
                            as u32;
                        if let Some(Some(src)) = self.instruments.get_mut(dense as usize)
                            && let Some(p) = src.chain.iter_mut().find(|p| p.slot_id == pp.uid)
                        {
                            p.params.push(PluginEvent::ParamValue {
                                time,
                                param_id: pp.param_id,
                                value: f64::from(pp.value),
                            });
                        }
                    }
                } else {
                    // 内置音源的通道处理段（CC7/10/11/71/74）；挂插件乐器的
                    // 通道跳过（CC 透传插件，插件自己响应）。两层互斥：
                    // CC 的消费者只有音源一侧，不存在广播/双发语义。
                    if let Some((cc_num, cc_value)) = raw_cc(&cc.event) {
                        let dense = self.channel_layout.dense_for(cc.channel as usize);
                        if dense != u32::MAX
                            && (dense as usize) < self.channel_layout.midi_compacted() as usize
                            && self
                                .instruments
                                .get(dense as usize)
                                .is_none_or(|s| s.is_none())
                            && yinhe_dsp::cc::DSP_CHANNEL_CCS.contains(&cc_num)
                            && let Some(chain) = self.channel_dsp.get_mut(dense as usize)
                        {
                            chain.apply_cc(cc_num, cc_value);
                            // 实际发送 → 打点（chase 应用时跳过，避免旧值覆盖新值）。
                            self.dispatched_skip.mark(&cc.event, cc.channel as usize);
                        }
                    }
                    if let Some(dense) = self.channel_plugin_dense(cc.channel as u8) {
                        // 该 MIDI 通道挂了插件 → CC/PB/RPN/PC 转原始 MIDI 字节喂实例；
                        // 否则走 xsynth/yinhe。RPN/NRPN 在这里拆成标准 CC 序列
                        // （MIDI 1.0 插件只认 CC，没有 RPN 报文类型）。
                        // 先算 frame offset（只读），再取可变实例引用，避免整机借用冲突。
                        let time = self
                            .tick_to_sample(cc.tick)
                            .saturating_sub(self.block_start_sample)
                            as u32;
                        let mut midi = MidiMessages::default();
                        event_to_midi(&cc.event, cc.channel as u8, &mut midi);
                        if !midi.is_empty()
                            && let Some(Some(slot)) = self.instruments.get_mut(dense)
                        {
                            for data in midi.as_slice() {
                                slot.events.push(PluginEvent::Midi { time, data: *data });
                            }
                            self.dispatched_skip.mark(&cc.event, cc.channel as usize);
                        }
                    } else {
                        let dense = self.channel_layout.dense_for(cc.channel as usize);
                        if dense != u32::MAX {
                            // 路由到当前 CPU 后端。
                            route_cpu_event!(self, dense, cc.event);
                            self.dispatched_skip.mark(&cc.event, cc.channel as usize);
                        }
                    }
                }
            }
            self.cc_cursor = seg_end;
        }
        if self.cc_cursor < self.cc_events.len() {
            let cc_tick = self.cc_events[self.cc_cursor].tick;
            if cc_tick < block_end_tick {
                next = Some(next.map_or(cc_tick, |t| t.min(cc_tick)));
            }
        }

        // ── NoteOn + 找下一个 NoteOn 边界（单次 KEY_COUNT 桶扫描）──
        // 直接读模型桶：游标单调前进，整曲每个音符只碰一次；力度阈值与
        // 0 BPM 剪切在这里逐音符现算，不再维护 `audible_notes` 副本。
        let Some(model) = self.yin_model.clone() else {
            return next;
        };
        for key in 0..yinhe_types::KEY_COUNT {
            let bucket = &model.notes[key];
            let mut cursor = self.note_cursor[key];

            while let Some((nc, note)) = bucket.peek(cursor) {
                cursor = nc;
                // 先做可听性判断：被力度阈值过滤的音符不是事件边界，
                // 否则每个哑音都会多触发一次 dispatch（每次都要扫 256 个 key）。
                let audible = crate::prepare_model::audible_end(&model, note, self.ignore_velocity);
                if note.start_tick > tick {
                    if audible.is_none() {
                        // 哑音不是边界：本块内直接越过；到块尾就停，
                        // 留给下一块继续（避免一次调用一路扫到曲尾）。
                        if note.start_tick >= block_end_tick {
                            break;
                        }
                        bucket.advance(&mut cursor);
                        continue;
                    }
                    // 该桶下一个待处理**可听**音符 → 记录为边界候选
                    if note.start_tick < block_end_tick {
                        next = Some(next.map_or(note.start_tick, |t| t.min(note.start_tick)));
                    }
                    break;
                }
                // start_tick ≤ tick 且可听 → dispatch NoteOn；被过滤则前进。
                let Some(end_tick) = audible else {
                    bucket.advance(&mut cursor);
                    continue;
                };
                let track = note.track as usize;
                let ch = self
                    .model
                    .as_ref()
                    .map(|m| m.track_channel(track))
                    .unwrap_or(0);
                if !self.skip_track.get(track).copied().unwrap_or(false) {
                    let dense = self.channel_layout.dense_for(ch as usize);
                    if dense != u32::MAX {
                        // 力度表记账（独立于后端分支，保证各后端一致）。
                        self.meter_notes.push(std::cmp::Reverse((
                            end_tick,
                            track as u16,
                            note.velocity,
                        )));
                        if let Some(h) = self.track_vel_hist.get_mut(track) {
                            let v = (note.velocity & 0x7f) as usize;
                            h[v] = h[v].saturating_add(1);
                            if note.velocity > self.track_vel_max[track] {
                                self.track_vel_max[track] = note.velocity;
                            }
                        }
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
                                    end_tick,
                                    track: track as u16,
                                }));
                            }
                        } else {
                            // yinhe CPU：直接投递带精确 `end_sample` 的 NoteOn（该音符
                            // 自身的 end_tick），voice 到期自释 + damper 快照，无需
                            // NoteOff；xsynth 回退路径才走 `route_cpu_event!`。
                            let sample = self.segment_start_sample;
                            let end_sample = self.tick_to_sample(end_tick);
                            let vel = note.velocity;
                            if let Some(cs) = self.cpu_synth.as_mut() {
                                cs.send_event(yinhe_synth::SynthEvent::NoteOn {
                                    sample,
                                    channel: dense as u8,
                                    key: key as u8,
                                    velocity: vel,
                                    end_sample,
                                });
                            } else {
                                route_cpu_event!(
                                    self,
                                    dense,
                                    crate::audio_model::AudioEvent::Channel(
                                        ChannelAudioEvent::NoteOn {
                                            key: key as u8,
                                            vel,
                                        }
                                    )
                                );
                            }
                            self.active_notes.push(Reverse(ActiveNote {
                                key: key as u8,
                                dense,
                                clap_channel: 0,
                                is_instrument: false,
                                end_tick,
                                track: track as u16,
                            }));
                        }
                    }
                }
                bucket.advance(&mut cursor);
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
        // 力度表：收音记账（所有后端统一，与 active_notes 独立）。
        while let Some(&std::cmp::Reverse((end, track, vel))) = self.meter_notes.peek() {
            if end > tick {
                break;
            }
            self.meter_notes.pop();
            self.track_vel_off(track, vel);
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
                if dense != u32::MAX {
                    route_cpu_event!(
                        self,
                        dense,
                        crate::audio_model::AudioEvent::Channel(ChannelAudioEvent::NoteOff {
                            key: an.key
                        })
                    );
                }
            }
        }

        next
    }
}
