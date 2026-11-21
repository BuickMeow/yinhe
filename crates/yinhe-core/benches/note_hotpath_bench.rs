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

    let grouped: Vec<Note> = notes.iter().map(|(n, _)| *n).collect();
    let t5 = Instant::now();
    let grouped_copy = grouped.clone();
    let grouped_clone_time = t5.elapsed();
    black_box(&grouped_copy);
    drop(grouped_copy);

    let t6 = Instant::now();
    let zstd1 = zstd::encode_all(Cursor::new(&encoded), 1).expect("zstd1 encode");
    let zstd1_encode_time = t6.elapsed();

    let t7 = Instant::now();
    let zstd1_decoded = zstd::decode_all(Cursor::new(&zstd1)).expect("zstd1 decode");
    let zstd1_decode_time = t7.elapsed();
    assert_eq!(zstd1_decoded.len(), encoded.len());

    let t8 = Instant::now();
    let lz4 = lz4_flex::compress_prepend_size(&encoded);
    let lz4_encode_time = t8.elapsed();

    let t9 = Instant::now();
    let lz4_decoded = lz4_flex::decompress_size_prepended(&lz4).expect("lz4 decode");
    let lz4_decode_time = t9.elapsed();
    assert_eq!(lz4_decoded.len(), encoded.len());

    let per = |bytes: usize| bytes as f64 / notes.len() as f64;
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms",
        "Vec<(Note,u8)> 拷贝",
        per(raw_bytes),
        clone_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 拷贝",
        "Vec<Note>（按 key 分组）",
        per(grouped.len() * std::mem::size_of::<Note>()),
        grouped_clone_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 编码  {:>10.1} ms 解码",
        "列式 delta",
        per(encoded.len()),
        encode_time.as_secs_f64() * 1e3,
        decode_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 编码  {:>10.1} ms 解码",
        "列式 + zstd1",
        per(zstd1.len()),
        zstd1_encode_time.as_secs_f64() * 1e3,
        zstd1_decode_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 编码  {:>10.1} ms 解码",
        "列式 + zstd3",
        per(compressed.len()),
        zstd_encode_time.as_secs_f64() * 1e3,
        zstd_decode_time.as_secs_f64() * 1e3
    );
    println!(
        "  {:<32} {:>10.2} B/音符  {:>10.1} ms 编码  {:>10.1} ms 解码",
        "列式 + lz4",
        per(lz4.len()),
        lz4_encode_time.as_secs_f64() * 1e3,
        lz4_decode_time.as_secs_f64() * 1e3
    );
    black_box((
        decoded.len(),
        decompressed.len(),
        zstd1_decoded.len(),
        lz4_decoded.len(),
    ));
}

// ---------------------------------------------------------------------------
// 优化后原型：locate_at + 单点搬移 + 增量统计（仅 bench 内，不是生产代码）
// ---------------------------------------------------------------------------

const PROTO_CHUNK: usize = 65_536;

struct ProtoBucket {
    chunks: Vec<Vec<Note>>,
    starts: Vec<u32>,
    max_ends: Vec<u32>,
}

impl ProtoBucket {
    fn from_sorted(notes: Vec<Note>) -> Self {
        let mut chunks = Vec::new();
        let mut starts = Vec::new();
        let mut max_ends = Vec::new();
        let mut current = Vec::with_capacity(PROTO_CHUNK);
        let mut max_end = 0u32;
        for n in notes {
            if current.is_empty() {
                starts.push(n.start_tick);
            }
            max_end = max_end.max(n.end_tick);
            current.push(n);
            if current.len() == PROTO_CHUNK {
                max_ends.push(max_end);
                max_end = 0;
                chunks.push(std::mem::replace(
                    &mut current,
                    Vec::with_capacity(PROTO_CHUNK),
                ));
            }
        }
        if !current.is_empty() {
            max_ends.push(max_end);
            chunks.push(current);
        }
        Self {
            chunks,
            starts,
            max_ends,
        }
    }

    fn chunk_index_for(&self, tick: u32) -> usize {
        self.starts
            .partition_point(|&s| s <= tick)
            .saturating_sub(1)
    }

    /// 生产版 `locate_at` 的等价原型。
    fn locate_at(&self, start: u32, track: u16, id: u32) -> Option<(usize, usize)> {
        if self.chunks.is_empty() {
            return None;
        }
        let mut ci = self.chunk_index_for(start);
        while ci > 0
            && self.chunks[ci - 1]
                .last()
                .is_some_and(|n| n.start_tick >= start)
        {
            ci -= 1;
        }
        for (i, chunk) in self.chunks.iter().enumerate().skip(ci) {
            let Some(first) = chunk.first() else {
                continue;
            };
            if first.start_tick > start {
                break;
            }
            let a = chunk.partition_point(|n| n.start_tick < start);
            let b = chunk.partition_point(|n| n.start_tick <= start);
            for (offset, n) in chunk[a..b].iter().enumerate() {
                if n.track == track && n.id == id {
                    return Some((i, a + offset));
                }
            }
        }
        None
    }

