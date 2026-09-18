//! GpuSynth 单元测试（原 gpu_synth.rs 内测试模块；文件过长拆出）。
//!
//! 本文件即 `gpu_synth::tests` 模块体（`#[cfg(test)] mod tests;` 引入）。

use super::*;
use crate::GpuVoiceState;

fn first_sample_ptr(s: &GpuSynth) -> *const f32 {
    s.port_key_maps[0]
        .iter()
        .flat_map(|e| e.map.iter())
        .flatten()
        .map(|info| info.sample_data.as_ptr())
        .next()
        .unwrap_or(std::ptr::null())
}

/// 进程级解析缓存：同路径第二次加载命中缓存，样本 Arc 跨实例共享。
#[test]
fn soundfont_parse_cache_shared_across_instances() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("YINHE_TEST_SFZ not set, skipping");
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    // 预热缓存：并行测试下也保证后续两次都是命中（不依赖执行顺序）。
    let mut warm = GpuSynth::new_default(44_100).expect("GpuSynth warm");
    warm.load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("warm load");

    let mut a = GpuSynth::new_default(44_100).expect("GpuSynth a");
    a.load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("load a");
    let mut b = GpuSynth::new_default(44_100).expect("GpuSynth b");
    b.load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("load b");

    let ptr_a = first_sample_ptr(&a);
    let ptr_b = first_sample_ptr(&b);
    assert!(!ptr_a.is_null(), "样本指针不应为空");
    assert_eq!(ptr_a, ptr_b, "同路径两次加载应共享同一份样本内存（Arc）");
}

/// 回归：seek 清空 voices 后到下一个音符之间必须静音——复用缓冲的残留
/// 音频会被原样写进混音台，导致空白区循环播放上一块（4096 帧）的余韵。
#[test]
fn seek_to_silence_without_voices() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("YINHE_TEST_SFZ not set, skipping");
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("load");
    synth.finish_soundfont_load();
    synth.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: 44_100,
    }]);

    let peak = |buffers: &[yinhe_mixer::ChannelBuffers]| {
        buffers
            .iter()
            .flat_map(|b| b.left.iter().chain(b.right.iter()))
            .fold(0.0f32, |m, v| m.max(v.abs()))
    };
    let frames = 512;
    let mut buffers: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();

    synth.render_to_mixer(&mut buffers);
    synth.render_to_mixer(&mut buffers);
    assert!(peak(&buffers) > 0.0, "音符期间应有输出");

    // seek 到音符之后：voices 清空、无新音符 → 必须静音
    synth.seek(4_000_000);
    synth.render_to_mixer(&mut buffers);
    assert_eq!(peak(&buffers), 0.0, "voices 清空后不得循环输出残留音频");
}

/// 回归：连续同 key 音符 + 短 end_sample（前一批还在 release 中就继续
/// 触发）——layer 淘汰候选必须包含 release 中的 voice，否则无候选会无限
/// 堆积，列表超过 MAX_VOICE_SLOTS 后新 voice 无 GPU 槽位（后面的音符永不
/// 发声，用户实测现象）。同时验证全局淘汰跳过墓碑。
#[test]
fn consecutive_same_key_notes_do_not_pile_up() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("YINHE_TEST_SFZ not set, skipping");
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("load");
    synth.finish_soundfont_load();
    synth.set_layer_count(Some(4));
    // 每 64 帧一个同 key 音符，128 帧后到期（release 尾巴 ~441 帧）→
    // 任意时刻同 key 在 release 中的 voice 远多于 layer 上限。
    let events: Vec<SynthEvent> = (0..600)
        .map(|i| SynthEvent::NoteOn {
            sample: i * 64,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: i * 64 + 128,
        })
        .collect();
    synth.load_events(events);
    let frames = 512usize;
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();
    for _ in 0..80 {
        synth.render_to_mixer(&mut bufs);
    }
    // 修复前：同 key voice 堆积数百（列表持续增长）
    assert!(
        synth.voice_count() <= 32,
        "连续同 key 音符不应堆积（实际 {} 个 voice）",
        synth.voice_count()
    );
}

/// layer 上限（对齐 xsynth）：同一 key 5 个递增力度音符 + layer=4 →
/// 活跃 voice 只 4 个（杀 velocity 最低的）。
#[test]
fn layer_limit_kills_quietest() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("load");
    synth.finish_soundfont_load();
    synth.set_layer_count(Some(4));
    let mut events = Vec::new();
    for i in 0..5u8 {
        events.push(SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 20 + i * 20,
            end_sample: 44_100,
        });
    }
    synth.load_events(events);
    let frames = 512;
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..1)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();
    synth.render_to_mixer(&mut bufs);
    assert_eq!(synth.voice_count(), 4, "layer=4 应限制同 key 活跃 voice 数");
}

