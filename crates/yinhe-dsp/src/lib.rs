//! yinhe 内置 DSP 效果器库。
//!
//! 设计见 `docs/spec-yinhe-dsp.md`：
//! - 效果器实现 [`yinhe_mixer::InsertProcessor`]，像插件一样挂到通道/bus/master 的 insert 链；
//! - CC 驱动模块（[`cc`]）声明自己接管的 CC，宿主据此把对应 CC 从合成器分流过来，
//!   使 xsynth 只保留音源层参数（采样/ADSR/音高/延音等）；
//! - 通过 [`BuiltinEffectKind`] 注册表供 UI 列出、加载与持久化（`InsertRef.plugin_id`）。

pub mod cc;
pub mod dsp;
mod registry;

pub use registry::BuiltinEffectKind;
