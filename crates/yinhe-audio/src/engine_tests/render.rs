use super::*;

/// 音源层 CC（如 Sustain=CC64）不进通道处理段（由合成器消费）。
#[test]
fn source_level_cc_not_routed_to_channel_dsp() {
    let sample_rate = 44100u32;
    let mut doc = Document::empty();
    {
        let model = Arc::make_mut(&mut doc.data.model);
        let track = Arc::make_mut(&mut model.tracks[1]);
        track.automation_lanes.push(AutomationLane {
            target: AutomationTarget::CC { controller: 64 },
            track: 1,
            events: vec![AutomationEvent {
                id: 0,
                tick: 0,
                value: 1.0,
                shape: SegmentShape::Step,
            }],
        });
    }
    doc.data.bump_revision();

    let mut engine = spawn_engine_for_doc(&doc, sample_rate);
    engine.playback.playing = true;
    engine.dispatch_and_find_next(0, 60_000);

    // CC64 走常规路径（xsynth 处理），不被通道处理段接管（处理段只管 DSP_CHANNEL_CCS）。
    let skip = engine.chase_skip();
    assert!(
        skip.cc_mask[0] & (1u128 << 64) != 0,
        "CC64 应走常规路径并打点（xsynth 消费）"
    );
}

/// yinhe CPU 后端（CpuSynth）经完整引擎链路的冒烟：加载音色库 → Play →
/// dispatch 投递事件 → 渲染出非零输出。
#[test]
#[ignore = "需要本地 SoundFont"]
fn yinhe_cpu_engine_render_smoke() {
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let model = make_model_with_notes(vec![(60, 0, 4_800, 100, 0)]);
    let layout = crate::spawn::channels_for_model(&model);
    let mut engine = AudioEngine::new(sr, layout);
    engine.handle_command(AudioCommand::LoadModel {
        model: std::sync::Arc::new(model),
    });

    let mut synth = yinhe_synth::CpuSynth::new(sr);
    synth
        .load_dense_soundfonts(0, &[std::path::PathBuf::from(sfz)])
        .expect("音色库加载");
    engine.synth.cpu_synth = synth;

    engine.handle_command(AudioCommand::Play { from_sample: 0 });
    let mut out = vec![0.0f32; 512 * 2];
    let mut peak = 0.0f32;
    for _ in 0..10 {
        engine.render(&mut out);
        peak = peak.max(out.iter().fold(0.0f32, |m, v| m.max(v.abs())));
    }
    assert!(peak > 0.0, "yinhe CPU 后端应渲染出输出（peak={peak}）");
}

