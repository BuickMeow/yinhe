//! 事件派发与 voice 生命周期：note_on/off、控制事件、layer/全局淘汰。
//!
//! 拆自 cpu_synth.rs（文件过长）。`dispatch_event`/`evict_excess`/
//! `process_control_channel` 被主文件（渲染与 chase 路径）调用，标
//! `pub(super)`；其余仅本模块内使用。

use super::*;

impl CpuSynth {
    /// 事件派发（帧内；`frame` = 块内帧偏移，`block_start_abs` = 块首绝对位置）。
    pub(super) fn dispatch_event(&mut self, ev: &SynthEvent, frame: u32, block_start_abs: u64) {
        match ev {
            SynthEvent::NoteOn {
                channel,
                key,
                velocity,
                end_sample,
                ..
            } => self.note_on(
                *channel,
                *key,
                *velocity,
                *end_sample,
                frame,
                block_start_abs,
            ),
            SynthEvent::NoteOff { channel, key, .. } => self.note_off(*channel, *key),
            SynthEvent::Control { channel, event, .. } => {
                self.process_control_channel(*channel, *event, frame, block_start_abs);
            }
        }
    }

    /// NoteOn：从 key map 快照创建 voice；超限淘汰最老的 release 中 voice。
    fn note_on(
        &mut self,
        channel: u8,
        key: u8,
        vel: u8,
        end_sample: u64,
        frame: u32,
        block_start_abs: u64,
    ) {
        let t_prof = std::time::Instant::now();
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let ch = self.channels[ch_idx];
        let entries = self.port_key_maps[ch_idx].as_slice();
        let t_sel = std::time::Instant::now();
        let Some(info) = sf_parser::select_key_info_multi(entries, ch.bank, ch.program, key, vel)
        else {
            return;
        };
        PROF_ON_SELECT_NS.fetch_add(
            t_sel.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        // 完全重复合批：先在最近创建的活跃 voice 里找同参数者（黑乐谱重复
        // NoteOn 常态，tau 峰值段实测完全重复冗余 57.9%）。命中则只把该批次
        // 的引用数 +1，不新建 voice；线性系统里两者数学等价。
        let slot = Self::key_slot(channel, key);
        let speed = info.speed_mult * ch.pitch_multiplier();
        let scan = self.key_indices[slot].len().min(16);
        let hit = self.key_indices[slot]
            .iter()
            .rev()
            .take(scan)
            .find(|&&i| {
                self.voices[i as usize].matches_batch(
                    &info.sample_data,
                    info.offset,
                    info.speed_mult,
                    speed,
                    vel,
                    end_sample,
                    frame,
                )
            })
            .copied();
        if let Some(i) = hit {
            self.voices[i as usize].absorb();
            PROF_NOTE_ON_NS.fetch_add(
                t_prof.elapsed().as_nanos() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            return;
        }
        let t_new = std::time::Instant::now();
        let voice = CpuVoice::new(
            info,
            channel,
            key,
            vel,
            end_sample,
            frame,
            self.sample_rate,
            &ch,
            block_start_abs,
        );
        PROF_ON_NEW_NS.fetch_add(
            t_new.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        let t_push = std::time::Instant::now();
        let new_index = self.voices.len();
        self.voices.push(voice);
        self.key_indices[slot].push(new_index as u32);
        PROF_ON_PUSH_NS.fetch_add(
            t_push.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );

        // per-key layer 上限：超限时按 xsynth 语义杀该 key velocity 最低的 voice
        //（跳过刚加入的，保证新音符发声）。
        if let Some(max) = self.max_layers {
            self.enforce_key_layers(slot, max, new_index);
        }

        PROF_NOTE_ON_NS.fetch_add(
            t_prof.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// 该 key 活跃 voice 超过 `max` 时，反复杀 velocity 最低的
    /// （xsynth `pop_quietest_voice_group` 语义；`keep` = 刚加入的索引不参与）。
    /// 经 `key_indices` 只扫该 key 的 voice（O(layer)，非 O(V)）。
    ///
    /// 候选**不排除已 release 的 voice**：xsynth 的 `pop_quietest_voice_group`
    /// 只排除 killed，releasing 的 voice 同样在 `buffer` 里参与淘汰——且它们
    /// 创建最早，velocity 并列时优先被杀。release 尾巴被截掉听感无害，这是
    /// xsynth「几乎不丢音」的关键；只杀在响 voice 会造成明显丢音。
    fn enforce_key_layers(&mut self, slot: usize, max: usize, keep: usize) {
        loop {
            // 先 O(layer) 数活跃数；未超限直接返回（多数 note_on 不分配、不扫候选）
            let active = self.key_indices[slot]
                .iter()
                .filter(|&&i| {
                    let v = &self.voices[i as usize];
                    !v.finished() && !v.is_killed()
                })
                .count();
            if active <= max {
                return;
            }
            // 超限（罕见）：velocity 最低的候选（含 release 中；并列取最早；
            // 判定与 GPU 共用 channel_state::layer_victim，避免再次分叉）
            let victim = crate::channel_state::layer_victim(
                self.key_indices[slot].iter().map(|&i| {
                    let v = &self.voices[i as usize];
                    (
                        i as usize,
                        v.velocity,
                        v.released,
                        !v.finished() && !v.is_killed(),
                    )
                }),
                keep,
            );
            let Some(victim) = victim else {
                return;
            };
            // 1ms 淡出（硬切会产生 click，用户实测）。
            self.voices[victim].signal_kill(self.sample_rate);
            crate::cpu_synth::LAYER_KILLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// 全局 voice 超限淘汰（块末调用）：优先 release 中的，不足时按创建顺序
    /// 杀最老的。立即结束（与 xsynth 的默认 kill 语义一致），块末统一 retain 回收。
    pub(super) fn evict_excess(&mut self, excess: usize) {
        // 与 GPU 同语义：**release 中优先**（envelope 越接近无声越先回收），
        // 正在演奏的（无论长短力度）最后动；同为演奏中时小力度先牺牲。
        let mut cands: Vec<(u8, f32, u8, usize)> = Vec::new();
        for (i, v) in self.voices.iter().enumerate() {
            if v.finished() || v.is_killed() {
                continue;
            }
            cands.push((
                if v.released { 0u8 } else { 1u8 },
                v.envelope,
                v.velocity,
                i,
            ));
        }
        cands.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        for &(_, _, _, i) in cands.iter().take(excess) {
            self.voices[i].signal_kill(self.sample_rate);
        }
    }

    /// NoteOff：释放该 (channel, key) 最老的未释放 voice（延音踏板按住时只标记）。
    fn note_off(&mut self, channel: u8, key: u8) {
        let t_prof = std::time::Instant::now();
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper = self.channels[ch_idx].damper;
        let slot = Self::key_slot(channel, key);
        // 索引表按创建顺序：正向找第一个未释放 = 最老未释放（O(layer)）
        let idxs = std::mem::take(&mut self.key_indices[slot]);
        for &i in idxs.iter() {
            let i = i as usize;
            let v = &mut self.voices[i];
            if !v.finished() && !v.released && !v.held_by_damper {
                // 合批 voice：每个 NoteOff 只消耗一个引用，归 1 才真正释放
                if v.dup > 1 {
                    v.dup -= 1;
                } else if damper {
                    v.held_by_damper = true;
                } else {
                    v.signal_release(ENV_RELEASE);
                }
                break;
            }
        }
        self.key_indices[slot] = idxs;
        PROF_NOTE_OFF_NS.fetch_add(
            t_prof.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// 控制事件：更新通道状态并把变化传播到该通道的活跃 voice。
    pub(super) fn process_control_channel(
        &mut self,
        channel: u8,
        event: ControlEvent,
        frame: u32,
        block_start_abs: u64,
    ) {
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper_released = self.channels[ch_idx].process_control(event);
        if damper_released {
            // 松开延音踏板：释放被保持的 voice（与 GpuSynth apply_chase 同语义）
            for v in self.voices.iter_mut() {
                if v.channel == channel && v.held_by_damper && !v.finished() {
                    v.held_by_damper = false;
                    v.signal_release(ENV_RELEASE);
                }
            }
        }
        // 弯音/调音变化：更新该通道活跃 voice 的速度（含 time 校正）。
        // 原生 RPN 中 0/1/2 是弯音灵敏度/微调/粗调，同样影响音高。
        if matches!(
            event,
            ControlEvent::PitchBend(_)
                | ControlEvent::PitchBendSensitivity(_)
                | ControlEvent::FineTune(_)
                | ControlEvent::CoarseTune(_)
                | ControlEvent::Rpn {
                    parameter: 0..=2,
                    ..
                }
        ) {
            let mult = self.channels[ch_idx].pitch_multiplier();
            for v in self.voices.iter_mut() {
                if v.channel == channel && !v.finished() {
                    v.set_speed(mult, frame, block_start_abs);
                }
            }
        }
        // CC72/73/121：重算活跃 voice 的包络时长
        if is_env_effect_cc(&event) {
            let ch = self.channels[ch_idx];
            let sr = self.sample_rate;
            for v in self.voices.iter_mut() {
                if v.channel == channel && !v.finished() {
                    v.apply_env_update(&ch, sr);
                }
            }
        }
    }
}
