//! GPU 合成后端（yinhe-synth `GpuSynth`）的引擎侧适配。
//!
//! 集中三件事，renderer 不再直接触碰 GpuSynth 的事件语义：
//! 1. **事件表构建**：模型 tick 域事件（鼓组/CC/音符）→ sample 域事件列表
//!    （`build_gpu_events`）；
//! 2. **同步状态机**：`gpu_backend_dirty` 标志 + `sync_gpu_backend`，
//!    把"何时重建/seek"的判断从 renderer 的散点调用收敛到引擎内部；
//! 3. **chase 应用**：seek 后把 worker 的通道状态快照应用到 GpuSynth
//!    （`apply_gpu_chase`，由 `apply_chase_result` 统一调用）。

use xsynth_core::channel::ControlEvent;

use crate::channel::ChaseSkip;
use crate::engine::AudioEngine;

impl AudioEngine {
    /// 构建 GpuSynth 的 sample 域事件表：鼓组注入 + CC + 音符，按 sample 排序。
    ///
    /// `seek_pos`：渲染起点（0 = 从头）。鼓组/乐器模式事件在起点注入；起点之前
    /// 开始、之后才结束的音符在起点重启——均与 CPU `seek_to` 语义一致
    /// （GpuSynth 的 `load_events` 会清空全部通道状态，必须重建）。
    pub(crate) fn build_gpu_events(&self, seek_pos: u64) -> Vec<yinhe_synth::SynthEvent> {
        let audio_model = match self.model.as_ref() {
            Some(m) => m,
            None => return Vec::new(),
        };

        let mut events: Vec<yinhe_synth::SynthEvent> = Vec::new();

        // ── 鼓组/乐器模式初始状态（渲染起点注入；与 CPU setup_percussion 同序）──
        // 先 GM 鼓通道（每 port 的 9 通道），再模型 bank 声明（>=120 鼓组），
        // 同通道多条声明后者覆盖前者。
        for p in 0..16u8 {
            let src = p as usize * 16 + 9;
            let dense = self.channel_layout.dense_for(src);
            // GPU 合成器只支持前 MAX_CHANNELS 个 dense 槽位。
            if dense != u32::MAX && (dense as usize) < yinhe_synth::MAX_CHANNELS {
                events.push(yinhe_synth::SynthEvent::Control {
                    sample: seek_pos,
                    channel: dense as u8,
                    event: yinhe_synth::ControlEvent::PercussionMode(true),
                });
            }
        }
        for (track_idx, banks) in audio_model.track_banks.iter().enumerate() {
            if banks.is_empty() {
                continue;
            }
            let src = audio_model.track_channel(track_idx) as usize;
            if src >= 256 {
                continue;
            }
            let dense = self.channel_layout.dense_for(src);
            if dense == u32::MAX || (dense as usize) >= yinhe_synth::MAX_CHANNELS {
                continue;
            }
            for &(_, value) in banks {
                events.push(yinhe_synth::SynthEvent::Control {
                    sample: seek_pos,
                    channel: dense as u8,
                    event: yinhe_synth::ControlEvent::PercussionMode(value >= 120),
                });
            }
        }

        // ── 通道控制事件（CC/pitch bend/RPN），tick 域转 sample 域 ──
        // 放在音符事件之前：同 sample 时 CC 先于 note 处理（与 CPU dispatch 一致）
        for cc in self.cc_events.iter() {
            // mute 的音轨：跳过其自动化事件（与 CPU 路径 dispatch 一致）；
            // AM M/S 动态掩码：跳过被旁通的 lane（PC 事件 lane==哨兵，天然不跳过）。
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
            if track_skipped || lane_skipped {
                continue;
            }
            // GPU 路径不经过混音台/插件：插件参数事件无接收方，跳过。
            if cc.plugin_param.is_some() {
                continue;
            }
            // 插件乐器通道的事件由 CPU dispatch 转 MIDI 喂插件，不进 GPU。
            if self.channel_plugin_dense(cc.channel as u8).is_some() {
                continue;
            }
            let dense = self.channel_layout.dense_for(cc.channel as usize);
            if dense == u32::MAX || (dense as usize) >= yinhe_synth::MAX_CHANNELS {
                continue;
            }
            let Some(event) = to_backend_control_event(&cc.event) else {
                continue;
            };
            events.push(yinhe_synth::SynthEvent::Control {
                sample: self.tick_to_sample(cc.tick),
                channel: dense as u8,
                event,
            });
        }

        // ── 音符事件（带 dense channel）──
        for key in 0..128usize {
            for note in self.audible_notes[key].iter() {
                let track = note.track as usize;
                if self.skip_track.get(track).copied().unwrap_or(false) {
                    continue;
                }
                let ch = audio_model.track_channel(track) as usize;
                // 插件乐器通道的音符由 CPU dispatch 喂插件，不进 GPU。
                if self.channel_plugin_dense(ch as u8).is_some() {
                    continue;
                }
                let dense = self.channel_layout.dense_for(ch);
                if dense == u32::MAX || (dense as usize) >= yinhe_synth::MAX_CHANNELS {
                    continue;
                }

                let start_sample = self.tick_to_sample(note.start_tick);
                let end_sample = self.tick_to_sample(note.end_tick);
                // 跨 seek 点的音符：在 seek 点重启（CPU seek_to 同语义）
                let on_sample = if start_sample < seek_pos && end_sample > seek_pos {
                    seek_pos
                } else {
                    start_sample
                };
                // NoteOn 自带 end_sample：voice 到期自释，事件表不再产生 NoteOff
                //（事件量减半，且窗口分页装载不会被跨窗口的 NoteOff 打乱顺序）。
                events.push(yinhe_synth::SynthEvent::NoteOn {
                    sample: on_sample,
                    channel: dense as u8,
                    key: key as u8,
                    velocity: note.velocity,
                    end_sample,
                });
            }
        }

        events.sort_by_key(|e| e.sample());
        events
    }

