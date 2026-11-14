//! 导出峰值内存探针。
//!
//! `cargo run --release -p yinhe-midi --features memtrace --example export_mem_probe [notes]`
//!
//! 构造单轨 N 音符模型，采样导出期间 MIDI tag 的峰值分配。

#[global_allocator]
static ALLOC: yinhe_memtrace::TaggedAlloc = yinhe_memtrace::TaggedAlloc;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use yinhe_core::{NoteEvent, TrackData, YinModel};
use yinhe_memtrace::{AllocTag, Snapshot};

fn main() {
    let arg = std::env::args().nth(1);
    let model = if let Some(path) = arg
        .as_deref()
        .filter(|p| p.ends_with(".mid") || p.ends_with(".midi"))
    {
        let t = std::time::Instant::now();
        let model = yinhe_midi::parse_path(path).expect("parse midi");
        eprintln!("[export-probe] parsed in {:?}", t.elapsed());
        model
    } else {
        let n: u32 = arg.and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
        let mut track = TrackData::new(0, 0);
        track.name = "Probe".into();
        let mut model = YinModel {
            tracks: vec![Arc::new(track)],
            ..Default::default()
        };
        let notes: Vec<NoteEvent> = (0..n)
            .map(|i| NoteEvent {
                id: 0,
                start_tick: i.saturating_mul(10),
                end_tick: i.saturating_mul(10).saturating_add(5),
                key: 60 + (i % 12) as u8,
                velocity: 100,
            })
            .collect();
        model.load_track_notes(vec![notes]);
        model.rebuild();
        model
    };
    let base = Snapshot::capture();
    eprintln!(
        "[export-probe] notes={}  baseline total={:.1} MB  midi={:.1} MB",
        model.note_count,
        base.total_mb(),
        base.mb(AllocTag::Midi)
    );

    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut peak_midi = 0isize;
            let mut peak_total = 0isize;
            while !stop.load(Ordering::Relaxed) {
                let s = Snapshot::capture();
                peak_midi = peak_midi.max(s.get(AllocTag::Midi));
                peak_total = peak_total.max(s.total_tracked());
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            (peak_midi, peak_total)
        })
    };

    let path = std::env::temp_dir().join("yinhe-export-mem-probe.mid");
    let t = std::time::Instant::now();
    let mut callbacks = 0u32;
    yinhe_memtrace::with_tag(AllocTag::Midi, || {
        yinhe_midi::write_to_file_with_progress(
            &model,
            &path,
            &yinhe_midi::MidiExportOptions::default(),
            |_| callbacks += 1,
        )
        .expect("export");
    });
    let dur = t.elapsed();
    stop.store(true, Ordering::Relaxed);
    let (peak_midi, peak_total) = sampler.join().unwrap();
    let file_len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "[export-probe] done in {:?}  file={:.1} MB  callbacks={}  peak midi={:.1} MB  peak total={:.1} MB",
        dur,
        file_len as f64 / 1_048_576.0,
        callbacks,
        peak_midi as f64 / 1_048_576.0,
        peak_total as f64 / 1_048_576.0,
    );
    let _ = std::fs::remove_file(&path);
}
