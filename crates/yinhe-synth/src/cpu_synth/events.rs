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
            SynthEvent::NoteOff { channel, key, .. } => {
                self.note_off(*channel, *key, frame, block_start_abs)
            }
            SynthEvent::Control { channel, event, .. } => {
                self.process_control_channel(*channel, *event, frame, block_start_abs);
            }
        }
    }

    /// NoteOn：从 key map 快照创建 voice（该 (key,vel) 的每个 region 层一个
    /// voice，同组号绑定生命周期）；超限淘汰最老的 release 中组。
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
        // SFZ keyswitch：落在开关区间的键不发声，只记录 `sw_last`。
        // 必须在 select 之前处理，否则开关键会按普通音触发。
        if self.update_keyswitch(ch_idx, key) {
            return;
        }
        let ch = self.channels[ch_idx];
        let maps = Arc::clone(&self.port_key_maps[ch_idx]);
        let entries = maps.as_slice();
        let t_sel = std::time::Instant::now();
        let Some(sel) =
            sf_parser::select_layers_multi(entries, ch.bank, ch.bank_lsb, ch.program, key, vel)
        else {
            return;
        };
        PROF_ON_SELECT_NS.fetch_add(
            t_sel.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        // 轮次计数（round-robin）：每 key 每次 note_on +1，先取后加。
        let slot = Self::key_slot(channel, key);
        let seq_counter = self.seq_counters[slot];
        self.seq_counters[slot] = seq_counter.wrapping_add(1);
        let rand = sf_parser::extra::hash_rand(channel, key, seq_counter);
        let gate_ctx = sf_parser::GateCtx {
            cc: &ch.cc,
            sw_last: ch.sw_last,
            seq_counter,
            rand,
        };
        let infos = sel.infos;
        let extras = sel.extras;
        let group = self.next_group;
        self.next_group = self.next_group.wrapping_add(1);
        // 完全重复合批：单层时先在最近创建的活跃 voice 里找同参数者（黑乐谱重复
        // NoteOn 常态，tau 峰值段实测完全重复冗余 57.9%）。多层组不参与合批——
        // 各层独立命中会让组内层数不一致，note_off 整组释放会拆散音色。
        let batchable = infos.len() == 1;
        let mut spawned = 0usize;
        for info in infos {
            // SF2 exclusiveClass：同通道同类新音符掐掉旧 voice（鼓组闭镲/开镲互斥）。
            if info.exclusive_class != 0 {
                self.cut_exclusive(channel, info.exclusive_class);
            }
            // SFZ 门控（xfin/xfout/on_cc/seq/sw/rand）：不命中则整层不发声。
            // pitch_random 以音分形式并入播放倍率（确定性，离线=实时）。
            let extra = info.extra_index().and_then(|i| extras.get(i));
            // 只改 region 基础倍率；通道弯音/调音倍率由 voice 内部乘上，
            // 不能被门控覆盖（否则 CPU 会丢弯音、与 GPU 不一致）。
            let mut gate_speed_scale = 1.0f32;
            let mut info_volume = info.volume;
            if let Some(extra) = extra {
                let Some(gate) = sf_parser::evaluate(extra, key, &gate_ctx) else {
                    continue;
                };
                info_volume *= gate.gain;
                if gate.pitch_cents != 0.0 {
                    gate_speed_scale = 2.0f32.powf(gate.pitch_cents / 1200.0);
                }
                if extra.off_by != 0 {
                    self.cut_group(extra.off_by, group);
                }
            }
            if batchable {
                let speed = info.speed_mult * gate_speed_scale * ch.pitch_multiplier();
                let scan = self.key_indices[slot].len().min(16);
                let hit = self.key_indices[slot]
                    .iter()
                    .rev()
                    .take(scan)
                    .find(|&&i| {
                        self.voices[i as usize].matches_batch(
                            &info.sample_data,
                            info.offset,
                            info.speed_mult * gate_speed_scale,
                            speed,
                            vel,
                            end_sample,
                            frame,
                        )
                    })
                    .copied();
                if let Some(i) = hit {
                    self.voices[i as usize].absorb();
                    continue;
                }
            }
            let t_new = std::time::Instant::now();
            let mut voice = CpuVoice::new(
                info,
                channel,
                key,
                vel,
                group,
                end_sample,
                frame,
                self.sample_rate,
                &ch,
                block_start_abs,
            );
            // 门控增益/音高：只缩放 region 基础倍率（不动通道弯音倍率）
            voice.apply_gate(gate_speed_scale, info_volume);
            if let Some(extra) = extra {
                voice.one_shot = extra.one_shot;
                voice.off_group = extra.group;
                voice.apply_lfo(extra);
            }
            PROF_ON_NEW_NS.fetch_add(
                t_new.elapsed().as_nanos() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            let new_index = self.voices.len();
            self.voices.push(voice);
            self.key_indices[slot].push(new_index as u32);
            spawned += 1;
        }

        // per-key layer 上限：超限时按 xsynth 语义杀最弱的整组（跳过刚加入的组）。
        if spawned > 0
            && let Some(max) = self.max_layers
        {
            self.enforce_key_layers(slot, max, group);
        }

        PROF_NOTE_ON_NS.fetch_add(
            t_prof.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// SFZ `off_by`：新音触发时掐掉同组的旧音（鼓组/断音互斥）。
    ///
    /// 只在该 region 声明了 `off_by` 时调用，热路径不扫描。组标签在 SFZ 里是
    /// 全局的（跨 channel 生效），因此按 voice 的 `off_group` 扫描。
    fn cut_group(&mut self, off_by: u16, keep_group: u32) {
        for i in 0..self.voices.len() {
            let v = &self.voices[i];
            if v.off_group == off_by && v.group != keep_group && !v.finished() && !v.is_killed() {
                self.voices[i].signal_kill(self.sample_rate);
            }
        }
    }

    /// SF2 `exclusiveClass`：同通道（同 class）的新音符掐掉旧 voice。
    ///
    /// 只在 region 声明了 exclusive class 时调用（鼓组），热路径不扫描；
    /// 与 SFZ `off_by` 的区别是 **按通道** 隔离（SF2 语义）。
    fn cut_exclusive(&mut self, channel: u8, class: u8) {
        for i in 0..self.voices.len() {
            let v = &self.voices[i];
            if v.channel == channel && v.exclusive_class == class && !v.finished() && !v.is_killed()
            {
                self.voices[i].signal_kill(self.sample_rate);
            }
        }
    }

    /// 把音色库的 `<control> set_ccN=v` 写进各通道的初始 CC。
    ///
    /// 必需：Starry 的琴键/锤/踏板三层全部由 `xfin_locc14/15/20` 门控，
    /// 初始 CC 为 0 时三层都会静音（或只剩一层），用户调 CC 也无效。
    pub(super) fn apply_cc_defaults(&mut self) {
        for (ch_idx, defaults) in self.channel_cc_defaults.iter().enumerate() {
            for &(cc, v) in defaults {
                self.channels[ch_idx].set_cc_default(cc, v);
            }
        }
    }

    /// SFZ keyswitch：`sw_lokey..sw_hikey` 区间内的键只切换 `sw_last` 不发声。
    ///
    /// 返回 true 表示该键是开关键、已吞掉。区间在加载音色库时缓存
    /// （`sw_ranges`），热路径只做最多几次区间比较。
    fn update_keyswitch(&mut self, ch_idx: usize, key: u8) -> bool {
        let hit = self.sw_ranges[ch_idx]
            .iter()
            .any(|&(lo, hi)| key >= lo && key <= hi);
        if hit {
            self.channels[ch_idx].sw_last = key;
        }
        hit
    }

    /// SFZ `trigger=release`：扫描块内到期（`end_sample` 落在本块）的音符，
    /// 为其创建释放层。
    ///
    /// 引擎事件表不生成 NoteOff（由 `end_sample` 取代），所以释放层只能在这里
    /// 按到期声音补齐。`release_spawned` 保证一个音符只触发一次。
    pub(super) fn spawn_due_release_layers(&mut self, block_start: u64, frames: usize) {
        let block_end = block_start + frames as u64;
        // 先收集（避免迭代 voices 时 push 触发 Vec 重分配/借用冲突）。
        // 同一组只取第一条：同一次 NoteOn 的多层共享 end_sample，重复触发会
        // 叠出多份释放声。
        let mut due: Vec<(u8, u8, u8, u32, u32)> = Vec::new();
        let mut seen: Vec<u32> = Vec::new();
        for v in &self.voices {
            if v.is_release_trigger
                || v.release_spawned
                || v.finished()
                || v.killed
                || v.end_sample < block_start
                || v.end_sample >= block_end
                || seen.contains(&v.group)
            {
                continue;
            }
            seen.push(v.group);
            due.push((
                v.channel,
                v.key,
                v.velocity,
                (v.end_sample - block_start) as u32,
                v.group,
            ));
        }
        for (channel, key, vel, frame, group) in due {
            for v in self.voices.iter_mut() {
                if v.group == group {
                    v.release_spawned = true;
                }
            }
            self.spawn_release_layers(channel, key, vel, frame, block_start);
        }
    }

    /// 该 key 活跃 voice 超过 `max` 时，反复杀 velocity 最低的**组**    /// （xsynth `pop_quietest_voice_group` 语义；`keep_group` = 刚加入的组不参与）。
    /// 经 `key_indices` 只扫该 key 的 voice（O(layer)，非 O(V)）。
    ///
    /// 候选**不排除已 release 的组**：xsynth 的 `pop_quietest_voice_group`
    /// 只排除 killed，releasing 的 voice 同样在 `buffer` 里参与淘汰——且它们
    /// 创建最早，velocity 并列时优先被杀。release 尾巴被截掉听感无害，这是
    /// xsynth「几乎不丢音」的关键；只杀在响 voice 会造成明显丢音。
    fn enforce_key_layers(&mut self, slot: usize, max: usize, keep_group: u32) {
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
            let idxs = std::mem::take(&mut self.key_indices[slot]);
            // 同组 voice 在索引表里连续（NoteOn 循环 push）：按组聚合选最弱组。
            // 排序与旧单 voice 语义一致：release 中的组优先 → 力度低 → 最早。
            let mut best: Option<(u8, u8, usize, u32)> = None;
            let mut i = 0;
            while i < idxs.len() {
                let group = self.voices[idxs[i] as usize].group;
                let (mut any_alive, mut all_released, mut vel, mut first) =
                    (false, true, u8::MAX, usize::MAX);
                while i < idxs.len() && self.voices[idxs[i] as usize].group == group {
                    let vi = idxs[i] as usize;
                    let v = &self.voices[vi];
                    if !v.finished() && !v.is_killed() {
                        any_alive = true;
                        all_released &= v.released;
                        vel = vel.min(v.velocity);
                        first = first.min(vi);
                    }
                    i += 1;
                }
                if any_alive
                    && group != keep_group
                    && best.is_none_or(|b| (u8::from(!all_released), vel, first, group) < b)
                {
                    best = Some((u8::from(!all_released), vel, first, group));
                }
            }
            let Some((_, _, _, victim_group)) = best else {
                self.key_indices[slot] = idxs;
                return;
            };
            let mut killed = 0u64;
            for &i in idxs.iter() {
                let v = &mut self.voices[i as usize];
                if v.group == victim_group && !v.finished() && !v.is_killed() {
                    // 1ms 淡出（硬切会产生 click，用户实测）。
                    v.signal_kill(self.sample_rate);
                    killed += 1;
                }
            }
            self.key_indices[slot] = idxs;
            if killed == 0 {
                return;
            }
            crate::cpu_synth::LAYER_KILLS.fetch_add(killed, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// 全局 voice 超限淘汰（块末调用）：按**组**淘汰（同一次 NoteOn 的多层
    /// 整组消失，避免鼓只剩左声道这类音色突变）。优先 release 中的组，
    /// 其次 envelope 更低、力度更小、触发更早的组。块末统一 retain 回收。
    pub(super) fn evict_excess(&mut self, excess: usize) {
        if excess == 0 {
            return;
        }
        // 先按组聚合（voice 由 note_on 循环 push，同组相邻；合批 voice 沿用旧组）。
        // 元组：(组号, 未 release, 组内最小 envelope, 力度, 组首索引)
        let mut groups: Vec<(u32, u8, f32, u8, usize)> = Vec::new();
        for (i, v) in self.voices.iter().enumerate() {
            if v.finished() || v.is_killed() {
                continue;
            }
            groups.push((v.group, u8::from(!v.released), v.envelope, v.velocity, i));
        }
        groups.sort_unstable_by_key(|g| g.0);
        // 聚合后：(组号, 未 release, 最小 envelope, 力度, 首索引, voice 数)
        let mut agg: Vec<(u32, u8, f32, u8, usize, usize)> = Vec::new();
        for g in groups {
            if let Some(last) = agg.last_mut()
                && last.0 == g.0
            {
                last.1 = last.1.max(g.1);
                last.2 = last.2.min(g.2);
                last.3 = last.3.min(g.3);
                last.4 = last.4.min(g.4);
                last.5 += 1;
            } else {
                agg.push((g.0, g.1, g.2, g.3, g.4, 1));
            }
        }
        agg.sort_unstable_by(|a, b| {
            a.1.cmp(&b.1)
                .then(a.2.total_cmp(&b.2))
                .then(a.3.cmp(&b.3))
                .then(a.4.cmp(&b.4))
        });
        // 选最弱的组，直到覆盖 excess 个 voice（整组淘汰可能多杀几层）。
        let mut victims: Vec<u32> = Vec::new();
        let mut killed = 0usize;
        for &(group, _, _, _, _, count) in agg.iter() {
            if killed >= excess {
                break;
            }
            victims.push(group);
            killed += count;
        }
        if victims.is_empty() {
            return;
        }
        victims.sort_unstable();
        for v in self.voices.iter_mut() {
            if !v.finished() && !v.is_killed() && victims.binary_search(&v.group).is_ok() {
                v.signal_kill(self.sample_rate);
            }
        }
    }

    /// NoteOff：释放该 (channel, key) 最老的未释放**层组**（延音踏板按住时只
    /// 标记）。同一次 NoteOn 展开的所有层共享组号，必须整组释放，否则鼓的
    /// L/R 层会被拆开（右声道提前消失）。
    ///
    /// `one_shot` 层（SFZ `loop_mode=one_shot`）忽略 NoteOff：踏板/琴键噪声要
    /// 播到采样末尾，否则会被硬切。
    fn note_off(&mut self, channel: u8, key: u8, frame: u32, block_start_abs: u64) {
        let t_prof = std::time::Instant::now();
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper = self.channels[ch_idx].damper;
        let slot = Self::key_slot(channel, key);
        // 索引表按创建顺序：正向找第一个未释放 = 最老未释放（O(layer)）
        let idxs = std::mem::take(&mut self.key_indices[slot]);
        let group = idxs.iter().find_map(|&i| {
            let v = &self.voices[i as usize];
            (!v.finished() && !v.released && !v.held_by_damper).then_some(v.group)
        });
        if let Some(group) = group {
            for &i in idxs.iter() {
                let i = i as usize;
                let v = &mut self.voices[i];
                if v.group != group || v.finished() || v.released || v.held_by_damper {
                    continue;
                }
                if v.one_shot {
                    continue;
                }
                // 合批 voice：每个 NoteOff 只消耗一个引用，归 1 才真正释放
                if v.dup > 1 {
                    v.dup -= 1;
                } else if damper {
                    v.held_by_damper = true;
                } else {
                    v.signal_release(ENV_RELEASE);
                }
            }
            // SFZ `trigger=release`：音符结束才发声（钢琴踏板/键释放噪声）。
            self.spawn_release_layers(
                channel,
                key,
                self.velocity_of(group),
                frame,
                block_start_abs,
            );
        }
        self.key_indices[slot] = idxs;
        PROF_NOTE_OFF_NS.fetch_add(
            t_prof.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// 该组首次 NoteOn 的力度（释放层沿用；找不到返回 0 = 用默认）。
    fn velocity_of(&self, group: u32) -> u8 {
        self.voices
            .iter()
            .find(|v| v.group == group)
            .map(|v| v.velocity)
            .unwrap_or(0)
    }

    /// 创建 `trigger=release` 层（该 (channel, key) 的全部释放 region 各一个 voice）。
    ///
    /// 门控（`on_locc64` 等）在**释放时刻**求值——踏板噪声按当时的踏板状态选层，
    /// 这是 SFZ 的语义，也是它比 note_on 时刻求值更正确的地方。
    fn spawn_release_layers(
        &mut self,
        channel: u8,
        key: u8,
        vel: u8,
        frame: u32,
        block_start_abs: u64,
    ) {
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let ch = self.channels[ch_idx];
        let maps = Arc::clone(&self.port_key_maps[ch_idx]);
        let entries = maps.as_slice();
        let Some(sel) = sf_parser::select_release_layers_multi(
            entries,
            ch.bank,
            ch.bank_lsb,
            ch.program,
            key,
            vel,
        ) else {
            return;
        };
        let rand = sf_parser::extra::hash_rand(
            channel,
            key,
            self.seq_counters[ch_idx * 128 + key as usize],
        );
        let gate_ctx = sf_parser::GateCtx {
            cc: &ch.cc,
            sw_last: ch.sw_last,
            seq_counter: 0,
            rand,
        };
        let group = self.next_group;
        self.next_group = self.next_group.wrapping_add(1);
        let slot = Self::key_slot(channel, key);
        for info in sel.infos {
            let extra = info.extra_index().and_then(|i| sel.extras.get(i));
            let mut speed_scale = 1.0f32;
            let mut volume = info.volume;
            if let Some(extra) = extra {
                let Some(gate) = sf_parser::evaluate(extra, key, &gate_ctx) else {
                    continue;
                };
                volume *= gate.gain;
                if gate.pitch_cents != 0.0 {
                    speed_scale = 2.0f32.powf(gate.pitch_cents / 1200.0);
                }
            }
            let mut voice = CpuVoice::new(
                info,
                channel,
                key,
                if vel == 0 { info.lovel } else { vel },
                group,
                u64::MAX,
                frame,
                self.sample_rate,
                &ch,
                block_start_abs,
            );
            voice.apply_gate(speed_scale, volume);
            voice.is_release_trigger = true;
            voice.release_spawned = true;
            if let Some(extra) = extra {
                voice.one_shot = extra.one_shot;
                voice.off_group = extra.group;
                voice.apply_lfo(extra);
            }
            let idx = self.voices.len();
            self.voices.push(voice);
            self.key_indices[slot].push(idx as u32);
        }
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
        let action = self.channels[ch_idx].process_control(event);
        // CC120 All Sound Off：立即结束该通道全部 voice（1ms 淡出）。
        if action.all_sound_off {
            for v in self.voices.iter_mut() {
                if v.channel == channel && !v.finished() && !v.is_killed() {
                    v.signal_kill(self.sample_rate);
                }
            }
        }
        // CC123 All Notes Off：释放非踏板保持的 voice（held 等踏板松开）。
        if action.all_notes_off {
            for v in self.voices.iter_mut() {
                if v.channel == channel && !v.finished() && !v.held_by_damper {
                    v.signal_release(ENV_RELEASE);
                }
            }
        }
        if action.damper_released {
            // 松开延音踏板：释放被保持的 voice（与 GpuSynth apply_chase 同语义），
            // 并为其补上 SFZ `trigger=release` 层（踏板噪声正是在这一刻发声）。
            let mut held: Vec<(u8, u8, u8)> = Vec::new();
            for v in self.voices.iter_mut() {
                if v.channel == channel && v.held_by_damper && !v.finished() {
                    v.held_by_damper = false;
                    v.signal_release(ENV_RELEASE);
                    if !v.is_release_trigger && !v.release_spawned {
                        v.release_spawned = true;
                        held.push((v.key, v.velocity, 0));
                    }
                }
            }
            for (key, vel, _) in held {
                self.spawn_release_layers(channel, key, vel, frame, block_start_abs);
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
        // CC71/74：按新值重算活跃 voice 的滤波器系数。
        // 延迟到段边界由 `apply_filter_dirty` 统一消费（同一段内多次 CC 只算一次）。
        let _ = action.filter_changed;
    }

    /// 消费 CC71/74 的脏标记：对每个变化过的通道重算活跃 voice 的滤波器。
    /// 每段边界调用一次（同段内多次 CC 只重算一次，避免 O(V) × 事件数）。
    pub(super) fn apply_filter_dirty(&mut self) {
        for ch_idx in 0..MAX_CHANNELS {
            if !self.channels[ch_idx].filter_dirty {
                continue;
            }
            self.channels[ch_idx].filter_dirty = false;
            let ch = self.channels[ch_idx];
            let sr = self.sample_rate;
            for v in self.voices.iter_mut() {
                if v.channel as usize == ch_idx && !v.finished() {
                    v.apply_filter_cc(&ch, sr);
                }
            }
        }
    }
}
