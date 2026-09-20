//! 合成大规模音符基准：现状 GPU cull + 渲染路径（无 LOD）的每帧成本。
//!
//! 目的：拿到「可见音符数 → 帧时间」的真实曲线，判断 1 亿音符全曲视图
//! 离 60fps 有多远、瓶颈在 CPU 命令录制还是 GPU。
//!
//! 运行：
//!   cargo test -p yinhe-wgpu --release bench_synthetic_scale -- --ignored --nocapture
//!
//! 环境变量：
//!   YIN_BENCH_NOTES  总音符数（默认 100_000_000）
//!   YIN_BENCH_STEP   同一 key 内相邻音符 start_tick 间距（默认 64）
//!   YIN_BENCH_FRAMES 每个缩放档位的测量帧数（默认 3）
//!   YIN_BENCH_TRACKS 轨道数（默认 8）

use super::CullState;
use crate::vertex::{MAX_SEL_RECTS, NoteInstance, SelectionUniform, Uniforms};
use wgpu::*;
use yinhe_types::{KEY_COUNT, MAX_KEY};

/// 基准专用 device：关闭 `VALIDATION_INDIRECT_CALL`，与生产配置
/// （main.rs 的 instance flags）一致。默认 release 构建仍带此 flag，
/// wgpu-core 会对每条 indirect args 做 CPU 校验（multi_draw 时逐条
/// `DrawBatcher::add`），1 亿音符全曲视图下约 28ms/帧，掩盖真实成本。
fn bench_device() -> Option<(Device, Queue)> {
    let mut desc = InstanceDescriptor::new_without_display_handle();
    desc.flags = InstanceFlags::from_build_config()
        .with_env()
        .difference(InstanceFlags::VALIDATION_INDIRECT_CALL);
    let instance = Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let caps = adapter.get_downlevel_capabilities();
    println!(
        "adapter: {:?} INDIRECT_EXECUTION={} MULTI_DRAW_INDIRECT_COUNT={}",
        adapter.get_info().backend,
        caps.flags.contains(DownlevelFlags::INDIRECT_EXECUTION),
        adapter
            .features()
            .contains(Features::MULTI_DRAW_INDIRECT_COUNT),
    );
    let desc = DeviceDescriptor {
        required_features: adapter.features() & Features::INDIRECT_FIRST_INSTANCE,
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&desc)).ok()?;
    Some((device, queue))
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// 合成音符：每个 key 内按 start_tick 升序，start = i * step + key 偏移，
/// 长度 1 + i%8 tick，track 在 0..tracks 间轮转（模拟多轨密集黑乐谱）。
///
/// 只用 MIDI 的 128 个 key（shader 纵向硬编码 128 key），
/// 返回 `(notes, per_key_offsets, total_ticks)`。
fn synth_notes(n: usize, step: u32, tracks: u16) -> (Vec<NoteInstance>, [u32; KEY_COUNT + 1], u32) {
    use rayon::prelude::*;
    const MIDI_KEYS: usize = 128;
    let keys = MIDI_KEYS;
    let per_key = n / keys;
    let buckets: Vec<Vec<NoteInstance>> = (0..keys)
        .into_par_iter()
        .map(|k| {
            let mut v = Vec::with_capacity(per_key);
            // key 之间错开，避免所有 key 挤在同一 tick；key 内保持升序。
            let phase = k as u32 % step.max(1);
            for i in 0..per_key as u32 {
                let start = i * step + phase;
                v.push(NoteInstance {
                    start_tick: start,
                    end_tick: start + 1 + i % 8,
                    packed: NoteInstance::pack(k as u8, (i as u16) % tracks.max(1), 100),
                });
            }
            v
        })
        .collect();

    let mut offsets = [0u32; KEY_COUNT + 1];
    let mut all = Vec::with_capacity(per_key * keys);
    let mut total = 0u32;
    for (k, bucket) in buckets.into_iter().enumerate() {
        offsets[k] = total;
        total += bucket.len() as u32;
        all.extend(bucket);
    }
    // 128..KEY_COUNT 的 key 为空：offsets 尾部全部指向 total。
    for v in offsets.iter_mut().skip(keys) {
        *v = total;
    }
    let total_ticks = per_key as u32 * step;
    (all, offsets, total_ticks.max(1))
}

