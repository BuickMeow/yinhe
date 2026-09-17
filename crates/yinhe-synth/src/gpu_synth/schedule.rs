//! 事件调度：块内事件收集为段结构，note on/off 与通道释放指令，chase 应用。

use crate::sfz_parser;
use crate::synth::buffers::MAX_VOICE_SLOTS;
use crate::synth::{ChState, EnvUpdateCmd, GpuVoiceState, ReleaseCmd, SegInfo};

use super::{ControlEvent, GpuSynth, MAX_CHANNELS, SynthEvent};
use crate::channel_state::ChaseSkip;
use crate::channel_state::{ChannelState, env_curve_frames, is_env_effect_cc};

/// dense 通道号 → 槽位索引；>= MAX_CHANNELS 返回 None（GPU 合成器只支持 32 槽位）。
fn dense_channel(channel: usize) -> Option<usize> {
    (channel < MAX_CHANNELS).then_some(channel)
}

/// 立即结束 voice 的 kill 指令（mode 6），vid 为 voices 列表索引。
fn kill_cmd(frame: u32, vid: usize) -> ReleaseCmd {
    ReleaseCmd {
        frame,
        vid: vid as u32,
        mode: 6,
        _pad: 0,
    }
}

/// 正常释放 voice 的 release 指令（mode 5）。
fn release_cmd(frame: u32, vid: usize) -> ReleaseCmd {
    ReleaseCmd {
        frame,
        vid: vid as u32,
        mode: 5,
        _pad: 0,
    }
}

/// voice + MIDI key + 所属通道 + 通道无关的基础参数。
#[derive(Clone, Debug)]
pub(super) struct Voice {
    pub(super) state: GpuVoiceState,
    pub(super) key: u8,
    pub(super) channel: u8,
    /// NoteOn 力度（per-key layer 超限时按 xsynth 语义杀最弱 voice）。
    pub(super) velocity: u8,
    /// 音符结束时间（绝对 sample）：voice 到期自行 release（NoteOn 携带）。
    /// 事件表因此不再需要 NoteOff 事件（分页装载时 NoteOff 归属会破坏事件顺序）。
    pub(super) end_sample: u64,
    /// region 原始 attack/release 帧数（CC72/73 重算的基准，多次 CC 不累积）
    pub(super) orig_attack_frames: f32,
    pub(super) orig_release_frames: f32,
    /// 是否被延音踏板保持（CC64 踩着时 note_off 只标记不释放）。
    pub(super) held_by_damper: bool,
    /// 已发 release 指令等待 shader 在指令帧应用（防止同 key 重复匹配）。
    /// 不预置 env_stage：预置会让 shader 在指令应用前就按 release 阶段推进
    /// （旧 env_start=0 会把 envelope 清零）。
    pub(super) release_pending: bool,
}

/// 一段的事件结构，render_to_mixer 内先按段 collect 保存所有权，
/// 再构造 RenderSegment 借用视图传给 renderer 一次性渲染。
/// 实例常驻 `GpuSynth::seg_scratch` 复用（每块只 clear，不重新分配）。
#[derive(Default)]
pub(super) struct SegBuffers {
    pub(super) frame_start: u32,
    pub(super) frame_length: u32,
    pub(super) segs: Vec<SegInfo>,
    pub(super) ch_updates: Vec<ChState>,
    pub(super) releases: Vec<ReleaseCmd>,
    pub(super) env_cmds: Vec<EnvUpdateCmd>,
}

