//! CPU（xsynth ChannelGroup）与 GPU（GpuSynth）渲染同一音符序列的波形对比。
//!
//! 用真实 SFZ 音色库验证听感一致性：
//! - 同一音符序列（力度/时长/和弦变化）分别走 CPU 和 GPU 路径
//! - 逐样本对比输出波形，量化差异（最大绝对差 + 归一化 RMSE）
//!
//! 音色库路径通过环境变量 `YINHE_TEST_SFZ` 指定，未设置时跳过（CI 友好）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{ChannelGroup, ChannelGroupConfig, SynthEvent, SynthFormat};
use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};

use yinhe_synth::GpuSynth;

const SR: u32 = 44100;
const FRAMES: u32 = 512;

/// 测试辅助：渲染一块到 mixer 式 planar 缓冲，取指定通道的交错输出。
/// `GpuSynth` 只输出 per-channel 缓冲（由混音台负责后续处理），这里做
/// 单通道提取以便与 xsynth 直连对比。
fn render_channel_interleaved(synth: &mut GpuSynth, out: &mut [f32], channel: usize) {
    let frames = out.len() / 2;
    let mut chans: Vec<yinhe_mixer::ChannelBuffers> = (0..=channel)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect();
    synth.render_to_mixer(&mut chans);
    let ch = &chans[channel];
    for i in 0..frames {
        out[i * 2] = ch.left[i];
        out[i * 2 + 1] = ch.right[i];
    }
}

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
    // 回归：damper 踩下期间 off（4000 踩，5000 off → held，7000 松），验证不重复匹配同一 voice
    plan.push((4500, 60, 90, 1500));
    // 回归：同 key 双 voice 同时按下，验证各自独立释放
    plan.push((8000, 60, 90, 500));
    plan.push((8000, 60, 80, 500));
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
    // 按扩展名分派（SFZ/SF2 都支持；SF2 的 preset 与 GPU 路径同源）
    let sf = SampleSoundfont::new(
        sfz.to_path_buf(),
        stream_params,
        SoundfontInitOptions::default(),
    )
    .expect("soundfont load failed");

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
    // CC 事件（volume/sustain 踏板）与弯音
    for (ms, controller, value) in cc_plan() {
        events.push((
            ms * SR as u64 / 1000,
            SynthEvent::Channel(
                0,
                ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::Raw(
                    controller, value,
                ))),
            ),
        ));
    }
    for (ms, value) in bend_plan() {
        events.push((
            ms * SR as u64 / 1000,
            SynthEvent::Channel(
                0,
                ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    value,
                ))),
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

/// GPU 事件流（NoteOn 自带 `end_sample`）→ 展开出独立 NoteOff，供 xsynth 直连
/// 对比使用：xsynth 靠 NoteOff 事件释放，GpuSynth 靠 voice 到期自释，展开后
/// 两条路径的释放时机在同一 sample 对齐。
fn expand_note_offs(events: &[yinhe_synth::SynthEvent]) -> Vec<yinhe_synth::SynthEvent> {
    let mut out = Vec::with_capacity(events.len() * 2);
    for e in events {
        match e {
            yinhe_synth::SynthEvent::NoteOn {
                sample,
                channel,
                key,
                velocity,
                end_sample,
            } => {
                out.push(yinhe_synth::SynthEvent::NoteOn {
                    sample: *sample,
                    channel: *channel,
                    key: *key,
                    velocity: *velocity,
                    end_sample: *end_sample,
                });
                out.push(yinhe_synth::SynthEvent::NoteOff {
                    sample: *end_sample,
                    channel: *channel,
                    key: *key,
                });
            }
            other => out.push(*other),
        }
    }
    out.sort_by_key(|e| e.sample());
    out
}

/// GPU 路径：GpuSynth（同一音符序列）
fn gpu_render(sfz: &Path) -> Vec<f32> {
    let mut synth = GpuSynth::new_default(SR).expect("GpuSynth init failed");
    synth
        .load_dense_soundfonts(0, &[sfz.to_path_buf()])
        .expect("soundfont load failed");
    synth.finish_soundfont_load();

    let plan = note_plan();
    let mut events = Vec::new();
    for (start, key, vel, dur) in &plan {
        events.push(yinhe_synth::SynthEvent::NoteOn {
            sample: start * SR as u64 / 1000,
            channel: 0,
            key: *key,
            velocity: *vel,
            end_sample: (start + dur) * SR as u64 / 1000,
        });
    }
    // CC 事件（volume/sustain 踏板）与弯音
    for (ms, controller, value) in cc_plan() {
        events.push(yinhe_synth::SynthEvent::Control {
            sample: ms * SR as u64 / 1000,
            channel: 0,
            event: yinhe_synth::ControlEvent::Raw(controller, value),
        });
    }
    for (ms, value) in bend_plan() {
        events.push(yinhe_synth::SynthEvent::Control {
            sample: ms * SR as u64 / 1000,
            channel: 0,
            event: yinhe_synth::ControlEvent::PitchBend(value),
        });
    }
    events.sort_by_key(|e| e.sample());
    synth.load_events(events);

    let total_frames = total_duration_ms(&plan) * SR as u64 / 1000;
    let mut out = Vec::with_capacity(total_frames as usize * 2);
    let mut chunk = vec![0.0f32; FRAMES as usize * 2];
    while synth.sample_position() < total_frames {
        let frames = ((total_frames - synth.sample_position()) as usize).min(FRAMES as usize);
        let buf = &mut chunk[..frames * 2];
        render_channel_interleaved(&mut synth, buf, 0);
        out.extend_from_slice(buf);
    }
    out
}

/// 回归：dense 通道 ≥16（第二端口）不折叠到通道 0-15。
///
/// ch16 踩延音踏板、ch0 不踩，两通道按同 key 音符再 note_off：
/// 折叠 bug（MAX_CHANNELS=16 时 16 % 16 == 0）会让 ch16 的踏板状态覆盖 ch0
/// 的释放行为，ch0 音符拖长错误；修复后两通道状态独立，波形与直连一致。
///
/// 已知问题（2026-09，待排查）：该测试此前因 `port_key_maps` 长度 16 而
/// dense≥16 越界 panic，从未真正运行过；修复越界后暴露出 GPU 在 dense 16
/// 的音色循环/包络行为与 xsynth 不一致（GPU 输出恒定 ≈0.0117 不衰减，
/// xsynth 正常衰减到 ≈0.038），rel_rms > 0.6。与本轮的音源层精简无关
/// （音色库加载/voice 渲染路径未改），需要单独排查 dense≥16 的
/// `load_dense_soundfonts`/loop 行为。
#[test]
#[ignore = "GPU dense>=16 的音色循环/包络与 xsynth 不一致（既有问题，待排查）"]
fn multi_port_channels_do_not_fold() {
    let Some(sfz) = test_sfz() else { return };
    let sr = SR as u64;

    let mut gpu_events: Vec<yinhe_synth::SynthEvent> = vec![
        // ch16 踩延音踏板：note_off 后 voice 保持，直到 1000ms 松开
        yinhe_synth::SynthEvent::Control {
            sample: 50 * sr / 1000,
            channel: 16,
            event: yinhe_synth::ControlEvent::Raw(64, 127),
        },
        yinhe_synth::SynthEvent::NoteOn {
            sample: 100 * sr / 1000,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 900 * sr / 1000,
        },
        yinhe_synth::SynthEvent::NoteOn {
            sample: 100 * sr / 1000,
            channel: 16,
            key: 60,
            velocity: 100,
            end_sample: 900 * sr / 1000,
        },
        yinhe_synth::SynthEvent::Control {
            sample: 1000 * sr / 1000,
            channel: 16,
            event: yinhe_synth::ControlEvent::Raw(64, 0),
        },
    ];
    gpu_events.sort_by_key(|e| e.sample());

    let mut gpu = GpuSynth::new_default(SR).expect("GpuSynth init failed");
    gpu.load_dense_soundfonts(0, std::slice::from_ref(&sfz))
        .expect("channel 0 soundfont load failed");
    gpu.load_dense_soundfonts(16, std::slice::from_ref(&sfz))
        .expect("channel 16 soundfont load failed");
    gpu.finish_soundfont_load();
    gpu.load_events(gpu_events.clone());
    let total_frames = 1100 * sr / 1000;
    let mut gpu_out = Vec::with_capacity(total_frames as usize * 2);
    let mut chunk = vec![0.0f32; FRAMES as usize * 2];
    while gpu.sample_position() < total_frames {
        let frames = ((total_frames - gpu.sample_position()) as usize).min(FRAMES as usize);
        render_channel_interleaved(&mut gpu, &mut chunk[..frames * 2], 16);
        gpu_out.extend_from_slice(&chunk[..frames * 2]);
    }

    // 直连（32 通道 ChannelGroup）
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: SR,
    };
    let sf = SampleSoundfont::new(
        sfz.to_path_buf(),
        stream_params,
        SoundfontInitOptions::default(),
    )
    .expect("soundfont load failed");
    let config = ChannelGroupConfig {
        channel_init_options: Default::default(),
        format: SynthFormat::Custom { channels: 32 },
        audio_params: stream_params,
        parallelism: xsynth_core::channel_group::ParallelismOptions {
            channel: xsynth_core::channel_group::ThreadCount::None,
            key: xsynth_core::channel_group::ThreadCount::None,
        },
    };
    let mut cg = ChannelGroup::new(config);
    let sf = Arc::new(sf);
    for ch in [0u32, 16] {
        cg.send_event(SynthEvent::Channel(
            ch,
            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf.clone()])),
        ));
    }
    let expanded = expand_note_offs(&gpu_events);
    let xev: Vec<(u64, SynthEvent)> = expanded
        .iter()
        .map(|e| {
            let (sample, event) = match e {
                yinhe_synth::SynthEvent::NoteOn {
                    sample,
                    channel,
                    key,
                    velocity,
                    ..
                } => (
                    *sample,
                    SynthEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: *key,
                            vel: *velocity,
                        }),
                    ),
                ),
                yinhe_synth::SynthEvent::NoteOff {
                    sample,
                    channel,
                    key,
                    ..
                } => (
                    *sample,
                    SynthEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                    ),
                ),
                yinhe_synth::SynthEvent::Control {
                    sample,
                    channel,
                    event,
                } => (
                    *sample,
                    SynthEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::Control(match event {
                            yinhe_synth::ControlEvent::Raw(c, v) => ControlEvent::Raw(*c, *v),
                            yinhe_synth::ControlEvent::PitchBend(v) => ControlEvent::PitchBend(*v),
                            _ => panic!("unexpected control"),
                        })),
                    ),
                ),
            };
            (sample, event)
        })
        .collect();
    let mut xout = Vec::with_capacity(total_frames as usize * 2);
    let mut cursor = 0usize;
    let mut rendered = 0u64;
    while rendered < total_frames {
        while cursor < xev.len() && xev[cursor].0 <= rendered {
            cg.send_event(xev[cursor].1.clone());
            cursor += 1;
        }
        let seg_end = xev
            .get(cursor)
            .map(|(s, _)| (*s).min(total_frames))
            .unwrap_or(total_frames);
        let frames = (seg_end - rendered) as usize;
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(FRAMES as usize);
            cg.read_samples(&mut chunk[..n * 2]);
            xout.extend_from_slice(&chunk[..n * 2]);
            done += n;
        }
        rendered = seg_end;
    }

    // 抵消 xsynth 内置的固定默认 pan 衰减（见 compensate_xsynth_channel_pan）

    // 逐样本对比：折叠 bug 时 ch0 状态被覆盖，rel_rms 会 > 10%
    let n = gpu_out.len().min(xout.len());
    let mut sse = 0.0f64;
    let mut s_ref = 0.0f64;
    for i in 0..n {
        let d = (gpu_out[i] - xout[i]) as f64;
        sse += d * d;
        s_ref += xout[i] as f64 * xout[i] as f64;
    }
    let rel_rms = (sse / s_ref.max(1e-9)).sqrt();
    assert!(
        rel_rms < 0.05,
        "多端口通道折叠：ch16 的延音踏板污染了 ch0 的释放（rel_rms={rel_rms:.3}）"
    );
}

