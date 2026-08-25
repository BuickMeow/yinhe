//! 实验：把 yin v5 的 5 个音符列交错回"事件流"形态，对比压缩率。
//! 用法: cargo run --release -p yinhe-yin --example interleave -- <dir>
//! 读 <dir>/l3_{delta,key,track,vel,gate}.bin（postcard 列），
//! 交错写出 interleaved.bin：每音符 [delta varint][key u8][track varint][vel u8][gate varint]。

fn push_varint(out: &mut Vec<u8>, v: u64) {
    if v <= 250 {
        out.push(v as u8);
    } else if v <= u16::MAX as u64 {
        out.push(251);
        out.extend_from_slice(&(v as u16).to_le_bytes());
    } else if v <= u32::MAX as u64 {
        out.push(252);
        out.extend_from_slice(&(v as u32).to_le_bytes());
    } else {
        out.push(253);
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/yin_exp".to_string());
    let read_bin = |name: &str| std::fs::read(format!("{dir}/{name}")).expect(name);

    let t = std::time::Instant::now();
    let delta: Vec<u32> = postcard::from_bytes(&read_bin("l3_delta.bin")).unwrap();
    let key: Vec<u8> = postcard::from_bytes(&read_bin("l3_key.bin")).unwrap();
    let track: Vec<u16> = postcard::from_bytes(&read_bin("l3_track.bin")).unwrap();
    let vel: Vec<u8> = postcard::from_bytes(&read_bin("l3_vel.bin")).unwrap();
    let gate: Vec<u32> = postcard::from_bytes(&read_bin("l3_gate.bin")).unwrap();
    let n = delta.len();
    assert_eq!(key.len(), n);
    assert_eq!(track.len(), n);
    assert_eq!(vel.len(), n);
    assert_eq!(gate.len(), n);
    println!("音符数 {n}, 解码耗时 {:?}", t.elapsed());

    let t = std::time::Instant::now();
    let mut out = Vec::with_capacity(n * 6);
    for i in 0..n {
        push_varint(&mut out, delta[i] as u64);
        out.push(key[i]);
        push_varint(&mut out, track[i] as u64);
        out.push(vel[i]);
        push_varint(&mut out, gate[i] as u64);
    }
    let path = format!("{dir}/interleaved.bin");
    std::fs::write(&path, &out).expect("write");
    println!(
        "interleaved.bin: {:.2} MiB, 耗时 {:?}",
        out.len() as f64 / (1u64 << 20) as f64,
        t.elapsed()
    );
}