/// xsynth FilterType → shader 滤波器类型编号（与 voice_render.wgsl 一致）
impl GpuSynth {
    /// 收集块内事件为段结构：
    /// - 段边界 = CC 事件位置（ch_updates 记录受影响通道的状态快照）
    /// - note_on 创建 voice（块内帧偏移）；note_off 发 release 指令（帧 + vid）
    /// - CC72/73/121 发 env 指令；damper 松开/AllNotesOff 发 release/kill 指令
    /// - CPU 通道状态按段推进（与 shader 逐帧推进线性一致）
    #[allow(clippy::too_many_arguments)] // 块内事件收集的上下文透传
    pub(super) fn collect_block(
        &mut self,
        block_start: u64,
        block_end: u64,
        segs: &mut Vec<SegInfo>,
        ch_updates: &mut Vec<ChState>,
        releases: &mut Vec<ReleaseCmd>,
        env_cmds: &mut Vec<EnvUpdateCmd>,
    ) {
        // 本块内的 CC 位置一次收集（升序；段边界逐段消费）。
        // 逐段重扫全部事件是 O(事件数 × CC 数)，黑乐谱密集事件块下不可忽略；
        // event_cursor 处事件恒 >= block_start（块末 cursor 停在 >= block_end 处）。
        self.cc_scratch.clear();
        for ev in &self.events[self.event_cursor..] {
            let s = ev.sample();
            if s >= block_end {
                break;
            }
            if matches!(ev, SynthEvent::Control { .. }) {
                self.cc_scratch.push(s);
            }
        }

        // 段 0：块起点的通道 pitch 变化（seek/chase/调音后）→ shader 初始化时应用
        // speed = base_speed × speed_mult。voice 状态常驻 GPU，CPU 不再逐 voice 同步。
        let seg0_off = ch_updates.len();
        for ch_idx in 0..MAX_CHANNELS {
            let m = self.channels[ch_idx].pitch_multiplier();
            if (m - self.channel_speed_cache[ch_idx]).abs() > f32::EPSILON {
                self.channel_speed_cache[ch_idx] = m;
                ch_updates.push(ChState {
                    ch: ch_idx as u32,
                    speed_mult: m,
                });
            }
        }
        let seg0_count = ch_updates.len() - seg0_off;
        // 段 0 的 start_frame=0：块起点有 CC 事件时其更新在段 1（start_frame=0）。
        segs.push(SegInfo {
            start_frame: 0,
            ch_off: seg0_off as u32,
            ch_count: seg0_count as u32,
            _pad: 0,
        });
        let mut seg_start = block_start;
        let mut seg_frame = 0u32;
        let mut seg_ch_off = ch_updates.len();
        let mut cc_idx = 0usize;

        loop {
            // 下一个未处理的 CC 位置（跳过已消费的项；cc_scratch 升序）
            let next_cc = self.cc_scratch.get(cc_idx).copied();

            // 段 [seg_start, next_cc) 内的音符事件（sample == next_cc 的留给段边界）
            while self.event_cursor < self.events.len() {
                let ev = self.events[self.event_cursor];
                if ev.sample() >= next_cc.unwrap_or(block_end) || ev.sample() >= block_end {
                    break;
                }
                if ev.sample() >= seg_start {
                    let seg_offset = (ev.sample() - seg_start) as u32;
                    let block_frame = seg_frame + seg_offset;
                    match ev {
                        SynthEvent::NoteOn {
                            channel,
                            key,
                            velocity,
                            end_sample,
                            ..
                        } => {
                            self.note_on(channel, key, velocity, end_sample, block_frame, releases)
                        }
                        SynthEvent::NoteOff { channel, key, .. } => {
                            self.note_off_to_cmd(channel, key, block_frame, releases);
                        }
                        SynthEvent::Control { .. } => unreachable!("CC 由段边界处理"),
                    }
                }
                self.event_cursor += 1;
            }

            // 段边界（CC 事件位置）：推进通道 → 处理该位置所有事件（CC + 音符）
            let Some(cc_sample) = next_cc.filter(|&s| s < block_end) else {
                break;
            };
            let frame = (cc_sample - block_start) as u32;
            let seg_ch_off_before = seg_ch_off;
            self.process_events_at(cc_sample, frame, ch_updates, releases, env_cmds);
            let ch_count = ch_updates.len() - seg_ch_off_before;
            // 同 sample 的重复 CC 项一并消费（事件已全部处理）
            while cc_idx < self.cc_scratch.len() && self.cc_scratch[cc_idx] <= cc_sample {
                cc_idx += 1;
            }

            segs.push(SegInfo {
                start_frame: frame,
                ch_off: seg_ch_off_before as u32,
                ch_count: ch_count as u32,
                _pad: 0,
            });
            seg_frame = frame;
            seg_ch_off = ch_updates.len();
            seg_start = cc_sample;
        }

        // 最后一段 [seg_start, block_end)
        segs.push(SegInfo {
            start_frame: seg_frame,
            ch_off: seg_ch_off as u32,
            ch_count: (ch_updates.len() - seg_ch_off) as u32,
            _pad: 0,
        });

        // 到期释放：NoteOn 自带 `end_sample`，落在本段 [block_start, block_end) 的
        // voice 于段内释放（取代 NoteOff 事件；延音踏板按住时只标记 held，
        // 由踏板松开路径统一释放）。每渲染段扫一次 O(V)，V ≤ MAX_VOICE_SLOTS。
        let dampers: [bool; MAX_CHANNELS] = std::array::from_fn(|i| self.channels[i].damper);
        for (i, v) in self.voices.iter_mut().enumerate() {
            if v.state.env_stage >= 5 || v.release_pending || v.held_by_damper {
                continue;
            }
            if v.end_sample < block_start || v.end_sample >= block_end {
                continue;
            }
            let Some(ch_idx) = dense_channel(v.channel as usize) else {
                continue;
            };
            if dampers[ch_idx] {
                v.held_by_damper = true;
            } else {
                v.release_pending = true;
                releases.push(release_cmd((v.end_sample - block_start) as u32, i));
            }
        }
    }

