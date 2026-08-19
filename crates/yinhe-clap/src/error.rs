//! 插件宿主错误类型。生产路径不使用 unwrap（AGENTS.md 第 17 条）。

use clack_host::entry::PluginEntryError;
use clack_host::plugin::PluginInstanceError;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("加载插件入口失败: {0:?}")]
    Entry(#[from] PluginEntryError),

    #[error("实例化插件失败: {0:?}")]
    Instance(#[from] PluginInstanceError),

    #[error("插件包内找不到指定 id: {0}")]
    PluginIdNotFound(String),

    #[error("插件不支持参数枚举")]
    NoParamsExtension,

    #[error("保存插件状态失败")]
    StateSave,

    #[error("恢复插件状态失败")]
    StateLoad,

    #[error("音频处理失败")]
    Process,

    #[error("插件不支持 GUI 扩展")]
    GuiNoExtension,

    #[error("插件不支持本平台的 GUI（浮动窗口）")]
    GuiUnsupported,

    #[error("创建插件界面失败")]
    GuiCreate,

    #[error("插件界面嵌入宿主窗口失败")]
    GuiAttach,

    #[error("显示插件界面失败")]
    GuiShow,
}
