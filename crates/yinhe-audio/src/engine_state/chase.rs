//! Chase（控制状态快照）应用：seek 后恢复 CC/PitchBend/RPN 与插件参数状态。

use yinhe_mixer::PluginEvent;

use crate::channel::{ChannelState, ChaseSkip};
use crate::engine::AudioEngine;

impl AudioEngine {
    /// 方案 B：应用 worker 线程异步算好的 256 通道状态快照。
    /// 在 `seek_to` 之后由 renderer 收到 `ChaseResult` 时调用，恢复各通道的
    /// volume / pan / program / pitch bend / RPN 等控制器值。
    ///
    /// chase 是异步的：结果到达时渲染器可能已经 dispatch 了 seek 点之后的
    /// 实时事件（包括 seek 点同 sample 的事件）。若整体覆盖，这些新值会被
    /// 打回 seek 前的旧值（从中间小节开始播放时 PBS/PitchBend 被覆盖的根因）。
    /// 因此对 `[chase_cc_base, cc_cursor)` 区间内已 dispatch 的控制器跳过，
    /// 只补齐尚未被实时事件覆盖的状态。
    /// 构建 chase 跳过掩码：`dispatched_skip` 中自 seek 以来实际发送的控制器。
    /// dispatch 在发送每个 CC/PB/RPN/PC 事件时打点，`seek_to` 清零；
    /// 与旧实现（按 `[chase_cc_base, cc_cursor)` 区间扫描）不同，mute 期间
    /// 被越过但未发送的事件不会被误标——unmute 后 chase 能恢复这些控制器。
    pub(crate) fn chase_skip(&self) -> ChaseSkip {
        self.dispatched_skip
    }

    pub(crate) fn apply_chase_result(
        &mut self,
        states: &[Option<ChannelState>; 256],
        plugin_params: &[(u8, u32, f32)],
    ) {
        let skip = self.chase_skip();
        for ch in 0..256u32 {
            let dense = self.channel_layout.dense_for(ch as usize);
            if dense == u32::MAX {
                continue;
            }
            // 无事件通道（如被 mute 轨独占的通道）不触碰：保持当前状态不重置。
            let Some(state) = &states[ch as usize] else {
                continue;
            };
            // GPU 模式的控制器恢复走 `apply_gpu_chase`（本函数末尾），
            // ChannelSet 不参与渲染。
            if self.cpu_synth_active() {
                state.send_to(dense, &mut self.channel_set, &skip);
            }
            // 内置音源通道处理段回填（与 xsynth 的 skip 语义一致：已被
            // dispatch 的 CC 不覆盖，避免旧值打回新值）。插件通道的 CC 由
            // 插件实例自身处理（透传语义），不经处理段。
            if (dense as usize) < self.channel_layout.midi_compacted() as usize
                && self
                    .instruments
                    .get(dense as usize)
                    .is_none_or(|s| s.is_none())
                && let Some(chain) = self.channel_dsp.get_mut(dense as usize)
            {
                for &cc in yinhe_dsp::cc::DSP_CHANNEL_CCS {
                    if skip.cc_mask[ch as usize] & (1u128 << cc) == 0 {
                        chain.apply_cc(cc, state.dsp_cc_value(cc));
                    }
                }
            }
        }
        // 插件参数 chase：seek 后插件已 reset（值丢失），把目标位置的
        // lane 值写回对应乐器实例（归一化值，下一块 process 生效）。
        for &(ch, param_id, value) in plugin_params {
            let Some(dense) = self.channel_plugin_dense(ch) else {
                continue;
            };
            if let Some(Some(slot)) = self.instruments.get_mut(dense) {
                slot.events.push(PluginEvent::ParamValue {
                    time: 0,
                    param_id,
                    value: f64::from(value),
                });
            }
        }
        // GPU 后端：同一份快照应用到 GpuSynth（skip 翻译与 dense 索引见 apply_gpu_chase）。
        #[cfg(feature = "gpu")]
        self.apply_gpu_chase(states);
    }
}
