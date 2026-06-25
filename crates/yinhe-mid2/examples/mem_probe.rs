//! 临时内存探针：加载真实 MIDI，观察各阶段 Midi tag 内存。
//! 跑完即删。

#[global_allocator]
static ALLOC: yinhe_memtrace::TaggedAlloc = yinhe_memtrace::TaggedAlloc;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/Users/jieneng/Music/MIDIs/start.mid".to_string());
    eprintln!("[mem-probe] loading: {path}");
    let t0 = std::time::Instant::now();
    match yinhe_mid2::parse_path(&path) {
        Ok(model) => {
            eprintln!(
                "[mem-probe] done in {:?}  note_count={}  tracks={}",
                t0.elapsed(),
                model.note_count,
                model.tracks.len(),
            );

            // ── 派生结构内存分解 ──
            const MB: f64 = 1_048_576.0;
            let note_sz = std::mem::size_of::<yinhe_core::NoteEvent>();
            let cachenote_sz = std::mem::size_of::<yinhe_types::Note>();

            let src_notes: usize = model.tracks.iter().map(|t| t.notes.len()).sum();
            let cache_notes: usize = model.key_notes_cache.iter().map(|v| v.len()).sum();
            let cache_cap: usize = model.key_notes_cache.iter().map(|v| v.capacity()).sum();

            eprintln!(
                "[mem-probe]   源 TrackData.notes : {} 条 × {}B = {:.1} MB",
                src_notes,
                note_sz,
                (src_notes * note_sz) as f64 / MB,
            );
            eprintln!(
                "[mem-probe]   key_notes_cache    : {} 条 × {}B = {:.1} MB (cap {:.1} MB)",
                cache_notes,
                cachenote_sz,
                (cache_notes * cachenote_sz) as f64 / MB,
                (cache_cap * cachenote_sz) as f64 / MB,
            );

            if let Some(si) = &model.scan_index {
                let blocks: usize = si.key_blocks.iter().map(|v| v.capacity()).sum();
                let bsz = std::mem::size_of::<yinhe_types::ScanBlock>();
                eprintln!(
                    "[mem-probe]   scan_index         : {} blocks × {}B = {:.1} MB",
                    blocks, bsz, (blocks * bsz) as f64 / MB,
                );
            }
            if let Some(tb) = &model.tick_buckets {
                let blocks: usize = tb.key_blocks.iter().map(|v| v.capacity()).sum();
                let bsz = std::mem::size_of::<yinhe_types::Bucket>();
                eprintln!(
                    "[mem-probe]   tick_buckets       : {} buckets × {}B = {:.1} MB",
                    blocks, bsz, (blocks * bsz) as f64 / MB,
                );
            }
        }
        Err(e) => eprintln!("[mem-probe] parse error: {e}"),
    }
    let snap = yinhe_memtrace::Snapshot::capture();
    eprintln!(
        "[mem-probe] 解析返回后常驻 midi={:.1} MB total={:.1} MB",
        snap.mb(yinhe_memtrace::AllocTag::Midi),
        snap.total_mb(),
    );
}
