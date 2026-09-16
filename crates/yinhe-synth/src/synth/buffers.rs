//! GPU 持久缓冲与按需重建。

use wgpu::util::DeviceExt;

use super::renderer::GpuAudioRenderer;
use super::types::{
    CHANNEL_COUNT, CHUNK_SIZE, ChState, EnvUpdateCmd, GpuVoiceState, MAX_CHUNKS, ReleaseCmd,
    RenderParams, SegInfo,
};

/// GPU voice 槽位上限：voice 状态常驻 GPU（不再每块读回/重传），
/// 槽位固定分配一次，避免扩容重建导致状态丢失。超限由 GpuSynth 侧压缩/淘汰。
pub(crate) const MAX_VOICE_SLOTS: u32 = 8192;

/// `ensure_buffers` 的容量需求（整块帧数与 partial 段长分开：分段渲染时
/// partial 只按段长上界分配，channel_mix/staging 按整块）。
pub(crate) struct BufferSpec {
    pub(crate) voice_count: u32,
    /// 整块帧数（channel_mix / staging）
    pub(crate) frame_count: u32,
    /// partial 的每 voice 帧容量（= 所有渲染段的最长段长）
    pub(crate) partial_frames: u32,
    pub(crate) segs_len: usize,
    pub(crate) ch_updates_len: usize,
    pub(crate) releases_len: usize,
    pub(crate) env_cmds_len: usize,
}

/// Persistent GPU state — all buffers allocated once, reused every block.
pub(crate) struct GpuBuffers {
    #[allow(dead_code)]
    pub(crate) sample_chunks: Vec<wgpu::Buffer>,
    #[allow(dead_code)]
    pub(crate) chunk_offsets_buf: wgpu::Buffer,
    pub(crate) chunk_count: u32,
    pub(crate) voice_state_buf: wgpu::Buffer,
    /// 固定槽位数（voice 状态常驻，扩容不重建）
    pub(crate) voice_slots: u32,
    /// 紧凑 env_stage（pass1 写、CPU 读回做 voice 清理）
    pub(crate) voice_stage_buf: wgpu::Buffer,
    /// partial 分配容量（每 voice 帧数；不足时重建）
    pub(crate) partial_frames: u32,
    /// partial 分配使用的 voice 容量（releases/env_cmds cap 用）
    pub(crate) max_voices: u32,
    /// 段/指令缓冲的容量（按块内实际需求幂等增长）
    pub(crate) segs_cap: usize,
    pub(crate) ch_updates_cap: usize,
    pub(crate) releases_cap: usize,
    pub(crate) env_cmds_cap: usize,
    /// per-channel 混音输出（32 通道 × frames × 2 f32），pass2 写入
    pub(crate) channel_mix_buf: wgpu::Buffer,
    pub(crate) params_buf: wgpu::Buffer,
    /// pass1 每 voice 每帧输出（voices × frames × 2 f32）
    #[allow(dead_code)] // 经 bind_groups 使用
    pub(crate) partial_buf: wgpu::Buffer,
    pub(crate) segs_buf: wgpu::Buffer,
    pub(crate) ch_updates_buf: wgpu::Buffer,
    pub(crate) release_by_frame_buf: wgpu::Buffer,
    pub(crate) release_cmds_buf: wgpu::Buffer,
    pub(crate) env_cmds_buf: wgpu::Buffer,
    /// 读回 staging（一次 map：先 channel_mix 后 voice_stage）
    pub(crate) staging: [wgpu::Buffer; 2],
    /// staging 中 voice_stage 区的字节偏移（= channel_mix_size）
    pub(crate) staging_stage_offset: u64,
    /// staging 中全字段 voice states 区的字节偏移（测试路径用；生产不 copy）
    pub(crate) staging_full_offset: u64,
    pub(crate) staging_idx: usize,
    pub(crate) bind_groups: [wgpu::BindGroup; 2],
}

