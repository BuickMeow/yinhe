//! yinhe 的 VST3 插件宿主。
//!
//! 分层与 yinhe-clap 对齐：
//! - 管理线程持有实例（IEditController 侧：参数、状态、GUI）；
//! - 渲染线程持有音频处理器（IComponent/IAudioProcessor 侧：`Send`，零分配处理）。
//!
//! 当前阶段：扫描与元数据（[`scan`]，只读文件，不加载二进制）。
//! 后续阶段：模块加载/factory、参数、音频处理、状态、编辑器。

pub mod scan;

mod moduleinfo;

pub use moduleinfo::{ModuleClassInfo, ModuleInfo, ModuleInfoError, read_moduleinfo};
