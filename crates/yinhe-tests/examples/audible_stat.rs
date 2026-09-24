//! 统计音符力度分布与 audible_notes 规模。
//! 用法: cargo run --release -p yinhe-tests --example audible_stat -- <yin或mid路径> [ignore_velocity]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/tmp/start_v9.yin".to_string());
    let ignore: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    let model = if path.ends_with(".yin") {
        yinhe_yin::load_yin(&path).expect("load yin")
    } else {
        let bytes = std::fs::read(&path).expect("read midi");
        yinhe_midi::parse_bytes(&bytes).expect("parse midi")
    };

    let mut hist = [0u64; 128];
    let mut total = 0u64;
    for bucket in model.notes.iter() {
        for n in bucket.iter() {
            hist[n.velocity.min(127) as usize] += 1;
            total += 1;
        }
    }
    let audible: u64 = hist
        .iter()
        .enumerate()
        .filter(|(v, _)| *v as u8 > ignore)
        .map(|(_, c)| *c)
        .sum();
    let bytes_audible = audible as f64 * 12.0;

    println!("midi: {path}");
    println!("总音符: {total}");
    println!("ignore_velocity = {ignore}");
    println!(
        "audible (vel > {ignore}): {audible}（{:.2}%），AudibleNote 12B × {audible} = {:.2} GB",
        audible as f64 / total.max(1) as f64 * 100.0,
        bytes_audible / (1u64 << 30) as f64
    );
    println!("\n力度直方图（仅非零桶）:");
    for (v, c) in hist.iter().enumerate() {
        if *c > 0 {
            println!("  vel {v:>3}: {c}");
        }
    }
}