    /// 处理段边界（同一 sample 位置）的所有事件：CC 更新通道状态并记录 ch_updates、
    /// 音符按偏移 0 分发（note_on 用段边界通道值快照）；damper 释放 / env 指令同发。
    fn process_events_at(
        &mut self,
        sample: u64,
        frame: u32,
        ch_updates: &mut Vec<ChState>,
        releases: &mut Vec<ReleaseCmd>,
        env_cmds: &mut Vec<EnvUpdateCmd>,
    ) {
        while self.event_cursor < self.events.len() {
            let ev = self.events[self.event_cursor];
            if ev.sample() != sample {
                break;
            }
            match ev {
                SynthEvent::NoteOn {
                    channel,
                    key,
                    velocity,
                    end_sample,
                    ..
                } => self.note_on(channel, key, velocity, end_sample, frame, releases),
                SynthEvent::NoteOff { channel, key, .. } => {
                    self.note_off_to_cmd(channel, key, frame, releases);
                }
                SynthEvent::Control { channel, event, .. } => {
                    let Some(ch_idx) = dense_channel(channel as usize) else {
                        continue;
                    };
                    match event {
                        // All Sounds Off (CC78)：结束所有 voice；
                        // All Notes Off (CC7B)：结束所有非 held voice（held 等 damper 松开）
                        ControlEvent::Raw(cc @ (0x78 | 0x7B), 0) => {
                            let all = cc == 0x78;
                            for (i, v) in self.voices.iter_mut().enumerate() {
                                if v.state.env_stage < 6 && (all || !v.held_by_damper) {
                                    v.state.env_stage = 6;
                                    releases.push(kill_cmd(frame, i));
                                }
                            }
                        }
                        _ => {
                            let damper_released = self.channels[ch_idx].process_control(event);
                            if damper_released {
                                // 松开延音踏板：释放该通道所有被保持的 voice
                                for (i, v) in self.voices.iter_mut().enumerate() {
                                    if v.channel == channel {
                                        if v.held_by_damper
                                            && v.state.env_stage < 5
                                            && !v.release_pending
                                        {
                                            v.release_pending = true;
                                            releases.push(release_cmd(frame, i));
                                        }
                                        v.held_by_damper = false;
                                    }
                                }
                            }
                            // CC72/73 修改包络时长、CC121 重置包络：传播到该通道活跃 voice
                            if is_env_effect_cc(&event) {
                                self.propagate_env_controls_to_cmds(ch_idx, frame, env_cmds);
                            }
                            // 记录该通道的段边界状态（shader 段边界应用）
                            let ch = self.channels[ch_idx];
                            ch_updates.push(ChState {
                                ch: ch_idx as u32,
                                speed_mult: ch.pitch_multiplier(),
                            });
                        }
                    }
                }
            }
            self.event_cursor += 1;
        }
    }

    /// CC72/73（及 CC121 重置）后重算该通道所有活跃 voice 的 attack/release 时长：
    /// 基于 region 原始值重算（多次 CC 不累积），shader 在指令帧应用并重走当前阶段。
    /// **不**同步修改 voice state：若提前更新，shader 在指令帧之前就用新时长推进
    /// （与 xsynth 事件帧才更新 params 不一致，release 起点 env 会偏差）。
    fn propagate_env_controls_to_cmds(
        &mut self,
        ch_idx: usize,
        frame: u32,
        env_cmds: &mut Vec<EnvUpdateCmd>,
    ) {
        let ch = self.channels[ch_idx];
        for (i, v) in self.voices.iter_mut().enumerate() {
            if v.channel as usize != ch_idx || v.state.env_stage >= 6 {
                continue;
            }
            let (attack_frames, release_frames) = Self::env_frames_for(&ch, v, self.sample_rate);
            env_cmds.push(EnvUpdateCmd {
                frame,
                vid: i as u32,
                attack_frames,
                release_frames,
            });
        }
    }

