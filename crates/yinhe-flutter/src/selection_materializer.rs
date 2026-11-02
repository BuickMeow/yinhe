//! 选区成员位图 + 拖动 ghost 数据的异步物化。
//!
//! 框选/缩放控制点松手时先把矩形选框交给 Flutter 绘制；成员位图（按 note id）
//! 在后台线程扫描，算完后回到场景。这样大选区不会卡住 UI，同时后续移动/复制
//! 仍使用显式 id 集合，不会把移动后才进入矩形的音符误带进来。
//!
//! 拖动 ghost（按 key 分桶的音符 + 原位置隐藏集）**按需**在第一次按下选框时
//! 后台计算并缓存：选区松手就全量算会在用户只是框选时白占几百 MB。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::{Selection, YinModel};
use yinhe_types::NoteSource;

use crate::edit::DragByKey;
use crate::scene::RenderScene;

/// 选区整体拖动所需的数据：按 key 分桶的 `(track, start, end, velocity)`，
/// 以及原位置隐藏集合（拖动时把原音符藏起来）。
pub struct DragData {
    pub by_key: DragByKey,
    pub hidden: Arc<yinhe_types::HiddenNotes>,
}

struct Pending {
    generation: u64,
    rects: Vec<(u32, u32, u8, u8, u16, u16)>,
    filter: yinhe_core::SelectionFilter,
    selection: Selection,
}

struct CachedDrag {
    generation: u64,
    revision: u64,
    rects: Vec<(u32, u32, u8, u8, u16, u16)>,
    filter: yinhe_core::SelectionFilter,
    data: Arc<DragData>,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);
static RESULT: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();
static DRAG: OnceLock<Mutex<Option<CachedDrag>>> = OnceLock::new();

fn result_slot() -> &'static Mutex<Option<Pending>> {
    RESULT.get_or_init(|| Mutex::new(None))
}

fn drag_slot() -> &'static Mutex<Option<CachedDrag>> {
    DRAG.get_or_init(|| Mutex::new(None))
}

/// 历史操作（undo/redo）或选区被替换前作废尚未应用的结果，避免旧数据覆盖新状态。
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut slot) = drag_slot().lock() {
        *slot = None;
    }
}

/// 安排异步物化当前矩形选区的成员位图。重复调用会让旧任务的结果作废。
pub fn schedule(s: &RenderScene) {
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    if doc.edit.selected.is_empty() || doc.edit.selected.has_explicit_members() {
        return;
    }
    // 选区已变：上一份 ghost 数据不再适用，顺手释放（可能有几百 MB）。
    if let Ok(mut slot) = drag_slot().lock() {
        *slot = None;
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
                selection: materialized,
            });
        }
    });
}

/// 按需后台计算拖动 ghost 数据（按下已有选框时调用）。
///
/// 结果按 `(generation, revision, 选区矩形/筛选)` 缓存；同一选区反复拖动直接
/// 复用（Arc），只有文档内容或选区变化才重算。结果未就绪时拖动预览会晚几帧
/// 出现，但不会阻塞 UI。
pub fn schedule_drag(s: &RenderScene) {
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    if doc.edit.selected.is_empty() {
        return;
    }
    let selection = doc.edit.selected.clone();
    let rects = selection.rects.clone();
    let filter = selection.filter.clone();
    let model = doc.data.model.clone();
    let revision = doc.data.revision;
    let generation = GENERATION.load(Ordering::Relaxed);
    std::thread::spawn(move || {
        // 选区还没物化成成员位图时，先在后台补物化再收集；全部在子线程完成。
        let selection = if selection.has_explicit_members() {
            selection
        } else {
            let mut materialized = selection;
            materialized.materialize_pending(&*model);
            materialized
        };
        if GENERATION.load(Ordering::Relaxed) != generation {
            return;
        }
        let (by_key, hidden) = collect(&model, &selection);
        if GENERATION.load(Ordering::Relaxed) != generation {
            return;
        }
        if let Ok(mut slot) = drag_slot().lock() {
            *slot = Some(CachedDrag {
                generation,
                revision,
                rects,
                filter,
                data: Arc::new(DragData {
                    by_key,
                    hidden: Arc::new(hidden),
                }),
            });
        }
    });
}

/// 取当前选区对应的拖动 ghost 数据；缓存缺失/过期时返回 `None`（调用方应
/// `schedule_drag` 等待后台，而不是在 UI 线程同步重扫）。
pub fn take_drag(s: &RenderScene) -> Option<Arc<DragData>> {
    let doc = s.document.as_ref()?;
    let slot = drag_slot().lock().ok()?;
    let cached = slot.as_ref()?;
    if cached.generation != GENERATION.load(Ordering::Relaxed)
        || cached.revision != doc.data.revision
        || doc.edit.selected.rects != cached.rects
        || doc.edit.selected.filter != cached.filter
    {
        return None;
    }
    Some(Arc::clone(&cached.data))
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
    // 只比较选区矩形和筛选条件。模型可能因无关编辑已经换代，但成员位图
    // 里记录的是选区建立时的 note id，正是桌面端“不把后来音符带进来”的语义。
    if doc.edit.selected.rects != pending.rects || doc.edit.selected.filter != pending.filter {
        return;
    }
    doc.edit.selected = pending.selection;
}

/// 扫描选区内的音符，产出拖动 ghost 数据。
///
/// 单矩形（绝大多数框选/全选）不会跨矩形重复产出音符：音符在桶内按 start
/// 排序，只需和同 key 上一个已接受的音符比较，避免百万级 HashSet。
fn collect(model: &YinModel, selection: &Selection) -> (DragByKey, yinhe_types::HiddenNotes) {
    use std::collections::HashSet;

    let mut by_key: DragByKey = (0..yinhe_types::KEY_COUNT).map(|_| Vec::new()).collect();
    let mut hidden = yinhe_types::HiddenNotesBuilder::new();
    let single_rect = selection.rects.len() == 1;
    let mut seen = HashSet::new();
    let mut last_start: Vec<Option<(u16, u32)>> = vec![None; yinhe_types::KEY_COUNT];
    for &(t0, t1, klo, khi, _, _) in &selection.rects {
        let (ts, te) = (t0.min(t1), t0.max(t1));
        if te <= ts {
            continue;
        }
        for key in klo..=khi {
            for n in model.key_notes_in_range(key, ts, te) {
                if n.start_tick < ts || n.start_tick >= te {
                    continue;
                }
                if !selection.accepts_note(n, key) {
                    continue;
                }
                if single_rect {
                    let slot = &mut last_start[key as usize];
                    if *slot == Some((n.track, n.start_tick)) {
                        continue;
                    }
                    *slot = Some((n.track, n.start_tick));
                } else if !seen.insert((n.track, n.start_tick, key)) {
                    continue;
                }
                by_key[key as usize].push((n.track, n.start_tick, n.end_tick, n.velocity));
                hidden.insert(n.id, key);
            }
        }
    }
    (by_key, hidden.build())
}
