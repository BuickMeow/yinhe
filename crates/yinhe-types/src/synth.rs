//! 合成后端选择：设置层（`AudioSettings`）与音频引擎共用的类型。
//!
//! 定义在 types 层而不是 settings 层，避免音频引擎反向依赖 settings crate。

use serde::{Deserialize, Serialize};

/// 合成后端选择（设置/UI 层）。
///
/// - `XSynthCpu`：xsynth-core 的 CPU 引擎（当前默认，成熟）；
/// - `YinheCpu`：yinhe-synth 的 CPU 引擎（自研，规划中——未实现时
///   spawn 会明确告警并回退 `XSynthCpu`，不静默假装成功）；
/// - `YinheGpu`：yinhe-synth 的 wgpu GPU 引擎（块长 4096）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SynthEngine {
    #[default]
    XSynthCpu,
    YinheCpu,
    YinheGpu,
}

impl SynthEngine {
    /// 实际可渲染的后端：`YinheCpu` 尚未实现，回退到 `XSynthCpu`。
    pub fn resolved(self) -> Self {
        match self {
            Self::YinheCpu => Self::XSynthCpu,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SynthEngine;

    #[test]
    fn unimplemented_cpu_engine_falls_back() {
        assert_eq!(SynthEngine::YinheCpu.resolved(), SynthEngine::XSynthCpu);
        assert_eq!(SynthEngine::YinheGpu.resolved(), SynthEngine::YinheGpu);
        assert_eq!(SynthEngine::XSynthCpu.resolved(), SynthEngine::XSynthCpu);
    }
}