    /// NoteOn（block_frame = 块内起始帧）。
    /// key_map 已按 (key, vel) 展开为最终参数快照，这里零公式计算直接消费。
    /// 超 voice 上限时淘汰最老的 voice（发 kill 指令，不 remove——索引保持稳定）。
    pub fn note_on(
        &mut self,
        channel: u8,
        key: u8,
        vel: u8,
        end_sample: u64,
        block_frame: u32,
        releases: &mut Vec<ReleaseCmd>,
    ) {
        // 音色库选择：dense 通道 → port → (bank, preset) 条目（与 xsynth
        // ChannelSoundfont::rebuild_matrix 一致：主选 + 兜底，落空静音）。
        let Some(ch_idx) = dense_channel(channel as usize) else {
            eprintln!("[dbg] note_on: dense_channel 失败 ch={channel}");
            return;
        };
        // voice 槽位上限（状态常驻 GPU，槽位固定）；超限时由 maybe_compact_voices
        // 在块边界压缩，这里防御性拒绝。
        if self.voices.len() >= MAX_VOICE_SLOTS as usize {
            eprintln!("[dbg] note_on: slots 满");
            return;
        }
        let ch = self.channels[ch_idx];
        let entries = &self.port_key_maps[self.channel_port[ch_idx] as usize];
        let info = match sfz_parser::select_key_info_multi(entries, ch.bank, ch.program, key, vel) {
            Some(i) => i,
            None => {
                eprintln!(
                    "[dbg] note_on: select 失败 key={key} vel={vel} bank={} prog={}",
                    ch.bank, ch.program
                );
                return;
            }
        };
        let (offset, length) = match self
            .sample_offsets
            .get(&(info.sample_data.as_ptr() as usize))
        {
            Some(&v) => v,
            None => {
                eprintln!("[dbg] note_on: offsets 无该采样指针 key={key}");
                return;
            }
        };
        if length == 0 {
            eprintln!("[dbg] note_on: length=0 key={key}");
            return;
        }

        // 声像：cos/sin 法则（无 1.42 等功率补偿）——与 CpuSynth 及 xsynth 实测
        // 行为一致（见 cpu_synth/voice.rs 注释）。
        let angle = info.pan * std::f32::consts::FRAC_PI_2;
        let (base_pan_l, base_pan_r) = (angle.cos().min(1.0), angle.sin().min(1.0));
        // 播放长度：SF2 的 sample_end（xsynth LoopParams.stop）封顶，SFZ 到采样末尾
        let sample_length = match info.stop {
            Some(stop) => stop
                .saturating_sub(info.offset)
                .min(length.saturating_sub(info.offset)),
            None => length.saturating_sub(info.offset),
        };

        // per-voice biquad 系数（RBJ cookbook，与 xsynth 一致）；cutoff=0 时无滤波器
        let (flt_b0, flt_b1, flt_b2, flt_a1, flt_a2) = if info.cutoff > 0.0 {
            crate::synth::biquad_coeffs(
                crate::sfz_parser::filter_type_code(info.filter_type),
                info.cutoff,
                info.resonance,
                self.sample_rate as f32,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0, 0.0)
        };

        // CC72/73：用通道当前值缩放 region 原始时长（多次 CC 不累积）
        let sr = self.sample_rate as f32;
        let orig_attack_frames = info.ampeg_attack * sr;
        let orig_release_frames = info.ampeg_release * sr;
        let attack_frames = match ch.env_attack {
            Some(cc) => env_curve_frames(cc, orig_attack_frames, self.sample_rate, false),
            None => orig_attack_frames,
        };
        let release_frames = match ch.env_release {
            Some(cc) => env_curve_frames(cc, orig_release_frames, self.sample_rate, true),
            None => orig_release_frames,
        };
        let new_index = self.voices.len();
        self.voices.push(Voice {
            key,
            channel,
            velocity: vel,
            end_sample,
            orig_attack_frames,
            orig_release_frames,
            held_by_damper: false,
            release_pending: false,
            state: GpuVoiceState {
                sample_offset: offset + info.offset,
                sample_length,
                speed: info.speed_mult * ch.pitch_multiplier(),
                base_speed: info.speed_mult,
                base_gain: info.volume,
                time: 0.0,
                start_offset: block_frame,
                // dense 通道号（note_on 已过滤 < MAX_CHANNELS）
                channel: channel as u32,
                envelope: info.ampeg_start,
                env_stage: 0,
                stage_progress: 0.0,
                // envelope 归一化 0..1，增益由 gain 单独乘（xsynth 语义）
                env_level: 1.0,
                sustain_level: info.ampeg_sustain,
                env_start: info.ampeg_start,
                decay_start: info.ampeg_start,
                delay_frames: info.ampeg_delay * sr,
                attack_frames,
                hold_frames: info.ampeg_hold * sr,
                decay_frames: info.ampeg_decay * sr,
                release_frames,
                base_pan_l,
                base_pan_r,
                loop_start: info.loop_start,
                loop_end: info.loop_end,
                loop_mode: info.loop_mode as u32,
                is_stereo: info.is_stereo as u32,
                interp: info.interp,
                cutoff: info.cutoff,
                resonance: info.resonance,
                filter_type: crate::sfz_parser::filter_type_code(info.filter_type),
                flt_b0,
                flt_b1,
                flt_b2,
                flt_a1,
                flt_a2,
                flt_x1: 0.0,
                flt_x2: 0.0,
                flt_y1: 0.0,
                flt_y2: 0.0,
                flt_x1r: 0.0,
                flt_x2r: 0.0,
                flt_y1r: 0.0,
                flt_y2r: 0.0,
            },
        });

        // per-key layer 上限（SetLayerCount）：超限时反复杀该 key velocity 最低的
        // 未释放 voice（xsynth pop_quietest_voice_group 语义；跳过刚加入的）。
        if let Some(max) = self.max_layers {
            loop {
                let count = self
                    .voices
                    .iter()
                    .filter(|v| v.channel == channel && v.key == key && v.state.env_stage < 6)
                    .count();
                if count <= max {
                    break;
                }
                let victim = self
                    .voices
                    .iter()
                    .enumerate()
                    .filter(|(i, v)| {
                        *i != new_index
                            && v.channel == channel
                            && v.key == key
                            && v.state.env_stage < 5
                            && !v.release_pending
                            && !v.held_by_damper
                    })
                    .min_by_key(|(_, v)| v.velocity)
                    .map(|(i, _)| i);
                let Some(idx) = victim else {
                    break; // 其余已在 release 中，无候选
                };
                let v = &mut self.voices[idx];
                v.state.env_stage = 6;
                v.held_by_damper = false;
                releases.push(kill_cmd(block_frame, idx));
            }
        }

        // 超限淘汰：优先杀最老的 release 中 voice（听感最弱），否则杀最老的 active。
        // 只预置 stage 6 + 发 kill 指令，不 remove——本块已生成的指令索引保持稳定。
        while self.voices.len() > self.max_voices {
            let idx = self
                .voices
                .iter()
                .position(|v| v.state.env_stage == 5)
                .unwrap_or(0);
            let v = &mut self.voices[idx];
            if v.state.env_stage < 6 {
                v.state.env_stage = 6;
                v.held_by_damper = false;
                releases.push(kill_cmd(block_frame, idx));
            } else {
                break; // 其余已被淘汰（块末统一清理），不再继续
            }
        }
    }

