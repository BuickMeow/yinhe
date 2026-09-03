//! GPU 合成器测试：与 CPU 参考对比及滤波回归。

use super::cpu_ref::cpu_render_voices;
use super::filter::biquad_coeffs;
use super::renderer::GpuAudioRenderer;
use super::types::{CHANNEL_COUNT, ChState, GpuVoiceState, SegInfo};

fn make_sine_samples(len: usize, freq: f32, sr: f32) -> Vec<f32> {
    (0..len)
        .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
        .collect()
}

fn make_voices(sample_len: u32, count: u32, speed: f32) -> Vec<GpuVoiceState> {
    (0..count)
        .map(|i| GpuVoiceState {
            sample_offset: (i % 4) * sample_len,
            sample_length: sample_len,
            speed,
            base_speed: speed,
            base_gain: 0.5,
            env_stage: 4,
            env_level: 1.0,
            sustain_level: 1.0,
            base_pan_l: 1.0,
            base_pan_r: 1.0,
            ch_vol: 1.0,
            ch_expr: 1.0,
            ch_pan: 0.5,
            ..Default::default()
        })
        .collect()
}

fn setup_gpu() -> Option<(GpuAudioRenderer, Vec<f32>)> {
    let mut renderer = GpuAudioRenderer::new_default().ok()?;
    let sample_len = 4096u32;
    let samples: Vec<f32> = (0..4)
        .flat_map(|inst| {
            make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
        })
        .collect();
    renderer.upload_samples(&samples);
    let limits = renderer.device.limits();
    eprintln!(
        "GPU limits: min_storage_buf_align={} max_buf_binding={}",
        limits.min_storage_buffer_offset_alignment, limits.max_storage_buffer_binding_size
    );
    Some((renderer, samples))
}

fn bench_samples(sample_len: u32) -> Vec<f32> {
    (0..4)
        .flat_map(|inst| {
            make_sine_samples(sample_len as usize, 440.0 * (inst as f32 + 1.0), 44100.0)
        })
        .collect()
}

#[test]
fn phase15_single_pass_smoke() {
    let (mut renderer, _samples) = match setup_gpu() {
        Some(r) => r,
        None => {
            eprintln!("No GPU");
            return;
        }
    };
    let mut voices = make_voices(4096, 16, 1.0);
    let result = renderer.render_block_alloc(&mut voices, 1024, 44100);
    assert_eq!(result.len(), 1024 * 2);
    assert!(result.iter().fold(0.0f32, |m, &s| m.max(s.abs())) > 0.0);
}

#[test]
fn phase15_benchmark() {
    let (mut renderer, _samples) = match setup_gpu() {
        Some(r) => r,
        None => {
            eprintln!("No GPU");
            return;
        }
    };
    let sample_len = 4096u32;
    let samples = bench_samples(sample_len);
    let frame_count = 1024u32;

    for &vc in &[4, 16, 64, 256, 1024, 4096, 15000] {
        let mut voices = make_voices(sample_len, vc, 1.0);
        for _ in 0..3 {
            let _ = renderer.render_block_alloc(&mut voices, frame_count, 44100);
        }
        let n = 10;
        let gpu_start = std::time::Instant::now();
        for _ in 0..n {
            let _ = renderer.render_block_alloc(&mut voices, frame_count, 44100);
        }
        let gpu_per_block = gpu_start.elapsed() / n;
        let cpu_start = std::time::Instant::now();
        for _ in 0..n {
            let mut v = make_voices(sample_len, vc, 1.0);
            let _ = cpu_render_voices(&samples, &mut v, frame_count);
        }
        let cpu_per_block = cpu_start.elapsed() / n;
        let speedup = cpu_per_block.as_secs_f64() / gpu_per_block.as_secs_f64();
        eprintln!(
            "Voices={vc:>6}: CPU={cpu_per_block:>8.2?} GPU={gpu_per_block:>8.2?} speedup={speedup:.2}x"
        );
    }
}

