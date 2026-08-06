//! 对比基准：单 Vec 桶（现状）vs 65536 音符/块结构（ChunkedVec）。
//!
//! 覆盖四个场景：
//! 1. 单点插入 1M 音符/key（128M 工程的均匀分布假设）
//! 2. 单点插入 10M 音符/key（黑乐谱单 key 极端密集）
//! 3. 批量插入 1M 音符/key + 10 万新音符（归并路径）
//! 4. 窄窗口区间查询（命中测试）
//!
//! 运行：cargo bench -p yinhe-core --bench chunked_bench
//!
//! 单 Vec 侧的实现与生产代码一致：
//! - 单点插入 = `partition_point` + `Vec::insert`（yinhe-editor-core add_note 路径）
//! - 批量插入 = 排序 + 双路归并（batch_ops::append_notes_ordered 路径）

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use yinhe_core::{NoteEvent, TrackData, YinModel};
use yinhe_types::Note;

/// 65536 音符/块 = 1MB/块（Note 16B）。
/// u16 块内偏移恰好够用（0..=65535）；块数上限 65536/key → 单 key 42 亿音符。
const CHUNK_CAP: usize = 65536;

// ---------------------------------------------------------------------------
// ChunkedVec：按 start_tick 分块的有序序列（最小实现，只含 bench 所需操作）
// ---------------------------------------------------------------------------

struct ChunkedVec {
    /// 块间有序：chunks[i] 的所有元素 start_tick <= chunks[i+1] 的任意元素。
    chunks: Vec<Vec<Note>>,
    /// 每块首元素的 start_tick，用于块级二分定位。
    starts: Vec<u32>,
}

impl ChunkedVec {
    fn from_sorted(notes: Vec<Note>) -> Self {
        let mut chunks = Vec::new();
        let mut starts = Vec::new();
        for part in notes.chunks(CHUNK_CAP) {
            starts.push(part[0].start_tick);
            chunks.push(part.to_vec());
        }
        if chunks.is_empty() {
            // 空结构也保留一个空块，保证 insert_sorted 的定位路径永远命中。
            chunks.push(Vec::new());
            starts.push(0);
        }
        Self { chunks, starts }
    }

    /// 块级二分定位 + 块内二分插入 + 满则均分（无合并阈值）。
    fn insert_sorted(&mut self, note: Note) {
        let idx = self
            .starts
            .partition_point(|&s| s <= note.start_tick)
            .saturating_sub(1);
        let chunk = &mut self.chunks[idx];
        let pos = chunk.partition_point(|n| n.start_tick < note.start_tick);
        chunk.insert(pos, note);
        if pos == 0 {
            // 插到块头改变了块首元素，starts 必须同步（否则后续块级二分失序）。
            self.starts[idx] = chunk[0].start_tick;
        }
        if chunk.len() > CHUNK_CAP {
            let split_at = chunk.len() / 2;
            let right = chunk.split_off(split_at);
            self.starts.insert(idx + 1, right[0].start_tick);
            self.chunks.insert(idx + 1, right);
        }
    }

    /// 批量插入：新音符排序后，按块切分逐块双路归并，O(N + K)。
    fn insert_batch_sorted(&mut self, mut new: Vec<Note>) {
        if new.is_empty() {
            return;
        }
        new.sort_by_key(|n| n.start_tick);
        let mut result: Vec<Vec<Note>> = Vec::with_capacity(self.chunks.len() + 1);
        let mut starts: Vec<u32> = Vec::with_capacity(result.capacity());
        let mut cursor = 0usize;
        for chunk in &self.chunks {
            // 首元素落在本块区间（<= 本块尾）的新音符归入本块一起归并。
            let end = chunk
                .last()
                .map(|last| {
                    new[cursor..].partition_point(|n| n.start_tick <= last.start_tick) + cursor
                })
                .unwrap_or(cursor);
            let merged = merge_runs(chunk, &new[cursor..end]);
            cursor = end;
            for part in merged.chunks(CHUNK_CAP) {
                starts.push(part[0].start_tick);
                result.push(part.to_vec());
            }
        }
        // 超出旧块范围的新音符，直接成块。
        for part in new[cursor..].chunks(CHUNK_CAP) {
            starts.push(part[0].start_tick);
            result.push(part.to_vec());
        }
        self.chunks = result;
        self.starts = starts;
    }

    /// 窄窗口查询：块级定位起止块，只扫命中的块（模拟 key_notes_in_range）。
    fn range_count(&self, lo_tick: u32, hi_tick: u32) -> usize {
        if self.starts.is_empty() || lo_tick >= hi_tick {
            return 0;
        }
        let lo_chunk = self
            .starts
            .partition_point(|&s| s <= lo_tick)
            .saturating_sub(1);
        let hi_chunk = self
            .starts
            .partition_point(|&s| s < hi_tick)
            .saturating_sub(1);
        let mut count = 0;
        for i in lo_chunk..=hi_chunk.min(self.chunks.len() - 1) {
            let chunk = &self.chunks[i];
            let lo = chunk.partition_point(|n| n.start_tick < lo_tick);
            let hi = chunk.partition_point(|n| n.start_tick < hi_tick);
            count += hi - lo;
        }
        count
    }
}

