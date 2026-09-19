//! 渲染流水线：提交/收割两阶段、输出 ring、compact 与流水线排空。
//!
//! 拆自 gpu_synth.rs（文件过长）；`drain_pending` 被主文件的
//! seek/load_events 调用，故标 `pub(super)`，其余仅本模块内使用。

use super::*;
use crate::synth::buffers::PIPELINE_DEPTH;

impl GpuSynth {
    /// 渲染一块到混音台的 planar 通道缓冲（覆盖写，与 CPU 路径
    /// `ChannelSet::render_segment` 同格式）：GPU 槽位 `ch` 写入 `buffers[ch]`，
    /// 超出 `MAX_CHANNELS` 的 dense 通道清零（GPU 合成器只支持前 32 个通道）。
    ///
    /// 块内事件（CC 段边界、note on/off、release/env 指令）在 CPU 收集为段结构，
    /// **一次 GPU 提交**渲染整块；voice 状态在 GPU 内逐帧推进（块末全字段读回）。
    pub fn render_to_mixer(&mut self, buffers: &mut [yinhe_mixer::ChannelBuffers]) {
        let frames = buffers.first().map(|b| b.left.len()).unwrap_or(0);
        if frames == 0 {
            return;
        }
        let per_frame = MAX_CHANNELS * 2;
        let need = frames * per_frame;
        self.diag_ms = [0.0; 8];
        self.diag_blocks = 0;
        // 预渲染：ring 不足时提交/收割（提交超前、收割入 ring；块大小可变化）
        while self.ring.len() < need {
            if self.compact_needed() {
                let t = std::time::Instant::now();
                self.drain_pending(true);
                self.compact_voices();
                self.diag_ms[5] += t.elapsed().as_secs_f64() * 1000.0;
            }
            let mut progressed = false;
            while self.pending.len() < PIPELINE_DEPTH && self.has_content() {
                let t = std::time::Instant::now();
                let ok = self.submit_one_block(frames);
                self.diag_ms[1] += t.elapsed().as_secs_f64() * 1000.0;
                if !ok {
                    break;
                }
                self.diag_blocks += 1;
                progressed = true;
            }
            if let Some(p) = self.pending.pop_front() {
                let t = std::time::Instant::now();
                self.harvest(&p);
                self.diag_ms[2] += t.elapsed().as_secs_f64() * 1000.0;
                let t = std::time::Instant::now();
                self.push_block_to_ring(p.frames);
                self.diag_ms[3] += t.elapsed().as_secs_f64() * 1000.0;
                progressed = true;
            }
            if !progressed {
                break;
            }
        }

        // 输出 frames 帧：ring 中的先给，不足部分静音补齐
        let t_out = std::time::Instant::now();
        let avail = (self.ring.len() / per_frame).min(frames);
        self.diag_ring_short = if avail < frames {
            (frames - avail) as u32
        } else {
            0
        };
        for (ch_idx, buf) in buffers.iter_mut().enumerate() {
            if ch_idx < MAX_CHANNELS {
                for f in 0..avail {
                    let base = f * per_frame + ch_idx * 2;
                    buf.left[f] = self.ring[base];
                    buf.right[f] = self.ring[base + 1];
                }
                for f in avail..frames {
                    buf.left[f] = 0.0;
                    buf.right[f] = 0.0;
                }
            } else {
                buf.left.fill(0.0);
                buf.right.fill(0.0);
            }
        }
        self.ring.drain(..avail * per_frame);
        self.diag_ms[4] = t_out.elapsed().as_secs_f64() * 1000.0;
        // 已输出位置（外部可见的播放进度）
        self.sample_position += frames as u64;
        // 无内容且 ring 已耗尽：提交游标与输出对齐（避免无限积压）
        if self.ring.is_empty() && !self.has_content() {
            self.render_position = self.sample_position;
        }
        let alive = self.voices.iter().filter(|v| v.state.env_stage < 6).count();
        self.peak_voices = self.peak_voices.max(alive);
        self.diag_alive = alive as u32;
    }

    /// 是否还有可渲染内容（活跃 voice 或未消费事件）。
    fn has_content(&self) -> bool {
        !self.voices.is_empty() || self.event_cursor < self.events.len()
    }

    /// 压缩预判：仅**无空闲槽位且已达容量**时兜底压缩。compact 会排空流水线
    /// 等待在途 GPU 块（用户实测 117ms 尖峰），必须低频；碎片化由 free list
    /// 复用与 harvest 尾部截断控制，不靠墓碑比例触发。
    fn compact_needed(&self) -> bool {
        self.free_slots.is_empty() && self.voices.len() >= self.voice_capacity
    }