    fn refresh_chunk(&mut self, ci: usize) {
        let chunk = &self.chunks[ci];
        self.starts[ci] = chunk[0].start_tick;
        self.max_ends[ci] = chunk.iter().map(|n| n.end_tick).max().unwrap_or(0);
    }

    fn remove_at(&mut self, start: u32, track: u16, id: u32) -> Option<Note> {
        let (ci, idx) = self.locate_at(start, track, id)?;
        let note = self.chunks[ci].remove(idx);
        if self.chunks[ci].is_empty() {
            self.chunks.remove(ci);
            self.starts.remove(ci);
            self.max_ends.remove(ci);
        } else {
            if idx == 0 {
                self.starts[ci] = self.chunks[ci][0].start_tick;
            }
            if note.end_tick == self.max_ends[ci] {
                self.refresh_chunk(ci);
            }
        }
        Some(note)
    }

    fn insert_sorted(&mut self, note: Note) {
        if self.chunks.is_empty() {
            self.starts.push(note.start_tick);
            self.max_ends.push(note.end_tick);
            self.chunks.push(vec![note]);
            return;
        }
        let ci = self.chunk_index_for(note.start_tick);
        let chunk = &mut self.chunks[ci];
        let pos = chunk.partition_point(|n| n.start_tick < note.start_tick);
        chunk.insert(pos, note);
        self.max_ends[ci] = self.max_ends[ci].max(note.end_tick);
        if pos == 0 {
            self.starts[ci] = chunk[0].start_tick;
        }
        if chunk.len() > PROTO_CHUNK {
            let split_at = chunk.len() / 2;
            let right = chunk.split_off(split_at);
            self.starts[ci] = chunk[0].start_tick;
            self.max_ends[ci] = chunk.iter().map(|n| n.end_tick).max().unwrap_or(0);
            let right_max = right.iter().map(|n| n.end_tick).max().unwrap_or(0);
            self.starts.insert(ci + 1, right[0].start_tick);
            self.max_ends.insert(ci + 1, right_max);
            self.chunks.insert(ci + 1, right);
        }
    }

    fn update_end(
        &mut self,
        start: u32,
        track: u16,
        id: u32,
        new_end: u32,
    ) -> Option<(Note, Note)> {
        let (ci, idx) = self.locate_at(start, track, id)?;
        let old = self.chunks[ci][idx];
        self.chunks[ci][idx].end_tick = new_end;
        if new_end > self.max_ends[ci] {
            self.max_ends[ci] = new_end;
        } else if old.end_tick == self.max_ends[ci] && new_end < old.end_tick {
            self.refresh_chunk(ci);
        }
        Some((old, self.chunks[ci][idx]))
    }

    fn update_start_keep_end(
        &mut self,
        start: u32,
        track: u16,
        id: u32,
        new_start: u32,
    ) -> Option<(Note, Note)> {
        let mut note = self.remove_at(start, track, id)?;
        let old = note;
        note.start_tick = new_start.min(note.end_tick.saturating_sub(1));
        self.insert_sorted(note);
        Some((old, note))
    }

    fn max_end(&self) -> u32 {
        self.max_ends.iter().copied().max().unwrap_or(0)
    }
}

struct ProtoStats {
    note_count: u64,
    track_total: u64,
    track_audible: u64,
    bucket_total: u64,
    bucket_track: std::collections::HashMap<u16, (u64, u64)>,
    tick_length: u32,
}

impl ProtoStats {
    fn from_bucket(bucket: &ProtoBucket) -> Self {
        let mut s = Self {
            note_count: 0,
            track_total: 0,
            track_audible: 0,
            bucket_total: 0,
            bucket_track: std::collections::HashMap::new(),
            tick_length: 0,
        };
        for n in bucket.chunks.iter().flatten() {
            s.add(n);
        }
        s.tick_length = bucket.max_end();
        s
    }

    fn add(&mut self, n: &Note) {
        self.note_count += 1;
        self.track_total += 1;
        self.bucket_total += 1;
        if n.velocity > 1 {
            self.track_audible += 1;
        }
        let e = self.bucket_track.entry(n.track).or_insert((0, 0));
        e.0 += 1;
        if n.velocity > 1 {
            e.1 += 1;
        }
    }

