//! 实验：比较 .yin 音符数据几种布局在不同压缩器下的体积（含 note id 列开销）。
//! 用法: cargo run --release -p yinhe-yin --example layout_exp -- <mid路径> <输出目录>
//!
//! 输出 raw 流文件供外部压缩器对比：
//! - A_global_cols: 全局 (start,track,key) 排序 + 5 列独立（当前 v7 同款）+ A_id*
//! - B_track_interleave: (track,start,key) 排序 + 单流交错
//! - C_track_cols: (track,start,key) 排序 + 4 列独立（track 段头）+ C_id*
//!
//! id 列三种编码：u64 定长（Lumino 同款 bincode）、u32 varint、zigzag varint delta。

use std::io::Write;
use std::time::Instant;

fn push_varint(out: &mut Vec<u8>, v: u64) {
    let mut v = v;
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

struct NoteRow {
    id: u32,
    start: u32,
    gate: u32,
    track: u16,
    key: u8,
    vel: u8,
}

fn collect(model: &yinhe_core::YinModel) -> Vec<NoteRow> {
    let total: usize = model.notes.iter().map(|b| b.len()).sum();
    let mut v = Vec::with_capacity(total);
    for (key, bucket) in model.notes.iter().enumerate() {
        for n in bucket.iter() {
            v.push(NoteRow {
                id: n.id,
                start: n.start_tick,
                gate: n.end_tick.saturating_sub(n.start_tick),
                track: n.track,
                key: key as u8,
                vel: n.velocity,
            });
        }
    }
    v
}

fn dump(name: &str, out_dir: &str, bytes: &[u8]) -> usize {
    let path = format!("{out_dir}/{name}.bin");
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(bytes).expect("write");
    println!(
        "{name}: {:.2} MiB",
        bytes.len() as f64 / (1u64 << 20) as f64
    );
    bytes.len()
}

/// id 列三种编码 + 单调性统计。
fn dump_id_cols(name: &str, out_dir: &str, v: &[NoteRow], idx: &[u32]) {
    let n = idx.len();
    // u64 定长（Lumino bincode 同款）
    let mut fixed = Vec::with_capacity(n * 8);
    for &i in idx {
        fixed.extend_from_slice(&(v[i as usize].id as u64).to_le_bytes());
    }
    dump(&format!("{name}_id_u64"), out_dir, &fixed);

    // u32 varint
    let mut vr = Vec::with_capacity(n);
    for &i in idx {
        push_varint(&mut vr, v[i as usize].id as u64);
    }
    dump(&format!("{name}_id_varint"), out_dir, &vr);

    // zigzag delta varint
    let mut dl = Vec::with_capacity(n);
    let mut prev = 0i64;
    let mut back = 0u64;
    let mut fwd = 0u64;
    for &i in idx {
        let id = v[i as usize].id as i64;
        let d = id - prev;
        if d < 0 {
            back += 1;
        } else {
            fwd += 1;
        }
        push_varint(&mut dl, zigzag(d));
        prev = id;
    }
    dump(&format!("{name}_id_delta"), out_dir, &dl);
    println!(
        "  [{name}] id 相邻差值: 递增 {fwd}, 回退 {back} (回退率 {:.4}%)",
        back as f64 / n as f64 * 100.0
    );
}

/// A：全局 (start,track,key) 排序，5 列独立。
fn global_cols(v: &[NoteRow], out_dir: &str) {
    let t = Instant::now();
    let mut idx: Vec<u32> = (0..v.len() as u32).collect();
    idx.sort_unstable_by_key(|&i| {
        let n = &v[i as usize];
        (n.start, n.track, n.key)
    });

    let n = v.len();
    let mut delta = Vec::with_capacity(n);
    let mut key = Vec::with_capacity(n);
    let mut track = Vec::with_capacity(n);
    let mut vel = Vec::with_capacity(n);
    let mut gate = Vec::with_capacity(n);
    let mut prev = 0u32;
    for (i, &ix) in idx.iter().enumerate() {
        let r = &v[ix as usize];
        delta.push(if i == 0 { r.start } else { r.start - prev });
        key.push(r.key);
        track.push(r.track);
        vel.push(r.vel);
        gate.push(r.gate);
        prev = r.start;
    }
    dump("A_delta", out_dir, &postcard::to_stdvec(&delta).unwrap());
    dump("A_key", out_dir, &postcard::to_stdvec(&key).unwrap());
    dump("A_track", out_dir, &postcard::to_stdvec(&track).unwrap());
    dump("A_vel", out_dir, &postcard::to_stdvec(&vel).unwrap());
    dump("A_gate", out_dir, &postcard::to_stdvec(&gate).unwrap());
    dump_id_cols("A", out_dir, v, &idx);
    println!("A 排序+编码: {:?}", t.elapsed());
}

/// B：(track,start,key) 排序，单流交错；换轨时写 [track varint]。
fn track_interleave(v: &[NoteRow], out_dir: &str) {
    let t = Instant::now();
    let mut idx: Vec<u32> = (0..v.len() as u32).collect();
    idx.sort_unstable_by_key(|&i| {
        let n = &v[i as usize];
        (n.track, n.start, n.key)
    });

    let mut out = Vec::with_capacity(v.len() * 4);
    let mut cur_track = u16::MAX;
    let mut prev_start = 0u32;
    for &ix in &idx {
        let n = &v[ix as usize];
        if n.track != cur_track {
            cur_track = n.track;
            prev_start = 0;
            push_varint(&mut out, n.track as u64);
        }
        push_varint(&mut out, (n.start - prev_start) as u64);
        out.push(n.key);
        out.push(n.vel);
        push_varint(&mut out, n.gate as u64);
        prev_start = n.start;
    }
    dump("B_interleave", out_dir, &out);
    println!("B 排序+编码: {:?}", t.elapsed());
}

/// C：(track,start,key) 排序，4 列独立；track 列表单独写。
fn track_cols(v: &[NoteRow], out_dir: &str) {
    let t = Instant::now();
    let mut idx: Vec<u32> = (0..v.len() as u32).collect();
    idx.sort_unstable_by_key(|&i| {
        let n = &v[i as usize];
        (n.track, n.start, n.key)
    });

    let n = v.len();
    let mut delta = Vec::with_capacity(n);
    let mut key = Vec::with_capacity(n);
    let mut vel = Vec::with_capacity(n);
    let mut gate = Vec::with_capacity(n);
    let mut tracks: Vec<u16> = Vec::new();
    let mut cur_track = u16::MAX;
    let mut prev_start = 0u32;
    for &ix in &idx {
        let r = &v[ix as usize];
        if r.track != cur_track {
            cur_track = r.track;
            prev_start = 0;
            tracks.push(r.track);
        }
        delta.push(r.start - prev_start);
        key.push(r.key);
        vel.push(r.vel);
        gate.push(r.gate);
        prev_start = r.start;
    }
    dump("C_delta", out_dir, &postcard::to_stdvec(&delta).unwrap());
    dump("C_key", out_dir, &postcard::to_stdvec(&key).unwrap());
    dump("C_vel", out_dir, &postcard::to_stdvec(&vel).unwrap());
    dump("C_gate", out_dir, &postcard::to_stdvec(&gate).unwrap());
    dump("C_tracks", out_dir, &postcard::to_stdvec(&tracks).unwrap());
    dump_id_cols("C", out_dir, v, &idx);
    println!("C 排序+编码: {:?}", t.elapsed());
}

fn main() {
    let mid = std::env::args()
        .nth(1)
        .expect("用法: layout_exp <mid> <out_dir>");
    let out_dir = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/yin_layout".to_string());
    std::fs::create_dir_all(&out_dir).expect("mkdir");

    let bytes = std::fs::read(&mid).expect("read mid");
    println!("mid: {:.1} MiB", bytes.len() as f64 / (1u64 << 20) as f64);
    let t = Instant::now();
    let model = yinhe_midi::parse_bytes(&bytes).expect("parse");
    println!("parse: {:?}", t.elapsed());
    drop(bytes);
    let v = collect(&model);
    println!("notes: {}, tracks: {}", v.len(), model.tracks.len());
    drop(model);

    global_cols(&v, &out_dir);
    track_interleave(&v, &out_dir);
    track_cols(&v, &out_dir);
    println!("输出目录: {out_dir}");
}
