//! 临时探针：nearest vs linear 插值的渲染耗时对比（CPU 路径）。
//!
//! 运行（release，否则数值无意义）：
//! YINHE_TEST_SFZ=... cargo test --release -p yinhe-synth --test tmp_interp_bench -- --nocapture
//!
//! interp 由 sf_parser 决定，需手动切换后跑两次对比。

use std::path::PathBuf;
use std::time::Instant;

use yinhe_mixer::ChannelBuffers;
use yinhe_synth::{CpuSynth, SynthEvent};

#[test]
fn bench_render_interp() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("跳过：未设置 YINHE_TEST_SFZ");
        return;
    };
    let mut synth = CpuSynth::new(48_000);
    synth
        .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
        .expect("load soundfont");

    // 88 键 × 4 层力度（352 voice，接近黑乐谱密集段）
    let mut events = Vec::new();
    for k in 21..109u8 {
        for layer in 0..4u8 {
            events.push(SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: k,
                velocity: 90 - layer * 5,
                end_sample: 48_000 * 60,
            });
        }
    }
    synth.load_events(events.clone());

    let frames = 512;
    let mut bufs: Vec<ChannelBuffers> = (0..2)
        .map(|_| ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();

    // 预热（线程池/缓存）
    synth.load_events(events.clone());
    for _ in 0..20 {
        synth.render_to_mixer(&mut bufs);
    }

    // 每轮重置事件（voice 会随样本播完而回收，必须重置才能全程满负荷）
    let blocks = 200;
    let mut best = f64::MAX;
    let mut voices = 0;
    for _ in 0..5 {
        synth.load_events(events.clone());
        let t0 = Instant::now();
        for _ in 0..blocks {
            synth.render_to_mixer(&mut bufs);
        }
        let dt = t0.elapsed().as_secs_f64();
        voices = synth.voice_count();
        best = best.min(dt);
    }
    let ms_per_block = best * 1000.0 / blocks as f64;
    let realtime = (blocks * frames) as f64 / 48_000.0 / best;
    println!(
        "voice={} 块={}：{:.2}ms/块（512 帧），实时倍率 {:.2}x，插值见 sf_parser",
        voices, blocks, ms_per_block, realtime
    );
}