    fn remove(&mut self, n: &Note) {
        self.note_count -= 1;
        self.track_total -= 1;
        self.bucket_total -= 1;
        if n.velocity > 1 {
            self.track_audible -= 1;
        }
        if let Some(e) = self.bucket_track.get_mut(&n.track) {
            e.0 -= 1;
            if n.velocity > 1 {
                e.1 -= 1;
            }
        }
    }

    /// 单音符统计增量：before/after 二选一或都有；`new_max_end` 是桶级最大值。
    fn apply(&mut self, before: Option<&Note>, after: Option<&Note>, new_max_end: u32) {
        if let Some(n) = before {
            self.remove(n);
        }
        if let Some(n) = after {
            self.add(n);
        }
        self.tick_length = new_max_end;
    }
}

fn bench_proto_after(count: usize, iters: usize, remove_iters: usize) {
    print_header(&format!(
        "优化后原型：{count} 音符/key（locate_at + 单点搬移 + 增量统计）"
    ));

    let mut rng = 0x0DDB_A11C_0FEE_1234;
    let mut starts: Vec<u32> = (0..count)
        .map(|_| (next_rng(&mut rng) % u64::from(MAX_TICK)) as u32)
        .collect();
    starts.sort_unstable();
    let notes: Vec<Note> = starts
        .iter()
        .enumerate()
        .map(|(i, &start)| Note {
            id: (i + 1) as u32,
            start_tick: start,
            end_tick: start.saturating_add(10),
            velocity: 100,
            track: TRACK,
        })
        .collect();
    let mut bucket = ProtoBucket::from_sorted(notes);
    let mut stats = ProtoStats::from_bucket(&bucket);

    let t0 = Instant::now();
    for _ in 0..iters {
        let i = (next_rng(&mut rng) as usize) % count;
        let start = starts[i];
        let id = (i + 1) as u32;
        let new_end = start + 10 + (next_rng(&mut rng) % 100) as u32;
        let (before, after) = bucket
            .update_end(start, TRACK, id, new_end)
            .expect("update_end 应命中");
        stats.apply(Some(&before), Some(&after), bucket.max_end());
        black_box((before, after));
    }
    print_op("改 end（原型）", t0.elapsed().as_secs_f64() / iters as f64);

    let t1 = Instant::now();
    for _ in 0..iters {
        let i = (next_rng(&mut rng) as usize) % count;
        let old = starts[i];
        let id = (i + 1) as u32;
        let mut new_start = (next_rng(&mut rng) % u64::from(MAX_TICK)) as u32;
        if new_start == old {
            new_start = new_start.saturating_add(1);
        }
        let (before, after) = bucket
            .update_start_keep_end(old, TRACK, id, new_start)
            .expect("左拉伸应命中");
        starts[i] = after.start_tick;
        stats.apply(Some(&before), Some(&after), bucket.max_end());
        black_box((before, after));
    }
    print_op(
        "左拉伸（原型：remove+insert）",
        t1.elapsed().as_secs_f64() / iters as f64,
    );

    let mut used = std::collections::HashSet::with_capacity(remove_iters);
    let mut removed = 0usize;
    let t2 = Instant::now();
    while removed < remove_iters {
        let i = (next_rng(&mut rng) as usize) % count;
        if !used.insert(i) {
            continue;
        }
        let start = starts[i];
        let id = (i + 1) as u32;
        let before = bucket.remove_at(start, TRACK, id).expect("删除应命中");
        stats.apply(Some(&before), None, bucket.max_end());
        removed += 1;
        black_box(before);
    }
    print_op(
        "删除（原型）",
        t2.elapsed().as_secs_f64() / remove_iters as f64,
    );

    println!(
        "  原型说明：Vec<Vec<Note>>，无 Arc COW；真实编辑若音频持有快照会多一次 ≤1MB 块复制。"
    );
    println!(
        "  统计结果自检：notes={} bucket={} track={} audible={} tick={}",
        stats.note_count,
        stats.bucket_total,
        stats.track_total,
        stats.track_audible,
        stats.tick_length
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--delta-huge") {
        println!("note_hotpath_bench: build mode = release, delta huge = true");
        bench_delta(100_000_000);
        println!();
        println!("done");
        return;
    }
    let quick = args.iter().any(|a| a == "--quick");
    println!("note_hotpath_bench: build mode = release, quick = {quick}");

    bench_model(1_000_000, 100, 100, 3);
    bench_proto_after(1_000_000, 100, 100);
    bench_delta(1_000_000);

    if !quick {
        bench_model(10_000_000, 10, 10, 1);
        bench_proto_after(10_000_000, 10, 10);
        bench_delta(10_000_000);
    }

    println!();
    println!("done");
}
