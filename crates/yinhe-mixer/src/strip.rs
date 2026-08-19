//! 通道条的运行时状态（渲染线程侧）。
//!
//! 与 [`crate::StripParams`] 的区别：StripParams 是可序列化的「目标值」，
//! 这里的 [`StripState`] 额外保存块内插值所需的上一块增益/声像，
//! 用于抗 zipper noise 的块内线性斜坡。

use crate::StripParams;

/// 等功率声像：pan ∈ [-1, 1] → (左增益, 右增益)。
pub(crate) fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// 单个通道条的渲染侧状态。
pub(crate) struct StripState {
    /// 目标值（本块结束时到达）。
    pub(crate) params: StripParams,
    /// 上一块结束时的增益（本块插值起点）。
    pub(crate) prev_gain: f32,
    /// 上一块结束时的左右声像增益。
    pub(crate) prev_pan: (f32, f32),
}

impl StripState {
    pub(crate) fn new(params: StripParams) -> Self {
        Self {
            params,
            prev_gain: params.gain,
            prev_pan: pan_gains(params.pan),
        }
    }

    /// 更新目标值。prev_* 保持不变，由下一块处理时斜坡过去。
    pub(crate) fn set_params(&mut self, params: StripParams) {
        self.params = params;
    }

    /// 块内逐样本处理：增益 × 声像斜坡后累加进主输出，返回是否发声（供电平表）。
    ///
    /// `audible` 由上层判定（mute/solo 逻辑），静音轨道不累加但斜坡状态照常推进，
    /// 保证 unmute 瞬间参数已是目标值、无爆音。
    pub(crate) fn accumulate(
        &mut self,
        track_l: &[f32],
        track_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        audible: bool,
    ) {
        let frames = track_l.len();
        let target_pan = pan_gains(self.params.pan);
        let gain_start = self.prev_gain;
        let gain_step = (self.params.gain - gain_start) / frames as f32;
        let pan_l_start = self.prev_pan.0;
        let pan_l_step = (target_pan.0 - pan_l_start) / frames as f32;
        let pan_r_start = self.prev_pan.1;
        let pan_r_step = (target_pan.1 - pan_r_start) / frames as f32;

        if audible {
            for i in 0..frames {
                let t = (i + 1) as f32;
                let g = gain_start + gain_step * t;
                out_l[i] += track_l[i] * g * (pan_l_start + pan_l_step * t);
                out_r[i] += track_r[i] * g * (pan_r_start + pan_r_step * t);
            }
        }

        self.prev_gain = self.params.gain;
        self.prev_pan = target_pan;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_center_is_equal_power() {
        let (l, r) = pan_gains(0.0);
        assert!((l - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((r - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn pan_hard_right_mutes_left() {
        let (l, r) = pan_gains(1.0);
        assert!(l.abs() < 1e-6);
        assert!((r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn muted_track_still_ramps_gain() {
        let mut s = StripState::new(StripParams::default());
        s.set_params(StripParams {
            gain: 0.0,
            ..StripParams::default()
        });
        let tl = [1.0f32; 4];
        let tr = [1.0f32; 4];
        let mut ol = [0.0f32; 4];
        let mut or_ = [0.0f32; 4];
        s.accumulate(&tl, &tr, &mut ol, &mut or_, false);
        assert!(ol.iter().all(|&v| v == 0.0));
        // 斜坡照常推进：下一块已是目标增益。
        assert_eq!(s.prev_gain, 0.0);
    }
}
