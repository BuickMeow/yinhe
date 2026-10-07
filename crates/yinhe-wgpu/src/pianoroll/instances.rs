use rayon::prelude::*;
use yinhe_core::Selection;
use yinhe_theme::GpuTheme;
use yinhe_types::{KEY_COUNT, MAX_KEY, NoteSource};

use crate::vertex::NoteInstance;
use yinhe_types::PianoRollView;

/// Stack red zone threshold. When stack usage exceeds this, the `stacker` will
/// allocate a new stack segment before calling the closure.
/// 32KB should be enough for a single key's note iteration.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size to allocate when the red zone is exceeded.
const STACK_SIZE: usize = 1024 * 1024; // 1MB per segment

/// PR 音符过滤循环：逐音符原样输出（不合并、不裁剪），仅过滤 track_visible /
/// hidden_notes / （可选）视口 tick 范围。PR 和 GPU cull 共用的唯一实现。
///
/// `range` 为 `Some((ts, te))` 时只输出与视口 tick 范围相交的音符，`None`
/// 输出该 key 的全部音符。notes 必须按 start_tick 升序。
///
/// `selected` 非空时逐音符查询成员位图，命中的实例打上选中位
/// （shader 填充纯黑；只有桌面可见层的 B 路径传入，GPU cull 全曲层不传）。
/// 遍历某个 key 的音符（`range` 为半开区间，`None` = 全部）。
#[inline]
fn for_each_key_note(
    midi: &dyn NoteSource,
    key: u8,
    range: Option<(f64, f64)>,
    mut f: impl FnMut(&yinhe_types::Note),
) {
    match range {
        Some((ts, te)) => {
            for note in midi.key_notes_in_range(key, ts as u32, te as u32) {
                if (note.end_tick as f64) < ts {
                    continue;
                }
                f(note);
            }
        }
        None => {
            for note in midi.key_notes(key).iter() {
                f(note);
            }
        }
    }
}

/// 单音符可见性过滤：轨道可见且不在 hidden 集合中。
#[inline]
fn instance_visible(
    note: &yinhe_types::Note,
    key: u8,
    track_visible: &[bool],
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
) -> bool {
    track_visible
        .get(note.track as usize)
        .copied()
        .unwrap_or(true)
        && !hidden_notes.contains(&(note.track, note.start_tick, key))
}

fn build_key_instances(
    out: &mut Vec<NoteInstance>,
    midi: &dyn NoteSource,
    key: u8,
    track_visible: &[bool],
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    range: Option<(f64, f64)>,
    selected: Option<&Selection>,
) {
    let start = out.len();
    for_each_key_note(midi, key, range, |note| {
        if !instance_visible(note, key, track_visible, hidden_notes) {
            return;
        }
        let mut inst = NoteInstance {
            start_tick: note.start_tick,
            end_tick: note.end_tick,
            packed: NoteInstance::pack(key, note.track, note.velocity),
        };
        if let Some(sel) = selected
            && sel.accepts_note(note, key)
        {
            inst.set_selected(true);
        }
        out.push(inst);
    });
    sort_key_z_order(&mut out[start..]);
}

/// 同一 key 内的绘制顺序（z-order），完整全序（逐字段定优先级）：
/// 1. `start_tick` 升序（必须保持，GPU cull 的 chunk/tick 分桶依赖它）；
/// 2. `end_tick` 降序——同起点长音符先画（在下）、短音符后画（在上）；
/// 3. `track` 升序（多轨和弦：低轨在下、高轨在上）；
/// 4. `velocity` 升序（其余全同时的最后裁决）。
///
/// 逐字段覆盖全部可变字段，保证 `sort_unstable` 下也是确定顺序（否则相等键
/// 顺序不确定会逐帧闪烁）。`key` 在单 key 桶内恒定，无需参与；选中位不参与。
fn sort_key_z_order(insts: &mut [NoteInstance]) {
    insts.sort_unstable_by(|a, b| {
        a.start_tick
            .cmp(&b.start_tick)
            .then(b.end_tick.cmp(&a.end_tick))
            // packed 去掉选中位后 = key|track|vel；桶内 key 恒定 → 等价 track 升、vel 升。
            .then(
                (a.packed & !NoteInstance::SELECTED_BIT)
                    .cmp(&(b.packed & !NoteInstance::SELECTED_BIT)),
            )
    });
}