/// 画像（本地 MIDI + SoundFont，ignored）：经典黑乐谱成本分解。
///
/// - `tau2.5.9.mid`：628 万音符，经典布局（音符为主）
/// - `cyber-night.mid`：CC 密集（控制器多的黑乐谱）
///
/// 每首曲子：三后端 baseline + CpuSynth 降级分解（全功能/无滤波/无采样/
/// 仅包络）——差值即各阶段成本占比，用于决定块级重写的收益点。
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn prof_blackmidi_cost_breakdown() {
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    let sfz = std::env::var("YINHE_PROF_SF").unwrap_or_else(|_| {
        "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz".into()
    });
    let sfz = sfz.as_str();
    let sr = 48_000u32;
    // 渲染秒数可配（`YINHE_PROF_SECS`，默认 3）：峰值段实时倍率低时
    // 用 1 秒快速迭代（AGENTS 五-7 单项测试 ≤120s）。
    let prof_secs: f64 = std::env::var("YINHE_PROF_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);

    let midis: Vec<String> = std::env::var("YINHE_PROF_MIDIS")
        .map(|s| {
            // 单个路径里可能有逗号（如 "44,444,444 Notes"），优先整体判定；
            // 多文件用 ';' 分隔。
            if std::path::Path::new(&s).exists() {
                vec![s]
            } else {
                s.split(';').map(str::to_string).collect()
            }
        })
        .unwrap_or_else(|_| {
            vec![
                "/Users/jieneng/Music/MIDIs/tau2.5.9.mid".into(),
                "/Users/jieneng/Music/MIDIs/cyber-night.mid".into(),
            ]
        });
    for midi in &midis {
        let t_parse = Instant::now();
        let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse"));
        let total: usize = (0..256).map(|k| model.notes[k].len()).sum();
        let ppq = model.meta.ppq;
        // 固定起点（`YINHE_PROF_START_SECS`）：两个版本的同一段对比时跳过
        // 峰值拍统计（否则 4400 万音符的直方图要扫一遍）。
        let fixed_start: Option<f64> = std::env::var("YINHE_PROF_START_SECS")
            .ok()
            .and_then(|s| s.parse().ok());
        let layout = if let Some(n) = std::env::var("YINHE_PROF_CHANNELS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
        {
            crate::channel_layout::ChannelLayout::from_mask(vec![true; n.min(256)])
        } else {
            crate::spawn::channels_for_model(&model)
        };
        // 多 port 工程：源通道最多 256 个，dense 索引与源通道不是一回事。
        let active: Vec<u8> = (0..256u16)
            .filter(|c| layout.is_active(*c as usize))
            .map(|c| c as u8)
            .collect();
        let denses: Vec<u32> = (0..layout.num_channels()).collect();

        let start_sample = if let Some(secs) = fixed_start {
            let start_sample = (secs * sr as f64) as u64;
            eprintln!(
                "\n=== {}\n  parse={:.1}s notes={total}（模型≈{:.0}MB） ppq={ppq} 固定起点={secs}s",
                midi.rsplit('/').next().unwrap_or(midi),
                t_parse.elapsed().as_secs_f64(),
                total as f64 * std::mem::size_of::<yinhe_types::Note>() as f64 / (1024.0 * 1024.0),
            );
            let probe = {
                let mut p = AudioEngine::new(sr, layout.clone());
                p.handle_command(AudioCommand::LoadModel {
                    model: Arc::clone(&model),
                });
                p
            };
            let start_tick = probe.sample_to_tick(start_sample);
            let end_tick = probe.sample_to_tick(start_sample + (prof_secs * sr as f64) as u64);
            let mut count = 0usize;
            let mut audible = 0usize;
            for key in 0..256 {
                for n in model.notes[key].range(start_tick, end_tick) {
                    count += 1;
                    if crate::prepare_model::audible_end(&model, n, 1).is_some() {
                        audible += 1;
                    }
                }
            }
            eprintln!(
                "  渲染窗口=[{start_tick},{end_tick}) 音符数={count} 可听={audible} 源通道={} dense={} 轨道={}",
                active.len(),
                layout.num_channels(),
                model.tracks.len()
            );
            start_sample
        } else {
            // 密度峰值（每拍音符数）
            let mut per_beat: std::collections::BTreeMap<u32, usize> = Default::default();
            for k in 0..256 {
                for n in model.notes[k].iter() {
                    *per_beat.entry(n.start_tick / ppq).or_default() += 1;
                }
            }
            let (peak_beat, peak_count) = per_beat
                .iter()
                .max_by_key(|(_, v)| **v)
                .map(|(b, c)| (*b, *c))
                .expect("non-empty");
            eprintln!(
                "\n=== {}\n  parse={:.1}s notes={total}（模型≈{:.0}MB） ppq={ppq} 峰值拍={peak_beat}（约第 {} 小节）count={peak_count}",
                midi.rsplit('/').next().unwrap_or(midi),
                t_parse.elapsed().as_secs_f64(),
                total as f64 * std::mem::size_of::<yinhe_types::Note>() as f64 / (1024.0 * 1024.0),
                peak_beat / 4 + 1
            );
            let peak_tick = peak_beat.saturating_mul(ppq).saturating_sub(ppq * 2);
            {
                // 峰值拍的门长：黑乐谱大量极短音符，决定"同时发声数"的上限。
                let lo = peak_beat * ppq;
                let hi = lo + ppq;
                let (mut n, mut sum) = (0u64, 0u64);
                for k in 0..256 {
                    for note in model.notes[k].iter() {
                        if note.start_tick >= lo && note.start_tick < hi {
                            n += 1;
                            sum += note.end_tick.saturating_sub(note.start_tick) as u64;
                        }
                    }
                }
                eprintln!(
                    "  峰值拍门长：note_on={n} 平均 gate={:.1} tick",
                    sum as f64 / n.max(1) as f64
                );
            }
            // tick→sample 必须用**加载模型后**的引擎（tempo map 来自模型；
            // 空引擎会退化成默认 120BPM，seek 到错误位置）。
            let start_sample = {
                let mut probe = AudioEngine::new(sr, layout.clone());
                probe.handle_command(AudioCommand::LoadModel {
                    model: Arc::clone(&model),
                });
                probe.tick_to_sample(peak_tick)
            };
            {
                let beat_secs = model
                    .tempo_map
                    .tick_to_seconds((peak_beat + 1) as u64 * ppq as u64)
                    - model
                        .tempo_map
                        .tick_to_seconds(peak_beat as u64 * ppq as u64);
                let probe = {
                    let mut p = AudioEngine::new(sr, layout.clone());
                    p.handle_command(AudioCommand::LoadModel {
                        model: Arc::clone(&model),
                    });
                    p
                };
                let start_tick = probe.sample_to_tick(start_sample);
                let end_tick = probe.sample_to_tick(start_sample + (prof_secs * sr as f64) as u64);
                let mut count = 0usize;
                let mut audible = 0usize;
                for key in 0..256 {
                    for n in model.notes[key].range(start_tick, end_tick) {
                        count += 1;
                        if crate::prepare_model::audible_end(&model, n, 1).is_some() {
                            audible += 1;
                        }
                    }
                }
                eprintln!(
                    "  峰值拍时长={beat_secs:.3}s bpm≈{:.0} 起点={:.2}s 渲染窗口=[{start_tick},{end_tick}) 音符数={count} 可听={audible}",
                    60.0 / beat_secs.max(1e-9),
                    start_sample as f64 / sr as f64
                );
            }
            start_sample
        };

        // 渲染 prof_secs 秒（调用方负责 Play/重置）
        let render_secs = |engine: &mut AudioEngine, secs: f64| -> (f64, u64, f64, f32) {
            let frames = 512usize;
            let mut buf = vec![0.0f32; frames * 2];
            let target: u64 = (secs * sr as f64) as u64;
            let mut rendered = 0u64;
            let mut peak_voice = 0u64;
            let mut peak_sample = 0.0f32;
            let mut max_chunk = 0.0f64;
            let t0 = Instant::now();
            while rendered < target {
                let tc = Instant::now();
                engine.render(&mut buf);
                let dt = tc.elapsed().as_secs_f64() * 1000.0;
                rendered += frames as u64;
                peak_voice = peak_voice.max(engine.voice_count());
                peak_sample = buf.iter().fold(peak_sample, |m, s| m.max(s.abs()));
                if rendered > sr as u64 {
                    max_chunk = max_chunk.max(dt);
                }
            }
            (
                t0.elapsed().as_secs_f64(),
                peak_voice,
                max_chunk,
                peak_sample,
            )
        };

        // ── baseline：两个 CPU 后端 ──
        for (name, backend) in [("xsynth-cpu", 0u8), ("yinhe-cpu", 1)] {
            let mut engine = AudioEngine::new(sr, layout.clone());
            let t_load = Instant::now();
            engine.handle_command(AudioCommand::LoadModel {
                model: Arc::clone(&model),
            });
            let load_ms = t_load.elapsed().as_secs_f64() * 1000.0;
            match backend {
                0 => {
                    let configs: Vec<(u8, Vec<String>)> =
                        active.iter().map(|c| (*c, vec![sfz.to_string()])).collect();
                    engine.handle_command(AudioCommand::SetSoundFonts {
                        configs: Box::new(configs),
                    });
                }
                1 => {
                    let mut cs = yinhe_synth::CpuSynth::new(sr);
                    cs.load_dense_soundfonts_many(&denses, &[std::path::PathBuf::from(sfz)])
                        .expect("cpu sf");
                    engine.synth.cpu_synth = cs;
                }
                _ => {}
            }
            engine.handle_command(AudioCommand::Play {
                from_sample: start_sample,
            });
            let _ = backend;
            let (el, pv, mc, ps) = render_secs(&mut engine, prof_secs);
            eprintln!(
                "  {name:<12} {prof_secs}s音频={el:>6.2}s ({:.2}x) peak_voice={pv:>6} max_chunk={mc:>7.1}ms peak_sample={ps:.3} LoadModel={load_ms:.1}ms",
                prof_secs / el.max(1e-9)
            );
        }

        // ── CpuSynth 降级分解（同一引擎，模式间 Play 重置）──
        // 超大工程可跳过（`YINHE_PROF_SKIP_BREAKDOWN=1`）：解析与渲染本身就够慢。
        if std::env::var("YINHE_PROF_SKIP_BREAKDOWN").is_ok_and(|v| v == "1") {
            continue;
        }
        let mut cs_engine = AudioEngine::new(sr, layout.clone());
        cs_engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        let mut cs = yinhe_synth::CpuSynth::new(sr);
        cs.load_dense_soundfonts_many(&denses, &[std::path::PathBuf::from(sfz)])
            .expect("cpu sf");
        cs_engine.synth.cpu_synth = cs;
        for (name, mode) in [("全功能", 0u8), ("无滤波", 1), ("无采样", 2), ("仅包络", 3)]
        {
            yinhe_synth::cpu_synth::CPU_PROFILE_MODE.store(mode, Ordering::Relaxed);
            use std::sync::atomic::AtomicU64;
            let base = |a: &AtomicU64| a.load(Ordering::Relaxed);
            let b_on = base(&yinhe_synth::cpu_synth::PROF_NOTE_ON_NS);
            let b_off = base(&yinhe_synth::cpu_synth::PROF_NOTE_OFF_NS);
            let b_render = base(&yinhe_synth::cpu_synth::PROF_RENDER_NS);
            let b_par = base(&yinhe_synth::cpu_synth::PROF_PAR_NS);
            let b_red = base(&yinhe_synth::cpu_synth::PROF_REDUCE_NS);
            let b_end = base(&yinhe_synth::cpu_synth::PROF_BLOCK_END_NS);
            let b_adv = base(&yinhe_synth::cpu_synth::PROF_ADVANCE_NS);
            let b_ret = base(&yinhe_synth::cpu_synth::PROF_RETAIN_NS);
            let b_reb = base(&yinhe_synth::cpu_synth::PROF_REBUILD_NS);
            cs_engine.handle_command(AudioCommand::Play {
                from_sample: start_sample,
            });
            let (el, pv, mc, _ps) = render_secs(&mut cs_engine, prof_secs);
            let d_on = base(&yinhe_synth::cpu_synth::PROF_NOTE_ON_NS) - b_on;
            let d_off = base(&yinhe_synth::cpu_synth::PROF_NOTE_OFF_NS) - b_off;
            let d_render = base(&yinhe_synth::cpu_synth::PROF_RENDER_NS) - b_render;
            let d_par = base(&yinhe_synth::cpu_synth::PROF_PAR_NS) - b_par;
            let d_red = base(&yinhe_synth::cpu_synth::PROF_REDUCE_NS) - b_red;
            let d_end = base(&yinhe_synth::cpu_synth::PROF_BLOCK_END_NS) - b_end;
            let d_adv = base(&yinhe_synth::cpu_synth::PROF_ADVANCE_NS) - b_adv;
            let d_ret = base(&yinhe_synth::cpu_synth::PROF_RETAIN_NS) - b_ret;
            let d_reb = base(&yinhe_synth::cpu_synth::PROF_REBUILD_NS) - b_reb;
            let total_ns = (el * 1e9) as u64;
            let r = d_render.max(1) as f64;
            eprintln!(
                "  CpuSynth-{name:<6} {prof_secs}s音频={el:>6.2}s ({:.2}x) peak_voice={pv:>6} max_chunk={mc:>7.1}ms | note_on={:.0}% note_off={:.0}% render={:.0}%",
                prof_secs / el.max(1e-9),
                100.0 * d_on as f64 / total_ns as f64,
                100.0 * d_off as f64 / total_ns as f64,
                100.0 * d_render as f64 / total_ns as f64,
            );
            eprintln!(
                "    └ render 细分: par={:.0}% reduce={:.0}% block_end={:.0}%（占总渲染）| block_end 内: advance={:.0}% retain={:.0}% rebuild={:.0}%",
                100.0 * d_par as f64 / r,
                100.0 * d_red as f64 / r,
                100.0 * d_end as f64 / r,
                100.0 * d_adv as f64 / d_end.max(1) as f64,
                100.0 * d_ret as f64 / d_end.max(1) as f64,
                100.0 * d_reb as f64 / d_end.max(1) as f64,
            );
        }
        yinhe_synth::cpu_synth::CPU_PROFILE_MODE.store(0, Ordering::Relaxed);
    }
}

/// 分析（本地 MIDI，ignored）：黑乐谱重复音符分布 —— 评估"同参数音符合批"
/// 的收益上限。
///
/// 统计峰值拍 ±8 拍窗口内的三档可合并比例：
/// - 完全重复（key/vel/start/end 全同）：可无损合批（note_off refcount）；
/// - 同起音（key/vel/start 同、长度可能不同）：可共享采样读取；
/// - 同参数（key/vel 同、起音不同）：仅缓存友好（相位不同不可合并）。
#[test]
#[ignore = "需要本地 MIDI"]
fn analyze_blackmidi_duplicates() {
    use std::collections::HashMap;

    let midis = [
        "/Users/jieneng/Music/MIDIs/tau2.5.9.mid",
        "/Users/jieneng/Music/MIDIs/cyber-night.mid",
        "/Users/jieneng/Music/MIDIs/5K 5,555,555 notes by The Atom Bomb.mid",
    ];
    for midi in midis {
        let Ok(model) = yinhe_midi::parse_path(midi) else {
            eprintln!("跳过（解析失败）：{midi}");
            continue;
        };
        let ppq = model.meta.ppq;
        // 峰值拍（每拍音符数）
        let mut per_beat: HashMap<u32, usize> = HashMap::new();
        for k in 0..128 {
            for n in model.notes[k].iter() {
                *per_beat.entry(n.start_tick / ppq.max(1)).or_default() += 1;
            }
        }
        let Some((&peak_beat, &peak_count)) = per_beat.iter().max_by_key(|(_, v)| **v) else {
            continue;
        };
        // 窗口：峰值拍 ±8 拍
        let win_start = peak_beat.saturating_sub(8).saturating_mul(ppq);
        let win_end = (peak_beat + 8).saturating_mul(ppq);

        let mut identical: HashMap<(u8, u8, u32, u32), u32> = HashMap::new();
        let mut identical_ch: HashMap<(u8, u8, u8, u32, u32), u32> = HashMap::new();
        let mut identical_peak: HashMap<(u8, u8, u8, u32, u32), u32> = HashMap::new();
        let track_channels: Vec<u8> = model.tracks.iter().map(|t| t.global_channel()).collect();
        let peak_start = peak_beat.saturating_mul(ppq);
        let peak_end = peak_start.saturating_add(ppq);
        let mut total = 0usize;
        let mut total_peak = 0usize;
        for k in 0..128u8 {
            for n in model.notes[k as usize].iter() {
                if n.start_tick < win_start || n.start_tick >= win_end || n.velocity <= 1 {
                    continue;
                }
                let ch = track_channels.get(n.track as usize).copied().unwrap_or(0);
                total += 1;
                *identical
                    .entry((k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
                *identical_ch
                    .entry((ch, k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
                if n.start_tick >= peak_start && n.start_tick < peak_end {
                    total_peak += 1;
                    *identical_peak
                        .entry((ch, k, n.velocity, n.start_tick, n.end_tick))
                        .or_default() += 1;
                }
            }
        }
        // 冗余率 = 1 - 组数/总数（可省掉的 voice 比例）
        let rate = |groups: usize| 1.0 - groups as f64 / total.max(1) as f64;
        // 峰值时刻"活跃"音符（start <= t < end）：直接对应 voice 峰的可减量
        let t_peak = peak_start.saturating_add(ppq / 2);
        let mut act_all: HashMap<(u8, u8, u32, u32), u32> = HashMap::new();
        let mut act_ch: HashMap<(u8, u8, u8, u32, u32), u32> = HashMap::new();
        let mut act_total = 0usize;
        for k in 0..128u8 {
            for n in model.notes[k as usize].iter() {
                if n.velocity <= 1 || n.start_tick > t_peak || n.end_tick <= t_peak {
                    continue;
                }
                let ch = track_channels.get(n.track as usize).copied().unwrap_or(0);
                act_total += 1;
                *act_all
                    .entry((k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
                *act_ch
                    .entry((ch, k, n.velocity, n.start_tick, n.end_tick))
                    .or_default() += 1;
            }
        }
        eprintln!(
            "\n=== {}\n  峰值第 {} 拍 count={} 窗口(±8拍)音符={total}\n  完全重复(跨通道)组={} 冗余率={:.1}% 最大组={}\n  完全重复(同通道)组={} 冗余率={:.1}% 最大组={}\n  峰值拍同通道 音符={total_peak} 冗余率={:.1}% 最大组={}",
            midi.rsplit('/').next().unwrap_or(midi),
            peak_beat / 4 + 1,
            peak_count,
            identical.len(),
            100.0 * rate(identical.len()),
            identical.values().copied().max().unwrap_or(0),
            identical_ch.len(),
            100.0 * rate(identical_ch.len()),
            identical_ch.values().copied().max().unwrap_or(0),
            100.0 * (1.0 - identical_peak.len() as f64 / total_peak.max(1) as f64),
            identical_peak.values().copied().max().unwrap_or(0),
        );
        eprintln!(
            "  峰值时刻活跃音符={act_total} 跨通道冗余率={:.1}%（最大组={}）同通道冗余率={:.1}%（最大组={}）",
            100.0 * (1.0 - act_all.len() as f64 / act_total.max(1) as f64),
            act_all.values().copied().max().unwrap_or(0),
            100.0 * (1.0 - act_ch.len() as f64 / act_total.max(1) as f64),
            act_ch.values().copied().max().unwrap_or(0),
        );
    }
}

/// 诊断（本地 MIDI，ignored）：Ouranos Track 13 第 33 小节起的音符模式
/// （同 key 间隔/同帧重复/力度分布）——定位 GPU 丢音与 CPU 的差异来源。
#[test]
#[ignore = "需要本地 MIDI"]
fn analyze_ouranos_track13_dense() {
    let path = "/Users/jieneng/Music/MIDIs/Ouranos - HDSQ & The Romanticist [v1.6.6].mid";
    let model = yinhe_midi::parse_path(path).expect("parse");
    let ppq = model.meta.ppq;
    eprintln!("tracks={} ppq={ppq}", model.tracks.len());
    for track_idx in [12usize, 13] {
        let mut notes: Vec<(u8, u8, u32, u32)> = Vec::new();
        for k in 0..128 {
            for n in model.notes[k].iter() {
                if n.track as usize == track_idx {
                    notes.push((k as u8, n.velocity, n.start_tick, n.end_tick));
                }
            }
        }
        if notes.is_empty() {
            continue;
        }
        notes.sort_by_key(|n| n.2);
        let bar33 = 32 * 4 * ppq;
        let dense: Vec<_> = notes.iter().filter(|n| n.2 >= bar33).take(200).collect();
        if dense.len() < 2 {
            continue;
        }
        let mut gap_count: std::collections::BTreeMap<i64, usize> = Default::default();
        for w in dense.windows(2) {
            *gap_count.entry(w[1].2 as i64 - w[0].2 as i64).or_default() += 1;
        }
        let keys: std::collections::BTreeSet<u8> = dense.iter().map(|n| n.0).collect();
        let vels: std::collections::BTreeSet<u8> = dense.iter().map(|n| n.1).collect();
        let lens: std::collections::BTreeSet<u32> = dense.iter().map(|n| n.3 - n.2).collect();
        eprintln!(
            "track[{track_idx}] 总={} 第33小节起取样={} keys={:?} vels={:?} 长度={:?}",
            notes.len(),
            dense.len(),
            keys.iter().take(6).collect::<Vec<_>>(),
            vels.iter().take(6).collect::<Vec<_>>(),
            lens.iter().take(6).collect::<Vec<_>>()
        );
        eprintln!(
            "  间隔分布(tick→次数，前12)：{:?}",
            gap_count.iter().take(12).collect::<Vec<_>>()
        );
        eprintln!("  前12个(start,end)：{:?}", &dense[..dense.len().min(12)]);
    }
}

/// 需求：同 MIDI 通道、同 tick、同一语义目标（CC1）的多轨自动化播放时取 max
/// （旧行为是后写覆盖 = track 索引大者胜出），事件流本身不合并（导出不变）。
/// 两个写入顺序都生效。
#[test]
fn test_same_tick_same_channel_cc_takes_max() {
    let sr = 44100u32;
    for (v0, v1) in [(40u8, 100u8), (100, 40)] {
        let model = Arc::new(make_two_track_cc_model(1, 100, v0, v1));
        let mut engine = AudioEngine::new(sr, ChannelLayout::from_mask(vec![true; 16]));
        engine.load_model(&model);
        engine.playback.playing = true;

        // 模型数据不合并：事件流仍保留两条（导出/编辑不受影响）。
        let cc1_count = engine
            .playback
            .cc_events
            .iter()
            .filter(|c| matches!(c.event, AudioEvent::Cc { controller: 1, .. }))
            .count();
        assert_eq!(cc1_count, 2, "事件流不合并数据（仍两条）");

        // CPU dispatch：两条都被消费（cursor 越过），实际只发一条代表。
        engine.dispatch_and_find_next(100, 100_000);
        assert_eq!(engine.playback.cc_cursor, 2, "两条都被消费");
        assert_ne!(
            engine.playback.dispatched_skip.cc_mask[0] & (1u128 << 1),
            0,
            "CC1 应已发送（代表事件）"
        );

        // 聚合结果：同键只发一条代表，值 = max。
        let reps = cc_reps(&engine);
        assert_eq!(reps.len(), 1, "同键只发一条代表事件");
        assert_eq!(reps[0], (1, 100.0), "同 tick 同通道同 CC 取 max");
    }
}

/// 需求：被 AM M/S 旁通（am_lane_skip）的 lane 不参与 max，取未旁通者的值；
/// 轨道 mute（skip_track）同理；全部被过滤则整组不发。
#[test]
fn test_same_tick_cc_max_respects_track_mute_and_am_ms() {
    let model = Arc::new(make_two_track_cc_model(1, 100, 40, 100));
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);

    // 无旁通：取 max = 100。
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);

    // 轨道 mute（skip_track）：取未被 mute 轨道那条的值。
    engine.playback.skip_track = vec![true, false];
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);
    engine.playback.skip_track = vec![false, true];
    assert_eq!(cc_reps(&engine), vec![(1, 40.0)]);
    // 两条都被 mute → 整组不发。
    engine.playback.skip_track = vec![true, true];
    assert!(cc_reps(&engine).is_empty());

    // CPU dispatch：全旁通时不发送（不打点 chase_skip），但 cursor 仍越过。
    engine.playback.cc_cursor = 0;
    engine.playback.dispatched_skip = crate::channel::ChaseSkip::default();
    engine.dispatch_and_find_next(100, 100_000);
    assert_eq!(
        engine.playback.dispatched_skip.cc_mask[0] & (1u128 << 1),
        0,
        "全被过滤则不发"
    );
    assert_eq!(engine.playback.cc_cursor, 2, "事件仍被消费越过");

    // AM M/S（am_lane_skip，由模型 + am_ms map 构建）：mute track0 的 lane
    // → 取 track1 = 100。
    engine.playback.skip_track = vec![false, false];
    let mut am_ms = crate::spawn::AmMsMap::new();
    am_ms.insert(
        (0u16, cc_target(1)),
        yinhe_types::AmMsState {
            mute: true,
            solo: false,
        },
    );
    engine.playback.am_lane_skip = crate::audio_model::build_am_lane_skip(&model, &am_ms);
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);

    // solo track1 的 lane：同轨只有它 solo，track0 未 solo 被旁通 → 取 100。
    let mut am_ms = crate::spawn::AmMsMap::new();
    am_ms.insert(
        (1u16, cc_target(1)),
        yinhe_types::AmMsState {
            mute: false,
            solo: true,
        },
    );
    engine.playback.am_lane_skip = crate::audio_model::build_am_lane_skip(&model, &am_ms);
    assert_eq!(cc_reps(&engine), vec![(1, 100.0)]);

    // 两条都 mute → 整组不发。
    let mut am_ms = crate::spawn::AmMsMap::new();
    for track in 0u16..2 {
        am_ms.insert(
            (track, cc_target(1)),
            yinhe_types::AmMsState {
                mute: true,
                solo: false,
            },
        );
    }
    engine.playback.am_lane_skip = crate::audio_model::build_am_lane_skip(&model, &am_ms);
    assert!(cc_reps(&engine).is_empty());
}

/// 需求：chase（seek 恢复）与播放一致——同通道同 tick 同目标取 max；
/// 轨道 mute / AM M/S 旁通的 lane 不参与；全被过滤则无状态（应用时不触碰通道）。
#[test]
fn test_chase_same_tick_same_target_takes_max() {
    // 两轨同通道 CC7（volume）：同 tick 100，track0=40、track1=100。
    let make = |v0: u8, v1: u8| {
        let lane = |track: u16, value: u8| AutomationLane {
            target: cc_target(7),
            track,
            events: vec![AutomationEvent {
                id: 0,
                tick: 100,
                value: value as f32 / 127.0,
                shape: SegmentShape::Step,
            }],
        };
        let mut t0 = TrackData::new(0, 0);
        t0.automation_lanes = vec![lane(0, v0)];
        let mut t1 = TrackData::new(0, 0);
        t1.automation_lanes = vec![lane(1, v1)];
        let mut m = YinModel {
            tracks: vec![Arc::new(t0), Arc::new(t1)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        m.rebuild();
        m
    };

    let model = make(40, 100);
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[false, false]);
    assert_eq!(
        states[0].as_ref().map(|s| s.volume),
        Some(100),
        "同 tick 同目标取 max"
    );

    // 写入顺序颠倒（track0=100、track1=40）仍取 max。
    let reversed = make(100, 40);
    let states = crate::spawn::compute_chase_states_for_test(&reversed, 200, &[false, false]);
    assert_eq!(
        states[0].as_ref().map(|s| s.volume),
        Some(100),
        "顺序颠倒仍取 max"
    );

    // 轨道 mute：只取未被 mute 那条。
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[true, false]);
    assert_eq!(states[0].as_ref().map(|s| s.volume), Some(100));
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[false, true]);
    assert_eq!(states[0].as_ref().map(|s| s.volume), Some(40));
    // 两条都 mute → 无状态（chase 应用时不触碰通道）。
    let states = crate::spawn::compute_chase_states_for_test(&model, 200, &[true, true]);
    assert!(states[0].is_none());

    // AM M/S 旁通：mute track0 的 CC7 lane → 取 track1 = 100。
    let mut am_ms = crate::spawn::AmMsMap::new();
    am_ms.insert(
        (0u16, cc_target(7)),
        yinhe_types::AmMsState {
            mute: true,
            solo: false,
        },
    );
    let states = crate::spawn::compute_chase_states_with_am_ms_for_test(
        &model,
        200,
        &[false, false],
        &am_ms,
    );
    assert_eq!(states[0].as_ref().map(|s| s.volume), Some(100));

    // 两条 lane 都 mute → 无状态。
    am_ms.insert(
        (1u16, cc_target(7)),
        yinhe_types::AmMsState {
            mute: true,
            solo: false,
        },
    );
    let states = crate::spawn::compute_chase_states_with_am_ms_for_test(
        &model,
        200,
        &[false, false],
        &am_ms,
    );
    assert!(states[0].is_none(), "全部旁通则整组不发（无状态）");
}

/// 需求：预览 chase 与播放一致——同 tick 同通道同目标取 max；不同语义键
/// （CC7 与 CC10）互不干扰；更晚 tick 的同键事件照常后写覆盖（不跨 tick 聚合）。
#[test]
fn test_preview_chase_same_tick_takes_max() {
    use crate::audio_model::SortedCC;
    use crate::preview_engine::chase_channel_states;

    let ev = |tick: u32, cc: u8, value: u8| SortedCC {
        tick,
        channel: 0,
        track: 0,
        lane: 0,
        plugin_param: None,
        event: AudioEvent::Cc {
            controller: cc,
            value: f32::from(value),
        },
    };
    let cc_events = vec![
        ev(10, 7, 40),
        ev(10, 7, 100),
        ev(10, 10, 20),
        ev(10, 10, 80),
        ev(20, 7, 10), // 更晚的同键事件：后写覆盖
    ];
    let states = chase_channel_states(&cc_events, 0, &[10, 20]);
    assert_eq!(states[0].volume, 100, "同 tick 同键取 max");
    assert_eq!(states[0].pan, 80, "不同语义键互不干扰");
    assert_eq!(states[1].volume, 10, "更晚的同键事件照常生效");
}

/// 画像（本地 tau + SoundFont，ignored）：复音上限 / layer 对实时倍率的影响。
///
/// 移动端默认 `max_voices=1024`、`layer=4`；tau 峰值段的实际 voice 数远超
/// 这个值。这里用固定 512 帧块测“1 秒音频需要多少墙钟”，定位默认值是否
/// 足以留出调度余量（实时倍率 > 1 才可能稳定）。
#[test]
#[ignore = "需要本地 tau + SoundFont"]
fn prof_tau_voice_caps() {
    use std::sync::Arc;
    use std::time::Instant;
    use yinhe_types::NoteSource;

    let midi = "/Users/jieneng/Music/MIDIs/tau2.5.9.mid";
    let sfz = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
    let sr = 48_000u32;
    let model = Arc::new(yinhe_midi::parse_path(midi).expect("parse tau"));
    let ppq = model.meta.ppq;
    let mut per_beat: BTreeMap<u32, usize> = Default::default();
    for key in 0..KEY_COUNT {
        for n in model.key_notes(key as u8).iter() {
            *per_beat.entry(n.start_tick / ppq).or_default() += 1;
        }
    }
    let (peak_beat, peak_count) = per_beat
        .iter()
        .max_by_key(|(_, v)| **v)
        .map(|(b, c)| (*b, *c))
        .expect("tau has notes");
    let layout = crate::spawn::channels_for_model(&model);
    let active: Vec<u8> = (0..16u8)
        .filter(|c| layout.is_active(*c as usize))
        .collect();
    let start_sample = {
        let mut probe = AudioEngine::new(sr, layout.clone());
        probe.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        probe.tick_to_sample(peak_beat.saturating_mul(ppq).saturating_sub(ppq * 2))
    };
    eprintln!(
        "=== tau voice caps: peak_beat={peak_beat} count={peak_count} start_sample={start_sample}"
    );

    for (cap, layer) in [
        (Some(1024usize), Some(4usize)),
        (Some(512), Some(4)),
        (Some(256), Some(4)),
        (Some(1024), Some(2)),
        (Some(1024), Some(1)),
    ] {
        let mut engine = AudioEngine::new(sr, layout.clone());
        engine.handle_command(AudioCommand::LoadModel {
            model: Arc::clone(&model),
        });
        let mut cs = yinhe_synth::CpuSynth::new(sr);
        for c in &active {
            cs.load_dense_soundfonts(*c as u32, &[std::path::PathBuf::from(sfz)])
                .expect("cpu sf");
        }
        if let Some(cap) = cap {
            cs.set_max_voices(cap);
        }
        cs.set_layer_count(layer);
        engine.synth.cpu_synth = cs;
        engine.set_max_voices(cap);
        engine.set_layer_count(layer);
        engine.handle_command(AudioCommand::Play {
            from_sample: start_sample,
        });

        let frames = 512usize;
        let mut buf = vec![0.0f32; frames * 2];
        let target = sr as u64 / 2; // 0.5s
        let mut rendered = 0u64;
        let mut peak_voice = 0u64;
        let mut max_chunk = 0.0f64;
        let t0 = Instant::now();
        while rendered < target {
            let tc = Instant::now();
            engine.render(&mut buf);
            let dt = tc.elapsed().as_secs_f64() * 1000.0;
            rendered += frames as u64;
            peak_voice = peak_voice.max(engine.voice_count());
            if rendered > sr as u64 / 8 {
                max_chunk = max_chunk.max(dt);
            }
        }
        let elapsed = t0.elapsed().as_secs_f64();
        eprintln!(
            "  cap={cap:?} layer={layer:?} 0.5s音频={elapsed:.2}s ({:.2}x) peak_voice={peak_voice} max_chunk={max_chunk:.1}ms",
            0.5 / elapsed.max(1e-9)
        );
    }
}

/// 循环区间映射到零 sample（极快 tempo 下 1 tick < 1 sample）时，回绕不能
/// 让播放位置前进；必须按普通播放继续，而不是在音频线程里死循环。
#[test]
fn zero_sample_loop_does_not_spin() {
    let model = make_model_with_tempo(vec![(0, 12000.0)], vec![]);
    let mut engine = AudioEngine::new(44100, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.playback.playing = true;

    // tick 0..1 在 12000 BPM 下都落在 sample 0。
    engine.set_loop(true, 0, 1);
    let mut out = vec![0.0f32; 512 * 2];
    engine.render(&mut out);
    assert_eq!(
        engine.playback.sample_position, 512,
        "零 sample 循环必须照常前进"
    );
}

/// 暂停后 insert 尾音仍由 render_tail 输出，且不推进走带。
#[test]
fn pause_keeps_insert_tail_and_transport_frozen() {
    let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
    let model = Arc::new(model);
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.insert_add(
        crate::spawn::InsertTarget::Midi(0),
        0,
        Box::new(TailInsert { value: 0.5 }),
    );

    let pos = engine.sample_position();
    engine.handle_command(crate::spawn::AudioCommand::Pause);
    let peak = tail_peak(&mut engine);

    assert!(peak > 0.01, "暂停后应继续输出 insert 尾音，peak={peak}");
    assert_eq!(engine.sample_position(), pos, "尾音渲染不得推进走带");
}

/// Stop 复位走带到 0，但不 reset insert，尾音继续。
#[test]
fn stop_keeps_insert_tail() {
    let model = make_model_with_notes(vec![(60, 0, 480, 100, 0)]);
    let model = Arc::new(model);
    let mut engine = AudioEngine::new(48000, ChannelLayout::from_mask(vec![true; 16]));
    engine.load_model(&model);
    engine.insert_add(
        crate::spawn::InsertTarget::Midi(0),
        0,
        Box::new(TailInsert { value: 0.5 }),
    );

    engine.handle_command(crate::spawn::AudioCommand::StopTransport);
    assert_eq!(engine.sample_position(), 0, "Stop 应回到 0");
    let peak = tail_peak(&mut engine);
    assert!(peak > 0.01, "Stop 后应继续输出 insert 尾音，peak={peak}");
}

/// MixerGain/MixerPan 从 LC 节点 automation 求值。
#[test]
fn mixer_gain_pan_automation_reads_lc_nodes() {
    use yinhe_mixer::MixerParams;
    use yinhe_types::{
        AutomationEvent, AutomationLane, AutomationTarget, LogicalChannelId, MixerChannelId,
        MixerNodeId, SegmentShape,
    };

    let layout = ChannelLayout::from_mask(vec![true]);
    let mut engine = AudioEngine::new(48_000, layout);
    let mut mixer = MixerParams::default();
    mixer.ensure_len();
    let channel = MixerChannelId::Midi(0);
    let node = mixer.midi_node_mut(0).expect("midi lc");
    node.automation.push(AutomationLane {
        target: AutomationTarget::MixerGain { channel },
        track: 0,
        events: vec![AutomationEvent {
            id: 1,
            tick: 0,
            value: 0.25,
            shape: SegmentShape::Step,
        }],
    });
    node.automation.push(AutomationLane {
        target: AutomationTarget::MixerPan { channel },
        track: 0,
        events: vec![AutomationEvent {
            id: 2,
            tick: 0,
            value: 0.75,
            shape: SegmentShape::Step,
        }],
    });
    let _ = MixerNodeId::LogicalChannel(LogicalChannelId(0));
    engine.mix.mixer_params = mixer;
    engine.apply_mixer_automation(1);

    let node = engine.mix.mixer_params.midi_node(0).expect("midi lc");
    assert!((node.gain - 0.25).abs() < f32::EPSILON);
    assert!((node.pan - 0.5).abs() < f32::EPSILON);
}

/// MixerSend 自动化写回统一发送表（`channel_sends`）的发送量。
#[test]
fn mixer_send_automation_updates_channel_send_amount() {
    use yinhe_mixer::MixerParams;
    use yinhe_types::{
        AutomationEvent, AutomationLane, AutomationTarget, ChannelId, LogicalChannelId,
        MixerChannelId, SegmentShape, TRACK_LC_BASE,
    };

    let layout = ChannelLayout::from_mask(vec![true]);
    let mut engine = AudioEngine::new(48_000, layout);
    let mut mixer = MixerParams::default();
    mixer.ensure_len();
    let target = ChannelId::Lc(LogicalChannelId(TRACK_LC_BASE + 1));
    let channel = MixerChannelId::Midi(0);
    let source = mixer.channel_id_for(channel);
    assert!(mixer.set_channel_send(source, target, 0.25, false));
    mixer
        .midi_node_mut(0)
        .expect("midi lc")
        .automation
        .push(AutomationLane {
            target: AutomationTarget::MixerSend {
                source: channel,
                target,
            },
            track: 0,
            events: vec![AutomationEvent {
                id: 1,
                tick: 0,
                value: 0.5,
                shape: SegmentShape::Step,
            }],
        });
    engine.mix.mixer_params = mixer;
    engine.apply_mixer_automation(1);

    let amount = engine.mix.mixer_params.channel_send_amount(source, target);
    assert!((amount - 1.0).abs() < f32::EPSILON);
}

/// InsertParam 自动化把归一化值送进对应 insert 处理器。
#[test]
fn insert_param_automation_writes_processor_param() {
    use std::sync::{Arc, Mutex};
    use yinhe_mixer::{InsertProcessor, MixerParams};
    use yinhe_types::{
        AutomationEvent, AutomationLane, AutomationTarget, MixerChannelId, SegmentShape,
    };

    struct Recorder {
        params: Arc<Mutex<Vec<(u32, f64)>>>,
    }
    impl InsertProcessor for Recorder {
        fn process(&mut self, _left: &mut [f32], _right: &mut [f32]) {}
        fn set_param(&mut self, id: u32, value: f64) {
            if let Ok(mut p) = self.params.lock() {
                p.push((id, value));
            }
        }
        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
    }

    let layout = ChannelLayout::from_mask(vec![true]);
    let mut engine = AudioEngine::new(48_000, layout);
    let recorded = Arc::new(Mutex::new(Vec::new()));
    engine.mix.mixer.insert_insert(
        0,
        0,
        Box::new(Recorder {
            params: Arc::clone(&recorded),
        }),
    );

    let mut mixer = MixerParams::default();
    mixer.ensure_len();
    let channel = MixerChannelId::Midi(0);
    mixer
        .midi_node_mut(0)
        .expect("midi 0")
        .automation
        .push(AutomationLane {
            target: AutomationTarget::InsertParam {
                target: channel,
                slot: 0,
                id: 42,
                name: "Cutoff".into(),
            },
            track: 0,
            events: vec![AutomationEvent {
                id: 1,
                tick: 0,
                value: 0.6,
                shape: SegmentShape::Step,
            }],
        });
    engine.mix.mixer_params = mixer;
    engine.apply_mixer_automation(1);

    let params = recorded.lock().expect("params");
    let (id, value) = params.last().copied().expect("param");
    assert_eq!(id, 42);
    assert!((value - 0.6).abs() < 1e-6);
}

/// 冻结 LC 播放：清空合成器写入并覆盖冻结 PCM。
#[test]
fn frozen_lc_overwrites_target_channel_buffer() {
    use crate::audio_source::{DecodedAudio, WavePeaks};
    use std::sync::Arc;
    use yinhe_mixer::{FrozenAudio, MixerParams};

    let layout = ChannelLayout::from_mask(vec![true]);
    let mut engine = AudioEngine::new(48_000, layout);
    engine.mix.mixer.resize(1, 8, &[]);
    let mut mixer = MixerParams::default();
    mixer.ensure_len();
    mixer.midi_node_mut(0).expect("midi 0").freeze = Some(FrozenAudio {
        source: "freeze-a".into(),
        source_revision: 1,
        sample_rate: 48_000,
        frames: 8,
    });
    engine.mix.mixer_params = mixer;
    engine.plugins.audio_sources.insert(
        "freeze-a".into(),
        Arc::new(DecodedAudio {
            sample_rate: 48_000,
            frames: 8,
            left: Arc::from([0.25f32; 8]),
            right: Arc::from([0.5f32; 8]),
            peaks: WavePeaks::build(&[0.25; 8], &[0.5; 8]),
        }),
    );
    // 先写入合成器残留，确认会被覆盖。
    if let Some(cb) = engine.mix.mixer.channel_buffers_mut(0) {
        cb.left.fill(0.9);
        cb.right.fill(0.9);
    }
    engine.render_frozen_lcs(0, 0, 8);
    let cb = engine.mix.mixer.channel_buffers_mut(0).expect("channel");
    assert!(cb.left.iter().all(|&v| (v - 0.25).abs() < f32::EPSILON));
    assert!(cb.right.iter().all(|&v| (v - 0.5).abs() < f32::EPSILON));
}

/// 冻结 LC 即使没有活跃 voice，也必须标记为 active，否则混音图会跳过它。
#[test]
fn frozen_lc_forces_channel_active() {
    use yinhe_mixer::{FrozenAudio, MixerParams};

    let layout = ChannelLayout::from_mask(vec![true]);
    let mut engine = AudioEngine::new(48_000, layout);
    engine.mix.mixer.resize(1, 512, &[]);
    engine.synth.cpu_synth = yinhe_synth::CpuSynth::new(48_000);
    let mut mixer = MixerParams::default();
    mixer.ensure_len();
    mixer.midi_node_mut(0).expect("midi 0").freeze = Some(FrozenAudio {
        source: "freeze".into(),
        source_revision: 0,
        sample_rate: 48_000,
        frames: 8,
    });
    engine.mix.mixer_params = mixer;
    let mut active = Vec::new();
    engine.channel_activity(&mut active);
    assert_eq!(active.first().copied(), Some(true));
}

/// 冻结端到端：真实 CpuSynth 音源 → FreezeJob 捕获 → 结果非静音。
#[test]
fn freeze_job_renders_real_lc_audio() {
    use std::sync::Arc;
    use yinhe_mixer::MixerParams;
    use yinhe_types::MixerChannelId;

    let dir = tempfile::tempdir().expect("tempdir");
    let wav_path = dir.path().join("tone.wav");
    let sfz_path = dir.path().join("tone.sfz");
    let samples = vec![0.25f32; 4_800];
    let bytes =
        crate::audio_source::encode_wav_bytes(&samples, &samples, 48_000).expect("encode wav");
    std::fs::write(&wav_path, bytes).expect("write wav");
    std::fs::write(&sfz_path, "<region>\nsample=tone.wav key=60\n").expect("write sfz");

    let sr = 48_000u32;
    let model = make_model_with_notes(vec![(60, 0, 2_000, 100, 0)]);
    let layout = crate::spawn::channels_for_model(&model);
    let mut engine = AudioEngine::new(sr, layout);
    engine.handle_command(AudioCommand::LoadModel {
        model: Arc::new(model),
    });
    let mut synth = yinhe_synth::CpuSynth::new(sr);
    synth
        .load_dense_soundfonts(0, &[sfz_path])
        .expect("load soundfont");
    engine.synth.cpu_synth = synth;
    engine.mix.mixer_params = MixerParams::default();
    engine.handle_command(AudioCommand::Play { from_sample: 0 });

    engine.begin_freeze_capture(0);
    let progress = crate::freeze::FreezeProgress::new();
    let mut job = crate::freeze::FreezeJob::new(
        MixerChannelId::Midi(0),
        sr,
        4_800,
        512,
        Arc::clone(&progress),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    while job.step(&mut engine).expect("freeze step") {}
    job.complete(&mut engine);

    let p = progress.lock().expect("progress");
    let result = p.result.as_ref().expect("freeze result");
    assert_eq!(result.frames(), 4_800);
    assert!(
        result.left.iter().any(|s| s.abs() > 1e-5),
        "冻结结果不应是静音"
    );
}
