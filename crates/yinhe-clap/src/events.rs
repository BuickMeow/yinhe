//! yinhe 风格事件 → CLAP 事件转换。
//!
//! 约定（与 yinhe-audio 对齐）：
//! - `time` 是块内 sample offset；
//! - NoteOn/NoteOff 走 CLAP note 事件（velocity 归一化到 0.0~1.0）；
//! - CC / 弯音 / ProgramChange 走原始 MIDI 事件（CLAP 标准做法，
//!   由插件的 note ports MIDI dialect 接收）；
//! - 插件参数变化走 ParamValue 事件。

use clack_host::events::event_types::{MidiEvent, NoteChokeEvent, NoteOffEvent, NoteOnEvent, ParamValueEvent};
use clack_host::events::io::EventBuffer;
use clack_host::events::{Match, Pckn};
use clack_host::prelude::ClapId;
use clack_host::utils::Cookie;

/// 渲染线程输入给插件的单条事件。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClapInputEvent {
    NoteOn {
        time: u32,
        channel: u8,
        key: u8,
        /// 0.0 ~ 1.0（MIDI velocity / 127）。
        velocity: f64,
    },
    NoteOff {
        time: u32,
        channel: u8,
        key: u8,
        velocity: f64,
    },
    /// 掐掉指定键的所有发声（note_id 不指定）。
    NoteChoke { time: u32, channel: u8, key: u8 },
    /// 原始 MIDI 1.0 消息（CC、弯音、ProgramChange 等），最多 3 字节。
    Midi { time: u32, data: [u8; 3] },
    /// 插件参数变化。
    ParamValue { time: u32, param_id: u32, value: f64 },
}

/// 推入事件缓冲。非法 param_id（u32::MAX）直接丢弃并记日志，不 panic。
pub(crate) fn push_event(buffer: &mut EventBuffer, event: &ClapInputEvent) {
    match *event {
        ClapInputEvent::NoteOn {
            time,
            channel,
            key,
            velocity,
        } => {
            let pckn = Pckn::new(
                0u16,
                u16::from(channel),
                u16::from(key),
                Match::<u32>::All,
            );
            buffer.push(&NoteOnEvent::new(time, pckn, velocity));
        }
        ClapInputEvent::NoteOff {
            time,
            channel,
            key,
            velocity,
        } => {
            let pckn = Pckn::new(
                0u16,
                u16::from(channel),
                u16::from(key),
                Match::<u32>::All,
            );
            buffer.push(&NoteOffEvent::new(time, pckn, velocity));
        }
        ClapInputEvent::NoteChoke { time, channel, key } => {
            let pckn = Pckn::new(
                0u16,
                u16::from(channel),
                u16::from(key),
                Match::<u32>::All,
            );
            buffer.push(&NoteChokeEvent::new(time, pckn));
        }
        ClapInputEvent::Midi { time, data } => {
            buffer.push(&MidiEvent::new(time, 0, data));
        }
        ClapInputEvent::ParamValue {
            time,
            param_id,
            value,
        } => {
            let Some(id) = ClapId::from_raw(param_id) else {
                tracing::warn!(target: "clap-plugin", "忽略非法 param_id: {param_id}");
                return;
            };
            buffer.push(&ParamValueEvent::new(
                time,
                id,
                Pckn::match_all(),
                value,
                Cookie::empty(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_pushed_in_order() {
        let mut buf = EventBuffer::with_capacity(8);
        push_event(
            &mut buf,
            &ClapInputEvent::NoteOn {
                time: 0,
                channel: 0,
                key: 60,
                velocity: 1.0,
            },
        );
        push_event(
            &mut buf,
            &ClapInputEvent::Midi {
                time: 3,
                data: [0xB0, 7, 100],
            },
        );
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn invalid_param_id_dropped() {
        let mut buf = EventBuffer::with_capacity(8);
        push_event(
            &mut buf,
            &ClapInputEvent::ParamValue {
                time: 0,
                param_id: u32::MAX,
                value: 1.0,
            },
        );
        assert!(buf.is_empty());
    }
}
