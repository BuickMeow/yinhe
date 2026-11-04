//! 对数（几何级数）缩放网格：把 [min, max] 分成整数档，边界正好落在档位上。
//!
//! 直接用 `h * ratio` 再 clamp 时，如果 max 不是 ratio 的整数倍，最后一档
//! 会被压到 max；反向缩放会从 max 直接跳到 max/ratio，丢掉边界附近的档位。
//! 这里用 `ratio = (max/min)^(1/steps)`，保证 min 和 max 都是精确档位。

#[derive(Clone, Copy, Debug)]
pub(crate) struct LogZoomGrid {
    min: f32,
    ratio: f32,
    steps: u32,
}

impl LogZoomGrid {
    pub(crate) fn new(min: f32, max: f32, target_ratio: f32) -> Option<Self> {
        if !min.is_finite() || !max.is_finite() || min <= 0.0 || max <= min {
            return None;
        }
        let target = target_ratio.max(1.0001);
        let total = max / min;
        let steps = ((total.ln() / target.ln()).round() as i64).max(1) as u32;
        let ratio = total.powf(1.0 / steps as f32);
        (ratio > 1.0).then_some(Self { min, ratio, steps })
    }

    fn index(&self, height: f32) -> i32 {
        let h = height.max(self.min);
        let k = (h / self.min).ln() / self.ratio.ln();
        (k.round() as i32).clamp(0, self.steps as i32)
    }

    fn height(&self, index: i32) -> f32 {
        let k = index.clamp(0, self.steps as i32) as f32;
        self.min * self.ratio.powf(k)
    }

    /// 从当前高度移动 `dir` 档（+1 放大 / -1 缩小）。
    pub(crate) fn step(&self, height: f32, dir: i32) -> f32 {
        self.height(self.index(height) + dir)
    }
}
