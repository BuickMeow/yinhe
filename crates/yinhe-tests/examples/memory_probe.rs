//! 直接测量文档里“待测”项的内存探针（不影响生产代码）。
//!
//! 用法：
//! `cargo run --release -p yinhe-tests --example memory_probe -- summary <mid|yin>`
//! `cargo run --release -p yinhe-tests --example memory_probe -- batch`
//! `cargo run --release -p yinhe-tests --example memory_probe -- cache`
//!
//! - summary：真实/合成工程的 GPU 原始层 + 8 档 LOD 摘要段数与显存估算。
//! - batch：`insert_batch_sorted` 的瞬时峰值（1M 桶插 10 万）。
//! - cache：`NoteLayerCache` 垂直滚完 256 个 key 后窗口外桶的常驻内存。
//!
//! 这个例子用计数分配器统计 Rust 堆峰值，不包含 wgpu 的真实 GPU buffer；
//! GPU 字节数按 `NoteInstance`/索引/args 的结构大小单独计算。

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use yinhe_core::{NoteEvent, Selection, TrackData, YinModel};
use yinhe_types::{KEY_COUNT, KeyLayout, Note, PianoRollView, TimelineViewBase};
use yinhe_wgpu::{
    NoteCacheParams, NoteLayerCache, SUMMARY_BLOCK_TICKS, build_all_notes, build_summary,
};

// ---------------------------------------------------------------------------
// 计数分配器
// ---------------------------------------------------------------------------

struct CountingAlloc;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

impl CountingAlloc {
    fn add(size: usize) {
        let cur = CURRENT.fetch_add(size, Ordering::Relaxed) + size;
        PEAK.fetch_max(cur, Ordering::Relaxed);
    }

