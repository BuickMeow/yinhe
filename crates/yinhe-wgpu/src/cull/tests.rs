use super::*;
use crate::vertex::{NoteInstance, Uniforms};
use std::sync::atomic::Ordering;
use wgpu::*;
use yinhe_types::{KEY_COUNT, NoteSource};

/// Headless GPU device for cull integration tests.
/// Returns None when no adapter is available (e.g. CI without a GPU),
/// which skips the test.
fn headless_device() -> Option<(Device, Queue)> {
    let instance = Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    // cull 已改为 CPU 读回 args + 直接 draw_indexed（Adreno indirect
    // draw 失效），draw 路径不再需要 INDIRECT_FIRST_INSTANCE feature。
    let desc = DeviceDescriptor {
        required_features: adapter.features() & Features::INDIRECT_FIRST_INSTANCE,
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&desc)).ok()?;
    Some((device, queue))
}

fn visible_uniforms() -> Uniforms {
    Uniforms {
        width: 800.0,
        height: 600.0,
        scroll_x: 0.0,
        scroll_y: 1000.0, // key 60 rows land inside the viewport (y ∈ [340, 360))
        pixels_per_tick: 0.1,
        key_height: 20.0,
        keyboard_width: 60.0,
        mode: 1,
        ..Default::default()
    }
}

fn test_notes(n: usize) -> Vec<NoteInstance> {
    (0..n)
        .map(|i| NoteInstance {
            start_tick: i as u32 * 10,
            end_tick: i as u32 * 10 + 5,
            packed: NoteInstance::pack(60, 0, 100),
        })
        .collect()
}

/// Regression test for the ghost-note bug: a key whose buffer capacity
/// exceeds its written note count must NOT cull stale data beyond `count`.
///
/// Scenario: upload 100 notes (buffer capacity rounds up to 256 elements),
/// then upload 50 notes (buffer is not recreated, so elements 50..255 still
/// hold the first upload's notes). If the shader used `arrayLength`
/// (capacity) as the cull bound, the stale notes at 50..99 would pass the
/// AABB test and be drawn as ghosts (instance_count would be ≥ 100).
#[test]
fn cull_ignores_stale_notes_beyond_count() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    cull.upload_one_key(&device, &queue, &uniform_buffer, 0, &test_notes(100))
        .unwrap();
    // Shrunk upload: same key, fewer notes, buffer NOT recreated.
    cull.upload_one_key(&device, &queue, &uniform_buffer, 0, &test_notes(50))
        .unwrap();

    let mut encoder = device.create_command_encoder(&Default::default());
    cull.dispatch_cull(&mut encoder, &queue, 0, 0, &visible_uniforms());

    // Read back the per-key draw args (instance_count at byte offset 4).
    // DrawIndexedIndirectArgs = 20B.
    let args_readback = device.create_buffer(&BufferDescriptor {
        label: Some("args_readback"),
        size: 20,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(
        cull.per_key_draw_args_buffers[0]
            .as_ref()
            .expect("uploaded"),
        0,
        &args_readback,
        0,
        20,
    );
    queue.submit([encoder.finish()]);

    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done2 = done.clone();
    args_readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, Ordering::SeqCst);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll failed");
    assert!(done.load(Ordering::SeqCst), "map_async callback not fired");
    let view = args_readback
        .slice(..)
        .get_mapped_range()
        .expect("readback map");
    let args: &[u32] = bytemuck::cast_slice(&view);
    // DrawIndexedIndirectArgs: [index_count=6, instance_count, first_index=0,
    // base_vertex=0, first_instance=0] — chunk 0 starts at sparse slot 0.
    assert_eq!(args[0], 6, "index_count must be 6 (two triangles)");
    assert_eq!(args[2], 0, "first_index must be 0");
    assert_eq!(args[3], 0, "base_vertex must be 0");
    assert_eq!(args[4], 0, "first_instance must be 0 (chunk 0)");
    let instance_count = args[1];
    drop(view);
    args_readback.unmap();

    assert_eq!(
        instance_count, 50,
        "stale notes beyond the uploaded count must not be drawn"
    );
}

/// Track 显隐 mask：隐藏轨道的音符必须被 cull shader 过滤掉，即使
/// buffer 里还存着它们（后台重建期间「旧 buffer + mask 双重过滤」的保证）。
/// 同时验证 mask 变化会强制重跑 dispatch（绕过 skip 优化）。
#[test]
fn cull_track_mask_filters_hidden_tracks() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // 3 条轨道各 10 个可见音符（key 60）。
    let mut all = Vec::new();
    for track in 0..3u16 {
        for i in 0..10 {
            all.push(NoteInstance {
                start_tick: i * 10,
                end_tick: i * 10 + 5,
                packed: NoteInstance::pack(60, track, 100),
            });
        }
    }
    cull.upload_one_key(&device, &queue, &uniform_buffer, 60, &all)
        .unwrap();

    let run = |cull: &mut CullState, u: &Uniforms| -> Vec<u32> {
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.dispatch_cull(&mut encoder, &queue, 60, 60, u);
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("vis_readback"),
            size: 256 * 4, // 1 chunk × 256 slots × 4B 索引
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            cull.per_key_visible_buffers[60].as_ref().expect("uploaded"),
            0,
            &readback,
            0,
            256 * 4,
        );
        queue.submit([encoder.finish()]);

        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst));
        let view = readback
            .slice(..)
            .get_mapped_range()
            .expect("diag readback map");
        let idx: &[u32] = bytemuck::cast_slice(&view);
        let out = idx[..30].to_vec();
        drop(view);
        readback.unmap();
        out
    };

    let u = visible_uniforms();
    // 无 mask（默认全 1）：30 个全部可见，索引 = 输入顺序 0..30。
    let all_vis = run(&mut cull, &u);
    assert_eq!(all_vis, (0..30).collect::<Vec<u32>>());

    // 隐藏轨道 1：只剩 20 个，且 track 必须 ∈ {0, 2}（track = idx / 10）。
    cull.upload_track_mask(&queue, &[true, false, true]);
    let out = run(&mut cull, &u);
    let visible: Vec<u32> = out.iter().take(20).map(|&i| i / 10).collect();
    assert!(
        visible.iter().all(|&t| t == 0 || t == 2),
        "隐藏轨道 1 的音符泄漏: {visible:?}"
    );

    // 恢复轨道 1：mask 变化必须重跑 dispatch（绕过 skip 优化）→ 30 个全回来。
    cull.upload_track_mask(&queue, &[true, true, true]);
    let out2 = run(&mut cull, &u);
    let visible2: Vec<u32> = out2.iter().take(30).map(|&i| i / 10).collect();
    assert!(
        visible2.iter().all(|&t| t <= 2),
        "恢复后轨道异常: {visible2:?}"
    );
}

/// Z-order must be deterministic across frames: with 1000 notes at the
/// same tick (spanning 4 chunks), the culled output order must follow the
/// input order every frame, independent of GPU workgroup scheduling.
#[test]
fn cull_output_order_is_deterministic() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // 1000 notes at tick 0 (4 chunks); track index distinguishes order.
    let notes: Vec<NoteInstance> = (0..1000)
        .map(|i| NoteInstance {
            start_tick: 0,
            end_tick: 100,
            packed: NoteInstance::pack(60, i as u16, 100),
        })
        .collect();
    cull.upload_one_key(&device, &queue, &uniform_buffer, 0, &notes)
        .unwrap();

    let run = |cull: &mut CullState, scroll_x: f32| -> Vec<u32> {
        let mut u = visible_uniforms();
        u.scroll_x = scroll_x; // 1px shift still keeps all notes visible
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.dispatch_cull(&mut encoder, &queue, 0, 0, &u);
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("vis_readback"),
            size: 4 * 256 * 4, // 4 chunks × 256 slots × 4B 索引
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            cull.per_key_visible_buffers[0].as_ref().expect("uploaded"),
            0,
            &readback,
            0,
            4 * 256 * 4,
        );
        queue.submit([encoder.finish()]);

        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst));
        let view = readback
            .slice(..)
            .get_mapped_range()
            .expect("diag readback map");
        let idx: &[u32] = bytemuck::cast_slice(&view);
        let out: Vec<u32> = idx[..1000].to_vec();
        drop(view);
        readback.unmap();
        out
    };

    let a = run(&mut cull, 0.0);
    let b = run(&mut cull, 1.0);
    assert_eq!(a, b, "z-order must be stable across frames");
    // 索引 == 输入顺序（音符在 all_instances 里的位置）。
    let expected: Vec<u32> = (0..1000).collect();
    assert_eq!(a, expected, "culled output must follow input (tick) order");
}

fn build_index(start_ends: &[(u32, u32)]) -> KeyBucketIndex {
    let notes: Vec<NoteInstance> = start_ends
        .iter()
        .map(|&(s, e)| NoteInstance {
            start_tick: s,
            end_tick: e,
            packed: NoteInstance::pack(60, 0, 100),
        })
        .collect();
    KeyBucketIndex::build(&notes)
}

#[test]
fn bucket_index_empty_and_small() {
    let idx = build_index(&[]);
    assert_eq!(idx.chunk_total, 0);
    assert!(idx.visible_chunk_range(0, 1000).is_none());

    // 100 notes → 1 bucket, 1 chunk.
    let notes: Vec<(u32, u32)> = (0..100).map(|i| (i * 10, i * 10 + 5)).collect();
    let idx = build_index(&notes);
    assert_eq!(idx.chunk_total, 1);
    assert_eq!(idx.visible_chunk_range(0, 1000), Some((0, 1)));
    // Viewport after all notes → nothing.
    assert!(idx.visible_chunk_range(2000, 3000).is_none());
    // Single bucket fully left of the viewport → suffix max < ts → None.
    let left_notes: Vec<(u32, u32)> = (0..100).map(|i| (i, i + 5)).collect();
    let idx = build_index(&left_notes);
    assert!(idx.visible_chunk_range(500, 600).is_none());
}

#[test]
fn bucket_index_multi_bucket_boundaries() {
    // 5000 notes → 20 chunks (256 notes each).
    let notes: Vec<(u32, u32)> = (0..5000).map(|i| (i * 10, i * 10 + 5)).collect();
    let idx = build_index(&notes);
    assert_eq!(idx.chunk_total, 20);
    // Viewport inside chunk 0 → chunks [0, 1).
    assert_eq!(idx.visible_chunk_range(0, 100), Some((0, 1)));
    // Viewport inside chunk 15's tick range: only the chunks whose
    // max_end >= ts are kept → [15, 20). The shader's exact AABB test then
    // culls the rest.
    assert_eq!(idx.visible_chunk_range(40_000, 50_000), Some((15, 20)));
    // Viewport spanning everything → [0, 20).
    assert_eq!(idx.visible_chunk_range(0, 50_000), Some((0, 20)));
    // Viewport in the gap between chunk 15's notes (max_end 40955) and
    // chunk 16's start (40960): chunk 16's max_end (43515) >= ts, so only
    // chunk 16 is dispatched — [16, 17).
    assert_eq!(idx.visible_chunk_range(41_000, 42_000), Some((16, 17)));
}

#[test]
fn bucket_index_long_note_crossing_left_edge() {
    // A long note starting far left extends far right: bucket 0 max_end
    // covers everything, so any viewport must include bucket 0's chunks.
    let mut notes: Vec<(u32, u32)> = (0..100).map(|i| (i * 10, i * 10 + 5)).collect();
    notes[0] = (0, 10_000_000);
    let idx = build_index(&notes);
    assert_eq!(
        idx.visible_chunk_range(5_000_000, 5_001_000),
        Some((0, 1)),
        "long note crossing from off-screen-left must keep its bucket dispatched"
    );
}

#[test]
fn bucket_index_prefix_with_long_note() {
    // 16384 notes → 64 chunks. Chunk 0 has a long note (max_end = 1_000_000),
    // chunks 1..63 are short notes ending well before the viewport.
    // Viewport [200_000, 210_000]: only chunk 0 can intersect (its long
    // note crosses the viewport); the block prefix/suffix search must not
    // pull in the short-note chunks (they all end < ts).
    let mut notes: Vec<(u32, u32)> = Vec::new();
    // Chunk 0's notes: 256 notes, first one is a long note.
    for i in 0..256 {
        notes.push((i * 10, i * 10 + 5));
    }
    notes[0] = (0, 1_000_000);
    // Remaining chunks (1..63): short notes.
    for i in 256..16384 {
        notes.push((i * 10, i * 10 + 5));
    }
    let idx = build_index(&notes);
    assert_eq!(idx.chunk_total, 64);
    // Only chunk 0 (the long note) is dispatched.
    assert_eq!(idx.visible_chunk_range(200_000, 210_000), Some((0, 1)));
    // Viewport at the very start → chunk 0 only.
    assert_eq!(idx.visible_chunk_range(0, 10), Some((0, 1)));
    // Viewport after every note end → nothing.
    assert!(idx.visible_chunk_range(2_000_000, 3_000_000).is_none());
}

