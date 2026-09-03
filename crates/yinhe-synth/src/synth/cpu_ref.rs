//! CPU 参考实现与 release 回归（与 GPU shader 逐帧对应）。

use super::types::GpuVoiceState;

#[cfg(test)]
use super::renderer::GpuAudioRenderer;
#[cfg(test)]
use super::types::ReleaseCmd;

/// 最小复现：sustain 阶段 voice + frame=0 release 指令，验证 release 推进正确
#[test]
fn release_cmd_advances_envelope() {
    let mut renderer = GpuAudioRenderer::new_default().expect("renderer");
    renderer.upload_samples(&[0.5f32; 4096]);

    let voice = GpuVoiceState {
        sample_offset: 0,
        sample_length: 4096,
        speed: 1.0,
        base_speed: 1.0,
        base_gain: 1.0,
        time: 0.0,
        start_offset: 0,
        channel: 0,
        envelope: 0.7,
        env_stage: 4, // Sustain
        stage_progress: 0.0,
        env_level: 1.0,
        sustain_level: 0.7,
        env_start: 0.7,
        decay_start: 0.7,
        delay_frames: 0.0,
        attack_frames: 0.0,
        hold_frames: 0.0,
        decay_frames: 0.0,
        release_frames: 39690.0,
        base_pan_l: 1.0,
        base_pan_r: 1.0,
        ch_vol: 1.0,
        ch_vol_step: 0.0,
        ch_vol_frames: 0,
        ch_expr: 1.0,
        ch_expr_step: 0.0,
        ch_expr_frames: 0,
        ch_pan: 0.5,
        ch_pan_step: 0.0,
        ch_pan_frames: 0,
        loop_start: 0,
        loop_end: 0,
        loop_mode: 0,
        is_stereo: 0,
        interp: 0,
        cutoff: 0.0,
        resonance: 0.0,
        filter_type: 0,
        flt_b0: 0.0,
        flt_b1: 0.0,
        flt_b2: 0.0,
        flt_a1: 0.0,
        flt_a2: 0.0,
        flt_x1: 0.0,
        flt_x2: 0.0,
        flt_y1: 0.0,
        flt_y2: 0.0,
        flt_x1r: 0.0,
        flt_x2r: 0.0,
        flt_y1r: 0.0,
        flt_y2r: 0.0,
    };
    let releases = [ReleaseCmd {
        frame: 0,
        vid: 0,
        mode: 5,
        _pad: 0,
    }];
    let mut mix = vec![0.0f32; 32 * 1024 * 2];
    let mut voices = vec![voice];
    renderer.render_block(&mut voices, &mut mix, &[], &[], &releases, &[], 44100);
    let voice = &voices[0];
    let t = 1024.0f32 / 39690.0;
    let expected_env = 0.7 * (1.0 - t).powi(8);
    eprintln!(
        "release test: stage={} env={:.6} expected_env={:.6} progress={} ch_pan={} mix_peak={}",
        voice.env_stage,
        voice.envelope,
        expected_env,
        voice.stage_progress,
        voice.ch_pan,
        mix.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    );
    assert_eq!(voice.env_stage, 5, "voice should be in release");
    assert!(
        (voice.envelope - expected_env).abs() < 1e-4,
        "env {} vs expected {}",
        voice.envelope,
        expected_env
    );
}

