//! yinhe 风格事件 → VST3 `Event` 转换。
//!
//! 约定：
//! - NoteOn/NoteOff 走标准 note 事件（velocity 0.0~1.0）；
//! - CC / 弯音 / 触后 / ProgramChange 走 `kLegacyMIDICCOutEvent`
//!   （VST3 宿主输入 MIDI 控制器的通用方式）；
//! - 参数变化不走事件（由 `IParameterChanges` 队列传递），返回 None。

use vst3::Steinberg::Vst::{
    ControllerNumbers_::{kAfterTouch, kCtrlProgramChange, kPitchBend},
    Event,
    Event_::EventTypes_::{kLegacyMIDICCOutEvent, kNoteOffEvent, kNoteOnEvent},
    LegacyMIDICCOutEvent, NoteOffEvent, NoteOnEvent,
};
use yinhe_mixer::PluginEvent;

/// 转换一条事件；参数变化返回 None（走参数队列）。
pub(crate) fn plugin_event_to_vst(e: &PluginEvent) -> Option<Event> {
    let mut event: Event = unsafe { std::mem::zeroed() };
    event.busIndex = 0;
    match *e {
        PluginEvent::NoteOn {
            time,
            channel,
            key,
            velocity,
        } => {
            event.sampleOffset = time as i32;
            event.r#type = kNoteOnEvent as u16;
            event.__field0.noteOn = NoteOnEvent {
                channel: channel as i16,
                pitch: key as i16,
                tuning: 0.0,
                velocity: velocity as f32,
                length: 0,
                noteId: -1,
            };
            Some(event)
        }
        PluginEvent::NoteOff {
            time,
            channel,
            key,
            velocity,
        } => {
            event.sampleOffset = time as i32;
            event.r#type = kNoteOffEvent as u16;
            event.__field0.noteOff = NoteOffEvent {
                channel: channel as i16,
                pitch: key as i16,
                velocity: velocity as f32,
                noteId: -1,
                tuning: 0.0,
            };
            Some(event)
        }
        PluginEvent::Midi { time, data } => {
            event.sampleOffset = time as i32;
            let status = data[0] & 0xF0;
            let channel = (data[0] & 0x0F) as i8;
            event.r#type = kLegacyMIDICCOutEvent as u16;
            event.__field0.midiCCOut = match status {
                0xB0 => LegacyMIDICCOutEvent {
                    controlNumber: data[1],
                    channel,
                    value: data[2] as i8,
                    value2: 0,
                },
                // 弯音：LSB 在 value、MSB 在 value2。
                0xE0 => LegacyMIDICCOutEvent {
                    controlNumber: kPitchBend as u8,
                    channel,
                    value: (data[1] & 0x7F) as i8,
                    value2: (data[2] & 0x7F) as i8,
                },
                // 通道触后。
                0xD0 => LegacyMIDICCOutEvent {
                    controlNumber: kAfterTouch as u8,
                    channel,
                    value: (data[1] & 0x7F) as i8,
                    value2: 0,
                },
                // 音色切换。
                0xC0 => LegacyMIDICCOutEvent {
                    controlNumber: kCtrlProgramChange as u8,
                    channel,
                    value: (data[1] & 0x7F) as i8,
                    value2: 0,
                },
                _ => return None,
            };
            Some(event)
        }
        PluginEvent::NoteChoke { .. } | PluginEvent::ParamValue { .. } => None,
    }
}
