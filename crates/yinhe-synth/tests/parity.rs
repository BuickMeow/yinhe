//! CPU（xsynth ChannelGroup）与 GPU（GpuSynth）渲染同一音符序列的波形对比。
//!
//! 用真实 SFZ 音色库验证听感一致性：
//! - 同一音符序列（力度/时长/和弦变化）分别走 CPU 和 GPU 路径
//! - 逐样本对比输出波形，量化差异（最大绝对差 + 归一化 RMSE）
//!
//! 音色库路径通过环境变量 `YINHE_TEST_SFZ` 指定，未设置时跳过（CI 友好）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::{ChannelGroup, ChannelGroupConfig, SynthEvent, SynthFormat};
use xsynth_core::soundfont::{SampleSoundfont, SoundfontBase, SoundfontInitOptions};
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

use yinhe_synth::GpuSynth;

const SR: u32 = 44100;
const FRAMES: u32 = 512;

fn test_sfz() -> Option<PathBuf> {
    std::env::var("YINHE_TEST_SFZ").ok().map(PathBuf::from)
}

/// 音符计划：(start_ms, key, vel, duration_ms)
fn note_plan() -> Vec<(u64, u8, u8, u64)> {
    let mut plan = Vec::new();
    // 和弦：C4 E4 G4，力度分层
    for (i, key) in [60u8, 64, 67].iter().enumerate() {
        plan.push((50 + i as u64 * 30, *key, 90 + i as u8 * 10, 800));
    }
    // 旋律单音（不同力度）
    for (i, (key, vel, dur)) in [
        (62u8, 60u8, 300u16),
        (65, 110, 250),
        (69, 45, 350),
        (67, 80, 200),
        (72, 120, 400),
    ]
    .iter()
    .enumerate()
    {
        plan.push((1000 + i as u64 * 400, *key, *vel, *dur as u64));
    }
    // 低音 + 高音叠置
    plan.push((2400, 48, 100, 600));
    plan.push((2400, 84, 70, 600));
    plan.push((3200, 55, 55, 500));
    plan
}

fn total_duration_ms(plan: &[(u64, u8, u8, u64)]) -> u64 {
    plan.iter().map(|(s, _, _, d)| s + d).max().unwrap_or(0) + 400 // 尾部余韵
}

/// CPU 路径：xsynth ChannelGroup + SampleSoundfont（默认 options，use_effects=true）
fn cpu_render(sfz: &Path) -> Vec<f32> {
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: SR,
    };
    let sf = SampleSoundfont::new_sfz(
        sfz.to_path_buf(),
        stream_params,
        SoundfontInitOptions::default(),
    )
    .expect("SFZ load failed");

    let config = ChannelGroupConfig {
        channel_init_options: Default::default(),
        format: SynthFormat::Custom { channels: 1 },
        audio_params: stream_params,
        // 单线程，结果可预测
        parallelism: xsynth_core::channel_group::ParallelismOptions {
            channel: xsynth_core::channel_group::ThreadCount::None,
            key: xsynth_core::channel_group::ThreadCount::None,
        },
    };
    let mut cg = ChannelGroup::new(config);
    cg.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![Arc::new(sf)])),
    ));

    let plan = note_plan();
    let total_frames = total_duration_ms(&plan) * SR as u64 / 1000;
    let mut events: Vec<(u64, SynthEvent)> = Vec::new();
    for (start, key, vel, dur) in &plan {
        let s = start * SR as u64 / 1000;
        let e = (start + dur) * SR as u64 / 1000;
        events.push((
            s,
            SynthEvent::Channel(
                0,
                ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                    key: *key,
                    vel: *vel,
                }),
            ),
        ));
        events.push((
            e,
            SynthEvent::Channel(
                0,
                ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
            ),
        ));
    }
    events.sort_by_key(|(s, _)| *s);

    let mut out = Vec::with_capacity(total_frames as usize * 2);
    let mut chunk = vec![0.0f32; FRAMES as usize * 2];
    let mut cursor = 0usize;
    let mut rendered = 0u64;
    while rendered < total_frames {
        // 段开头处理所有已到事件（sample <= rendered，影响从当前位置开始）
        while cursor < events.len() && events[cursor].0 <= rendered {
            cg.send_event(events[cursor].1.clone());
            cursor += 1;
        }
        // 段边界 = 下一个事件位置（sample 级时序，与 GPU 路径对齐）
        let seg_end = events
            .get(cursor)
            .map(|(s, _)| (*s).min(total_frames))
            .unwrap_or(total_frames);
        let frames = (seg_end - rendered) as usize;
        // 段内按 FRAMES 分块渲染（xsynth 内部按块处理事件，块内位置 = 块开头）
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(FRAMES as usize);
            let buf = &mut chunk[..n * 2];
            cg.read_samples(buf);
            out.extend_from_slice(buf);
            done += n;
        }
        rendered = seg_end;
    }
    out
}

