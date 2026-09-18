//! 渲染主路径：每块从合成后端写 planar 通道缓冲 → 插件/通道处理段/音频轨 → 混音输出。

use crate::engine::AudioEngine;

use super::STEREO_CHANNELS;

impl AudioEngine {
    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 || !self.playing {
            output.fill(0.0);
            return;
        }

        // GPU 路径：GpuSynth 渲染到混音台 planar 通道缓冲，之后与 CPU 路径
        // 共用插件乐器 / 音频轨 / mixer.process（insert 效果器、总线、推子全部生效）。
        // 块长变化（导出用 1024、实时 512）：与 CPU 路径同一逻辑。
        #[cfg(feature = "gpu")]
        if self.gpu_synth.is_some() {
            if self.mixer.frames() != frames {
                let strips = self.dense_strip_params();
                let count = self.mixer.channel_count();
                self.mixer.resize(count, frames, &strips);
                self.channel_set.resize_scratches(frames);
            }
            let block_start_sample = self.sample_position;
            let block_end_sample = block_start_sample + frames as u64;
            let block_end_tick = self.sample_to_tick(block_end_sample);
            self.block_start_sample = block_start_sample;

            // GPU 渲染（覆盖写 dense 0..MAX_CHANNELS，其余清零）。
            // 实时块耗时诊断：超过本块时长预算（frames / sample_rate）即打印
            // （限频每秒一条）——用于定位 GPU 后端断续是否由块超时（underrun）
            // 引起，而非渲染语义差异。
            let t_gpu = std::time::Instant::now();
            if let Some(synth) = self.gpu_synth.as_mut() {
                synth.render_to_mixer(self.mixer.buffers_mut());
            }
            let gpu_ms = t_gpu.elapsed().as_secs_f64() * 1000.0;
            let budget_ms = frames as f64 / self.sample_rate as f64 * 1000.0;
            if gpu_ms > budget_ms {
                static LAST_LOG: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let last = LAST_LOG.load(std::sync::atomic::Ordering::Relaxed);
                if now.saturating_sub(last) >= 1000 {
                    LAST_LOG.store(now, std::sync::atomic::Ordering::Relaxed);
                    let now_sample = self.sample_position;
                    let (d, alive, blocks, hist, aged, topk) = match self.gpu_synth.as_ref() {
                        Some(s) => (
                            s.diag_ms,
                            s.diag_alive,
                            s.diag_blocks,
                            s.velocity_histogram(),
                            s.long_lived_voices(now_sample, self.sample_rate as u64 * 10),
                            s.top_keys(3),
                        ),
                        None => ([0.0; 6], 0, 0, [0u32; 8], 0, Vec::new()),
                    };
                    crate::audio_renderer::play_log(&format!(
                        "[gpu] 块超时：{gpu_ms:.2}ms > 预算 {budget_ms:.2}ms（frames={frames}）\
                         collect={:.1} submit={:.1} harvest={:.1} ring={:.1} out={:.1} compact={:.1} alive={alive} blocks={blocks} 力度[127-112/111-96/95-80/79-64/63-48/47-32/31-16/15-0]={:?} 长寿(>10s)={aged} top键={topk:?}",
                        d[0], d[1], d[2], d[3], d[4], d[5], hist
                    ));
                }
            }
            // 插件乐器事件：按块推进 tick（GPU 模式下非插件通道不喂 xsynth）。
            self.dispatch_block_events(block_end_tick);
            self.dispatch_plugin_previews(block_start_sample, frames);
            self.render_instruments(block_start_sample, frames);
            self.render_channel_dsp(frames);
            self.render_audio_tracks(block_start_sample, frames);

            let (master_l, master_r) = self.mixer.process();
            for (i, chunk) in output.chunks_exact_mut(STEREO_CHANNELS).enumerate() {
                chunk[0] = master_l[i];
                chunk[1] = master_r[i];
            }
            self.sample_position = block_end_sample;
            self.current_tick = block_end_tick;
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
        self.block_start_sample = block_start_sample;
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
                // 本段的事件 sample 依据（dispatch 在下一轮循环开头使用）
                #[cfg(feature = "gpu")]
                {
                    self.segment_start_sample = rendered_until_sample;
                }
                self.render_cpu_segment(offset_frames, segment_frames);
                rendered_until_sample = next_sample;
                offset_frames += segment_frames;
            }
            rendered_until_tick = next_tick;
        }

        // 补齐剩余帧（浮点/块对齐：tick_to_sample(block_end_tick) 可能略小于
        // block_end_sample，剩余段无事件）。
        let remaining = block_end_sample - rendered_until_sample;
        if remaining > 0 {
            #[cfg(feature = "gpu")]
            {
                self.segment_start_sample = rendered_until_sample;
            }
            self.render_cpu_segment(offset_frames, remaining as usize);
        }

        // 乐器插件：把每块累积的事件喂给各自实例，输出写进对应乐器 dense 通道。
        self.dispatch_plugin_previews(block_start_sample, frames);
        self.render_instruments(block_start_sample, frames);

        // 内置音源通道处理段（CC7/10/11/71/74）在混音台 insert 链之前生效。
        self.render_channel_dsp(frames);

        // 音频片段回放：按绝对时间直接混进对应音频 dense 通道（无状态）。
        self.render_audio_tracks(block_start_sample, frames);

        // 混音：insert → 增益/声像斜坡 → mute/solo → master，然后交错输出。
        let (master_l, master_r) = self.mixer.process();
        for (i, chunk) in output.chunks_exact_mut(STEREO_CHANNELS).enumerate() {
            chunk[0] = master_l[i];
            chunk[1] = master_r[i];
        }

        self.sample_position = block_end_sample;
        self.current_tick = block_end_tick;
    }

    /// 空闲渲染（停止/暂停）：不推进走带、不派发音符，只驱动乐器插件
    ///（GUI 键盘、插件预览、插件尾音）与混音输出。
    /// 存在已安装乐器时由渲染器持续调用（成熟 DAW 语义：乐器插件始终在跑）。
    /// 渲染一个 CPU 段到混音台通道缓冲：yinhe `CpuSynth` 存在时走它，
    /// 否则走 xsynth `ChannelSet`（GPU 模式不调用本方法）。
    fn render_cpu_segment(&mut self, offset_frames: usize, frames: usize) {
        #[cfg(feature = "gpu")]
        if let Some(cs) = self.cpu_synth.as_mut() {
            cs.render_range(self.mixer.buffers_mut(), offset_frames, frames);
            return;
        }
        self.channel_set
            .render_segment(self.mixer.buffers_mut(), offset_frames, frames);
    }

    pub(crate) fn render_idle(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 {
            output.fill(0.0);
            return;
        }
        // 块长变化（导出用 1024、实时 512）：与 render 同一逻辑。
        if self.mixer.frames() != frames {
            let strips = self.dense_strip_params();
            let count = self.mixer.channel_count();
            self.mixer.resize(count, frames, &strips);
            self.channel_set.resize_scratches(frames);
        }
        let block_start = self.sample_position;
        self.block_start_sample = block_start;
        self.mixer.clear_channel_buffers();
        self.dispatch_plugin_previews(block_start, frames);
        self.render_instruments(block_start, frames);
        let (master_l, master_r) = self.mixer.process();
        for (i, chunk) in output.chunks_exact_mut(STEREO_CHANNELS).enumerate() {
            chunk[0] = master_l[i];
            chunk[1] = master_r[i];
        }
    }

    /// 内置音源通道处理段：合成器输出之后、`mixer.process()` 之前，
    /// 对所有内置音源 MIDI 通道应用 CC7/10/11/71/74 的当前值。
    /// 插件乐器通道跳过（CC 已透传插件，插件自行处理）。
    pub(super) fn render_channel_dsp(&mut self, frames: usize) {
        let midi_count = self.channel_layout.midi_compacted() as usize;
        // 字段级借用拆分：mixer 缓冲 / 处理段 / 乐器表互不相交。
        let Self {
            mixer,
            channel_dsp,
            instruments,
            ..
        } = self;
        for (dense, buf) in mixer.buffers_mut().iter_mut().enumerate().take(midi_count) {
            if instruments.get(dense).is_some_and(|s| s.is_some()) {
                continue;
            }
            if let Some(chain) = channel_dsp.get_mut(dense) {
                let f = frames.min(buf.left.len()).min(buf.right.len());
                chain.process(&mut buf.left[..f], &mut buf.right[..f]);
            }
        }
    }

    /// 音频轨片段回放：按块内**绝对时间**把每条音频轨的片段混进对应音频
    /// dense 通道缓冲。完全无状态（seek/暂停/导出天然正确），PDC 由混音台
    /// 统一补偿。音频通道是覆盖写（没有 xsynth/插件替它清零）。
    pub(super) fn render_audio_tracks(&mut self, block_start_sample: u64, frames: usize) {
        if self.channel_layout.audio_channels().is_empty() {
            return;
        }
        let Some(model) = self.yin_model.clone() else {
            return;
        };
        for &ach in self.channel_layout.audio_channels() {
            let dense = self.channel_layout.audio_dense_for(ach) as usize;
            if let Some(cb) = self.mixer.channel_buffers_mut(dense) {
                let n = frames.min(cb.left.len()).min(cb.right.len());
                cb.left[..n].fill(0.0);
                cb.right[..n].fill(0.0);
            }
        }
        let sr = self.sample_rate as f64;
        let block_start = block_start_sample as i64;
        let block_end = block_start + frames as i64;
        for (track_idx, track) in model.tracks.iter().enumerate() {
            if track.kind != yinhe_core::TrackKind::Audio || track.audio_clips.is_empty() {
                continue;
            }
            if self.skip_track.get(track_idx).copied().unwrap_or(false) {
                continue;
            }
            let Some(ach) = track.audio_channel else {
                continue;
            };
            let dense = self.channel_layout.audio_dense_for(ach);
            if dense == u32::MAX {
                continue;
            }
            let dense = dense as usize;
            for (ci, clip) in track.audio_clips.iter().enumerate() {
                let Some(pcm) = self.audio_sources.get(&clip.source) else {
                    continue;
                };
                if pcm.frames == 0 || clip.duration_seconds <= 0.0 {
                    continue;
                }
                let clip_start = (clip.start_seconds * sr).round() as i64;
                let clip_end = (clip.end_seconds() * sr).round() as i64;
                let from = clip_start.max(block_start);
                let to = clip_end.min(block_end);
                if to <= from {
                    continue;
                }
                let (fade_in, fade_out) =
                    crate::audio_model::effective_fades(&track.audio_clips, ci);
                let Some(cb) = self.mixer.channel_buffers_mut(dense) else {
                    continue;
                };
                for g in from..to {
                    let n = (g - block_start) as usize;
                    if n >= frames {
                        break;
                    }
                    // 片段内已播时长（秒）；淡入淡出与素材偏移都以它为准。
                    let p = g as f64 / sr - clip.start_seconds;
                    let mut gain = clip.gain;
                    if fade_in > 0.0 && p < fade_in {
                        gain *= (p / fade_in).clamp(0.0, 1.0) as f32;
                    }
                    let remain = clip.duration_seconds - p;
                    if fade_out > 0.0 && remain < fade_out {
                        gain *= (remain / fade_out).clamp(0.0, 1.0) as f32;
                    }
                    let src_sec = if clip.reversed {
                        clip.offset_seconds + (clip.duration_seconds - p)
                    } else {
                        clip.offset_seconds + p
                    };
                    let src_frame = (src_sec * sr).floor();
                    if src_frame < 0.0 {
                        continue;
                    }
                    let si = src_frame as usize;
                    if si >= pcm.frames {
                        continue;
                    }
                    cb.left[n] += pcm.left[si] * gain;
                    cb.right[n] += pcm.right[si] * gain;
                }
            }
        }
    }
}
