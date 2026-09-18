//! 临时原型：帧内 SIMD（lane = 时间，4 帧/批）vs 标量渲染内核的纯计算对比。
//!
//! 只测 Sustain 段内核（位置 → 采样 gather → 线性插值 → 增益 → 输出交错累加），
//! 不含包络推进/循环回绕/边界检查（收益上限估计）。仅 aarch64（NEON）。
//!
//! 运行：
//! cargo test --release -p yinhe-synth --test tmp_simd_bench -- --nocapture

use std::hint::black_box;
use std::time::Instant;

/// 合成立体声采样（交错 LRLR），基频 + 谐波，避免编译器识别为简单模式。
fn make_sample(frames: usize) -> Vec<f32> {
    let mut s = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let x = i as f32 * 0.0137;
        let l = (x.sin() + (x * 2.7).sin() * 0.4 + (x * 5.3).sin() * 0.2) * 0.5;
        let r = (x * 1.01).sin() + (x * 2.9).cos() * 0.3;
        s.push(l);
        s.push(r);
    }
    s
}

#[inline]
fn scalar_frames(
    sample: &[f32],
    t0: f64,
    speed: f64,
    g_l: f32,
    g_r: f32,
    out: &mut [f32],
    n: usize,
) {
    let mut t = t0;
    for i in 0..n {
        let idx = t as u32 as usize;
        let frac = (t - idx as f64) as f32;
        let si = idx * 2;
        let l0 = sample[si];
        let r0 = sample[si + 1];
        let l1 = sample[si + 2];
        let r1 = sample[si + 3];
        let l = l0 + (l1 - l0) * frac;
        let r = r0 + (r1 - r0) * frac;
        out[i * 2] += l * g_l;
        out[i * 2 + 1] += r * g_r;
        t += speed;
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
#[allow(unsafe_op_in_unsafe_fn)] // 原型：整函数为 unsafe 上下文
unsafe fn simd4_frames(
    sample: &[f32],
    t0: f64,
    speed: f64,
    g_l: f32,
    g_r: f32,
    out: &mut [f32],
    n: usize,
) {
    use std::arch::aarch64::*;
    let mut t = t0;
    let mut i = 0;
    while i + 4 <= n {
        // 位置与 gather（标量，f64 无法 4 宽）
        let mut idx = [0usize; 4];
        let mut frac = [0f32; 4];
        for k in 0..4 {
            let p = t as u32 as usize;
            idx[k] = p * 2;
            frac[k] = (t - p as f64) as f32;
            t += speed;
        }
        let mut l0 = [0f32; 4];
        let mut l1 = [0f32; 4];
        let mut r0 = [0f32; 4];
        let mut r1 = [0f32; 4];
        for k in 0..4 {
            let si = idx[k];
            l0[k] = *sample.get_unchecked(si);
            r0[k] = *sample.get_unchecked(si + 1);
            l1[k] = *sample.get_unchecked(si + 2);
            r1[k] = *sample.get_unchecked(si + 3);
        }
        // 打包 + SIMD 插值/增益
        let f = vld1q_f32(frac.as_ptr());
        let il = vmlaq_f32(
            vld1q_f32(l0.as_ptr()),
            vsubq_f32(vld1q_f32(l1.as_ptr()), vld1q_f32(l0.as_ptr())),
            f,
        );
        let ir = vmlaq_f32(
            vld1q_f32(r0.as_ptr()),
            vsubq_f32(vld1q_f32(r1.as_ptr()), vld1q_f32(r0.as_ptr())),
            f,
        );
        let sl = vmulq_n_f32(il, g_l);
        let sr = vmulq_n_f32(ir, g_r);
        // 输出交错累加：vld2/vst2 一次处理 4 组 (L,R)
        let dst = out.as_mut_ptr().add(i * 2);
        let mut ov = vld2q_f32(dst);
        ov.0 = vaddq_f32(ov.0, sl);
        ov.1 = vaddq_f32(ov.1, sr);
        vst2q_f32(dst, ov);
        i += 4;
    }
    // 尾巴
    if i < n {
        let mut tt = t;
        for k in i..n {
            let idx = tt as u32 as usize;
            let frac = (tt - idx as f64) as f32;
            let si = idx * 2;
            let l = sample[si] + (sample[si + 2] - sample[si]) * frac;
            let r = sample[si + 1] + (sample[si + 3] - sample[si + 1]) * frac;
            out[k * 2] += l * g_l;
            out[k * 2 + 1] += r * g_r;
            tt += speed;
        }
    }
}

fn bench(label: &str, voices: usize, frames: usize, rounds: usize, speeds: &[f64], simd: bool) -> f64 {
    let sample = make_sample(32768);
    let mut out = vec![0f32; frames * 2];
    let bases: Vec<f64> = (0..voices).map(|v| (v % 3000) as f64 + 100.0).collect();
    let mut best = f64::MAX;
    for _ in 0..rounds {
        out.iter_mut().for_each(|x| *x = 0.0);
        let t0 = Instant::now();
        for v in 0..voices {
            let g_l = 0.3 + (v % 7) as f32 * 0.01;
            let g_r = 0.29 + (v % 5) as f32 * 0.01;
            if simd {
                #[cfg(target_arch = "aarch64")]
                // SAFETY: 位置由 (bases[v] + 512*speed) 限制在 sample 内（见下）。
                unsafe {
                    simd4_frames(
                        black_box(&sample),
                        bases[v],
                        speeds[v],
                        g_l,
                        g_r,
                        black_box(out.as_mut_slice()),
                        frames,
                    );
                }
                #[cfg(not(target_arch = "aarch64"))]
                {
                    let _ = (g_l, g_r);
                }
            } else {
                scalar_frames(
                    black_box(&sample),
                    bases[v],
                    speeds[v],
                    g_l,
                    g_r,
                    black_box(out.as_mut_slice()),
                    frames,
                );
            }
        }
        let dt = t0.elapsed().as_secs_f64();
        best = best.min(dt);
    }
    let ns_per_frame_voice = best * 1e9 / (voices * frames) as f64;
    println!(
        "{label:<28} {ns_per_frame_voice:>7.2} ns/frame/voice  （{voices} voice × {frames} 帧 × {rounds} 轮取 min）"
    );
    black_box(out.iter().sum::<f32>());
    ns_per_frame_voice
}

/// 诊断：常用钢琴库的滤波/循环分布——决定帧内 SIMD 的现实适用范围
/// （有滤波的 voice 无法帧内向量化 IIR）。
#[test]
fn diag_piano_cutoff_and_loop() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("跳过：未设置 YINHE_TEST_SFZ");
        return;
    };
    let entries =
        yinhe_synth::sfz_parser::build_key_maps(std::path::Path::new(&sfz), 48_000, 0)
            .expect("load soundfont");
    let (mut total, mut with_cut, mut loop_sus, mut loop_cont, mut no_loop) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for e in &entries {
        for key in 0..128usize {
            for info in &e.map[key] {
                total += 1;
                if info.cutoff > 0.0 {
                    with_cut += 1;
                }
                match info.loop_mode {
                    yinhe_synth::sfz_parser::LoopMode::LoopSustain => loop_sus += 1,
                    yinhe_synth::sfz_parser::LoopMode::LoopContinuous => loop_cont += 1,
                    _ => no_loop += 1,
                }
            }
        }
    }
    println!(
        "KeyInfo 总数={total} cutoff>0={with_cut}（{:.1}%） LoopSustain={loop_sus} LoopContinuous={loop_cont} 无循环={no_loop}",
        100.0 * with_cut as f64 / total.max(1) as f64
    );
    // cutoff/resonance/filter_type 的"随 key/vel 变化"程度：决定能否把
    // per-voice 滤波折叠为组级/通道级（线性系统：先求和后滤波等价）。
    use std::collections::BTreeSet;
    let mut all_cut = BTreeSet::new();
    let mut keys_active = 0usize;
    let mut per_vel_unique = 0usize;
    let mut res_set = BTreeSet::new();
    let mut ty_set = BTreeSet::new();
    let mut cut_min = f32::MAX;
    let mut cut_max = f32::MIN;
    for e in &entries {
        for key in 0..128usize {
            if e.map[key].is_empty() {
                continue;
            }
            keys_active += 1;
            let mut key_vals = BTreeSet::new();
            for info in &e.map[key] {
                let bits = info.cutoff.to_bits();
                all_cut.insert(bits);
                key_vals.insert(bits);
                res_set.insert(info.resonance.to_bits());
                ty_set.insert(format!("{:?}", info.filter_type));
                cut_min = cut_min.min(info.cutoff);
                cut_max = cut_max.max(info.cutoff);
            }
            if key_vals.len() > 1 {
                per_vel_unique += 1;
            }
        }
    }
    println!(
        "cutoff 全局唯一值={}（范围 {:.1}~{:.1} Hz；活跃键 {}）| 键内随力度变化={} 键数 | resonance 唯一值={} | filter_type={:?}",
        all_cut.len(),
        cut_min,
        cut_max,
        keys_active,
        per_vel_unique,
        res_set.len(),
        ty_set,
    );
}