    fn sub(size: usize) {
        CURRENT.fetch_sub(size, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            Self::add(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            Self::add(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        Self::sub(layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                Self::add(new_size - layout.size());
            } else {
                Self::sub(layout.size() - new_size);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1_048_576.0
}

fn current() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

fn reset_peak() {
    PEAK.store(current(), Ordering::Relaxed);
}

fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// 合成模型
// ---------------------------------------------------------------------------

/// `count` 个音符，均匀分到 `keys` 个 key，start 落在 `[0, tick_span)`。
fn make_model(count: usize, keys: usize, tick_span: u32) -> YinModel {
    let per_key = count.div_ceil(keys);
    let mut by_key: Vec<Vec<NoteEvent>> = (0..keys).map(|_| Vec::with_capacity(per_key)).collect();
    for i in 0..count {
        let key = (i % keys) as u8;
        let start = (i as u64 * u64::from(tick_span) / count as u64) as u32;
        by_key[key as usize].push(NoteEvent {
            id: 0,
            start_tick: start,
            end_tick: start.saturating_add(10),
            key,
            velocity: 100,
        });
    }
    let per_track = vec![by_key.into_iter().flatten().collect()];
    let mut model = YinModel {
        tracks: vec![Arc::new(TrackData::new(0, 0))],
        ..Default::default()
    };
    model.load_track_notes(per_track);
    model.rebuild();
    model
}

// ---------------------------------------------------------------------------
// summary：GPU 原始层 + 8 档摘要
// ---------------------------------------------------------------------------

fn load_model(path: &str) -> YinModel {
    if path.ends_with(".yin") {
        yinhe_yin::load_yin(path).expect("加载 .yin 失败")
    } else {
        yinhe_midi::parse_path(path).expect("解析 MIDI 失败")
    }
}

fn gpu_bytes(offsets: &[u32; KEY_COUNT + 1]) -> (usize, usize, usize) {
    let mut all = 0usize;
    let mut visible = 0usize;
    let mut args = 0usize;
    for key in 0..KEY_COUNT {
        let n = (offsets[key + 1] - offsets[key]) as usize;
        if n == 0 {
            continue;
        }
        all += n * std::mem::size_of::<yinhe_wgpu::NoteInstance>();
        let chunks = n.div_ceil(256);
        visible += chunks * 256 * std::mem::size_of::<u32>();
        args += chunks * 40;
    }
    (all, visible, args)
}

fn summary(path: &str) {
    println!("[summary] loading: {path}");
    let model = load_model(path);
    let tracks = model.tracks.len();
    println!(
        "[summary] notes={} tracks={} tick_length={}",
        model.note_count, tracks, model.tick_length
    );

    let visible = vec![true; tracks];
    let hidden: HashSet<(u16, u32, u8)> = HashSet::new();
    let t0 = Instant::now();
    let (notes, offsets) = build_all_notes(&model, &hidden, &visible);
    let (raw_all, raw_vis, raw_args) = gpu_bytes(&offsets);
    println!(
        "[summary] raw: instances={} all={:.1}MB visible={:.1}MB args={:.1}MB build={:?}",
        notes.len(),
        mb(raw_all),
        mb(raw_vis),
        mb(raw_args),
        t0.elapsed()
    );

    let mut total_all = 0usize;
    let mut total_vis = 0usize;
    let mut total_args = 0usize;
    for (level, &block) in SUMMARY_BLOCK_TICKS.iter().enumerate() {
        let t = Instant::now();
        let (summary, summary_offsets) = build_summary(&notes, &offsets, block);
        let (all, vis, args) = gpu_bytes(&summary_offsets);
        total_all += all;
        total_vis += vis;
        total_args += args;
        println!(
            "[summary] level {level} block={block:>6} segments={:>10} all={:>7.1}MB visible={:>6.1}MB args={:>5.1}MB build={:?}",
            summary.len(),
            mb(all),
            mb(vis),
            mb(args),
            t.elapsed()
        );
    }
    println!(
        "[summary] 8 levels total: all={:.1}MB visible={:.1}MB args={:.1}MB",
        mb(total_all),
        mb(total_vis),
        mb(total_args)
    );
}

// ---------------------------------------------------------------------------
// batch：insert_batch_sorted 峰值
// ---------------------------------------------------------------------------

fn batch(base: usize) {
    const ADD: usize = 100_000;
    println!("[batch] building {base} notes in one key...");
    let mut model = make_model(base, 1, 10_000_000);
    let bucket_len = model.notes[0].len();
    let mut new_notes = Vec::with_capacity(ADD);
    for i in 0..ADD {
        let start = ((i as u64 * 10_000_000) / ADD as u64) as u32;
        new_notes.push(Note {
            id: (base + i) as u32,
            start_tick: start,
            end_tick: start.saturating_add(10),
            velocity: 100,
            track: 0,
        });
    }

    reset_peak();
    let base_bytes = current();
    let t = Instant::now();
    Arc::make_mut(&mut model.notes[0]).insert_batch_sorted(new_notes);
    let elapsed = t.elapsed();
    let peak_extra = peak().saturating_sub(base_bytes);
    println!(
        "[batch] bucket {bucket_len} + {ADD}: base={:.1}MB peak_extra={:.1}MB peak={:.1}MB time={:?}",
        mb(base_bytes),
        mb(peak_extra),
        mb(peak()),
        elapsed
    );
}

// ---------------------------------------------------------------------------
// cache：NoteLayerCache 窗口外桶
// ---------------------------------------------------------------------------

fn cache_view(scroll_y: f32) -> PianoRollView {
    PianoRollView {
        base: TimelineViewBase {
            pixels_per_tick: 0.3, // >0.25：不触发 16 tick 摘要，走原始层
            scroll_x: 0.0,
            scroll_y,
            left_panel_width: 0.0,
            dirty: true,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
            follow_target: None,
            follow_anim_start: 0.0,
            follow_anim_elapsed: 0.0,
        },
        key_height: 10.0,
        layout: KeyLayout::for_key_count(256),
        ..Default::default()
    }
}

fn cache() {
    const NOTES: usize = 5_000_000;
    println!("[cache] building {NOTES} notes over 256 keys, all within 3000 ticks...");
    let model = make_model(NOTES, 256, 3000);
    let mut cache = NoteLayerCache::new(1);
    let selection = Selection::default();
    let tracks = HashSet::new();
    let hidden: HashSet<(u16, u32, u8)> = HashSet::new();
    let visible = vec![true; model.tracks.len()];
    let revisions = model.note_revisions;
    let mut view = cache_view(0.0);

    let base_bytes = current();
    let mut last_report = 0usize;
    for step in 0..=256 {
        view.base.scroll_y = step as f32 * 10.0;
        cache.update(&NoteCacheParams {
            midi: &model,
            view: &view,
            width: 1000.0,
            height: 100.0,
            track_visible: &visible,
            track_selected: &tracks,
            hidden_notes: &hidden,
            hidden_hash: 0,
            selection: Some(&selection),
            note_revisions: &revisions,
            lod_enabled: true,
            source_tag: 1,
        });
        if step % 32 == 0 {
            let now = current();
            println!(
                "[cache] step {step:>3} keys: current={:.1}MB (+{:.1}MB) flat={}",
                mb(now),
                mb(now.saturating_sub(base_bytes)),
                cache.instances().len()
            );
            last_report = now;
        }
    }
    let now = current();
    println!(
        "[cache] final: current={:.1}MB, retained over base={:.1}MB, final flat={}",
        mb(now),
        mb(now.saturating_sub(base_bytes)),
        cache.instances().len()
    );
    let _ = last_report;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("summary") => {
            let path = args.get(2).map(String::as_str).unwrap_or("start.yin");
            summary(path);
        }
        Some("batch") => {
            let base = args
                .get(2)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1_000_000);
            batch(base);
        }
        Some("cache") => cache(),
        _ => {
            eprintln!("用法: memory_probe <summary <mid|yin> | batch | cache>");
            std::process::exit(2);
        }
    }
}