/// 回归：内部分段渲染（外层块 4096 = 8×512 段）与小块（512，单段）
/// 输出一致，验证跨段 voice 状态（time/包络/滤波）与段间事件推进连续。
#[test]
fn segmented_render_matches_small_blocks() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("YINHE_TEST_SFZ not set, skipping");
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    let mut events = vec![
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 96_000,
        },
        SynthEvent::NoteOn {
            sample: 20_000,
            channel: 0,
            key: 64,
            velocity: 90,
            end_sample: 30_000,
        },
        // 踩/松延音踏板（跨段事件）
        SynthEvent::Control {
            sample: 10_000,
            channel: 0,
            event: ControlEvent::Raw(64, 127),
        },
        SynthEvent::Control {
            sample: 50_000,
            channel: 0,
            event: ControlEvent::Raw(64, 0),
        },
    ];
    events.sort_by_key(|e| e.sample());
    let render = |frames: usize| -> Vec<f32> {
        let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.load_events(events.clone());
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        let mut out = Vec::with_capacity(120_000 * 2);
        while out.len() < 120_000 * 2 {
            synth.render_to_mixer(&mut bufs);
            for i in 0..frames {
                out.push(bufs[0].left[i]);
                out.push(bufs[0].right[i]);
            }
        }
        out
    };
    let a = render(512);
    let b = render(4096);
    let n = a.len().min(b.len());
    let max_diff = a[..n]
        .iter()
        .zip(&b[..n])
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "应有输出");
    assert!(
        max_diff < peak * 0.01,
        "分段（4096）与小块（512）输出不一致: max_diff={max_diff} peak={peak}"
    );
}

/// 回归：密集 pitch bend（段内反复换 speed）+ 音符在段内起始时，
/// 分段（4096=8×512）与小块（512）输出一致（验证跨段时间推进连续）。
#[test]
fn segmented_render_matches_with_dense_bend() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("YINHE_TEST_SFZ not set, skipping");
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    let mut events: Vec<SynthEvent> = Vec::new();
    // 音符在段内多个位置创建（start_offset 非 0），跨多个渲染段
    for (i, start) in [100usize, 700, 1500, 2600, 3900].iter().enumerate() {
        events.push(SynthEvent::NoteOn {
            sample: *start as u64,
            channel: 0,
            key: 60 + i as u8,
            velocity: 100,
            end_sample: (*start + 3000) as u64,
        });
    }
    // 每 64 帧一次 pitch bend（段内反复换 speed，触发段边界 time 修正）
    for k in 0..180 {
        let v = ((k % 40) as f32 - 20.0) / 20.0 * 0.5;
        events.push(SynthEvent::Control {
            sample: (64 * k) as u64,
            channel: 0,
            event: ControlEvent::PitchBend(v),
        });
    }
    events.sort_by_key(|e| e.sample());

    let render = |frames: usize| -> Vec<f32> {
        let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.load_events(events.clone());
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        let mut out = Vec::with_capacity(120_000 * 2);
        while out.len() < 120_000 * 2 {
            synth.render_to_mixer(&mut bufs);
            for i in 0..frames {
                out.push(bufs[0].left[i]);
                out.push(bufs[0].right[i]);
            }
        }
        out
    };
    let a = render(512);
    let b = render(4096);
    let n = a.len().min(b.len());
    let max_diff = a[..n]
        .iter()
        .zip(&b[..n])
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "应有输出");
    assert!(
        max_diff < peak * 0.01,
        "密集 bend 下分段与小块输出不一致: max_diff={max_diff} peak={peak}"
    );
}

/// 预热（含哑渲染）不破坏后续正常渲染：加载 → finish → prewarm → 音符正常出声。
#[test]
fn prewarm_then_render_ok() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("YINHE_TEST_SFZ not set, skipping");
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("load");
    synth.finish_soundfont_load();
    // 预热（分配 + 哑渲染）：不应 panic，也不污染后续 voice 槽位
    synth.prewarm(4096);
    synth.load_events(vec![SynthEvent::NoteOn {
        sample: 0,
        channel: 0,
        key: 60,
        velocity: 100,
        end_sample: 44_100,
    }]);
    let frames = 4096;
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();
    synth.render_to_mixer(&mut bufs);
    let peak = bufs
        .iter()
        .flat_map(|b| b.left.iter().chain(b.right.iter()))
        .fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "预热后正常渲染应有输出（peak={peak}）");
}

