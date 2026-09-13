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

    /// 块内逐样本处理：增益 × 声像斜坡**原地**应用到缓冲。
    ///
    /// 调用方负责把结果累加到 master/bus（audible 判定由上层处理）；
    /// 静音轨道也照常推进斜坡状态，保证 unmute 瞬间参数已是目标值、无爆音。
    pub(crate) fn apply_fader(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len();
        let target_pan = pan_gains(self.params.pan);
        let gain_start = self.prev_gain;
        let gain_step = (self.params.gain - gain_start) / frames as f32;
        let pan_l_start = self.prev_pan.0;
        let pan_l_step = (target_pan.0 - pan_l_start) / frames as f32;
        let pan_r_start = self.prev_pan.1;
        let pan_r_step = (target_pan.1 - pan_r_start) / frames as f32;

        for i in 0..frames {
            let t = (i + 1) as f32;
            let g = gain_start + gain_step * t;
            left[i] *= g * (pan_l_start + pan_l_step * t);
            right[i] *= g * (pan_r_start + pan_r_step * t);
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
    fn fader_ramps_to_target_and_keeps_state() {
        let mut s = StripState::new(StripParams::default());
        s.set_params(StripParams {
            gain: 0.0,
            ..StripParams::default()
        });
        let mut l = [1.0f32; 4];
        let mut r = [1.0f32; 4];
        s.apply_fader(&mut l, &mut r);
        // 斜坡末到达目标增益 0（0.0 增益输出全零）。
        assert_eq!(l[3], 0.0);
        assert_eq!(r[3], 0.0);
        // 斜坡状态照常推进：下一块起点已是目标增益。
        assert_eq!(s.prev_gain, 0.0);
    }

    #[test]
    fn fader_applies_pan_and_gain_in_place() {
        let mut s = StripState::new(StripParams::default());
        let mut l = [1.0f32; 2];
        let mut r = [1.0f32; 2];
        s.apply_fader(&mut l, &mut r);
        let c = core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[1] - c).abs() < 1e-6);
        assert!((r[1] - c).abs() < 1e-6);
    }
}
