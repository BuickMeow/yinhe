//! 格式无关事件 → CLAP 事件转换。
//!
//! 约定（与 yinhe-audio 对齐）：
//! - `time` 是块内 sample offset；
//! - NoteOn/NoteOff 走 CLAP note 事件（velocity 归一化到 0.0~1.0）；
//! - CC / 弯音 / ProgramChange 走原始 MIDI 事件（CLAP 标准做法，
//!   由插件的 note ports MIDI dialect 接收）；
//! - 插件参数变化走 ParamValue 事件。

use clack_host::events::event_types::{
    MidiEvent, NoteChokeEvent, NoteOffEvent, NoteOnEvent, ParamValueEvent,
};
use clack_host::events::io::EventBuffer;
use clack_host::events::{Match, Pckn};
use clack_host::prelude::ClapId;
use clack_host::utils::Cookie;
use yinhe_mixer::PluginEvent;

/// 推入事件缓冲。非法 param_id（u32::MAX）直接丢弃并记日志，不 panic。
pub(crate) fn push_event(buffer: &mut EventBuffer, event: &PluginEvent) {
    match *event {
        PluginEvent::NoteOn {
            time,
            channel,
            key,
            velocity,
        } => {
            let pckn = Pckn::new(0u16, u16::from(channel), u16::from(key), Match::<u32>::All);
            buffer.push(&NoteOnEvent::new(time, pckn, velocity));
        }
        PluginEvent::NoteOff {
            time,
            channel,
            key,
            velocity,
        } => {
            let pckn = Pckn::new(0u16, u16::from(channel), u16::from(key), Match::<u32>::All);
            buffer.push(&NoteOffEvent::new(time, pckn, velocity));
        }
        PluginEvent::NoteChoke { time, channel, key } => {
            let pckn = Pckn::new(0u16, u16::from(channel), u16::from(key), Match::<u32>::All);
            buffer.push(&NoteChokeEvent::new(time, pckn));
        }
        PluginEvent::Midi { time, data } => {
            buffer.push(&MidiEvent::new(time, 0, data));
        }
        PluginEvent::ParamValue {
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
            &PluginEvent::NoteOn {
                time: 0,
                channel: 0,
                key: 60,
                velocity: 1.0,
            },
        );
        push_event(
            &mut buf,
            &PluginEvent::Midi {
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
            &PluginEvent::ParamValue {
                time: 0,
                param_id: u32::MAX,
                value: 1.0,
            },
        );
        assert!(buf.is_empty());
    }
}
