//! 采样插值方式：设置层（`AudioSettings`）与音频引擎共用的类型。
//!
//! 定义在 types 层而不是 settings 层，避免音频引擎反向依赖 settings crate。

use serde::{Deserialize, Serialize};

/// 采样插值方式（变调时的音质/性能权衡）。
///
/// pitch bend / 键位变调通过改变采样播放速率实现：非整数播放位置需要插值。
/// - `Nearest`（与 xsynth `SoundfontInitOptions` 一致）：最快；位置取整——
///   向上跳采（混叠）、向下重复取点（阶梯/镜像），高移调时沙粒感明显；
/// - `Linear`（默认）：变调干净；每样本多一次读取 + 混合（352 voice 密集段
///   约 +7~8% 渲染耗时）；
/// - `Cubic`：4 点 Catmull-Rom（与音频素材同款），高移调/弯音下高频更干净；
///   每样本 4 次读取，密集段比 `Linear` 再慢约 20%。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interpolation {
    Nearest,
    #[default]
    Linear,
    Cubic,
}

impl Interpolation {
    /// 渲染器编码（`KeyInfo.interp` / WGSL `interp` 字段：0/1/2）。
    pub fn code(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Linear => 1,
            Self::Cubic => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Interpolation;

    #[test]
    fn interpolation_codes() {
        assert_eq!(Interpolation::default(), Interpolation::Linear);
        assert_eq!(Interpolation::Nearest.code(), 0);
        assert_eq!(Interpolation::Linear.code(), 1);
        assert_eq!(Interpolation::Cubic.code(), 2);
    }
}
