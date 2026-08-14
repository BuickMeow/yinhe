//! 实验：start.mid → yin v5 的压缩率与耗时拆解（临时验证用）。
//!
//! 用法: cargo run --release -p yinhe-yin --example exp_save -- <mid路径> <输出目录>
//!
//! 输出：
//! - parse / save 耗时
//! - data 段 6 个流各自"原始(varint) → 压缩"大小
//! - 各流原始字节导出为 <out>/l<level>_<name>.bin，供外部 zstd CLI 做
//!   不同级别/窗口参数的对比压缩测试。

use std::time::Instant;

fn main() {
    let mid = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/Users/jieneng/Music/MIDIs/start.mid".to_string());
    let out_dir = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/yin_exp".to_string());
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let mid_bytes = std::fs::read(&mid).expect("read mid");
    println!(
        "mid 原始: {:.2} GiB ({} bytes)",
        mid_bytes.len() as f64 / (1u64 << 30) as f64,
        mid_bytes.len()
    );

    let t = Instant::now();
    let mut model = yinhe_midi::parse_bytes(&mid_bytes).expect("parse mid");
    println!("parse 耗时: {:?}", t.elapsed());
    let n: usize = model.notes.iter().map(|b| b.len()).sum();
    println!("音符数: {n}, tracks: {}", model.tracks.len());
    println!(
        "模型内存占用参考: NoteEvent 16B × {n} ≈ {:.2} GiB",
        n as f64 * 16.0 / (1u64 << 30) as f64
    );

    for lvl in [3, 19] {
        model.meta.compression_level = lvl;
        let t = Instant::now();
        let bytes = yinhe_yin::save_yin_bytes(&model).expect("save yin");
        let dt = t.elapsed();
        println!(
            "save level {lvl}: {:.2} MiB, 耗时 {:?} (吞吐 {:.0} MiB/s)",
            bytes.len() as f64 / (1u64 << 20) as f64,
            dt,
            bytes.len() as f64 / dt.as_secs_f64() / (1u64 << 20) as f64
        );
        let path = format!("{out_dir}/yin_l{lvl}.yin");
        std::fs::write(&path, &bytes).expect("write yin");
        analyze(&bytes, lvl, &out_dir);
    }
}

/// 解析 .yin 容器，拆出 data 段 6 个流，量大小并导出原始字节。
fn analyze(bytes: &[u8], lvl: i32, out_dir: &str) {
    let mut pos = 4 + 2; // magic + version
    let plen = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4 + plen;
    let mlen = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4 + mlen;
    let dlen = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;
    let data = &bytes[pos..pos + dlen];

    let names = ["meta", "delta", "key", "track", "vel", "gate"];
    let mut total_plain = 0usize;
    let mut total_comp = 0usize;
    let mut off = 0usize;
    for name in names.iter() {
        let len = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        off += 4;
        let comp = &data[off..off + len];
        off += len;
        let plain = zstd::decode_all(std::io::Cursor::new(comp)).expect("zstd decode");
        std::fs::write(format!("{out_dir}/l{lvl}_{name}.bin"), &plain).expect("write stream");
        println!(
            "  [{name}] 原始 {:.2} MiB → zstd{lvl} {:.2} MiB (ratio {:.1}x)",
            plain.len() as f64 / (1u64 << 20) as f64,
            comp.len() as f64 / (1u64 << 20) as f64,
            plain.len() as f64 / comp.len() as f64
        );
        total_plain += plain.len();
        total_comp += comp.len();
    }
    println!(
        "  data 段 6 流合计: 原始 {:.2} MiB → {:.2} MiB ({:.1}x)",
        total_plain as f64 / (1u64 << 20) as f64,
        total_comp as f64 / (1u64 << 20) as f64,
        total_plain as f64 / total_comp as f64
    );
}
