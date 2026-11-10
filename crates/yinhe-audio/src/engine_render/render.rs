//! 渲染主路径：每块从合成后端写 planar 通道缓冲 → 插件/通道处理段/音频轨 → 混音输出。

use crate::engine::AudioEngine;

use super::STEREO_CHANNELS;

/// 把 mixer 的 master 立体声输出交错写入输出缓冲。
fn write_master(output: &mut [f32], master_l: &[f32], master_r: &[f32]) {
    for i in 0..output.len() / STEREO_CHANNELS {
        output[i * STEREO_CHANNELS] = master_l[i];
        output[i * STEREO_CHANNELS + 1] = master_r[i];
    }
}

impl AudioEngine {
    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 {
            output.fill(0.0);
            return;
        }
        // 未播放：主引擎不推进走带，只把试听预览混进同一 mixer + 通道 DSP 输出。
        if !self.playing {
            self.render_preview_only(output, frames);
            return;
        }

        // 块长变化（导出用 1024、实时 512）：mixer 缓冲与通道暂存按实际块长
        // 重建一次。循环拆块只改**输出内偏移**，不改 mixer 块长。
        self.ensure_block_buffers(frames);
        // 音频轨缓冲是覆盖写；整块只清一次，之后各 span 按 offset 写入。
        self.clear_audio_buffers(frames);

        let mut done = 0usize;
        while done < frames {
            let (span, wrapped) = self.next_loop_span(frames - done);
            if span == 0 {
                self.rewind_to_loop_start();
                continue;
            }
            self.render_span(done, span);
            done += span;
            if wrapped {
                self.rewind_to_loop_start();
            }
        }

        // 试听预览与走带无关：整块混入一次。
        self.inject_preview(frames);
        // 内置音源通道处理段（CC7/10/11/71/74）在混音台 insert 链之前生效。
        self.render_channel_dsp(frames);