/// 测试用 voice（sustain 阶段），只填被 chase 路径读取的字段。
fn test_voice(stage: u32) -> Voice {
    Voice {
        start_sample: 0,
        kill_pending: false,
        state: GpuVoiceState {
            env_stage: stage,
            ..Default::default()
        },
        key: 60,
        channel: 0,
        velocity: 0,
        end_sample: u64::MAX,
        orig_attack_frames: 0.0,
        orig_release_frames: 0.0,
        held_by_damper: false,
        release_pending: false,
    }
}

/// chase 应用：damper 松开把 held voice 置入 release（块外直接写状态路径）。
#[test]
fn apply_chase_damper_release_marks_held_voices() {
    let Ok(mut synth) = GpuSynth::new_default(44_100) else {
        eprintln!("no GPU, skipping");
        return;
    };
    let mut v = test_voice(4);
    v.state.envelope = 0.7;
    v.held_by_damper = true;
    synth.voices.push(v);

    // 踩下再松开延音踏板：held voice 进入 release
    synth.apply_chase(0, &[ControlEvent::Raw(64, 127), ControlEvent::Raw(64, 0)]);
    let v = &synth.voices[0];
    assert_eq!(v.state.env_stage, 5, "held voice 应进入 release");
    assert_eq!(v.state.env_start, 0.7, "release 起点 = 当前 amp");
    assert_eq!(v.state.stage_progress, 0.0);
    assert!(v.release_pending);
    assert!(!v.held_by_damper);
}

/// chase 应用：CC73 修改 attack 时长后按 shader 规则重走当前阶段。
#[test]
fn apply_chase_env_cc_rewalks_stage() {
    let Ok(mut synth) = GpuSynth::new_default(44_100) else {
        eprintln!("no GPU, skipping");
        return;
    };
    let mut v = test_voice(3);
    v.state.envelope = 0.5;
    v.state.decay_start = 0.7;
    v.state.stage_progress = 10.0;
    v.state.attack_frames = 1000.0;
    v.state.release_frames = 2000.0;
    v.orig_attack_frames = 4410.0;
    // region 原值：无 CC74 生效时重算应回到它（此前 GPU 的 None 分支
    // 保留 state 旧值、CPU 回到 orig——统一到 CPU 语义：无 CC = region
    // 原值，CC 被 121 清除时也应回退到它）
    v.orig_release_frames = 2000.0;
    synth.voices.push(v);

    synth.apply_chase(0, &[ControlEvent::Raw(0x49, 100)]);
    let v = &synth.voices[0];
    let expected = crate::channel_state::env_curve_frames(100, 4410.0, 44_100, false);
    assert!(
        (v.state.attack_frames - expected).abs() < 1e-3,
        "CC73 应重算 attack 时长: {} vs {expected}",
        v.state.attack_frames
    );
    assert_eq!(v.state.release_frames, 2000.0, "未修改的 release 保持原值");
    assert_eq!(v.state.decay_start, 0.5, "Decay 重走起点 = 当前 amp");
    assert_eq!(v.state.stage_progress, 0.0);
}

/// 临时诊断：dump 高音区采样的包络参数。
#[test]
#[ignore = "需要 YINHE_TEST_SFZ"]
fn tmp_dump_envelope_params() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        return;
    };
    let path = std::path::PathBuf::from(&sfz);
    let mut synth = GpuSynth::new_default(48_000).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("load");
    synth.finish_soundfont_load();
    for key in [60u8, 107, 108, 120, 127] {
        synth.voices.clear();
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key,
            velocity: 127,
            end_sample: 44_100,
        }]);
        let frames = 512;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..1)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        synth.render_to_mixer(&mut bufs);
        if let Some(v) = synth.voices.first() {
            eprintln!(
                "key={key}: attack={:.1} hold={:.1} decay={:.0} release={:.1} sustain={:.4} env_level={} speed={:.4} sample_len={}",
                v.state.attack_frames,
                v.state.hold_frames,
                v.state.decay_frames,
                v.state.release_frames,
                v.state.sustain_level,
                v.state.env_level,
                v.state.speed,
                v.state.sample_length,
            );
        } else {
            eprintln!("key={key}: 无 voice（选不到音色？）");
        }
    }
}

