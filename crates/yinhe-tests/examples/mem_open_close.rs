//! 模拟 GUI 的"打开 MIDI → 关闭文档"流程，测量内存残留。
//!
//! 与 mode bar 的 MEM 指示器同源：memtrace 开启时 MEM 显示的就是
//! `yinhe_memtrace::Snapshot::capture().total_mb()`。本程序按 GUI 的
//! 打开/关闭路径（`parse_bytes_with_encoding` + `Document::from_model` /
//! drop Document + `purge_free_pages`）复刻一遍，对比基线、打开后、关闭后
//! 三个时间点的分类堆内存，残留 = 关闭后 − 基线。
//!
//! 用法:
//!   cargo run --release -p yinhe-tests --features memtrace --example mem_open_close -- <midi路径>
//!
//! 不带参数时用默认测试文件。

use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_memtrace::{AllocTag, Snapshot, TaggedAlloc};
use yinhe_midi::MidiImportEncoding;

#[global_allocator]
static GLOBAL_ALLOC: TaggedAlloc = TaggedAlloc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/Users/jieneng/Music/MIDIs/APT.mid".to_string());

    // sysinfo 自身的一次性初始化，放在基线之前，不干扰差值。
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let rss_mb = |sys: &mut sysinfo::System| -> f64 {
        sys.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::get_current_pid().unwrap()]),
            false,
        );
        sys.process(sysinfo::get_current_pid().unwrap())
            .map(|p| p.memory() as f64 / 1_048_576.0)
            .unwrap_or(0.0)
    };

    // ── 基线 ──
    let base = Snapshot::capture();
    let base_rss = rss_mb(&mut sys);

    // ── 打开（与 GUI 相同路径）──
    let data = std::fs::read(&path).expect("read midi");
    let model = yinhe_memtrace::with_tag(AllocTag::Midi, || {
        yinhe_midi::parse_bytes_with_encoding(&data, MidiImportEncoding::Utf8, |_| {})
            .expect("parse midi")
    });
    let doc = Document::from_model(
        &path,
        model,
        QuantizePreset::Fraction(1, 4),
        QuantizePreset::Fraction(1, 16),
        Default::default(),
        Default::default(),
    )
    .expect("build document");
    drop(data);

    let opened = Snapshot::capture();
    let opened_rss = rss_mb(&mut sys);
    println!(
        "opened: {} notes, {} tracks",
        doc.data.model.note_count,
        doc.data.model.tracks.len()
    );

    // ── 关闭（与 GUI 相同路径：drop Document + purge_free_pages）──
    drop(doc);
    yinhe_memtrace::purge_free_pages();

    let closed = Snapshot::capture();
    let closed_rss = rss_mb(&mut sys);

    // ── 报告 ──
    println!("\n=== memtrace 堆内存（MEM 指示器同源）===");
    println!(
        "{:<10} {:>10} {:>10} {:>10}",
        "分类", "基线MB", "打开后MB", "关闭后MB"
    );
    for tag in AllocTag::ALL {
        let b = base.mb(tag);
        let o = opened.mb(tag);
        let c = closed.mb(tag);
        if b.abs() > 0.01 || o.abs() > 0.01 || c.abs() > 0.01 {
            println!("{:<10} {:>10.2} {:>10.2} {:>10.2}", tag.name(), b, o, c);
        }
    }
    let total_open = opened.total_mb() - base.total_mb();
    let total_closed = closed.total_mb() - base.total_mb();
    println!(
        "{:<10} {:>10.2} {:>10.2} {:>10.2}",
        "合计",
        base.total_mb(),
        opened.total_mb(),
        closed.total_mb()
    );
    println!("\n打开占用:   {:.2} MB", total_open);
    println!("关闭后残留: {:.2} MB", total_closed);
    println!(
        "释放比例:   {:.1}%",
        if total_open > 0.0 {
            (1.0 - total_closed / total_open) * 100.0
        } else {
            0.0
        }
    );

    println!("\n=== 进程 RSS（系统视角，仅供参考）===");
    println!(
        "基线 {:.1} MB → 打开后 {:.1} MB → 关闭后 {:.1} MB",
        base_rss, opened_rss, closed_rss
    );
}