/// 统计某 key 的可见实例数（不构造实例，供全量构建预分配）。
fn count_key_instances(
    midi: &dyn NoteSource,
    key: u8,
    track_visible: &[bool],
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
) -> usize {
    let mut n = 0;
    for_each_key_note(midi, key, None, |note| {
        if instance_visible(note, key, track_visible, hidden_notes) {
            n += 1;
        }
    });
    n
}

/// 把某 key 的可见实例写入预分配切片，返回写入数量。
fn fill_key_instances(
    out: &mut [NoteInstance],
    midi: &dyn NoteSource,
    key: u8,
    track_visible: &[bool],
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
) -> usize {
    let mut i = 0;
    for_each_key_note(midi, key, None, |note| {
        if i >= out.len() || !instance_visible(note, key, track_visible, hidden_notes) {
            return;
        }
        out[i] = NoteInstance {
            start_tick: note.start_tick,
            end_tick: note.end_tick,
            packed: NoteInstance::pack(key, note.track, note.velocity),
        };
        i += 1;
    });
    // 与 `build_key_instances` 同一 z-order：同 start_tick 长音符在下、短音符在上。
    sort_key_z_order(&mut out[..i]);
    i
}

/// Build a single ghost note
/// Build note instances (layer 2).
/// Dependencies: selection, track_visible, tick range (scroll_x)
///
/// Output is 16B `NoteInstance` (semantic data only: ticks, key, track, vel).
/// All pixel positions and colors are computed in the GPU vertex shader from
/// uniforms, so scroll_y and key_height changes do NOT invalidate the cache.
///
/// Padding: one screen width on each side of the visible tick range,
/// so fast scrolling doesn't flash empty space before the cache rebuilds.
///
/// Uses `stacker::maybe_grow` to dynamically allocate new stack segments
/// when the current stack is close to overflowing. This prevents
/// STATUS_STACK_BUFFER_OVERRUN on Windows when rendering millions of notes
/// at very low zoom levels.
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn build_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &PianoRollView,
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    selected: Option<&Selection>,
) {
    let (tick_start, tick_end) = view.visible_main_range(view.main_axis_len(w, h));
    let (key_lo, key_hi) = view.visible_cross_range(view.cross_axis_len(w, h));
    // Only build notes whose visible interval overlaps the current viewport.
    // key_notes_in_range looks back via the max_end index, so any note that
    // starts off-screen-left but extends into view is still included — no
    // padding required, regardless of note length.
    let range = Some((tick_start.max(0.0), tick_end));

    let results: Vec<Vec<NoteInstance>> = (key_lo..=key_hi)
        .into_par_iter()
        .filter_map(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let mut local = Vec::new();
                build_key_instances(
                    &mut local,
                    midi,
                    key,
                    track_visible,
                    hidden_notes,
                    range,
                    selected,
                );
                if local.is_empty() { None } else { Some(local) }
            })
        })
        .collect();

    out.extend(results.into_iter().flatten());
}

/// 构建 LOD 摘要层的可见实例（用于 **CPU 渲染路径**，即 GPU cull 关闭时）。
///
/// 复用 GPU 摘要的聚合函数 [`super::build_key_summary`]：对可见 key 取与可见
/// tick 范围相交的**完整块**聚合（块对齐全局网格、不随视图抖动），再按可见
/// tick 范围过滤。方向无关（横向/纵向共用同一份语义数据，像素由 shader 计算）。
///
/// 选中高亮不在实例里逐音符标记：CPU 路径的 shader 会用矩形选区 uniform
/// 实时补位（与 GPU cull 摘要路径一致）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn build_summary_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &PianoRollView,
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    block_ticks: u32,
) {
    let (tick_start, tick_end) = view.visible_main_range(view.main_axis_len(w, h));
    let (key_lo, key_hi) = view.visible_cross_range(view.cross_axis_len(w, h));
    let lo = tick_start.max(0.0);
    let block = block_ticks.max(1);
    // 只扫「与可见 tick 范围相交的完整块」：[floor(lo/block), ceil(hi/block)+1)，
    // 避免为聚合而扫全曲每个 key 的全部音符；`key_notes_in_range` 会回看长音符。
    let block_f = block as f64;
    let scan_lo = (lo / block_f).floor() * block_f;
    let scan_hi = ((tick_end / block_f).ceil() + 1.0) * block_f;
    let scan = Some((scan_lo, scan_hi));

    let results: Vec<Vec<NoteInstance>> = (key_lo..=key_hi)
        .into_par_iter()
        .filter_map(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let mut local = Vec::new();
                build_key_instances(
                    &mut local,
                    midi,
                    key,
                    track_visible,
                    hidden_notes,
                    scan,
                    None,
                );
                let mut segs = super::build_key_summary(key, &local, block);
                segs.retain(|s| (s.start_tick as f64) < tick_end && (s.end_tick as f64) > lo);
                if segs.is_empty() { None } else { Some(segs) }
            })
        })
        .collect();

    out.extend(results.into_iter().flatten());
}