    /// NoteOff — 释放该 (channel, key) 最老的未释放 voice（与 xsynth `release_next_voice` 一致：
    /// 同 key 多次按下的 voice 逐个释放，后按的 voice 继续响）。
    /// 延音踏板踩着时只标记 held，不释放。
    /// 实际释放由 shader 在 frame 帧应用 release 指令完成。
    pub fn note_off_to_cmd(
        &mut self,
        channel: u8,
        key: u8,
        frame: u32,
        releases: &mut Vec<ReleaseCmd>,
    ) {
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper = self.channels[ch_idx].damper;
        for (i, v) in self.voices.iter_mut().enumerate() {
            // 跳过已 held 的 voice（xsynth damper 分支只匹配 "isn't being held" 的
            // voice：否则同 key 多个 off 会重复匹配同一个 held voice，其余 voice 永不释放）
            if v.channel == channel
                && v.key == key
                && v.state.env_stage < 5
                && !v.held_by_damper
                && !v.release_pending
            {
                if damper {
                    v.held_by_damper = true;
                } else {
                    v.release_pending = true;
                    releases.push(release_cmd(frame, i));
                }
                break;
            }
        }
    }

    /// CC72/73 重算单个 voice 的 attack/release 帧数（基于 region 原始值，多次 CC 不累积）。
    fn env_frames_for(ch: &ChannelState, v: &Voice, sample_rate: u32) -> (f32, f32) {
        let attack_frames = match ch.env_attack {
            Some(cc) => env_curve_frames(cc, v.orig_attack_frames, sample_rate, false),
            None => v.state.attack_frames,
        };
        let release_frames = match ch.env_release {
            Some(cc) => env_curve_frames(cc, v.orig_release_frames, sample_rate, true),
            None => v.state.release_frames,
        };
        (attack_frames, release_frames)
    }

