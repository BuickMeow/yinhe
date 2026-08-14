//! 一次性实验：把 start.mid 用当前代码（yinhe-mid2 + yinhe-yin v4）转成 .yin。
//! 用法: cargo run --release -p yinhe-tests --example convert_midi -- <mid路径> <yin输出>

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mid = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/Users/jieneng/Music/MIDIs/start.mid".to_string());
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "/Users/jieneng/Music/MIDIs/start_v4.yin".to_string());

    let model = yinhe_midi::parse_path(&mid).expect("parse mid");
    yinhe_yin::save_yin(&model, &out).expect("save yin");
    println!("saved {out}");
}
