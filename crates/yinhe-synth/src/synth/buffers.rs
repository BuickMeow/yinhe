//! GPU 持久缓冲与按需重建。

use wgpu::util::DeviceExt;

use super::renderer::GpuAudioRenderer;
use super::types::{
    CHANNEL_COUNT, ChState, EnvUpdateCmd, GpuVoiceState, MAX_CHUNKS, ReleaseCmd, RenderParams,
    SegInfo,
};

/// Persistent GPU state — all buffers allocated once, reused every block.
pub(crate) struct GpuBuffers {
    #[allow(dead_code)]
    pub(crate) sample_chunks: Vec<wgpu::Buffer>,
    #[allow(dead_code)]
    pub(crate) chunk_offsets_buf: wgpu::Buffer,
    pub(crate) chunk_count: u32,
    pub(crate) voice_state_buf: wgpu::Buffer,
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
    pub(crate) staging: [wgpu::Buffer; 2],
    /// 读回 voice 状态（块末全字段，作为下一块起点）
    pub(crate) staging_voice: [wgpu::Buffer; 2],
    pub(crate) staging_idx: usize,
    pub(crate) bind_groups: [wgpu::BindGroup; 2],
}

impl GpuAudioRenderer {
    pub(crate) fn ensure_buffers(
        &mut self,
        voice_count: u32,
        frame_count: u32,
        segs_len: usize,
        ch_updates_len: usize,
        releases_len: usize,
        env_cmds_len: usize,
    ) {
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
        let needs_recreate = if self.sample_chunks.is_empty() {
            return;
        } else {
            match &self.buffers {
                Some(b) => {
                    b.max_voices < rounded_voices
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

        let device = &self.device;
        let chunk_count = self.sample_chunks.len().min(MAX_CHUNKS) as u32;

        // Create sample chunk buffers
        let sample_chunks: Vec<wgpu::Buffer> = self
            .sample_chunks
            .iter()
            .map(|data| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("sample_chunk"),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                })
            })
            .collect();

        // Create chunk_offsets buffer (uniform, padded to 32 bytes = 8 u32 for 16-byte alignment)
        let mut offsets: Vec<u32> = Vec::with_capacity(8);
        let mut acc = 0u32;
        for chunk in &self.sample_chunks {
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

        // Other persistent buffers（用 rounded_voices 分配，和 max_voices 一致）
        // 其他持久 buffer（用 rounded_voices 分配，和 max_voices 一致）
        let voice_state_size =
            (rounded_voices as usize * std::mem::size_of::<GpuVoiceState>()) as u64;
        // per-channel 混音：32 通道 × frames × 2
        let channel_mix_size =
            (CHANNEL_COUNT * frame_count.max(1) as usize * 2 * std::mem::size_of::<f32>()) as u64;
        // pass1 每 voice 每帧输出（按分配的最大 voice 数）
        let partial_size = (rounded_voices as usize
            * frame_count.max(1) as usize
            * 2
            * std::mem::size_of::<f32>()) as u64;
        let params_size = std::mem::size_of::<RenderParams>() as u64;
        // 块内段/指令结构：按实际需求容量（幂等增长）分配
        let segs_size = (segs_cap * std::mem::size_of::<SegInfo>()) as u64;
        let ch_updates_size = (ch_updates_cap * std::mem::size_of::<ChState>()) as u64;
        let release_by_frame_size =
            ((frame_count.max(1) as usize + 2) * std::mem::size_of::<u32>()) as u64;
        let release_cmds_size = (releases_cap * std::mem::size_of::<ReleaseCmd>()) as u64;
        let env_cmds_size = (env_cmds_cap * std::mem::size_of::<EnvUpdateCmd>()) as u64;

        let voice_state_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_voice_states"),
            size: voice_state_size,
            // read_write：pass1 块末写回滤波器 IIR 状态
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let partial_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_partial"),
            size: partial_size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
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
        let staging0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_0"),
            size: channel_mix_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_1"),
            size: channel_mix_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // voice 状态读回（块末全字段，作为下一块起点）
        let staging_voice0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_voice_0"),
            size: voice_state_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging_voice1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging_voice_1"),
            size: voice_state_size,
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
                       ec: &wgpu::Buffer| {
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
                ),
            ],
            sample_chunks,
            chunk_offsets_buf,
            chunk_count,
            voice_state_buf,
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
            staging_voice: [staging_voice0, staging_voice1],
            staging_idx: 0,
        });
        self.frame_count = frame_count;
    }
}
