use crate::audio_model::{ChannelAudioEvent, ControlEvent};

use crate::audio_model::AudioEvent;
use crate::engine::AudioEngine;

mod audio_clip;
mod dispatch;
mod plugins;
mod render;

/// Number of output channels (stereo).
const STEREO_CHANNELS: usize = 2;

impl AudioEngine {
    /// 本块有输入信号的 dense 通道（混音台空闲通道跳过用）。
    ///
    /// yinhe CPU 用合成器的活跃 voice 位图；xsynth 没有可用的通道级信号，
    /// 保守全开（不做跳过）。插件乐器/音频通道一律置活跃（可能带尾音）。
    pub(crate) fn channel_activity(&mut self, active: &mut Vec<bool>) {
        let n = self.mix.mixer.channel_count();
        active.clear();
        active.resize(n, true);
        {
            let cs = &mut self.cpu_synth;
            cs.refresh_channel_mask();
            let mask = *cs.channel_mask();
            cs.clear_channel_mask();
            for (c, a) in active.iter_mut().enumerate() {
                *a = c < yinhe_synth::MAX_CHANNELS && mask[c / 64] & (1u64 << (c % 64)) != 0;
            }
        }
        for (dense, slot) in self.plugins.instruments.iter().enumerate() {
            if slot.is_some() {
                // 插件链与其输出同 dense（OwnLc = home LC / AP = 共享源）。
                let out = dense;
                if out < n {
                    active[out] = true;
                }
            }
        }
        for &ach in self.channel_layout.audio_channels() {
            let d = self.channel_layout.audio_dense_for(ach) as usize;
            if d < n {
                active[d] = true;
            }
        }
        if let Some(dense) = self.channel_layout.direct_master_dense()
            && (dense as usize) < n
        {
            active[dense as usize] = true;
        }
        // 冻结的 LC 即使没有活跃 voice 也必须参与混音（推子/insert/send），
        // 否则冻结音频会被空通道跳过。
        for node in self.mix.mixer_params.nodes() {
            if node.freeze.is_none() {
                continue;
            }
            let dense = match node.id {
                yinhe_types::ChannelId::Lc(id) => self.lc_dense(id),
                _ => u32::MAX,
            };
            if dense != u32::MAX && (dense as usize) < n {
                active[dense as usize] = true;
            }
        }
    }
}

/// 提取原始 CC（号 + 值 f32，0..127）；非 CC 事件返回 None。
pub(crate) fn raw_cc(event: &AudioEvent) -> Option<(u8, f32)> {
    match event {
        AudioEvent::Cc { controller, value } => Some((*controller, *value)),
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(cc, value))) => {
            Some((*cc, f32::from(*value)))
        }
        _ => None,
    }
}

/// 栈上固定容量的 MIDI 报文缓冲（一条事件最多展开 4 条：RPN/NRPN 拆 CC 序列）。
/// dispatch 热路径零分配。
#[derive(Default)]
pub(crate) struct MidiMessages {
    msgs: [[u8; 3]; 4],
    len: usize,
}

impl MidiMessages {
    fn push(&mut self, msg: [u8; 3]) {
        if self.len < self.msgs.len() {
            self.msgs[self.len] = msg;
            self.len += 1;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn as_slice(&self) -> &[[u8; 3]] {
        &self.msgs[..self.len]
    }
}

/// 把 xsynth 风格通道事件 / 原生 RPN/NRPN 转成原始 MIDI 报文
/// （CC/弯音/ProgramChange；RPN 拆 101/100/6/38，NRPN 拆 99/98/6/38），
/// status 字节带上音轨的 MIDI 通道（0..15）。仅用于乐器轨的自动化路由。
pub(crate) fn event_to_midi(event: &AudioEvent, channel: u8, out: &mut MidiMessages) {
    let ch = 0x0F & channel;
    match event {
        AudioEvent::Channel(ev) => match ev {
            ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)) => {
                out.push([0xB0 | ch, *cc, *val]);
            }
            ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => {
                // v ∈ [-1, 1] → 14 bit 弯音值
                let raw = ((v.clamp(-1.0, 1.0) + 1.0) * 8191.5) as u32;
                out.push([0xE0 | ch, (raw & 0x7F) as u8, ((raw >> 7) & 0x7F) as u8]);
            }
            ChannelAudioEvent::ProgramChange(p) => out.push([0xC0 | ch, *p, 0]),
            // 高层事件不再出现在事件流中；历史数据出现也不转（保持原行为）。
            _ => {}
        },
        AudioEvent::Cc { controller, value } => {
            // 引擎内部浮点 CC → MIDI 字节（插件只认 7-bit）。
            out.push([
                0xB0 | ch,
                *controller,
                value.round().clamp(0.0, 127.0) as u8,
            ]);
        }
        AudioEvent::PitchBend(v) => {
            // v ∈ [-1, 1] → 14 bit 弯音值
            let raw = ((v.clamp(-1.0, 1.0) + 1.0) * 8191.5) as u32;
            out.push([0xE0 | ch, (raw & 0x7F) as u8, ((raw >> 7) & 0x7F) as u8]);
        }
        AudioEvent::ProgramChange(p) => out.push([0xC0 | ch, *p, 0]),
        AudioEvent::Rpn { parameter, value } => {
            let raw = quantize_rpn(*parameter, *value);
            out.push([0xB0 | ch, 101, ((parameter >> 7) & 0x7F) as u8]);
            out.push([0xB0 | ch, 100, (parameter & 0x7F) as u8]);
            push_rpn_data_entry(out, ch, *parameter, raw);
        }
        AudioEvent::Nrpn { parameter, value } => {
            let raw = quantize_rpn(*parameter, *value);
            out.push([0xB0 | ch, 99, ((parameter >> 7) & 0x7F) as u8]);
            out.push([0xB0 | ch, 98, (parameter & 0x7F) as u8]);
            push_data_entry(out, ch, raw);
        }
    }
}

