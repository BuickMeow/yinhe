//! 全选移动的内存/耗时验证：加载 .yin → 全选 → 移动 → 采样峰值 RSS。
//!
//! 验证 apply_note_shift 的原地路径（同 key 平移不再 collect/remove/insert
//! 全量物化）。用法:
//!   cargo run --release -p yinhe-tests --example move_mem -- <yin路径> [delta_ticks]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/tmp/start_v9.yin".to_string());
    let delta: i64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    let t = Instant::now();
    let model = yinhe_yin::load_yin(&path).expect("load yin");
    println!(
        "loaded: {} notes, {} tracks, {:.1}s",
        model.note_count,
        model.tracks.len(),
        t.elapsed().as_secs_f64()
    );

    let mut doc = Document::from_model(
        &path,
        model,
        QuantizePreset::Fraction(1, 4),
        QuantizePreset::Fraction(1, 16),
        Default::default(),
        Default::default(),
        None,
    )
    .expect("build document");

    // 全选（矩形态，零物化）。
    doc.edit
        .selected
        .add_rect_track(0, u32::MAX, 0, 255, 0, u16::MAX);

    // 后台采样峰值 RSS。
    let peak = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let pid = sysinfo::get_current_pid().expect("pid");
    let sampler = std::thread::spawn({
        let peak = Arc::clone(&peak);
        let stop = Arc::clone(&stop);
        move || {
            let mut sys = sysinfo::System::new();
            while !stop.load(Ordering::Relaxed) {
                sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), false);
                if let Some(p) = sys.process(pid) {
                    peak.fetch_max(p.memory(), Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    });

    let t = Instant::now();
    let action = doc.move_selected_notes(delta, 0);
    let dt = t.elapsed();

    stop.store(true, Ordering::Relaxed);
    sampler.join().expect("sampler");

    let peak_gb = peak.load(Ordering::Relaxed) as f64 / (1u64 << 30) as f64;
    match &action {
        Some(yinhe_editor_core::history::UndoAction::MoveNotes { .. }) => {
            println!("move action: MoveNotes（操作式，O(1)）");
        }
        Some(_) => println!("move action: 其他（副本制）"),
        None => println!("move action: None"),
    }
    println!("move 耗时: {:.1}s", dt.as_secs_f64());
    println!("峰值 RSS: {:.2} GB", peak_gb);
}