    /// 压缩：清理已结束 voice（tombstone）并全量重传槽位状态。
    fn compact_voices(&mut self) {
        self.voices
            .retain(|v| v.state.env_stage < 6 || v.kill_pending);
        self.free_slots.clear();
        self.freed_flags.clear();
        self.freed_flags.resize(self.voices.len(), false);
        for (i, v) in self.voices.iter().enumerate() {
            self.renderer.write_voice_state(i as u32, &v.state);
        }
    }

    /// 提交一个块（不等待）：collect + 上传新 voice + renderer.submit_block。
    /// 返回 false 表示无可提交内容（无 voice / 无 GPU 缓冲）。
    fn submit_one_block(&mut self, frames: usize) -> bool {
        let block_start = self.render_position;
        let block_end = block_start + frames as u64;
        let upload_from = self.voices.len();
        let mut seg_data = std::mem::take(&mut self.seg_scratch);
        let mut seg_used = 0usize;
        let mut offset = 0usize;
        while offset < frames {
            let seg_frames = (frames - offset).min(RENDER_SEGMENT_FRAMES as usize);
            let s0 = block_start + offset as u64;
            let s1 = s0 + seg_frames as u64;
            if seg_used == seg_data.len() {
                seg_data.push(SegBuffers::default());
            }
            let sb = &mut seg_data[seg_used];
            sb.frame_start = offset as u32;
            sb.frame_length = seg_frames as u32;
            sb.segs.clear();
            sb.ch_updates.clear();
            sb.releases.clear();
            sb.env_cmds.clear();
            let t_collect = std::time::Instant::now();
            self.collect_block(
                s0,
                s1,
                offset as u32,
                &mut sb.segs,
                &mut sb.ch_updates,
                &mut sb.releases,
                &mut sb.env_cmds,
            );
            self.diag_ms[0] += t_collect.elapsed().as_secs_f64() * 1000.0;
            // 活跃 voice 列表（每段重建）：pass1/pass2 只遍历活跃槽位，
            // 渲染量与 `voices.len()`（槽位高水位/墓碑）彻底解耦。
            let t_active = std::time::Instant::now();
            sb.active.clear();
            sb.active_ranges.clear();
            sb.active_ranges.resize(MAX_CHANNELS * 2, 0);
            let mut counts = [0u32; MAX_CHANNELS];
            for v in self.voices.iter() {
                // kill_pending 的仍需渲染（应用 1ms 淡出，见 Voice::kill_pending）
                if v.state.env_stage < 6 || v.kill_pending {
                    counts[v.state.channel as usize] += 1;
                }
            }
            let mut acc = 0u32;
            for (c, &n) in counts.iter().enumerate() {
                sb.active_ranges[c * 2] = acc;
                sb.active_ranges[c * 2 + 1] = n;
                acc += n;
            }
            let mut cursor: [u32; MAX_CHANNELS] = std::array::from_fn(|c| sb.active_ranges[c * 2]);
            sb.active.resize(acc as usize, 0);
            for (i, v) in self.voices.iter().enumerate() {
                if v.state.env_stage >= 6 && !v.kill_pending {
                    continue;
                }
                let c = v.state.channel as usize;
                let slot = cursor[c] as usize;
                cursor[c] += 1;
                sb.active[slot] = i as u32;
            }
            sb.active_count = acc;
            self.diag_ms[6] += t_active.elapsed().as_secs_f64() * 1000.0;
            seg_used += 1;
            offset += seg_frames;
        }

        // 只上传本块新增的 voice 槽位（状态常驻 GPU，不再整块重传）。
        let t_upload = std::time::Instant::now();
        for (i, v) in self.voices.iter().enumerate().skip(upload_from) {
            self.renderer.write_voice_state(i as u32, &v.state);
        }
        self.diag_ms[7] += t_upload.elapsed().as_secs_f64() * 1000.0;

        self.channel_mix.resize(MAX_CHANNELS * frames * 2, 0.0);
        if self.voices.is_empty() {
            // 本块无 voice：输出静音，但提交游标必须前进（时间在流逝，事件可能
            // 在后续块；原实现在无 voice 时也无条件推进 sample_position）。
            self.channel_mix.fill(0.0);
            self.render_position = block_end;
            self.seg_scratch = seg_data;
            return false;
        }
        let submitted = {
            let segments: Vec<RenderSegment<'_>> = seg_data[..seg_used]
                .iter()
                .map(|s| RenderSegment {
                    frame_start: s.frame_start,
                    frame_length: s.frame_length,
                    segs: &s.segs,
                    ch_updates: &s.ch_updates,
                    releases: &s.releases,
                    env_cmds: &s.env_cmds,
                    active_count: s.active_count,
                    active_data: &s.active,
                    active_ranges: &s.active_ranges,
                })
                .collect();
            // 全字段读回：收割时用 GPU 权威状态覆盖 CPU 镜像。
            let rb = self.renderer.submit_block(
                self.voices.len() as u32,
                frames as u32,
                true,
                &segments,
                self.sample_rate,
            );
            drop(segments);
            rb
        };
        self.seg_scratch = seg_data;
        match submitted {
            Some(readback) => {
                self.pending.push_back(PendingGpuBlock {
                    readback,
                    frames,
                    voice_count: self.voices.len(),
                    voice_gens: self.voices.iter().map(|v| v.slot_gen).collect(),
                });
                self.render_position = block_end;
                true
            }
            None => false,
        }
    }

