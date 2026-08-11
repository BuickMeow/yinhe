//! 真实 MIDI：CPU（xsynth AudioEngine 导出路径）vs GPU（GpuSynth 导出路径）对比。
//!
//! 对比指标：
//! - 听感：分 10 秒段统计 RMS / 峰值差异（两段 WAV 用各自真实导出路径渲染，
//!   两侧都用 Bit24（带限幅）保证对称）
//! - 速度：两端各自打印总耗时与 rtf（实时倍率）
//! - 第三参考：xsynth ChannelGroup 直连（事件精确 sample 位置），隔离调度层差异
//!
//! 用法：
//! ```sh
//! cargo run --release -p yinhe-audio --features gpu --example gpu_vs_cpu -- \
//!   <midi.mid> <soundfont.sfz> [限制渲染秒数]
//! ```

use std::path::Path;
use std::sync::Arc;

use yinhe_audio::export::{WavBitDepth, export_wav, export_wav_gpu};
use yinhe_mid2::parse_path;

const SR: u32 = 44100;
const SEG_SECS: u64 = 10;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: gpu_vs_cpu <midi.mid> <soundfont.sfz> [limit_secs]");
        std::process::exit(1);
    }
    let midi_path = Path::new(&args[1]);
    let sfz_path = Path::new(&args[2]);
    let limit_secs: Option<u64> = args.get(3).and_then(|s| s.parse().ok());

    let model = Arc::new(parse_path(midi_path).expect("midi parse failed"));
    println!(
        "[cmp] {}: {} tracks, ppq={}",
        midi_path.display(),
        model.tracks.len(),
        model.tempo_map.ticks_per_beat
    );
    // CC64（damper）事件计数，用于说明对比文件中的延音踏板密度
    let cc64_count: usize = model
        .tracks
        .iter()
        .flat_map(|t| t.automation_lanes.iter())
        .filter(|l| {
            matches!(
                l.target,
                yinhe_types::AutomationTarget::CC { controller: 64 }
            )
        })
        .map(|l| l.events.len())
        .sum();
    println!("[cmp] cc64 events = {cc64_count}");

    let stem = midi_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("midi");
    let tmp = std::env::temp_dir();
    let cpu_wav = tmp.join(format!("yinhe_cmp_{stem}_cpu.wav"));
    let gpu_wav = tmp.join(format!("yinhe_cmp_{stem}_gpu.wav"));
    let _ = std::fs::remove_file(&cpu_wav);
    let _ = std::fs::remove_file(&gpu_wav);

    // ── CPU 路径（xsynth AudioEngine）──
    println!("[cmp] === CPU (xsynth AudioEngine) ===");
    let t0 = std::time::Instant::now();
    export_wav(
        Arc::clone(&model),
        SR,
        &[(0, vec![sfz_path.to_string_lossy().into_owned()])],
        &[],
        &cpu_wav,
        // Bit24 走带限幅的导出路径，与实时播放/GPU 路径一致（Bit32Float 会关闭限幅）
        WavBitDepth::Bit24,
        None,
        |_, _| {},
        None,
        None,
    )
    .expect("cpu export failed");
    let cpu_elapsed = t0.elapsed();

    // ── GPU 事件构建统计（note on/off 计数与总时长）──
    let layout = yinhe_audio::spawn::channels_for_model(&model);
    let segments = &model.tempo_map.tempo_segments;
    let tpb = model.tempo_map.ticks_per_beat as f64;
    let to_sample = |tick: u32| -> u64 {
        let idx = match segments.binary_search_by_key(&tick, |s| s.start_tick) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let seg = &segments[idx];
        let t_sec = seg.start_time
            + (tick - seg.start_tick) as f64 * seg.micros_per_quarter as f64 / 1e6 / tpb;
        (t_sec * SR as f64) as u64
    };
    let mut on = 0usize;
    let mut off = 0usize;
    let mut last_off = 0u64;
    for key in 0..128usize {
        for note in model.notes[key].iter() {
            if note.velocity <= 1 {
                continue;
            }
            let track = note.track as usize;
            let ch = model.tracks[track].global_channel() as usize;
            let dense = layout.dense_for(ch);
            if dense == u32::MAX {
                continue;
            }
            on += 1;
            off += 1;
            last_off = to_sample(note.end_tick);
        }
    }
    println!(
        "[cmp] notes: on={on} off={off} 时长≈{:.1}s",
        last_off as f64 / SR as f64
    );

    // ── GPU 路径（GpuSynth）──
    println!("[cmp] === GPU (GpuSynth) ===");
    use yinhe_audio::synth::wgpu;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gpu_vs_cpu"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: 512 * 1024 * 1024,
            max_buffer_size: 512 * 1024 * 1024,
            // GPU 合成器需要 13 个 storage buffer（采样块 + 段结构 + 指令）
            max_storage_buffers_per_shader_stage: 16,
            ..wgpu::Limits::default()
        },
        memory_hints: wgpu::MemoryHints::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        trace: wgpu::Trace::Off,
    }))
    .expect("device failed");
    let t0 = std::time::Instant::now();
    export_wav_gpu(
        Arc::clone(&model),
        SR,
        sfz_path,
        &[],
        &gpu_wav,
        // Bit24 与 CPU 路径对称（两侧都带限幅 + 相同量化）
        WavBitDepth::Bit24,
        |_, _| {},
        Arc::new(device),
        Arc::new(queue),
    )
    .expect("gpu export failed");
    let gpu_elapsed = t0.elapsed();

    // ── xsynth 直连（parity 方式，事件精确 sample 位置）作第三参考 ──
    use xsynth_core::channel::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent};
    use xsynth_core::channel_group::{
        ChannelGroup, ChannelGroupConfig, SynthEvent as XSynthEvent, SynthFormat,
    };
    use xsynth_core::soundfont::{SampleSoundfont, SoundfontInitOptions};
    use xsynth_core::{AudioPipe, AudioStreamParams, ChannelCount};
    println!("[cmp] === xsynth 直连 (parity 方式) ===");
    let stream_params = AudioStreamParams {
        channels: ChannelCount::Stereo,
        sample_rate: SR,
    };
    let sf = SampleSoundfont::new_sfz(
        sfz_path.to_path_buf(),
        stream_params,
        SoundfontInitOptions::default(),
    )
    .expect("sfz load");
    // 构建事件（dense 映射 + tick_to_sample，与 GPU 一致）
    // 注意顺序：AudioEngine 的 dispatch 在同一 tick 上 CC 先于 note（cc_cursor
    // 循环在 note 循环之前），这里先 push CC 再 push note，stable sort 后同 sample 时
    // CC 在前，与 AudioEngine 一致。
    let layout2 = yinhe_audio::spawn::channels_for_model(&model);
    let cfg = ChannelGroupConfig {
        // 与 AudioEngine 配置一致：fade_out_killing=true + compacted_channels
        channel_init_options: xsynth_core::channel::ChannelInitOptions {
            fade_out_killing: true,
        },
        format: SynthFormat::Custom {
            channels: layout2.compacted_channels(),
        },
        audio_params: stream_params,
        parallelism: xsynth_core::channel_group::ParallelismOptions {
            channel: xsynth_core::channel_group::ThreadCount::None,
            key: xsynth_core::channel_group::ThreadCount::None,
        },
    };
    let mut cg = ChannelGroup::new(cfg);
    cg.send_event(XSynthEvent::AllChannels(ChannelEvent::Config(
        ChannelConfigEvent::SetLayerCount(None),
    )));
    cg.send_event(XSynthEvent::Channel(
        0,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![Arc::new(sf)])),
    ));
    let mut xev: Vec<(u64, XSynthEvent)> = Vec::new();
    // ── CC/PB/RPN 事件（同 sample 时先于 note 处理）──
    // 通道控制事件（与 CPU/GPU 路径同一语义：CC/PB/RPN 映射见 emit_automation_event）
    for t in model.tracks.iter() {
        let dense = layout2.dense_for(t.global_channel() as usize);
        if dense == u32::MAX {
            continue;
        }
        for lane in &t.automation_lanes {
            let evs: Vec<XSynthEvent> = match lane.target {
                yinhe_types::AutomationTarget::CC { controller } => lane
                    .events
                    .iter()
                    .map(|e| {
                        XSynthEvent::Channel(
                            dense,
                            ChannelEvent::Audio(ChannelAudioEvent::Control(
                                xsynth_core::channel::ControlEvent::Raw(
                                    controller,
                                    e.value.round().clamp(0.0, 127.0) as u8,
                                ),
                            )),
                        )
                    })
                    .collect(),
                yinhe_types::AutomationTarget::PitchBend => lane
                    .events
                    .iter()
                    .map(|e| {
                        XSynthEvent::Channel(
                            dense,
                            ChannelEvent::Audio(ChannelAudioEvent::Control(
                                xsynth_core::channel::ControlEvent::PitchBendValue(
                                    (e.value - 8192.0) / 8192.0,
                                ),
                            )),
                        )
                    })
                    .collect(),
                yinhe_types::AutomationTarget::Rpn { parameter } => {
                    let mut out = Vec::new();
                    for e in &lane.events {
                        match parameter {
                            0 => out.push(XSynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::Control(
                                    xsynth_core::channel::ControlEvent::PitchBendSensitivity(
                                        e.value,
                                    ),
                                )),
                            )),
                            1 => out.push(XSynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::Control(
                                    xsynth_core::channel::ControlEvent::FineTune(
                                        (e.value - 8192.0) / 8192.0 * 100.0,
                                    ),
                                )),
                            )),
                            2 => out.push(XSynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::Control(
                                    xsynth_core::channel::ControlEvent::CoarseTune(e.value - 64.0),
                                )),
                            )),
                            _ => {
                                let msb = ((parameter >> 8) & 0x7F) as u8;
                                let lsb = (parameter & 0x7F) as u8;
                                let v = e.value.round().clamp(0.0, 16383.0) as u16;
                                let dmsb = ((v >> 7) & 0x7F) as u8;
                                let dlsb = (v & 0x7F) as u8;
                                out.push(XSynthEvent::Channel(
                                    dense,
                                    ChannelEvent::Audio(ChannelAudioEvent::Control(
                                        xsynth_core::channel::ControlEvent::Raw(101, msb),
                                    )),
                                ));
                                out.push(XSynthEvent::Channel(
                                    dense,
                                    ChannelEvent::Audio(ChannelAudioEvent::Control(
                                        xsynth_core::channel::ControlEvent::Raw(100, lsb),
                                    )),
                                ));
                                out.push(XSynthEvent::Channel(
                                    dense,
                                    ChannelEvent::Audio(ChannelAudioEvent::Control(
                                        xsynth_core::channel::ControlEvent::Raw(6, dmsb),
                                    )),
                                ));
                                if dlsb != 0 {
                                    out.push(XSynthEvent::Channel(
                                        dense,
                                        ChannelEvent::Audio(ChannelAudioEvent::Control(
                                            xsynth_core::channel::ControlEvent::Raw(38, dlsb),
                                        )),
                                    ));
                                }
                            }
                        }
                    }
                    out
                }
                yinhe_types::AutomationTarget::Nrpn { parameter } => {
                    let mut out = Vec::new();
                    for e in &lane.events {
                        let msb = ((parameter >> 8) & 0x7F) as u8;
                        let lsb = (parameter & 0x7F) as u8;
                        let v = e.value.round().clamp(0.0, 16383.0) as u16;
                        let dmsb = ((v >> 7) & 0x7F) as u8;
                        let dlsb = (v & 0x7F) as u8;
                        out.push(XSynthEvent::Channel(
                            dense,
                            ChannelEvent::Audio(ChannelAudioEvent::Control(
                                xsynth_core::channel::ControlEvent::Raw(99, msb),
                            )),
                        ));
                        out.push(XSynthEvent::Channel(
                            dense,
                            ChannelEvent::Audio(ChannelAudioEvent::Control(
                                xsynth_core::channel::ControlEvent::Raw(98, lsb),
                            )),
                        ));
                        out.push(XSynthEvent::Channel(
                            dense,
                            ChannelEvent::Audio(ChannelAudioEvent::Control(
                                xsynth_core::channel::ControlEvent::Raw(6, dmsb),
                            )),
                        ));
                        if dlsb != 0 {
                            out.push(XSynthEvent::Channel(
                                dense,
                                ChannelEvent::Audio(ChannelAudioEvent::Control(
                                    xsynth_core::channel::ControlEvent::Raw(38, dlsb),
                                )),
                            ));
                        }
                    }
                    out
                }
                yinhe_types::AutomationTarget::Tempo => Vec::new(),
            };
            for (e, ev) in lane.events.iter().zip(evs) {
                xev.push((to_sample(e.tick), ev));
            }
        }
    }

    // ── note 事件（放在 CC 之后，同 sample 时 CC 先处理，与 AudioEngine dispatch 一致）──
    for key in 0..128usize {
        for note in model.notes[key].iter() {
            if note.velocity <= 1 {
                continue;
            }
            let track = note.track as usize;
            let ch = model.tracks[track].global_channel() as usize;
            let dense = layout2.dense_for(ch);
            if dense == u32::MAX {
                continue;
            }
            let s = to_sample(note.start_tick);
            let e = to_sample(note.end_tick);
            xev.push((
                s,
                XSynthEvent::Channel(
                    dense,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOn {
                        key: key as u8,
                        vel: note.velocity,
                    }),
                ),
            ));
            xev.push((
                e,
                XSynthEvent::Channel(
                    dense,
                    ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: key as u8 }),
                ),
            ));
        }
    }
    xev.sort_by_key(|(s, _)| *s);
    let xdur = xev.last().map(|(s, _)| *s).unwrap_or(0) + SR as u64;
    let t0 = std::time::Instant::now();
    let mut xout: Vec<f32> = Vec::with_capacity(xdur as usize * 2);
    let mut chunk = vec![0.0f32; 512 * 2];
    // 直连渲染也过限幅（与 CPU/GPU 路径对称：低峰值时等价 ÷2）。
    // 注意粒度：CPU 导出路径的 limiter 按 1024 帧块调用（export_wav 的
    // RENDER_CHUNK_FRAMES），这里必须同样按 1024 帧 limit——按事件段 limit
    // 会让 loudness 状态轨迹不同，低峰值段输出出现 2 倍关系以外的偏差。
    let mut limiter = yinhe_synth::limiter::VolumeLimiter::new(2);
    let mut acc: Vec<f32> = Vec::with_capacity(1024 * 2);
    let mut cursor = 0usize;
    let mut rendered = 0u64;
    while rendered < xdur {
        while cursor < xev.len() && xev[cursor].0 <= rendered {
            cg.send_event(xev[cursor].1.clone());
            cursor += 1;
        }
        let seg_end = xev.get(cursor).map(|(s, _)| (*s).min(xdur)).unwrap_or(xdur);
        let frames = (seg_end - rendered) as usize;
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(512);
            let buf = &mut chunk[..n * 2];
            cg.read_samples(buf);
            acc.extend_from_slice(buf);
            if acc.len() == 1024 * 2 {
                limiter.limit(&mut acc);
                xout.extend_from_slice(&acc);
                acc.clear();
            }
            done += n;
        }
        rendered = seg_end;
    }
    // 尾部不足一块的部分同样 limit（与 CPU 导出最后一块一致）
    if !acc.is_empty() {
        limiter.limit(&mut acc);
        xout.extend_from_slice(&acc);
    }
    println!("[cmp] xsynth 直连: {:.2?}", t0.elapsed());
    let xrms = (xout.iter().map(|v| (v * v) as f64).sum::<f64>() / xout.len().max(1) as f64).sqrt();
    let tail_rms = (xout
        .iter()
        .rev()
        .take(512 * 2)
        .map(|v| (v * v) as f64)
        .sum::<f64>()
        / 1024.0)
        .sqrt();
    println!(
        "[cmp] xout frames={} rms={:.5} tail_rms={:.5} xev={} xdur={}",
        xout.len() / 2,
        xrms,
        tail_rms,
        xev.len(),
        xdur
    );

    // ── 读回对比 ──
    let mut cpu_reader = hound::WavReader::open(&cpu_wav).expect("open cpu wav");
    let mut gpu_reader = hound::WavReader::open(&gpu_wav).expect("open gpu wav");
    let read = |r: &mut hound::WavReader<std::io::BufReader<std::fs::File>>| -> Vec<f32> {
        match r.spec().bits_per_sample {
            32 => r.samples::<f32>().map(|s| s.unwrap()).collect(),
            24 => r
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / 8_388_607.0)
                .collect(),
            16 => r
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / 32768.0)
                .collect(),
            b => panic!("unsupported bits {b}"),
        }
    };
    let cpu: Vec<f32> = read(&mut cpu_reader);
    let gpu: Vec<f32> = read(&mut gpu_reader);

    // xsynth 直连 vs GPU / vs CPU(AudioEngine)，用每通道全局能量比较
    let xn = xout.len().min(gpu.len());
    let mut sx = 0.0f64;
    let mut sg = 0.0f64;
    for i in 0..xn {
        let d = (xout[i] - gpu[i]) as f64;
        sx += d * d;
        sg += gpu[i] as f64 * gpu[i] as f64;
    }
    println!(
        "[cmp] xsynth直连 vs GPU rel_rms={:.4}",
        (sx / sg.max(1e-9)).sqrt()
    );
    let xn2 = xout.len().min(cpu.len());
    let mut sx2 = 0.0f64;
    let mut sc2 = 0.0f64;
    for i in 0..xn2 {
        let d = (xout[i] - cpu[i]) as f64;
        sx2 += d * d;
        sc2 += cpu[i] as f64 * cpu[i] as f64;
    }
    println!(
        "[cmp] xsynth直连 vs CPU(AudioEngine) rel_rms={:.4}",
        (sx2 / sc2.max(1e-9)).sqrt()
    );
    let cpu_frames = cpu.len() / 2;
    let gpu_frames = gpu.len() / 2;
    println!(
        "[cmp] cpu frames={} ({:.1}s) gpu frames={} ({:.1}s)",
        cpu_frames,
        cpu_frames as f64 / SR as f64,
        gpu_frames,
        gpu_frames as f64 / SR as f64
    );

    // 直连分 10s 段对比 CPU/GPU
    let xf = xout.len() / 2;
    let segs = (cpu_frames.min(gpu_frames).min(xf) as u64 / (SEG_SECS * SR as u64)).max(1);
    for seg in 0..segs {
        let start = (seg * SEG_SECS * SR as u64) as usize * 2;
        let end = (((seg + 1) * SEG_SECS * SR as u64) as usize * 2)
            .min(xout.len())
            .min(cpu.len());
        let mut sxc = 0.0f64;
        let mut sc = 0.0f64;
        let mut sxg = 0.0f64;
        let mut sg = 0.0f64;
        let mut sxc_r = 0.0f64;
        let mut sc_r = 0.0f64;
        let mut sxg_r = 0.0f64;
        let mut sg_r = 0.0f64;
        for i in (start..end).step_by(2) {
            let dxc = (xout[i] - cpu[i]) as f64;
            sxc += dxc * dxc;
            sc += cpu[i] as f64 * cpu[i] as f64;
            let dxg = (xout[i] - gpu[i]) as f64;
            sxg += dxg * dxg;
            sg += gpu[i] as f64 * gpu[i] as f64;
            let dxc_r = (xout[i + 1] - cpu[i + 1]) as f64;
            sxc_r += dxc_r * dxc_r;
            sc_r += cpu[i + 1] as f64 * cpu[i + 1] as f64;
            let dxg_r = (xout[i + 1] - gpu[i + 1]) as f64;
            sxg_r += dxg_r * dxg_r;
            sg_r += gpu[i + 1] as f64 * gpu[i + 1] as f64;
        }
        println!(
            "[cmp] xseg {:>3}s: L x-vs-cpu={:.3} x-vs-gpu={:.3} | R x-vs-cpu={:.3} x-vs-gpu={:.3}",
            seg * SEG_SECS,
            (sxc / sc.max(1e-9)).sqrt(),
            (sxg / sg.max(1e-9)).sqrt(),
            (sxc_r / sc_r.max(1e-9)).sqrt(),
            (sxg_r / sg_r.max(1e-9)).sqrt()
        );
    }

    let cmp_frames = match limit_secs {
        Some(s) => (s * SR as u64) as usize,
        None => cpu_frames.min(gpu_frames),
    };
    let cmp_frames = cmp_frames.min(cpu_frames).min(gpu_frames);
    println!("[cmp] 对比长度 {}s", cmp_frames as f64 / SR as f64);

    // 全局峰值归一化（避免限幅后整体偏小导致百分比虚高）
    let peak = cpu[..cmp_frames * 2]
        .iter()
        .chain(&gpu[..cmp_frames * 2])
        .fold(0.0f32, |m, &s| m.max(s.abs()))
        .max(1e-6);

    println!("[cmp] === 分段差异（每段 {SEG_SECS}s）peak={peak:.4} ===");
    let mut seg = 0u64;
    let mut max_seg_diff = 0.0f32;
    let mut max_seg_rms = 0.0f32;
    while ((seg * SEG_SECS * SR as u64) as usize) < cmp_frames {
        let start = (seg * SEG_SECS * SR as u64) as usize;
        let end = ((seg + 1) * SEG_SECS * SR as u64).min(cmp_frames as u64) as usize;
        let mut sum_sq = 0.0f64;
        let mut sum_sq_gpu = 0.0f64;
        let mut seg_peak_diff = 0.0f32;
        let mut seg_peak = 0.0f32;
        for i in (start * 2)..(end * 2) {
            let d = (cpu[i] - gpu[i]).abs();
            if d > seg_peak_diff {
                seg_peak_diff = d;
            }
            if cpu[i].abs() > seg_peak {
                seg_peak = cpu[i].abs();
            }
            sum_sq += (cpu[i] - gpu[i]) as f64 * (cpu[i] - gpu[i]) as f64;
            sum_sq_gpu += gpu[i] as f64 * gpu[i] as f64;
        }
        let n = (end - start) as f64 * 2.0;
        let rms = (sum_sq / n).sqrt();
        let rms_gpu = (sum_sq_gpu / n).sqrt();
        let rel_rms = rms / rms_gpu.max(1e-9);
        let pct_peak = seg_peak_diff / peak.max(1e-6) * 100.0;
        max_seg_diff = max_seg_diff.max(seg_peak_diff);
        max_seg_rms = max_seg_rms.max(rel_rms as f32);
        println!(
            "seg {:>3}s: rms_diff={:.5} rel_rms={:.4} peak_diff={:.5} ({:.2}%) cpu_peak={:.4}",
            seg * SEG_SECS,
            rms,
            rel_rms,
            seg_peak_diff,
            pct_peak,
            seg_peak
        );
        seg += 1;
    }

    let audio_secs = cmp_frames as f64 / SR as f64;
    println!(
        "[cmp] === 速度：cpu rtf={:.1}x ({:.2?}) gpu rtf={:.1}x ({:.2?}) ===",
        audio_secs / cpu_elapsed.as_secs_f64(),
        cpu_elapsed,
        audio_secs / gpu_elapsed.as_secs_f64(),
        gpu_elapsed
    );
    println!(
        "[cmp] 最差段：peak_diff={:.5} ({:.2}%) rel_rms={:.4}",
        max_seg_diff,
        max_seg_diff / peak.max(1e-6) * 100.0,
        max_seg_rms
    );

    // ── 互相关（降采样 8 倍，[15s,19s) 窗口，±0.2s 搜索）判断时间偏移/内容关系 ──
    let ds = 8usize;
    let win_start = SR as usize * 15 / ds;
    let win_end = cmp_frames.min(SR as usize * 19) / ds;
    let n2 = win_end.saturating_sub(win_start);
    let search = SR as usize / 5 / ds;
    let x: Vec<f64> = (0..n2)
        .map(|i| cpu[(win_start + i) * ds * 2] as f64)
        .collect();
    let y: Vec<f64> = (0..n2)
        .map(|i| gpu[(win_start + i) * ds * 2] as f64)
        .collect();
    let xm: f64 = x.iter().sum::<f64>() / n2 as f64;
    let ym: f64 = y.iter().sum::<f64>() / n2 as f64;
    let mut best = (0i64, 0.0f64);
    for lag in -(search as i64)..(search as i64) {
        let mut s = 0.0f64;
        if lag < 0 {
            for i in (-lag as usize)..n2 {
                s += (x[i] - xm) * (y[(i as i64 + lag) as usize] - ym);
            }
        } else {
            for i in 0..(n2 - lag as usize) {
                s += (x[i] - xm) * (y[i + lag as usize] - ym);
            }
        }
        if s.abs() > best.1.abs() {
            best = (lag, s);
        }
    }
    let nx = (x.iter().map(|v| (v - xm).powi(2)).sum::<f64>()).sqrt();
    let ny = (y.iter().map(|v| (v - ym).powi(2)).sum::<f64>()).sqrt();
    let lag = best.0;
    let lag_frames = lag * ds as i64;
    println!(
        "[cmp] 互相关(前2s): lag={} frames ({:.1}ms) corr={:.4}",
        lag_frames,
        lag_frames as f64 / SR as f64 * 1000.0,
        best.1 / (nx * ny)
    );
    // 对齐后的振幅比（同相位窗口）
    let (mut c0, mut d0): (Vec<f64>, Vec<f64>) = if lag_frames >= 0 {
        (
            x[lag_frames as usize..].to_vec(),
            y[..n2 - lag_frames as usize].to_vec(),
        )
    } else {
        (
            x[..(n2 as i64 + lag_frames) as usize].to_vec(),
            y[(-lag_frames) as usize..].to_vec(),
        )
    };
    let m = c0.len().min(d0.len());
    c0.truncate(m);
    d0.truncate(m);
    let k = c0.iter().zip(&d0).map(|(c, d)| c * d).sum::<f64>()
        / (c0.iter().map(|c| c * c).sum::<f64>() + 1e-30);
    let resid = (d0
        .iter()
        .zip(&c0)
        .map(|(d, c)| (d - k * c).powi(2))
        .sum::<f64>()
        / m as f64)
        .sqrt();
    let c_rms = (c0.iter().map(|c| c * c).sum::<f64>() / m as f64).sqrt();
    println!(
        "[cmp] 对齐后: gpu = {k:.4}×cpu，残差RMS={resid:.5}（=cpu RMS 的 {:.1}%）",
        resid / c_rms.max(1e-9) * 100.0
    );
}
