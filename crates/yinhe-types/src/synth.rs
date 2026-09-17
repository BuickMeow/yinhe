//! 合成后端选择：设置层（`AudioSettings`）与音频引擎共用的类型。
//!
//! 定义在 types 层而不是 settings 层，避免音频引擎反向依赖 settings crate。

use serde::{Deserialize, Serialize};

/// 合成后端选择（设置/UI 层）。
///
/// - `XSynthCpu`：xsynth-core 的 CPU 引擎（当前默认，成熟）；
/// - `YinheCpu`：yinhe-synth 的 CPU 引擎（自研，对等 GpuSynth）；
/// - `YinheGpu`：yinhe-synth 的 wgpu GPU 引擎（块长 4096）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SynthEngine {
    #[default]
    XSynthCpu,
    YinheCpu,
    YinheGpu,
}

impl SynthEngine {
    /// 实际可渲染的后端。三个后端均已实现，选择即实际后端；
    /// 编译期可用性（无 `gpu` feature 时无 yinhe-synth）由 spawn 入口收敛。
    pub fn resolved(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::SynthEngine;

    #[test]
    fn resolved_is_identity() {
        assert_eq!(SynthEngine::YinheCpu.resolved(), SynthEngine::YinheCpu);
        assert_eq!(SynthEngine::YinheGpu.resolved(), SynthEngine::YinheGpu);
        assert_eq!(SynthEngine::XSynthCpu.resolved(), SynthEngine::XSynthCpu);
    }
}
