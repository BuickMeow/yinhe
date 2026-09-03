//! CPU 端 voice 推进（解析公式，块级推进）。

use super::types::GpuVoiceState;

/// CPU 端推进 voice 状态：用解析公式直接计算，不逐帧迭代。
/// 7 阶段: 0=Delay, 1=Attack(线性), 2=Hold, 3=Decay(指数), 4=Sustain, 5=Release(指数), 6=Finished
pub fn advance_voices(voices: &mut [GpuVoiceState], frame_count: u32) {
    for voice in voices.iter_mut() {
        let active_frames = frame_count.saturating_sub(voice.start_offset);
        voice.start_offset = 0;
        if voice.env_stage >= 6 || active_frames == 0 {
            continue;
        }
        voice.time += voice.speed * active_frames as f32;

        // 循环回绕（与 shader/xsynth 一致：> loop_end，loop 区间含 end）
        let has_loop = voice.loop_mode > 0 && voice.loop_end > voice.loop_start;
        if has_loop && voice.time > voice.loop_end as f32 {
            let loop_len = (voice.loop_end - voice.loop_start) as f32;
            if loop_len > 0.0 {
                // 回绕到回绕区 [end+1, end+len]（不落回恒等区 [start, end]）：
                // xsynth 原始位置永不回绕，回绕环 = len 样本（不含 end）；
                // 落回恒等区会让下一段相位漂移（多播 end，循环周期变 len+1）。
                let off = (voice.time - voice.loop_end as f32 - 1.0) % loop_len;
                voice.time = voice.loop_end as f32 + 1.0 + off;
            }
        }

        let peak = voice.env_level;
        let sus = voice.sustain_level * peak;
        let mut remaining = active_frames as f32;

        while remaining > 0.0 && voice.env_stage < 6 {
            match voice.env_stage {
                0 => {
                    // Delay
                    let dur = voice.delay_frames - voice.stage_progress;
                    if remaining < dur {
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        remaining -= dur;
                        voice.env_stage = 1;
                        voice.stage_progress = 0.0;
                    }
                }
                1 => {
                    // Attack: 线性
                    let dur = voice.attack_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.attack_frames;
                        voice.envelope = voice.env_start + (peak - voice.env_start) * t;
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = peak;
                        remaining -= dur;
                        voice.env_stage = 2;
                        voice.stage_progress = 0.0;
                    }
                }
                2 => {
                    // Hold
                    let dur = voice.hold_frames - voice.stage_progress;
                    if remaining < dur {
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.decay_start = voice.envelope; // 进入 Decay 的起点 = 当前 amp
                        remaining -= dur;
                        voice.env_stage = 3;
                        voice.stage_progress = 0.0;
                    }
                }
                3 => {
                    // Decay: 指数 (1-t)^8，从 decay_start 到 sustain
                    let dur = voice.decay_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.decay_frames;
                        voice.envelope = sus + (voice.decay_start - sus) * (1.0 - t).powi(8);
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = sus;
                        remaining -= dur;
                        voice.env_stage = 4;
                        voice.stage_progress = 0.0;
                    }
                }
                4 => {
                    // Sustain: envelope 恒为 sus（与 GPU 逐帧推进一致）
                    voice.envelope = sus;
                    remaining = 0.0;
                } // Sustain: 无限
                5 => {
                    // Release: 指数 (1-t)^8
                    let dur = voice.release_frames - voice.stage_progress;
                    if remaining < dur {
                        let t = (voice.stage_progress + remaining) / voice.release_frames;
                        voice.envelope = voice.env_start * (1.0 - t).powi(8);
                        voice.stage_progress += remaining;
                        remaining = 0.0;
                    } else {
                        voice.envelope = 0.0;
                        remaining -= dur;
                        voice.env_stage = 6;
                        voice.stage_progress = 0.0;
                    }
                }
                _ => break,
            }
        }
    }
}
