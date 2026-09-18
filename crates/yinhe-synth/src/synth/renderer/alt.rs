//! 交替渲染路径（单段整块 / 混合器输出 / 动态分配）：测试与参考路径。

use super::*;

impl GpuAudioRenderer {
    /// 单段渲染（整块作一段；测试/CPU 参考路径）。块长可超过
    /// [`RENDER_SEGMENT_FRAMES`]（partial 按实际块长分配）。
    #[allow(clippy::too_many_arguments)] // 透传上下文，见 AGENTS 约定
    pub fn render_block_single(
        &mut self,
        voice_count: u32,
        readback_states: Option<&mut [GpuVoiceState]>,
        channel_mix: &mut [f32],
        voice_stage_out: &mut [u32],
        segs: &[SegInfo],
        ch_updates: &[ChState],
        releases: &[ReleaseCmd],
        env_cmds: &[EnvUpdateCmd],
        sample_rate: u32,
    ) -> u32 {
        let frame_count = (channel_mix.len() / 2 / CHANNEL_COUNT) as u32;
        let seg = RenderSegment {
            frame_start: 0,
            frame_length: frame_count,
            segs,
            ch_updates,
            releases,
            env_cmds,
        };
        self.render_block(
            voice_count,
            readback_states,
            channel_mix,
            voice_stage_out,
            &[seg],
            sample_rate,
        )
    }

    /// Render a block of audio using the GPU（测试/便利路径）。
    /// 渲染一块音频（frames × 2 立体声交错）：per-channel 混音求和，无通道滤波。
    /// `voices` 会被更新：读回 GPU 端推进的**全字段**状态（时间/包络/滤波）。
    /// 调用方**不应再**用 CPU 推进 voice 状态（GPU 已推进）。
    /// 返回实际 voice 数量（0 表示静音）。
    ///
    /// 注意：生产路径（GpuSynth::render_to_mixer）用 `render_block` 的紧凑读回；
    /// 本方法保留全字段读回供 CPU/GPU 一致性测试使用。
    pub fn render_into(
        &mut self,
        voices: &mut [GpuVoiceState],
        output: &mut [f32],
        sample_rate: u32,
    ) -> u32 {
        let frames = output.len() / 2;
        let mut scratch = std::mem::take(&mut self.mix_scratch);
        scratch.resize(CHANNEL_COUNT * frames * 2, 0.0);
        // 测试路径：先全量上传（状态自包含），再渲染 + 全字段读回。
        self.upload_voice_states(voices);
        let mut stage = vec![0u32; voices.len()];
        // 测试路径单段：整块一次 pass1+pass2
        let seg = RenderSegment {
            frame_start: 0,
            frame_length: frames as u32,
            segs: &[],
            ch_updates: &[],
            releases: &[],
            env_cmds: &[],
        };
        let n = self.render_block(
            voices.len() as u32,
            Some(voices),
            &mut scratch,
            &mut stage,
            &[seg],
            sample_rate,
        );
        self.mix_scratch = scratch;
        output.fill(0.0);
        for ch in 0..CHANNEL_COUNT {
            let base = ch * frames * 2;
            for (i, o) in output.iter_mut().enumerate() {
                *o += self.mix_scratch[base + i];
            }
        }
        n
    }

    /// 渲染一块音频（返回新分配的 Vec，辅助测试用）。
    pub fn render_block_alloc(
        &mut self,
        voices: &mut [GpuVoiceState],
        frame_count: u32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let mut output = vec![0.0; frame_count as usize * 2];
        self.render_into(voices, &mut output, sample_rate);
        output
    }
}
