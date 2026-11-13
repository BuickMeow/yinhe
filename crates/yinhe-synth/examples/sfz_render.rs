//! 端到端回放验证：加载真实 SFZ，播放一段音阶 + CC 门控变化，输出 wav。
//!
//! 用途：确认 `xfin_*`（分层音量）、`trigger=release`（释放采样）、
//! `pitch_random`、`amplfo_*` 真的进了引擎——而不是被静默丢弃。
//!
//! ```sh
//! cargo run --release -p yinhe-synth --example sfz_render -- <file.sfz> [out.wav]
//! ```

use std::path::PathBuf;

use yinhe_mixer::ChannelBuffers;
use yinhe_synth::ControlEvent;
use yinhe_synth::{CpuSynth, SynthEvent};

const SR: u32 = 48_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(sfz) = args.next() else {
        eprintln!("用法: sfz_render <file.sfz> [out.wav]");
        std::process::exit(2);
    };
    let out = args.next().unwrap_or_else(|| "sfz_render.wav".into());
    let sfz = PathBuf::from(sfz);

    let mut synth = CpuSynth::new(SR);
    // 2 = Cubic（四点 Catmull-Rom；见 `yinhe_types::Interpolation::code`）
    synth.set_interpolation(2);
    if let Err(e) = synth.load_dense_soundfonts(0, std::slice::from_ref(&sfz)) {
        eprintln!("加载失败: {e}");
        std::process::exit(1);
    }

    // 事件：CC14/15/20 先设为库默认值，然后一段半音阶 + 释放
    let mut events: Vec<SynthEvent> = Vec::new();
    let mut t = 0u64;
    for (cc, v) in [(14u8, 127.0f32), (15u8, 85.0f32), (20u8, 100.0f32)] {
        events.push(SynthEvent::Control {
            sample: t,
            channel: 0,
            event: ControlEvent::Raw(cc, v),
        });
    }
    // 12 个音，每个 400ms，力度递增；后一个音在前一个结束后 50ms 触发（留释放空间）
    for i in 0..12u32 {
        let key = 55 + i as u8;
        let vel = 40 + (i as u8) * 7;
        events.push(SynthEvent::NoteOn {
            track: 0,
            sample: t,
            channel: 0,
            key,
            velocity: vel,
            end_sample: t + 400 * SR as u64 / 1000,
        });
        t += 450 * SR as u64 / 1000;
    }
    // 最后一音后留 3 秒尾音（释放采样在这里发声）
    let total = t + 3 * SR as u64;
    synth.load_events(events);

    let frames = 512usize;
    let mut left = Vec::with_capacity(total as usize);
    let mut right = Vec::with_capacity(total as usize);
    let mut buf = ChannelBuffers {
        left: vec![0.0; frames],
        right: vec![0.0; frames],
    };
    let mut peak = 0.0f32;
    let mut rms = 0.0f64;
    let mut rendered = 0u64;
    while rendered < total {
        let n = frames.min((total - rendered) as usize);
        let mut bufs = [ChannelBuffers {
            left: vec![0.0; n],
            right: vec![0.0; n],
        }];
        synth.render_to_mixer(&mut bufs);
        left.extend_from_slice(&bufs[0].left);
        right.extend_from_slice(&bufs[0].right);
        for &s in &bufs[0].left {
            peak = peak.max(s.abs());
            rms += (s as f64) * (s as f64);
        }
        rendered += n as u64;
        let _ = &mut buf;
    }

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SR,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&out, spec).expect("创建 wav");
    for i in 0..left.len() {
        writer.write_sample(left[i]).unwrap();
        writer.write_sample(right[i]).unwrap();
    }
    writer.finalize().unwrap();

    let rms = (rms / left.len() as f64).sqrt();
    println!("wav={out}");
    println!(
        "frames={} seconds={:.2}",
        left.len(),
        left.len() as f64 / SR as f64
    );
    println!("peak={peak:.6} rms={rms:.6} silent={}", peak < 1e-6);
}