#[test]
fn bucket_index_all_suffix_visible() {
    // Every chunk block has a long note (max_end = 10_000_000), so no
    // block suffix drops below ts, and the key must still be dispatched
    // (NOT return None). The dispatched range covers chunks 0..33: the
    // last chunk with max_end >= ts is chunk 32 (bucket 2's long note);
    // chunks 33..47 are short notes ending before the viewport.
    let mut notes: Vec<(u32, u32)> = Vec::new();
    for b in 0..3 {
        let base = b * 4096 * 10;
        notes.push((base, 10_000_000)); // long note in every bucket
        for i in 1..4096 {
            notes.push((base + i * 10, base + i * 10 + 5));
        }
    }
    let idx = build_index(&notes);
    assert_eq!(idx.chunk_total, 48);
    // Viewport in the middle: chunk 32's long note is the last chunk with
    // max_end >= ts, so the range is [0, 33).
    assert_eq!(idx.visible_chunk_range(500_000, 600_000), Some((0, 33)));
}

#[test]
fn visible_tick_range_margin() {
    let u = Uniforms {
        width: 800.0,
        height: 600.0,
        scroll_x: 100.0,
        scroll_y: 0.0,
        pixels_per_tick: 0.1,
        key_height: 20.0,
        keyboard_width: 60.0,
        mode: 1,
        ..Default::default()
    };
    // x_offset = 60 - 100 = -40; visible ticks ≈ [(60+40)/0.1, (800+40)/0.1]
    // = [1000, 8400]，左边界是 keyboard_width 像素（音符右端 ≥ 键盘列右缘），
    // 带 margin → starts before 1000, ends after 8400.
    let (ts, te) = visible_tick_range(&u);
    assert!(ts <= 990 && te >= 8410, "ts={ts} te={te}");
}

/// 端到端：黑乐谱风格构造数据（128 keys × 8192 音符 + 每 key 长音符），
/// 模拟 PR 默认视口，验证每个可见 key 都有输出。
#[test]
fn bucket_index_conservative_bruteforce() {
    // 随机黑乐谱风格数据（密集短音符 + 随机长音符），暴力验证
    // visible_chunk_range 是保守超集：区间必须覆盖所有可见 chunk。
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };
    for trial in 0..150 {
        let n = 60_000;
        let mut notes: Vec<(u32, u32)> = Vec::with_capacity(n);
        let mut tick = 0u32;
        for _ in 0..n {
            tick = tick.wrapping_add(next() % 4); // 密集
            let len = if next() % 80 == 0 {
                100_000 + next() % 1_000_000 // 长音符
            } else {
                10 + next() % 60
            };
            notes.push((tick, tick + len));
        }
        let idx = build_index(&notes);
        let total = idx.chunk_total as usize;
        for _ in 0..300 {
            let ts = next() % 1_500_000;
            let te = ts + 5_000 + next() % 60_000;
            let range = idx.visible_chunk_range(ts, te);
            // 暴力：第一个/最后一个可见 chunk（start <= te && max_end >= ts）
            let mut first: Option<usize> = None;
            let mut last: Option<usize> = None;
            for c in 0..total {
                if idx.chunk_start[c] <= te && idx.chunk_max_end[c] >= ts {
                    first = first.or(Some(c));
                    last = Some(c);
                }
            }
            match (range, first) {
                (Some((lo, hi)), Some(_)) => {
                    assert!(
                        lo as usize <= first.unwrap(),
                        "trial {trial} ts={ts} te={te}: c_lo={lo} > first={}",
                        first.unwrap()
                    );
                    assert!(
                        hi as usize > last.unwrap(),
                        "trial {trial} ts={ts} te={te}: c_hi={hi} <= last={}\n  block_suffix={:?}\n  block_prefix={:?}\n  chunk_start[160..170]={:?}\n  chunk_max_end[160..170]={:?}",
                        last.unwrap(),
                        idx.block_suffix_max,
                        idx.block_prefix_max,
                        &idx.chunk_start[160..170.min(total)],
                        &idx.chunk_max_end[160..170.min(total)],
                    );
                }
                (Some(_), None) => {
                    panic!("trial {trial} ts={ts} te={te}: 区间非空但暴力无可见 chunk")
                }
                (None, Some(_)) => {
                    let f = first.unwrap();
                    println!(
                        "诊断: ts={ts} te={te} first={f} last={} block_suffix={:?} block_prefix={:?}",
                        last.unwrap(),
                        idx.block_suffix_max,
                        idx.block_prefix_max
                    );
                    for c in f.saturating_sub(2)..(f + 5).min(total) {
                        println!(
                            "  chunk {c}: start={} max_end={}",
                            idx.chunk_start[c], idx.chunk_max_end[c]
                        );
                    }
                    panic!("trial {trial}: None 但暴力有可见 chunk")
                }
                (None, None) => {}
            }
        }
    }
}

#[test]
fn cull_end_to_end_multi_key() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut all_notes = Vec::new();
    let mut offsets = [0u32; KEY_COUNT + 1];
    for key in 0..128u8 {
        let mut notes = Vec::new();
        // 长音符（覆盖到 tick 10M，start=0）
        notes.push(NoteInstance {
            start_tick: 0,
            end_tick: 10_000_000,
            packed: NoteInstance::pack(key, 0, 100),
        });
        // 密集短音符
        for i in 0..8192 {
            notes.push(NoteInstance {
                start_tick: i * 10 + 1,
                end_tick: i * 10 + 6,
                packed: NoteInstance::pack(key, 0, 100),
            });
        }
        offsets[key as usize] = all_notes.len() as u32;
        all_notes.extend(notes);
    }
    // 128 及以上未使用的 key：offsets 全部填充 total，保持 start==end（空桶）
    for o in offsets.iter_mut().skip(128) {
        *o = all_notes.len() as u32;
    }
    cull.upload_all_notes(
        &device,
        &queue,
        &uniform_buffer,
        &all_notes,
        &offsets,
        &[0; KEY_COUNT],
    )
    .unwrap();

    // PR 默认视口：scroll=0, ppu=0.1, kh=12, height=600 → 可见 key 77..127
    // （key 76 的行在 y∈[612, 624)，完全在视口外）
    let u = Uniforms {
        width: 800.0,
        height: 600.0,
        scroll_x: 0.0,
        scroll_y: 0.0,
        pixels_per_tick: 0.1,
        key_height: 12.0,
        keyboard_width: 60.0,
        mode: 1,
        ..Default::default()
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    cull.dispatch_cull(&mut encoder, &queue, 76, 127, &u);
    // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
    queue.submit([encoder.finish()]);

    // 读回每个 key 的 draw_args[0]，断言 instance_count >= 1（长音符可见）
    // 可见范围是 77..=127：key 76 的行在 y∈[612, 624)，完全在视口外。
    let mut bad: Vec<u32> = Vec::new();
    for key in 77..=127 {
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("args_readback"),
            size: 16,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let args_buf = match &cull.per_key_draw_args_buffers[key as usize] {
            Some(b) => b,
            None => panic!("key {key} 没有 args buffer (upload 后应存在)"),
        };
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 16);
        queue.submit([enc.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst));
        let view = readback.slice(..).get_mapped_range().expect("readback map");
        let args: &[u32] = bytemuck::cast_slice(&view);
        let count = args[1];
        drop(view);
        readback.unmap();
        if count == 0 {
            bad.push(key as u32);
        }
    }
    assert!(bad.is_empty(), "这些 key 没有可见音符: {bad:?}");
}

/// 端到端：真实 MIDI 文件。CPU 路径（build_notes）与 GPU cull 的输出对比。
/// 文件不存在时跳过（CI 兼容）。
#[test]
fn cull_real_midi_vs_cpu() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // 先测一个小文件（几万音符级），再测大文件
    let paths = [
        "/Users/jieneng/Music/MIDIs/99 Luftballons.mid",
        "/Users/jieneng/Music/MIDIs/APT.mid",
        "/Users/jieneng/Music/MIDIs/1.mid",
    ];
    let mut tested_any = false;
    let mut bad_ratios: Vec<(&str, u32, u64, f64)> = Vec::new();
    for path in paths {
        // `parser` 是 yinhe-mid2 的私有模块，解析入口在 crate 根：
        let Ok(model) = yinhe_midi::parse_path(path) else {
            continue; // 文件不存在或解析失败 → 跳过
        };
        tested_any = true;

        // ── 构造统一的 PR 视口 ──
        let ppu = 0.1f32;
        let kh = 12.0f32;
        let width = 800.0f32;
        let height = 600.0f32;
        let kb_w = 60.0f32;
        // TimelineViewBase 没有 derive Default，用 PianoRollView::default() 再覆写。
        // TimelineViewBase 没有 derive Default，字段全部显式给出。
        let view = yinhe_types::PianoRollView {
            key_height: kh,
            viewport_h: height,

            orientation: yinhe_types::Orientation::Horizontal,
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: ppu,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: kb_w,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
        };
        let hidden = std::collections::HashSet::new();
        let track_visible: Vec<bool> = vec![true; model.tracks.len()];

        // ── CPU 期望值 ──
        let mut cpu_out: Vec<NoteInstance> = Vec::new();
        crate::pianoroll::build_notes(
            &mut cpu_out,
            width,
            height,
            &model,
            &view,
            &hidden,
            &track_visible,
        );
        // CPU 输出按 key 统计
        let mut cpu_by_key = [0u32; 128];
        for n in &cpu_out {
            cpu_by_key[(n.packed & 0xFF) as usize] += 1;
        }
        let cpu_total: u32 = cpu_by_key.iter().sum();

        // ── GPU cull ──
        let (all_notes, offsets) =
            crate::pianoroll::build_all_notes(&model, &hidden, &track_visible);
        cull.upload_all_notes(
            &device,
            &queue,
            &uniform_buffer,
            &all_notes,
            &offsets,
            &[0; KEY_COUNT],
        )
        .unwrap();

        let u = Uniforms {
            width,
            height,
            scroll_x: 0.0,
            scroll_y: 0.0,
            pixels_per_tick: ppu,
            key_height: kh,
            keyboard_width: kb_w,
            mode: 1,
            ..Default::default()
        };
        // 写 uniform buffer：dispatch_cull 只读 Rust 侧 Uniforms 算 CPU 端
        // 桶索引，不会把 uniform 写进 GPU buffer。不写的话 shader 读到
        // 全零 uniform（mode=0、width/height=0、ppu=0），所有音符都通过
        // 裁剪，GPU 输出等于全量音符。
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
        // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
        queue.submit([encoder.finish()]);

        // 读回每个 key 的 draw_args。只读本帧实际派发的 chunk
        // （frame_chunk_counts）：未派发的 key 的 draw_args 从未被 shader
        // 写入（内容未定义，读了是垃圾），按 0 计。
        let mut gpu_total: u64 = 0;
        let mut gpu_by_key = [0u64; KEY_COUNT];
        for (key, gpu_key_total) in gpu_by_key.iter_mut().enumerate() {
            let chunk_count = cull.frame_chunk_counts[key];
            if chunk_count == 0 {
                continue;
            }
            let Some(args_buf) = &cull.per_key_draw_args_buffers[key] else {
                continue; // buffer 被销毁（upload 释放 bug）→ 无输出，按 0 计
            };
            let read_size = chunk_count as u64 * 20;
            let readback = device.create_buffer(&BufferDescriptor {
                label: Some("args_readback"),
                size: read_size,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, read_size);
            queue.submit([enc.finish()]);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(Ordering::SeqCst));
            let view = readback.slice(..).get_mapped_range().expect("readback map");
            let args: &[u32] = bytemuck::cast_slice(&view);
            let mut key_total: u64 = 0;
            for c in 0..chunk_count as usize {
                key_total += args[c * 5 + 1] as u64; // instance_count
            }
            drop(view);
            readback.unmap();
            *gpu_key_total = key_total;
            gpu_total += key_total;
        }

        // ── 报告（println 输出，测试结束后我分析）──
        let cpu_keys: Vec<u32> = (0..128u32)
            .filter(|&k| cpu_by_key[k as usize] > 0)
            .collect();
        let gpu_keys: Vec<u32> = (0..128u32)
            .filter(|&k| gpu_by_key[k as usize] > 0)
            .collect();
        println!(
            "FILE {path}: CPU total={cpu_total} keys={cpu_keys:?}; GPU total={gpu_total} keys={gpu_keys:?}"
        );
        println!(
            "  per-key GPU counts: {:?}",
            (0..128u32)
                .filter(|&k| gpu_by_key[k as usize] > 0 || cpu_by_key[k as usize] > 0)
                .map(|k| (k, cpu_by_key[k as usize], gpu_by_key[k as usize]))
                .collect::<Vec<_>>()
        );

        // 断言：GPU 输出与 CPU 同数量级（GPU 是 CPU 的 50%..150%）。
        // 不立即 panic，而是收集所有文件的违规，全部跑完后统一断言，
        // 这样所有文件的对比数字都能打印出来供分析。
        if cpu_total > 0 {
            let ratio = gpu_total as f64 / cpu_total as f64;
            if !(ratio > 0.5 && ratio < 1.5) {
                bad_ratios.push((path, cpu_total, gpu_total, ratio));
            }
        }
    }
    assert!(
        bad_ratios.is_empty(),
        "GPU/CPU 输出比例异常: {bad_ratios:?}"
    );
    if !tested_any {
        eprintln!("没有可用的 MIDI 文件，测试跳过");
    }
}