    /// 把一块已收割的 `channel_mix`（**通道优先** `[ch][frame][lr]`）重排为
    /// ring 的**帧优先**布局 `[frame][ch][lr]`（ring 跨块拼接只按帧消费）。
    fn push_block_to_ring(&mut self, frames: usize) {
        let ch = MAX_CHANNELS;
        for f in 0..frames {
            for c in 0..ch {
                let src = c * frames * 2 + f * 2;
                if src + 1 < self.channel_mix.len() {
                    self.ring.push_back(self.channel_mix[src]);
                    self.ring.push_back(self.channel_mix[src + 1]);
                } else {
                    self.ring.push_back(0.0);
                    self.ring.push_back(0.0);
                }
            }
        }
    }

    /// 收割读回（等待 + 拷贝），并用 GPU 权威 env_stage 更新 CPU 镜像。
    fn harvest(&mut self, p: &PendingGpuBlock) -> u32 {
        self.states_buf
            .resize(self.voices.len(), GpuVoiceState::default());
        let n = self.renderer.finish_block(
            &p.readback,
            &mut self.channel_mix,
            &mut [],
            Some(self.states_buf.as_mut_slice()),
        );
        let cnt = p.voice_count.min(self.voices.len());
        self.diag_gpu_mix_peak = self.channel_mix.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        // GPU 权威状态覆盖 CPU 镜像（仅该块提交时的前 N 个槽位；之后新 push 的
        // voice 保持 CPU 侧初值）。compact 重传前必须一致。
        for (i, (v, st)) in self.voices[..cnt]
            .iter_mut()
            .zip(self.states_buf.iter())
            .enumerate()
        {
            // 身份校验：槽位已被 note_on 复用为新 voice（代号不同）时，本块读回
            // 的是旧 voice 的状态，丢弃（否则会把新 voice 镜像写成旧 voice 的
            // 死亡状态 → 误回收 → 槽位被反复复用覆盖 → 声音消失/断续）。
            if i < p.voice_gens.len() && v.slot_gen != p.voice_gens[i] {
                continue;
            }
            // 待确认 kill 的 voice：GPU 还在 1ms 淡出（读回 env_stage=5），
            // 该状态是提交本块前的旧值；覆盖会把 CPU 的"已判死"复活，
            // 导致下段重复淘汰同一 voice。等 GPU 确认 >=6 再同步并回收。
            if v.kill_pending && st.env_stage < 6 {
                continue;
            }
            let was_active = v.state.env_stage < 6;
            v.state = *st;
            if st.env_stage >= 6 {
                // GPU 已确认结束（含 kill 淡出完成）→ 清除待确认标记
                v.kill_pending = false;
                if was_active {
                    // GPU 自然结束（非 CPU kill）：per-key layer 活跃计数 -1
                    let b = v.channel as usize * 128 + v.key as usize;
                    self.layer_counts[b] = self.layer_counts[b].saturating_sub(1);
                }
            }
            // GPU 权威判定已结束 → 槽位立即回收到 free list（下次 note_on 复用）。
            // 墓碑不再累积：note_on 不会因容量拒绝新音，compact 也不再每块排空
            // 流水线（旧行为是丢音与 60-90ms 卡顿的来源）。
            if st.env_stage >= 6 && !self.freed_flags[i] {
                self.freed_flags[i] = true;
                self.free_slots.push_back(i as u32);
            }
        }
        // 尾部截断：末尾连续墓碑直接 pop（索引不变、无需重传 GPU 状态）。
        // 若不截断，`voices.len()`（= 提交给 shader 的 voice_count）会停在
        // 高水位，pass2 每块仍扫全部槽位——实测 alive 4300 时 harvest 仍 92ms。
        while let Some(v) = self.voices.last() {
            if v.state.env_stage < 6 || v.kill_pending {
                break;
            }
            self.voices.pop();
            self.freed_flags.pop();
        }
        // 被 pop 掉的槽位不在 free list 中的需补入（它们已不在 voices 里）
        self.free_slots
            .retain(|&s| (s as usize) < self.voices.len());
        n
    }

    /// 排空流水线：`update_stage` 时收割入 ring（不丢音频），否则直接丢弃
    /// （seek/换事件时旧内容不应再输出）。
    pub(super) fn drain_pending(&mut self, update_stage: bool) {
        while let Some(p) = self.pending.pop_front() {
            if update_stage {
                self.harvest(&p);
                self.push_block_to_ring(p.frames);
            } else {
                self.renderer.discard_block(&p.readback);
            }
        }
    }
}
