//! 输出设备抽象层。
//!
//! - 桌面 / Android：`cpal`（CoreAudio / WASAPI / AAudio）；
//! - OpenHarmony：`OHAudio`（`OH_AudioRenderer`），因为 cpal 把 OHOS 当成 Linux
//!   去找 ALSA，而鸿蒙没有 ALSA。
//!
//! 上层（`spawn.rs`）只依赖这里的少量类型，不再直接引用 cpal。

#[cfg(not(target_env = "ohos"))]
mod desktop;
#[cfg(target_env = "ohos")]
#[path = "audio_out/ohos.rs"]
mod ohos;

#[cfg(not(target_env = "ohos"))]
pub(crate) use desktop::{Device, Stream, default_output_device, find_output_device, list_devices};
#[cfg(target_env = "ohos")]
pub(crate) use ohos::{Device, Stream, default_output_device, find_output_device, list_devices};

/// 请求的缓冲区大小（与 cpal 解耦：OHOS 不编译 cpal）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferSize {
    Default,
    Fixed(u32),
}

/// 实际建流用的输出格式。
#[derive(Clone, Copy, Debug)]
pub struct OutputFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// 流错误类别（只保留上层真正区分处理的两类）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamErrorKind {
    /// 设备采样率被其他 app 改变（macOS SRC 场景）：流还活着，恢复即可。
    StreamInvalidated,
    /// 其余后端错误：置位 stream_error，提示用户重启/换设备。
    #[allow(dead_code)] // OHOS 后端暂未上报流错误
    BackendError,
}

/// 后端上报的流错误。
#[derive(Debug)]
pub struct StreamError {
    pub kind: StreamErrorKind,
    pub message: Option<String>,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.message {
            Some(msg) => write!(f, "{msg}"),
            None => write!(f, "{:?}", self.kind),
        }
    }
}