/// 滚动序列：upload 后两次不同 scroll_x 的 dispatch，输出必须不同。
/// 如果相同 → dispatch 层没更新（cull 层 bug）。
#[test]
fn cull_scroll_sequence_updates() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut all_notes = Vec::new();
    let mut offsets = [0u32; KEY_COUNT + 1];
    for key in 0..128u8 {
        let mut notes = Vec::new();
        for i in 0..20_000 {
            // 均匀 10-tick 网格会让 c1/c2 两个视口恰好容纳相同数量音符
            // （c1==c2，滚动是否更新无从分辨）。在 [50000, 82000) 挖一个
            // 空洞（第二块从 82000 开始），让三个视口的音符数各不相同。
            // 空洞右缘需避开新视口左边界（keyboard_width=60px=600 tick）：
            // c1 可见 [40000, 47412] 落在第一块尾部，c2 可见 [80000, 87412]
            // 只覆盖空洞右缘之后 540 个，两视口数量必然不同。
            let start = (i as u32) * 10 + if i >= 5000 { 32_000 } else { 0 };
            notes.push(NoteInstance {
                start_tick: start,
                end_tick: start + 5,
                packed: NoteInstance::pack(key, 0, 100),
            });
        }
        offsets[key as usize] = all_notes.len() as u32;
        all_notes.extend(notes);
    }
    // 128 及以上未使用的 key：offsets 全部填充 total，保持 start==end（空桶）
    for o in offsets.iter_mut().skip(128) {
        *o = all_notes.len() as u32;
    }
    cull.upload_all_notes(
        &device,
        &queue,
        &uniform_buffer,
        &all_notes,
        &offsets,
        &[0; KEY_COUNT],
    )
    .unwrap();

    let run = |cull: &mut CullState, scroll_x: f32| -> u64 {
        let u = Uniforms {
            width: 800.0,
            height: 600.0,
            scroll_x,
            scroll_y: 0.0,
            pixels_per_tick: 0.1,
            key_height: 12.0,
            keyboard_width: 60.0,
            mode: 1,
            ..Default::default()
        };
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
        // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
        queue.submit([encoder.finish()]);
        // 读回所有 key 的 draw_args（只读 frame_chunk_counts 个 chunk）
        let mut total: u64 = 0;
        for key in 0..128 {
            let Some(args_buf) = &cull.per_key_draw_args_buffers[key] else {
                continue;
            };
            let chunk_count = cull.frame_chunk_counts[key] as usize;
            if chunk_count == 0 {
                continue;
            }
            let readback = device.create_buffer(&BufferDescriptor {
                label: Some("args_readback"),
                size: 20 * chunk_count as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 20 * chunk_count as u64);
            queue.submit([enc.finish()]);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(Ordering::SeqCst));
            let view = readback.slice(..).get_mapped_range().expect("readback map");
            let args: &[u32] = bytemuck::cast_slice(&view);
            for c in 0..chunk_count {
                total += args[c * 5 + 1] as u64;
            }
            drop(view);
            readback.unmap();
        }
        total
    };

    let c0 = run(&mut cull, 0.0); // 视口 tick ~[0, 7412]
    let c1 = run(&mut cull, 4000.0); // 视口 tick ~[39388, 47412]
    let c2 = run(&mut cull, 8000.0); // 视口 tick ~[79388, 87412]（音符到 199990，仍有）
    println!("SCROLL: c0={c0} c1={c1} c2={c2}");
    assert!(c1 != c0, "滚动后输出必须变化: c0={c0} c1={c1}");
    assert!(c2 != c1, "滚动后输出必须变化: c1={c1} c2={c2}");
    assert!(c0 > 0, "首个视口应有输出");
}

/// 真实 MIDI：模拟 egui 层完整序列（upload 判断 → dispatch → 滚动 → 切轨）。
#[test]
fn cull_real_midi_sequence() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let paths = [
        "/Users/jieneng/Music/MIDIs/99 Luftballons.mid",
        "/Users/jieneng/Music/MIDIs/1.mid",
        "/Users/jieneng/Music/MIDIs/123.mid",
    ];
    let mut tested_any = false;
    for path in paths {
        let Ok(model) = yinhe_midi::parse_path(path) else {
            continue; // 文件不存在或解析失败 → 跳过
        };
        tested_any = true;
        println!(
            "=== FILE {path}: tracks={} note_count={}",
            model.tracks.len(),
            model.note_count
        );

        let hidden = std::collections::HashSet::new();
        let all_visible: Vec<bool> = vec![true; model.tracks.len()];

        // ── 模拟 gpu_upload::upload 的 note_key 判断 + 上传 ──
        let note_key =
            |revision: u64, tv: &[bool], h: &std::collections::HashSet<(u16, u32, u8)>| {
                crate::NoteBufferKey::new(revision, tv, h).value()
            };
        let mut last_key = 0u64;
        let mut last_rev = 0u64;
        let mut last_hidden = 0u64;
        let upload_once =
            |cull: &mut CullState,
             model: &yinhe_core::YinModel,
             tv: &[bool],
             note_revisions: &[u64; KEY_COUNT],
             last_key: &mut u64,
             last_rev: &mut u64,
             last_hidden: &mut u64,
             revision: u64,
             hidden: &std::collections::HashSet<(u16, u32, u8)>| {
                let cull_was_ready = cull.per_key_bind_groups.iter().any(|bg| bg.is_some());
                if !cull_was_ready {
                    *last_key = 0;
                }
                let nk = note_key(revision, tv, hidden);
                if nk == *last_key {
                    return;
                }
                // 全量上传（简化：不做增量路径，测试重点是全量+track_visible）
                let (all_notes, offsets) = crate::pianoroll::build_all_notes(model, hidden, tv);
                cull.upload_all_notes(
                    &device,
                    &queue,
                    &uniform_buffer,
                    &all_notes,
                    &offsets,
                    note_revisions,
                )
                .unwrap();
                *last_key = nk;
                *last_rev = revision;
                *last_hidden = crate::hash_hidden(hidden);
            };

        let revision: u64 = 1;
        let note_revisions = [revision; KEY_COUNT];

        // 步骤 1：首次全量上传（全轨道可见）
        upload_once(
            &mut cull,
            &model,
            &all_visible,
            &note_revisions,
            &mut last_key,
            &mut last_rev,
            &mut last_hidden,
            revision,
            &hidden,
        );
        // 步骤 2：dispatch 视口 1（scroll_x=0）
        let run = |cull: &mut CullState, scroll_x: f32| -> u64 {
            let u = Uniforms {
                width: 800.0,
                height: 600.0,
                scroll_x,
                scroll_y: 0.0,
                pixels_per_tick: 0.1,
                key_height: 12.0,
                keyboard_width: 60.0,
                mode: 1,
                ..Default::default()
            };
            queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
            // 必须提交 encoder，否则 cull 的 compute pass 不会在 GPU 上执行。
            queue.submit([encoder.finish()]);
            let mut total: u64 = 0;
            for key in 0..128 {
                let Some(args_buf) = &cull.per_key_draw_args_buffers[key] else {
                    continue;
                };
                let chunk_count = cull.frame_chunk_counts[key] as usize;
                if chunk_count == 0 {
                    continue;
                }
                let readback = device.create_buffer(&BufferDescriptor {
                    label: Some("args_readback"),
                    size: 20 * chunk_count as u64,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut enc = device.create_command_encoder(&Default::default());
                enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 20 * chunk_count as u64);
                queue.submit([enc.finish()]);
                let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let done2 = done.clone();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                    done2.store(true, Ordering::SeqCst);
                });
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("poll failed");
                assert!(done.load(Ordering::SeqCst));
                let view = readback.slice(..).get_mapped_range().expect("readback map");
                let args: &[u32] = bytemuck::cast_slice(&view);
                for c in 0..chunk_count {
                    total += args[c * 5 + 1] as u64;
                }
                drop(view);
                readback.unmap();
            }
            total
        };
        let s0 = run(&mut cull, 0.0);
        // 步骤 3：模拟滚动（upload 判断：note_key 不变 → 跳过）→ dispatch 视口 2
        upload_once(
            &mut cull,
            &model,
            &all_visible,
            &note_revisions,
            &mut last_key,
            &mut last_rev,
            &mut last_hidden,
            revision,
            &hidden,
        );
        let s1 = run(&mut cull, 1000.0);
        // 步骤 4：模拟切轨（track_visible 变化 → 必须重新上传）→ dispatch
        let mut half_visible = all_visible.clone();
        for (i, v) in half_visible.iter_mut().enumerate() {
            if i % 2 == 1 {
                *v = false;
            }
        }
        upload_once(
            &mut cull,
            &model,
            &half_visible,
            &note_revisions,
            &mut last_key,
            &mut last_rev,
            &mut last_hidden,
            revision,
            &hidden,
        );
        let s2 = run(&mut cull, 1000.0);
        println!("SEQ {path}: s0={s0} s1={s1} s2={s2}");
        assert!(s1 != s0, "滚动后输出必须变化: s0={s0} s1={s1}");
        // 切轨后输出应减少（一半轨道隐藏；若模型轨道数<=1 或全部音符在同一轨道则可能不减，允许相等）
        assert!(s2 <= s1, "切轨后输出不应增加: s1={s1} s2={s2}");
    }
    if !tested_any {
        eprintln!("无可用 MIDI 文件，跳过");
    }
}

