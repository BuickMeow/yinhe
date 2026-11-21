//! 单音符热路径 + undo delta 基准。
//!
//! 目的：在改动写路径之前，量化当前 `NoteBucket` / `YinModel` 的单音符操作
//! 代价，以及 `NoteDelta` 列式编码的 CPU/内存取舍。
//!
//! 覆盖：
//! 1. 改 end / velocity / start（左拉伸）、删除、移动；分别测“只调写原语”和
//!    “写原语 + `rebuild_dirty()`”两条路径。
//! 2. `NoteBucket::insert_batch_sorted` 批量归并。
//! 3. `NoteDelta` 的 `Vec<(Note, u8)>`、列式 delta、列式 + zstd 三组对比。
//!
//! 运行：
//! `cargo bench -p yinhe-core --bench note_hotpath_bench`
//! `cargo bench -p yinhe-core --bench note_hotpath_bench -- --quick`（只跑 1M）
//!
//! 默认 1M / 10M 音符/key；10M 是压力目标，不是产品硬限制。

use std::hint::black_box;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Instant;

use yinhe_core::{NoteEvent, TrackData, YinModel};
use yinhe_types::Note;

const KEY: u8 = 60;
const TRACK: u16 = 0;
const MAX_TICK: u32 = 10_000_000;