/// 双路归并两个按 start_tick 有序的切片（稳定：old 优先于 new）。
fn merge_runs(old: &[Note], new: &[Note]) -> Vec<Note> {
    let mut merged = Vec::with_capacity(old.len() + new.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() && j < new.len() {
        if old[i].start_tick <= new[j].start_tick {
            merged.push(old[i]);
            i += 1;
        } else {
            merged.push(new[j]);
            j += 1;
        }
    }
    merged.extend_from_slice(&old[i..]);
    merged.extend_from_slice(&new[j..]);
    merged
}

// ---------------------------------------------------------------------------
// 现状实现（与生产代码同构）
// ---------------------------------------------------------------------------

/// add_note 的单点插入路径：partition_point + Vec::insert。
fn vec_insert_sorted(bucket: &mut Vec<Note>, note: Note) {
    let pos = bucket.partition_point(|n| n.start_tick < note.start_tick);
    bucket.insert(pos, note);
}

/// batch_ops::append_notes_ordered：排序 + 尾部快路径 / 双路归并。
fn vec_append_ordered(bucket: &mut Vec<Note>, mut new: Vec<Note>) {
    if new.is_empty() {
        return;
    }
    new.sort_by_key(|n| n.start_tick);
    let tail_ok = bucket
        .last()
        .is_none_or(|last| last.start_tick <= new[0].start_tick);
    if tail_ok {
        bucket.extend(new);
        return;
    }
    let old = std::mem::take(bucket);
    let mut merged = Vec::with_capacity(old.len() + new.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() && j < new.len() {
        if old[i].start_tick <= new[j].start_tick {
            merged.push(old[i]);
            i += 1;
        } else {
            merged.push(new[j]);
            j += 1;
        }
    }
    merged.extend_from_slice(&old[i..]);
    merged.extend_from_slice(&new[j..]);
    *bucket = merged;
}

/// key_notes_in_range：两个 partition_point 定位区间。
fn vec_range_count(bucket: &[Note], lo_tick: u32, hi_tick: u32) -> usize {
    let lo = bucket.partition_point(|n| n.start_tick < lo_tick);
    let hi = bucket.partition_point(|n| n.start_tick < hi_tick);
    hi - lo
}

// ---------------------------------------------------------------------------
// 数据生成（splitmix64，避免引入 rand 依赖）
// ---------------------------------------------------------------------------

fn next_rng(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// 生成 count 个 start_tick 在 [0, max_tick) 的音符，按 start_tick 排序，id 唯一。
fn make_notes(count: usize, max_tick: u32, seed: u64) -> Vec<Note> {
    let mut rng = seed;
    let mut notes: Vec<Note> = (0..count)
        .map(|i| Note {
            id: i as u32,
            start_tick: (next_rng(&mut rng) % max_tick as u64) as u32,
            end_tick: 0, // bench 只关心 start_tick 排序键
            velocity: 100,
            track: 0,
        })
        .collect();
    notes.sort_by_key(|n| n.start_tick);
    notes
}

// ---------------------------------------------------------------------------
// 场景
// ---------------------------------------------------------------------------

fn bench_single_insert(notes_count: usize, insert_count: usize, label: &str) {
    let max_tick = 10_000_000u32;
    let notes = make_notes(notes_count, max_tick, 42);
    let inserts = make_notes(insert_count, max_tick, 7);

    let mut v = notes.clone();
    let mut c = ChunkedVec::from_sorted(notes);

    let t0 = Instant::now();
    for &n in &inserts {
        vec_insert_sorted(&mut v, n);
    }
    let d0 = t0.elapsed();

    let t1 = Instant::now();
    for &n in &inserts {
        c.insert_sorted(n);
    }
    let d1 = t1.elapsed();

    let v_per = d0.as_secs_f64() / insert_count as f64;
    let c_per = d1.as_secs_f64() / insert_count as f64;
    println!();
    println!(
        "== 单点插入 {} 音符/key，{insert_count} 次随机落笔（{label}） ==",
        notes_count
    );
    println!("  Vec          {:>10.2} µs/op", v_per * 1e6);
    println!("  ChunkedVec   {:>10.2} µs/op", c_per * 1e6);
    println!(
        "  加速比        {:>10.1}x   (每次 memmove: Vec={}B, ChunkedVec<=1MB)",
        v_per / c_per,
        notes_count as u64 / 2 * 16
    );
    black_box((v.len(), c.chunks.len()));
}

fn bench_batch_insert() {
    let max_tick = 10_000_000u32;
    let base = make_notes(1_000_000, max_tick, 42);
    let batch = make_notes(100_000, max_tick, 7);

    let mut v = base.clone();
    let mut c = ChunkedVec::from_sorted(base);

    let t0 = Instant::now();
    vec_append_ordered(&mut v, batch.clone());
    let d0 = t0.elapsed();

    let t1 = Instant::now();
    c.insert_batch_sorted(batch);
    let d1 = t1.elapsed();

    println!();
    println!("== 批量插入：1M 音符/key + 10 万新音符（归并路径） ==");
    println!("  Vec          {:>10.2} ms/op", d0.as_secs_f64() * 1e3);
    println!("  ChunkedVec   {:>10.2} ms/op", d1.as_secs_f64() * 1e3);
    println!(
        "  加速比        {:>10.1}x",
        d0.as_secs_f64() / d1.as_secs_f64()
    );
    black_box((v.len(), c.chunks.len()));
}

/// 真实生产路径：YinModel（块化 NoteBucket）+ `add_note` 的单点插入核心
/// （Arc::make_mut + insert_sorted + mark_dirty），单 key 密集分布。
fn bench_real_model_insert(count: usize, iters: usize, label: &str) {
    let max_tick = 10_000_000u32;
    let per_track: Vec<Vec<NoteEvent>> = vec![
        (0..count)
            .map(|i| NoteEvent {
                id: 0,
                start_tick: (i as u64 * max_tick as u64 / count as u64) as u32,
                end_tick: 0,
                key: 60,
                velocity: 100,
            })
            .collect(),
    ];
    let mut model = YinModel {
        tracks: vec![Arc::new(TrackData::new(0, 0))],
        ..Default::default()
    };
    model.load_track_notes(per_track);
    model.rebuild();

    // 随机插入序列：id 避开已分配的 1..=count（避免与 remove_by_id/find_mut 冲突）。
    let inserts: Vec<Note> = make_notes(iters, max_tick, 7)
        .into_iter()
        .enumerate()
        .map(|(i, mut n)| {
            n.id = (count + i) as u32;
            n
        })
        .collect();

    let t0 = Instant::now();
    for n in inserts {
        Arc::make_mut(&mut model.notes[60]).insert_sorted(n);
        model.mark_dirty(60);
    }
    let d0 = t0.elapsed();
    let per = d0.as_secs_f64() / iters as f64;
    println!();
    println!("== 真实 YinModel（块化）单点插入：{count} 音符/key，{iters} 次（{label}） ==");
    println!("  NoteBucket    {:>10.2} µs/op", per * 1e6);
    black_box(model.notes[60].len());
}

fn bench_range_query() {
    let max_tick = 10_000_000u32;
    let notes = make_notes(1_000_000, max_tick, 42);
    let v = notes.clone();
    let c = ChunkedVec::from_sorted(notes);

    // 10 万次窄窗口查询（宽 5000 tick ≈ 100 音符/视口），位置随机。
    let queries: Vec<(u32, u32)> = {
        let mut rng = 99;
        (0..100_000)
            .map(|_| {
                let lo = (next_rng(&mut rng) % (max_tick as u64 - 5000)) as u32;
                (lo, lo + 5000)
            })
            .collect()
    };

    let t0 = Instant::now();
    let mut n0 = 0usize;
    for &(lo, hi) in &queries {
        n0 += vec_range_count(&v, lo, hi);
    }
    let d0 = t0.elapsed();

    let t1 = Instant::now();
    let mut n1 = 0usize;
    for &(lo, hi) in &queries {
        n1 += c.range_count(lo, hi);
    }
    let d1 = t1.elapsed();

    println!();
    println!("== 区间查询：1M 音符/key，10 万次窄窗口命中 ==");
    println!(
        "  Vec          {:>10.2} µs/op",
        d0.as_secs_f64() / 100_000.0 * 1e6
    );
    println!(
        "  ChunkedVec   {:>10.2} µs/op",
        d1.as_secs_f64() / 100_000.0 * 1e6
    );
    println!(
        "  加速比        {:>10.1}x   (两边计数一致: {})",
        d0.as_secs_f64() / d1.as_secs_f64(),
        n0 == n1
    );
    black_box(n0 + n1);
}

fn main() {
    bench_single_insert(1_000_000, 1000, "128M 工程 / 128 key");
    bench_single_insert(10_000_000, 100, "黑乐谱单 key 极端密集");
    bench_batch_insert();
    bench_range_query();
    bench_real_model_insert(1_000_000, 1000, "128M 工程 / 128 key");
    bench_real_model_insert(10_000_000, 100, "黑乐谱单 key 极端密集");
}
