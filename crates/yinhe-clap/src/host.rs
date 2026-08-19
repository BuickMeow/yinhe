//! 宿主回调实现（yinhe 侧）。
//!
//! 插件反向调用宿主的入口全部收敛在这里：
//! - Shared：线程安全回调，插件可能在任意线程触发，一律只置原子标志，
//!   实际处理由主线程轮询（参照 yinhe-audio 的 stream_error 模式）。
//! - MainThread：主线程回调，只记脏标记，不做重活。

use std::sync::atomic::{AtomicBool, Ordering};

use clack_extensions::latency::HostLatencyImpl;
use clack_extensions::log::{HostLogImpl, LogSeverity};
use clack_extensions::params::{HostParamsImplMainThread, ParamClearFlags, ParamRescanFlags};
use clack_extensions::state::HostStateImpl;
use clack_host::host::{
    AudioProcessorHandler, HostExtensions, HostHandlers, MainThreadHandler, SharedHandler,
};
use clack_host::plugin::{InitializedPluginHandle, InitializingPluginHandle};
use clack_host::prelude::ClapId;

pub struct YinheShared {
    /// 插件请求重启处理（activate/deactivate 循环）。
    pub(crate) restart_requested: AtomicBool,
    /// 插件请求 process。
    pub(crate) process_requested: AtomicBool,
    /// 插件请求主线程回调。
    pub(crate) callback_requested: AtomicBool,
    /// 插件请求参数 flush。
    pub(crate) flush_requested: AtomicBool,
}

impl YinheShared {
    pub(crate) fn new() -> Self {
        Self {
            restart_requested: AtomicBool::new(false),
            process_requested: AtomicBool::new(false),
            callback_requested: AtomicBool::new(false),
            flush_requested: AtomicBool::new(false),
        }
    }

    /// 取出并清除标志（单次原子操作）。
    fn take(flag: &AtomicBool) -> bool {
        flag.fetch_and(false, Ordering::SeqCst)
    }

    pub(crate) fn take_restart(&self) -> bool {
        Self::take(&self.restart_requested)
    }

    pub(crate) fn take_process(&self) -> bool {
        Self::take(&self.process_requested)
    }

    pub(crate) fn take_callback(&self) -> bool {
        Self::take(&self.callback_requested)
    }

    pub(crate) fn take_flush(&self) -> bool {
        Self::take(&self.flush_requested)
    }
}

impl<'a> SharedHandler<'a> for YinheShared {
    fn initializing(&self, _instance: InitializingPluginHandle<'a>) {}

    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::SeqCst);
    }

    fn request_process(&self) {
        self.process_requested.store(true, Ordering::SeqCst);
    }

    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst);
    }
}

impl HostLogImpl for YinheShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        // 实时线程也可能打日志：tracing 的写入不是硬实时安全的，
        // 但崩溃诊断价值大于偶发抖动；若后续发现抖动问题再改 lock-free 队列。
        match severity {
            LogSeverity::Error | LogSeverity::HostMisbehaving | LogSeverity::PluginMisbehaving => {
                tracing::error!(target: "clap-plugin", "{message}");
            }
            LogSeverity::Warning => {
                tracing::warn!(target: "clap-plugin", "{message}");
            }
            _ => {
                tracing::info!(target: "clap-plugin", "{message}");
            }
        }
    }
}

impl clack_extensions::params::HostParamsImplShared for YinheShared {
    fn request_flush(&self) {
        self.flush_requested.store(true, Ordering::SeqCst);
    }
}

pub struct YinheMainThread<'a> {
    #[allow(dead_code)]
    shared: &'a YinheShared,
    /// 插件状态脏标记（需要重新保存）。
    pub(crate) state_dirty: bool,
    /// 插件请求重扫参数列表。
    pub(crate) params_rescan_requested: bool,
    /// 插件延迟变化（PDC 预留，第一期不消费）。
    pub(crate) latency_changed: bool,
}

impl<'a> YinheMainThread<'a> {
    pub(crate) fn new(shared: &'a YinheShared) -> Self {
        Self {
            shared,
            state_dirty: false,
            params_rescan_requested: false,
            latency_changed: false,
        }
    }
}

impl<'a> MainThreadHandler<'a> for YinheMainThread<'a> {
    fn initialized(&mut self, _instance: InitializedPluginHandle<'a>) {}
}

impl<'a> HostStateImpl for YinheMainThread<'a> {
    fn mark_dirty(&mut self) {
        self.state_dirty = true;
    }
}

impl<'a> HostLatencyImpl for YinheMainThread<'a> {
    fn changed(&mut self) {
        self.latency_changed = true;
    }
}

impl<'a> HostParamsImplMainThread for YinheMainThread<'a> {
    fn rescan(&mut self, _flags: ParamRescanFlags) {
        self.params_rescan_requested = true;
    }

    fn clear(&mut self, _param_id: ClapId, _flags: ParamClearFlags) {
        // 插件参数 ↔ AM 自动化 lane 打通后在这里解除引用，第一期无自动化引用可清。
    }
}

pub struct YinheHost;

impl HostHandlers for YinheHost {
    type Shared<'a> = YinheShared;
    type MainThread<'a> = YinheMainThread<'a>;
    type AudioProcessor<'a> = YinheAudioProcessor;

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<clack_extensions::log::HostLog>()
            .register::<clack_extensions::latency::HostLatency>()
            .register::<clack_extensions::params::HostParams>()
            .register::<clack_extensions::state::HostState>();
    }
}

/// 音频线程侧宿主数据，第一期为空。
pub struct YinheAudioProcessor;

impl<'a> AudioProcessorHandler<'a> for YinheAudioProcessor {}
