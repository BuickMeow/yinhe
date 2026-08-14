//! MIDI I/O 错误类型。

use thiserror::Error;

/// MIDI 设备 I/O 错误。
#[derive(Debug, Error)]
pub enum MidiIoError {
    /// midir 初始化失败（系统 MIDI 服务不可用）。
    #[error("MIDI 系统初始化失败: {0}")]
    Init(String),
    /// 指定名字的输入端口不存在（未插拔或已拔出）。
    #[error("找不到 MIDI 输入端口: {0}")]
    PortNotFound(String),
    /// 读取端口信息失败。
    #[error("读取 MIDI 端口信息失败: {0}")]
    PortInfo(String),
    /// 打开端口失败。
    #[error("打开 MIDI 输入端口失败: {0}")]
    Connect(String),
}

impl From<midir::InitError> for MidiIoError {
    fn from(e: midir::InitError) -> Self {
        Self::Init(e.to_string())
    }
}

impl From<midir::PortInfoError> for MidiIoError {
    fn from(e: midir::PortInfoError) -> Self {
        Self::PortInfo(e.to_string())
    }
}

impl From<midir::ConnectError<midir::MidiInput>> for MidiIoError {
    fn from(e: midir::ConnectError<midir::MidiInput>) -> Self {
        Self::Connect(e.to_string())
    }
}
