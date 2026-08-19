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
    /// 插件 GUI 已 create（浮动窗口模型；drop 前必须 destroy，否则插件进程内资源泄漏/崩溃）。
    gui_created: bool,
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
            gui_created: false,
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
        // 查询插件声明的端口布局：必须给全所有端口（含 Aux），只给主端口
        // 会让按声明端口数读 audio_inputs[i] 的包装层越界（Element FX 实测崩）。
        let layout = self.query_port_layout();
        let stopped = self.instance.activate(
            |_, _| crate::host::YinheAudioProcessor,
            PluginAudioConfiguration {
                sample_rate,
                min_frames_count: 1,
                max_frames_count: max_frames,
            },
        )?;
        Ok(ClapProcessor::new(stopped, max_frames as usize, &layout))
    }

    /// 查询插件声明的 audio ports 布局；插件不支持 audio-ports 扩展时
    /// 回退到「一进一出各立体声」的最小布局。
    fn query_port_layout(&mut self) -> crate::processor::PortLayout {
        use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
        let mut handle = self.instance.plugin_handle();
        let Some(ports) = handle.get_extension::<PluginAudioPorts>() else {
            return crate::processor::PortLayout {
                in_channels: vec![2],
                out_channels: vec![2],
            };
        };
        let query =
            |is_input: bool, handle: &mut clack_host::plugin::PluginMainThreadHandle<'_>| {
                let count = ports.count(handle, is_input);
                let mut buf = AudioPortInfoBuffer::new();
                (0..count)
                    .map(|i| {
                        ports
                            .get(handle, i, is_input, &mut buf)
                            .map(|info| info.channel_count)
                            .unwrap_or(2)
                    })
                    .collect::<Vec<u32>>()
            };
        let in_channels = query(true, &mut handle);
        let out_channels = query(false, &mut handle);
        crate::processor::PortLayout {
            in_channels,
            out_channels,
        }
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

    /// 诊断：dump 插件声明的 audio ports（临时调试用）。
    pub fn debug_dump_ports(&mut self) -> Vec<String> {
        use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
        let mut handle = self.instance.plugin_handle();
        let Some(ports) = handle.get_extension::<PluginAudioPorts>() else {
            return vec!["(no audio-ports extension)".into()];
        };
        let mut out = Vec::new();
        for is_input in [true, false] {
            let count = ports.count(&mut handle, is_input);
            for i in 0..count {
                let mut buf = AudioPortInfoBuffer::new();
                match ports.get(&mut handle, i, is_input, &mut buf) {
                    Some(info) => out.push(format!(
                        "{}[{}]: channels={} flags={:?} type={:?} name={}",
                        if is_input { "in" } else { "out" },
                        i,
                        info.channel_count,
                        info.flags,
                        info.port_type.map(|t| format!("{:?}", t)),
                        String::from_utf8_lossy(info.name),
                    )),
                    None => out.push(format!(
                        "{}[{}]: (get failed)",
                        if is_input { "in" } else { "out" },
                        i
                    )),
                }
            }
        }
        out
    }

    /// 插件状态脏标记（需要重新 save_state），取出即清除。
    pub fn take_state_dirty(&mut self) -> bool {
        self.instance.access_handler_mut(|main_thread| {
            let dirty = main_thread.state_dirty;
            main_thread.state_dirty = false;
            dirty
        })
    }

    // ── 插件原生 GUI（浮动窗口：插件自己开/管理原生窗口）──

    /// 创建插件 GUI 资源（embedded 模式），返回插件首选尺寸（供 host 建窗）。
    ///
    /// 为什么不用浮动窗口：JUCE 系插件（Element FX 等）的 CLAP 包装层
    /// 明确拒绝 is_floating=true，只支持 set_parent 嵌入。host 自建
    /// 顶层窗口、把 content view 交给插件嵌入，是这类插件唯一的 GUI 路径。
    /// 重复调用幂等（已创建则直接返回尺寸）。
    pub fn create_gui(&mut self) -> Result<(u32, u32), PluginError> {
        use clack_extensions::gui::{GuiApiType, GuiConfiguration, PluginGui};
        let mut handle = self.instance.plugin_handle();
        let Some(gui) = handle.get_extension::<PluginGui>() else {
            return Err(PluginError::GuiNoExtension);
        };
        let Some(api_type) = GuiApiType::default_for_current_platform() else {
            return Err(PluginError::GuiUnsupported);
        };
        let config = GuiConfiguration {
            api_type,
            is_floating: false,
        };
        if !self.gui_created {
            if !gui.is_api_supported(&mut handle, config) {
                return Err(PluginError::GuiUnsupported);
            }
            gui.create(&mut handle, config)
                .map_err(|_| PluginError::GuiCreate)?;
            self.gui_created = true;
        }
        let size = gui
            .get_size(&mut handle)
            .map(|s| (s.width, s.height))
            .unwrap_or((800, 600));
        Ok(size)
    }

    /// 把插件 GUI 挂到宿主窗口的 view（macOS: NSView 指针）并显示。
    /// 调用顺序：create_gui → host 建窗 → attach_and_show_gui。
    pub fn attach_and_show_gui(
        &mut self,
        parent_view: *mut std::ffi::c_void,
    ) -> Result<(), PluginError> {
        use clack_extensions::gui::{PluginGui, Window};
        let mut handle = self.instance.plugin_handle();
        let Some(gui) = handle.get_extension::<PluginGui>() else {
            return Err(PluginError::GuiNoExtension);
        };
        if !self.gui_created {
            return Err(PluginError::GuiCreate);
        }
        // SAFETY: parent_view 必须存活到 GUI destroy。约定由调用方（机架
        // SlotRuntime 字段顺序）保证：实例先于窗口对象 drop，close_gui
        // （destroy）发生时 view 仍然有效。
        unsafe { gui.set_parent(&mut handle, Window::from_cocoa_nsview(parent_view)) }
            .map_err(|_| PluginError::GuiAttach)?;
        gui.show(&mut handle).map_err(|_| PluginError::GuiShow)?;
        Ok(())
    }

    /// 关闭并销毁插件界面（hide + destroy）。
    pub fn close_gui(&mut self) {
        if !self.gui_created {
            return;
        }
        let mut handle = self.instance.plugin_handle();
        if let Some(gui) = handle.get_extension::<clack_extensions::gui::PluginGui>() {
            let _ = gui.hide(&mut handle);
            gui.destroy(&mut handle);
        }
        self.gui_created = false;
    }

    /// 插件报告浮动窗口被用户关闭：按 CLAP 规范 host 须 destroy 一次确认。
    pub fn on_gui_closed(&mut self) {
        if !self.gui_created {
            return;
        }
        let mut handle = self.instance.plugin_handle();
        if let Some(gui) = handle.get_extension::<clack_extensions::gui::PluginGui>() {
            gui.destroy(&mut handle);
        }
        self.gui_created = false;
    }

    /// 插件侧报告过窗口关闭（取出即清除）。
    pub fn take_gui_closed(&mut self) -> bool {
        self.instance
            .access_shared_handler(|shared| shared.take_gui_closed())
    }

    /// 插件请求调整窗口尺寸（取出即清除）。
    pub fn take_gui_resize(&mut self) -> Option<(u32, u32)> {
        self.instance
            .access_shared_handler(|shared| shared.take_gui_resize())
    }
}

impl Drop for ClapPluginInstance {
    fn drop(&mut self) {
        // GUI 必须先于插件实例销毁（JUCE 等插件不 destroy GUI 直接 drop 会崩）。
        self.close_gui();
    }
}