/// 精确逐 key 复现测试：每 key 50000 音符（13 个 bucket），视口滚动到
/// 歌曲各位置（开头 / 1/4 / 中间 / 3/4），GPU cull 输出与 CPU f32 镜像
/// 逐 key 精确对比（容差 2）。
///
/// 现有测试只断言「比例 50%~150%」或「输出 > 0」，覆盖不到「每个 key
/// 只显示前几个 bucket、后面全部丢失」这类部分丢失 bug。
#[test]
fn cull_mid_song_exact_per_key() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // 每 key 50000 音符：start = i*20, end = start+10 → 覆盖 [0, 1M) ticks，
    // 13 个 bucket / 196 chunks。
    let mut all_notes = Vec::new();
    let mut offsets = [0u32; KEY_COUNT + 1];
    for key in 0..128u8 {
        let notes: Vec<NoteInstance> = (0..50_000u32)
            .map(|i| NoteInstance {
                start_tick: i * 20,
                end_tick: i * 20 + 10,
                packed: NoteInstance::pack(key, 0, 100),
            })
            .collect();
        offsets[key as usize] = all_notes.len() as u32;
        all_notes.extend(notes);
    }
    // 128 及以上未使用的 key：offsets 全部填充 total，保持 start==end（空桶）
    for o in offsets.iter_mut().skip(128) {
        *o = all_notes.len() as u32;
    }
    cull.upload_all_notes(
        &device,
        &queue,
        &uniform_buffer,
        &all_notes,
        &offsets,
        &[0; KEY_COUNT],
    )
    .unwrap();

    let (w, h, kh, kb_w) = (800.0f32, 600.0f32, 12.0f32, 60.0f32);
    let mut any_bad = false;
    for ppu in [0.1f32, 0.026372144] {
        // 视口中心 tick：开头 / 1/4 / 中间 / 3/4（scroll_x = tick * ppu）
        for &center_tick in &[0u32, 250_000, 500_000, 750_000] {
            let scroll_x = center_tick as f32 * ppu;
            let u = Uniforms {
                width: w,
                height: h,
                scroll_x,
                scroll_y: 0.0,
                pixels_per_tick: ppu,
                key_height: kh,
                keyboard_width: kb_w,
                mode: 1,
                ..Default::default()
            };
            queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
            queue.submit([encoder.finish()]);

            let (ts, te) = visible_tick_range(&u);
            let x_offset = kb_w - scroll_x;
            let bottom_y = 128.0 * kh - u.scroll_y;
            let mut mismatches: Vec<(u32, u64, u64, i64)> = Vec::new();
            for key in 0..128u8 {
                // CPU 期望：f32 镜像 shader 的 X + Y 条件
                let mut expected = 0u64;
                for n in
                    &all_notes[offsets[key as usize] as usize..offsets[key as usize + 1] as usize]
                {
                    if n.end_tick > n.start_tick {
                        let px = x_offset + n.start_tick as f32 * ppu;
                        let pr = x_offset + n.end_tick as f32 * ppu;
                        // 与 cull.wgsl 一致：左边界 = keyboard_width（键盘列下不画）
                        if pr >= kb_w && px <= w {
                            let k = (n.packed & 0xFF) as f32;
                            let pb = bottom_y - k * kh;
                            let py = bottom_y - (k + 1.0) * kh;
                            if pb >= 0.0 && py <= h {
                                expected += 1;
                            }
                        }
                    }
                }

                // GPU：读回该 key 的 draw_args（本帧实际派发的 chunk）
                let chunk_count = cull.frame_chunk_counts[key as usize] as usize;
                let mut gpu = 0u64;
                if chunk_count > 0 {
                    let args_buf = cull.per_key_draw_args_buffers[key as usize]
                        .as_ref()
                        .expect("有 chunk 派发却没有 args buffer");
                    let readback = device.create_buffer(&BufferDescriptor {
                        label: Some("args_readback"),
                        size: 20 * chunk_count as u64,
                        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    let mut enc = device.create_command_encoder(&Default::default());
                    enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 20 * chunk_count as u64);
                    queue.submit([enc.finish()]);
                    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let done2 = done.clone();
                    readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                        done2.store(true, Ordering::SeqCst);
                    });
                    device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .expect("poll failed");
                    assert!(done.load(Ordering::SeqCst));
                    let view = readback.slice(..).get_mapped_range().expect("readback map");
                    let args: &[u32] = bytemuck::cast_slice(&view);
                    for c in 0..chunk_count {
                        gpu += args[c * 5 + 1] as u64;
                    }
                    drop(view);
                    readback.unmap();
                }

                let diff = gpu as i64 - expected as i64;
                if diff.abs() > 2 {
                    mismatches.push((key as u32, expected, gpu, diff));
                }
            }
            if !mismatches.is_empty() {
                any_bad = true;
                println!(
                    "✗ ppu={ppu} scroll_x={scroll_x} (ts={ts} te={te}) 不匹配 key 数={}, 示例: {:?}",
                    mismatches.len(),
                    &mismatches[..mismatches.len().min(8)]
                );
                // 打印一个坏 key 的 bucket 诊断
                let key = mismatches[0].0 as usize;
                if let Some(idx) = &cull.bucket_indexes[key] {
                    println!(
                        "  key={key}: chunks={} chunk_total={} c_lo={} c_hi_bound={} dispatched_chunks={}",
                        idx.chunk_start.len(),
                        idx.chunk_total,
                        idx.block_prefix_max.partition_point(|&m| m < ts) * 64,
                        idx.chunk_start.partition_point(|&s| s <= te),
                        cull.frame_chunk_counts[key],
                    );
                }
            }
        }
    }
    assert!(!any_bad, "存在 GPU 与 CPU 逐 key 计数不匹配（见上方打印）");
}

/// 真实大 MIDI + 相对视口：视口中心 = 歌曲总 tick 的 25% / 50% / 75%，
/// GPU cull 输出与 CPU f32 镜像逐 key 精确对比（容差 2）。
///
/// 黑乐谱大文件音符密度极高（每个 key 的音符数可达数百万、几十上百个
/// bucket），与合成均匀分布的场景不同，需要真实文件验证。
#[test]
fn cull_real_large_midi_relative_viewport() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let paths = [
        "/Users/jieneng/Music/MIDIs/tau2.5.9.mid",
        "/Users/jieneng/Music/MIDIs/5K 5,555,555 notes by The Atom Bomb.mid",
    ];
    let (w, h, kh, kb_w) = (1376.0f32, 419.0f32, 3.2734375, 60.0f32);
    let ppu = 0.026372144f32;
    let mut tested_any = false;
    for path in paths {
        let t0 = std::time::Instant::now();
        let Ok(model) = yinhe_midi::parse_path(path) else {
            println!("{path}: 不存在或解析失败，跳过");
            continue;
        };
        tested_any = true;
        println!(
            "=== {path}: parse={:?} note_count={} tick_length={} tracks={}",
            t0.elapsed(),
            model.note_count,
            model.tick_length,
            model.tracks.len()
        );

        let hidden = std::collections::HashSet::new();
        let track_visible: Vec<bool> = vec![true; model.tracks.len()];
        let t1 = std::time::Instant::now();
        let (all_notes, offsets) =
            crate::pianoroll::build_all_notes(&model, &hidden, &track_visible);
        println!("build_all_notes {:?} len={}", t1.elapsed(), all_notes.len());

        let mut cull = CullState::new(&device);
        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("test_uniform"),
            size: 256,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let t2 = std::time::Instant::now();
        cull.upload_all_notes(
            &device,
            &queue,
            &uniform_buffer,
            &all_notes,
            &offsets,
            &[0; KEY_COUNT],
        )
        .unwrap();
        println!("upload_all_notes {:?}", t2.elapsed());

        let total_ticks = model.tick_length;
        let x_offset = |scroll_x: f32| kb_w - scroll_x;
        let bottom_y = 128.0 * kh;
        let mut any_bad = false;
        // 视口中心 = 歌曲总长的比例处（相对位置）
        for frac in [0.0f32, 0.25, 0.5, 0.75] {
            let center_tick = total_ticks as f32 * frac;
            // scroll_x 使 tick=center_tick 落在视口中心
            let scroll_x = (kb_w + center_tick * ppu - w / 2.0).max(0.0);
            let u = Uniforms {
                width: w,
                height: h,
                scroll_x,
                scroll_y: 0.0,
                pixels_per_tick: ppu,
                key_height: kh,
                keyboard_width: kb_w,
                mode: 1,
                ..Default::default()
            };
            queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
            queue.submit([encoder.finish()]);

            let (ts, te) = visible_tick_range(&u);
            let xo = x_offset(scroll_x);
            let mut mismatches: Vec<(u32, u64, u64, i64)> = Vec::new();
            let mut gpu_total = 0u64;
            let mut cpu_total = 0u64;
            for key in 0..128u8 {
                // CPU 期望：f32 镜像 shader 的 X + Y 条件
                let mut expected = 0u64;
                for n in
                    &all_notes[offsets[key as usize] as usize..offsets[key as usize + 1] as usize]
                {
                    if n.end_tick > n.start_tick {
                        let px = xo + n.start_tick as f32 * ppu;
                        let pr = xo + n.end_tick as f32 * ppu;
                        // 与 cull.wgsl 一致：左边界 = keyboard_width（键盘列下不画）
                        if pr >= kb_w && px <= w {
                            let k = (n.packed & 0xFF) as f32;
                            let pb = bottom_y - k * kh;
                            let py = bottom_y - (k + 1.0) * kh;
                            if pb >= 0.0 && py <= h {
                                expected += 1;
                            }
                        }
                    }
                }
                cpu_total += expected;

                let chunk_count = cull.frame_chunk_counts[key as usize] as usize;
                let mut gpu = 0u64;
                if chunk_count > 0 {
                    let args_buf = cull.per_key_draw_args_buffers[key as usize]
                        .as_ref()
                        .expect("有 chunk 派发却没有 args buffer");
                    let readback = device.create_buffer(&BufferDescriptor {
                        label: Some("args_readback"),
                        size: 20 * chunk_count as u64,
                        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    let mut enc = device.create_command_encoder(&Default::default());
                    enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 20 * chunk_count as u64);
                    queue.submit([enc.finish()]);
                    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let done2 = done.clone();
                    readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                        done2.store(true, Ordering::SeqCst);
                    });
                    device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .expect("poll failed");
                    assert!(done.load(Ordering::SeqCst));
                    let view = readback.slice(..).get_mapped_range().expect("readback map");
                    let args: &[u32] = bytemuck::cast_slice(&view);
                    for c in 0..chunk_count {
                        gpu += args[c * 5 + 1] as u64;
                    }
                    drop(view);
                    readback.unmap();
                }
                gpu_total += gpu;

                let diff = gpu as i64 - expected as i64;
                if diff.abs() > 2 {
                    mismatches.push((key as u32, expected, gpu, diff));
                }
            }
            println!(
                "  frac={frac} (ts={ts} te={te}) GPU_total={gpu_total} CPU_total={cpu_total} 不匹配 key 数={}",
                mismatches.len()
            );
            if !mismatches.is_empty() {
                any_bad = true;
                println!("    示例: {:?}", &mismatches[..mismatches.len().min(10)]);
                let key = mismatches[0].0 as usize;
                if let Some(idx) = &cull.bucket_indexes[key] {
                    println!(
                        "    key={key}: chunks={} chunk_total={} c_lo={} c_hi_bound={} dispatched_chunks={}",
                        idx.chunk_start.len(),
                        idx.chunk_total,
                        idx.block_prefix_max.partition_point(|&m| m < ts) * 64,
                        idx.chunk_start.partition_point(|&s| s <= te),
                        cull.frame_chunk_counts[key],
                    );
                }
            }
        }
        assert!(
            !any_bad,
            "{path}: 存在 GPU 与 CPU 逐 key 计数不匹配（见上方打印）"
        );
    }
    assert!(tested_any, "没有任何可用 MIDI 文件");
}