/// 读回各 key 的 draw args，累加 instance_count（可见实例总数）。
/// 只在测量帧之后调用；读回本身不计入帧时间。
fn count_visible_instances(device: &Device, queue: &Queue, cull: &CullState) -> u64 {
    let mut total_bytes = 0u64;
    for key in 0u8..=MAX_KEY {
        let cc = cull.frame_chunk_counts[key as usize] as u64;
        if cc > 0 && cull.per_key_draw_args_buffers[key as usize].is_some() {
            total_bytes += cc * 20;
        }
    }
    if total_bytes == 0 {
        return 0;
    }
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("bench_args_readback"),
        size: total_bytes,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    let mut offset = 0u64;
    for key in 0u8..=MAX_KEY {
        let cc = cull.frame_chunk_counts[key as usize] as u64;
        if cc == 0 {
            continue;
        }
        let Some(src) = &cull.per_key_draw_args_buffers[key as usize] else {
            continue;
        };
        enc.copy_buffer_to_buffer(src, 0, &readback, offset, cc * 20);
        offset += cc * 20;
    }
    queue.submit([enc.finish()]);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done2 = done.clone();
    readback.slice(..).map_async(MapMode::Read, move |_| {
        done2.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    if device.poll(PollType::wait_indefinitely()).is_err() {
        return 0;
    }
    if !done.load(std::sync::atomic::Ordering::SeqCst) {
        return 0;
    }
    let Ok(view) = readback.slice(..).get_mapped_range() else {
        return 0;
    };
    let mut visible = 0u64;
    for chunk in view.chunks_exact(20) {
        visible += u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as u64;
    }
    drop(view);
    readback.unmap();
    visible
}

/// args 数量 → CPU 提交成本曲线（A 大 chunk / B 紧凑输出的收益来源）。
/// 每条 args 的 instance_count=0，GPU 不执行绘制，只测命令编码成本。
fn bench_args_scaling(
    device: &Device,
    queue: &Queue,
    renderer: &crate::InstanceRenderer,
    target_view: &TextureView,
    pw: u32,
    ph: u32,
) {
    let max_args = 390_625u32; // 1e8 / 256
    let mut args_data = vec![0u32; max_args as usize * 5];
    for chunk in args_data.chunks_exact_mut(5) {
        chunk[0] = 6; // index_count；instance_count = 0
    }
    let args_buf = device.create_buffer(&BufferDescriptor {
        label: Some("bench_fake_args"),
        size: max_args as u64 * 20,
        usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&args_buf, 0, bytemuck::cast_slice(&args_data));

    let rs = renderer.render_state();
    let vis = renderer.cull.per_key_visible_buffers[0]
        .as_ref()
        .expect("key0 visible buffer");
    let bg = renderer.cull.per_key_all_bind_group(0).expect("key0 bg");

    println!("\nargs 数量 → CPU 提交成本（multi_draw，instance_count=0）");
    for &n in &[390_625u32, 24_414, 12_207, 3_052, 765, 128] {
        let mut min_cpu = f64::MAX;
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut pass = crate::util::begin_pianoroll_pass(
                    &mut enc,
                    target_view,
                    &rs.pipeline,
                    &rs.bind_group,
                    pw,
                    ph,
                );
                pass.set_pipeline(&rs.note_pipeline);
                pass.set_bind_group(0, &rs.bind_group, &[]);
                pass.set_index_buffer(rs.index_buffer.slice(..), IndexFormat::Uint32);
                pass.set_bind_group(1, bg, &[]);
                pass.set_vertex_buffer(0, vis.slice(..));
                pass.multi_draw_indexed_indirect(&args_buf, 0, n);
            }
            queue.submit([enc.finish()]);
            min_cpu = min_cpu.min(t.elapsed().as_secs_f64() * 1e3);
        }
        println!("{n:>8} args: CPU {min_cpu:.2}ms");
    }
    device
        .poll(PollType::wait_indefinitely())
        .expect("poll failed");
}

#[test]
#[ignore]
fn bench_synthetic_scale() {
    let n = env_usize("YIN_BENCH_NOTES", 100_000_000);
    let step = env_usize("YIN_BENCH_STEP", 64) as u32;
    let frames = env_usize("YIN_BENCH_FRAMES", 3).max(2);
    let tracks = env_usize("YIN_BENCH_TRACKS", 8) as u16;

    println!("== 合成规模基准（现状：无 LOD） ==");
    println!("目标音符 {n}，step {step} tick，tracks {tracks}，每档 {frames} 帧");

    let t = std::time::Instant::now();
    let (all, offsets, total_ticks) = synth_notes(n, step, tracks);
    println!(
        "合成 {} 音符（{:.0}MB CPU）: {:.0}ms，时间轴 {total_ticks} ticks",
        all.len(),
        all.len() as f64 * 12.0 / 1e6,
        t.elapsed().as_secs_f64() * 1e3
    );

    let Some((device, queue)) = bench_device() else {
        eprintln!("无可用 GPU 适配器，跳过");
        return;
    };

    // ── C 方案：LOD 摘要层（生产函数）的段数与构建成本 ──
    let mut summaries: Vec<(Vec<NoteInstance>, [u32; KEY_COUNT + 1])> =
        Vec::with_capacity(crate::pianoroll::SUMMARY_BLOCK_TICKS.len());
    let mut summary_build_ms = 0.0f64;
    for &bt in &crate::pianoroll::SUMMARY_BLOCK_TICKS {
        let t = std::time::Instant::now();
        let (s, so) = crate::pianoroll::build_summary(&all, &offsets, bt);
        summary_build_ms += t.elapsed().as_secs_f64() * 1e3;
        let skipped = s.len() > crate::pianoroll::SUMMARY_MAX_SEGMENTS;
        println!(
            "摘要 block={bt:>6}: {:>9} 段 {:>7.1}MB 构建 {:>4}ms{}",
            s.len(),
            s.len() as f64 * 12.0 / 1e6,
            t.elapsed().as_secs_f64() * 1e3,
            if skipped {
                "（超段数上限，跳过）"
            } else {
                ""
            }
        );
        if skipped {
            summaries.push((Vec::new(), [0u32; KEY_COUNT + 1]));
        } else {
            summaries.push((s, so));
        }
    }
    println!("摘要构建合计 {summary_build_ms:.0}ms（加载时一次，后台线程）");

    let format = TextureFormat::Rgba8UnormSrgb;
    let mut renderer = crate::InstanceRenderer::new(device.clone(), queue.clone(), format);

    let t = std::time::Instant::now();
    renderer.upload_all_notes_for_cull(&all, &offsets, &[0; KEY_COUNT]);
    renderer.upload_summary_for_cull(&summaries);
    println!(
        "全量上传（音符 + 摘要）: {:.0}ms, cull_ready={}",
        t.elapsed().as_secs_f64() * 1e3,
        renderer.cull_ready()
    );
    assert!(renderer.cull_ready(), "cull 未就绪（显存预算失败？）");
    drop(all);

    let colors: Vec<[f32; 4]> = (0..tracks.max(1))
        .map(|i| {
            let f = i as f32 / tracks.max(1) as f32;
            [0.2 + 0.6 * f, 0.5, 1.0 - 0.5 * f, 1.0]
        })
        .collect();
    renderer.upload_track_colors(&colors);
    renderer.upload_selection(&SelectionUniform {
        rects: [[0; 4]; MAX_SEL_RECTS * 2],
    });

    // 清空队列（write_buffer 的拷贝在 submit 时才发生），并测空 submit+poll 开销。
    queue.submit([device.create_command_encoder(&Default::default()).finish()]);
    device
        .poll(PollType::wait_indefinitely())
        .expect("poll failed");
    let mut empty_poll_ms = f64::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        queue.submit([device.create_command_encoder(&Default::default()).finish()]);
        device
            .poll(PollType::wait_indefinitely())
            .expect("poll failed");
        empty_poll_ms = empty_poll_ms.min(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("空 submit+poll 固定开销: {empty_poll_ms:.3}ms");

    let (pw, ph) = (1600u32, 900u32);
    let target = device.create_texture(&TextureDescriptor {
        label: Some("bench_target"),
        size: Extent3d {
            width: pw,
            height: ph,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());

    let kb_w = 80.0f32;
    // 只考虑 MIDI 的 128 key；key_height=7 让 128 个 key 全部落在 900px
    // 视口内，测量最坏的「全 key + 全曲」可见情况。
    let kh = 7.0f32;
    let main_w = pw as f32 - kb_w;

    // 屏上可见 tick 数逐档 ×8：从全曲到放大到 1 tick ≈ 1px 以上。
    let divisors = [1u32, 8, 64, 512, 4096, 32768, 131072];
    println!(
        "\n{:>11} {:>11} {:>11} {:>6} {:>9} {:>9} {:>9} {:>9}",
        "屏上tick", "ppu", "实例/段", "层", "CPU/ms", "GPU/ms", "帧/ms", "FPS"
    );
    for &div in &divisors {
        let tos = (total_ticks / div).max(64);
        let ppu = main_w / tos as f32;
        let scroll_x = ((total_ticks as f32 * ppu - main_w) / 2.0).max(0.0);

        let mut min_cpu = f64::MAX;
        let mut min_gpu = f64::MAX;
        for f in 0..frames {
            let u = Uniforms {
                width: pw as f32,
                height: ph as f32,
                scroll_x: scroll_x + f as f32,
                scroll_y: 0.0,
                pixels_per_tick: ppu,
                key_height: kh,
                keyboard_width: kb_w,
                mode: 1,
                track_count: tracks as u32,
                ..Default::default()
            };
            renderer.upload_uniforms(u);
            let t = std::time::Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            renderer.draw(&mut enc, &target_view, pw, ph);
            queue.submit([enc.finish()]);
            let cpu_ms = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            device
                .poll(PollType::wait_indefinitely())
                .expect("poll failed");
            let gpu_ms = (t.elapsed().as_secs_f64() * 1e3 - empty_poll_ms).max(0.0);
            if f > 0 {
                min_cpu = min_cpu.min(cpu_ms);
                min_gpu = min_gpu.min(gpu_ms);
            }
        }
        // 摘要层生效时，原始层 args 不含本帧内容；直接显示摘要段数。
        let active_level = crate::pianoroll::select_summary_level(ppu).and_then(|best| {
            (best..crate::pianoroll::SUMMARY_BLOCK_TICKS.len())
                .find(|&level| renderer.cull.summary_level_ready(level))
        });
        let visible = match active_level {
            Some(level) => renderer.cull.summary_levels[level]
                .per_key_counts
                .iter()
                .map(|&c| c as u64)
                .sum(),
            None => count_visible_instances(&device, &queue, &renderer.cull),
        };
        let summary_tag = match active_level {
            Some(level) => format!("L{level}段"),
            None => String::new(),
        };
        let frame_ms = min_cpu.max(min_gpu);
        println!(
            "{:>11} {:>11.5} {:>11} {:>6} {:>9.2} {:>9.2} {:>9.2} {:>9.1}",
            tos,
            ppu,
            visible,
            summary_tag,
            min_cpu,
            min_gpu,
            frame_ms,
            1000.0 / frame_ms.max(0.001)
        );
    }

    // ── 诊断：全曲视图但只显示 1 个 key（绘制量≈0，buffer 规模不变）──
    // 区分 CPU 帧时间是与「绘制规模」相关还是与「buffer 总规模 / 提交」相关。
    {
        let u = Uniforms {
            width: pw as f32,
            height: ph as f32,
            scroll_x: 0.0,
            scroll_y: 0.0,
            pixels_per_tick: main_w / total_ticks as f32,
            key_height: 1000.0, // 纵向只覆盖 1 个 key
            keyboard_width: kb_w,
            mode: 1,
            track_count: tracks as u32,
            ..Default::default()
        };
        renderer.upload_uniforms(u);
        let mut min_cpu = f64::MAX;
        for f in 0..frames.max(2) {
            let mut u2 = u;
            u2.scroll_x = f as f32;
            renderer.upload_uniforms(u2);
            let t = std::time::Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            renderer.draw(&mut enc, &target_view, pw, ph);
            queue.submit([enc.finish()]);
            let cpu_ms = t.elapsed().as_secs_f64() * 1e3;
            device
                .poll(PollType::wait_indefinitely())
                .expect("poll failed");
            if f > 0 {
                min_cpu = min_cpu.min(cpu_ms);
            }
        }
        let visible = count_visible_instances(&device, &queue, &renderer.cull);
        println!("诊断（全曲视图 / 1 key，可见 {visible}）: CPU {min_cpu:.2}ms");
    }

    // 静止帧（uniforms 不变 → cull dispatch skip）：测量纯 draw 录制 + 上帧 args 的绘制。
    let u = Uniforms {
        width: pw as f32,
        height: ph as f32,
        scroll_x: 0.0,
        scroll_y: 0.0,
        pixels_per_tick: main_w / total_ticks as f32,
        key_height: kh,
        keyboard_width: kb_w,
        mode: 1,
        track_count: tracks as u32,
        ..Default::default()
    };
    renderer.upload_uniforms(u);
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw(&mut enc, &target_view, pw, ph);
    queue.submit([enc.finish()]);
    device
        .poll(PollType::wait_indefinitely())
        .expect("poll failed");
    let t = std::time::Instant::now();
    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw(&mut enc, &target_view, pw, ph);
    queue.submit([enc.finish()]);
    let idle_cpu = t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    device
        .poll(PollType::wait_indefinitely())
        .expect("poll failed");
    let idle_gpu = (t.elapsed().as_secs_f64() * 1e3 - empty_poll_ms).max(0.0);
    println!("\n静止帧（全曲视图，cull skip）: CPU {idle_cpu:.2}ms, GPU {idle_gpu:.2}ms");

    // ── A/B 方案：args 数量 → CPU 提交成本（大 chunk / 紧凑输出）──
    bench_args_scaling(&device, &queue, &renderer, &target_view, pw, ph);
}

/// 真实 MIDI 端到端基准：解析 → 构建音符 + 摘要 → 上传 → 多缩放档位
/// 实测 CPU/GPU 帧时间。用生产渲染路径（含 LOD 摘要层自动选档）。
///
/// 运行：
///   YIN_BENCH_MIDI=/path/to.mid cargo test -p yinhe-wgpu --release \
///     bench_real_midi -- --ignored --nocapture
#[test]
#[ignore]
fn bench_real_midi() {
    let path = std::env::var("YIN_BENCH_MIDI")
        .unwrap_or_else(|_| "/Users/jieneng/Music/MIDIs/ReptilianDarkRitual.mid".into());
    let t = std::time::Instant::now();
    let model = yinhe_midi::parse_path(&path).expect("解析 MIDI 失败");
    println!(
        "解析: {:.0}ms, {} 音符, 总长 {} tick, {} 轨道",
        t.elapsed().as_secs_f64() * 1e3,
        model.note_count,
        model.tick_length,
        model.tracks.len()
    );

    let hidden = std::collections::HashSet::new();
    let track_visible: Vec<bool> = vec![true; model.tracks.len()];
    let t = std::time::Instant::now();
    let (all, offsets) = crate::pianoroll::build_all_notes(&model, &hidden, &track_visible);
    println!(
        "build_all_notes: {:.0}ms, {} 音符 ({:.0}MB)",
        t.elapsed().as_secs_f64() * 1e3,
        all.len(),
        all.len() as f64 * 12.0 / 1e6
    );

    let mut summaries: Vec<(Vec<NoteInstance>, [u32; KEY_COUNT + 1])> =
        Vec::with_capacity(crate::pianoroll::SUMMARY_BLOCK_TICKS.len());
    for &bt in &crate::pianoroll::SUMMARY_BLOCK_TICKS {
        let t = std::time::Instant::now();
        let (s, so) = crate::pianoroll::build_summary(&all, &offsets, bt);
        let skipped = s.len() > crate::pianoroll::SUMMARY_MAX_SEGMENTS;
        println!(
            "摘要 block={bt:>6}: {:>9} 段 {:>7.1}MB 构建 {:>5}ms{}",
            s.len(),
            s.len() as f64 * 12.0 / 1e6,
            t.elapsed().as_secs_f64() * 1e3,
            if skipped { "（超上限跳过）" } else { "" }
        );
        if skipped {
            summaries.push((Vec::new(), [0u32; KEY_COUNT + 1]));
        } else {
            summaries.push((s, so));
        }
    }

    let Some((device, queue)) = bench_device() else {
        eprintln!("无可用 GPU 适配器，跳过");
        return;
    };
    let format = TextureFormat::Rgba8UnormSrgb;
    let mut renderer = crate::InstanceRenderer::new(device.clone(), queue.clone(), format);
    let t = std::time::Instant::now();
    renderer.upload_all_notes_for_cull(&all, &offsets, &[0; KEY_COUNT]);
    renderer.upload_summary_for_cull(&summaries);
    println!(
        "全量上传（音符 + 摘要）: {:.0}ms, cull_ready={}",
        t.elapsed().as_secs_f64() * 1e3,
        renderer.cull_ready()
    );
    drop(all);

    let colors: Vec<[f32; 4]> = (0..model.tracks.len().max(1))
        .map(|i| {
            let f = (i % 16) as f32 / 16.0;
            [0.2 + 0.6 * f, 0.5, 1.0 - 0.5 * f, 1.0]
        })
        .collect();
    renderer.upload_track_colors(&colors);
    renderer.upload_selection(&SelectionUniform {
        rects: [[0; 4]; MAX_SEL_RECTS * 2],
    });

    queue.submit([device.create_command_encoder(&Default::default()).finish()]);
    device
        .poll(PollType::wait_indefinitely())
        .expect("poll failed");
    let mut empty_poll_ms = f64::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        queue.submit([device.create_command_encoder(&Default::default()).finish()]);
        device
            .poll(PollType::wait_indefinitely())
            .expect("poll failed");
        empty_poll_ms = empty_poll_ms.min(t.elapsed().as_secs_f64() * 1e3);
    }

    let (pw, ph) = (1520u32, 900u32);
    let target = device.create_texture(&TextureDescriptor {
        label: Some("real_midi_target"),
        size: Extent3d {
            width: pw,
            height: ph,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());

    let kb_w = 80.0f32;
    let kh = 7.0f32;
    let main_w = pw as f32 - kb_w;
    let total_ticks = model.tick_length.max(1);
    let frames = env_usize("YIN_BENCH_FRAMES", 3).max(2);
    println!(
        "\n{:>11} {:>11} {:>11} {:>6} {:>9} {:>9} {:>9} {:>9}",
        "屏上tick", "ppu", "实例/段", "层", "CPU/ms", "GPU/ms", "帧/ms", "FPS"
    );
    for &div in &[1u32, 4, 16, 32, 64, 256, 1024, 4096, 16384] {
        let tos = (total_ticks / u64::from(div)).max(64);
        let ppu = main_w / tos as f32;
        let scroll_x = ((total_ticks as f32 * ppu - main_w) / 2.0).max(0.0);
        let mut min_cpu = f64::MAX;
        let mut min_gpu = f64::MAX;
        for f in 0..frames {
            let u = Uniforms {
                width: pw as f32,
                height: ph as f32,
                scroll_x: scroll_x + f as f32,
                scroll_y: 0.0,
                pixels_per_tick: ppu,
                key_height: kh,
                keyboard_width: kb_w,
                mode: 1,
                track_count: colors.len() as u32,
                ..Default::default()
            };
            renderer.upload_uniforms(u);
            let t = std::time::Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            renderer.draw(&mut enc, &target_view, pw, ph);
            queue.submit([enc.finish()]);
            let cpu_ms = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            device
                .poll(PollType::wait_indefinitely())
                .expect("poll failed");
            let gpu_ms = (t.elapsed().as_secs_f64() * 1e3 - empty_poll_ms).max(0.0);
            if f > 0 {
                min_cpu = min_cpu.min(cpu_ms);
                min_gpu = min_gpu.min(gpu_ms);
            }
        }
        let active_level = crate::pianoroll::select_summary_level(ppu).and_then(|best| {
            (best..crate::pianoroll::SUMMARY_BLOCK_TICKS.len())
                .find(|&level| renderer.cull.summary_level_ready(level))
        });
        let (visible, tag) = match active_level {
            Some(level) => (
                renderer.cull.summary_levels[level]
                    .per_key_counts
                    .iter()
                    .map(|&c| c as u64)
                    .sum(),
                format!("L{level}"),
            ),
            None => (
                count_visible_instances(&device, &queue, &renderer.cull),
                String::new(),
            ),
        };
        let frame_ms = min_cpu.max(min_gpu);
        println!(
            "{:>11} {:>11.5} {:>11} {:>6} {:>9.2} {:>9.2} {:>9.2} {:>9.1}",
            tos,
            ppu,
            visible,
            tag,
            min_cpu,
            min_gpu,
            frame_ms,
            1000.0 / frame_ms.max(0.001)
        );
    }

    // ── AM 力度条 LOD：摘要构建 + 各缩放档位的切片构建成本 ──
    use yinhe_types::{AutomationPanelView, TimelineViewBase};
    let track_visible_all = vec![true; model.tracks.len()];
    let t = std::time::Instant::now();
    let vsum = crate::automation::VelocitySummary::build(&model, &track_visible_all);
    println!(
        "\n力度条摘要构建（全量音符）: {:.0}ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    println!(
        "{:>11} {:>11} {:>11} {:>6} {:>10}",
        "屏上tick", "ppu", "bars", "档", "构建/ms"
    );
    for &div in &[1u32, 4, 16, 32, 64, 256, 1024] {
        let tos = (total_ticks / u64::from(div)).max(64);
        let ppu = main_w / tos as f32;
        let scroll_x = ((total_ticks as f32 * ppu - main_w) / 2.0).max(0.0);
        let view = AutomationPanelView {
            base: TimelineViewBase {
                pixels_per_tick: ppu,
                scroll_x,
                scroll_y: 0.0,
                left_panel_width: kb_w,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            ..Default::default()
        };
        let level = vsum.level_for_ppu(ppu);
        let mut bars = Vec::new();
        let mut min_ms = f64::MAX;
        for _ in 0..3 {
            bars.clear();
            let t = std::time::Instant::now();
            crate::automation::velocity_bars::build_velocity_bars(
                &mut bars,
                main_w,
                &model,
                &view,
                &track_visible_all,
                Some(&vsum),
            );
            min_ms = min_ms.min(t.elapsed().as_secs_f64() * 1e3);
        }
        println!(
            "{:>11} {:>11.5} {:>11} {:>6} {:>10.2}",
            tos,
            ppu,
            bars.len(),
            level.map(|l| format!("L{l}")).unwrap_or_default(),
            min_ms
        );
    }
}
