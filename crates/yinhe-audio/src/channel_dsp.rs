//! 内置音源的通道处理段（CC7/10/11/71/74）。
//!
//! 通道处理不再由合成器内部或外挂 insert 效果器承担，而是作为"音源的一部分"
//! 在合成器输出之后、混音台 insert 链之前统一处理（CPU 侧，xsynth 与
//! GpuSynth 共用同一条路径）：
//!
//! - **内置音源通道**（未挂插件乐器）→ 本处理段消费 CC7/10/11/71/74；
//! - **插件乐器通道** → CC 原样透传插件（插件自己响应），处理段跳过。
//!
//! 两层互斥，CC 的消费者只有音源一侧：不存在"外挂效果器广播 CC"的
//! 双发语义（见 docs/spec-yinhe-dsp.md D14/D15 修订）。

use yinhe_dsp::cc::filter::ChannelFilter;
use yinhe_dsp::cc::gain::ChannelGain;
use yinhe_dsp::cc::pan::ChannelPan;
use yinhe_mixer::InsertProcessor;

/// 单个 dense 通道的 DSP 处理段（Gain → Pan → Filter，顺序即处理顺序）。
pub(crate) struct ChannelDspChain {
    gain: ChannelGain,
    pan: ChannelPan,
    filter: ChannelFilter,
}

impl ChannelDspChain {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            gain: ChannelGain::new(sample_rate),
            pan: ChannelPan::new(sample_rate),
            filter: ChannelFilter::new(sample_rate),
        }
    }

    /// 应用一条通道级 CC。7/11 → Gain、10 → Pan、71/74 → Filter，
    /// 各模块自行过滤无关 CC（调用方已按 `DSP_CHANNEL_CCS` 筛选）。
    pub(crate) fn apply_cc(&mut self, cc: u8, value: u8) {
        self.gain.apply_cc(cc, value);
        self.pan.apply_cc(cc, value);
        self.filter.apply_cc(cc, value);
    }

    /// 处理一个块的立体声采样（就地）。
    pub(crate) fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.gain.process(left, right);
        self.pan.process(left, right);
        self.filter.process(left, right);
    }

    /// seek 后清空内部状态（filter 历史等）。
    pub(crate) fn reset(&mut self) {
        self.gain.reset();
        self.pan.reset();
        self.filter.reset();
    }

    /// 暂停/停止时把待发参数送达（与 insert 的同名语义一致）。
    pub(crate) fn flush_pending_params(&mut self, position_samples: u64) {
        self.gain.flush_pending_params(position_samples);
        self.pan.flush_pending_params(position_samples);
        self.filter.flush_pending_params(position_samples);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CC7 经通道处理段衰减：输出相对"未调链"的比例等于 `(v/127)^2`
    /// （相对比较，隔离 Pan 中心的固定等功率增益）。
    #[test]
    fn gain_cc_scales_output() {
        let mut base = ChannelDspChain::new(48_000);
        let mut bl = [1.0f32; 4096];
        let mut br = [1.0f32; 4096];
        base.process(&mut bl, &mut br);

        let mut chain = ChannelDspChain::new(48_000);
        chain.apply_cc(7, 64);
        let mut l = [1.0f32; 4096];
        let mut r = [1.0f32; 4096];
        chain.process(&mut l, &mut r);

        let expect = (64.0f32 / 127.0).powi(2);
        let ratio_l = l[4095] / bl[4095];
        assert!(
            (ratio_l - expect).abs() < 1e-3,
            "左声道相对衰减应约 {expect}，实际 {ratio_l}"
        );
        let ratio_r = r[4095] / br[4095];
        assert!((ratio_r - expect).abs() < 1e-3);
    }

    /// reset 只清历史状态，已应用的 CC 值保留。
    #[test]
    fn reset_keeps_cc_values() {
        let mut base = ChannelDspChain::new(48_000);
        let mut bl = [1.0f32; 4096];
        let mut br = [1.0f32; 4096];
        base.process(&mut bl, &mut br);

        let mut chain = ChannelDspChain::new(48_000);
        chain.apply_cc(7, 64);
        chain.reset();
        let mut l = [1.0f32; 4096];
        let mut r = [1.0f32; 4096];
        chain.process(&mut l, &mut r);
        let expect = (64.0f32 / 127.0).powi(2);
        assert!((l[4095] / bl[4095] - expect).abs() < 1e-3);
    }
}
