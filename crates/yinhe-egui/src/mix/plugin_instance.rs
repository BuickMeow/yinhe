//! UI 侧统一插件实例：CLAP / VST3 双格式。
//!
//! - [`PluginEntry`]：插件浏览器条目（两种格式的扫描结果统一）；
//! - [`PluginParam`]：统一参数描述（VST3 为归一化 0..1）；
//! - [`PluginInstance`]：已加载实例（管理线程持有），参数/状态/队列的统一入口；
//!   激活产出处理器由机架按格式包装（CLAP 的 ClapInsert 需要旁通/owner 状态）。

use std::path::PathBuf;
use std::sync::Arc;

use yinhe_clap::ClapPluginInstance;
use yinhe_mixer::{ParamQueue, PluginFormat};
use yinhe_vst3::Vst3PluginInstance;

use super::rack::host_info;

/// 插件浏览器条目（CLAP/VST3 扫描结果统一）。
#[derive(Clone, Debug)]
pub(crate) struct PluginEntry {
    pub format: PluginFormat,
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub is_instrument: bool,
    pub is_effect: bool,
}

/// 统一参数描述（VST3 值的语义为归一化 0..1）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PluginParam {
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub read_only: bool,
}

/// 已加载的插件实例。
pub(crate) enum PluginInstance {
    Clap(ClapPluginInstance),
    Vst3 {
        instance: Vst3PluginInstance,
        /// 显示名（VST3 实例自身不带 UI 名，持久化/扫描时记录）。
        name: String,
    },
}

impl PluginInstance {
    /// 按扫描条目加载（不激活）。
    pub fn load(entry: &PluginEntry) -> Result<Self, String> {
        match entry.format {
            PluginFormat::Clap => {
                let info = yinhe_clap::PluginInfo {
                    path: entry.path.clone(),
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    vendor: Some(entry.vendor.clone()),
                    version: None,
                    features: Vec::new(),
                };
                let instance =
                    ClapPluginInstance::load(&info, &host_info()).map_err(|e| format!("{e}"))?;
                Ok(Self::Clap(instance))
            }
            PluginFormat::Vst3 => {
                let instance =
                    Vst3PluginInstance::load(&entry.path, &entry.id).map_err(|e| format!("{e}"))?;
                Ok(Self::Vst3 {
                    instance,
                    name: entry.name.clone(),
                })
            }
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Clap(inst) => &inst.info().name,
            Self::Vst3 { name, .. } => name,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Clap(inst) => &inst.info().id,
            Self::Vst3 { instance, .. } => instance.class_id(),
        }
    }

    /// 参数枚举。
    pub fn param_list(&mut self) -> Vec<PluginParam> {
        match self {
            Self::Clap(inst) => inst
                .param_list()
                .into_iter()
                .map(|p| PluginParam {
                    id: p.id,
                    name: p.name,
                    module: p.module,
                    min: p.min_value,
                    max: p.max_value,
                    default: p.default_value,
                    read_only: p.read_only,
                })
                .collect(),
            Self::Vst3 { instance, .. } => instance
                .params()
                .iter()
                .map(|p| PluginParam {
                    id: p.id,
                    name: p.title.clone(),
                    module: p.units.clone(),
                    // VST3 参数值语义为归一化 0..1。
                    min: 0.0,
                    max: 1.0,
                    default: p.default_normalized,
                    read_only: p.flags & 0x2 != 0, // kIsReadOnly = 1<<1
                })
                .collect(),
        }
    }

    pub fn get_param_value(&mut self, id: u32) -> Option<f64> {
        match self {
            Self::Clap(inst) => inst.get_param_value(id),
            Self::Vst3 { instance, .. } => Some(instance.get_param_normalized(id)),
        }
    }

    pub fn value_to_text(&mut self, id: u32, value: f64) -> Option<String> {
        match self {
            Self::Clap(inst) => inst.value_to_text(id, value),
            Self::Vst3 { instance, .. } => instance.format_param(id, value),
        }
    }

    /// 保存状态（CLAP state / VST3 component+controller 两段）。
    /// 无状态或失败返回 None（保留工程中的旧 state）。
    pub fn save_state(&mut self) -> Option<Vec<u8>> {
        match self {
            Self::Clap(inst) => match inst.save_state() {
                Ok(Some(bytes)) => Some(bytes),
                Ok(None) => None,
                Err(e) => {
                    tracing::warn!("保存 CLAP 插件状态失败: {e}");
                    None
                }
            },
            Self::Vst3 { instance, .. } => {
                // 未激活时 getState 可能段错误（实测 Serum 2）：跳过并保留旧 state。
                if !instance.is_activated() {
                    tracing::warn!("VST3 插件未激活，跳过状态保存（激活前的 getState 不安全）");
                    return None;
                }
                Some(instance.save_state())
            }
        }
    }

    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Clap(inst) => inst.load_state(bytes).map_err(|e| format!("{e}")),
            Self::Vst3 { instance, .. } => instance.load_state(bytes).map_err(|e| format!("{e}")),
        }
    }

    /// 插件请求重扫参数（CLAP 有；VST3 参数列表静态，恒 false）。
    pub fn take_params_rescan(&mut self) -> bool {
        match self {
            Self::Clap(inst) => inst.take_params_rescan(),
            Self::Vst3 { .. } => false,
        }
    }

    pub fn param_queue(&self) -> Arc<ParamQueue> {
        match self {
            Self::Clap(inst) => inst.param_queue(),
            Self::Vst3 { instance, .. } => instance.param_queue(),
        }
    }
}
