// 诊断：解析 .mid → 导出 .mid，比较不同 NoteOff 编码的文件体积。
use yinhe_midi::MidiExportOptions;

fn main() {
    for path in std::env::args().skip(1) {
        let t0 = std::time::Instant::now();
        let m = match yinhe_midi::parse_path(&path) {
            Ok(m) => m,
            Err(e) => {
                println!("{path} -> ERR {e}");
                continue;
            }
        };
        let notes: usize = m.tracks.iter().map(|t| t.notes.len()).sum();
        let out = std::env::temp_dir().join("yinhe_probe_out.mid");
        let opts = MidiExportOptions::default();
        match yinhe_midi::write_to_file(&m, &out, &opts) {
            Ok(()) => {
                let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                println!(
                    "{:54} notes={:>9} 导出={:7.2}MB 用时={:?}",
                    path.rsplit('/').next().unwrap_or(&path),
                    notes,
                    size as f64 / 1e6,
                    t0.elapsed()
                );
            }
            Err(e) => println!("{path} -> write ERR {e}"),
        }
    }
}
