//! 位图方案画像（ignored）：5 个候选各自测「现状成本 vs 位图成本」。
//!
//! 目的：判断位图值不值得引入，而不是先写进生产代码再回头量。
//! 运行：
//! ```sh
//! cargo test --release -p yinhe-audio bitmap_bench -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use yinhe_core::YinModel;

const BW_DIR: &str = "/Users/jieneng/Music/MIDIs/[LC](Original)Broken World 44,444,444 Notes =)";
const SF2: &str = "/Users/jieneng/Music/Soundfonts/GeneralUser-GS.sf2";

fn parse(path: &str) -> Arc<YinModel> {
    Arc::new(yinhe_midi::parse_path(path).expect("parse"))
}

// ─────────────────────────────────────────────────────────────────────────
// 1. 混音台空闲通道：192 通道里每块真正有电平的通道数
// ─────────────────────────────────────────────────────────────────────────

/// 音符画版 Broken World：采样每块"有电平的通道数"，给出空闲通道跳过
/// 的收益上限（混音台 16→192 通道实测成本约 20%）。
#[test]
#[ignore = "需要本地 MIDI + SoundFont"]
fn b1_mixer_idle_channels() {
    let model = parse(&format!(
        "{BW_DIR}/[LC] (Original) Broken World 44,444,444 Notes =).mid"
    ));
    let layout = crate::channel_layout::ChannelLayout::from_mask(vec![true; 192]);
    let channels = layout.num_channels() as usize;
    let mut engine = crate::engine::AudioEngine::new(48_000, layout);
    engine.handle_command(crate::spawn::AudioCommand::LoadModel {
        model: Arc::clone(&model),
    });
    let mut synth = yinhe_synth::CpuSynth::new(48_000);
    synth
        .load_dense_soundfonts_many(&(0..channels as u32).collect::<Vec<_>>(), &[SF2.into()])
        .expect("sf");
    engine.cpu_synth = Some(synth);
    let start = (415.38 * 48_000.0) as u64;
    engine.handle_command(crate::spawn::AudioCommand::Play { from_sample: start });

    let mut out = vec![0.0f32; 512 * 2];
    let mut ever = vec![false; channels];
    let mut active_sum = 0u64;
    let mut blocks = 0u64;
    let mut max_active = 0usize;
    let t0 = Instant::now();
    let target = 3 * 48_000;
    let mut rendered = 0u64;
    while rendered < target {
        engine.render(&mut out);
        rendered += 512;
        let mut n = 0usize;
        for c in 0..channels {
            if let Some(m) = engine.mixer.channel_meter_reading(c) {
                let (l, r) = m.read();
                if l.max(r) > 0.0 {
                    ever[c] = true;
                    n += 1;
                }
            }
        }
        active_sum += n as u64;
        max_active = max_active.max(n);
        blocks += 1;
    }
    let el = t0.elapsed().as_secs_f64();
    let ever_n = ever.iter().filter(|&&v| v).count();
    eprintln!(
        "\n[B1] 混音台空闲通道\n  通道={channels} 3s渲染={el:.2}s ({:.1}x)\n  出现过电平的通道={ever_n} 平均每块活跃={:.1} 峰值={max_active}\n  若跳过全零通道的收益上限≈{:.0}%",
        3.0 / el.max(1e-9),
        active_sum as f64 / blocks.max(1) as f64,
        100.0 * (1.0 - active_sum as f64 / blocks.max(1) as f64 / channels as f64),
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 2. 每轨时间网格位图（冻结/裁剪用）
// ─────────────────────────────────────────────────────────────────────────

/// 每轨每拍 1 bit（该拍是否有可听音符）：构造成本、内存、区间查询成本。
#[test]
#[ignore = "需要本地 MIDI"]
fn b2_track_activity_bitmap() {
    let model = parse(&format!(
        "{BW_DIR}/[LC] (Original) Broken World 44,444,444 Notes =).mid"
    ));
    let ppq = model.meta.ppq.max(1);
    let beats = (model.tick_length / ppq as u64 + 1) as usize;
    let tracks = model.tracks.len();
    let words_per_track = beats.div_ceil(64);

    let t = Instant::now();
    let mut bits = vec![0u64; tracks * words_per_track];
    for key in 0..256 {
        for n in model.notes[key].iter() {
            if n.velocity <= 1 {
                continue;
            }
            let b = (n.start_tick / ppq) as usize;
            if b < beats {
                let i = n.track as usize * words_per_track + b / 64;
                bits[i] |= 1 << (b % 64);
            }
        }
    }
    let build = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "\n[B2] 每轨时间网格位图（Broken World 音符画）\n  轨道={tracks} 拍={beats} 构建={build:.1}ms 内存={:.2}MB",
        bits.len() as f64 * 8.0 / (1024.0 * 1024.0)
    );

    // 区间查询：位图 vs 逐 key range 扫描。
    let mut x = 0x9e3779b97f4a7c15u64;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let queries: Vec<(usize, usize, usize)> = (0..10_000)
        .map(|_| {
            let track = rnd() as usize % tracks;
            let a = rnd() as usize % beats;
            let len = 1 + rnd() as usize % 64;
            (track, a, (a + len).min(beats))
        })
        .collect();

    let t = Instant::now();
    let mut hits = 0usize;
    for &(track, a, b) in &queries {
        let base = track * words_per_track;
        let (wa, wb) = (a / 64, (b - 1) / 64);
        let mut found = false;
        for w in wa..=wb {
            let mut word = bits[base + w];
            if w == wa {
                word &= !0u64 << (a % 64);
            }
            if w == wb && b % 64 != 0 {
                word &= !0u64 >> (64 - b % 64);
            }
            if word != 0 {
                found = true;
                break;
            }
        }
        hits += found as usize;
    }
    let bit_ns = t.elapsed().as_nanos() as f64 / queries.len() as f64;

    let t = Instant::now();
    let mut hits2 = 0usize;
    for &(track, a, b) in queries.iter().take(500) {
        let (lo, hi) = (a as u32 * ppq, b as u32 * ppq);
        let found = (0..256).any(|k| {
            model.notes[k]
                .range(lo, hi)
                .any(|n| n.track as usize == track && n.velocity > 1)
        });
        hits2 += found as usize;
    }
    let scan_us = t.elapsed().as_secs_f64() * 1e6 / 500.0;
    eprintln!(
        "  查询：位图 {bit_ns:.0}ns/次（命中 {hits}） vs 逐key扫描 {scan_us:.1}µs/次（500 次命中 {hits2}）"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 3. NoteBucket 空块位图
// ─────────────────────────────────────────────────────────────────────────

/// 90% 空块时，区间查询扫块的成本 vs 空块位图跳过。
#[test]
#[ignore = "bench（需要 --ignored 手动跑）"]
fn b3_empty_chunk_bitmap() {
    // 模拟 1000 块、10% 有 64 个音符（start_tick 每块递增）。
    const CHUNKS: usize = 1000;
    let mut chunks: Vec<Vec<u32>> = vec![Vec::new(); CHUNKS];
    for (i, c) in chunks.iter_mut().enumerate() {
        if i % 10 == 0 {
            *c = (0..64).map(|k| (i * 64 + k) as u32).collect();
        }
    }
    let starts: Vec<u32> = chunks
        .iter()
        .map(|c| c.first().copied().unwrap_or(u32::MAX))
        .collect();
    let occupancy: Vec<u64> = (0..CHUNKS.div_ceil(64))
        .map(|w| {
            let mut v = 0u64;
            for b in 0..64 {
                let i = w * 64 + b;
                if i < CHUNKS && !chunks[i].is_empty() {
                    v |= 1 << b;
                }
            }
            v
        })
        .collect();

    let q: Vec<(usize, usize)> = (0..100_000)
        .map(|i| {
            let a = (i * 37) % CHUNKS;
            (a, (a + 3).min(CHUNKS))
        })
        .collect();

    let t = Instant::now();
    let mut sum = 0usize;
    for &(a, b) in &q {
        for (i, c) in chunks.iter().enumerate().take(b).skip(a) {
            let _ = starts[i];
            sum += c.len();
        }
    }
    let scan_ns = t.elapsed().as_nanos() as f64 / q.len() as f64;

    let t = Instant::now();
    let mut sum2 = 0usize;
    for &(a, b) in &q {
        let (wa, wb) = (a / 64, (b - 1) / 64);
        for w in wa..=wb {
            let mut word = occupancy[w];
            if w == wa {
                word &= !0u64 << (a % 64);
            }
            if w == wb && b % 64 != 0 {
                word &= !0u64 >> (64 - b % 64);
            }
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                sum2 += chunks[w * 64 + bit].len();
                word &= word - 1;
            }
        }
    }
    eprintln!(
        "\n[B3] 空块位图（{CHUNKS} 块，90% 空，区间 3 块）\n  扫块 {scan_ns:.0}ns/次 vs 位图 {:.0}ns/次（sum {sum} / {sum2}）",
        t.elapsed().as_nanos() as f64 / q.len() as f64
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 4. 自动化 lane 覆盖位图
// ─────────────────────────────────────────────────────────────────────────

/// 2000 条 lane × 2000 个事件；查询「某区间有没有事件」：排序二分 vs 位图。
#[test]
#[ignore = "bench（需要 --ignored 手动跑）"]
fn b4_lane_coverage_bitmap() {
    const LANES: usize = 2000;
    const EVENTS: usize = 2000;
    const TICKS: u32 = 1_000_000;
    const CELL: u32 = 512; // 每 bit 覆盖 512 tick
    let cells = (TICKS / CELL + 1) as usize;
    let words = cells.div_ceil(64);

    let mut lanes: Vec<Vec<u32>> = (0..LANES)
        .map(|l| {
            (0..EVENTS)
                .map(|e| ((l * 7919 + e * 104729) % TICKS as usize) as u32)
                .collect::<Vec<_>>()
                .tap_sort()
        })
        .collect();
    let bits: Vec<Vec<u64>> = lanes
        .iter()
        .map(|ev| {
            let mut w = vec![0u64; words];
            for &t in ev {
                let c = (t / CELL) as usize;
                w[c / 64] |= 1 << (c % 64);
            }
            w
        })
        .collect();

    let q: Vec<(usize, u32, u32)> = (0..200_000)
        .map(|i| {
            let lane = (i * 37) % LANES;
            let a = ((i * 104729) % (TICKS as usize / 2)) as u32;
            (lane, a, a + 20_000)
        })
        .collect();

    let t = Instant::now();
    let mut hit = 0usize;
    for &(lane, a, b) in &q {
        let ev = &lanes[lane];
        let i = ev.partition_point(|&t| t < a);
        if i < ev.len() && ev[i] < b {
            hit += 1;
        }
    }
    let bin_ns = t.elapsed().as_nanos() as f64 / q.len() as f64;

    let t = Instant::now();
    let mut hit2 = 0usize;
    for &(lane, a, b) in &q {
        let w = &bits[lane];
        let (ca, cb) = ((a / CELL) as usize, (b / CELL) as usize);
        let (wa, wb) = (ca / 64, (cb / 64).min(words - 1));
        let mut found = false;
        for wi in wa..=wb {
            let mut word = w[wi];
            if wi == wa {
                word &= !0u64 << (ca % 64);
            }
            if wi == wb {
                word &= !0u64 >> (63 - cb % 64);
            }
            if word != 0 {
                found = true;
                break;
            }
        }
        hit2 += found as usize;
    }
    let bit_ns = t.elapsed().as_nanos() as f64 / q.len() as f64;
    // 防止优化器把未读的 lanes 去掉
    for l in &mut lanes {
        l.truncate(0);
    }
    eprintln!(
        "\n[B4] 自动化 lane 覆盖位图（{LANES} lane × {EVENTS} 事件，区间 20000 tick）\n  二分 {bin_ns:.0}ns/次（命中 {hit}） vs 位图 {bit_ns:.0}ns/次（命中 {hit2}） 位图内存={:.2}MB",
        bits.len() * words * 8 / (1024 * 1024)
    );
}

trait TapSort {
    fn tap_sort(self) -> Self;
}
impl TapSort for Vec<u32> {
    fn tap_sort(mut self) -> Self {
        self.sort_unstable();
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5. 音频片段时间块位图
// ─────────────────────────────────────────────────────────────────────────

/// 10000 个片段、1 小时：每块线性扫描 vs 「块内有片段」位图 + 线性。
#[test]
#[ignore = "bench（需要 --ignored 手动跑）"]
fn b5_clip_block_bitmap() {
    const CLIPS: usize = 10_000;
    const SR: u64 = 48_000;
    const FRAMES: usize = 512;
    const HOUR: u64 = 3600;
    let blocks = (HOUR * SR / FRAMES as u64) as usize;
    // 片段长度 1 秒，起点均匀分布，允许重叠。
    let clips: Vec<(u64, u64, u64)> = (0..CLIPS)
        .map(|i| {
            let start = (i as u64 * SR * 3600 / CLIPS as u64) % (HOUR * SR);
            (i as u64, start, start + SR)
        })
        .collect();
    let mut bits = vec![0u64; blocks.div_ceil(64)];
    for &(_, s, e) in &clips {
        let (a, b) = ((s / FRAMES as u64) as usize, (e / FRAMES as u64) as usize);
        for blk in a..=b.min(blocks - 1) {
            bits[blk / 64] |= 1 << (blk % 64);
        }
    }

    let t = Instant::now();
    let mut hit = 0usize;
    for blk in 0..blocks {
        let s = blk as u64 * FRAMES as u64;
        let e = s + FRAMES as u64;
        for &(_, cs, ce) in &clips {
            if cs < e && ce > s {
                hit += 1;
            }
        }
    }
    let scan_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let mut hit2 = 0usize;
    for blk in 0..blocks {
        let word = bits[blk / 64];
        if word & (1 << (blk % 64)) == 0 {
            continue;
        }
        let s = blk as u64 * FRAMES as u64;
        let e = s + FRAMES as u64;
        for &(_, cs, ce) in &clips {
            if cs < e && ce > s {
                hit2 += 1;
            }
        }
    }
    eprintln!(
        "\n[B5] 音频片段时间块位图（{CLIPS} 片段，1 小时，{blocks} 块）\n  全块线性 {scan_ms:.2}ms vs 位图跳过空块 {:.2}ms（命中 {hit}/{hit2}） 位图={}B",
        t.elapsed().as_secs_f64() * 1000.0,
        bits.len() * 8
    );
}
