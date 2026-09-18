//! Ouranos 最高潮片段 GPU 渲染基准（手动运行，优化前后对比）：
//! YINHE_TEST_SFZ="..." cargo test --release -p yinhe-synth --test tmp_ouranos_bench -- --ignored --nocapture

use std::time::Instant;
use yinhe_synth::SynthEvent;

const OURANOS: &str = "/Users/jieneng/Music/MIDIs/Ouranos - HDSQ & The Romanticist [v1.6.6].mid";

fn mk_bufs(frames: usize) -> Vec<yinhe_mixer::ChannelBuffers> {
    (0..16)
        .map(|_| yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        })
        .collect()
}

#[test]
#[ignore]
fn ouranos_peak_gpu_bench() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("跳过：未设置 YINHE_TEST_SFZ");
        return;
    };
    let sr = 48_000u32;
    let frames = 4096usize;
    let t_parse = Instant::now();
    let model = yinhe_midi::parse_path(OURANOS).expect("parse");
    let ppq = model.meta.ppq as u64;
    eprintln!("解析 {:.2}s ppq={ppq}", t_parse.elapsed().as_secs_f64());

    // 全曲扫描线：活跃音符峰值（同时发声数）
    let mut points: Vec<(u64, i64)> = Vec::new();
    for k in 0..128usize {
        for n in model.notes[k].iter() {
            if n.velocity > 1 {
                points.push((n.start_tick as u64, 1));
                points.push((n.end_tick as u64, -1));
            }
        }
    }
    points.sort_unstable();
    let (mut alive, mut peak, mut peak_tick) = (0i64, 0i64, 0u64);
    for (t, d) in &points {
        alive += d;
        if alive > peak {
            peak = alive;
            peak_tick = *t;
        }
    }
    let mut bar = peak_tick / (4 * ppq);
    if let Some(b) = std::env::var("YINHE_BENCH_BAR")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        bar = b;
    }
    eprintln!(
        "全曲峰值：bar={}（第 {} 小节）tick={peak_tick} 同时发声={peak}",
        bar,
        bar + 1
    );

    // 取峰值小节前 1 后 3 小节窗口
    let win_start = bar.saturating_sub(1) * 4 * ppq;
    let win_end = (bar + 3) * 4 * ppq;
    let win_start_sec = model.tempo_map.tick_to_seconds(win_start);
    let win_start_sample = (win_start_sec * sr as f64) as u64;
    let mut events: Vec<SynthEvent> = Vec::new();
    let mut tracks: std::collections::BTreeSet<u16> = Default::default();
    let mut keys: std::collections::BTreeSet<u8> = Default::default();
    for k in 0..128usize {
        for n in model.notes[k].iter() {
            let s = n.start_tick as u64;
            if n.velocity <= 1 || s < win_start || s >= win_end {
                continue;
            }
            let e = n.end_tick as u64;
            let ss = (model.tempo_map.tick_to_seconds(s) * sr as f64) as u64;
            let es = (model.tempo_map.tick_to_seconds(e) * sr as f64) as u64;
            let sample = ss.saturating_sub(win_start_sample);
            events.push(SynthEvent::NoteOn {
                sample,
                channel: (n.track % 16) as u8,
                key: k as u8,
                velocity: n.velocity,
                end_sample: sample + (es - ss),
            });
            tracks.insert(n.track);
            keys.insert(k as u8);
        }
    }
    events.sort_by_key(|e| e.sample());
    eprintln!(
        "窗口 [bar{}-bar{}]：事件={} tracks={} keys={} 起={:.2}s",
        win_start / (4 * ppq) + 1,
        win_end / (4 * ppq),
        events.len(),
        tracks.len(),
        keys.len(),
        win_start_sec
    );

    let Ok(mut gpu) = yinhe_synth::GpuSynth::new_default(sr) else {
        eprintln!("跳过 GPU：不可用");
        return;
    };
    gpu.load_dense_soundfonts(0, &[std::path::PathBuf::from(&sfz)])
        .expect("load soundfont");
    gpu.finish_soundfont_load();
    gpu.prewarm(frames as u32);
    gpu.set_layer_count(None);
    if let Some(mv) = std::env::var("YINHE_BENCH_MAXV")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        gpu.set_max_voices(mv);
    }
    let mut bufs = mk_bufs(frames);
    gpu.load_events(events);
    // 预热 2 块
    for _ in 0..2 {
        gpu.render_to_mixer(&mut bufs);
    }
    let blocks = 40usize;
    let mut sum = [0.0f64; 6];
    let mut max_alive = 0u32;
    let mut sum_blocks = 0u32;
    let t0 = Instant::now();
    for _ in 0..blocks {
        gpu.render_to_mixer(&mut bufs);
        for (a, b) in sum.iter_mut().zip(gpu.diag_ms) {
            *a += b;
        }
        max_alive = max_alive.max(gpu.diag_alive);
        sum_blocks += gpu.diag_blocks;
    }
    let wall = t0.elapsed().as_secs_f64() * 1000.0 / blocks as f64;

    let n = blocks as f64;
    eprintln!(
        "GPU {blocks} 块（{frames}帧, 预算 {:.2}ms）：墙钟 {wall:.2}ms/块",
        frames as f64 / sr as f64 * 1000.0
    );
    eprintln!(
        "  分解/块：collect={:.2} submit={:.2} harvest={:.2} ring={:.2} out={:.2} compact={:.2} 平均dispatch={:.1} 峰值alive={max_alive}",
        sum[0] / n,
        sum[1] / n,
        sum[2] / n,
        sum[3] / n,
        sum[4] / n,
        sum[5] / n,
        sum_blocks as f64 / n
    );
    eprintln!(
        "  淘汰力度分桶（LO≤31/MID≤63/HI≥64）：{}/{}/{}  note_on 拒绝={}",
        yinhe_synth::gpu_synth::EVICT_VEL_LO.load(std::sync::atomic::Ordering::Relaxed),
        yinhe_synth::gpu_synth::EVICT_VEL_MID.load(std::sync::atomic::Ordering::Relaxed),
        yinhe_synth::gpu_synth::EVICT_VEL_HI.load(std::sync::atomic::Ordering::Relaxed),
        yinhe_synth::gpu_synth::NOTE_ON_REJECTED.load(std::sync::atomic::Ordering::Relaxed)
    );
    eprintln!(
        "  淘汰计数（累计到此刻）：layer={} evict={}",
        yinhe_synth::gpu_synth::LAYER_KILLS.load(std::sync::atomic::Ordering::Relaxed),
        yinhe_synth::gpu_synth::EVICT_KILLS.load(std::sync::atomic::Ordering::Relaxed)
    );
    let energy: f32 = bufs
        .iter()
        .map(|b| {
            b.left
                .iter()
                .chain(b.right.iter())
                .map(|v| v.abs())
                .sum::<f32>()
        })
        .sum();
    eprintln!("  末块输出能量 {energy:.3}");
}