    /// 计算 seek 后已处理的控制事件跳过掩码（事件区间 [chase_base, event_cursor)）。
    /// 由 yinhe-audio 在 `ChaseResult` 到达时调用，跳过的控制器不再被 chase 覆盖。
    pub fn chase_skip(&self) -> ChaseSkip {
        let mut skip = ChaseSkip::default();
        for ev in &self.events[self.chase_base..self.event_cursor] {
            let SynthEvent::Control { channel, event, .. } = ev else {
                continue;
            };
            let Some(ch) = dense_channel(*channel as usize) else {
                continue;
            };
            match event {
                ControlEvent::Raw(cc, _) => skip.cc_mask[ch] |= 1u128 << cc,
                ControlEvent::PitchBend(_) => skip.pitch_bend[ch] = true,
                ControlEvent::PitchBendSensitivity(_) => skip.pbs[ch] = true,
                ControlEvent::FineTune(_) => skip.fine_tune[ch] = true,
                ControlEvent::CoarseTune(_) => skip.coarse_tune[ch] = true,
                ControlEvent::ProgramChange(_) => skip.program[ch] = true,
                ControlEvent::PercussionMode(_) => {}
            }
        }
        skip
    }

    /// 应用 chase 通道状态快照（yinhe-audio 在 `ChaseResult` 到达时调用）。
    /// 与 CPU 路径 `channel_group.send_event` 对等：逐事件走通道状态机；
    /// damper 松开 / CC72/73 传播到当前活跃 voice（seek 后复活音符）。
    pub fn apply_chase(&mut self, dense: u32, events: &[ControlEvent]) {
        let Some(ch_idx) = dense_channel(dense as usize) else {
            return;
        };
        // 被修改的 voice 槽位（状态常驻 GPU，改完需写回）。
        let mut dirty: Vec<u32> = Vec::new();
        for &ev in events {
            let damper_released = self.channels[ch_idx].process_control(ev);
            if damper_released {
                // 松开延音踏板：释放该通道所有被保持的 voice（与 shader release 指令同语义）
                for (i, v) in self.voices.iter_mut().enumerate() {
                    if v.channel == dense as u8
                        && v.held_by_damper
                        && v.state.env_stage < 5
                        && !v.release_pending
                    {
                        v.release_pending = true;
                        v.state.env_start = v.state.envelope;
                        v.state.env_stage = 5;
                        v.state.stage_progress = 0.0;
                        dirty.push(i as u32);
                    }
                    v.held_by_damper = false;
                }
            }
            // CC72/73 修改包络时长、CC121 重置包络：直接写 voice 状态
            //（chase 不在渲染块内，无法发指令；块边界写入与指令帧效果一致）
            if is_env_effect_cc(&ev) {
                let ch = self.channels[ch_idx];
                for (i, v) in self.voices.iter_mut().enumerate() {
                    if v.channel as usize != ch_idx || v.state.env_stage >= 6 {
                        continue;
                    }
                    let (attack_frames, release_frames) =
                        Self::env_frames_for(&ch, v, self.sample_rate);
                    v.state.attack_frames = attack_frames;
                    v.state.release_frames = release_frames;
                    // 与 shader EnvUpdateCmd 的阶段重走规则一致
                    match v.state.env_stage {
                        0 | 2 => v.state.stage_progress = 0.0,
                        1 | 5 => {
                            v.state.env_start = v.state.envelope;
                            v.state.stage_progress = 0.0;
                        }
                        3 => {
                            v.state.decay_start = v.state.envelope;
                            v.state.stage_progress = 0.0;
                        }
                        _ => {}
                    }
                    dirty.push(i as u32);
                }
            }
        }
        // 写回被修改的槽位（chase 不频繁，逐个写可接受）。
        for vid in dirty {
            if let Some(v) = self.voices.get(vid as usize) {
                self.renderer.write_voice_state(vid, &v.state);
            }
        }
    }
}