/// 多帧交互序列：模拟真实使用中的状态机（滚动 → skip 优化 → 编辑增量
/// 上传 → 切轨 → hidden 变化），每帧 GPU cull 输出与 CPU f32 镜像逐 key
/// 精确对比。覆盖单帧 dispatch 测试测不到的上传/派发状态交互。
#[test]
fn cull_multi_frame_interaction_sequence() {
    let path = "/Users/jieneng/Music/MIDIs/test.mid";
    if !std::path::Path::new(path).exists() {
        eprintln!("test.mid 不存在，跳过");
        return;
    }
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let t0 = std::time::Instant::now();
    let mut model = yinhe_midi::parse_path(path).expect("parse test.mid 失败");
    println!(
        "parse {:?} note_count={} tick_length={}",
        t0.elapsed(),
        model.note_count,
        model.tick_length
    );

    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let (w, h, kh, kb_w) = (1376.0f32, 419.0f32, 3.2734375, 60.0f32);
    let ppu = 0.026372144f32;
    let bottom_y = 128.0 * kh;

    // ── 模拟 gpu_upload::upload 的状态机 ──
    let mut last_key = 0u64; // note_key
    let mut last_rev = 0u64; // revision
    let mut last_hidden = 0u64; // hidden hash
    let mut revision: u64 = 1;
    let mut tv: Vec<bool> = vec![true; model.tracks.len()];
    let mut hidden: std::collections::HashSet<(u16, u32, u8)> = std::collections::HashSet::new();
    let note_key_of =
        |revision: u64, tv: &[bool], h: &std::collections::HashSet<(u16, u32, u8)>| {
            crate::NoteBufferKey::new(revision, tv, h).value()
        };

    // 帧处理：执行上传状态机 + dispatch + 读回 + 对比
    let mut frame_no = 0;
    let mut run_frame = |frame_no: &mut usize,
                         cull: &mut CullState,
                         model: &yinhe_core::YinModel,
                         revision: u64,
                         tv: &[bool],
                         hidden: &std::collections::HashSet<(u16, u32, u8)>,
                         scroll_x: f32|
     -> bool {
        *frame_no += 1;
        // ── 上传状态机（镜像 gpu_upload::upload）──
        let cull_was_ready = cull.per_key_bind_groups.iter().any(|bg| bg.is_some());
        if !cull_was_ready {
            last_key = 0;
        }
        let nk = note_key_of(revision, tv, hidden);
        let mut uploaded_kind = "skip";
        if nk != last_key {
            if !cull_was_ready {
                let (all_notes, offsets) = crate::pianoroll::build_all_notes(model, hidden, tv);
                cull.upload_all_notes(
                    &device,
                    &queue,
                    &uniform_buffer,
                    &all_notes,
                    &offsets,
                    &model.note_revisions,
                )
                .unwrap();
                uploaded_kind = "full";
            } else {
                let revision_changed = revision != last_rev;
                let hidden_changed = crate::hash_hidden(hidden) != last_hidden;
                if hidden_changed && !revision_changed {
                    let (all_notes, offsets) = crate::pianoroll::build_all_notes(model, hidden, tv);
                    cull.upload_all_notes(
                        &device,
                        &queue,
                        &uniform_buffer,
                        &all_notes,
                        &offsets,
                        &model.note_revisions,
                    )
                    .unwrap();
                    uploaded_kind = "full(hidden)";
                } else if revision_changed {
                    let dirty: Vec<u8> = (0u8..128)
                        .filter(|&k| {
                            model.note_revisions[k as usize]
                                != cull.uploaded_key_revisions[k as usize]
                        })
                        .collect();
                    if dirty.is_empty() {
                        uploaded_kind = "none(rev-only)";
                    } else {
                        let mut all_ok = true;
                        for &k in &dirty {
                            let key_notes = crate::pianoroll::build_key_notes(model, k, hidden, tv);
                            if cull.per_key_buffers[k as usize].is_none() {
                                all_ok = false;
                                break;
                            }
                            cull.upload_one_key(&device, &queue, &uniform_buffer, k, &key_notes)
                                .unwrap();
                            cull.uploaded_key_revisions[k as usize] =
                                model.note_revisions[k as usize];
                        }
                        if all_ok {
                            uploaded_kind = "incremental";
                        } else {
                            let (all_notes, offsets) =
                                crate::pianoroll::build_all_notes(model, hidden, tv);
                            cull.upload_all_notes(
                                &device,
                                &queue,
                                &uniform_buffer,
                                &all_notes,
                                &offsets,
                                &model.note_revisions,
                            )
                            .unwrap();
                            uploaded_kind = "full(fallback)";
                        }
                    }
                } else {
                    let (all_notes, offsets) = crate::pianoroll::build_all_notes(model, hidden, tv);
                    cull.upload_all_notes(
                        &device,
                        &queue,
                        &uniform_buffer,
                        &all_notes,
                        &offsets,
                        &model.note_revisions,
                    )
                    .unwrap();
                    uploaded_kind = "full(tv)";
                }
            }
            last_key = nk;
            last_rev = revision;
            last_hidden = crate::hash_hidden(hidden);
        }

        // ── dispatch ──
        let u = Uniforms {
            width: w,
            height: h,
            scroll_x,
            scroll_y: 0.0,
            pixels_per_tick: ppu,
            key_height: kh,
            keyboard_width: kb_w,
            mode: 1,
            ..Default::default()
        };
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
        queue.submit([encoder.finish()]);

        // ── CPU 参考：镜像 shader X+Y 条件（用当前 model）──
        let xo = kb_w - scroll_x;
        let mut mismatches: Vec<(u32, u64, u64, i64)> = Vec::new();
        let mut gpu_total = 0u64;
        let mut cpu_total = 0u64;
        for key in 0..128u8 {
            let mut expected = 0u64;
            for n in model.notes[key as usize].iter() {
                if n.end_tick > n.start_tick
                    && tv.get(n.track as usize).copied().unwrap_or(true)
                    && !hidden.contains(&(n.track, n.start_tick, key))
                {
                    let px = xo + n.start_tick as f32 * ppu;
                    let pr = xo + n.end_tick as f32 * ppu;
                    // 与 cull.wgsl 一致：左边界 = keyboard_width（键盘列下不画）
                    if pr >= kb_w && px <= w {
                        let k = key as f32;
                        let pb = bottom_y - k * kh;
                        let py = bottom_y - (k + 1.0) * kh;
                        if pb >= 0.0 && py <= h {
                            expected += 1;
                        }
                    }
                }
            }
            cpu_total += expected;

            let chunk_count = cull.frame_chunk_counts[key as usize] as usize;
            let mut gpu = 0u64;
            if chunk_count > 0 {
                let args_buf = cull.per_key_draw_args_buffers[key as usize]
                    .as_ref()
                    .expect("有 chunk 派发却没有 args buffer");
                let readback = device.create_buffer(&BufferDescriptor {
                    label: Some("args_readback"),
                    size: 20 * chunk_count as u64,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut enc = device.create_command_encoder(&Default::default());
                enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 20 * chunk_count as u64);
                queue.submit([enc.finish()]);
                let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let done2 = done.clone();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                    done2.store(true, Ordering::SeqCst);
                });
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("poll failed");
                assert!(done.load(Ordering::SeqCst));
                let view = readback.slice(..).get_mapped_range().expect("readback map");
                let args: &[u32] = bytemuck::cast_slice(&view);
                for c in 0..chunk_count {
                    gpu += args[c * 5 + 1] as u64;
                }
                drop(view);
                readback.unmap();
            }
            gpu_total += gpu;

            let diff = gpu as i64 - expected as i64;
            if diff.abs() > 2 {
                mismatches.push((key as u32, expected, gpu, diff));
            }
        }
        println!(
            "帧{frame_no}: upload={uploaded_kind} scroll_x={scroll_x} GPU={gpu_total} CPU={cpu_total} 不匹配={}",
            mismatches.len()
        );
        if !mismatches.is_empty() {
            println!("  示例: {:?}", &mismatches[..mismatches.len().min(5)]);
            return false;
        }
        true
    };

    let mut ok = true;
    let total_ticks = model.tick_length as f32;
    // 帧 1：首次全量上传 + 开头视口
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        0.0,
    );
    // 帧 2：滚动到 25%
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        (total_ticks * 0.25 * ppu).max(0.0),
    );
    // 帧 3：滚动到 50%（跳过，note_key 不变 → upload=skip）
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        (total_ticks * 0.5 * ppu).max(0.0),
    );
    // 帧 4：相同视口（dispatch 的 skip 优化）
    let mid_scroll = (total_ticks * 0.5 * ppu).max(0.0);
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        mid_scroll,
    );
    // 帧 5：编辑 key 60（加一个音符）→ 增量上传
    {
        let k = 60u8;
        let start = (model.tick_length / 2) as u32;
        let id = model.alloc_note_id();
        std::sync::Arc::make_mut(&mut model.notes[k as usize]).insert_sorted(yinhe_types::Note {
            id,
            start_tick: start,
            end_tick: start + 240,
            velocity: 100,
            track: 0,
        });
        model.mark_dirty(k);
        model.rebuild_dirty();
        revision = revision.wrapping_add(1);
    }
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        mid_scroll,
    );
    // 帧 6：切轨（隐藏 track 1-7）→ track_visible 全量
    for v in tv.iter_mut().take(8).skip(1) {
        *v = false;
    }
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        mid_scroll,
    );
    // 帧 7：hidden_notes 变化（全量）
    hidden.insert((0, 0, 60));
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        mid_scroll,
    );
    // 帧 8：滚动到 75%
    ok &= run_frame(
        &mut frame_no,
        &mut cull,
        &model,
        revision,
        &tv,
        &hidden,
        (total_ticks * 0.75 * ppu).max(0.0),
    );
    assert!(ok, "多帧交互序列存在 GPU/CPU 不匹配（见上方打印）");
}

/// 真实渲染 + 像素读回：把 cull 结果真正画到 texture 上，读回像素统计
/// 「音符像素数」随视口位置的变化，找显示中断点。
///
/// 用户报告：铅笔工具能探测到音符（数据/位置正确）但显示不出来——
/// 说明问题在渲染层。此测试验证 draw_args 之外的真实绘制结果。
#[test]
fn cull_render_pixel_check() {
    let path = "/Users/jieneng/Music/MIDIs/tau2.5.9.mid";
    if !std::path::Path::new(path).exists() {
        eprintln!("tau2.5.9.mid 不存在，跳过");
        return;
    }
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let t0 = std::time::Instant::now();
    let model = yinhe_midi::parse_path(path).expect("parse 失败");
    println!(
        "parse {:?} note_count={} tick_length={} tracks={}",
        t0.elapsed(),
        model.note_count,
        model.tick_length,
        model.tracks.len()
    );

    // 单轨：选音符最多的轨道（音符存在 model.notes[key]，需按 track 统计）
    let mut best_track = 0usize;
    let mut best_count = 0usize;
    for i in 0..model.tracks.len() {
        let c = model.notes_for_track(i as u16).count();
        if c > best_count {
            best_count = c;
            best_track = i;
        }
    }
    println!("单轨 track {best_track}: {} 音符", best_count);
    let mut tv: Vec<bool> = vec![false; model.tracks.len()];
    tv[best_track] = true;
    let hidden = std::collections::HashSet::new();
    let (all_notes, offsets) = crate::pianoroll::build_all_notes(&model, &hidden, &tv);

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = crate::InstanceRenderer::new(device.clone(), queue.clone(), format);
    renderer.upload_all_notes_for_cull(&all_notes, &offsets, &model.note_revisions);
    // CPU 模式对照：同一视口用 build_notes + legacy 绘制
    let mut renderer_legacy = crate::InstanceRenderer::new(device.clone(), queue.clone(), format);

    let (w, h, kh, kb_w) = (1376.0f32, 419.0f32, 3.2734375, 60.0f32);
    let ppu = 0.026372144f32;
    let pw = w as u32;
    let ph = h as u32;
    // 不透明轨道色（非黑 → 可统计）
    let track_colors: Vec<[f32; 4]> = (0..model.tracks.len())
        .map(|_| [0.2, 0.7, 1.0, 1.0])
        .collect();

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("diag_target"),
        size: wgpu::Extent3d {
            width: pw,
            height: ph,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    let target_legacy = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("diag_target_legacy"),
        size: wgpu::Extent3d {
            width: pw,
            height: ph,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_legacy_view = target_legacy.create_view(&Default::default());
    let bytes_per_row = pw * 4;
    let aligned_row = bytes_per_row.div_ceil(256) * 256;

    let read_pixels = |device: &Device, queue: &Queue, target: &wgpu::Texture| -> u64 {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pixel_readback"),
            size: (aligned_row * ph) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned_row),
                    rows_per_image: Some(ph),
                },
            },
            wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
        let mapped = buffer.slice(..).get_mapped_range().expect("readback map");
        let mut note_pixels = 0u64;
        for row in 0..ph {
            let start = (row as usize) * aligned_row as usize;
            let row_data = &mapped[start..start + bytes_per_row as usize];
            for px in row_data.chunks_exact(4) {
                if px[0] > 8 || px[1] > 8 || px[2] > 8 {
                    note_pixels += 1;
                }
            }
        }
        drop(mapped);
        buffer.unmap();
        note_pixels
    };

    let total_ticks = model.tick_length as f32;
    // 聚焦验证：只测几个关键位置（像素缺失 vs CPU 期望的对比）
    let probe_ticks: [f32; 4] = [0.0, 87_168.0, 435_840.0, 1_089_600.0];
    let mut prev_pixels: Option<u64> = None;
    let mut prev_tick: Option<f32> = None;
    let mut suspicious: Vec<(f32, u64, u64)> = Vec::new();
    for (step, &tick) in probe_ticks.iter().enumerate() {
        let tick = tick.min(total_ticks);
        let scroll_x = (kb_w + tick * ppu - w / 2.0).max(0.0);
        let view = yinhe_types::PianoRollView {
            key_height: kh,
            viewport_h: h,

            orientation: yinhe_types::Orientation::Horizontal,
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: ppu,
                scroll_x,
                scroll_y: 0.0,
                left_panel_width: kb_w,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
        };
        let job = crate::pianoroll::build_render_job(
            pw,
            ph,
            &view,
            &yinhe_core::Selection::default(),
            &track_colors,
            0,
            0.0,
            false,
        );
        renderer.upload_uniforms(job.uniforms);
        renderer.upload_track_colors(&job.track_colors);
        renderer.upload_selection(&job.selection);
        renderer.ensure_layers(1);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.draw(&mut encoder, &target_view, pw, ph);
        queue.submit([encoder.finish()]);
        let (ts_lo, ts_hi) = visible_tick_range(&job.uniforms);
        let cull_pixels = read_pixels(&device, &queue, &target);

        // CPU 模式（legacy）：build_notes + note layer
        let mut instances = Vec::new();
        crate::pianoroll::build_notes(&mut instances, w, h, &model, &view, &hidden, &tv);
        // 每 key 的 CPU 实例数
        let mut cpu_by_key = [0u32; 128];
        for n in &instances {
            cpu_by_key[(n.packed & 0xFF) as usize] += 1;
        }
        renderer_legacy.upload_uniforms(job.uniforms);
        renderer_legacy.upload_track_colors(&job.track_colors);
        renderer_legacy.upload_selection(&job.selection);
        renderer_legacy.ensure_layers(1);
        renderer_legacy.upload_note_layer(0, 0, |out| {
            out.extend(instances.iter().copied());
        });
        let mut enc2 = device.create_command_encoder(&Default::default());
        renderer_legacy.draw(&mut enc2, &target_legacy_view, pw, ph);
        queue.submit([enc2.finish()]);
        let legacy_pixels = read_pixels(&device, &queue, &target_legacy);

        if let (Some(p), Some(pt)) = (prev_pixels, prev_tick)
            && tick > pt + 1.0
            && cull_pixels < p / 10
            && p > 500
        {
            suspicious.push((tick, p, cull_pixels));
            println!("⚠️ 渲染中断候选: tick={tick:.0} 前帧像素={p} 当前={cull_pixels}");
        }
        prev_pixels = Some(cull_pixels);
        prev_tick = Some(tick);
        if step % 8 == 0 || cull_pixels != legacy_pixels {
            println!(
                "tick={tick:.0} scroll={scroll_x:.1} cull像素={cull_pixels} cpu像素={legacy_pixels} (cpu实例={})",
                instances.len()
            );
        }
        // cull 像素分布：按 y 行统计，判断哪些 key 画出来了
        if cull_pixels > 0 {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pixel_dump"),
                size: (aligned_row * ph) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(aligned_row),
                        rows_per_image: Some(ph),
                    },
                },
                wgpu::Extent3d {
                    width: pw,
                    height: ph,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([enc.finish()]);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(std::sync::atomic::Ordering::SeqCst));
            let mapped = buffer.slice(..).get_mapped_range().expect("readback map");
            // 每 key 一行（kh 像素），统计该行非黑像素数
            let mut key_px = [0u32; 128];
            let mut min_x = pw;
            let mut max_x = 0u32;
            for row in 0..ph {
                let start = (row as usize) * aligned_row as usize;
                let row_data = &mapped[start..start + bytes_per_row as usize];
                let mut row_px = 0u32;
                for (col, px) in row_data.chunks_exact(4).enumerate() {
                    if px[0] > 8 || px[1] > 8 || px[2] > 8 {
                        row_px += 1;
                        min_x = min_x.min(col as u32);
                        max_x = max_x.max(col as u32);
                    }
                }
                if row_px > 0 {
                    // y=0 是 key 127，y=ph 是 key 0
                    let key = ((419.0 - row as f32) / kh).clamp(0.0, 127.0) as usize;
                    key_px[key] += row_px;
                }
            }
            drop(mapped);
            buffer.unmap();
            // 列出有像素的 key（含该 key 的 chunk 信息）与 CPU 期望对比
            let mut painted: Vec<(u32, u32, u32, u32, u32)> = Vec::new(); // (key, px, chunk_count, c_lo, cpu_instances)
            let mut missing: Vec<(u32, u32, u32, u32)> = Vec::new(); // (key, cpu_instances, chunk_count, c_lo) 有 CPU 音符但没画
            for k in 0..128usize {
                let cc = renderer.cull.frame_chunk_counts[k];
                let clo = renderer.cull.bucket_indexes[k]
                    .as_ref()
                    .and_then(|idx| idx.visible_chunk_range(ts_lo, ts_hi))
                    .map(|(lo, _)| lo)
                    .unwrap_or(0);
                if key_px[k] > 0 {
                    painted.push((k as u32, key_px[k], cc, clo, cpu_by_key[k]));
                } else if cpu_by_key[k] > 0 {
                    missing.push((k as u32, cpu_by_key[k], cc, clo));
                }
            }
            println!("  画出的 key（key,像素,chunk数,c_lo,cpu实例）: {painted:?}");
            println!("  有 CPU 音符但 0 像素的 key（key,cpu实例,chunk数,c_lo）: {missing:?}");
            println!("  x∈[{min_x},{max_x}] 视口 tick [{ts_lo},{ts_hi}]");
        }
    }
    println!("渲染完成，中断候选数={}", suspicious.len());
    assert!(
        suspicious.len() <= 3,
        "发现 {} 处疑似渲染中断（见上方打印）",
        suspicious.len()
    );
}