/// Build ALL note instances (no viewport culling) for GPU compute cull.
/// Upload once on MIDI load/change; the GPU cull shader handles per-frame
/// viewport culling.
///
/// Filters by `track_visible` and `hidden_notes` on the CPU so the GPU
/// buffer doesn't contain invisible notes (saving memory and draw calls).
/// When `track_visible`/`hidden_notes` change, call this again to re-upload.
///
/// Returns `(notes, per_key_offsets)` where `per_key_offsets[k]` is the
/// start index of key k's notes in the flat buffer, and `per_key_offsets[KEY_COUNT]`
/// is the total count.
pub fn build_all_notes(
    midi: &dyn NoteSource,
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
) -> (Vec<NoteInstance>, [u32; KEY_COUNT + 1]) {
    // 计数（无实例构造）→ 前缀和一次分配 → 按 key 并行分段写入：
    // 旧实现「每 key 一个 Vec + 串行 extend」在 1.64 亿音符时中间结果与最终
    // 缓冲同时存在（峰值约 5GB），现在峰值只有最终缓冲一份（约 2.6GB）。
    let counts: Vec<usize> = (0u8..=MAX_KEY)
        .into_par_iter()
        .map(|key| count_key_instances(midi, key, track_visible, hidden_notes))
        .collect();

    let mut offsets = [0u32; KEY_COUNT + 1];
    let mut total = 0usize;
    for (k, &c) in counts.iter().enumerate() {
        offsets[k] = total as u32;
        total += c;
    }
    offsets[KEY_COUNT] = total as u32;

    let mut all = vec![
        NoteInstance {
            start_tick: 0,
            end_tick: 0,
            packed: 0,
        };
        total
    ];
    let mut rest = all.as_mut_slice();
    let mut slices: Vec<(&mut [NoteInstance], u8)> = Vec::with_capacity(KEY_COUNT);
    for (k, &c) in counts.iter().enumerate() {
        let (head, tail) = rest.split_at_mut(c);
        slices.push((head, k as u8));
        rest = tail;
    }
    slices.into_par_iter().for_each(|(slice, key)| {
        stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
            let written = fill_key_instances(slice, midi, key, track_visible, hidden_notes);
            debug_assert_eq!(written, slice.len(), "计数与写入数量不一致");
        });
    });

    (all, offsets)
}

/// Build note instances for a single key bucket (for incremental upload).
/// Same filtering logic as `build_all_notes` but scoped to one key.
pub fn build_key_notes(
    midi: &dyn NoteSource,
    key: u8,
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
) -> Vec<NoteInstance> {
    let mut local = Vec::new();
    build_key_instances(
        &mut local,
        midi,
        key,
        track_visible,
        hidden_notes,
        None,
        None,
    );
    local
}