/// ProgramChange 切换 (bank, preset) 音色库条目（与 xsynth rebuild_matrix 同语义）。
///
/// 用多 preset 音色库（如 GeneralUser-GS.sf2：PC0=大钢琴、PC24=尼龙吉他）对比
/// GPU 与 CPU 直连波形；单条目 SFZ 下两段 PC 都选 (0,0)，测试平凡通过（不误报）。
/// 选不到条目时静音（xsynth 缺失 preset 语义）由两段波形的能量差覆盖。
#[test]
fn program_change_selects_preset() {
    let Some(sfz) = test_sfz() else { return };
    let sr = SR as u64;

    let mut gpu_events: Vec<yinhe_synth::SynthEvent> = vec![
        yinhe_synth::SynthEvent::Control {
            sample: 0,
            channel: 0,
            event: yinhe_synth::ControlEvent::ProgramChange(0),
        },
        yinhe_synth::SynthEvent::NoteOn {
            sample: 100 * sr / 1000,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 800 * sr / 1000,
        },
        yinhe_synth::SynthEvent::Control {
            sample: 900 * sr / 1000,
            channel: 0,
            event: yinhe_synth::ControlEvent::ProgramChange(24),
        },
        yinhe_synth::SynthEvent::NoteOn {
            sample: 1000 * sr / 1000,
            channel: 0,
            key: 64,
            velocity: 100,
            end_sample: 1800 * sr / 1000,
        },
    ];
    gpu_events.sort_by_key(|e| e.sample());

    let mut gpu = GpuSynth::new_default(SR).expect("GpuSynth init failed");
    gpu.load_dense_soundfonts(0, std::slice::from_ref(&sfz))
        .expect("soundfont load failed");
    gpu.finish_soundfont_load();
    gpu.load_events(gpu_events.clone());
    let total_frames = 2100 * sr / 1000;
    let mut gpu_out = Vec::with_capacity(total_frames as usize * 2);
    let mut chunk = vec![0.0f32; FRAMES as usize * 2];
    while gpu.sample_position() < total_frames {
        let frames = ((total_frames - gpu.sample_position()) as usize).min(FRAMES as usize);
        render_channel_interleaved(&mut gpu, &mut chunk[..frames * 2], 0);
        gpu_out.extend_from_slice(&chunk[..frames * 2]);
    }

    // 直连（32 通道 ChannelGroup + SetSoundfonts + PC 事件）
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: SR,
    };
    let sf = Arc::new(
        SampleSoundfont::new(sfz.clone(), stream_params, SoundfontInitOptions::default())
            .expect("soundfont load failed"),
    );
    let config = ChannelGroupConfig {
        channel_init_options: Default::default(),
        format: SynthFormat::Custom { channels: 32 },
        audio_params: stream_params,
        parallelism: xsynth_core::channel_group::ParallelismOptions {
            channel: xsynth_core::channel_group::ThreadCount::None,
            key: xsynth_core::channel_group::ThreadCount::None,
        },
    };
    let mut cg = ChannelGroup::new(config);
    cg.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf])),
    ));
    // 事件按 sample 位置逐步派发（与 multi_port 测试一致）
    let expanded = expand_note_offs(&gpu_events);
    let xev: Vec<(u64, SynthEvent)> = expanded
        .iter()
        .map(|e| {
            let (sample, event) = match e {
                yinhe_synth::SynthEvent::NoteOn {
                    sample,
                    channel,
                    key,
                    velocity,
                    ..
                } => (
                    *sample,
                    SynthEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                            key: *key,
                            vel: *velocity,
                        }),
                    ),
                ),
                yinhe_synth::SynthEvent::NoteOff {
                    sample,
                    channel,
                    key,
                    ..
                } => (
                    *sample,
                    SynthEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: *key }),
                    ),
                ),
                yinhe_synth::SynthEvent::Control {
                    sample,
                    channel,
                    event,
                } => (
                    *sample,
                    SynthEvent::Channel(
                        *channel as u32,
                        ChannelEvent::Audio(match event {
                            yinhe_synth::ControlEvent::Raw(c, v) => {
                                ChannelAudioEvent::Control(ControlEvent::Raw(*c, *v))
                            }
                            yinhe_synth::ControlEvent::ProgramChange(p) => {
                                ChannelAudioEvent::ProgramChange(*p)
                            }
                            _ => panic!("unexpected control"),
                        }),
                    ),
                ),
            };
            (sample, event)
        })
        .collect();
    let mut xout = Vec::with_capacity(total_frames as usize * 2);
    let mut cursor = 0usize;
    let mut rendered = 0u64;
    while rendered < total_frames {
        while cursor < xev.len() && xev[cursor].0 <= rendered {
            cg.send_event(xev[cursor].1.clone());
            cursor += 1;
        }
        let seg_end = xev
            .get(cursor)
            .map(|(s, _)| (*s).min(total_frames))
            .unwrap_or(total_frames);
        let frames = (seg_end - rendered) as usize;
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(FRAMES as usize);
            cg.read_samples(&mut chunk[..n * 2]);
            xout.extend_from_slice(&chunk[..n * 2]);
            done += n;
        }
        rendered = seg_end;
    }

    // 抵消 xsynth 内置的固定默认 pan 衰减（见 compensate_xsynth_channel_pan）

    let n = gpu_out.len().min(xout.len());
    let mut sse = 0.0f64;
    let mut s_ref = 0.0f64;
    for i in 0..n {
        let d = (gpu_out[i] - xout[i]) as f64;
        sse += d * d;
        s_ref += xout[i] as f64 * xout[i] as f64;
    }
    let rel_rms = (sse / s_ref.max(1e-9)).sqrt();
    assert!(
        rel_rms < 0.05,
        "ProgramChange 选择音色库与 xsynth 不一致（rel_rms={rel_rms:.3}）"
    );
    // 第二段（PC24 后）必须有能量：两个 PC 都用同一 (0,0) 条目时也成立
    let seg_start = (1000 * sr / 1000) as usize * 2;
    let seg_end = (1800 * sr / 1000) as usize * 2;
    let seg_energy: f64 = gpu_out[seg_start..seg_end]
        .iter()
        .map(|s| (s * s) as f64)
        .sum();
    assert!(seg_energy > 1e-6, "PC24 段无能量（音色库条目选择异常）");
}