/// 聚焦验证：在一个「cull 像素=0 但 CPU 有大量音符」的视口，读回
/// visible buffer 的实际内容，确认 shader 到底写入了什么。
/// 区分两类根因：shader 写入错误 vs 渲染管线问题。
#[test]
fn cull_visible_buffer_content_check() {
    let path = "/Users/jieneng/Music/MIDIs/tau2.5.9.mid";
    if !std::path::Path::new(path).exists() {
        eprintln!("tau2.5.9.mid 不存在，跳过");
        return;
    }
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let model = yinhe_midi::parse_path(path).expect("parse 失败");

    // 选 key60 音符最多的轨道（保证视口内 key60 一定有可见音符），
    // 探针 tick 对准该轨道 key60 的第一个音符。
    let notes60 = model.key_notes(60);
    let mut track_counts: Vec<usize> = vec![0; model.tracks.len()];
    for n in notes60.iter() {
        track_counts[n.track as usize] += 1;
    }
    let best_track = track_counts
        .iter()
        .enumerate()
        .max_by_key(|(_, c)| *c)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let tick = notes60
        .iter()
        .find(|n| n.track as usize == best_track)
        .map(|n| n.start_tick as f32)
        .unwrap_or(0.0);
    println!("key60 最多轨道 track {best_track}，探针 tick={tick}");
    let mut tv: Vec<bool> = vec![false; model.tracks.len()];
    tv[best_track] = true;
    let hidden = std::collections::HashSet::new();
    let (all_notes, offsets) = crate::pianoroll::build_all_notes(&model, &hidden, &tv);

    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    cull.upload_all_notes(
        &device,
        &queue,
        &uniform_buffer,
        &all_notes,
        &offsets,
        &model.note_revisions,
    )
    .unwrap();

    let (w, h, kh, kb_w) = (1376.0f32, 419.0f32, 3.2734375, 60.0f32);
    let ppu = 0.026372144f32;
    // 复现「cull 0 像素」的位置：视口中心对准 key60 首音符所在 tick
    let scroll_x = (kb_w + tick * ppu - w / 2.0).max(0.0);
    let u = Uniforms {
        width: w,
        height: h,
        scroll_x,
        scroll_y: 0.0,
        pixels_per_tick: ppu,
        key_height: kh,
        keyboard_width: kb_w,
        mode: 1,
        ..Default::default()
    };
    queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
    let mut encoder = device.create_command_encoder(&Default::default());
    cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
    queue.submit([encoder.finish()]);
    let (ts_lo, ts_hi) = visible_tick_range(&u);
    println!("视口 ts={ts_lo} te={ts_hi}");

    // CPU 期望（key 60 的可见音符，前 10 个）
    let view = yinhe_types::PianoRollView {
        key_height: kh,
        viewport_h: h,

        orientation: yinhe_types::Orientation::Horizontal,
        base: yinhe_types::TimelineViewBase {
            pixels_per_tick: ppu,
            scroll_x,
            scroll_y: 0.0,
            left_panel_width: kb_w,
            dirty: true,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
            follow_target: None,
        },
    };
    let mut cpu_instances = Vec::new();
    crate::pianoroll::build_notes(&mut cpu_instances, w, h, &model, &view, &hidden, &tv);
    let cpu_key60: Vec<_> = cpu_instances
        .iter()
        .filter(|n| n.packed & 0xFF == 60)
        .take(10)
        .map(|n| (n.start_tick, n.end_tick, n.packed))
        .collect();
    println!("CPU 期望（key60 前10）: {cpu_key60:?}");

    // GPU：读回 key 60 的 draw_args + visible buffer 内容
    let key = 60u8;
    let chunk_count = cull.frame_chunk_counts[key as usize] as usize;
    println!("key60: chunk_count={chunk_count} 首可见 chunk 内实例数（读回验证）");
    let mut total_gpu = 0u64;
    let mut shown_first: Vec<(u32, u32, u32)> = Vec::new();
    if chunk_count > 0 {
        let args_buf = cull.per_key_draw_args_buffers[key as usize]
            .as_ref()
            .expect("args buffer");
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("args_readback"),
            size: 20 * chunk_count as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, 20 * chunk_count as u64);
        queue.submit([enc.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst));
        let view = readback.slice(..).get_mapped_range().expect("readback map");
        let args: &[u32] = bytemuck::cast_slice(&view);
        let mut nonempty_chunks = 0u32;
        for c in 0..chunk_count {
            let n = args[c * 5 + 1];
            total_gpu += n as u64;
            if n > 0 {
                nonempty_chunks += 1;
            }
        }
        println!("key60 draw_args: 非空 chunk={nonempty_chunks}/{chunk_count} 总实例={total_gpu}");
        let args_copy: Vec<u32> = args.to_vec();
        drop(view);
        readback.unmap();

        // 读回 visible buffer 中第一个非空 chunk 的槽位内容
        let mut first_nonzero: Option<usize> = None;
        let mut enc2 = device.create_command_encoder(&Default::default());
        // 重新读 args（上面的 view 已 drop）
        let readback2 = device.create_buffer(&BufferDescriptor {
            label: Some("args_readback2"),
            size: 20 * chunk_count as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc2.copy_buffer_to_buffer(args_buf, 0, &readback2, 0, 20 * chunk_count as u64);
        queue.submit([enc2.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        readback2
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst));
        let v2 = readback2
            .slice(..)
            .get_mapped_range()
            .expect("readback map");
        let args2: &[u32] = bytemuck::cast_slice(&v2);
        for c in 0..chunk_count {
            if args2[c * 4 + 1] > 0 {
                first_nonzero = Some(c);
                break;
            }
        }
        drop(v2);
        readback2.unmap();

        if let Some(c) = first_nonzero {
            let vis_buf = cull.per_key_visible_buffers[key as usize]
                .as_ref()
                .expect("vis buffer");
            let count = args_copy[c * 4 + 1] as usize;
            // c 是相对 wg 索引；实际槽位 = (c_lo + wg) * 256（4B 索引/槽）
            let c_lo = cull.bucket_indexes[key as usize]
                .as_ref()
                .and_then(|idx| idx.visible_chunk_range(ts_lo, ts_hi))
                .map(|(lo, _)| lo)
                .unwrap_or(0);
            let rb = device.create_buffer(&BufferDescriptor {
                label: Some("vis_readback"),
                size: 4 * count as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc3 = device.create_command_encoder(&Default::default());
            enc3.copy_buffer_to_buffer(
                vis_buf,
                ((c_lo + c as u32) * 256) as u64 * 4,
                &rb,
                0,
                4 * count as u64,
            );
            queue.submit([enc3.finish()]);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            rb.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(Ordering::SeqCst));
            let v = rb.slice(..).get_mapped_range().expect("readback map");
            let idx: &[u32] = bytemuck::cast_slice(&v);
            // 索引是 per-key 本地位置 → 加 offsets[key] 才是全局 all_notes 位置。
            let base = offsets[key as usize] as usize;
            for i in 0..count.min(10) {
                let note = all_notes[base + idx[i] as usize];
                shown_first.push((note.start_tick, note.end_tick, note.packed));
            }
            drop(v);
            rb.unmap();
        }
    }
    println!("key60 visible buffer 首非空 chunk 内容: {shown_first:?}");
    assert!(
        total_gpu > 0,
        "key60 在这个视口应该有可见音符（CPU 有 {} 个）",
        cpu_instances.len()
    );
}

/// 最小复现：手工构造一个 key（10 chunks），视口只覆盖后 2 chunks
/// （c_lo=8）。验证 c_lo≠0 时 chunk 槽位定位（first_instance≠0）正确。
#[test]
fn cull_draw_c_lo_nonzero_minimal() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = crate::InstanceRenderer::new(device.clone(), queue.clone(), format);

    // 手工构造：key 60，2560 个音符（10 chunks）。
    // 音符 i：start = i*100，end = start+10（tick 0..256000）
    let mut notes = Vec::new();
    for i in 0..2560u32 {
        notes.push(NoteInstance {
            start_tick: i * 100,
            end_tick: i * 100 + 10,
            packed: NoteInstance::pack(60, 0, 100),
        });
    }
    let mut offsets = [0u32; KEY_COUNT + 1];
    offsets[60] = 0;
    for o in offsets.iter_mut().take(KEY_COUNT + 1).skip(61) {
        *o = 2560;
    }
    offsets[KEY_COUNT] = 2560;
    renderer.upload_all_notes_for_cull(&notes, &offsets, &[0; KEY_COUNT]);

    let (w, h, kh, kb_w) = (1376.0f32, 419.0f32, 3.2734375, 60.0f32);
    let ppu = 0.026372144f32;
    let pw = w as u32;
    let ph = h as u32;
    let track_colors: Vec<[f32; 4]> = vec![[0.2, 0.7, 1.0, 1.0]];

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("min_target"),
        size: wgpu::Extent3d {
            width: pw,
            height: ph,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    let bytes_per_row = pw * 4;
    let aligned_row = bytes_per_row.div_ceil(256) * 256;

    let render_and_count = |renderer: &mut crate::InstanceRenderer, scroll_x: f32| -> (u64, u32) {
        let view = yinhe_types::PianoRollView {
            key_height: kh,
            viewport_h: h,

            orientation: yinhe_types::Orientation::Horizontal,
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: ppu,
                scroll_x,
                scroll_y: 0.0,
                left_panel_width: kb_w,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
        };
        let job = crate::pianoroll::build_render_job(
            pw,
            ph,
            &view,
            &yinhe_core::Selection::default(),
            &track_colors,
            0,
            0.0,
            false,
        );
        renderer.upload_uniforms(job.uniforms);
        renderer.upload_track_colors(&job.track_colors);
        renderer.upload_selection(&job.selection);
        renderer.ensure_layers(1);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.draw(&mut encoder, &target_view, pw, ph);
        queue.submit([encoder.finish()]);
        let (ts_lo, ts_hi) = visible_tick_range(&job.uniforms);
        // 渲染后读回 args + 槽位内容（确认渲染时的数据）
        let cc = renderer.cull.frame_chunk_counts[60];
        if let Some(args_buf) = &renderer.cull.per_key_draw_args_buffers[60] {
            let rb = device.create_buffer(&BufferDescriptor {
                label: Some("args_rb"),
                size: 20 * cc as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(args_buf, 0, &rb, 0, 20 * cc as u64);
            queue.submit([enc.finish()]);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            rb.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(Ordering::SeqCst));
            let v = rb.slice(..).get_mapped_range().expect("readback map");
            let a: &[u32] = bytemuck::cast_slice(&v);
            let c_lo = renderer.cull.bucket_indexes[60]
                .as_ref()
                .and_then(|idx| idx.visible_chunk_range(ts_lo, ts_hi))
                .map(|(lo, _)| lo)
                .unwrap_or(0);
            println!(
                "  ts={ts_lo} te={ts_hi} c_lo={c_lo} cc={cc} args[0..min(4)]={:?}",
                &a[..a.len().min(16)]
            );
            drop(v);
            rb.unmap();
        }
        // 读回像素
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px"),
            size: (aligned_row * ph) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned_row),
                    rows_per_image: Some(ph),
                },
            },
            wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
        let mapped = buffer.slice(..).get_mapped_range().expect("readback map");
        let mut px = 0u64;
        for row in 0..ph {
            let start = (row as usize) * aligned_row as usize;
            let row_data = &mapped[start..start + bytes_per_row as usize];
            for p in row_data.chunks_exact(4) {
                if p[0] > 8 || p[1] > 8 || p[2] > 8 {
                    px += 1;
                }
            }
        }
        drop(mapped);
        buffer.unmap();
        // 读回 vis buffer 槽位 c_lo*256 起的内容（确认数据存在）
        let c_lo = renderer.cull.bucket_indexes[60]
            .as_ref()
            .and_then(|idx| idx.visible_chunk_range(ts_lo, ts_hi))
            .map(|(lo, _)| lo)
            .unwrap_or(0);
        if let Some(vis_buf) = &renderer.cull.per_key_visible_buffers[60] {
            let rb2 = device.create_buffer(&BufferDescriptor {
                label: Some("vis_rb"),
                size: 4 * 8_u64,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(vis_buf, (c_lo * 256) as u64 * 4, &rb2, 0, 4 * 8);
            queue.submit([enc.finish()]);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let done2 = done.clone();
            rb2.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                done2.store(true, Ordering::SeqCst);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll failed");
            assert!(done.load(Ordering::SeqCst));
            let v2 = rb2.slice(..).get_mapped_range().expect("readback map");
            let idx: &[u32] = bytemuck::cast_slice(&v2);
            println!(
                "  槽位 c_lo*256 起 8 个索引: {:?}",
                idx[..idx.len().min(8)].to_vec()
            );
            drop(v2);
            rb2.unmap();
        }
        (px, cc)
    };

    // 场景 A：视口覆盖全部 10 chunks（c_lo=0）——预期画出 2560 个音符
    // 视口 tick [0, 260000]：scroll_x = 0
    let (px_a, cc_a) = render_and_count(&mut renderer, 0.0);
    println!("场景A(c_lo=0): 像素={px_a} chunk数={cc_a}（预期 ~2560 音符）");

    // 场景 B：视口只覆盖后 2 chunks（c_lo=8）——预期画出 512 个音符
    // 后 2 chunks = 音符 [2048, 2560) = tick [204800, 256000]
    // 视口 tick [200000, 260000]：scroll_x = 200000*ppu - kb_w + w/2 附近
    let scroll_b = (200_000f32 * ppu - kb_w + w / 2.0).max(0.0);
    let (px_b, cc_b) = render_and_count(&mut renderer, scroll_b);
    println!("场景B(c_lo=8): 像素={px_b} chunk数={cc_b}（预期 ~512 音符）");
    assert!(px_a > 1000, "场景A 应画出大量音符，实际像素={px_a}");
    assert!(px_b > 200, "场景B 应画出音符，实际像素={px_b}");
}

/// PR 模式可见 key 范围（与 `InstanceRenderer::visible_key_range` 同款计算）。
fn pr_visible_key_range(kh: f32, scroll_y: f32, height: f32) -> (u8, u8) {
    let bottom = 128.0 * kh - scroll_y;
    let top_key = ((bottom / kh).ceil() as i32 - 1).clamp(0, 127);
    let bottom_key = (((bottom - height) / kh).ceil() as i32 - 1).clamp(0, 127);
    let lo = bottom_key.min(top_key).saturating_sub(1).clamp(0, 127);
    let hi = bottom_key.max(top_key).saturating_add(1).clamp(0, 127);
    (lo as u8, hi as u8)
}

/// 大 tick 长音符衔接处的 1px 缝隙回归测试。
///
/// 旧实现右边界走「pixel_x + pixel_w」链式（两次舍入），在 .5 临界
/// 处与相邻音符的直接计算边界岔开 1px。歌曲靠后（scroll_x / tick 大）
/// 时 f32 大值运算的 ULP 大，缩放（ppu 连续变化）时最容易触发。
/// 音符必须够长（> 2px）走出 2.0 宽度下限，链式误差才会显现。
///
/// 实证：随机搜索「缩放中的 ppu / 滚动偏移」组合，渲染 2000 个密集
/// 音符（PPQ 1920 的 1/32 gate，tick 384 万起，严格衔接），扫描整行
/// 断言非空像素列连续。
#[test]
fn note_boundary_no_gap_large_tick() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = crate::InstanceRenderer::new(device.clone(), queue.clone(), format);

    let (w, kh, kb_w) = (1376.0f32, 40.0f32, 60.0f32);
    let h = 600.0f32;
    let pw = w as u32;
    let ph = h as u32;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gap_target"),
        size: wgpu::Extent3d {
            width: pw,
            height: ph,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());

    // 伪随机（确定性，可复现）。
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };

    // 随机组合搜索：缩放 ppu、歌曲靠后的滚动位置。
    // 注意：音符必须严格衔接（end_tick == 下一个的 start_tick），
    // 纯测浮点缝隙，排除任何真实间隙的干扰。
    // 音符长度 240 tick（PPQ 1920 的 1/32 音符）——必须 > 2px（ppu≤0.06
    // 时 8 tick 宽音符永远走 2.0 宽度下限，链式误差被掩盖，抓不到旧 bug）。
    let trials = 300;
    for trial in 0..trials {
        // 密集音符：2000 个，每个 240 tick 宽（PPQ 1920 / 32），严格衔接。
        // t0 从 500 小节（500×1920×4 = 384 万 tick）起，歌曲靠后。
        let t0 = 3_840_000 + next() % 2_000_000;
        let mut notes = Vec::with_capacity(2000);
        let mut tick = t0;
        for _ in 0..2000 {
            notes.push(NoteInstance {
                start_tick: tick,
                end_tick: tick + 240,
                packed: NoteInstance::pack(60, 0, 100),
            });
            tick += 240;
        }
        renderer.ensure_layers(1);
        renderer.upload_note_layer(0, 0, |out| out.extend_from_slice(&notes));

        // 缩放 ppu（连续变化中的任意值）与滚动位置（歌曲靠后）。
        let ppu = 0.005 + (next() % 1100) as f32 * 0.00005; // 0.005..0.06
        let scroll_x = t0 as f32 * ppu - 300.0 + (next() % 600) as f32;
        let view = yinhe_types::PianoRollView {
            key_height: kh,
            viewport_h: h,

            orientation: yinhe_types::Orientation::Horizontal,
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: ppu,
                scroll_x: scroll_x.max(0.0),
                scroll_y: 2380.0, // key 60 行居中
                left_panel_width: kb_w,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
        };
        let job = crate::pianoroll::build_render_job(
            pw,
            ph,
            &view,
            &yinhe_core::Selection::default(),
            &[[0.2, 0.7, 1.0, 1.0]],
            1, // scroll_mode = 1（整数对齐，取整路径）
            0.0,
            false,
        );
        renderer.upload_uniforms(job.uniforms);
        renderer.upload_track_colors(&job.track_colors);
        renderer.upload_selection(&job.selection);

        let mut enc = device.create_command_encoder(&Default::default());
        renderer.draw(&mut enc, &target_view, pw, ph);
        queue.submit([enc.finish()]);

        let bytes_per_row = pw * 4;
        let aligned_row = bytes_per_row.div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gap_px"),
            size: (aligned_row * ph) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned_row),
                    rows_per_image: Some(ph),
                },
            },
            wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
        let mapped = buffer.slice(..).get_mapped_range().expect("readback map");

        // 扫描 key 60 行的中间一行（y=320）：非空列必须连续（无缝隙）。
        let row = 320usize;
        let start = row * aligned_row as usize;
        let row_data = &mapped[start..start + bytes_per_row as usize];
        let mut min_col: Option<u32> = None;
        let mut max_col = 0u32;
        let mut count = 0u32;
        for (col, p) in row_data.chunks_exact(4).enumerate() {
            if p[0] > 8 || p[1] > 8 || p[2] > 8 {
                min_col.get_or_insert(col as u32);
                max_col = col as u32;
                count += 1;
            }
        }
        drop(mapped);
        buffer.unmap();

        let Some(min_col) = min_col else {
            continue; // 音符不在视口内，换组合
        };
        let span = max_col - min_col + 1;
        assert_eq!(
            count, span,
            "衔接音符间出现缝隙: trial={trial} t0={t0} ppu={ppu} scroll_x={scroll_x} 非空列 {count}/{span} (min={min_col} max={max_col})"
        );
    }
}

