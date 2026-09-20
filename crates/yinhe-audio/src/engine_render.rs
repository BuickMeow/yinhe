use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};

use crate::audio_model::AudioEvent;
use crate::engine::AudioEngine;

mod dispatch;
mod plugins;
mod render;

/// Number of output channels (stereo).
const STEREO_CHANNELS: usize = 2;

impl AudioEngine {
    /// GPU 合成器是否启用（无 `gpu` feature 时恒 false）。
    /// dispatch 用它决定是否把事件喂给 xsynth（GPU 自管事件列表）。
    #[inline]
    pub(crate) fn gpu_synth_active(&self) -> bool {
        #[cfg(feature = "gpu")]
        {
            self.gpu_synth.is_some()
        }
        #[cfg(not(feature = "gpu"))]
        {
            false
        }
    }
}

/// 提取原始 CC（号 + 值）；非 CC 事件返回 None。
pub(crate) fn raw_cc(event: &AudioEvent) -> Option<(u8, u8)> {
    match event {
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(cc, value))) => {
            Some((*cc, *value))
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

/// XSynth 通道事件序列（栈上固定容量）：RPN 0/1/2 → 高层事件（xsynth 原生），
/// 其余 RPN/NRPN → 标准 CC 序列（xsynth 的选择器状态机会解析标准参数，
/// 非标准号被忽略）。
pub(crate) struct XsynthMessages {
    msgs: [ChannelAudioEvent; 4],
    len: usize,
}

impl Default for XsynthMessages {
    fn default() -> Self {
        Self {
            // 占位元素不会出现在 `as_slice()`（len 初始 0）。
            msgs: [ChannelAudioEvent::AllNotesOff; 4],
            len: 0,
        }
    }
}

impl XsynthMessages {
    fn push(&mut self, msg: ChannelAudioEvent) {
        if self.len < self.msgs.len() {
            self.msgs[self.len] = msg;
            self.len += 1;
        }
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[ChannelAudioEvent] {
        &self.msgs[..self.len]
    }
}

/// 把一条 `AudioEvent` 适配为 XSynth 的通道事件序列（最多 4 条）。
/// XSynth 没有 RPN 事件类型：标准 0/1/2 用高层 f32 事件（无量化，语义与
/// 旧 `emit_midi_binding` 一致），其余 RPN/NRPN 走 CC 序列（出口量化）。
pub(crate) fn xsynth_events(event: &AudioEvent) -> XsynthMessages {
    let mut out = XsynthMessages::default();
    let control = |ev: ControlEvent| ChannelAudioEvent::Control(ev);
    match event {
        AudioEvent::Channel(ev) => out.push(*ev),
        AudioEvent::Rpn { parameter, value } => match parameter {
            // 归一化 0..1 → 高层语义值（半音/音分/半音），全 f32 无量化。
            0 => out.push(control(ControlEvent::PitchBendSensitivity(value * 127.0))),
            1 => out.push(control(ControlEvent::FineTune((value - 0.5) * 200.0))),
            2 => out.push(control(ControlEvent::CoarseTune(value * 127.0 - 64.0))),
            _ => {
                let raw = quantize_rpn(*parameter, *value);
                let (msgs, len) = rpn_cc_sequence(101, 100, *parameter, raw);
                for msg in &msgs[..len] {
                    out.push(control(*msg));
                }
            }
        },
        AudioEvent::Nrpn { parameter, value } => {
            let raw = quantize_rpn(*parameter, *value);
            let (msgs, len) = rpn_cc_sequence(99, 98, *parameter, raw);
            for msg in &msgs[..len] {
                out.push(control(*msg));
            }
        }
    }
    out
}

/// RPN/NRPN 选择器 + Data Entry 的 CC 序列（值 LSB 为 0 时省略 CC38，与旧行为一致）。
fn rpn_cc_sequence(
    msb_cc: u8,
    lsb_cc: u8,
    parameter: u16,
    value: u16,
) -> ([ControlEvent; 4], usize) {
    let msgs = [
        ControlEvent::Raw(msb_cc, ((parameter >> 7) & 0x7F) as u8),
        ControlEvent::Raw(lsb_cc, (parameter & 0x7F) as u8),
        ControlEvent::Raw(6, ((value >> 7) & 0x7F) as u8),
        ControlEvent::Raw(38, (value & 0x7F) as u8),
    ];
    let len = if value & 0x7F == 0 { 3 } else { 4 };
    (msgs, len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xsynth_core::channel::ControlEvent;

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

    /// XSynth 出口：标准 RPN 0/1/2 → 高层 f32 事件（无量化）。
    #[test]
    fn xsynth_standard_rpn_maps_to_high_level_events() {
        let pbs = xsynth_events(&AudioEvent::Rpn {
            parameter: 0,
            value: 5.5 / 127.0,
        });
        assert!(matches!(
            pbs.as_slice(),
            [ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(v))]
                if (v - 5.5).abs() < 1e-5
        ));

        let fine = xsynth_events(&AudioEvent::Rpn {
            parameter: 1,
            value: 0.75,
        });
        assert!(matches!(
            fine.as_slice(),
            [ChannelAudioEvent::Control(ControlEvent::FineTune(v))]
                if (v - 50.0).abs() < 1e-3
        ));

        let coarse = xsynth_events(&AudioEvent::Rpn {
            parameter: 2,
            value: 70.0 / 127.0,
        });
        assert!(matches!(
            coarse.as_slice(),
            [ChannelAudioEvent::Control(ControlEvent::CoarseTune(v))]
                if (v - 6.0).abs() < 1e-5
        ));
    }

    /// XSynth 出口：非标准 RPN/NRPN 拆 CC 序列（出口量化）。
    #[test]
    fn xsynth_nonstandard_rpn_and_nrpn_split_cc_sequence() {
        // value = 100/16383 → raw 100：data msb 0、lsb 100。
        let rpn = xsynth_events(&AudioEvent::Rpn {
            parameter: 5,
            value: 100.0 / 16383.0,
        });
        assert!(matches!(
            rpn.as_slice(),
            [
                ChannelAudioEvent::Control(ControlEvent::Raw(101, 0)),
                ChannelAudioEvent::Control(ControlEvent::Raw(100, 5)),
                ChannelAudioEvent::Control(ControlEvent::Raw(6, 0)),
                ChannelAudioEvent::Control(ControlEvent::Raw(38, 100)),
            ]
        ));

        let nrpn = xsynth_events(&AudioEvent::Nrpn {
            parameter: 10,
            value: 100.0 / 16383.0,
        });
        assert!(matches!(
            nrpn.as_slice(),
            [
                ChannelAudioEvent::Control(ControlEvent::Raw(99, 0)),
                ChannelAudioEvent::Control(ControlEvent::Raw(98, 10)),
                ChannelAudioEvent::Control(ControlEvent::Raw(6, 0)),
                ChannelAudioEvent::Control(ControlEvent::Raw(38, 100)),
            ]
        ));
    }
}

/// GPU 合成器接入混音台的冒烟测试（需 `YINHE_TEST_SFZ` 指向 SFZ 文件）。
#[cfg(all(test, feature = "gpu"))]
mod gpu_tests {
    use std::sync::Arc;

    use yinhe_core::{ConductorData, NoteEvent, ProjectMeta, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    use crate::channel_layout::ChannelLayout;
    use crate::engine::AudioEngine;
    use crate::spawn::AudioCommand;

    /// 1 拍、单个音符的模型（120 BPM / PPQ 480）。
    fn tiny_model() -> Arc<YinModel> {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.load_track_notes(vec![vec![NoteEvent {
            start_tick: 0,
            end_tick: 480,
            key: 60,
            velocity: 100,
            id: 0,
        }]]);
        model.rebuild();
        Arc::new(model)
    }

    /// GPU 路径经混音台渲染：不卡死、输出有限且有声音。
    #[test]
    fn gpu_engine_render_smoke() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let model = tiny_model();
        let layout = ChannelLayout::from_model(&model);
        let mut engine = AudioEngine::new(48_000, layout);
        engine.handle_command(AudioCommand::LoadModel { model });

        let mut synth = yinhe_synth::GpuSynth::new_default(48_000).expect("GpuSynth init");
        synth
            .load_dense_soundfonts(0, &[std::path::PathBuf::from(&sfz)])
            .expect("soundfont load");
        synth.finish_soundfont_load();
        // 与真实路径一致：加载事件列表（这里手动构造一个 0..100ms 的音符）。
        synth.load_events(vec![yinhe_synth::SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 4800,
        }]);
        engine.gpu_synth = Some(synth);

        engine.handle_command(AudioCommand::Play { from_sample: 0 });
        let mut out = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for i in 0..200 {
            engine.render(&mut out);
            assert!(out.iter().all(|v| v.is_finite()), "block {i} 输出异常");
            peak = out.iter().fold(peak, |m, v| m.max(v.abs()));
        }
        assert!(peak > 0.0, "GPU 渲染无输出（peak=0）");
    }
}