/// Build a single ghost note instance for the pencil tool preview (layer 4).
/// Uses the note's track color at full opacity so it appears as a solid preview
/// on top of the existing notes. Color is fetched from track_colors storage
/// buffer in the shader (same as regular notes).
///
/// `vel=127` 表示"不参与力度着色"（填充 = 原色），见 shader `MAX_FILL_LIGHTEN`。
pub fn build_ghost_note(
    out: &mut Vec<NoteInstance>,
    start_tick: u32,
    end_tick: u32,
    key: u8,
    track: u16,
    // true = 选中态预览（选择工具拖动）：按选中样式加深，与选框同步移动；
    // false = 新建预览（铅笔/刷子）：保持原色。
    selected: bool,
    _theme: &GpuTheme,
) {
    let mut inst = NoteInstance {
        start_tick,
        end_tick,
        packed: NoteInstance::pack(key, track, 127),
    };
    inst.set_selected(selected);
    out.push(inst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_midi;
    use yinhe_types::TimelineViewBase;

    fn make_view() -> PianoRollView {
        PianoRollView {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            key_height: 12.0,
            viewport_h: 0.0,
            main_size: 0.0,

            orientation: yinhe_types::Orientation::Horizontal,
        }
    }

    /// 全量构建：多 key 的 offsets/总数正确，分段内容与单 key 构建一致，
    /// 且 track_visible/hidden 过滤在计数与写入两遍中一致。
    #[test]
    fn test_build_all_notes_multi_key_offsets_and_filters() {
        let midi = make_midi(vec![
            (0, 0, 480, 0, 100),
            (0, 480, 960, 0, 100),
            (60, 0, 480, 1, 100),
            (64, 0, 480, 0, 100),
        ]);
        let track_visible = vec![true, true];
        let mut hidden = std::collections::HashSet::new();
        hidden.insert((0u16, 0u32, 64u8));

        let (all, offsets) = build_all_notes(&midi, &hidden, &track_visible);
        assert_eq!(offsets[0], 0);
        assert_eq!(offsets[1], 2, "key 0 两个实例");
        assert_eq!(offsets[61], 3, "key 60 一个实例");
        assert_eq!(offsets[64], 3, "key 64 被 hidden 过滤");
        assert_eq!(offsets[KEY_COUNT], 3);
        assert_eq!(all.len(), 3);

        let k60 = build_key_notes(&midi, 60, &hidden, &track_visible);
        assert_eq!(
            &all[offsets[60] as usize..offsets[61] as usize],
            k60.as_slice(),
            "分段内容应与单 key 构建一致"
        );
    }

    #[test]
    fn test_build_notes_basic() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            None,
        );
        assert!(!out.is_empty(), "should produce note instances");
        let note = &out[0];
        assert_eq!(note.start_tick, 0);
        assert_eq!(note.end_tick, 480);
        // packed = key(100) | track(0) | vel(100)
        assert_eq!(note.packed & 0xFF, 100, "key");
        assert_eq!((note.packed >> 8) & 0xFFFF, 0, "track");
        assert_eq!((note.packed >> 24) & 0xFF, 100, "velocity");
    }

    /// CPU LOD：同一 block 内的音符合并为一段，跨 block 另起一段。
    #[test]
    fn test_build_summary_notes_merges_blocks() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![
            (60, 0, 10, 0, 100),
            (60, 100, 110, 0, 100), // 与上一条同属 block(256)
            (60, 300, 310, 0, 100), // 下一 block
        ]);
        let mut view = make_view();
        // 让 key 60 进入横向可见范围。
        view.base.scroll_y = 500.0;
        let track_visible = vec![true];
        let hidden = std::collections::HashSet::new();

        build_summary_notes(
            &mut out,
            1000.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            256,
        );
        assert_eq!(out.len(), 2, "两个 block 各一段");
        assert_eq!(out[0].packed & 0xFF, 60, "key 保留");
        assert_eq!((out[0].start_tick, out[0].end_tick), (0, 110), "块内合并");
        assert_eq!((out[1].start_tick, out[1].end_tick), (300, 310));
    }

    #[test]
    fn test_build_notes_hidden_track() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let track_visible = vec![false];

        let hidden = std::collections::HashSet::new();
        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            None,
        );
        assert!(out.is_empty(), "notes on hidden track should be skipped");
    }

    #[test]
    fn test_build_notes_tag_is_track_index() {
        let mut out: Vec<NoteInstance> = Vec::new();
        // Create a note on track 2
        let midi = make_midi(vec![(100, 0, 480, 2, 100)]);
        let view = make_view();
        let track_visible = vec![true, true, true];

        let hidden = std::collections::HashSet::new();
        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            None,
        );
        assert_eq!((out[0].packed >> 8) & 0xFFFF, 2, "track should be 2");
    }

    #[test]
    fn test_build_notes_tag_is_track_zero() {
        let mut out: Vec<NoteInstance> = Vec::new();
        // Create a note on track 0
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            None,
        );
        assert_eq!((out[0].packed >> 8) & 0xFFFF, 0, "track should be 0");
    }

    #[test]
    fn test_build_notes_multiple_keys() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![
            (100, 0, 480, 0, 100),
            (104, 0, 480, 0, 80),
            (107, 0, 480, 0, 90),
        ]);
        let view = make_view();
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            None,
        );
        assert_eq!(out.len(), 3, "should produce 3 note instances");
    }

    #[test]
    fn test_build_notes_long_note_crossing_left_edge() {
        // A note that starts far off-screen-left but extends into the viewport
        // must still be built (no padding; relies on the max_end look-back).
        let mut out: Vec<NoteInstance> = Vec::new();
        // Note spans tick 0..100000, key 100.
        let midi = make_midi(vec![(100, 0, 100000, 0, 100)]);
        let mut view = make_view();
        // Scroll right so the note's start is far off-screen to the left,
        // but its body still covers the viewport.
        view.base.scroll_x = 5000.0;
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            None,
        );
        assert!(
            !out.is_empty(),
            "long note crossing the left edge must be included"
        );
    }

    #[test]
    fn test_build_notes_skips_fully_offscreen() {
        // A short note entirely to the left of the viewport must NOT be built.
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let mut view = make_view();
        view.base.scroll_x = 5000.0; // viewport starts well past tick 480
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            None,
        );
        assert!(out.is_empty(), "note fully off-screen-left must be culled");
    }

    /// ghost 选中位：选择工具的拖动预览带 SELECTED_BIT（与选框同步的深色），
    /// 铅笔/刷子预览保持原色。
    #[test]
    fn ghost_note_selected_bit() {
        let theme = GpuTheme::from_base(yinhe_theme::base::BaseColors::DARK);
        let mut out = Vec::new();
        build_ghost_note(&mut out, 100, 200, 60, 0, true, &theme);
        build_ghost_note(&mut out, 300, 400, 62, 0, false, &theme);
        assert!(out[0].is_selected(), "选择工具拖动的 ghost 应为选中态");
        assert!(!out[1].is_selected(), "铅笔/刷子预览保持非选中");
        assert_eq!(out[0].velocity(), 127, "ghost 力度位仍为原色基准");
    }

    /// 选中位：传入 Selection 后，成员音符实例带 SELECTED_BIT，力度不被污染。
    #[test]
    fn test_build_notes_marks_selected_members() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100), (100, 600, 960, 0, 80)]);
        let view = make_view();
        let track_visible = vec![true];
        let hidden = std::collections::HashSet::new();

        let mut sel = yinhe_core::Selection::default();
        sel.add_rect(0, 480, 100, 100);
        sel.materialize_pending(&midi);

        build_notes(
            &mut out,
            800.0,
            500.0,
            &midi,
            &view,
            &hidden,
            &track_visible,
            Some(&sel),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(
            out.iter().filter(|n| n.is_selected()).count(),
            1,
            "只有成员音符带选中位"
        );
        let sel_inst = out.iter().find(|n| n.is_selected()).unwrap();
        assert_eq!(sel_inst.start_tick, 0);
        assert_eq!(sel_inst.velocity(), 100, "选中位不得污染力度字段");
        assert_eq!(
            out.iter().find(|n| !n.is_selected()).unwrap().velocity(),
            80
        );
    }

    /// 同 key 的 z-order：start_tick 升序；同 start_tick 时 end_tick 降序
    /// （长音符在下、短音符在上，短音符尾部不被遮）。
    #[test]
    fn test_key_z_order_long_behind_short() {
        let inst = |start: u32, end: u32| NoteInstance {
            start_tick: start,
            end_tick: end,
            packed: NoteInstance::pack(60, 0, 100),
        };
        // 同起点：长 [0,400]、短 [0,100]；不同起点：[200,300]。
        let mut v = vec![inst(0, 100), inst(0, 400), inst(200, 300)];
        sort_key_z_order(&mut v);
        let order: Vec<(u32, u32)> = v.iter().map(|n| (n.start_tick, n.end_tick)).collect();
        assert_eq!(
            order,
            vec![(0, 400), (0, 100), (200, 300)],
            "同起点长在前(在下)、短在后(在上)；start_tick 仍升序"
        );
    }

    /// 全序补全：同 start/end 时按 track 升序（多轨和弦不闪烁）。
    #[test]
    fn test_key_z_order_track_tiebreak() {
        let inst = |track: u16| NoteInstance {
            start_tick: 0,
            end_tick: 100,
            packed: NoteInstance::pack(60, track, 100),
        };
        let mut v = vec![inst(2), inst(0), inst(1)];
        sort_key_z_order(&mut v);
        let tracks: Vec<u16> = v
            .iter()
            .map(|n| ((n.packed >> 8) & 0xFFFF) as u16)
            .collect();
        assert_eq!(tracks, vec![0, 1, 2], "track 升序，确定不闪烁");
    }
}
