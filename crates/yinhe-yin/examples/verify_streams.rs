//! 验证：用 bincode 库本身反序列化导出的各列流。
fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/yin_exp".to_string());
    use bincode::Options;
    let opt = bincode::DefaultOptions::new()
        .with_varint_encoding()
        .with_little_endian();
    for name in ["meta", "delta", "key", "track", "vel", "gate"] {
        let data = std::fs::read(format!("{dir}/l3_{name}.bin")).unwrap();
        let t = std::time::Instant::now();
        match name {
            "key" | "vel" => {
                let v: Vec<u8> = opt.deserialize(&data).unwrap();
                println!("{name}: len={} 耗时 {:?}", v.len(), t.elapsed());
            }
            "delta" | "gate" => {
                let v: Vec<u32> = opt.deserialize(&data).unwrap();
                println!("{name}: len={} 耗时 {:?}", v.len(), t.elapsed());
            }
            "track" => {
                let v: Vec<u16> = opt.deserialize(&data).unwrap();
                println!("{name}: len={} 耗时 {:?}", v.len(), t.elapsed());
            }
            _ => println!("{name}: 跳过"),
        }
    }
}