/// 临时诊断：5 批高音簇的 voice 状态 dump。
#[test]
#[ignore = "需要本地环境"]
fn tmp_multi_batch_voice_dump() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wav_path = dir.path().join("tone.wav");
    let sfz_path = dir.path().join("tone.sfz");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav");
    for i in 0..480_000u32 {
        let v = ((i as f32) * 0.05).sin() * 20_000.0;
        w.write_sample(v as i16).expect("write");
    }
    w.finalize().expect("finalize");
    std::fs::write(
            &sfz_path,
            "<region>\nsample=tone.wav\nlokey=0 hikey=127\nampeg_hold=0.6\nampeg_decay=89.88\nampeg_sustain=1.778\nampeg_release=3.5\n",
        )
        .expect("sfz");

    let sr = 48_000u32;
    let mut events = Vec::new();
    for b in 0..5u64 {
        let t0 = b * 5_294;
        for key in 60u8..65 {
            events.push(SynthEvent::NoteOn {
                sample: t0,
                channel: 0,
                key,
                velocity: 127,
                end_sample: t0 + 4_963,
            });
        }
    }
    let mut synth = GpuSynth::new_default(sr).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
        .expect("load");
    synth.finish_soundfont_load();
    synth.load_events(events);
    synth.seek(0);
    let frames = 4096; // 与生产块一致（多段渲染路径）
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = vec![yinhe_mixer::ChannelBuffers {
        left: vec![0.0; frames],
        right: vec![0.0; frames],
    }];
    for i in 0..30 {
        synth.render_to_mixer(&mut bufs);
        let pos = (i + 1) * frames;
        if [1, 2, 3, 4, 5, 6, 7, 8, 10, 15, 20, 25].contains(&i) {
            let vc: Vec<String> = synth
                .voices
                .iter()
                .map(|v| format!("k{}:s{}", v.key, v.state.env_stage))
                .collect();
            eprintln!("块{}（帧{}）: n={} {}", i + 1, pos, vc.len(), vc.join(" "));
        }
    }
}

/// 最小复现：批 0 on@0/off@4963 + 批 1 on@4096/off@9059（块 4096）。
#[test]
#[ignore = "诊断"]
fn tmp_min_repro_batches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wav_path = dir.path().join("tone.wav");
    let sfz_path = dir.path().join("tone.sfz");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav");
    for i in 0..480_000u32 {
        let v = ((i as f32) * 0.05).sin() * 20_000.0;
        w.write_sample(v as i16).expect("write");
    }
    w.finalize().expect("finalize");
    std::fs::write(
            &sfz_path,
            "<region>\nsample=tone.wav\nampeg_hold=0.6\nampeg_decay=89.88\nampeg_sustain=1.778\nampeg_release=3.5\n",
        )
        .expect("sfz");

    let sr = 48_000u32;
    let events = vec![
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 127,
            end_sample: 4_963,
        },
        SynthEvent::NoteOn {
            sample: 4_096,
            channel: 0,
            key: 61,
            velocity: 127,
            end_sample: 9_059,
        },
    ];
    // 验证 sfz 的 keyrange
    {
        let maps = crate::sf_parser::build_key_maps(&sfz_path, sr, 0).expect("build maps");
        for key in [60u8, 61] {
            let ok = crate::sf_parser::select_key_info(&maps[0].map, key, 127).is_some();
            eprintln!("  key={key} region存在={ok}");
        }
    }
    let mut synth = GpuSynth::new_default(sr).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
        .expect("load");
    synth.finish_soundfont_load();
    synth.load_events(events);
    synth.seek(0);
    let frames = 4096;
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = vec![yinhe_mixer::ChannelBuffers {
        left: vec![0.0; frames],
        right: vec![0.0; frames],
    }];
    for i in 0..4 {
        synth.render_to_mixer(&mut bufs);
        let rms = (bufs[0].left.iter().map(|x| (x * x) as f64).sum::<f64>() / frames as f64).sqrt();
        let vc: Vec<String> = synth
            .voices
            .iter()
            .enumerate()
            .map(|(idx, v)| {
                format!(
                    "[{idx}]k{}:s{}/so{}/rp{}",
                    v.key, v.state.env_stage, v.state.start_offset, v.release_pending as u8
                )
            })
            .collect();
        eprintln!("块{}: rms={rms:.5} voices={vc:?}", i + 1);
    }
}

