//! MIDI 设备输入（midir 封装）。
//!
//! - 枚举系统 MIDI 输入端口（macOS: CoreMIDI / Windows: winmm / Linux: ALSA / Android: NDK）。
//! - 打开端口后由 midir 回调线程把原始字节推入内部缓冲，
//!   UI 线程每帧 drain() 消费，不阻塞回调线程。
//! - 事件解析见 events（纯函数，可单测）。

mod error;
pub mod events;

pub use error::MidiIoError;
pub use events::{MidiEvent, parse_event};

use midir::{MidiInput, MidiInputConnection};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// 枚举系统全部 MIDI 输入端口名（按系统顺序）。
pub fn list_input_ports() -> Result<Vec<String>, MidiIoError> {
    let input = MidiInput::new("yinhe")?;
    Ok(input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect())
}

/// 已打开的 MIDI 输入流。
///
/// midir 回调线程把原始消息推入内部缓冲；UI 线程每帧调用 drain 取走。
/// Drop 时自动断开端口。
pub struct MidiInputStream {
    _conn: MidiInputConnection<()>,
    buffer: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

impl MidiInputStream {
    /// 打开指定名字的输入端口；找不到返回 PortNotFound。
    ///
    /// 每次打开都会新建 midir 客户端（midir 0.10+ 的 connect 拿走所有权）。
    pub fn open(name: &str) -> Result<Self, MidiIoError> {
        let input = MidiInput::new("yinhe")?;
        let ports = input.ports();
        let port = ports
            .iter()
            .find(|p| input.port_name(p).as_deref() == Ok(name))
            .ok_or_else(|| MidiIoError::PortNotFound(name.to_string()))?;
        let port_name = input.port_name(port)?;
        let buffer: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
        let cb_buffer = Arc::clone(&buffer);
        let callback = move |_ts: u64, data: &[u8], _: &mut ()| {
            if let Ok(mut buf) = cb_buffer.lock() {
                buf.push_back(data.to_vec());
            }
        };
        let conn = input.connect(port, &port_name, callback, ())?;
        Ok(Self {
            _conn: conn,
            buffer,
        })
    }

    /// 取出缓冲中全部原始消息（清空缓冲）。
    pub fn drain(&self) -> Vec<Vec<u8>> {
        match self.buffer.lock() {
            Ok(mut buf) => buf.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }
}