        let (master_l, master_r) = self.mixer.process();
        write_master(output, master_l, master_r);
    }

    fn ensure_block_buffers(&mut self, frames: usize) {
        if self.mixer.frames() != frames {
            let strips = self.dense_strip_params();
            let count = self.mixer.channel_count();
            self.mixer.resize(count, frames, &strips);
            self.channel_set.resize_scratches(frames);
        }
    }

    /// 本块还能连续渲染多少帧，以及该 span 是否正好撞到 loop end。
    fn next_loop_span(&self, remaining: usize) -> (usize, bool) {
        if !self.loop_enabled || self.export_mode {
            return (remaining, false);
        }
        let end = self.loop_end_sample();
        let pos = self.sample_position;
        if pos >= end {
            return (0, true);
        }
        let span = (remaining as u64).min(end - pos) as usize;
        let wrapped = pos + span as u64 >= end;
        (span.max(1), wrapped)
    }

    /// 渲染一个连续 span 到输出缓冲的 `offset..offset+frames`。
    fn render_span(&mut self, offset: usize, frames: usize) {
        if frames == 0 {
            return;
        }
        let block_start_sample = self.sample_position;
        let block_end_sample = block_start_sample + frames as u64;
        let block_end_tick = self.sample_to_tick(block_end_sample);
        self.block_start_sample = block_start_sample;

        if self.gpu_synth.is_some() {
            // GPU 渲染（只写本 span 的 offset 区间，超出通道清零由 span 外的
            // `mixer.process` 之前保证；ring 不足时按 span 帧数提交）。
            let t_gpu = std::time::Instant::now();
            if let Some(synth) = self.gpu_synth.as_mut() {
                synth.render_to_mixer_range(self.mixer.buffers_mut(), offset, frames);
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
                    use yinhe_synth::gpu_synth::{
                        EVICT_VEL_HI, EVICT_VEL_LO, EVICT_VEL_MID, NOTE_ON_REJECTED, REJECT_VEL_HI,
                        REJECT_VEL_LO, REJECT_VEL_MID,
                    };
                    let ak = |v: &std::sync::atomic::AtomicU64| {
                        v.load(std::sync::atomic::Ordering::Relaxed)
                    };
                    let (d, alive, blocks, hist, aged, topk) = match self.gpu_synth.as_ref() {
                        Some(s) => (
                            s.diag_ms,
                            s.diag_alive,
                            s.diag_blocks,
                            s.velocity_histogram(),
                            s.long_lived_voices(now_sample, self.sample_rate as u64 * 10),
                            s.top_keys(3),
                        ),
                        None => ([0.0; 8], 0, 0, [0u32; 8], 0, Vec::new()),
                    };
                    crate::audio_renderer::play_log(&format!(
                        "[gpu] 块超时：{gpu_ms:.2}ms > 预算 {budget_ms:.2}ms（frames={frames}）\
                         collect={:.1} submit={:.1} harvest={:.1} ring={:.1} out={:.1} compact={:.1} active={:.1} upload={:.1} alive={alive} blocks={blocks} 力度[127-112/111-96/95-80/79-64/63-48/47-32/31-16/15-0]={:?} 长寿(>10s)={aged} top键={topk:?} 淘汰力度累计[小/中/大]={:?} 累计{} 拒绝[小/中/大]={:?} 累计={} flush={}次/{}槽/{}us",
                        d[0],
                        d[1],
                        d[2],
                        d[3],
                        d[4],
                        d[5],
                        d[6],
                        d[7],
                        hist,
                        [ak(&EVICT_VEL_LO), ak(&EVICT_VEL_MID), ak(&EVICT_VEL_HI)],
                        ak(&EVICT_VEL_LO) + ak(&EVICT_VEL_MID) + ak(&EVICT_VEL_HI),
                        [ak(&REJECT_VEL_LO), ak(&REJECT_VEL_MID), ak(&REJECT_VEL_HI)],
                        ak(&NOTE_ON_REJECTED),
                        ak(&yinhe_synth::gpu_synth::FLUSH_WRITES),
                        ak(&yinhe_synth::gpu_synth::FLUSH_SLOTS),
                        ak(&yinhe_synth::gpu_synth::FLUSH_US)
                    ));
                }
            }
            // 插件乐器事件：按 span 结束 tick 推进（GPU 模式下非插件通道不喂 xsynth）。
            self.dispatch_block_events(block_end_tick);
            self.dispatch_plugin_previews(block_start_sample, frames);
            self.render_instruments(block_start_sample, offset, frames);
            self.render_audio_tracks(block_start_sample, offset, frames);
        } else {
            // CPU 路径：xsynth 逐段分发+渲染。事件比较全在 tick 域
            // （dispatch 基准 = current_tick，块边界 = sample→tick 反查），
            // 只有"渲染段边界"才转一次 sample（每块事件数量级）。
            let mut rendered_until_sample = block_start_sample;
            let mut rendered_until_tick = self.current_tick;
            let mut offset_frames = offset;

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
                    self.segment_start_sample = rendered_until_sample;
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
                self.segment_start_sample = rendered_until_sample;
                self.render_cpu_segment(offset_frames, remaining as usize);
            }

            // 乐器插件：把本 span 累积的事件喂给各自实例，输出写进对应乐器 dense 通道。
            self.dispatch_plugin_previews(block_start_sample, frames);
            self.render_instruments(block_start_sample, offset, frames);
            // 音频片段回放：按绝对时间混进对应音频 dense 通道（无状态）。
            self.render_audio_tracks(block_start_sample, offset, frames);
        }

        self.sample_position = block_end_sample;
        self.current_tick = block_end_tick;
    }

    /// 渲染一个 CPU 段到混音台通道缓冲（`offset_frames` 是输出块内偏移）。
    fn render_cpu_segment(&mut self, offset_frames: usize, frames: usize) {
        if let Some(cs) = self.cpu_synth.as_mut() {
            cs.render_range(self.mixer.buffers_mut(), offset_frames, frames);
            return;
        }
        self.channel_set
            .render_segment(self.mixer.buffers_mut(), offset_frames, frames);
    }

    /// 音频通道缓冲是覆盖写（没有 xsynth/插件替它清零），整块清一次。
    fn clear_audio_buffers(&mut self, frames: usize) {
        for &ach in self.channel_layout.audio_channels() {
            let dense = self.channel_layout.audio_dense_for(ach) as usize;
            if let Some(cb) = self.mixer.channel_buffers_mut(dense) {
                let n = frames.min(cb.left.len()).min(cb.right.len());
                cb.left[..n].fill(0.0);
                cb.right[..n].fill(0.0);
            }
        }
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
        self.render_instruments(block_start, 0, frames);
        self.inject_preview(frames);
        let (master_l, master_r) = self.mixer.process();
        write_master(output, master_l, master_r);
    }

    /// 未播放时的纯预览渲染：清空 mixer 通道缓冲后写入预览音频，再走与播放
    /// 完全相同的通道 DSP + 混音台链路输出（推子/master/insert/mute-solo 全生效）。
    fn render_preview_only(&mut self, output: &mut [f32], frames: usize) {
        if !self.previewing() {
            output.fill(0.0);
            return;
        }
        // 块长变化（实时 512 / GPU 4096 / 导出 1024）：与播放路径同一逻辑。
        if self.mixer.frames() != frames {
            let strips = self.dense_strip_params();
            let count = self.mixer.channel_count();
            self.mixer.resize(count, frames, &strips);
            self.channel_set.resize_scratches(frames);
        }
        self.mixer.clear_channel_buffers();
        self.inject_preview(frames);
        self.render_channel_dsp(frames);
        let (master_l, master_r) = self.mixer.process();
        write_master(output, master_l, master_r);
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
        for (dense, chain) in mixer.buffers_mut().iter_mut().enumerate().take(midi_count) {
            if instruments.get(dense).is_some_and(|s| s.is_some()) {
                continue;
            }
            if let Some(dsp) = channel_dsp.get_mut(dense) {
                let buf = &mut **chain;
                let f = frames.min(buf.left.len()).min(buf.right.len());
                dsp.process(&mut buf.left[..f], &mut buf.right[..f]);
            }
        }
    }

    /// 音频轨片段回放：按块内**绝对时间**把每条音频轨的片段混进对应音频
    /// dense 通道缓冲。完全无状态（seek/暂停/导出天然正确），PDC 由混音台
    /// 统一补偿。音频通道是覆盖写（没有 xsynth/插件替它清零）。
    pub(super) fn render_audio_tracks(
        &mut self,
        block_start_sample: u64,
        offset_frames: usize,
        frames: usize,
    ) {
        if self.channel_layout.audio_channels().is_empty() {
            return;
        }
        let Some(model) = self.yin_model.clone() else {
            return;
        };
        // `AudioModel` 与 `yin_model` 由同一次模型快照原子替换，clip_fades 索引对齐；
        // 取不到（未加载模型）时与 yin_model 缺失同义，直接跳过音频轨。
        let Some(audio_model) = self.model.as_ref() else {
            return;
        };
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
                // 0 BPM 剪切：片段起点在剪切内 → 丢弃；跨剪切起点 → 截断；
                // 剪切之后的片段整体左移，与 tick/音符使用同一时间轴。
                let Some((adj_start, adj_end)) = model
                    .tempo_map
                    .cut_clip_times(clip.start_seconds, clip.end_seconds())
                else {
                    continue;
                };
                let adj_duration = adj_end - adj_start;
                if adj_duration <= 0.0 {
                    continue;
                }
                let clip_start = (adj_start * sr).round() as i64;
                let clip_end = (adj_end * sr).round() as i64;
                let from = clip_start.max(block_start);
                let to = clip_end.min(block_end);
                if to <= from {
                    continue;
                }
                // 预计算的 (淡入, 淡出)：索引与 audio_clips 对齐，O(1) 无扫描。
                let Some(&(fade_in, fade_out)) = audio_model
                    .clip_fades
                    .get(track_idx)
                    .and_then(|f| f.get(ci))
                else {
                    continue;
                };
                let Some(cb) = self.mixer.channel_buffers_mut(dense) else {
                    continue;
                };
                let out_end = offset_frames + frames;
                for g in from..to {
                    let n = offset_frames + (g - block_start) as usize;
                    if n >= out_end {
                        break;
                    }
                    // 片段内已播时长（秒）；淡入淡出与素材偏移都以它为准。
                    let p = g as f64 / sr - adj_start;
                    let mut gain = clip.gain;
                    if fade_in > 0.0 && p < fade_in {
                        gain *= (p / fade_in).clamp(0.0, 1.0) as f32;
                    }
                    let remain = adj_duration - p;
                    if fade_out > 0.0 && remain < fade_out {
                        gain *= (remain / fade_out).clamp(0.0, 1.0) as f32;
                    }
                    let src_sec = if clip.reversed {
                        clip.offset_seconds + (adj_duration - p)
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
