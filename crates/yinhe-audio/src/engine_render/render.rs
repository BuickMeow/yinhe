//! 渲染主路径：每块从合成后端写 planar 通道缓冲 → 插件/通道处理段/音频轨 → 混音输出。

use crate::engine::AudioEngine;

use super::STEREO_CHANNELS;

/// 对一段缓冲应用 [start, end) 的淡入/淡出包络；越界自动裁剪。
fn apply_fade_range(buf: &mut [f32], start: usize, end: usize, fade_in: bool) {
    let end = end.min(buf.len());
    let start = start.min(end);
    let len = (end - start) as f32;
    if len <= 0.0 {
        return;
    }
    for (k, s) in buf[start..end].iter_mut().enumerate() {
        let g = if fade_in {
            (k + 1) as f32 / len
        } else {
            1.0 - (k + 1) as f32 / (len + 1.0)
        };
        *s *= g.clamp(0.0, 1.0);
    }
}

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
        self.loop_wrap_offsets.clear();

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
                self.loop_wrap_offsets.push(done);
                self.rewind_to_loop_start();
            }
        }

        // 循环回绕点做极短淡出/淡入（去咔哒）；跨块时把剩余淡入带到下一块。
        self.apply_loop_declick(frames);
        // 试听预览与走带无关：整块混入一次。
        self.inject_preview(frames);
        // 内置音源通道处理段（CC7/10/11/71/74）在混音台 insert 链之前生效。
        self.render_channel_dsp(frames);
        // 暂停/停止的淡出作用在**干声输入**（insert 链之前）：效果器尾音
        // 不会被一起淡掉，之后可继续自然衰减。
        self.apply_pause_fade_out(frames);

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

    /// 循环回绕点的去咔哒包络：回绕前 1ms 淡出、回绕后 1ms 淡入。
    ///
    /// 回绕会清空 voice（保持各后端语义一致），硬切在持续音上会产生 click。
    /// 这里在通道输入侧做极短包络，insert/发送的尾音仍按原样继续。
    fn apply_loop_declick(&mut self, frames: usize) {
        let n = (self.sample_rate as usize / 1000).max(1);
        let pending = std::mem::take(&mut self.loop_fade_in_pending);
        // 取出并复用 Vec，避免每块在音频线程分配。
        let mut offsets = std::mem::take(&mut self.loop_wrap_offsets);
        if pending == 0 && offsets.is_empty() {
            self.loop_wrap_offsets = offsets;
            return;
        }
        if let Some(&last) = offsets.last() {
            self.loop_fade_in_pending = (last + n).saturating_sub(frames);
        }
        for chain in self.mixer.buffers_mut() {
            let buffers: &mut yinhe_mixer::ChannelBuffers = &mut *chain;
            for buf in [&mut buffers.left, &mut buffers.right] {
                if pending > 0 {
                    apply_fade_range(buf, 0, pending.min(frames), true);
                }
                for &at in &offsets {
                    apply_fade_range(buf, at.saturating_sub(n), at, false);
                    apply_fade_range(buf, at, (at + n).min(frames), true);
                }
            }
        }
        offsets.clear();
        self.loop_wrap_offsets = offsets;
        // 极短块（帧数小于淡入长度）时把未用完的淡入带到下一块。
        if pending > frames {
            self.loop_fade_in_pending = self.loop_fade_in_pending.max(pending - frames);
        }
    }

    /// 本块还能连续渲染多少帧，以及该 span 是否正好撞到 loop end。
    fn next_loop_span(&self, remaining: usize) -> (usize, bool) {
        if !self.loop_enabled || self.export_mode {
            return (remaining, false);
        }
        let end = self.loop_end_sample();
        let pos = self.sample_position;
        // 循环区间映射到零 sample（0 BPM 剪切段或极短区间）时，回绕无法让
        // 位置前进；继续 `rewind → span==0` 会让音频线程死循环。按普通播放处理。
        if end <= self.loop_start_sample() {
            return (remaining, false);
        }
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

    /// 暂停/停止前把干声输入线性淡出（效果器状态保留，尾音继续）。
    fn apply_pause_fade_out(&mut self, frames: usize) {
        if self.pause_fade_remaining == 0 {
            return;
        }
        let total = self.pause_fade_total.max(1) as f32;
        let mut remaining = self.pause_fade_remaining;
        for ch in 0..self.mixer.channel_count() {
            let Some(cb) = self.mixer.channel_buffers_mut(ch) else {
                continue;
            };
            let n = frames.min(cb.left.len()).min(cb.right.len());
            for f in 0..n {
                let g = if remaining > 0 {
                    remaining -= 1;
                    remaining as f32 / total
                } else {
                    0.0
                };
                cb.left[f] *= g;
                cb.right[f] *= g;
            }
        }
        self.pause_fade_remaining = remaining;
    }

    /// 停止/暂停后的尾音渲染：不推进走带、不派发音符/音频，只让混音台
    /// （通道 DSP / insert / send / bus / master）把已有状态自然衰减完。
    pub(crate) fn render_tail(&mut self, output: &mut [f32]) {
        let frames = output.len() / STEREO_CHANNELS;
        if frames == 0 {
            output.fill(0.0);
            return;
        }
        if self.mixer.frames() != frames {
            let strips = self.dense_strip_params();
            let count = self.mixer.channel_count();
            self.mixer.resize(count, frames, &strips);
            self.channel_set.resize_scratches(frames);
        }
        self.block_start_sample = self.sample_position;
        self.mixer.clear_channel_buffers();
        // 通道 DSP 的滤波历史在空输入下继续衰减；insert/bus/master 尾音同理。
        self.render_channel_dsp(frames);
        let (master_l, master_r) = self.mixer.process();
        write_master(output, master_l, master_r);
        self.publish_track_levels();
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
        self.loop_wrap_offsets.clear();
        self.loop_fade_in_pending = 0;
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
    /// dense 通道缓冲。实现见 `audio_clip::AudioClipRenderer`（M0 拆分）。
    pub(super) fn render_audio_tracks(
        &mut self,
        block_start_sample: u64,
        offset_frames: usize,
        frames: usize,
    ) {
        let Some(yin_model) = self.yin_model.clone() else {
            return;
        };
        let Some(audio_model) = self.model.as_ref() else {
            return;
        };
        let mut renderer = super::audio_clip::AudioClipRenderer {
            model: &yin_model,
            audio_model,
            sources: &self.audio_sources,
            mixer: &mut self.mixer,
            layout: &self.channel_layout,
            skip_track: &self.skip_track,
            sample_rate: self.sample_rate as f64,
        };
        renderer.render_block(block_start_sample, offset_frames, frames);
    }
}

#[cfg(test)]
mod tests {
    use super::apply_fade_range;

    /// 回绕点：淡出末样本接近 0、淡入首样本接近 0，中间单调恢复。
    #[test]
    fn loop_declick_ramps_are_monotonic() {
        let mut buf = vec![1.0f32; 96];
        apply_fade_range(&mut buf, 48, 48 + 48, true);
        assert!(buf[48] < 0.05, "淡入首样本应接近 0: {}", buf[48]);
        assert!(buf[95] > 0.95, "淡入末样本应接近 1: {}", buf[95]);
        assert!(
            buf[48..].windows(2).all(|w| w[0] <= w[1] + 1e-6),
            "淡入必须单调"
        );

        let mut buf = vec![1.0f32; 96];
        apply_fade_range(&mut buf, 0, 48, false);
        assert!(buf[47] < 0.05, "淡出末样本应接近 0: {}", buf[47]);
        assert!(
            buf[..48].windows(2).all(|w| w[0] >= w[1] - 1e-6),
            "淡出必须单调"
        );
    }
}
