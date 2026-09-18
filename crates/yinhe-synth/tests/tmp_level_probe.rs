//! 临时探针：测量密集和弦下 CpuSynth 输出的峰值（limiter 之前）。
//!
//! 用途：验证"limiter 后仍感觉爆音"是否来自 tanh 饱和（输入远超 1）。
//! 运行：YINHE_TEST_SFZ=... cargo test -p yinhe-synth --test tmp_level_probe -- --nocapture

use std::path::PathBuf;

use yinhe_mixer::ChannelBuffers;
use yinhe_synth::{CpuSynth, SynthEvent};

#[test]
fn probe_dense_chord_peak() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("跳过：未设置 YINHE_TEST_SFZ");
        return;
    };
    let mut synth = CpuSynth::new(48_000);
    synth
        .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
        .expect("load soundfont");

    // 场景 1：88 键齐奏（钢琴全键盘）
    // 场景 2：叠 4 层（每键 4 个力度）= 352 voice
    for (name, layers) in [("88 voice", 1u8), ("352 voice", 4u8)] {
        let mut events = Vec::new();
        for k in 21..109u8 {
            for layer in 0..layers {
                events.push(SynthEvent::NoteOn {
                    sample: 0,
                    channel: 0,
                    key: k,
                    velocity: 100 - layer * 5,
                    end_sample: 480_000,
                });
            }
        }
        synth.load_events(events);
        let frames = 512;
        let mut bufs: Vec<ChannelBuffers> = (0..2)
            .map(|_| ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();

        let mut peak: f32 = 0.0;
        let mut peak_after_tanh: f32 = 0.0;
        let mut sum_sq = 0.0f64;
        let mut n = 0u64;
        // 跳过起音头（前 5 块），统计第 5~25 块的稳态
        for block in 0..25 {
            synth.render_to_mixer(&mut bufs);
            if block < 5 {
                continue;
            }
            for b in &bufs {
                for &v in b.left.iter().chain(&b.right) {
                    peak = peak.max(v.abs());
                    peak_after_tanh = peak_after_tanh.max(v.tanh().abs());
                    sum_sq += (v as f64) * (v as f64);
                    n += 1;
                }
            }
        }
        let rms = (sum_sq / n.max(1) as f64).sqrt();
        println!(
            "[{name}] voice={} 峰值(limiter 前)={peak:.3} RMS={rms:.3} 峰值(tanh 后)={peak_after_tanh:.6}",
            synth.voice_count()
        );
    }
}