/// 临时诊断：同 key 3 批音符的 voice 释放顺序（note_off 是否释放"最老"）。
#[test]
#[ignore = "诊断"]
fn tmp_three_batches_release_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wav_path = dir.path().join("tone.wav");
    let sfz_path = dir.path().join("tone.sfz");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav");
    for i in 0..480_000u32 {
        let v = ((i as f32) * 0.05).sin() * 20_000.0;
        w.write_sample(v as i16).expect("write");
    }
    w.finalize().expect("finalize");
    std::fs::write(
            &sfz_path,
            "<region>\nsample=tone.wav\nampeg_hold=0.6\nampeg_decay=89.88\nampeg_sustain=1.778\nampeg_release=3.5\n",
        )
        .expect("sfz");

    let sr = 48_000u32;
    // 3 批同 key：on 0 / off 4963 / on 5294 / off 10257 / on 10588 / off 15551
    let events = vec![
        SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 4_963,
        },
        SynthEvent::NoteOn {
            sample: 5_294,
            channel: 0,
            key: 60,
            velocity: 110,
            end_sample: 10_257,
        },
        SynthEvent::NoteOn {
            sample: 10_588,
            channel: 0,
            key: 60,
            velocity: 120,
            end_sample: 15_551,
        },
    ];
    let mut synth = GpuSynth::new_default(sr).expect("GpuSynth");
    synth
        .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
        .expect("load");
    synth.finish_soundfont_load();
    synth.load_events(events);
    synth.seek(0);
    let frames = 4096;
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = vec![yinhe_mixer::ChannelBuffers {
        left: vec![0.0; frames],
        right: vec![0.0; frames],
    }];
    for i in 0..6 {
        synth.render_to_mixer(&mut bufs);
        let vc: Vec<String> = synth
            .voices
            .iter()
            .enumerate()
            .map(|(idx, v)| format!("[{idx}]vel?/stage{}", v.state.env_stage))
            .collect();
        eprintln!(
            "块{}（帧{}）: n={} {}",
            i + 1,
            (i + 1) * frames,
            vc.len(),
            vc.join(" ")
        );
    }
}

/// chase_skip：只标记 seek 之后被实时处理过的控制事件（区间 [chase_base, cursor)）。
#[test]
fn chase_skip_marks_only_post_seek_controls() {
    let Ok(mut synth) = GpuSynth::new_default(44_100) else {
        eprintln!("no GPU, skipping");
        return;
    };
    synth.load_events(vec![
        SynthEvent::Control {
            sample: 100,
            channel: 0,
            event: ControlEvent::Raw(7, 100),
        },
        SynthEvent::Control {
            sample: 200,
            channel: 0,
            event: ControlEvent::PitchBend(0.5),
        },
        SynthEvent::NoteOn {
            sample: 300,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 100_000,
        },
        SynthEvent::Control {
            sample: 400,
            channel: 0,
            event: ControlEvent::Raw(64, 127),
        },
    ]);
    synth.seek(300);
    // 渲染一块推进 cursor 过 300（音符）与 400（CC64）
    let frames = 512;
    let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();
    synth.render_to_mixer(&mut bufs);

    let skip = synth.chase_skip();
    assert!(
        skip.cc_mask[0] & (1u128 << 64) != 0,
        "seek 后被实时处理的 CC64 应标记"
    );
    assert!(
        skip.cc_mask[0] & (1u128 << 7) == 0,
        "seek 前的 CC7 不应标记"
    );
    assert!(!skip.pitch_bend[0], "seek 前的 PitchBend 不应标记");
}

/// 诊断/回归：块内任意帧开始的音符必须在正确帧出声。
/// 生产块长 4096（8×512 渲染段），而多数测试用 512 块（单段）——
/// 段偏移（start_offset 的段内相对 vs 块内帧语义）只在多段块暴露。
#[test]
fn note_starts_on_time_in_later_segments() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wav_path = dir.path().join("tone.wav");
    let sfz_path = dir.path().join("tone.sfz");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav create");
    for _ in 0..44_100 {
        w.write_sample(16_000i16).expect("wav write");
    }
    w.finalize().expect("wav finalize");
    std::fs::write(&sfz_path, "<region>\nsample=tone.wav key=60\n").expect("sfz write");

    let onset_for = |note_sample: u64| -> Option<usize> {
        let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: note_sample,
            channel: 0,
            key: 60,
            velocity: 127,
            end_sample: note_sample + 44_100,
        }]);
        let frames = 2048usize;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        synth.render_to_mixer(&mut bufs);
        bufs[0].left.iter().position(|&x| x.abs() > 0.001)
    };

    let mut onsets = Vec::new();
    for note_sample in [100u64, 600, 1500, 2000] {
        let onset = onset_for(note_sample).expect("应有输出");
        onsets.push((note_sample, onset));
    }
    for (note_sample, onset) in onsets {
        let diff = onset as i64 - note_sample as i64;
        assert!(
            diff.abs() < 64,
            "块内帧 {note_sample} 的音符 onset 错位（实际 {onset}，差 {diff}）"
        );
    }
}
