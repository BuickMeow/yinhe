//! yinhe 的 VST3 插件宿主。
//!
//! 分层与 yinhe-clap 对齐：
//! - 管理线程持有实例（IEditController 侧：参数、状态、GUI）；
//! - 渲染线程持有音频处理器（IComponent/IAudioProcessor 侧：`Send`，零分配处理）。
//!
//! 当前阶段：扫描与元数据——有 moduleinfo.json 的直接解析，旧插件走
//! factory 动态加载枚举（[`loader`] + [`factory`]，进程内加载二进制）。
//! 后续阶段：参数、音频处理（IComponent/IAudioProcessor）、状态、编辑器。

pub mod scan;

mod factory;
mod loader;
mod moduleinfo;

pub use factory::{FactoryClass, FactoryInfo};
pub use loader::{LoadError, LoadedModule};
pub use moduleinfo::{ModuleClassInfo, ModuleInfo, ModuleInfoError, read_moduleinfo};
