//! 渲染块的提交/收割（流水线两阶段）与同步包装、读回丢弃。
//!
//! 拆自 renderer.rs（文件过长），含 `PendingReadback`。

use super::*;
use crate::synth::types::MIX_FRAMES_PER_WG;

/// 已提交未收割的读回（`submit_block` → `finish_block`；流水线用）。
pub struct PendingReadback {
    /// 本块最后一段的提交序号：finish_block 用 `PollType::Wait` 精确等待它，
    /// 避免 `yield_now` 轮询受 OS 调度粒度影响（小块实测固定 18ms 开销）。
    submission_index: wgpu::SubmissionIndex,
    rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    /// 提交该块时使用的 staging 缓冲（直接持有引用，而非索引）。
    /// `ensure_buffers` 每块重建 staging 数组并把 `staging_idx` 归零——
    /// 按索引读会让在途块读到"当前数组"里的另一只/未写入缓冲（偶发整块静音、
    /// 交替断续的真身）。
    staging_buf: wgpu::Buffer,
    mix_size: usize,
    full_size: usize,
    want_full: bool,
    voice_count: u32,
}

impl GpuAudioRenderer {
    /// 渲染一块音频的 per-channel 混音（32 通道 × frames × 2 f32，立体声交错）。
    ///
    /// **分段渲染**：外层块在内部按 [`RenderSegment`] 切段，每段独立跑 pass1
    /// （voice 状态推进 + partial）与 pass2（归约到该段的 channel_mix 区间）；
    /// partial 只按段长上界 [`RENDER_SEGMENT_FRAMES`] 分配（满容量 8192 voice
    /// 也只有 ~32MB，加载阶段预热后不再重建）。每段的 uniform/指令写入后立即
    /// submit（缓冲只有一份，避免后段覆盖前段），最后一段统一读回，
    /// CPU↔GPU 往返仍为一次。
    ///
    /// **voice 状态常驻 GPU**：新 voice 的状态由调用方在 dispatch 前通过
    /// [`write_voice_state`] 写入槽位；本方法读回全字段状态供调用方镜像。
    ///
    /// 返回实际 voice 数量（0 表示静音）。
    #[allow(clippy::too_many_arguments)] // 渲染上下文透传，见 AGENTS 约定
    /// 提交一个块（流水线：写完命令 + submit + 发起 map，**不等待**）。
    ///
    /// 返回 `None` 表示无 GPU 缓冲/零 voice/零段（调用方输出静音）；
    /// 否则用 [`Self::finish_block`] 收割（此时再等 GPU 完成）。
    /// 与 `render_block` 的区别：提交与等待分离，让下一块的 CPU 准备与
    /// 本块的 GPU 执行重叠。
    #[allow(clippy::too_many_arguments)] // 透传上下文，见 AGENTS 约定
    pub fn submit_block(
        &mut self,
        voice_count: u32,
        frame_count: u32,
        want_full: bool,
        segments: &[RenderSegment<'_>],
        sample_rate: u32,
    ) -> Option<PendingReadback> {
        let voice_count = voice_count.min(crate::synth::buffers::MAX_VOICE_SLOTS);
        if voice_count == 0 || frame_count == 0 || segments.is_empty() {
            return None;
        }

        // 容量按所有段的最大需求（避免逐段扩容重建）
        let max_segs = segments.iter().map(|s| s.segs.len()).max().unwrap_or(0);
        let max_ch = segments
            .iter()
            .map(|s| s.ch_updates.len())
            .max()
            .unwrap_or(0);
        let max_rel = segments.iter().map(|s| s.releases.len()).max().unwrap_or(0);
        let max_env = segments.iter().map(|s| s.env_cmds.len()).max().unwrap_or(0);
        let partial_frames = segments.iter().map(|s| s.frame_length).max().unwrap_or(0);
        self.ensure_buffers(&BufferSpec {
            voice_count,
            frame_count,
            partial_frames,
            segs_len: max_segs,
            ch_updates_len: max_ch,
            releases_len: max_rel,
            env_cmds_len: max_env,
        });
        // 未 upload 采样时（音色库为空）直接输出静音，绝不 panic
        self.buffers.as_ref()?;
        // 首块/重建后：把待写槽位 flush（buffer 就绪前调用的 write 不丢）。
        self.flush_pending_voice_writes();

        let voice_wg_count = voice_count.div_ceil(WORKGROUP_SIZE);

        let mix_size =
            (CHANNEL_COUNT * frame_count as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let full_size = (voice_count as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
        let last = segments.len() - 1;

        // 全部段共用同一个 command encoder，段循环结束后一次性 submit：
        // 原实现每段一次 queue.submit（8 次/块），命令缓冲提交与 pass 调度的
        // 固定开销 ~11ms/段（预热哑渲染 1 voice 也要 29ms 即铁证），与 voice
        // 数无关——合并后整个块的固定开销只付一次。
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("audio_render"),
            });

        // voice 状态：flush 已把状态紧凑写入 staging 的当前轮转区域，
        // 这里一次 scatter dispatch 把脏槽位散写进 voice_state_buf
        // （只写 CPU 指定的槽位，不覆盖 GPU 推进中的其他槽位；多块在途安全）。
        // 计数按"每块一次性消费"语义 take：编码后清零，空闲块不会重放。
        let scatter_count = std::mem::take(&mut self.scatter_count);
        let scatter_items_base = self.scatter_items_base;
        if scatter_count > 0 {
            let buf = self.buffers.as_ref()?;
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scatter_pass"),
                ..Default::default()
            });
            cpass.set_pipeline(&self.scatter_pipeline);
            cpass.set_bind_group(0, &buf.bind_groups[0], &[]);
            cpass.dispatch_workgroups(scatter_count.div_ceil(WORKGROUP_SIZE), 1, 1);
        }