/// 轻量 GPU vs CPU（yinhe-cpu）逐样本对比：Ouranos 高潮窗口、20 块（约 1.7s 音频）。
/// YINHE_TEST_SFZ=... YINHE_BENCH_BAR=148 cargo test --release -p yinhe-synth --test tmp_ouranos_bench gpu_vs_cpu_light -- --ignored --nocapture
#[test]
#[ignore]
fn gpu_vs_cpu_light() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("跳过：未设置 YINHE_TEST_SFZ");
        return;
    };
    let sr = 48_000u32;
    let frames = 4096usize;
    let bar = std::env::var("YINHE_BENCH_BAR")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(148);
    let model = yinhe_midi::parse_path(OURANOS).expect("parse");
    let ppq = model.meta.ppq as u64;
    let win_start = bar.saturating_sub(1) * 4 * ppq;
    let win_end = (bar + 3) * 4 * ppq;
    let win_start_sample = (model.tempo_map.tick_to_seconds(win_start) * sr as f64) as u64;
    let mut events: Vec<SynthEvent> = Vec::new();
    for k in 0..128usize {
        for n in model.notes[k].iter() {
            let st = n.start_tick as u64;
            if n.velocity <= 1 || st < win_start || st >= win_end {
                continue;
            }
            let ss = (model.tempo_map.tick_to_seconds(st) * sr as f64) as u64;
            let es = (model.tempo_map.tick_to_seconds(n.end_tick as u64) * sr as f64) as u64;
            let sample = ss.saturating_sub(win_start_sample);
            events.push(SynthEvent::NoteOn {
                sample,
                channel: (n.track % 16) as u8,
                key: k as u8,
                velocity: n.velocity,
                end_sample: sample + (es - ss),
            });
        }
    }
    events.sort_by_key(|e| e.sample());
    eprintln!("窗口 bar{bar}：事件={}", events.len());

    let path = std::path::PathBuf::from(&sfz);
    let Ok(mut gpu) = yinhe_synth::GpuSynth::new_default(sr) else {
        eprintln!("跳过 GPU");
        return;
    };
    gpu.load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("gpu load");
    gpu.finish_soundfont_load();
    gpu.prewarm(frames as u32);
    gpu.set_layer_count(None);
    gpu.load_events(events.clone());

    let mut cpu = yinhe_synth::CpuSynth::new(sr);
    cpu.load_dense_soundfonts(0, std::slice::from_ref(&path))
        .expect("cpu load");
    cpu.finish_soundfont_load();
    cpu.set_layer_count(None);
    cpu.load_events(events);

    let mut gbufs = mk_bufs(16);
    let mut cbufs = mk_bufs(16);
    let _ = frames;
    let mut max_diff = 0.0f64;
    let mut sq = 0.0f64;
    let mut n = 0usize;
    let mut worst = 0usize;
    let blocks = 20usize;
    let t = Instant::now();
    for b in 0..blocks {
        gpu.render_to_mixer(&mut gbufs);
        cpu.render_to_mixer(&mut cbufs);
        for ch in 0..gbufs.len() {
            for i in 0..gbufs[ch].left.len() {
                let g = gbufs[ch].left[i] as f64;
                let c = cbufs[ch].left[i] as f64;
                let d = (g - c).abs();
                if d > max_diff {
                    max_diff = d;
                    worst = b * gbufs[ch].left.len() + i;
                }
                sq += d * d;
                n += 1;
            }
        }
    }
    let rmse = (sq / n as f64).sqrt();
    let cpeak = cbufs
        .iter()
        .map(|b| b.left.iter().fold(0.0f32, |m, v| m.max(v.abs())))
        .fold(0.0f32, f32::max);
    let gpeak = gbufs
        .iter()
        .map(|b| b.left.iter().fold(0.0f32, |m, v| m.max(v.abs())))
        .fold(0.0f32, f32::max);
    eprintln!("末块峰值：gpu={gpeak:.4} cpu={cpeak:.4}");
    let peak = gbufs
        .iter()
        .map(|b| b.left.iter().fold(0.0f32, |m, v| m.max(v.abs())))
        .fold(0.0f32, f32::max) as f64;
    eprintln!(
        "GPU vs CPU：{blocks} 块用 {:.1}s  max_diff={max_diff:.5}（峰值 {peak:.3}，{:.2}%）rmse={rmse:.5} worst@block_frame={worst}",
        t.elapsed().as_secs_f64(),
        max_diff / peak.max(1e-9) * 100.0
    );
}
