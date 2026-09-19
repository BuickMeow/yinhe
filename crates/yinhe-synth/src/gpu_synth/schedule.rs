//! 事件调度：块内事件收集为段结构，note on/off 与通道释放指令，chase 应用。

use crate::sf_parser;
use crate::synth::buffers::MAX_VOICE_SLOTS;
use crate::synth::{ChState, EnvUpdateCmd, GpuVoiceState, ReleaseCmd, SegInfo};

use super::{ControlEvent, GpuSynth, MAX_CHANNELS, SynthEvent};
use crate::channel_state::ChaseSkip;
use crate::channel_state::{ChannelState, dense_channel, is_env_effect_cc};

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
    /// 音符起始（绝对 sample；gate = end_sample - start，淘汰按短优先）
    pub(super) start_sample: u64,
    /// 已发 kill 但 GPU 尚未确认结束：仍需参与渲染让 1ms 淡出真正执行。
    /// （若直接从活跃列表排除，kill 指令不会被 pass1 应用，harvest 读回的
    /// GPU 旧状态还会覆盖 CPU 镜像 → voice 复活 + 硬切 click。）
    pub(super) kill_pending: bool,
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
    /// 槽位复用代号：harvest 状态回读的身份校验。读回状态属于"提交该块时的
    /// voice"——若槽位已被 note_on 复用为新 voice（代号不同），读回的是旧
    /// voice 的（可能已死亡）状态，必须丢弃而不是覆盖新 voice 的镜像
    /// （否则新 voice 被误回收 → 槽位被反复复用覆盖 → 正在响的声音消失）。
    pub(super) slot_gen: u32,
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
    /// 活跃 voice 的槽位索引（按通道分桶、每段重建；pass1/pass2 只遍历活跃）
    pub(super) active: Vec<u32>,
    /// 每通道区间 `[off, count]`（与 active 对应）
    pub(super) active_ranges: Vec<u32>,
    /// 活跃 voice 数（= active.len()）
    pub(super) active_count: u32,
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
        seg_base: u32,
        segs: &mut Vec<SegInfo>,
        ch_updates: &mut Vec<ChState>,
        releases: &mut Vec<ReleaseCmd>,
        env_cmds: &mut Vec<EnvUpdateCmd>,
    ) {
        // 本块内的 CC 位置一次收集（升序；段边界逐段消费）。
        // 逐段重扫全部事件是 O(事件数 × CC 数)，黑乐谱密集事件块下不可忽略；
        // event_cursor 处事件恒 >= block_start（块末 cursor 停在 >= block_end 处）。
        self.cc_scratch.clear();
        for b in &mut self.batch_buckets {
            b.clear();
        }
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
                        } => self.note_on(
                            channel,
                            key,
                            velocity,
                            end_sample,
                            block_start + block_frame as u64,
                            block_frame,
                            seg_base,
                            releases,
                        ),
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
            self.process_events_at(cc_sample, frame, seg_base, ch_updates, releases, env_cmds);
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
            if v.end_sample < block_start {
                // 过期未释放（end_sample 落在已过去的窗口：事件跳跃/边界错位）：
                // 立即释放。原逻辑直接 continue → 这类 voice 永远不再被扫描 →
                // 永久存活占槽位（低潮段 alive 很低却仍按满槽位渲染的元凶之一）。
                v.release_pending = true;
                releases.push(release_cmd(0, i));
                continue;
            }
            if v.end_sample >= block_end {
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

        // 段末统一做一次全局超限淘汰（原先每个音符一次，见 enforce_voice_limit）
        self.enforce_voice_limit(0, block_start, releases);
    }

    /// 处理段边界（同一 sample 位置）的所有事件：CC 更新通道状态并记录 ch_updates、
    /// 音符按偏移 0 分发（note_on 用段边界通道值快照）；damper 释放 / env 指令同发。
    fn process_events_at(
        &mut self,
        sample: u64,
        frame: u32,
        seg_base: u32,
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
                } => self.note_on(
                    channel, key, velocity, end_sample, sample, frame, seg_base, releases,
                ),
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
                                    // 与淘汰 kill 同语义：等待 GPU 确认 1ms 淡出结束，
                                    // 期间 harvest 不得用旧状态覆盖（否则 voice 复活）。
                                    v.kill_pending = true;
                                    let b = v.channel as usize * 128 + v.key as usize;
                                    self.layer_counts[b] = self.layer_counts[b].saturating_sub(1);
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
    #[allow(clippy::too_many_arguments)] // 渲染上下文透传，见 AGENTS 约定
    pub fn note_on(
        &mut self,
        channel: u8,
        key: u8,
        vel: u8,
        end_sample: u64,
        start_sample: u64,
        block_frame: u32,
        seg_base: u32,
        releases: &mut Vec<ReleaseCmd>,
    ) {
        // 音色库选择：dense 通道 → port → (bank, preset) 条目（与 xsynth
        // ChannelSoundfont::rebuild_matrix 一致：主选 + 兜底，落空静音）。
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        // 容量上限：**有空闲槽位（free list）时不算满**——结束的 voice 槽位
        // 由 harvest 即时回收、note_on 复用，不再依赖 compact 清墓碑。
        if self.voices.len() >= MAX_VOICE_SLOTS as usize && self.free_slots.is_empty() {
            use std::sync::atomic::Ordering::Relaxed;
            crate::gpu_synth::NOTE_ON_REJECTED.fetch_add(1, Relaxed);
            let bucket = match vel {
                0..=31 => &crate::gpu_synth::REJECT_VEL_LO,
                32..=63 => &crate::gpu_synth::REJECT_VEL_MID,
                _ => &crate::gpu_synth::REJECT_VEL_HI,
            };
            bucket.fetch_add(1, Relaxed);
            return;
        }
        let ch = self.channels[ch_idx];
        let entries = &self.port_key_maps[self.channel_port[ch_idx] as usize];
        let info = match sf_parser::select_key_info_multi(entries, ch.bank, ch.program, key, vel) {
            Some(i) => i,
            None => {
                // 选不到 region（如 key≥128 的扩展键）：静默，与 CPU 一致
                return;
            }
        };
        let (offset, length) = match self
            .sample_offsets
            .get(&(info.sample_data.as_ptr() as usize))
        {
            Some(&v) => v,
            None => {
                return;
            }
        };
        if length == 0 {
            return;
        }

        // 播放长度（绝对帧语义）：SF2 的 sample_end（xsynth LoopParams.stop）
        // 封顶，SFZ 到采样末尾。voice 的 time/idx 都是"样本内绝对帧"，
        // 与 loop_start/loop_end（绝对帧）一致——旧实现把 info.offset 同时
        // 加进 sample_offset 又保留 loop 回绕的绝对帧，造成双重偏移：
        // 带循环且 offset>0 的样本（如立体声钢琴库）回绕后读到静音区。
        let sample_length = match info.stop {
            Some(stop) => stop.min(length),
            None => length,
        };

        // 展开共享参数（speed/增益/声像/滤波/包络时长；公式唯一实现在
        // `voice_params::VoiceParams`，与 CPU 同源同值，CC72/73 修正也在其中）。
        let p = crate::voice_params::VoiceParams::from_key_info(info, &ch, self.sample_rate);

        // 完全重复 NoteOn 合批（对齐 CpuSynth `matches_batch`）：本块内创建的、
        // 尚未渲染的、参数完全相同的活跃 voice 直接 dup+1 复用（线性系统里
        // N 个同相位同参数 voice 之和 = 单个 ×N，无损）。黑乐谱重复 NoteOn
        // 常态化时省 voice 并对齐 CPU 能量（release 尾巴仅一份）。
        // 候选只扫本块新建的 voice（分桶），避免全表 O(voices) 扫描的热路径成本。
        let new_sample_offset = offset;
        let bucket = channel as usize * 128 + key as usize;
        let hit = self.batch_buckets[bucket]
            .iter()
            .rev()
            .find(|&&i| {
                let v = &self.voices[i as usize];
                v.velocity == vel
                    && v.end_sample == end_sample
                    && !v.kill_pending
                    && !v.release_pending
                    && !v.held_by_damper
                    && v.state.env_stage < 6
                    && v.state.sample_offset == new_sample_offset
                    && v.state.sample_length == sample_length
                    && v.state.speed == p.speed
                    && v.state.base_speed == p.base_speed
                    && v.state.start_offset == block_frame + seg_base
            })
            .copied();
        if let Some(i) = hit {
            let i = i as usize;
            self.voices[i].state.dup += 1;
            let st = self.voices[i].state;
            self.renderer.write_voice_state(i as u32, &st);
            crate::gpu_synth::BATCH_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        // 诊断：合批失配原因（宽松找同 (vel,end_sample) 的候选，记录第一失配字段）
        if crate::gpu_synth::PROBE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            use std::sync::atomic::Ordering::Relaxed;
            let miss = self.batch_buckets[bucket]
                .iter()
                .rev()
                .find(|&&i| {
                    let v = &self.voices[i as usize];
                    v.velocity == vel && v.end_sample == end_sample
                })
                .map(|&i| {
                    let v = &self.voices[i as usize];
                    if v.kill_pending {
                        0
                    } else if v.release_pending {
                        1
                    } else if v.held_by_damper {
                        2
                    } else if v.state.env_stage >= 6 {
                        3
                    } else if v.state.sample_offset != new_sample_offset {
                        4
                    } else if v.state.sample_length != sample_length {
                        5
                    } else if v.state.speed != p.speed {
                        6
                    } else if v.state.base_speed != p.base_speed {
                        7
                    } else if v.state.start_offset != block_frame + seg_base {
                        8
                    } else {
                        9
                    }
                })
                .unwrap_or(10);
            crate::gpu_synth::PROBE_BATCH_MISS[miss].fetch_add(1, Relaxed);
        }

        let voice = Voice {
            key,
            channel,
            start_sample,
            kill_pending: false,
            velocity: vel,
            end_sample,
            orig_attack_frames: p.orig_attack_frames,
            orig_release_frames: p.orig_release_frames,
            held_by_damper: false,
            release_pending: false,
            slot_gen: self.next_gen,
            state: GpuVoiceState {
                // offset 是拼接内的**元素**起点；info.offset 是**帧**偏移
                // → 立体声需 ×scale（此前直接相加，立体声样本起点错位）。
                // 拼接元素偏移（不含样本内起始偏移；起始偏移体现在 time 上）
                sample_offset: offset,
                sample_length,
                speed: p.speed,
                base_speed: p.base_speed,
                base_gain: p.base_gain,
                // 样本内绝对帧起点（xsynth 语义：播放位置 = 样本内绝对帧）
                time: info.offset as f32,
                // 块内帧：段内帧 + 段在块内的偏移。**创建时就换算**——否则复用
                // 槽位创建的 voice 会被"新增 voice 转换循环"漏掉（复用不改变
                // 数组长度），start_offset 停留在段内帧语义，导致提前数段渲染、
                // time 错误推进（越往后越乱的真身）。
                start_offset: block_frame + seg_base,
                // dense 通道号（note_on 已过滤 < MAX_CHANNELS）
                channel: channel as u32,
                envelope: p.env_start,
                env_stage: 0,
                stage_progress: 0.0,
                // envelope 归一化 0..1，增益由 gain 单独乘（xsynth 语义）
                env_level: 1.0,
                sustain_level: p.sustain_level,
                env_start: p.env_start,
                decay_start: p.env_start,
                delay_frames: p.delay_frames,
                attack_frames: p.attack_frames,
                hold_frames: p.hold_frames,
                decay_frames: p.decay_frames,
                release_frames: p.release_frames,
                base_pan_l: p.pan_l,
                base_pan_r: p.pan_r,
                loop_start: p.loop_start,
                loop_end: p.loop_end,
                loop_mode: p.loop_mode,
                is_stereo: p.is_stereo as u32,
                interp: p.interp,
                cutoff: info.cutoff,
                resonance: info.resonance,
                filter_type: crate::sf_parser::filter_type_code(info.filter_type),
                flt_b0: p.biquad.map(|b| b[0]).unwrap_or(0.0),
                flt_b1: p.biquad.map(|b| b[1]).unwrap_or(0.0),
                flt_b2: p.biquad.map(|b| b[2]).unwrap_or(0.0),
                flt_a1: p.biquad.map(|b| b[3]).unwrap_or(0.0),
                flt_a2: p.biquad.map(|b| b[4]).unwrap_or(0.0),
                flt_x1: 0.0,
                flt_x2: 0.0,
                flt_y1: 0.0,
                flt_y2: 0.0,
                flt_x1r: 0.0,
                flt_x2r: 0.0,
                flt_y1r: 0.0,
                flt_y2r: 0.0,
                dup: 1,
            },
        };
        self.next_gen = self.next_gen.wrapping_add(1);

        // 槽位分配：优先复用已结束 voice 的槽位（free list）；复用时**必须
        // 显式上传状态**（submit 的上传只覆盖本次新增的尾部区间）。原实现
        // 只能追加，墓碑累积顶到容量后 note_on 拒绝新音、且周期性 compact
        // 需排空流水线等待在途 GPU 块（实测 60-90ms）。
        let new_index = match self.free_slots.pop_front() {
            Some(slot) => {
                let slot = slot as usize;
                self.freed_flags[slot] = false;
                self.voices[slot] = voice;
                let st = self.voices[slot].state;
                self.renderer.write_voice_state(slot as u32, &st);
                slot
            }
            None => {
                self.voices.push(voice);
                self.freed_flags.push(false);
                self.voices.len() - 1
            }
        };

        self.batch_buckets[bucket].push(new_index as u32);

        // per-key layer 上限（SetLayerCount）：超限时反复杀该 key velocity 最低的
        // 未释放 voice（xsynth pop_quietest_voice_group 语义；跳过刚加入的）。
        // 活跃计数用增量维护的 `layer_counts`（O(1) 判定，替代每音一次 O(V)
        // 全扫——黑乐谱密集 NoteOn 下这是 CPU 侧最大热点）；仅超限时才扫描
        // 该 key 的 voice 选 victim。
        self.layer_counts[bucket] += 1;
        if let Some(max) = self.max_layers {
            while self.layer_counts[bucket] as usize > max {
                // 候选条件与计数条件一致（env_stage < 6）：**包含 release 中的**
                // voice——release 尾巴被截掉听感无害，这是 xsynth「几乎不丢音」
                // 的关键（判定与 CPU 共用 channel_state::layer_victim）。
                let victim = crate::channel_state::layer_victim(
                    self.voices.iter().enumerate().filter_map(|(i, v)| {
                        (v.channel == channel && v.key == key && v.state.env_stage < 6).then_some((
                            i,
                            v.velocity,
                            v.release_pending || v.state.env_stage == 5,
                            true,
                        ))
                    }),
                    new_index,
                );
                let Some(idx) = victim else {
                    break; // 其余已在 release 中，无候选
                };
                let v = &mut self.voices[idx];
                v.state.env_stage = 6;
                v.kill_pending = true;
                v.held_by_damper = false;
                releases.push(kill_cmd(block_frame, idx));
                self.layer_counts[bucket] -= 1;
                crate::gpu_synth::LAYER_KILLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// 全局 voice 超限淘汰（**每渲染段一次**）。
    ///
    /// 选择顺序（黑乐谱高潮段的听感关键，kiva 式"过载时牺牲小力度"）：
    /// 先按 velocity 升序（大力度音符永远最后、绝不错切），同力度下 release
    /// 中的优先（尾巴先于音头），再按 envelope 升序（更听不见的优先），
    /// 并列取创建顺序。淘汰量按活跃数（非墓碑）计算；跳过墓碑（等 compact
    /// 清理）；只预置 stage 6 + 发 kill，不 remove（本段指令索引保持稳定）。
    pub(super) fn enforce_voice_limit(
        &mut self,
        block_frame: u32,
        now_sample: u64,
        releases: &mut Vec<ReleaseCmd>,
    ) {
        // 待确认 kill 的 voice 已判死（GPU 淡出中）：不计入 alive、不作候选，
        // 否则 harvest 读回淡出中的旧状态会把它"复活"，下段重复淘汰同一 voice。
        let alive = self
            .voices
            .iter()
            .filter(|v| v.state.env_stage < 6 && !v.kill_pending)
            .count();
        let excess = alive.saturating_sub(self.max_voices);
        if excess == 0 {
            return;
        }
        // 排序键（用户定稿策略："小力度优先，其次 noteoff 优先"）：
        //   组序：release 中的优先——正在演奏的音符不先动（探针实测该曲
        //   可杀候选 100% 均为 release 尾巴，组序与组内键在真实数据上等价）；
        //   组内（两组同键）：① velocity 升序（小力度先死，大力度永远最后）；
        //   ② end_sample 升序（noteoff 早的先死，"结束最久"优先）；
        //   ③ envelope 升序（更听不见的优先）；④ 并列取创建顺序。
        let mut cands: Vec<(u8, f64, f64, f64, usize)> = Vec::with_capacity(alive);
        for (i, v) in self.voices.iter().enumerate() {
            if v.state.env_stage >= 6 || v.kill_pending {
                continue;
            }
            let releasing = v.release_pending || v.state.env_stage == 5;
            let end = v.end_sample;
            let env = v.state.envelope as f64;
            let vel = v.velocity as f64;
            let (k1, k2, k3) = (vel, end as f64, env);
            cands.push((if releasing { 0u8 } else { 1u8 }, k1, k2, k3, i));
        }
        // 只取最小的 excess 个：select_nth 是 O(n)，满排序是 O(n log n)；
        // 高潮段 excess 小（几百）而 alive 大（1.5 万+），每段省的比较量可观。
        // 被选中的集合与排序无关（kill 顺序不影响听感），无需全序。
        let cmp = |a: &(u8, f64, f64, f64, usize), b: &(u8, f64, f64, f64, usize)| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.total_cmp(&b.2))
                .then(a.3.total_cmp(&b.3))
                .then(a.4.cmp(&b.4))
        };
        if excess < cands.len() {
            cands.select_nth_unstable_by(excess, cmp);
        } else {
            cands.sort_unstable_by(cmp);
        }
        for &(rel, k1, _, _, idx) in cands.iter().take(excess) {
            let vel = if rel == 0 {
                k1 as u8
            } else {
                self.voices[idx].velocity
            };
            let bucket = match vel {
                0..=31 => &crate::gpu_synth::EVICT_VEL_LO,
                32..=63 => &crate::gpu_synth::EVICT_VEL_MID,
                _ => &crate::gpu_synth::EVICT_VEL_HI,
            };
            bucket.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.kill_voice_for_evict(block_frame, idx, releases);
        }

        // 反事实探针（诊断）：同一批候选分别按两种策略取 victim 统计特征，
        // 不改变上面的实际淘汰选择。用于用真实数据决定排序键。
        if crate::gpu_synth::PROBE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            use std::sync::atomic::Ordering::Relaxed;
            crate::gpu_synth::PROBE_EVENTS.fetch_add(1, Relaxed);
            let sr = self.sample_rate as f64;
            // "结束最久优先"：已结束的优先，组内 end_sample 升序（最早结束的先杀）
            let mut probe_end: Vec<(bool, u64, u8, usize)> = cands
                .iter()
                .map(|c| {
                    let v = &self.voices[c.4];
                    (v.end_sample > now_sample, v.end_sample, v.velocity, c.4)
                })
                .collect();
            probe_end.sort_unstable();
            for &(not_ended, end, _, _) in probe_end.iter().take(excess) {
                let secs = (now_sample as i64 - end as i64) as f64 / sr;
                let bucket = match secs {
                    s if s < -2.0 => 0,
                    s if s < -0.5 => 1,
                    s if s < -0.1 => 2,
                    s if s < 0.0 => 3,
                    s if s < 0.1 => 4,
                    s if s < 0.5 => 5,
                    s if s < 2.0 => 6,
                    _ => 7,
                };
                crate::gpu_synth::PROBE_END_AGE[bucket].fetch_add(1, Relaxed);
                if !not_ended {
                    crate::gpu_synth::PROBE_END_RELEASED.fetch_add(1, Relaxed);
                }
            }
            // "力度优先"：力度升序取前 excess
            let mut probe_vel: Vec<(u8, u64, usize)> = cands
                .iter()
                .map(|c| {
                    let v = &self.voices[c.4];
                    (v.velocity, v.end_sample, c.4)
                })
                .collect();
            probe_vel.sort_unstable();
            for &(vel, _, _) in probe_vel.iter().take(excess) {
                crate::gpu_synth::PROBE_VEL16[(vel as usize / 8).min(15)].fetch_add(1, Relaxed);
            }
        }
    }

    fn kill_voice_for_evict(
        &mut self,
        block_frame: u32,
        idx: usize,
        releases: &mut Vec<ReleaseCmd>,
    ) {
        let v = &mut self.voices[idx];
        v.state.env_stage = 6;
        v.kill_pending = true;
        v.held_by_damper = false;
        let b = v.channel as usize * 128 + v.key as usize;
        self.layer_counts[b] = self.layer_counts[b].saturating_sub(1);
        releases.push(kill_cmd(block_frame, idx));
        crate::gpu_synth::EVICT_KILLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        // 跳过已 held 的 voice（xsynth damper 分支只匹配 "isn't being held" 的
        // voice：否则同 key 多个 off 会重复匹配同一个 held voice，其余 voice 永不释放）
        let hit = self.voices.iter().position(|v| {
            v.channel == channel
                && v.key == key
                && v.state.env_stage < 5
                && !v.held_by_damper
                && !v.release_pending
        });
        let Some(i) = hit else {
            return;
        };
        let v = &mut self.voices[i];
        // 合批 voice：每个 NoteOff 只消耗一个引用，归 1 才真正释放（与 CpuSynth 一致）
        if v.state.dup > 1 {
            v.state.dup -= 1;
            let st = v.state;
            self.renderer.write_voice_state(i as u32, &st);
        } else if damper {
            v.held_by_damper = true;
        } else {
            v.release_pending = true;
            releases.push(release_cmd(frame, i));
        }
    }

    /// CC72/73 重算单个 voice 的 attack/release 帧数（基于 region 原始值，多次 CC 不累积）。
    fn env_frames_for(ch: &ChannelState, v: &Voice, sample_rate: u32) -> (f32, f32) {
        // 共享实现：voice_params::env_frames_for（此前 CPU/GPU 各一份）
        crate::voice_params::env_frames_for(
            ch,
            v.orig_attack_frames,
            v.orig_release_frames,
            sample_rate,
        )
    }

    /// 计算 seek 后已处理的控制事件跳过掩码（事件区间 [chase_base, event_cursor)）。
    /// 由 yinhe-audio 在 `ChaseResult` 到达时调用，跳过的控制器不再被 chase 覆盖。
    pub fn chase_skip(&self) -> ChaseSkip {
        crate::channel_state::chase_skip(&self.events[self.chase_base..self.event_cursor])
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