        for (seg_i, seg) in segments.iter().enumerate() {
            if seg.frame_length == 0 {
                continue;
            }
            let Some(buf) = self.buffers.as_ref() else {
                break;
            };
            let idx = buf.staging_idx;
            // release 按帧前缀和（段内帧；scratch 复用，clear 后全量填零）
            self.release_by_frame_scratch.clear();
            self.release_by_frame_scratch
                .resize(seg.frame_length as usize + 2, 0);
            for r in seg.releases {
                self.release_by_frame_scratch[r.frame as usize + 1] += 1;
            }
            for i in 1..self.release_by_frame_scratch.len() {
                self.release_by_frame_scratch[i] += self.release_by_frame_scratch[i - 1];
            }
            let params = RenderParams {
                frame_count: seg.frame_length,
                voice_count,
                sample_rate,
                sample_chunk_count: buf.chunk_count,
                voice_wg_count,
                seg_count: seg.segs.len() as u32,
                release_count: seg.releases.len() as u32,
                env_update_count: seg.env_cmds.len() as u32,
                partial_stride: partial_frames,
                channel_mix_frames: frame_count,
                mix_offset: seg.frame_start,
                active_count: seg.active_count,
                ranges_off: crate::synth::buffers::MAX_VOICE_SLOTS,
                scatter_count,
                scatter_items_base,
                _pad: 0,
            };
            self.queue
                .write_buffer(&buf.active_buf, 0, bytemuck::cast_slice(seg.active_data));
            self.queue.write_buffer(
                &buf.active_buf,
                crate::synth::buffers::MAX_VOICE_SLOTS as u64 * 4,
                bytemuck::cast_slice(seg.active_ranges),
            );
            self.queue
                .write_buffer(&buf.segs_bufs[seg_i], 0, bytemuck::cast_slice(seg.segs));
            self.queue.write_buffer(
                &buf.ch_updates_bufs[seg_i],
                0,
                bytemuck::cast_slice(seg.ch_updates),
            );
            self.queue.write_buffer(
                &buf.release_by_frame_bufs[seg_i],
                0,
                bytemuck::cast_slice(&self.release_by_frame_scratch),
            );
            self.queue.write_buffer(
                &buf.release_cmds_bufs[seg_i],
                0,
                bytemuck::cast_slice(seg.releases),
            );
            self.queue.write_buffer(
                &buf.env_cmds_bufs[seg_i],
                0,
                bytemuck::cast_slice(seg.env_cmds),
            );
            self.queue
                .write_buffer(&buf.params_bufs[seg_i], 0, bytemuck::bytes_of(&params));

            // pass1：每线程一个 voice，串行推进本段所有帧
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("voice_pass"),
                    ..Default::default()
                });
                cpass.set_pipeline(&self.pipeline);
                cpass.set_bind_group(0, &buf.bind_groups[seg_i], &[]);
                cpass.dispatch_workgroups(voice_wg_count, 1, 1);
            }
            // pass2：每帧一个 workgroup，把 partial 归约到 channel_mix 本段区间
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("mix_pass"),
                    ..Default::default()
                });
                cpass.set_pipeline(&self.mix_pipeline);
                cpass.set_bind_group(0, &buf.bind_groups[seg_i], &[]);
                cpass.dispatch_workgroups(seg.frame_length.div_ceil(MIX_FRAMES_PER_WG), 1, 1);
            }
            // 最后一段：读回 channel_mix（整块）+ 全字段 voice states，
            // 由紧随其后的 submit 一并执行（此后不再有段需要该缓冲）。
            if seg_i == last {
                encoder.copy_buffer_to_buffer(
                    &buf.channel_mix_buf,
                    0,
                    &buf.staging[idx],
                    0,
                    mix_size,
                );
                if want_full {
                    encoder.copy_buffer_to_buffer(
                        &buf.voice_state_buf,
                        0,
                        &buf.staging[idx],
                        buf.staging_full_offset,
                        full_size,
                    );
                }
            }
        }
        let submit_index = Some(self.queue.submit(std::iter::once(encoder.finish())));

        // 分配 staging（双缓冲轮转；收割时 unmap 后归还）
        let idx = self.buffers.as_ref()?.staging_idx;
        if let Some(b) = self.buffers.as_mut() {
            b.staging_idx = (idx + 1) % crate::synth::buffers::PIPELINE_DEPTH;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let staging_buf = {
            let buf = self.buffers.as_ref()?;
            let buffer_slice = buf.staging[idx].slice(..);
            buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            buf.staging[idx].clone()
        };
        Some(PendingReadback {
            submission_index: submit_index?,
            rx: receiver,
            staging_buf,
            mix_size: mix_size as usize,
            full_size: full_size as usize,
            want_full,
            voice_count,
        })
    }

    /// 收割 [`Self::submit_block`] 的读回（等待 GPU 完成 + 拷贝 + unmap）。
    pub fn finish_block(
        &mut self,
        pending: &PendingReadback,
        channel_mix: &mut [f32],
        _voice_stage_out: &mut [u32],
        readback_states: Option<&mut [GpuVoiceState]>,
    ) -> u32 {
        let Some(buf) = self.buffers.as_ref() else {
            channel_mix.fill(0.0);
            return 0;
        };
        // 只等**这一块**完成：`PollType::Wait { submission_index }` 精确等待本块
        // 最后一段的提交（不带动后续在途块，保留流水线重叠）。原实现用
        // `poll(Poll)` + `yield_now()` 忙等：macOS 调度粒度会把每轮 yield 拉长
        // 到 ms 级，小块（441 帧）实测固定 18ms 开销。
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(pending.submission_index.clone()),
            timeout: None,
        });
        // Wait 返回后 map 回调必然已触发；失败（设备丢失等）输出静音保命
        match pending.rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => {
                channel_mix.fill(0.0);
                return 0;
            }
        }
        let buffer_slice = pending.staging_buf.slice(..);
        let data = match buffer_slice.get_mapped_range() {
            Ok(d) => d,
            Err(_) => {
                channel_mix.fill(0.0);
                return 0;
            }
        };
        // channel_mix
        let mix_bytes = pending.mix_size.min(data.len());
        let gpu_mix: &[f32] = bytemuck::cast_slice(&data[..mix_bytes]);
        let n_mix = gpu_mix.len().min(channel_mix.len());
        channel_mix[..n_mix].copy_from_slice(&gpu_mix[..n_mix]);
        // 全字段读回
        if let (Some(out), true) = (readback_states, pending.want_full) {
            let full_start = buf.staging_full_offset as usize;
            let full_end = (full_start + pending.full_size).min(data.len());
            let states: &[GpuVoiceState] = bytemuck::cast_slice(&data[full_start..full_end]);
            let n = states.len().min(out.len());
            out[..n].copy_from_slice(&states[..n]);
        }
        drop(data);
        pending.staging_buf.unmap();
        pending.voice_count
    }

    /// 提交并立即收割（等价旧的同步路径；供测试/参考路径使用）。
    #[allow(clippy::too_many_arguments)] // 透传上下文，见 AGENTS 约定
    pub fn render_block(
        &mut self,
        voice_count: u32,
        readback_states: Option<&mut [GpuVoiceState]>,
        channel_mix: &mut [f32],
        voice_stage_out: &mut [u32],
        segments: &[RenderSegment<'_>],
        sample_rate: u32,
    ) -> u32 {
        let frame_count = (channel_mix.len() / 2 / CHANNEL_COUNT) as u32;
        let want_full = readback_states.is_some();
        let Some(pending) =
            self.submit_block(voice_count, frame_count, want_full, segments, sample_rate)
        else {
            channel_mix.fill(0.0);
            return 0;
        };
        self.finish_block(&pending, channel_mix, voice_stage_out, readback_states)
    }

    /// 丢弃一个已提交未收割的块（等待完成 + unmap，不取数据）。
    pub fn discard_block(&mut self, pending: &PendingReadback) {
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let _ = pending.rx.recv();
        if pending.staging_buf.slice(..).get_mapped_range().is_ok() {
            // get_mapped_range 的借用在这里结束，立即 unmap
        }
        pending.staging_buf.unmap();
    }
}
