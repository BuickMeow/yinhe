//! 内存探针：加载真实 MIDI，追踪各阶段峰值内存。
//!
//! 用法: cargo run --release --example mem_probe -p yinhe-mid2 [path]

#[global_allocator]
static ALLOC: yinhe_memtrace::TaggedAlloc = yinhe_memtrace::TaggedAlloc;

fn snap(label: &str) {
    let s = yinhe_memtrace::Snapshot::capture();
    eprintln!(
        "[mem-probe] {:<30} midi={:>8.1} MB  unknown={:>8.1} MB  total={:>8.1} MB",
        label,
        s.mb(yinhe_memtrace::AllocTag::Midi),
        s.mb(yinhe_memtrace::AllocTag::Unknown),
        s.total_mb(),
    );
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/Users/jieneng/Music/MIDIs/start.mid".to_string());
    eprintln!("[mem-probe] loading: {path}");
    snap("before anything");

    // Phase 1: parse（并行采样峰值）
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sampler = {
        let stop = std::sync::Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut peak_midi = 0isize;
            let mut peak_total = 0isize;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let s = yinhe_memtrace::Snapshot::capture();
                peak_midi = peak_midi.max(s.get(yinhe_memtrace::AllocTag::Midi));
                peak_total = peak_total.max(s.total_tracked());
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            (peak_midi, peak_total)
        })
    };
    let t0 = std::time::Instant::now();
    let model = yinhe_midi::parse_path(&path);
    let parse_dur = t0.elapsed();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (peak_midi, peak_total) = sampler.join().unwrap();
    eprintln!(
        "[mem-probe] parse peak midi={:.1} MB  total={:.1} MB",
        peak_midi as f64 / 1_048_576.0,
        peak_total as f64 / 1_048_576.0
    );
    match &model {
        Ok(m) => {
            eprintln!(
                "[mem-probe] parse done in {:?}  notes={}  tracks={}",
                parse_dur,
                m.note_count,
                m.tracks.len(),
            );
        }
        Err(e) => {
            eprintln!("[mem-probe] parse error: {e}");
            return;
        }
    }
    snap("after parse_path");

    // Phase 2: rebuild (sort + scan_index + tick_buckets)
    let mut model = model.unwrap();
    let t1 = std::time::Instant::now();
    model.rebuild();
    let rebuild_dur = t1.elapsed();
    eprintln!("[mem-probe] rebuild done in {:?}", rebuild_dur);
    snap("after rebuild");

    // Hold the model alive, then drop
    eprintln!(
        "[mem-probe] final: notes={}  tick_length={}  tempo_segments={}",
        model.note_count,
        model.tick_length,
        model.tempo_map.tempo_segments.len(),
    );
    snap("holding model");

    drop(model);
    snap("after drop model");
}
