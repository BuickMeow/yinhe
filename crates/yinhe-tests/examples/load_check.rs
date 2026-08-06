//! 一次性验证：加载 v5 .yin，打印音符数/轨道数，确认可二次打开。
//! 用法: cargo run --release -p yinhe-tests --example load_check -- <yin路径>

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/Users/jieneng/Music/MIDIs/start_v5.yin".to_string());

    let start = std::time::Instant::now();
    let (model, _sf, _mapping) = yinhe_yin::load_yin_with_sf(&path).expect("load yin");
    println!(
        "loaded: {} notes, {} tracks, {:.1}s",
        model.note_count,
        model.tracks.len(),
        start.elapsed().as_secs_f64()
    );
}