    /// 标记 GPU 后端需要重新同步（事件表失效：模型/掩码/音符变化后调用）。
    pub(crate) fn invalidate_gpu_events(&mut self) {
        self.gpu_backend_dirty = true;
    }

    /// 把 GpuSynth 同步到当前位置：重建事件表 + seek。幂等，渲染线程每轮渲染前调用。
    ///
    /// GpuSynth 未创建时保持 dirty（音色库加载完成后创建，创建时立即同步），
    /// 创建后 dirty 消费一次即清零，不会每轮重复重建。
    pub(crate) fn sync_gpu_backend(&mut self) {
        if !self.gpu_backend_dirty || self.gpu_synth.is_none() {
            return;
        }
        // GPU 只支持前 MAX_CHANNELS 个 MIDI dense 通道：超出的通道事件在
        // `build_gpu_events` 里被静默丢弃。这里告警一次（不静默丢音）。
        if !self.gpu_overflow_warned
            && self.channel_layout.midi_compacted() > yinhe_synth::MAX_CHANNELS as u32
        {
            tracing::error!(
                "GPU 合成只支持前 {} 个 MIDI 通道（当前工程 {} 个），超出的通道将静音",
                yinhe_synth::MAX_CHANNELS,
                self.channel_layout.midi_compacted()
            );
            self.gpu_overflow_warned = true;
        }
        let pos = self.sample_position;
        // 先构建（&self）再装载（&mut self），避免借用冲突。
        let events = self.build_gpu_events(pos);
        let n = events.len();
        let t = std::time::Instant::now();
        if let Some(synth) = self.gpu_synth.as_mut() {
            synth.load_events(events);
            synth.seek(pos);
        }
        crate::audio_renderer::play_log(&format!(
            "[play] gpu后端同步：事件重建（{n} 事件）+ seek 用时={:?} pos={pos}",
            t.elapsed()
        ));
        self.gpu_backend_dirty = false;
    }

    /// 把 worker 算好的 chase 快照应用到 GpuSynth 的通道状态。
    pub(crate) fn apply_gpu_chase(&mut self, states: &[Option<crate::channel::ChannelState>; 256]) {
        let skip = match self.gpu_synth.as_ref() {
            Some(synth) => translate_backend_skip(&self.channel_layout, &synth.chase_skip()),
            None => return,
        };
        let Some(synth) = self.gpu_synth.as_mut() else {
            return;
        };
        for ch in 0..256u32 {
            let dense = self.channel_layout.dense_for(ch as usize);
            if dense == u32::MAX || (dense as usize) >= yinhe_synth::MAX_CHANNELS {
                continue;
            }
            // 无事件通道不触碰（与 CPU 路径 apply_chase_result 一致）。
            let Some(state) = &states[ch as usize] else {
                continue;
            };
            let events: Vec<yinhe_synth::ControlEvent> = state
                .events_to_send(ch as usize, &skip)
                .iter()
                .filter_map(to_backend_control_event)
                .collect();
            if !events.is_empty() {
                synth.apply_chase(dense, &events);
            }
        }
    }
}