/// 归一化 f32 → 参数值域的原始整数（出口量化，四舍五入 + 钳制）。
/// RPN 0/2 是 7-bit，其余（含 NRPN）14-bit。
pub(crate) fn quantize_rpn(parameter: u16, value: f32) -> u16 {
    let max = crate::audio_model::rpn_raw_max(parameter);
    (value.clamp(0.0, 1.0) * max).round().clamp(0.0, max) as u16
}

/// RPN 的 Data Entry：7-bit 参数（RPN 0/2）的值直接进 CC6（msb），
/// 14-bit 参数拆 CC6/CC38。
fn push_rpn_data_entry(out: &mut MidiMessages, ch: u8, parameter: u16, raw: u16) {
    if crate::audio_model::rpn_raw_max(parameter) <= 127.0 {
        out.push([0xB0 | ch, 6, raw as u8]);
    } else {
        push_data_entry(out, ch, raw);
    }
}

/// Data Entry（CC6 + 非零 CC38）。
fn push_data_entry(out: &mut MidiMessages, ch: u8, value: u16) {
    out.push([0xB0 | ch, 6, ((value >> 7) & 0x7F) as u8]);
    if value & 0x7F != 0 {
        out.push([0xB0 | ch, 38, (value & 0x7F) as u8]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midi_of(event: &AudioEvent, channel: u8) -> Vec<[u8; 3]> {
        let mut out = MidiMessages::default();
        event_to_midi(event, channel, &mut out);
        out.as_slice().to_vec()
    }

    #[test]
    fn midi_status_uses_channel_nibble() {
        let event = AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)));
        let msgs = midi_of(&event, 0x0A);
        // 0xB0 | 通道低 4 位（0x0A）= 0xBA
        assert_eq!(msgs, vec![[0xBA, 7, 100]]);
    }

    #[test]
    fn midi_pitchbend_14bit() {
        let event = AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
            0.0,
        )));
        let msgs = midi_of(&event, 0);
        assert_eq!(msgs.len(), 1);
        let msg = msgs[0];
        assert_eq!(msg[0] & 0xF0, 0xE0);
        let raw = ((msg[2] as u32) << 7) | (msg[1] as u32);
        assert!(
            (8190..=8192).contains(&raw),
            "中部弯音值应在中点附近, got {raw}"
        );
    }

    #[test]
    fn midi_program_change() {
        let event = AudioEvent::Channel(ChannelAudioEvent::ProgramChange(42));
        assert_eq!(midi_of(&event, 3), vec![[0xC3, 42, 0]]);
    }

    #[test]
    fn midi_unhandled_returns_empty() {
        // NoteOn 类事件不是控制器/ProgramChange，不应转 MIDI 报文。
        let event = AudioEvent::Channel(ChannelAudioEvent::NoteOn { key: 60, vel: 100 });
        assert!(midi_of(&event, 0).is_empty());
    }

    /// 插件出口：原生 RPN 拆标准 CC 序列（101/100/6 + 非零 38），值在出口量化。
    #[test]
    fn midi_rpn_splits_standard_cc_sequence() {
        // RPN1（14-bit）：值 0.75 → raw = round(0.75 * 16383) = 12287 → msb 95, lsb 127
        let event = AudioEvent::Rpn {
            parameter: 1,
            value: 0.75,
        };
        let msgs = midi_of(&event, 2);
        assert_eq!(
            msgs,
            vec![
                [0xB2, 101, 0],
                [0xB2, 100, 1],
                [0xB2, 6, 95],
                [0xB2, 38, 127],
            ]
        );
    }

    /// 7-bit 参数的出口量化：RPN0 值 0.5 → raw = round(0.5 * 127) = 64，
    /// 直接进 CC6（msb），不发 CC38。
    #[test]
    fn midi_rpn_7bit_param_quantizes_to_msb_range() {
        let event = AudioEvent::Rpn {
            parameter: 0,
            value: 0.5,
        };
        let msgs = midi_of(&event, 0);
        assert_eq!(msgs, vec![[0xB0, 101, 0], [0xB0, 100, 0], [0xB0, 6, 64]]);
    }

    /// NRPN 拆 99/98/6/38 序列（值 LSB 为 0 时省略 CC38）。
    #[test]
    fn midi_nrpn_splits_standard_cc_sequence() {
        let event = AudioEvent::Nrpn {
            parameter: 0x1234,
            value: 0.5,
        };
        let msgs = midi_of(&event, 1);
        // raw = round(0.5 * 16383) = 8192 → msb 64, lsb 0（省略 CC38）。
        assert_eq!(
            msgs,
            vec![[0xB1, 99, 0x24], [0xB1, 98, 0x34], [0xB1, 6, 64]]
        );
    }
}