/// GPU 路径：GpuSynth（同一音符序列）
fn gpu_render(sfz: &Path) -> Vec<f32> {
    let mut synth = GpuSynth::new_default(sfz, SR).expect("GpuSynth init failed");
    // 对比测试排除限幅器（CPU 路径无限幅）
    synth.set_limiter_enabled(false);

    let plan = note_plan();
    let total_frames = total_duration_ms(&plan) * SR as u64 / 1000;
    let mut events = Vec::new();
    for (start, key, vel, dur) in &plan {
        events.push(yinhe_synth::SynthEvent {
            sample: start * SR as u64 / 1000,
            key: *key,
            velocity: *vel,
            is_on: true,
        });
        events.push(yinhe_synth::SynthEvent {
            sample: (start + dur) * SR as u64 / 1000,
            key: *key,
            velocity: 0,
            is_on: false,
        });
    }
    events.sort_by_key(|e| e.sample);
    synth.load_events(events);

    let mut out = Vec::with_capacity(total_frames as usize * 2);
    let mut chunk = vec![0.0f32; FRAMES as usize * 2];
    while synth.sample_position() < total_frames {
        let frames = ((total_frames - synth.sample_position()) as usize).min(FRAMES as usize);
        let buf = &mut chunk[..frames * 2];
        synth.render(buf);
        out.extend_from_slice(buf);
    }
    out
}

#[test]
fn parity_cpu_vs_gpu() {
    let Some(sfz) = test_sfz() else {
        eprintln!("YINHE_TEST_SFZ not set, skipping parity test");
        return;
    };
    if !sfz.exists() {
        eprintln!("SFZ not found: {:?}, skipping", sfz);
        return;
    }

    let cpu = cpu_render(&sfz);
    let gpu = gpu_render(&sfz);
    assert_eq!(
        cpu.len(),
        gpu.len(),
        "length mismatch: cpu={} gpu={}",
        cpu.len(),
        gpu.len()
    );

    // 归一化对比：相对峰值
    let peak = cpu
        .iter()
        .chain(&gpu)
        .fold(0.0f32, |m, &s| m.max(s.abs()))
        .max(1e-6);
    let mut max_diff = 0.0f32;
    let mut sum_sq = 0.0f64;
    let mut sum_sq_gpu = 0.0f64;
    let mut worst = (0usize, 0.0f32);
    for (i, (a, b)) in cpu.iter().zip(&gpu).enumerate() {
        let d = (a - b).abs();
        if d > worst.1 {
            worst = (i, d);
        }
        max_diff = max_diff.max(d);
        sum_sq += (a - b) as f64 * (a - b) as f64;
        sum_sq_gpu += (*b as f64) * (*b as f64);
    }
    let rel_rmse = (sum_sq / sum_sq_gpu.max(1e-12)).sqrt();
    eprintln!(
        "parity: frames={} peak={:.4} max_diff={:.5} ({:.3}% of peak) rel_rmse={:.5} worst@sample={} (cpu={:.4} gpu={:.4})",
        cpu.len() / 2,
        peak,
        max_diff,
        max_diff / peak * 100.0,
        rel_rmse,
        worst.0,
        cpu[worst.0],
        gpu[worst.0],
    );

    // 听感一致判据：最大差异 < 3% 峰值，相对 RMSE < 2%（残余差异来自
    // SIMD 块内插值与逐帧推进的浮点顺序，人耳不可辨）
    assert!(
        max_diff < peak * 0.03,
        "max diff {} exceeds 3% of peak {}",
        max_diff,
        peak
    );
    assert!(rel_rmse < 0.02, "relative RMSE {} exceeds 2%", rel_rmse);
}