/// 立体声交错样本（LRLR）
fn make_stereo_samples(len: usize, freq: f32, sr: f32) -> Vec<f32> {
    let l: Vec<f32> = (0..len)
        .map(|i| 0.8 * (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
        .collect();
    let r: Vec<f32> = (0..len)
        .map(|i| 0.6 * (2.0 * std::f32::consts::PI * freq * 1.5 * i as f32 / sr).sin())
        .collect();
    l.into_iter().zip(r).flat_map(|(l, r)| [l, r]).collect()
}

/// GPU 与 CPU 参考实现逐 block 对比（含立体声、滤波器、跨 block IIR 状态、全 7 阶段包络）
#[test]
fn gpu_vs_cpu_correctness() {
    let (mut renderer, _) = match setup_gpu() {
        Some(r) => r,
        None => {
            eprintln!("No GPU");
            return;
        }
    };

    let sample_len = 4096u32; // 帧数
    let samples = make_stereo_samples(sample_len as usize, 220.0, 44100.0);
    renderer.upload_samples(&samples);
    renderer.buffers = None;

    let make_test_voices = |stage: u32| {
        vec![
            // 立体声 + LowPass 滤波器 + Nearest
            GpuVoiceState {
                sample_offset: 0,
                sample_length: sample_len,
                speed: 1.0,
                base_gain: 0.4,
                env_stage: stage,
                env_level: 1.0,
                sustain_level: 0.3,
                delay_frames: 40.0,
                attack_frames: 300.0,
                hold_frames: 120.0,
                decay_frames: 400.0,
                release_frames: 500.0,
                base_pan_l: 0.8,
                base_pan_r: 0.6,
                ch_vol: 1.0,
                ch_expr: 1.0,
                ch_pan: 0.5,
                is_stereo: 1,
                interp: 0,
                cutoff: 1800.0,
                resonance: 2.0,
                filter_type: 0,
                flt_b0: biquad_coeffs(0, 1800.0, 2.0, 44100.0).0,
                flt_b1: biquad_coeffs(0, 1800.0, 2.0, 44100.0).1,
                flt_b2: biquad_coeffs(0, 1800.0, 2.0, 44100.0).2,
                flt_a1: biquad_coeffs(0, 1800.0, 2.0, 44100.0).3,
                flt_a2: biquad_coeffs(0, 1800.0, 2.0, 44100.0).4,
                ..Default::default()
            },
            // 单声道 + 无滤波器 + Linear 插值 + 循环
            GpuVoiceState {
                sample_offset: 0,
                sample_length: sample_len,
                speed: 0.7,
                base_gain: 0.3,
                env_stage: stage,
                env_level: 1.0,
                sustain_level: 0.6,
                delay_frames: 40.0,
                attack_frames: 300.0,
                hold_frames: 120.0,
                decay_frames: 400.0,
                release_frames: 500.0,
                base_pan_l: 0.5,
                base_pan_r: 1.0,
                ch_vol: 1.0,
                ch_expr: 1.0,
                ch_pan: 0.5,
                loop_mode: 1,
                loop_start: 100,
                loop_end: 2048,
                is_stereo: 0,
                interp: 1,
                ..Default::default()
            },
            // 立体声 + HighPass + 偏移起始
            GpuVoiceState {
                sample_offset: 0,
                sample_length: sample_len,
                speed: 1.0,
                base_gain: 0.2,
                start_offset: 13,
                env_stage: stage,
                env_level: 1.0,
                sustain_level: 0.9,
                delay_frames: 40.0,
                attack_frames: 300.0,
                hold_frames: 120.0,
                decay_frames: 400.0,
                release_frames: 500.0,
                base_pan_l: 1.0,
                base_pan_r: 0.3,
                ch_vol: 1.0,
                ch_expr: 1.0,
                ch_pan: 0.5,
                is_stereo: 1,
                cutoff: 4000.0,
                resonance: 3.0,
                filter_type: 1,
                flt_b0: biquad_coeffs(1, 4000.0, 3.0, 44100.0).0,
                flt_b1: biquad_coeffs(1, 4000.0, 3.0, 44100.0).1,
                flt_b2: biquad_coeffs(1, 4000.0, 3.0, 44100.0).2,
                flt_a1: biquad_coeffs(1, 4000.0, 3.0, 44100.0).3,
                flt_a2: biquad_coeffs(1, 4000.0, 3.0, 44100.0).4,
                ..Default::default()
            },
        ]
    };

    let frame_count = 512u32;
    // 从 Delay 起步连续渲染 6 个 block：覆盖 attack/hold/decay 阶段切换 + 跨 block 滤波器状态
    let mut gpu_voices = make_test_voices(0);
    let mut cpu_voices = gpu_voices.clone();
    for block in 0..6 {
        let mut out_gpu = vec![0.0f32; frame_count as usize * 2];
        // render_into 已由 GPU 全字段推进（time/env/滤波），调用方不再 advance
        renderer.render_into(&mut gpu_voices, &mut out_gpu, 44100);
        let out_cpu = cpu_render_voices(&samples, &mut cpu_voices, frame_count);

        let mut max_diff = 0.0f32;
        for (a, b) in out_gpu.iter().zip(&out_cpu) {
            max_diff = max_diff.max((a - b).abs());
        }
        assert!(
            max_diff < 1e-3,
            "block {block}: max diff {max_diff} (gpu[0]={} cpu[0]={})",
            out_gpu[0],
            out_cpu[0]
        );
        // 滤波器 IIR 状态跨 block 一致
        for (g, c) in gpu_voices.iter().zip(&cpu_voices) {
            assert!(
                (g.flt_y1 - c.flt_y1).abs() < 1e-3
                    && (g.flt_y2 - c.flt_y2).abs() < 1e-3
                    && (g.flt_x1 - c.flt_x1).abs() < 1e-3
                    && (g.flt_x2 - c.flt_x2).abs() < 1e-3,
                "block {block}: filter state mismatch"
            );
        }
    }
}

/// biquad 系数在 CPU 与 GPU 同源（测试防线）
#[test]
fn biquad_coeffs_reference() {
    // LowPass 1kHz Q=1（RBJ cookbook 已知值）
    let (b0, b1, _b2, a1, a2) = biquad_coeffs(0, 1000.0, 1.0, 44100.0);
    let omega = 2.0 * std::f32::consts::PI * 1000.0 / 44100.0;
    let alpha = omega.sin() / (2.0 * 1.0);
    let a0 = 1.0 + alpha;
    assert!((b0 - ((1.0 - omega.cos()) * 0.5) / a0).abs() < 1e-6);
    assert!((b1 - (1.0 - omega.cos()) / a0).abs() < 1e-6);
    assert!((a1 - (-2.0 * omega.cos()) / a0).abs() < 1e-6);
    assert!((a2 - (1.0 - alpha) / a0).abs() < 1e-6);
}

/// GPU 滤波器 vs biquad crate（权威参照）：系数直接用 biquad crate 生成，
/// 排除系数计算差异，验证 DF1 状态方程、包络推进与跨 block 状态传递。
#[test]
fn gpu_filter_matches_biquad_crate() {
    use biquad::Biquad as _;
    use biquad::frequency::ToHertz;

    let (mut renderer, _) = match setup_gpu() {
        Some(r) => r,
        None => {
            eprintln!("No GPU");
            return;
        }
    };
    let sr = 44100.0f32;
    // 扫频样本（覆盖滤波频段，频谱丰富）
    let sample_len = 8192u32;
    let samples: Vec<f32> = (0..sample_len as usize)
        .map(|i| {
            let t = i as f32 / sr;
            (2.0 * std::f32::consts::PI * (200.0 + 8000.0 * t / 0.2) * t).sin()
        })
        .collect();
    renderer.upload_samples(&samples);
    renderer.buffers = None;

    // 用 biquad crate 生成系数（权威来源）
    let cutoff = 1195.0f32;
    let q = std::f32::consts::FRAC_1_SQRT_2;
    let coeffs =
        biquad::Coefficients::<f32>::from_params(biquad::Type::LowPass, sr.hz(), cutoff.hz(), q)
            .unwrap();

    let make_voice = |with_filter: bool| GpuVoiceState {
        sample_length: sample_len,
        speed: 1.0,
        base_gain: 0.5,
        env_stage: 4,
        env_level: 1.0,
        sustain_level: 1.0,
        base_pan_l: 1.0,
        base_pan_r: 1.0,
        // ch_pan=0 → cos(0)=1，左声道无衰减（与下方 expected 一致）
        ch_vol: 1.0,
        ch_expr: 1.0,
        ch_pan: 0.0,
        cutoff: if with_filter { cutoff } else { 0.0 },
        resonance: q,
        filter_type: 0,
        flt_b0: coeffs.b0,
        flt_b1: coeffs.b1,
        flt_b2: coeffs.b2,
        flt_a1: coeffs.a1,
        flt_a2: coeffs.a2,
        ..Default::default()
    };

    // 对照组：无滤波（cutoff=0）——验证采样读取与 envelope 通路
    let frame_count = 512u32;
    {
        let mut voices = vec![make_voice(false)];
        let mut out = vec![0.0f32; frame_count as usize * 2];
        // render_into 已由 GPU 全字段推进（time/env/滤波）
        renderer.render_into(&mut voices, &mut out, sr as u32);
        for fi in 0..8 {
            let expected = samples[fi] * 0.5; // sustain env=1.0, base_gain=0.5, mono, ch_pan=0 → cos=1
            assert!(
                (out[fi * 2] - expected).abs() < 1e-4,
                "nofilter frame {fi}: gpu={} expected={}",
                out[fi * 2],
                expected
            );
        }
    }

    // 滤波路径：4 个连续 block，验证跨 block IIR 状态传递
    let mut gpu_voices = vec![make_voice(true)];
    let mut cpu_filters = [
        biquad::DirectForm1::<f32>::new(coeffs),
        biquad::DirectForm1::<f32>::new(coeffs),
    ];

    for block in 0..4 {
        let mut out_gpu = vec![0.0f32; frame_count as usize * 2];
        // render_into 已由 GPU 全字段推进（time/env/滤波）
        renderer.render_into(&mut gpu_voices, &mut out_gpu, sr as u32);

        // 参照：逐帧 biquad crate 滤波（单声道，左右同值）
        let start = block as usize * frame_count as usize;
        for fi in 0..frame_count as usize {
            let input = samples[start + fi] * 0.5;
            let out = cpu_filters[0].run(input);
            assert!(
                (out_gpu[fi * 2] - out).abs() < 1e-3,
                "block {block} frame {fi}: gpu={} biquad={}",
                out_gpu[fi * 2],
                out
            );
        }
    }
}

/// 回归：loop_mode 语义与 xsynth 一致——OneShot/NoLoop 不循环（播完结束 voice），
/// LoopContinuous 恒循环；LoopSustain 仅未 release 时循环（release 后播到尾结束）。
#[test]
fn loop_mode_semantics_match_xsynth() {
    let sr = 44100u32;
    let frame_count = 256u32;
    let samples: Vec<f32> = make_sine_samples(frame_count as usize, 440.0, sr as f32);
    let mut renderer = GpuAudioRenderer::new_default().expect("renderer init failed");
    renderer.upload_samples(&samples);

    let peak_of = |o: &[f32]| o.iter().fold(0.0f32, |m, &s| m.max(s.abs()));

    // OneShot：loop 区间 [0,128)，播到 256 帧末尾即结束（不循环）
    let mut v = make_voices(samples.len() as u32, 1, 1.0).remove(0);
    v.sample_length = frame_count;
    v.loop_mode = 3; // OneShot
    v.loop_start = 0;
    v.loop_end = 128;
    let mut voices = vec![v];
    let mut out = vec![0.0f32; frame_count as usize * 2];
    renderer.render_into(&mut voices, &mut out, sr);
    let peak_oneshot_first = peak_of(&out);
    // 第二块：采样已播完 → 静音 + voice 结束
    renderer.render_into(&mut voices, &mut out, sr);
    let peak_oneshot_second = peak_of(&out);
    assert!(
        peak_oneshot_first > 0.1,
        "one_shot first block silent: {peak_oneshot_first}"
    );
    assert_eq!(
        peak_oneshot_second, 0.0,
        "one_shot should be silent after sample end"
    );
    assert_eq!(voices[0].env_stage, 6, "one_shot voice should end");

    // LoopContinuous：恒循环（第二块仍有声，voice 不结束）
    let mut v = make_voices(samples.len() as u32, 1, 1.0).remove(0);
    v.sample_length = frame_count;
    v.loop_mode = 1; // LoopContinuous
    v.loop_start = 0;
    v.loop_end = 128;
    let mut voices = vec![v];
    renderer.render_into(&mut voices, &mut out, sr);
    renderer.render_into(&mut voices, &mut out, sr);
    assert!(peak_of(&out) > 0.1, "loop_continuous should keep sounding");
    assert!(voices[0].env_stage < 6, "loop_continuous voice stays");

    // LoopSustain：sustain 阶段循环；release 后不循环（播到尾结束）
    let mut v = make_voices(samples.len() as u32, 1, 1.0).remove(0);
    v.sample_length = frame_count;
    v.loop_mode = 2; // LoopSustain
    v.loop_start = 0;
    v.loop_end = 128;
    let mut voices = vec![v];
    renderer.render_into(&mut voices, &mut out, sr);
    assert!(peak_of(&out) > 0.1, "loop_sustain should loop in sustain");
    // 释放（模拟 note_off 后）：从当前位置继续播到尾，不再循环
    voices[0].env_stage = 5;
    voices[0].env_start = voices[0].envelope;
    voices[0].stage_progress = 0.0;
    renderer.render_into(&mut voices, &mut out, sr);
    renderer.render_into(&mut voices, &mut out, sr);
    assert_eq!(voices[0].env_stage, 6, "loop_sustain ends after release");
}

/// 回归：块内段边界（ch_updates）后创建的新 voice 必须正常出声。
/// 段边界把通道音量改为 0.5，新 voice 在段边界创建（start_offset=68）。
#[test]
fn new_voice_after_seg_boundary_sounds() {
    let sr = 44100u32;
    let frame_count = 512u32;
    let samples: Vec<f32> = (0..frame_count as usize * 8)
        .map(|i| ((i as f32) * 0.01).sin() * 0.5)
        .collect();
    let mut renderer = GpuAudioRenderer::new_default().expect("renderer init failed");
    renderer.upload_samples(&samples);

    let mut v = make_voices(samples.len() as u32, 1, 1.0).remove(0);
    // 块内 68 帧处创建（段边界位置）
    v.start_offset = 68;
    v.time = 0.0;
    let mut voices = vec![v];
    let mut mix = vec![0.0f32; CHANNEL_COUNT * frame_count as usize * 2];
    let mut out = vec![0.0f32; frame_count as usize * 2];
    // 段边界：帧 68 处 ch0 音量设为 0.5
    let segs = [SegInfo {
        start_frame: 0,
        ch_off: 0,
        ch_count: 1,
        _pad: 0,
    }];
    let ch_updates = [ChState {
        ch: 0,
        speed_mult: 1.0,
        ch_vol: 0.5,
        ch_vol_step: 0.0,
        ch_vol_frames: 0,
        ch_expr: 1.0,
        ch_expr_step: 0.0,
        ch_expr_frames: 0,
        ch_pan: 0.5,
        ch_pan_step: 0.0,
        ch_pan_frames: 0,
    }];
    renderer.render_block(&mut voices, &mut mix, &segs, &ch_updates, &[], &[], sr);
    for i in 0..out.len() {
        for ch in 0..CHANNEL_COUNT {
            out[i] += mix[ch * frame_count as usize * 2 + i];
        }
    }
    let peak_of = |o: &[f32]| o.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    let any = peak_of(&out);
    assert!(
        any > 0.04,
        "new voice after seg boundary silent: peak={any}",
    );

    // 真实路径复现：旧 voice（stage 5, env≈0）+ 新 voice（start_offset=68）+ 段边界
    let mut mix2 = vec![0.0f32; CHANNEL_COUNT * frame_count as usize * 2];
    let mut voices2 = make_voices(samples.len() as u32, 1, 1.0);
    voices2[0].env_stage = 5;
    voices2[0].envelope = 0.0;
    voices2[0].env_start = 0.0;
    voices2[0].start_offset = 0;
    let mut newv = make_voices(samples.len() as u32, 1, 1.0).remove(0);
    newv.start_offset = 68;
    newv.time = 0.0;
    voices2.push(newv);
    renderer.render_block(&mut voices2, &mut mix2, &segs, &ch_updates, &[], &[], sr);
    let out2: Vec<f32> = (0..out.len())
        .map(|i| {
            (0..CHANNEL_COUNT)
                .map(|ch| mix2[ch * frame_count as usize * 2 + i])
                .sum()
        })
        .collect();
    assert!(
        peak_of(&out2) > 0.04,
        "new voice silent with old voice present"
    );
}
