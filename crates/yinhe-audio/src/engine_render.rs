use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};

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
pub(crate) fn raw_cc(event: &ChannelAudioEvent) -> Option<(u8, u8)> {
    match event {
        ChannelAudioEvent::Control(ControlEvent::Raw(cc, value)) => Some((*cc, *value)),
        _ => None,
    }
}

/// 把 xsynth 风格的通道事件转成原始 MIDI 报文（CC/弯音/ProgramChange），
/// status 字节带上音轨的 MIDI 通道（0..15）。仅用于乐器轨的自动化路由。
fn cc_to_midi(event: &ChannelAudioEvent, channel: u8) -> Option<[u8; 3]> {
    let ch = 0x0F & channel;
    match event {
        ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)) => Some([0xB0 | ch, *cc, *val]),
        ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => {
            // v ∈ [-1, 1] → 14 bit 弯音值
            let raw = ((v.clamp(-1.0, 1.0) + 1.0) * 8191.5) as u32;
            Some([0xE0 | ch, (raw & 0x7F) as u8, ((raw >> 7) & 0x7F) as u8])
        }
        ChannelAudioEvent::ProgramChange(p) => Some([0xC0 | ch, *p, 0]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xsynth_core::channel::ControlEvent;

    #[test]
    fn cc_to_midi_status_uses_channel_nibble() {
        let msg = cc_to_midi(&ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)), 0x0A).unwrap();
        // 0xB0 | 通道低 4 位（0x0A）= 0xBA
        assert_eq!(msg, [0xBA, 7, 100]);
    }

    #[test]
    fn cc_to_midi_pitchbend_14bit() {
        let msg = cc_to_midi(
            &ChannelAudioEvent::Control(ControlEvent::PitchBendValue(0.0)),
            0,
        )
        .unwrap();
        assert_eq!(msg[0] & 0xF0, 0xE0);
        let raw = ((msg[2] as u32) << 7) | (msg[1] as u32);
        assert!(
            (8190..=8192).contains(&raw),
            "中部弯音值应在中点附近, got {raw}"
        );
    }

    #[test]
    fn cc_to_midi_program_change() {
        let msg = cc_to_midi(&ChannelAudioEvent::ProgramChange(42), 3).unwrap();
        assert_eq!(msg, [0xC3, 42, 0]);
    }

    #[test]
    fn cc_to_midi_unhandled_returns_none() {
        // NoteOn 类事件不是控制器/ProgramChange，不应转 MIDI 报文。
        let r = cc_to_midi(&ChannelAudioEvent::NoteOn { key: 60, vel: 100 }, 0);
        assert!(r.is_none());
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