impl GpuAudioRenderer {
    pub(crate) fn ensure_buffers(&mut self, spec: &BufferSpec) {
        let BufferSpec {
            voice_count,
            frame_count,
            partial_frames,
            segs_len,
            ch_updates_len,
            releases_len,
            env_cmds_len,
        } = *spec;
        // voice 数超槽位上限：调用方（GpuSynth）负责压缩/淘汰；这里仅防御。
        let voice_count = voice_count.min(MAX_VOICE_SLOTS);
        // 幂增长策略：向上取整到 2 的幂次，避免每个 block 都重建缓冲区
        let rounded_voices = voice_count.max(64).next_power_of_two();
        // 指令/段缓冲按实际需求（块内事件数 × voice 数）分配，与 voice/帧数无关：
        // 密集 CC（每帧多事件 × 每通道多活跃 voice）可远超 voice 数，固定上界会越界。
        let segs_cap = (frame_count as usize + 1).max(segs_len).next_power_of_two();
        let ch_updates_cap = (frame_count as usize * CHANNEL_COUNT)
            .max(ch_updates_len)
            .next_power_of_two();
        let releases_cap = (rounded_voices as usize)
            .max(releases_len)
            .next_power_of_two();
        let env_cmds_cap = (rounded_voices as usize)
            .max(env_cmds_len)
            .next_power_of_two();
        let needs_recreate = if self.sample_data.is_empty() {
            return;
        } else {
            match &self.buffers {
                Some(b) => {
                    b.max_voices < rounded_voices
                        || b.partial_frames < partial_frames
                        || self.frame_count < frame_count
                        || b.segs_cap < segs_cap
                        || b.ch_updates_cap < ch_updates_cap
                        || b.releases_cap < releases_cap
                        || b.env_cmds_cap < env_cmds_cap
                }
                None => true,
            }
        };
        if !needs_recreate {
            return;
        }

        let t_create = std::time::Instant::now();
        let device = &self.device;
        let chunk_count = self.sample_data.len().div_ceil(CHUNK_SIZE).min(MAX_CHUNKS) as u32;

        // 采样 chunk buffer：数据未变时复用已有 GPU buffer（voice/帧数扩容
        // 触发的重建不重传采样数据）。仅 `upload_samples` 后重建并上传一次。
        let sample_chunks: Vec<wgpu::Buffer> = match self.sample_buffers.take() {
            Some(b) => b,
            None => {
                let t_samples = std::time::Instant::now();
                let queue = &self.queue;
                let created: Vec<wgpu::Buffer> = self
                    .sample_data
                    .as_slice()
                    .chunks(CHUNK_SIZE)
                    .take(MAX_CHUNKS)
                    .map(|data| {
                        let buf = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("sample_chunk"),
                            size: std::mem::size_of_val(data) as u64,
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: true,
                        });
                        let mapped = buf.slice(..).get_mapped_range_mut();
                        match mapped {
                            Ok(mut view) => {
                                view.copy_from_slice(bytemuck::cast_slice(data));
                                drop(view);
                                buf.unmap();
                            }
                            // mapped_at_creation 立即映射，正常不会失败；退化到
                            // write_buffer 兜底保证数据仍然正确（不静默出静音）。
                            Err(_) => queue.write_buffer(&buf, 0, bytemuck::cast_slice(data)),
                        }
                        buf
                    })
                    .collect();
                eprintln!(
                    "[gpu] 采样 buffer 上传={:?}（{} chunks，{:.0}MB）",
                    t_samples.elapsed(),
                    created.len(),
                    self.sample_data.len() as f64 * 4.0 / (1024.0 * 1024.0)
                );
                #[cfg(test)]
                {
                    self.sample_upload_count += 1;
                }
                created
            }
        };
        self.sample_buffers = Some(sample_chunks.clone());

        // Create chunk_offsets buffer (uniform, padded to 32 bytes = 8 u32 for 16-byte alignment)
        let mut offsets: Vec<u32> = Vec::with_capacity(8);
        let mut acc = 0u32;
        for chunk in self
            .sample_data
            .as_slice()
            .chunks(CHUNK_SIZE)
            .take(MAX_CHUNKS)
        {
            offsets.push(acc);
            acc += chunk.len() as u32;
        }
        offsets.push(acc); // total = sentinel
        // Pad to exactly 8 entries for WGSL struct alignment
        while offsets.len() < 8 {
            offsets.push(0);
        }
        let chunk_offsets_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_offsets"),
            contents: bytemuck::cast_slice(&offsets),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // voice 状态/紧凑 stage：固定 MAX_VOICE_SLOTS 分配并**跨重建复用**
        // （voice 状态常驻 GPU；扩容 partial 等缓冲时不能丢状态）。
        let slots = MAX_VOICE_SLOTS as usize;
        let voice_state_size = (slots * std::mem::size_of::<GpuVoiceState>()) as u64;
        let voice_stage_size = (slots * std::mem::size_of::<u32>()) as u64;
        // pass1 每 voice 每帧输出：分段渲染只按段长上界分配（与整块帧数无关，
        // 满容量 8192 voice + 512 帧段长也仅 ~32MB，且跨重建复用）
        let partial_size =
            (slots * partial_frames as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let (voice_state_buf, voice_stage_buf, partial_buf) = match self.buffers.take() {
            Some(b) if b.voice_slots >= MAX_VOICE_SLOTS && b.partial_frames >= partial_frames => {
                (b.voice_state_buf, b.voice_stage_buf, b.partial_buf)
            }
            _ => (
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu_voice_states"),
                    size: voice_state_size,
                    // read_write：pass1 块末写回；COPY_DST：新 voice 槽位上传
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu_voice_stage"),
                    size: voice_stage_size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu_partial"),
                    size: partial_size,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }),
            ),
        };
        // per-channel 混音：32 通道 × frames × 2
        let channel_mix_size =
            (CHANNEL_COUNT * frame_count.max(1) as usize * 2 * std::mem::size_of::<f32>()) as u64;
        let params_size = std::mem::size_of::<RenderParams>() as u64;
        // 块内段/指令结构：按实际需求容量（幂等增长）分配
        let segs_size = (segs_cap * std::mem::size_of::<SegInfo>()) as u64;
        let ch_updates_size = (ch_updates_cap * std::mem::size_of::<ChState>()) as u64;
        let release_by_frame_size =
            ((frame_count.max(1) as usize + 2) * std::mem::size_of::<u32>()) as u64;
        let release_cmds_size = (releases_cap * std::mem::size_of::<ReleaseCmd>()) as u64;
        let env_cmds_size = (env_cmds_cap * std::mem::size_of::<EnvUpdateCmd>()) as u64;

        let channel_mix_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_channel_mix"),
            size: channel_mix_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_params"),
            size: params_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 读回 staging 三区（一次 map/poll）：
        // [channel_mix][voice_stage][full voice states（仅测试路径 copy，生产不读）]
        let staging_full_offset = channel_mix_size + voice_stage_size;
        let staging_size = staging_full_offset + voice_state_size;
        let staging0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_0"),
            size: staging_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_1"),
            size: staging_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 块内段结构与指令缓冲（每块 write_buffer 覆盖）
        let segs_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_segs"),
            size: segs_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ch_updates_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_ch_updates"),
            size: ch_updates_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let release_by_frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_release_by_frame"),
            size: release_by_frame_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let release_cmds_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_release_cmds"),
            size: release_cmds_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let env_cmds_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_env_cmds"),
            size: env_cmds_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Build bind group entries
        #[allow(clippy::too_many_arguments)]
        let make_bg = |p: &wgpu::Buffer,
                       v: &wgpu::Buffer,
                       f: &wgpu::Buffer,
                       co: &wgpu::Buffer,
                       sc: &[wgpu::Buffer],
                       db: &wgpu::Buffer,
                       pt: &wgpu::Buffer,
                       sg: &wgpu::Buffer,
                       cu: &wgpu::Buffer,
                       rbf: &wgpu::Buffer,
                       rc: &wgpu::Buffer,
                       ec: &wgpu::Buffer,
                       vst: &wgpu::Buffer| {
            let mut bg_entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: p.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: v.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: f.as_entire_binding(),
                },
            ];
            // 必须固定迭代 MAX_CHUNKS 次：sc 可能不足 MAX_CHUNKS，
            // 其余 binding slot 用 dummy buffer 占位（layout 要求全部填充）。
            #[allow(clippy::needless_range_loop)]
            for i in 0..MAX_CHUNKS {
                let resource = if (i as u32) < chunk_count {
                    sc[i].as_entire_binding()
                } else {
                    db.as_entire_binding()
                };
                bg_entries.push(wgpu::BindGroupEntry {
                    binding: (3 + i) as u32,
                    resource,
                });
            }
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 8,
                resource: co.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 9,
                resource: pt.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 10,
                resource: sg.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 11,
                resource: cu.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 12,
                resource: rbf.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 13,
                resource: rc.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 14,
                resource: ec.as_entire_binding(),
            });
            bg_entries.push(wgpu::BindGroupEntry {
                binding: 15,
                resource: vst.as_entire_binding(),
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("audio_bg"),
                layout: &self.bind_group_layout,
                entries: &bg_entries,
            })
        };

        self.buffers = Some(GpuBuffers {
            bind_groups: [
                make_bg(
                    &params_buf,
                    &voice_state_buf,
                    &channel_mix_buf,
                    &chunk_offsets_buf,
                    &sample_chunks,
                    &self.dummy_buf,
                    &partial_buf,
                    &segs_buf,
                    &ch_updates_buf,
                    &release_by_frame_buf,
                    &release_cmds_buf,
                    &env_cmds_buf,
                    &voice_stage_buf,
                ),
                make_bg(
                    &params_buf,
                    &voice_state_buf,
                    &channel_mix_buf,
                    &chunk_offsets_buf,
                    &sample_chunks,
                    &self.dummy_buf,
                    &partial_buf,
                    &segs_buf,
                    &ch_updates_buf,
                    &release_by_frame_buf,
                    &release_cmds_buf,
                    &env_cmds_buf,
                    &voice_stage_buf,
                ),
            ],
            sample_chunks,
            chunk_offsets_buf,
            chunk_count,
            voice_state_buf,
            voice_slots: MAX_VOICE_SLOTS,
            voice_stage_buf,
            partial_frames,
            max_voices: rounded_voices,
            segs_cap,
            ch_updates_cap,
            releases_cap,
            env_cmds_cap,
            channel_mix_buf,
            params_buf,
            partial_buf,
            segs_buf,
            ch_updates_buf,
            release_by_frame_buf,
            release_cmds_buf,
            env_cmds_buf,
            staging: [staging0, staging1],
            staging_stage_offset: channel_mix_size,
            staging_full_offset,
            staging_idx: 0,
        });
        self.frame_count = frame_count;
        eprintln!(
            "[gpu] GPU 缓冲重建={:?}（partial={:.0}MB/{}帧，frames={frame_count}）",
            t_create.elapsed(),
            partial_size as f64 / (1024.0 * 1024.0),
            partial_frames
        );
    }
}
