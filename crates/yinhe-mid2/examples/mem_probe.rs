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
    let model = yinhe_mid2::parse_path(&path);
    match &model {
        Ok(m) => {
            eprintln!(
                "[mem-probe] done in {:?}  note_count={}  tracks={}",
                t0.elapsed(),
                m.note_count,
                m.tracks.len(),
            );
        }
        Err(e) => eprintln!("[mem-probe] parse error: {e}"),
    }
    let snap = yinhe_memtrace::Snapshot::capture();
    eprintln!(
        "[mem-probe] 解析返回后常驻 midi={:.1} MB total={:.1} MB",
        snap.mb(yinhe_memtrace::AllocTag::Midi),
        snap.total_mb(),
    );
    drop(model);
}
