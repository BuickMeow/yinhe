//! 画像（本地 MIDI，ignored）：`audible_notes`（发声音符副本）的时间与内存成本。
//!
//! 用来判断"按力度过滤"是否值得在 dispatch 时现算，而不是预构建一份 12B/音
//! 的副本。MIDI 列表可用 `YINHE_PROF_MIDIS`（逗号分隔）覆盖。

use std::mem::size_of;
use std::time::Instant;

use yinhe_types::Note;

use crate::audio_model::AudibleNote;

fn midis() -> Vec<String> {
    std::env::var("YINHE_PROF_MIDIS")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_else(|_| {
            vec![
                "/Users/jieneng/Music/MIDIs/Echoes of Memoria v1.0.5Fixed.mid".into(),
            ]
        })
}

#[test]
#[ignore = "需要本地 MIDI"]
fn prof_audible_notes_cost() {
    let mb = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    for midi in midis() {
        let t_parse = Instant::now();
        let model = yinhe_midi::parse_path(&midi).expect("parse");
        let total = model.note_count as usize;
        eprintln!(
            "\n=== {}\n  parse={:.1}s 模型音符={total} Note={}B → {:.0}MB",
            midi.rsplit('/').next().unwrap_or(&midi),
            t_parse.elapsed().as_secs_f64(),
            size_of::<Note>(),
            mb(total * size_of::<Note>()),
        );
        // 直接过滤方案的上界成本：一次顺序扫描全部音符（光标单调前进，
        // 整曲只扫一遍），与 build 的扫描量相同。
        {
            let t = Instant::now();
            let mut touched = 0usize;
            let mut sum = 0u64;
            for key in 0..yinhe_types::KEY_COUNT {
                for n in model.notes[key].iter() {
                    touched += 1;
                    if n.velocity > 1 {
                        sum += n.velocity as u64;
                    }
                }
            }
            eprintln!(
                "  顺序扫描全部音符: touched={touched} sum={sum} 用时={:?}",
                t.elapsed()
            );
        }
        for threshold in [0u8, 1, 10, 40] {
            let t = Instant::now();
            let buckets = crate::prepare_model::build_audible_notes(&model, threshold);
            let dt = t.elapsed();
            let n: usize = buckets.iter().map(|b| b.len()).sum();
            let bytes: usize = buckets
                .iter()
                .map(|b| b.capacity() * size_of::<AudibleNote>())
                .sum();
            let empty = buckets.iter().filter(|b| b.is_empty()).count();
            eprintln!(
                "  thr={threshold:>2}: audible={n:>10} ({:>5.1}%) build={dt:>7.2?} 副本={:>7.0}MB AudibleNote={}B 空桶={empty}",
                100.0 * n as f64 / total.max(1) as f64,
                mb(bytes),
                size_of::<AudibleNote>(),
            );
            drop(buckets);
        }
    }
}