/// GPU vs CPU（同场景长音符，1/4 层力度）：GPU 路径每块有状态读回/段上传，
/// 小复音下固定开销主导——量化当前差距与规模趋势。
#[test]
fn gpu_vs_cpu_scale() {
    let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
        eprintln!("跳过：未设置 YINHE_TEST_SFZ");
        return;
    };
    let frames = 512usize;
    let mk_bufs = || {
        (0..16)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect::<Vec<_>>()
    };
    for &layers in &[1u8, 4, 16, 46] {
        let events: Vec<yinhe_synth::SynthEvent> = (21..109u8)
            .flat_map(|k| {
                (0..layers).map(move |l| yinhe_synth::SynthEvent::NoteOn {
                    sample: 0,
                    channel: 0,
                    key: k,
                    velocity: 120u8.saturating_sub(l * 6).max(2),
                    end_sample: 48_000 * 30,
                })
            })
            .collect();
        let voices = events.len();
        // 预热用短事件序列（触发后保持响）
        let mut cpu = yinhe_synth::CpuSynth::new(48_000);
        cpu.load_dense_soundfonts(0, &[std::path::PathBuf::from(&sfz)])
            .expect("load soundfont");
        cpu.set_layer_count(None);
        let mut bufs = mk_bufs();
        cpu.load_events(events.clone());
        for _ in 0..20 {
            cpu.render_to_mixer(&mut bufs);
        }
        cpu.load_events(events.clone());
        let t0 = std::time::Instant::now();
        for _ in 0..200 {
            cpu.render_to_mixer(&mut bufs);
        }
        let cpu_ms = t0.elapsed().as_secs_f64() * 1000.0 / 200.0;

        let Ok(mut gpu) = yinhe_synth::GpuSynth::new_default(48_000) else {
            eprintln!("跳过 GPU：不可用");
            return;
        };
        gpu.load_dense_soundfonts(0, &[std::path::PathBuf::from(&sfz)])
            .expect("load soundfont");
        gpu.finish_soundfont_load();
        gpu.prewarm(frames as u32);
        gpu.set_layer_count(None);
        let mut gbufs = mk_bufs();
        gpu.load_events(events.clone());
        for _ in 0..5 {
            gpu.render_to_mixer(&mut gbufs);
        }
        gpu.load_events(events.clone());
        let t1 = std::time::Instant::now();
        for _ in 0..50 {
            gpu.render_to_mixer(&mut gbufs);
        }
        let gpu_ms = t1.elapsed().as_secs_f64() * 1000.0 / 50.0;
        println!("  （GPU voice_count = {}）", gpu.voice_count());
        let energy: f32 = gbufs
            .iter()
            .map(|b| {
                b.left
                    .iter()
                    .chain(b.right.iter())
                    .map(|v| v.abs())
                    .sum::<f32>()
            })
            .sum();
        println!("  （GPU 输出能量 {energy:.3}，0 = 未实际渲染）");
        println!(
            "voice={voices:>5}: CPU {cpu_ms:>7.3} ms/块（{:.2}x）  GPU {gpu_ms:>7.2} ms/块（{:.2}x）  预算 10.67ms",
            10.67 / cpu_ms,
            10.67 / gpu_ms
        );
    }
}

#[test]
fn simd4_vs_scalar_sustain() {
    if !cfg!(target_arch = "aarch64") {
        eprintln!("跳过：本原型仅 aarch64（NEON）");
        return;
    }
    let voices = 4000;
    let frames = 512;
    let rounds = 30;
    // 同 pitch（speed=1，位置近连续）与变调（0.5~2.0，gather）两种场景
    let same = vec![1.0f64; voices];
    let varied: Vec<f64> = (0..voices)
        .map(|v| 0.5 + (v % 37) as f64 * 0.04)
        .collect();
    println!("—— 同 pitch（speed=1）——");
    let a = bench("标量", voices, frames, rounds, &same, false);
    let b = bench("SIMD4", voices, frames, rounds, &same, true);
    println!("  加速比 {:.3}x", a / b);
    println!("—— 变调（speed 0.5~1.94）——");
    let c = bench("标量", voices, frames, rounds, &varied, false);
    let d = bench("SIMD4", voices, frames, rounds, &varied, true);
    println!("  加速比 {:.3}x", c / d);
}