/// 多块渲染：release 指令只应在指定帧应用一次，prog 每块 +1024（块大小）。
#[test]
fn release_progress_advances_full_block() {
    let mut renderer = GpuAudioRenderer::new_default().expect("renderer");
    renderer.upload_samples(&[0.5f32; 4096]);

    let make_voice = |vid: u32| GpuVoiceState {
        sample_offset: 0,
        sample_length: 4096,
        speed: 1.0,
        base_speed: 1.0,
        base_gain: 1.0,
        time: 0.0,
        start_offset: 0,
        channel: vid,
        envelope: 0.7,
        env_stage: 4,
        stage_progress: 0.0,
        env_level: 1.0,
        sustain_level: 0.7,
        env_start: 0.7,
        decay_start: 0.7,
        delay_frames: 0.0,
        attack_frames: 0.0,
        hold_frames: 0.0,
        decay_frames: 0.0,
        release_frames: 100000.0,
        base_pan_l: 1.0,
        base_pan_r: 1.0,
        ch_vol: 1.0,
        ch_vol_step: 0.0,
        ch_vol_frames: 0,
        ch_expr: 1.0,
        ch_expr_step: 0.0,
        ch_expr_frames: 0,
        ch_pan: 0.5,
        ch_pan_step: 0.0,
        ch_pan_frames: 0,
        loop_start: 0,
        loop_end: 0,
        loop_mode: 0,
        is_stereo: 0,
        interp: 0,
        cutoff: 0.0,
        resonance: 0.0,
        filter_type: 0,
        flt_b0: 0.0,
        flt_b1: 0.0,
        flt_b2: 0.0,
        flt_a1: 0.0,
        flt_a2: 0.0,
        flt_x1: 0.0,
        flt_x2: 0.0,
        flt_y1: 0.0,
        flt_y2: 0.0,
        flt_x1r: 0.0,
        flt_x2r: 0.0,
        flt_y1r: 0.0,
        flt_y2r: 0.0,
    };
    let mut voices = vec![make_voice(0), make_voice(1)];
    // 第一块：两个 release 指令（frame 100 / 500）
    let releases = [
        ReleaseCmd {
            frame: 100,
            vid: 0,
            mode: 5,
            _pad: 0,
        },
        ReleaseCmd {
            frame: 500,
            vid: 1,
            mode: 5,
            _pad: 0,
        },
    ];
    let mut mix = vec![0.0f32; 32 * 1024 * 2];
    renderer.render_block(&mut voices, &mut mix, &[], &[], &releases, &[], 44100);
    eprintln!(
        "block1: v0 stage={} prog={:.0} | v1 stage={} prog={:.0}",
        voices[0].env_stage,
        voices[0].stage_progress,
        voices[1].env_stage,
        voices[1].stage_progress
    );
    assert_eq!(voices[0].env_stage, 5);
    assert_eq!(voices[0].stage_progress, 924.0); // 1024-100
    assert_eq!(voices[1].env_stage, 5);
    assert_eq!(voices[1].stage_progress, 524.0); // 1024-500

    // 第二块：无 release，prog 应该 +1024
    renderer.render_block(&mut voices, &mut mix, &[], &[], &[], &[], 44100);
    eprintln!(
        "block2: v0 stage={} prog={:.0} | v1 stage={} prog={:.0}",
        voices[0].env_stage,
        voices[0].stage_progress,
        voices[1].env_stage,
        voices[1].stage_progress
    );
    assert_eq!(voices[0].stage_progress, 1948.0);
    assert_eq!(voices[1].stage_progress, 1548.0);
}

