//! 实验：测量 bincode varint 序列化 5 个音符列（1.64 亿元素）的耗时。
//! 用法: cargo run --release -p yinhe-yin --example bench_serialize

use bincode::Options;

fn main() {
    let opt = bincode::DefaultOptions::new()
        .with_varint_encoding()
        .with_little_endian();
    let n = 164_203_965usize;
    // 代表性数据：delta 大部分 0；key 0..127 循环；track 0..798 循环；vel/gate 常数
    let delta: Vec<u32> = (0..n)
        .map(|i| {
            if i % 128 == 0 {
                ((i / 128) % 480) as u32
            } else {
                0
            }
        })
        .collect();
    let key: Vec<u8> = (0..n).map(|i| (i % 128) as u8).collect();
    let track: Vec<u16> = (0..n).map(|i| ((i / 128) % 799) as u16).collect();
    let vel: Vec<u8> = vec![100u8; n];
    let gate: Vec<u32> = vec![480u32; n];

    let t = std::time::Instant::now();
    let b = opt.serialize(&delta).unwrap();
    println!(
        "delta 序列化: {:?} -> {} MiB",
        t.elapsed(),
        b.len() / (1 << 20)
    );
    let t = std::time::Instant::now();
    let b = opt.serialize(&key).unwrap();
    println!(
        "key 序列化: {:?} -> {} MiB",
        t.elapsed(),
        b.len() / (1 << 20)
    );
    let t = std::time::Instant::now();
    let b = opt.serialize(&track).unwrap();
    println!(
        "track 序列化: {:?} -> {} MiB",
        t.elapsed(),
        b.len() / (1 << 20)
    );
    let t = std::time::Instant::now();
    let b = opt.serialize(&vel).unwrap();
    println!(
        "vel 序列化: {:?} -> {} MiB",
        t.elapsed(),
        b.len() / (1 << 20)
    );
    let t = std::time::Instant::now();
    let b = opt.serialize(&gate).unwrap();
    println!(
        "gate 序列化: {:?} -> {} MiB",
        t.elapsed(),
        b.len() / (1 << 20)
    );
}
