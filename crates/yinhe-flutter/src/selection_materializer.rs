//! 选区成员位图的异步物化。
//!
//! 框选/缩放控制点松手时先把矩形选框交给 Flutter 绘制；成员位图（按 note id）
//! 在后台线程扫描，算完后回到场景。这样大选区不会卡住 UI，同时后续移动/复制
//! 仍使用显式 id 集合，不会把移动后才进入矩形的音符误带进来。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::{Selection, YinModel};

use crate::scene::RenderScene;

struct Pending {
    generation: u64,
    rects: Vec<(u32, u32, u8, u8, u16, u16)>,
    filter: yinhe_core::SelectionFilter,
    model: Arc<YinModel>,
    selection: Selection,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);
static RESULT: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();

fn result_slot() -> &'static Mutex<Option<Pending>> {
    RESULT.get_or_init(|| Mutex::new(None))
}

/// 安排异步物化当前矩形选区。重复调用会让旧任务的结果作废。
pub fn schedule(s: &RenderScene) {
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    if doc.edit.selected.is_empty() || doc.edit.selected.has_explicit_members() {
        return;
    }
    let selection = doc.edit.selected.clone();
    let rects = selection.rects.clone();
    let filter = selection.filter.clone();
    let model = doc.data.model.clone();
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    std::thread::spawn(move || {
        let mut materialized = selection;
        materialized.materialize_pending(&*model);
        if GENERATION.load(Ordering::Relaxed) != generation {
            return;
        }
        if let Ok(mut slot) = result_slot().lock() {
            *slot = Some(Pending {
                generation,
                rects,
                filter,
                model,
                selection: materialized,
            });
        }
    });
}

/// 在渲染/编辑入口应用已完成的结果；选区或模型已经变化时丢弃。
pub fn apply_pending(s: &mut RenderScene) {
    let Some(pending) = result_slot().lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    if pending.generation != GENERATION.load(Ordering::Relaxed) {
        return;
    }
    let Some(doc) = s.document.as_mut() else {
        return;
    };
    if doc.edit.selected.rects != pending.rects
        || doc.edit.selected.filter != pending.filter
        || !Arc::ptr_eq(&doc.data.model, &pending.model)
    {
        return;
    }
    doc.edit.selected = pending.selection;
}