/// CPU reference implementation (与 GPU shader pass1 逐帧逻辑完全对应).
/// 7 阶段: 0=Delay, 1=Attack, 2=Hold, 3=Decay, 4=Sustain, 5=Release, 6=Finished
/// 立体声采样 + 插值 + per-voice biquad 滤波均与 shader 一致，用于对比测试。
pub fn cpu_render_voices(
    sample_data: &[f32],
    voices: &mut [GpuVoiceState],
    frame_count: u32,
) -> Vec<f32> {
    let mut output = vec![0.0f32; frame_count as usize * 2];
    for voice in voices.iter_mut() {
        for fi in 0..frame_count as usize {
            if voice.env_stage >= 6 {
                break;
            }
            if fi < voice.start_offset as usize {
                continue;
            }
            let frame_in_voice = fi - voice.start_offset as usize;

            // 通道渐变逐帧推进（与 shader/xsynth ValueLerp 一致）
            if voice.ch_vol_frames > 0 {
                voice.ch_vol += voice.ch_vol_step;
                voice.ch_vol_frames -= 1;
            }
            if voice.ch_expr_frames > 0 {
                voice.ch_expr += voice.ch_expr_step;
                voice.ch_expr_frames -= 1;
            }
            if voice.ch_pan_frames > 0 {
                voice.ch_pan += voice.ch_pan_step;
                voice.ch_pan_frames -= 1;
            }
            let ch_vol = voice.ch_vol * voice.ch_expr;
            let ch_gain = voice.base_gain * ch_vol * ch_vol;
            let ch_ang = voice.ch_pan * std::f32::consts::FRAC_PI_2;
            let ch_pan_l = voice.base_pan_l * ch_ang.cos();
            let ch_pan_r = voice.base_pan_r * ch_ang.sin();

            let t = voice.time + frame_in_voice as f32 * voice.speed;
            let mut idx = t as u32;
            let frac = t - idx as f32;
            let max_idx = voice.sample_length.saturating_sub(1);

            // 循环回绕（与 shader/xsynth 一致：> loop_end，loop 区间含 end）
            let has_loop = voice.loop_mode > 0 && voice.loop_end > voice.loop_start;
            if has_loop && idx > voice.loop_end {
                let loop_len = voice.loop_end - voice.loop_start;
                if loop_len > 0 {
                    idx = (idx - voice.loop_end - 1) % loop_len + voice.loop_start;
                }
            }

            if idx < voice.sample_length {
                let scale = 1 + voice.is_stereo as usize;
                let i = voice.sample_offset as usize + idx as usize * scale;
                let (mut l0, mut r0) = if voice.is_stereo == 1 {
                    (sample_data[i], sample_data[i + 1])
                } else {
                    let s = sample_data[i];
                    (s, s)
                };
                if voice.interp == 1 && idx < max_idx {
                    let j = i + scale;
                    let (l1, r1) = if voice.is_stereo == 1 {
                        (sample_data[j], sample_data[j + 1])
                    } else {
                        let s = sample_data[j];
                        (s, s)
                    };
                    l0 += (l1 - l0) * frac;
                    r0 += (r1 - r0) * frac;
                }

                let mut s_l = l0 * ch_gain * voice.envelope;
                let mut s_r = r0 * ch_gain * voice.envelope;
                if voice.cutoff > 0.0 {
                    // 单声道样本只用一组滤波器，右声道复用左声道输出（与 shader/xsynth 一致）
                    let (x1, x2, y1, y2) = (voice.flt_x1, voice.flt_x2, voice.flt_y1, voice.flt_y2);
                    let out_l = voice.flt_b0 * s_l + voice.flt_b1 * x1 + voice.flt_b2 * x2
                        - voice.flt_a1 * y1
                        - voice.flt_a2 * y2;
                    voice.flt_x1 = s_l;
                    voice.flt_x2 = x1;
                    voice.flt_y1 = out_l;
                    voice.flt_y2 = y1;
                    s_l = out_l;
                    if voice.is_stereo == 1 {
                        let (x1r, x2r, y1r, y2r) =
                            (voice.flt_x1r, voice.flt_x2r, voice.flt_y1r, voice.flt_y2r);
                        let out_r = voice.flt_b0 * s_r + voice.flt_b1 * x1r + voice.flt_b2 * x2r
                            - voice.flt_a1 * y1r
                            - voice.flt_a2 * y2r;
                        voice.flt_x1r = s_r;
                        voice.flt_x2r = x1r;
                        voice.flt_y1r = out_r;
                        voice.flt_y2r = y1r;
                        s_r = out_r;
                    } else {
                        s_r = s_l;
                    }
                }
                output[fi * 2] += s_l * ch_pan_l;
                output[fi * 2 + 1] += s_r * ch_pan_r;
            }
            advance_env_cpu(voice);
        }
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.time += voice.speed * active_frames as f32;
        voice.start_offset = 0;
    }
    output
}

/// 逐帧推进 envelope（与 shader `advance_env` 完全对应）。
fn advance_env_cpu(v: &mut GpuVoiceState) {
    if v.env_stage >= 6 {
        return;
    }
    let peak = v.env_level;
    let sus = v.sustain_level * peak;
    match v.env_stage {
        0 => {
            // Delay
            if v.stage_progress + 1.0 >= v.delay_frames {
                v.env_stage = 1;
                v.stage_progress = 0.0;
            } else {
                v.stage_progress += 1.0;
            }
        }
        1 => {
            // Attack: 线性
            let n = v.stage_progress + 1.0;
            if n >= v.attack_frames {
                v.envelope = peak;
                v.env_stage = 2;
                v.stage_progress = 0.0;
            } else {
                v.envelope = v.env_start + (peak - v.env_start) * (n / v.attack_frames);
                v.stage_progress = n;
            }
        }
        2 => {
            // Hold
            if v.stage_progress + 1.0 >= v.hold_frames {
                v.decay_start = v.envelope; // 进入 Decay 的起点 = 当前 amp
                v.env_stage = 3;
                v.stage_progress = 0.0;
            } else {
                v.stage_progress += 1.0;
            }
        }
        3 => {
            // Decay: 指数 (1-t)^8，从 decay_start 到 sustain
            let n = v.stage_progress + 1.0;
            if n >= v.decay_frames {
                v.envelope = sus;
                v.env_stage = 4;
                v.stage_progress = 0.0;
            } else {
                let t = n / v.decay_frames;
                v.envelope = sus + (v.decay_start - sus) * (1.0 - t).powi(8);
                v.stage_progress = n;
            }
        }
        4 => {
            // Sustain
            v.envelope = sus;
        }
        5 => {
            // Release: 指数 (1-t)^8
            let n = v.stage_progress + 1.0;
            if n >= v.release_frames {
                v.envelope = 0.0;
                v.env_stage = 6;
                v.stage_progress = 0.0;
            } else {
                let t = n / v.release_frames;
                v.envelope = v.env_start * (1.0 - t).powi(8);
                v.stage_progress = n;
            }
        }
        _ => {}
    }
}