fn next_rng(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

struct BenchModel {
    model: YinModel,
    /// 按输入顺序存的 start_tick；id = 下标 + 1（NoteLoader 按输入顺序发号）。
    starts: Vec<u32>,
}

impl BenchModel {
    fn build(count: usize) -> Self {
        let mut rng = 0x5EED_1234_5678_9ABC;
        let mut starts = Vec::with_capacity(count);
        let notes: Vec<NoteEvent> = (0..count)
            .map(|_| {
                let start = (next_rng(&mut rng) % u64::from(MAX_TICK)) as u32;
                starts.push(start);
                NoteEvent {
                    id: 0,
                    start_tick: start,
                    end_tick: start.saturating_add(10),
                    key: KEY,
                    velocity: 100,
                }
            })
            .collect();

        let mut model = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        model.load_track_notes(vec![notes]);
        model.rebuild();
        assert_eq!(model.notes[KEY as usize].len(), count);
        Self { model, starts }
    }

    fn id(&self, index: usize) -> u32 {
        (index + 1) as u32
    }

    fn index(&self, rng: &mut u64) -> usize {
        (next_rng(rng) as usize) % self.starts.len()
    }
}

fn print_header(title: &str) {
    println!();
    println!("== {title} ==");
}

fn print_op(label: &str, per_op_secs: f64) {
    println!("  {label:<36} {:>14.1} µs/op", per_op_secs * 1e6);
}

fn bench_update_end(bm: &mut BenchModel, iters: usize, with_rebuild: bool, label: &str) {
    let mut rng = 0xA11C_E123;
    let t0 = Instant::now();
    for _ in 0..iters {
        let i = bm.index(&mut rng);
        let start = bm.starts[i];
        let gate = 10 + (next_rng(&mut rng) % 100) as u32;
        let id = bm.id(i);
        let updated = bm
            .model
            .update_note_by_id(KEY, id, |n| n.end_tick = start + gate);
        if with_rebuild {
            bm.model.rebuild_dirty();
        }
        black_box(updated);
    }
    print_op(label, t0.elapsed().as_secs_f64() / iters as f64);
}

fn bench_update_velocity(bm: &mut BenchModel, iters: usize, with_rebuild: bool, label: &str) {
    let mut rng = 0xBEEF_4321;
    let t0 = Instant::now();
    for k in 0..iters {
        let i = bm.index(&mut rng);
        let id = bm.id(i);
        let vel = 1 + (k % 126) as u8;
        let updated = bm.model.update_note_by_id(KEY, id, |n| n.velocity = vel);
        if with_rebuild {
            bm.model.rebuild_dirty();
        }
        black_box(updated);
    }
    print_op(label, t0.elapsed().as_secs_f64() / iters as f64);
}

fn bench_move(bm: &mut BenchModel, iters: usize, with_rebuild: bool, label: &str) {
    let mut rng = 0xCAFE_F00D;
    let t0 = Instant::now();
    for _ in 0..iters {
        let i = bm.index(&mut rng);
        let id = bm.id(i);
        let new_start = (next_rng(&mut rng) % u64::from(MAX_TICK)) as u32;
        let moved = bm.model.move_note(KEY, id, KEY, new_start);
        if moved.is_some() {
            bm.starts[i] = new_start;
        }
        if with_rebuild {
            bm.model.rebuild_dirty();
        }
        black_box(moved);
    }
    print_op(label, t0.elapsed().as_secs_f64() / iters as f64);
}

fn bench_resize_left(bm: &mut BenchModel, iters: usize, label: &str) {
    let mut rng = 0xDEAD_BEEF;
    for k in 0..iters {
        let i = bm.index(&mut rng);
        let old = bm.starts[i];
        let mut new_start = (next_rng(&mut rng) % u64::from(MAX_TICK)) as u32;
        if new_start == old {
            new_start = new_start.saturating_add(1);
        }
        let id = bm.id(i);
        let t0 = Instant::now();
        let updated = bm
            .model
            .update_note_by_id(KEY, id, |n| n.start_tick = new_start);
        let sort_time = t0.elapsed();
        if updated.is_some() {
            bm.starts[i] = new_start;
        }
        let t1 = Instant::now();
        bm.model.rebuild_dirty();
        let stats_time = t1.elapsed();
        black_box(updated);
        println!(
            "  {label:<36} sort {:>8.1} ms + stats {:>8.1} ms = {:>8.1} ms (第 {} 次)",
            sort_time.as_secs_f64() * 1e3,
            stats_time.as_secs_f64() * 1e3,
            (sort_time + stats_time).as_secs_f64() * 1e3,
            k + 1
        );
    }
}

fn bench_remove(
    bm: &mut BenchModel,
    iters: usize,
    with_rebuild: bool,
    label: &str,
    used: &mut std::collections::HashSet<usize>,
) {
    let count = bm.starts.len();
    assert!(used.len() + iters <= count);
    let mut rng = 0x1234_ABCD;
    let mut removed = 0usize;
    let t0 = Instant::now();
    while removed < iters {
        let i = (next_rng(&mut rng) as usize) % count;
        if !used.insert(i) {
            continue;
        }
        let id = bm.id(i);
        let r = bm.model.remove_note_by_id(KEY, id);
        if with_rebuild {
            bm.model.rebuild_dirty();
        }
        assert!(r.is_some(), "id {id} 应该存在");
        removed += 1;
        black_box(r);
    }
    print_op(label, t0.elapsed().as_secs_f64() / iters as f64);
}

fn bench_insert_batch(bm: &mut BenchModel, count: usize) {
    let base = bm.starts.len();
    let mut rng = 0x0BAD_F00D;
    let new_notes: Vec<Note> = (0..count)
        .map(|i| {
            let start = (next_rng(&mut rng) % u64::from(MAX_TICK)) as u32;
            Note {
                id: (base + i + 1) as u32,
                start_tick: start,
                end_tick: start.saturating_add(10),
                velocity: 100,
                track: TRACK,
            }
        })
        .collect();

    let t0 = Instant::now();
    Arc::make_mut(&mut bm.model.notes[KEY as usize]).insert_batch_sorted(new_notes);
    let elapsed = t0.elapsed();
    println!(
        "  {:<36} {:>14.1} ms/op",
        format!("insert_batch_sorted {}k", count / 1000),
        elapsed.as_secs_f64() * 1e3
    );
}

fn bench_model(count: usize, iters: usize, remove_iters: usize, resize_iters: usize) {
    print_header(&format!(
        "单音符热路径：{count} 音符/key（key={KEY}, track={TRACK}）"
    ));
    let mut bm = BenchModel::build(count);

    bench_update_end(&mut bm, iters, false, "改 end（只写原语）");
    bench_update_end(&mut bm, iters, true, "改 end + rebuild_dirty");
    bench_update_velocity(&mut bm, iters, false, "改 velocity（只写原语）");
    bench_update_velocity(&mut bm, iters, true, "改 velocity + rebuild_dirty");
    bench_move(&mut bm, iters, false, "移动（只写原语）");
    bench_move(&mut bm, iters, true, "移动 + rebuild_dirty");
    bench_resize_left(&mut bm, resize_iters, "左拉伸（start 改变，触发 sort）");
    let mut removed_indices = std::collections::HashSet::with_capacity(remove_iters * 2);
    bench_remove(
        &mut bm,
        remove_iters,
        false,
        "删除（只写原语）",
        &mut removed_indices,
    );
    bench_remove(
        &mut bm,
        remove_iters,
        true,
        "删除 + rebuild_dirty",
        &mut removed_indices,
    );
    bench_insert_batch(&mut bm, 100_000);
}

// ---------------------------------------------------------------------------
// NoteDelta 原型：列式 delta
// ---------------------------------------------------------------------------

fn push_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_varint(data: &[u8], pos: &mut usize) -> u64 {
    let mut shift = 0u32;
    let mut v = 0u64;
    loop {
        let b = data[*pos];
        *pos += 1;
        v |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    v
}

fn zigzag_i64(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag_i64(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// 列式编码：count varint + 每音符
/// start delta varint / gate varint / key u8 / vel u8 / track u16 / id delta zigzag。
fn encode_notes(notes: &[(Note, u8)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(notes.len() * 10);
    push_varint(&mut out, notes.len() as u64);
    let mut prev_start = 0u32;
    let mut prev_id = 0i64;
    for (n, key) in notes {
        push_varint(&mut out, u64::from(n.start_tick - prev_start));
        push_varint(&mut out, u64::from(n.end_tick - n.start_tick));
        out.push(*key);
        out.push(n.velocity);
        out.extend_from_slice(&n.track.to_le_bytes());
        push_varint(&mut out, zigzag_i64(i64::from(n.id) - prev_id));
        prev_start = n.start_tick;
        prev_id = i64::from(n.id);
    }
    out
}

fn decode_notes(data: &[u8]) -> Vec<(Note, u8)> {
    let mut pos = 0usize;
    let count = read_varint(data, &mut pos) as usize;
    let mut out = Vec::with_capacity(count);
    let mut prev_start = 0u32;
    let mut prev_id = 0i64;
    for _ in 0..count {
        let delta = read_varint(data, &mut pos) as u32;
        let start = prev_start + delta;
        let gate = read_varint(data, &mut pos) as u32;
        let key = data[pos];
        pos += 1;
        let velocity = data[pos];
        pos += 1;
        let track = u16::from_le_bytes([data[pos], data[pos + 1]]);
        pos += 2;
        let id_delta = unzigzag_i64(read_varint(data, &mut pos));
        let id = (prev_id + id_delta) as u32;
        out.push((
            Note {
                id,
                start_tick: start,
                end_tick: start + gate,
                velocity,
                track,
            },
            key,
        ));
        prev_start = start;
        prev_id = i64::from(id);
    }
    out
}

fn make_delta_notes(count: usize) -> Vec<(Note, u8)> {
    let mut rng = 0xFEED_FACE_1234_5678;
    let mut starts: Vec<u32> = (0..count)
        .map(|_| (next_rng(&mut rng) % u64::from(MAX_TICK)) as u32)
        .collect();
    starts.sort_unstable();
    starts
        .into_iter()
        .enumerate()
        .map(|(i, start)| {
            let gate = 1 + (next_rng(&mut rng) % 1000) as u32;
            let velocity = 1 + (next_rng(&mut rng) % 126) as u8;
            let track = (next_rng(&mut rng) % 8) as u16;
            (
                Note {
                    id: (i + 1) as u32,
                    start_tick: start,
                    end_tick: start.saturating_add(gate),
                    velocity,
                    track,
                },
                KEY,
            )
        })
        .collect()
}

fn bench_delta(count: usize) {
    print_header(&format!("undo NoteDelta 编码 / 解码：{count} 音符"));
    let notes = make_delta_notes(count);
    let raw_bytes = notes.len() * std::mem::size_of::<(Note, u8)>();

    let t0 = Instant::now();
    let copy = notes.clone();
    let clone_time = t0.elapsed();
    black_box(&copy);
    drop(copy);

    let t1 = Instant::now();
    let encoded = encode_notes(&notes);
    let encode_time = t1.elapsed();

    let t2 = Instant::now();
    let decoded = decode_notes(&encoded);
    let decode_time = t2.elapsed();
    assert_eq!(decoded.len(), notes.len());
    assert_eq!(
        decoded.first().map(|x| x.0.id),
        notes.first().map(|x| x.0.id)
    );
    assert_eq!(decoded.last().map(|x| x.0.id), notes.last().map(|x| x.0.id));

    let t3 = Instant::now();
    let compressed = zstd::encode_all(Cursor::new(&encoded), 3).expect("zstd encode");
    let zstd_encode_time = t3.elapsed();

    let t4 = Instant::now();
    let decompressed = zstd::decode_all(Cursor::new(&compressed)).expect("zstd decode");
    let zstd_decode_time = t4.elapsed();
    assert_eq!(decompressed.len(), encoded.len());

    let per = |bytes: usize| bytes as f64 / notes.len() as f64;
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms",
        "Vec<(Note,u8)> 拷贝",
        per(raw_bytes),
        clone_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 编码",
        "列式 delta",
        per(encoded.len()),
        encode_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 解码",
        "列式 delta",
        per(encoded.len()),
        decode_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 编码  {:>10.1} ms 解码",
        "列式 + zstd3",
        per(compressed.len()),
        zstd_encode_time.as_secs_f64() * 1e3,
        zstd_decode_time.as_secs_f64() * 1e3
    );
    black_box((decoded.len(), decompressed.len()));
}

fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    println!("note_hotpath_bench: build mode = release, quick = {quick}");

    bench_model(1_000_000, 100, 100, 3);
    bench_delta(1_000_000);

    if !quick {
        bench_model(10_000_000, 10, 10, 1);
        bench_delta(10_000_000);
    }

    println!();
    println!("done");
}
