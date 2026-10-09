//! key map 内存基准：加载音色库，打印 `KeyInfo` 大小与 key map 的结构内存。
//!
//! 用法（需显式传音色库路径）：
//! ```sh
//! cargo run --release -p yinhe-synth --example keymap_size -- <a.sfz|sf2> [...]
//! ```

use std::collections::HashSet;

use yinhe_synth::{KeyInfo, build_key_maps};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("用法: keymap_size <a.sfz|sf2> [...]");
        std::process::exit(2);
    }

    let sr = 48_000u32;
    let interp = 1u32;
    let ki_size = std::mem::size_of::<KeyInfo>();

    let mut total_ki = 0usize;
    let mut total_map_bytes = 0usize;
    let mut sample_bytes = 0usize;
    let mut seen: HashSet<usize> = HashSet::new();
    let mut entries_total = 0usize;

    for p in &paths {
        let t = std::time::Instant::now();
        let entries = build_key_maps(std::path::Path::new(p), sr, interp)
            .unwrap_or_else(|e| panic!("load {p}: {e}"));
        let mut ki = 0usize;
        let mut map_bytes = 0usize;
        for entry in &entries {
            // 外层 map: 128 个 Vec<KeyInfo> 头（ptr/len/cap = 24B）
            map_bytes += 128 * std::mem::size_of::<Vec<KeyInfo>>();
            for key_layers in entry.map() {
                ki += key_layers.len();
                map_bytes += key_layers.capacity() * ki_size;
                for info in key_layers {
                    let ptr = info.sample_data.as_ptr() as usize;
                    if seen.insert(ptr) {
                        sample_bytes += info.sample_data.len() * std::mem::size_of::<f32>();
                    }
                }
            }
            map_bytes += std::mem::size_of::<yinhe_synth::KeyMapEntry>();
        }
        total_ki += ki;
        total_map_bytes += map_bytes;
        entries_total += entries.len();
        eprintln!(
            "{p}: {:?}, {} entries, {} KeyInfo, map≈{:.1}MB",
            t.elapsed(),
            entries.len(),
            ki,
            map_bytes as f64 / 1048576.0
        );
    }

    println!("KeyInfo size = {ki_size} B");
    println!("entries = {entries_total}");
    println!("KeyInfo count = {total_ki}");
    println!(
        "key map 结构内存 ≈ {:.1} MB（KeyInfo {}×{}B + Vec 头）",
        total_map_bytes as f64 / 1048576.0,
        total_ki,
        ki_size
    );
    println!(
        "样本数据（去重后）≈ {:.1} MB",
        sample_bytes as f64 / 1048576.0
    );
    println!(
        "合计 ≈ {:.1} MB",
        (total_map_bytes + sample_bytes) as f64 / 1048576.0
    );
}
