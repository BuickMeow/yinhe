//! summarize_selected 全选耗时的量化验证（Info 面板/sel_hint 每帧调用）。
//!
//! 用法: cargo run --release -p yinhe-tests --example sum_bench -- <yin路径>

use std::time::Instant;

use yinhe_editor_core::batch_ops::summarize_selected;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/start_v9.yin".to_string());

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

    // 全选（矩形态，零物化）——对应 Cmd+A。
    doc.edit
        .selected
        .add_rect_track(0, u32::MAX, 0, 255, 0, u16::MAX);

    let note_count = doc.data.model.note_count;
    for i in 0..3 {
        let t = Instant::now();
        let s = summarize_selected(&doc.data.model, &doc.edit.selected);
        println!(
            "full run{i}: count={} (model={note_count}), vel={:?}, gate={:?}, key={:?}, tick={:?}, {:.3}s",
            s.count,
            s.velocity,
            s.gate,
            s.key,
            s.tick,
            t.elapsed().as_secs_f64()
        );
    }

    // 非全选（半程 tick 范围）：count 必须扫全，uniform 无法提前退出。
    let mut doc2 = Document::from_model(
        &path,
        yinhe_yin::load_yin(&path).expect("load yin"),
        QuantizePreset::Fraction(1, 4),
        QuantizePreset::Fraction(1, 16),
        Default::default(),
        Default::default(),
        None,
    )
    .expect("build document");
    let half = doc2.data.model.tick_length as u32 / 2;
    doc2.edit
        .selected
        .add_rect_track(0, half, 0, 255, 0, u16::MAX);
    for i in 0..2 {
        let t = Instant::now();
        let s = summarize_selected(&doc2.data.model, &doc2.edit.selected);
        println!(
            "half run{i}: count={}, vel={:?}, gate={:?}, key={:?}, tick={:?}, {:.3}s",
            s.count,
            s.velocity,
            s.gate,
            s.key,
            s.tick,
            t.elapsed().as_secs_f64()
        );
    }
}