/// yinhe-synth 后端的 `ChaseSkip`（按 dense 槽位索引）→ audio 侧 `ChaseSkip`
/// （按源通道 0..256 索引）。GPU 与 yinhe CPU 后端共用。
pub(crate) fn translate_backend_skip(
    layout: &crate::channel_layout::ChannelLayout,
    backend_skip: &yinhe_synth::ChaseSkip,
) -> ChaseSkip {
    let mut skip = ChaseSkip::default();
    for ch in 0..256usize {
        let dense = layout.dense_for(ch);
        if dense == u32::MAX || (dense as usize) >= yinhe_synth::MAX_CHANNELS {
            continue;
        }
        let idx = dense as usize;
        skip.cc_mask[ch] = backend_skip.cc_mask[idx];
        skip.pitch_bend[ch] = backend_skip.pitch_bend[idx];
        skip.pbs[ch] = backend_skip.pbs[idx];
        skip.fine_tune[ch] = backend_skip.fine_tune[idx];
        skip.coarse_tune[ch] = backend_skip.coarse_tune[idx];
        skip.program[ch] = backend_skip.program[idx];
    }
    skip
}

/// 事件流 `AudioEvent` → yinhe-synth 控制事件（GPU 事件构建与 CPU 分发共用）。
pub(crate) fn to_backend_control_event(
    ev: &crate::audio_model::AudioEvent,
) -> Option<yinhe_synth::ControlEvent> {
    use crate::audio_model::AudioEvent;
    match *ev {
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::Control(
            ControlEvent::Raw(c, v),
        )) => {
            // 通道级 DSP CC 由 yinhe-dsp 效果器处理，GPU 合成器不再接收。
            if yinhe_dsp::cc::DSP_CHANNEL_CCS.contains(&c) {
                return None;
            }
            Some(yinhe_synth::ControlEvent::Raw(c, v))
        }
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::Control(
            ControlEvent::PitchBendValue(v),
        )) => Some(yinhe_synth::ControlEvent::PitchBend(v)),
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::Control(
            ControlEvent::PitchBendSensitivity(v),
        )) => Some(yinhe_synth::ControlEvent::PitchBendSensitivity(v)),
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::Control(
            ControlEvent::FineTune(v),
        )) => Some(yinhe_synth::ControlEvent::FineTune(v)),
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::Control(
            ControlEvent::CoarseTune(v),
        )) => Some(yinhe_synth::ControlEvent::CoarseTune(v)),
        AudioEvent::Channel(xsynth_core::channel::ChannelAudioEvent::ProgramChange(p)) => {
            Some(yinhe_synth::ControlEvent::ProgramChange(p))
        }
        // 原生 RPN/NRPN：yinhe-synth 一等事件（u16 参数号），不拆 CC。
        AudioEvent::Rpn { parameter, value } => {
            Some(yinhe_synth::ControlEvent::Rpn { parameter, value })
        }
        AudioEvent::Nrpn { parameter, value } => {
            Some(yinhe_synth::ControlEvent::Nrpn { parameter, value })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::channel_layout::ChannelLayout;
    use crate::engine::AudioEngine;

    /// CPU 模式（gpu_synth = None）下 sync_gpu_backend 是 no-op：
    /// dirty 保持，但不得 panic、不得产生副作用（引擎无 GPU 后端时可安全调用）。
    #[test]
    fn sync_backend_is_noop_without_gpu_synth() {
        let layout = ChannelLayout::from_mask(vec![true]);
        let mut engine = AudioEngine::new(44_100, layout);
        engine.invalidate_gpu_events();
        engine.sync_gpu_backend();
        assert!(
            engine.gpu_backend_dirty,
            "无 GPU 后端时 dirty 保持，等创建后同步"
        );
    }
}
