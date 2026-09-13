//! yinhe 的 VST3 插件宿主。
//!
//! 分层与 yinhe-clap 对齐：
//! - 管理线程持有实例（IEditController 侧：参数、状态、GUI）；
//! - 渲染线程持有音频处理器（IComponent/IAudioProcessor 侧：`Send`，零分配处理）。
//!
//! 当前阶段：扫描/元数据（[`scan`]）+ 实例创建与参数枚举（[`instance`]）。
//! 后续阶段：音频处理（IComponent/IAudioProcessor）、状态、编辑器。

pub mod scan;

mod event_list;
mod events;
mod factory;
mod host;
mod instance;
mod loader;
mod moduleinfo;
mod processor;
mod stream;

pub use factory::{FactoryClass, FactoryInfo};
pub use host::restart_flags;
pub use instance::{InstanceError, Vst3ParamInfo, Vst3PluginInstance};
pub use loader::{LoadError, LoadedModule};
pub use moduleinfo::{ModuleClassInfo, ModuleInfo, ModuleInfoError, read_moduleinfo};
pub use processor::{ProcessError, Vst3Insert, Vst3Processor};
