//! 压缩 MIDI 导出耗时探针。
//!
//! `cargo run --release -p yinhe-editor-core --example archive_bench [notes]`

use std::sync::Arc;
use yinhe_core::{NoteEvent, TrackData, YinModel};
use yinhe_editor_core::midi_7z::{MidiArchiveFormat, export_midi_archive};
use yinhe_midi::MidiExportOptions;

fn main() {
    let n: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let mut track = TrackData::new(0, 0);
    track.name = "Bench".into();
    let mut model = YinModel {
        tracks: vec![Arc::new(track)],
        ..Default::default()
    };
    let mut seed = 0x1234_5678u32;
    let notes: Vec<NoteEvent> = (0..n)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let start = i.saturating_mul(10);
            NoteEvent {
                id: 0,
                start_tick: start,
                end_tick: start.saturating_add(5 + (seed >> 24)),
                key: 21 + ((seed >> 16) % 88) as u8,
                velocity: 20 + ((seed >> 8) % 108) as u8,
            }
        })
        .collect();
    model.load_track_notes(vec![notes]);
    model.rebuild();
    let path = std::env::temp_dir().join("yinhe-archive-bench.7z");
    let t = std::time::Instant::now();
    let mut last = 0.0;
    export_midi_archive(
        &model,
        &path,
        MidiArchiveFormat::SevenZ,
        &MidiExportOptions::default(),
        &std::sync::atomic::AtomicBool::new(false),
        &std::sync::atomic::AtomicBool::new(false),
        |p| last = p,
    )
    .expect("export");
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "[archive-bench] notes={} time={:?} size={:.1} MB last={:.2}",
        n,
        t.elapsed(),
        len as f64 / 1_048_576.0,
        last
    );
    let _ = std::fs::remove_file(&path);
}
