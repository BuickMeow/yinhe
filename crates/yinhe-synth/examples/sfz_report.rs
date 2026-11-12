//! SFZ 语义体检：解析一个音色库，打印门控/触发/缺失样本统计。
//!
//! 用途：确认 `xfin_*`、`trigger=release`、`seq_*`、`pitch_random` 等
//! 「决定像不像真乐器」的 opcode 真的进了引擎，而不是被静默丢弃。
//!
//! ```sh
//! cargo run --release -p yinhe-synth --example sfz_report -- <a.sfz>
//! ```

use std::path::Path;

use yinhe_synth::sfz;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("用法: sfz_report <file.sfz>");
        std::process::exit(2);
    };
    let path = Path::new(&path);
    let t = std::time::Instant::now();
    let file = match sfz::parse(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("解析失败: {e}");
            std::process::exit(1);
        }
    };
    let ms = t.elapsed().as_millis();

    let total = file.regions.len();
    let release = file
        .regions
        .iter()
        .filter(|r| r.trigger == sfz::Trigger::Release)
        .count();
    let with_cc_gate = file
        .regions
        .iter()
        .filter(|r| !r.cc_gates.is_empty())
        .count();
    let with_on_gate = file
        .regions
        .iter()
        .filter(|r| !r.on_gates.is_empty())
        .count();
    let with_seq = file.regions.iter().filter(|r| r.seq_length > 0).count();
    let with_pitch_random = file.regions.iter().filter(|r| r.pitch_random > 0.0).count();
    let with_amp_lfo = file.regions.iter().filter(|r| r.amplfo.is_some()).count();
    let with_pitch_lfo = file.regions.iter().filter(|r| r.pitchlfo.is_some()).count();
    let one_shot = file
        .regions
        .iter()
        .filter(|r| r.loop_mode == sfz::LoopMode::OneShot)
        .count();
    let with_sw = file.regions.iter().filter(|r| r.sw_last.is_some()).count();
    let with_off_by = file.regions.iter().filter(|r| r.off_by > 0).count();

    println!("文件: {}", path.display());
    println!("解析耗时: {ms} ms");
    println!("region: {total}");
    println!("  释放触发 (trigger=release): {release}");
    println!("  CC 淡入淡出 (xfin/xfout): {with_cc_gate}");
    println!("  CC 区间门 (on_locc): {with_on_gate}");
    println!("  round-robin (seq_*): {with_seq}");
    println!("  keyswitch (sw_*): {with_sw}");
    println!("  off_by: {with_off_by}");
    println!("  one_shot: {one_shot}");
    println!("  pitch_random: {with_pitch_random}");
    println!("  amplfo: {with_amp_lfo}");
    println!("  pitchlfo: {with_pitch_lfo}");
    println!("缺失采样: {}", file.missing_samples);
    println!("通道默认 CC: {:?}", file.cc_defaults);

    if !file.unknown_opcodes.is_empty() {
        let mut items: Vec<_> = file.unknown_opcodes.iter().collect();
        items.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        println!("\n未识别 opcode（前 20）:");
        for (name, count) in items.into_iter().take(20) {
            println!("  {name} ×{count}");
        }
    }
}
