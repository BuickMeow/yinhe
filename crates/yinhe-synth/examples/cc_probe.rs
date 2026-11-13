//! 诊断：加载 SF2，发一个音符 + 指定 CC71/74，输出 0.5s 单声道 f32。
use std::io::Write;
use std::path::PathBuf;
use yinhe_mixer::ChannelBuffers;
use yinhe_synth::ControlEvent;
use yinhe_synth::{CpuSynth, SynthEvent};

const SR: u32 = 48_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let sf2 = PathBuf::from(args.next().expect("sf2"));
    let out = args.next().expect("out");
    let cc74: f32 = args.next().unwrap_or_else(|| "64".into()).parse().unwrap();
    let cc71: f32 = args.next().unwrap_or_else(|| "64".into()).parse().unwrap();
    let vel: u8 = args.next().unwrap_or_else(|| "100".into()).parse().unwrap();
    let program: u8 = args.next().unwrap_or_else(|| "0".into()).parse().unwrap();
    let mut synth = CpuSynth::new(SR);
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&sf2))
        .unwrap();
    synth.load_events(vec![
        SynthEvent::Control {
            sample: 0,
            channel: 0,
            event: ControlEvent::ProgramChange(program),
        },
        SynthEvent::Control {
            sample: 0,
            channel: 0,
            event: ControlEvent::Raw(74, cc74),
        },
        SynthEvent::Control {
            sample: 0,
            channel: 0,
            event: ControlEvent::Raw(71, cc71),
        },
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: vel,
            end_sample: u64::MAX,
        },
    ]);
    let mut file = std::fs::File::create(&out).unwrap();
    let total = SR as usize / 2;
    let mut written = 0usize;
    while written < total {
        let frames = 4096.min(total - written);
        let mut bufs: Vec<ChannelBuffers> = (0..2)
            .map(|_| ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        synth.render_range(&mut bufs, 0, frames);
        let bytes: Vec<u8> = bufs[0].left.iter().flat_map(|v| v.to_le_bytes()).collect();
        file.write_all(&bytes).unwrap();
        written += frames;
    }
}
