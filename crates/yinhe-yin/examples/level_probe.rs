//! 诊断：同一模型在不同 zstd 级别下的 .yin 体积与耗时。
//! 用法: cargo run --release -p yinhe-yin --example level_probe -- <mid路径>
fn main() {
    let path = std::env::args().nth(1).expect("需要 .mid 路径");
    let m = yinhe_midi::parse_path(&path).expect("解析失败");
    println!(
        "{}: notes={} tracks={} tick_len={}",
        path.rsplit('/').next().unwrap_or(&path),
        m.note_count,
        m.tracks.len(),
        m.tick_length
    );
    for lvl in [1, 3, 6, 9, 15, 19] {
        let mut model = m.clone();
        model.meta.compression_level = lvl;
        let t = std::time::Instant::now();
        match yinhe_yin::save_yin_bytes(&model) {
            Ok(bytes) => println!(
                "  level {:>2}: {:7.2}MB  用时 {:?}",
                lvl,
                bytes.len() as f64 / 1e6,
                t.elapsed()
            ),
            Err(e) => println!("  level {lvl}: ERR {e}"),
        }
    }
}