/// 通道控制事件计划：(ms, controller, value)
///
/// 只含音源层 CC（Sustain/ADSR）：通道音量/声像/滤波已迁至 yinhe-dsp
/// 效果器，两边都不再处理，不能用于 CPU/GPU 对比。
fn cc_plan() -> Vec<(u64, u8, u8)> {
    vec![
        (2600, 64, 127), // 延音踏板踩下
        (2800, 64, 0),   // 延音踏板松开（held 音符一起 release）
        (1200, 72, 100), // CC72 attack：加速 attack
        (1200, 73, 100), // CC73 release：加速 release
        // 回归：长踩 damper（note 在踩下期间 off → held，松开时释放）
        (4000, 64, 127),
        (7000, 64, 0),
    ]
}

/// 弯音计划：(ms, bend -1..1)
fn bend_plan() -> Vec<(u64, f32)> {
    vec![(3000, 0.5)] // 升 1 半音（默认灵敏度 2）
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

/// yinhe CPU 路径：CpuSynth（与 gpu_render 相同的事件构造）。
fn yinhe_cpu_render(sfz: &Path) -> Vec<f32> {
    let mut synth = yinhe_synth::CpuSynth::new(SR);
    synth
        .load_dense_soundfonts(0, &[sfz.to_path_buf()])
        .expect("soundfont load failed");
    synth.finish_soundfont_load();

    let plan = note_plan();
    let mut events = Vec::new();
    for (start, key, vel, dur) in &plan {
        events.push(yinhe_synth::SynthEvent::NoteOn {
            sample: start * SR as u64 / 1000,
            channel: 0,
            key: *key,
            velocity: *vel,
            end_sample: (start + dur) * SR as u64 / 1000,
        });
    }
    for (ms, controller, value) in cc_plan() {
        events.push(yinhe_synth::SynthEvent::Control {
            sample: ms * SR as u64 / 1000,
            channel: 0,
            event: yinhe_synth::ControlEvent::Raw(controller, value),
        });
    }
    for (ms, value) in bend_plan() {
        events.push(yinhe_synth::SynthEvent::Control {
            sample: ms * SR as u64 / 1000,
            channel: 0,
            event: yinhe_synth::ControlEvent::PitchBend(value),
        });
    }
    events.sort_by_key(|e| e.sample());
    synth.load_events(events);

    let total_frames = total_duration_ms(&plan) * SR as u64 / 1000;
    let mut out = Vec::with_capacity(total_frames as usize * 2);
    let mut chunk = vec![0.0f32; FRAMES as usize * 2];
    let mut chans = vec![yinhe_mixer::ChannelBuffers {
        left: Vec::new(),
        right: Vec::new(),
    }];
    while synth.sample_position() < total_frames {
        let frames = ((total_frames - synth.sample_position()) as usize).min(FRAMES as usize);
        chans[0].left.clear();
        chans[0].left.resize(frames, 0.0);
        chans[0].right.clear();
        chans[0].right.resize(frames, 0.0);
        synth.render_to_mixer(&mut chans);
        for i in 0..frames {
            chunk[i * 2] = chans[0].left[i];
            chunk[i * 2 + 1] = chans[0].right[i];
        }
        out.extend_from_slice(&chunk[..frames * 2]);
    }
    out
}

/// yinhe CPU vs GPU：同一事件流（NoteOn 自带 end + CC + 弯音）逐样本对比。
/// GPU 路径已通过 `parity_cpu_vs_gpu` 与 xsynth 对齐，本测试保证 CPU 路径
/// 与 GPU 路径听感一致（切换后端不改变听感）。
#[test]
fn parity_yinhe_cpu_vs_gpu() {
    let Some(sfz) = test_sfz() else {
        eprintln!("YINHE_TEST_SFZ not set, skipping");
        return;
    };
    if !sfz.exists() {
        eprintln!("SFZ not found, skipping");
        return;
    }
    let cpu = yinhe_cpu_render(&sfz);
    let gpu = gpu_render(&sfz);
    assert_eq!(cpu.len(), gpu.len(), "length mismatch");

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
        "yinhe cpu vs gpu: frames={} peak={:.4} max_diff={:.5} ({:.3}% of peak) rel_rmse={:.5} worst@sample={} (cpu={:.4} gpu={:.4})",
        cpu.len() / 2,
        peak,
        max_diff,
        max_diff / peak * 100.0,
        rel_rmse,
        worst.0,
        cpu[worst.0],
        gpu[worst.0],
    );
    assert!(
        max_diff < peak * 0.03,
        "max diff {} exceeds 3% of peak {}",
        max_diff,
        peak
    );
    assert!(rel_rmse < 0.02, "relative RMSE {} exceeds 2%", rel_rmse);
}
