//! yinhe 的 CLAP 插件宿主。
//!
//! 分层：
//! - [`scan`]：目录扫描，产出 [`PluginInfo`] 列表（只读 factory 元数据，不实例化）。
//! - [`instance`]：[`ClapPluginInstance`]，主线程侧：加载、参数枚举、状态存取。
//! - [`processor`]：[`ClapProcessor`]，渲染线程侧：`Send`，处理期间零分配。
//! - [`events`]：yinhe 风格 MIDI/参数事件 → CLAP 事件的转换。
//!
//! 线程模型：实例的加载/激活/状态读写/参数枚举都在插件管理线程做；
//! 激活后得到 [`ClapProcessor`]，move 进渲染线程使用。

mod describe;
mod error;
mod events;
mod host;
mod instance;
mod processor;
pub mod scan;

pub use describe::PluginInfo;
pub use error::PluginError;
pub use host::YinheHost;
pub use events::ClapInputEvent;
pub use instance::{ClapPluginInstance, ParamDescriptor};
pub use processor::ClapProcessor;
