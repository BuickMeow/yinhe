//! 插件实例（主线程/插件管理线程侧）。
//!
//! 职责：加载、参数枚举、状态存取、激活产出 [`ClapProcessor`]。
//! 本类型的所有方法都不应在渲染线程调用（可能分配内存、可能阻塞）。

use std::ffi::CString;

use clack_extensions::params::{ParamInfoBuffer, PluginParams};
use clack_extensions::state::PluginState;
use clack_host::entry::PluginEntry;
use clack_host::host::HostInfo;
use clack_host::plugin::PluginInstance;
use clack_host::process::{PluginAudioConfiguration, StoppedPluginAudioProcessor};

use crate::describe::PluginInfo;
use crate::error::PluginError;
use crate::host::{YinheHost, YinheMainThread, YinheShared};
use crate::processor::ClapProcessor;

/// 插件参数描述（供 egui 通用参数面板使用）。
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDescriptor {
    pub id: u32,
    pub name: String,
    /// 模块路径，如 "Oscillators/Wavetable 1"，可用 `/` 分层。
    pub module: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    /// 是否可自动化（CLAP_PARAM_INFO_IS_AUTOMATABLE）。
    pub automatable: bool,
    /// 是否只读（如插件的指示性参数）。
    pub read_only: bool,
}

/// 一个已加载（未必激活）的插件实例。
pub struct ClapPluginInstance {
    instance: PluginInstance<YinheHost>,
    info: PluginInfo,
}

impl ClapPluginInstance {
    /// 加载并实例化插件。
    ///
    /// `host_info` 由调用方构造（宿主名/版本会显示给插件）。
    pub fn load(info: &PluginInfo, host_info: &HostInfo) -> Result<Self, PluginError> {
        // SAFETY: 加载外部动态库，安全性说明见 scan 模块文档。
        let entry = unsafe { PluginEntry::load(info.path.as_os_str())? };
        let plugin_id = CString::new(info.id.as_str())
            .map_err(|_| PluginError::PluginIdNotFound(info.id.clone()))?;

        let instance = PluginInstance::<YinheHost>::new(
            |_| YinheShared::new(),
            |shared| YinheMainThread::new(shared),
            &entry,
            &plugin_id,
            host_info,
        )?;

        Ok(Self {
            instance,
            info: info.clone(),
        })
    }

    pub fn info(&self) -> &PluginInfo {
        &self.info
    }

    /// 枚举插件参数。插件不支持 params 扩展时返回空列表（不算错误，
    /// 有些乐器没有可调参数）。
    pub fn param_list(&mut self) -> Vec<ParamDescriptor> {
        let mut handle = self.instance.plugin_handle();
        let Some(params_ext) = handle.get_extension::<PluginParams>() else {
            return Vec::new();
        };
        let count = params_ext.count(&mut handle);
        let mut buffer = ParamInfoBuffer::new();
        let mut result = Vec::with_capacity(count as usize);
        for i in 0..count {
            let Some(param_info) = params_ext.get_info(&mut handle, i, &mut buffer) else {
                continue;
            };
            result.push(ParamDescriptor {
                id: param_info.id.get(),
                name: String::from_utf8_lossy(param_info.name).into_owned(),
                module: String::from_utf8_lossy(param_info.module).into_owned(),
                min_value: param_info.min_value,
                max_value: param_info.max_value,
                default_value: param_info.default_value,
                automatable: param_info
                    .flags
                    .contains(clack_extensions::params::ParamInfoFlags::IS_AUTOMATABLE),
                read_only: param_info
                    .flags
                    .contains(clack_extensions::params::ParamInfoFlags::IS_READONLY),
            });
        }
        result
    }

    /// 读取参数当前值（显示用）。
    pub fn get_param_value(&mut self, param_id: u32) -> Option<f64> {
        let id = clack_host::prelude::ClapId::from_raw(param_id)?;
        let mut handle = self.instance.plugin_handle();
        let params_ext = handle.get_extension::<PluginParams>()?;
        params_ext.get_value(&mut handle, id)
    }

    /// 保存插件状态（进工程文件）。插件不支持 state 扩展时返回 Ok(None)。
    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>, PluginError> {
        let mut handle = self.instance.plugin_handle();
        let Some(state_ext) = handle.get_extension::<PluginState>() else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        state_ext
            .save(&mut handle, &mut bytes)
            .map_err(|_| PluginError::StateSave)?;
        Ok(Some(bytes))
    }

    /// 恢复插件状态。
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), PluginError> {
        let mut handle = self.instance.plugin_handle();
        let Some(state_ext) = handle.get_extension::<PluginState>() else {
            return Ok(());
        };
        let mut cursor = std::io::Cursor::new(bytes);
        state_ext
            .load(&mut handle, &mut cursor)
            .map_err(|_| PluginError::StateLoad)
    }

    /// 激活并产出渲染线程用的处理器。
    ///
    /// `max_frames` 用引擎的实际块长；`min_frames` 给 1（我们按块处理，但允许插件
    /// 内部依赖较小 min 值的情况不存在——CLAP 只要求 host 不超过 max）。
    pub fn activate(
        &mut self,
        sample_rate: f64,
        max_frames: u32,
    ) -> Result<ClapProcessor, PluginError> {
        let stopped = self.instance.activate(
            |_, _| crate::host::YinheAudioProcessor,
            PluginAudioConfiguration {
                sample_rate,
                min_frames_count: 1,
                max_frames_count: max_frames,
            },
        )?;
        Ok(ClapProcessor::new(stopped, max_frames as usize))
    }

    /// 回收处理器并反激活。处理器应先移到本线程再调用（渲染线程 drop 后传回）。
    pub fn deactivate(&mut self, processor: ClapProcessor) {
        let stopped: StoppedPluginAudioProcessor<YinheHost> = processor.into_stopped();
        self.instance.deactivate(stopped);
    }

    /// 轮询插件的反向请求（restart/process/callback/flush），UI 每帧或管理线程循环调用。
    /// 返回 (restart, process, callback, flush) 四个标志，取出即清除。
    pub fn take_requests(&mut self) -> (bool, bool, bool, bool) {
        let result = self.instance.access_shared_handler(|shared| {
            (
                shared.take_restart(),
                shared.take_process(),
                shared.take_callback(),
                shared.take_flush(),
            )
        });
        if result.2 {
            self.instance.call_on_main_thread_callback();
        }
        result
    }

    /// 插件状态脏标记（需要重新 save_state），取出即清除。
    pub fn take_state_dirty(&mut self) -> bool {
        self.instance.access_handler_mut(|main_thread| {
            let dirty = main_thread.state_dirty;
            main_thread.state_dirty = false;
            dirty
        })
    }
}
