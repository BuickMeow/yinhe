//! SoA 渲染内核与现有 AoS（`CpuVoice`）的逐样本对比测试。
//!
//! 判据：同一 `KeyInfo`/通道状态/lane 参数下，两种实现的输出必须**逐位
//! 一致**（浮点顺序刻意保持与 AoS 相同）。覆盖基础播放、立体声插值、
//! 滤波、循环、释放、踏板保持、段内起始等路径。

use std::sync::Arc;

use fearless_simd::{Level, dispatch};

use super::*;
use crate::cpu_synth::voice::CpuVoice;

const SR: u32 = 48_000;

fn wave(len: usize) -> Arc<[f32]> {
    (0..len)
        .map(|i| (i as f32 * 0.05).sin() * 0.5)
        .collect::<Vec<_>>()
        .into()
}

fn wave_stereo(frames: usize) -> Arc<[f32]> {
    let mut v = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        v.push((i as f32 * 0.05).sin() * 0.5);
        v.push((i as f32 * 0.04).cos() * 0.4);
    }
    v.into()
}

fn base_info(sample: Arc<[f32]>) -> KeyInfo {
    KeyInfo {
        sample_data: sample,
        sample_rate: SR,
        volume: 1.0,
        pan: 0.5,
        speed_mult: 1.0,
        ..Default::default()
    }
}

/// 单段渲染对比：SoA 与 AoS 输出逐位一致。
fn compare_with_aos(
    info: &KeyInfo,
    end_sample: u64,
    start_offset: u32,
    frames: usize,
    sample_start: u64,
    damper: bool,
) {
    let ch = ChannelState::new(SR);

    let mut soa = VoiceSoa::new();
    let slot = soa.init_lane(info, 0, end_sample, start_offset, SR, &ch);
    assert_eq!(slot, 0);

    let mut aos = CpuVoice::new(info, 0, 60, 100, end_sample, start_offset, SR, &ch);

    let mut out_soa = vec![0f32; frames * 2];
    let mut out_aos = vec![0f32; frames * 2];

    let level = Level::new();
    let damper_flags = [damper; 32];
    dispatch!(level, simd => soa.render(
        simd, &mut out_soa, 0, frames, sample_start, &damper_flags
    ));
    aos.render_block(&mut out_aos, frames, 0, sample_start, damper, 0);

    for (i, (a, b)) in out_soa.iter().zip(&out_aos).enumerate() {
        assert_eq!(
            a, b,
            "样本 {i} 不一致（SoA={a} AoS={b}）；frames={frames} start_offset={start_offset}"
        );
    }
    assert_eq!(out_soa, out_aos);
}

/// 基础：单声道、无循环、无滤波、无释放。
#[test]
fn render_matches_aos_basic() {
    let info = base_info(wave(200));
    compare_with_aos(&info, 1_000_000, 0, 256, 0, false);
}

/// 段内起始：voice 在段内第 100 帧才开始发声。
#[test]
fn render_matches_aos_start_offset() {
    let info = base_info(wave(200));
    compare_with_aos(&info, 1_000_000, 100, 256, 0, false);
}

/// 立体声 + 线性插值 + per-voice 滤波。
#[test]
fn render_matches_aos_stereo_interp_filter() {
    let mut info = base_info(wave_stereo(200));
    info.interp = 1;
    info.cutoff = 4_000.0;
    info.resonance = 0.707;
    compare_with_aos(&info, 1_000_000, 0, 256, 0, false);
}

/// 循环（LoopContinuous）跨回绕点。
#[test]
fn render_matches_aos_loop() {
    let mut info = base_info(wave(200));
    info.loop_mode = LoopMode::LoopContinuous;
    info.loop_start = 50;
    info.loop_end = 150;
    compare_with_aos(&info, 1_000_000, 0, 512, 0, false);
}

/// 音符到期释放（end_sample 在段内）。
#[test]
fn render_matches_aos_release() {
    let info = base_info(wave(200));
    compare_with_aos(&info, 100, 0, 512, 0, false);
}

/// 到期时踏板按住：只标记 held，不释放。
#[test]
fn render_matches_aos_held_by_damper() {
    let info = base_info(wave(200));
    compare_with_aos(&info, 100, 0, 512, 0, true);
}

/// 多 lane：不同参数混在一个块内，与逐个 AoS voice 输出一致。
#[test]
fn render_matches_aos_multi_lane() {
    let ch = ChannelState::new(SR);
    let mut soa = VoiceSoa::new();

    let info_a = base_info(wave(300));
    let mut info_b = base_info(wave_stereo(300));
    info_b.interp = 1;
    info_b.cutoff = 3_000.0;
    let mut info_c = base_info(wave(100));
    info_c.loop_mode = LoopMode::LoopContinuous;
    info_c.loop_start = 10;
    info_c.loop_end = 90;

    let frames = 256;
    soa.init_lane(&info_a, 0, 1_000_000, 0, SR, &ch);
    soa.init_lane(&info_b, 0, 100, 0, SR, &ch);
    soa.init_lane(&info_c, 0, 1_000_000, 30, SR, &ch);

    let mut aos_a = CpuVoice::new(&info_a, 0, 60, 100, 1_000_000, 0, SR, &ch);
    let mut aos_b = CpuVoice::new(&info_b, 0, 62, 100, 100, 0, SR, &ch);
    let mut aos_c = CpuVoice::new(&info_c, 0, 64, 100, 1_000_000, 30, SR, &ch);

    let mut out_soa = vec![0f32; frames * 2];
    let mut out_a = vec![0f32; frames * 2];
    let mut out_b = vec![0f32; frames * 2];
    let mut out_c = vec![0f32; frames * 2];

    let level = Level::new();
    dispatch!(level, simd => soa.render(
        simd, &mut out_soa, 0, frames, 0, &[false; 32]
    ));
    aos_a.render_block(&mut out_a, frames, 0, 0, false, 0);
    aos_b.render_block(&mut out_b, frames, 0, 0, false, 0);
    aos_c.render_block(&mut out_c, frames, 0, 0, false, 0);

    for i in 0..frames * 2 {
        let expected = out_a[i] + out_b[i] + out_c[i];
        assert_eq!(out_soa[i], expected, "样本 {i} 不一致");
    }
}