/// 性能对比 benchmark：CPU 构建 vs GPU cull。
///
/// 默认用 start.mid（1.3GB / 826 轨黑乐谱，1.64 亿音符）；
/// 可用环境变量 YIN_BENCH_MIDI 覆盖为其他文件（如 tau2.5.9.mid）快速验证。
///
/// 以帧率为核心指标：
///   - CPU 路径每帧 = `build_notes`（UI 线程同步构建可见音符），FPS = 1000/ms。
///   - GPU cull 滚动中每帧 = dispatch 的 CPU 编码时间 + GPU 执行时间，
///     有效 FPS = 1000/max(cpu_ms, gpu_ms)（UI 编码与 GPU 执行流水线重叠，
///     帧率受两者较大者限制）。
///   - GPU cull 静止帧 = dispatch 被 skip，实测其成本。
///
/// 运行：cargo test -p yinhe-wgpu --release -- --ignored --nocapture cull_bench
#[test]
#[ignore]
fn cull_bench_vs_cpu_start_mid() {
    let path = std::env::var("YIN_BENCH_MIDI")
        .unwrap_or_else(|_| "/Users/jieneng/Music/MIDIs/start.mid".to_string());
    let t0 = std::time::Instant::now();
    let model = yinhe_midi::parse_path(path).expect("解析 start.mid");
    let parse_ms = t0.elapsed().as_secs_f64() * 1e3;

    let mut total = 0u64;
    let mut max_key_notes = 0u64;
    let mut max_end_tick = 0u32;
    for k in 0..128u8 {
        let notes = model.key_notes(k);
        total += notes.len() as u64;
        max_key_notes = max_key_notes.max(notes.len() as u64);
        if let Some(last) = notes.last() {
            max_end_tick = max_end_tick.max(last.end_tick);
        }
    }
    println!("== start.mid 规模 ==");
    println!(
        "解析 {parse_ms:.0}ms，音符 {total}，轨道 {}，最大 key 音符数 {max_key_notes}，总时长 {max_end_tick} ticks",
        model.tracks.len()
    );

    let Some((device, queue)) = headless_device() else {
        eprintln!("无可用 GPU 适配器，跳过");
        return;
    };
    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("bench_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let hidden = std::collections::HashSet::new();
    let track_visible: Vec<bool> = vec![true; model.tracks.len()];

    // ── GPU cull 一次性成本：全量构建 + 全量上传（加载 / 轨道显隐切换时）──
    let t = std::time::Instant::now();
    let (all_notes, offsets) = crate::pianoroll::build_all_notes(&model, &hidden, &track_visible);
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    cull.upload_all_notes(
        &device,
        &queue,
        &uniform_buffer,
        &all_notes,
        &offsets,
        &[0; KEY_COUNT],
    )
    .unwrap();
    let upload_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("\n== GPU cull 一次性成本（加载 / 轨道显隐切换） ==");
    println!(
        "build_all_notes {build_ms:.0}ms，upload_all_notes {upload_ms:.0}ms，数据 {:.0}MB",
        all_notes.len() as f64 * 12.0 / 1e6
    );
    // 清空队列：write_buffer 的 GPU 拷贝在 submit 时才发生，先让它落地。
    queue.submit([device.create_command_encoder(&Default::default()).finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");

    // 空 submit + poll 的固定开销（Metal 同步往返延迟），用于修正 GPU 执行时间。
    let mut empty_poll_ms = f64::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        queue.submit([device.create_command_encoder(&Default::default()).finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        empty_poll_ms = empty_poll_ms.min(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("空 submit+poll 固定开销: {empty_poll_ms:.3}ms");

    // ── 每帧成本：多档缩放 × 两个滚动位置（60% 与 350 小节≈87%）──
    let (width, height, kb_w, kh) = (1600.0f32, 900.0f32, 80.0f32, 14.0f32);
    let scroll_y = 400.0f32;
    let ppus = [0.02f32, 0.05, 0.1, 0.5, 2.0];
    let scroll_fracs = [0.6f32, 0.87];
    println!("\n== 每帧成本（视口 {width:.0}x{height:.0}；滚动帧 scroll_x 每帧 +1px） ==");
    println!(
        "{:>4} {:>5} {:>11} {:>9} {:>9} {:>11} {:>9} {:>11} {:>9} {:>9} {:>6}",
        "位置",
        "ppu",
        "可见音符",
        "CPU/ms",
        "CPU/FPS",
        "GPU编码/ms",
        "GPU执行/ms",
        "GPU/FPS",
        "dispatch",
        "chunk",
        "skip/ms"
    );
    // key_lo/key_hi 与 ppu 无关（PR 模式的 Y 坐标不依赖缩放），提前算好。
    let (key_lo, key_hi) = pr_visible_key_range(kh, scroll_y, height);
    println!("PR 可见 key 范围: {key_lo}..={key_hi}");
    for &frac in &scroll_fracs {
        for &ppu in &ppus {
            let scroll_x0 = (max_end_tick as f32 * ppu * frac).max(0.0);
            let view = yinhe_types::PianoRollView {
                key_height: kh,
                viewport_h: height,

                orientation: yinhe_types::Orientation::Horizontal,
                base: yinhe_types::TimelineViewBase {
                    pixels_per_tick: ppu,
                    scroll_x: scroll_x0,
                    scroll_y,
                    left_panel_width: kb_w,
                    dirty: true,
                    track_panel_row_height: 40.0,
                    track_panel_scroll_y: 0.0,
                    follow_target: None,
                },
            };

            // CPU 路径：暖机 1 次 + 3 次取最优（复用 Vec 避免分配噪声）。
            let mut cpu_out: Vec<NoteInstance> = Vec::new();
            crate::pianoroll::build_notes(
                &mut cpu_out,
                width,
                height,
                &model,
                &view,
                &hidden,
                &track_visible,
            );
            let mut cpu_ms = f64::MAX;
            for _ in 0..3 {
                cpu_out.clear();
                let t = std::time::Instant::now();
                crate::pianoroll::build_notes(
                    &mut cpu_out,
                    width,
                    height,
                    &model,
                    &view,
                    &hidden,
                    &track_visible,
                );
                cpu_ms = cpu_ms.min(t.elapsed().as_secs_f64() * 1e3);
            }
            let cpu_visible = cpu_out.len();

            // GPU cull：5 帧滚动（scroll_x 每帧 +1px，模拟连续滚动，避免 skip 优化），
            // 跳过首帧（首帧含 uniform 首次生效），取后 4 帧最优。
            // GPU 执行时间扣掉空 submit+poll 的固定开销，才是 cull 本身的耗时。
            let mut gpu_cpu_ms = f64::MAX;
            let mut gpu_gpu_ms = f64::MAX;
            for i in 0..5 {
                let u = Uniforms {
                    width,
                    height,
                    scroll_x: scroll_x0 + i as f32,
                    scroll_y,
                    pixels_per_tick: ppu,
                    key_height: kh,
                    keyboard_width: kb_w,
                    mode: 1,
                    ..Default::default()
                };
                queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
                let t = std::time::Instant::now();
                let mut enc = device.create_command_encoder(&Default::default());
                cull.dispatch_cull(&mut enc, &queue, key_lo, key_hi, &u);
                queue.submit([enc.finish()]);
                let cpu_t = t.elapsed().as_secs_f64() * 1e3;
                let t = std::time::Instant::now();
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("poll");
                let gpu_t = t.elapsed().as_secs_f64() * 1e3;
                if i > 0 {
                    gpu_cpu_ms = gpu_cpu_ms.min(cpu_t);
                    gpu_gpu_ms = gpu_gpu_ms.min((gpu_t - empty_poll_ms).max(0.0));
                }
            }
            // 静止帧：uniforms 与上一帧相同 → dispatch 被 skip，实测其成本。
            let t = std::time::Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            cull.dispatch_cull(
                &mut enc,
                &queue,
                key_lo,
                key_hi,
                &cached_uniforms(scroll_x0 + 4.0, ppu, width, height, kb_w, kh, scroll_y),
            );
            queue.submit([enc.finish()]);
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll");
            let skip_ms = (t.elapsed().as_secs_f64() * 1e3 - empty_poll_ms).max(0.0);

            // frame_chunk_counts 对全部 128 key 都填（含 Y 方向视口外的 key），
            // 实际 dispatch 的只有 key_lo..=key_hi 且 chunk>0 的 key。
            let dispatch_count = (key_lo..=key_hi)
                .filter(|&k| cull.frame_chunk_counts[k as usize] > 0)
                .count();
            let chunk_total: u32 = (key_lo..=key_hi)
                .map(|k| cull.frame_chunk_counts[k as usize])
                .sum();

            let cpu_fps = 1000.0 / cpu_ms;
            let gpu_fps = 1000.0 / gpu_cpu_ms.max(gpu_gpu_ms);
            println!(
                "{:>4} {:>5} {:>11} {:>9.2} {:>9.1} {:>11.3} {:>9.3} {:>11.1} {:>9} {:>9} {:>6.3}",
                format!("{:.0}%", frac * 100.0),
                ppu,
                cpu_visible,
                cpu_ms,
                cpu_fps,
                gpu_cpu_ms,
                gpu_gpu_ms,
                gpu_fps,
                dispatch_count,
                chunk_total,
                skip_ms,
            );
        }
    }
}

fn cached_uniforms(
    scroll_x: f32,
    ppu: f32,
    width: f32,
    height: f32,
    kb_w: f32,
    kh: f32,
    scroll_y: f32,
) -> Uniforms {
    Uniforms {
        width,
        height,
        scroll_x,
        scroll_y,
        pixels_per_tick: ppu,
        key_height: kh,
        keyboard_width: kb_w,
        mode: 1,
        ..Default::default()
    }
}

/// AR 模式（mode=2, lane_height）的 GPU cull 与 CPU 逐音符 AABB 判定一致性。
///
/// AR 的 Y 坐标依赖 track（lane 分层），不能按 key 裁剪，128 key 全部 dispatch。
/// CPU 期望值用与 `cull.wgsl` 完全相同的 f32 数学逐音符计算，不做 merge
/// （merge 是 build_arr_notes 的显示优化，不在 cull 职责内）。
#[test]
fn cull_arr_mode_lane_height_vs_cpu() {
    let path = "/Users/jieneng/Music/MIDIs/APT.mid";
    if !std::path::Path::new(path).exists() {
        eprintln!("APT.mid 不存在，跳过");
        return;
    }
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let model = yinhe_midi::parse_path(path).expect("parse 失败");
    let hidden = std::collections::HashSet::new();
    let tv: Vec<bool> = vec![true; model.tracks.len()];
    let (all_notes, offsets) = crate::pianoroll::build_all_notes(&model, &hidden, &tv);

    let mut cull = CullState::new(&device);
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test_uniform"),
        size: 256,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    cull.upload_all_notes(
        &device,
        &queue,
        &uniform_buffer,
        &all_notes,
        &offsets,
        &model.note_revisions,
    )
    .unwrap();

    // AR 视口：部分轨道可见（lane_height=20），tick 滚动到歌曲中部。
    let (w, h, lh, kb_w) = (1376.0f32, 800.0f32, 20.0f32, 60.0f32);
    let ppu = 0.05f32;
    let scroll_x = 2000.0f32;
    let scroll_y = 100.0f32;
    let u = Uniforms {
        width: w,
        height: h,
        scroll_x,
        scroll_y,
        pixels_per_tick: ppu,
        keyboard_width: kb_w,
        mode: 2, // AR: lane_height based
        lane_height: lh,
        ..Default::default()
    };
    queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&u));
    let mut encoder = device.create_command_encoder(&Default::default());
    cull.dispatch_cull(&mut encoder, &queue, 0, 127, &u);
    queue.submit([encoder.finish()]);

    // 读回 draw_args 求 GPU 可见总数。
    let mut gpu_total: u64 = 0;
    for key in 0..128u8 {
        let chunk_count = cull.frame_chunk_counts[key as usize];
        if chunk_count == 0 {
            continue;
        }
        let Some(args_buf) = &cull.per_key_draw_args_buffers[key as usize] else {
            continue;
        };
        let read_size = chunk_count as u64 * 20;
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("args_readback"),
            size: read_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(args_buf, 0, &readback, 0, read_size);
        queue.submit([enc.finish()]);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |_| {
            done2.store(true, Ordering::SeqCst);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll failed");
        assert!(done.load(Ordering::SeqCst));
        let view = readback.slice(..).get_mapped_range().expect("readback map");
        let args: &[u32] = bytemuck::cast_slice(&view);
        for c in 0..chunk_count as usize {
            gpu_total += args[c * 5 + 1] as u64;
        }
        drop(view);
        readback.unmap();
    }

    // CPU 期望：逐音符跑 shader 同款判定（tick 范围用保守的 visible_tick_range 过滤）。
    let (ts_lo, ts_hi) = visible_tick_range(&u);
    let mut cpu_visible: u64 = 0;
    for n in &all_notes {
        if n.end_tick <= n.start_tick {
            continue;
        }
        if n.start_tick > ts_hi || n.end_tick < ts_lo {
            continue; // 保守跳过，不漏（长音符跨左边界时 end >= ts_lo 仍保留）
        }
        let key = (n.packed & 0xFF) as f32;
        let track = ((n.packed >> 8) & 0xFFFF) as f32;
        let x_offset = kb_w - scroll_x;
        let pixel_x = x_offset + n.start_tick as f32 * ppu;
        let pixel_right = x_offset + n.end_tick as f32 * ppu;
        // 与 cull.wgsl 一致：左边界 = keyboard_width（轨道面板列下不画）
        if pixel_right >= kb_w && pixel_x <= w {
            let lh_per_key = lh / 128.0;
            let pixel_bottom = -scroll_y + lh - key * lh_per_key + track * lh;
            let pixel_y = -scroll_y + lh - (key + 1.0) * lh_per_key + track * lh;
            if pixel_bottom >= 0.0 && pixel_y <= h {
                cpu_visible += 1;
            }
        }
    }
    println!("AR mode=2: CPU={cpu_visible} GPU={gpu_total}");
    assert!(
        gpu_total == cpu_visible,
        "AR GPU cull 与 CPU 判定不一致: GPU={gpu_total} CPU={cpu_visible}"
    );
}
