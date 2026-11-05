//! 复制：原地复制（源保留）与 Alt 拖动复制（选区跟随副本）。

use std::sync::Arc;

use crate::batch_ops;
use crate::document::{Document, RepeatTile, repeat_tiles_x};
use crate::history::{NoteDelta, UndoAction};

/// Riff 的纵向轴：PR 复制到不同 key，AR 复制到不同 track。
#[derive(Clone, Copy)]
enum FillAxis {
    Key,
    Track,
}

/// 一次 Riff 的选区范围与延伸距离。
#[derive(Clone, Copy)]
struct RepeatExtent {
    sel_start: i64,
    sel_end: i64,
    extent_x: i64,
    y_start: i64,
    y_end: i64,
    extent_y: i64,
}

#[allow(clippy::too_many_arguments)]
fn push_repeat_note(
    model: &yinhe_core::YinModel,
    new_by_key: &mut std::collections::HashMap<u8, Vec<yinhe_types::Note>>,
    allow_overlap: bool,
    new_key: u8,
    new_track: u16,
    new_start: i64,
    new_end: i64,
    velocity: u8,
) {
    let start = new_start.max(0);
    let mut end = new_end.max(0);
    if end <= start {
        return;
    }
    let max_tick = i64::from(u32::MAX);
    if start > max_tick {
        return;
    }
    end = end.min(max_tick);
    if end <= start {
        return;
    }
    let start = start as u32;
    let end = end as u32;
    if !allow_overlap && batch_ops::has_overlapping_note(model, new_track, new_key, start, end) {
        return;
    }
    new_by_key
        .entry(new_key)
        .or_default()
        .push(yinhe_types::Note {
            id: 0,
            start_tick: start,
            end_tick: end,
            velocity,
            track: new_track,
        });
}

/// 把一个源音符复制到 `(xt, yt)` 对应的格子，按部分窗口裁剪。
#[allow(clippy::too_many_arguments)]
fn copy_cell(
    model: &yinhe_core::YinModel,
    new_by_key: &mut std::collections::HashMap<u8, Vec<yinhe_types::Note>>,
    allow_overlap: bool,
    axis: FillAxis,
    num_tracks: usize,
    note: &yinhe_types::Note,
    old_key: u8,
    xt: RepeatTile,
    yt: RepeatTile,
    width: i64,
) {
    // 纵向：note 的 key/track 必须落在源窗口内；最后一段只取前/后若干行。
    let note_y = match axis {
        FillAxis::Key => i64::from(old_key),
        FillAxis::Track => i64::from(note.track),
    };
    if note_y < yt.src_start || note_y >= yt.src_start + yt.len {
        return;
    }
    let new_y = yt.dst_start + (note_y - yt.src_start);
    let (new_key, new_track) = match axis {
        FillAxis::Key => {
            if !(0..=i64::from(yinhe_types::MAX_KEY)).contains(&new_y) {
                return;
            }
            (new_y as u8, note.track)
        }
        FillAxis::Track => {
            if new_y < 0 || new_y as usize >= num_tracks {
                return;
            }
            if model.tracks[new_y as usize].kind != yinhe_core::TrackKind::Midi {
                return;
            }
            (old_key, new_y as u16)
        }
    };
    // 横向：完整周期复制整条音符，部分周期按源窗口裁剪。
    let full = xt.len == width;
    let src_lo = if full { i64::MIN } else { xt.src_start };
    let src_hi = if full {
        i64::MAX
    } else {
        xt.src_start + xt.len
    };
    let src_start = i64::from(note.start_tick).max(src_lo);
    let src_end = i64::from(note.end_tick).min(src_hi);
    if src_end <= src_start {
        return;
    }
    let offset = xt.dst_start - xt.src_start;
    push_repeat_note(
        model,
        new_by_key,
        allow_overlap,
        new_key,
        new_track,
        src_start + offset,
        src_end + offset,
        note.velocity,
    );
}

impl Document {
    /// Duplicate all selected notes. Returns an `UndoAction` if any notes were duplicated.
    pub fn duplicate_selected(&mut self) -> Option<UndoAction> {
        self.duplicate_impl(0, 0, true)
    }

    /// Duplicate selected notes and offset the copies by `(delta_ticks, delta_keys)`.
    ///
    /// 原音符保留不动，副本平移到目标位置；选区同步移到副本范围，便于连续操作。
    /// 用于 Alt+拖动复制：一步操作，一个 undo entry。
    pub fn duplicate_selected_to(
        &mut self,
        delta_ticks: i64,
        delta_keys: i32,
    ) -> Option<UndoAction> {
        self.duplicate_impl(delta_ticks, delta_keys, false)
    }

    /// PR Riff：横向按 `extent_ticks`、纵向按 `extent_keys` 延伸到选区外。
    ///
    /// `extent_ticks` / `extent_keys` 是拖拽距离（可为负），不是完整选区步数。
    /// 最后一段按剩余距离裁剪：选区 1 小节、横向延伸到 5/4 小节时，生成
    /// 1 小节完整副本 + 开头 1/4 小节的部分副本。纵向同理：选两轨向下
    /// 延伸一轨时，只复制第一轨（选区的一半）到第三轨。
    pub fn repeat_selected_keys_extent(
        &mut self,
        sel_start: i64,
        sel_end: i64,
        extent_ticks: i64,
        key_start: i64,
        key_end: i64,
        extent_keys: i64,
    ) -> Option<UndoAction> {
        self.repeat_selected_extent_impl(
            RepeatExtent {
                sel_start,
                sel_end,
                extent_x: extent_ticks,
                y_start: key_start,
                y_end: key_end,
                extent_y: extent_keys,
            },
            FillAxis::Key,
        )
    }

    /// AR Riff：横向按 `extent_ticks`、纵向按 `extent_rows` 延伸到选区外。
    pub fn repeat_selected_tracks_extent(
        &mut self,
        sel_start: i64,
        sel_end: i64,
        extent_ticks: i64,
        row_start: i64,
        row_end: i64,
        extent_rows: i64,
    ) -> Option<UndoAction> {
        self.repeat_selected_extent_impl(
            RepeatExtent {
                sel_start,
                sel_end,
                extent_x: extent_ticks,
                y_start: row_start,
                y_end: row_end,
                extent_y: extent_rows,
            },
            FillAxis::Track,
        )
    }

    fn repeat_selected_extent_impl(
        &mut self,
        extent: RepeatExtent,
        axis: FillAxis,
    ) -> Option<UndoAction> {
        let RepeatExtent {
            sel_start,
            sel_end,
            extent_x,
            y_start,
            y_end,
            extent_y,
        } = extent;
        if self.edit.selected.is_empty() || (extent_x == 0 && extent_y == 0) {
            return None;
        }
        let width = sel_end - sel_start;
        let y_span = y_end - y_start;
        if width <= 0 || y_span <= 0 {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let num_tracks = self.data.model.tracks.len();
        let mut x_tiles = Vec::with_capacity(4);
        x_tiles.push(RepeatTile {
            src_start: sel_start,
            dst_start: sel_start,
            len: width,
        });
        x_tiles.extend(repeat_tiles_x(sel_start, sel_end, extent_x));
        let mut y_tiles = Vec::with_capacity(4);
        y_tiles.push(RepeatTile {
            src_start: y_start,
            dst_start: y_start,
            len: y_span,
        });
        y_tiles.extend(repeat_tiles_x(y_start, y_end, extent_y));
        let after = {
            let model = &self.data.model;
            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            batch_ops::for_each_selected(model, &self.edit.selected, |note, old_key| {
                let (x_origin, x_tiles) = (x_tiles.first(), &x_tiles[1..]);
                for &yt in y_tiles.iter() {
                    let y_origin = yt.src_start == y_start && yt.dst_start == y_start;
                    // 原选区本身不复制；只处理至少一个轴有偏移的格子。
                    if y_origin {
                        for &xt in x_tiles {
                            copy_cell(
                                model,
                                &mut new_by_key,
                                allow_overlap,
                                axis,
                                num_tracks,
                                note,
                                old_key,
                                xt,
                                yt,
                                width,
                            );
                        }
                        continue;
                    }
                    // y 方向有偏移：x=0 的整列 + 横向延伸段。
                    if let Some(xt) = x_origin {
                        copy_cell(
                            model,
                            &mut new_by_key,
                            allow_overlap,
                            axis,
                            num_tracks,
                            note,
                            old_key,
                            *xt,
                            yt,
                            width,
                        );
                    }
                    for &xt in x_tiles {
                        copy_cell(
                            model,
                            &mut new_by_key,
                            allow_overlap,
                            axis,
                            num_tracks,
                            note,
                            old_key,
                            xt,
                            yt,
                            width,
                        );
                    }
                }
            });
            if new_by_key.is_empty() {
                return None;
            }
            let model = Arc::make_mut(&mut self.data.model);
            let mut after = Vec::new();
            for (key, notes) in new_by_key.iter_mut() {
                for n in notes.iter_mut() {
                    n.id = model.alloc_note_id();
                    after.push((*n, *key));
                }
            }
            batch_ops::insert_batch(model, new_by_key);
            after
        };
        self.data.rebuild_model_dirty();
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }

    /// 复制的公共实现（`duplicate_selected` / `duplicate_selected_to`）。
    ///
    /// 流式两遍（不物化 `collect_selected`）：`auto_span` 时先求选区跨度作偏移，
    /// 再逐音符构建副本批次。1.64 亿全选下省去一份 ~3GB 中间 `Vec`。
    /// 「允许新重叠音符」关闭时，与操作前已有音符重叠的副本逐个跳过；
    /// 全部被跳过则不动选区、不产生 undo。
    fn duplicate_impl(
        &mut self,
        delta_ticks: i64,
        delta_keys: i32,
        auto_span: bool,
    ) -> Option<UndoAction> {
        if self.edit.selected.is_empty() {
            return None;
        }
        let allow_overlap = self.edit.allow_overlapping_notes;
        let after = {
            let model = &self.data.model;

            // 自动偏移（duplicate_selected）：取选区跨度（流式，不物化）。
            let (delta_ticks, delta_keys) = if auto_span {
                let mut min_start = u32::MAX;
                let mut max_end = 0u32;
                batch_ops::for_each_selected(model, &self.edit.selected, |n, _| {
                    min_start = min_start.min(n.start_tick);
                    max_end = max_end.max(n.end_tick);
                });
                if min_start == u32::MAX {
                    return None;
                }
                ((((max_end - min_start).max(1)) as i64), 0)
            } else {
                (delta_ticks, delta_keys)
            };

            let mut new_by_key: std::collections::HashMap<u8, Vec<yinhe_types::Note>> =
                std::collections::HashMap::new();
            batch_ops::for_each_selected(model, &self.edit.selected, |note, old_key| {
                let new_key =
                    ((old_key as i32) + delta_keys).clamp(0, yinhe_types::MAX_KEY as i32) as u8;
                let new_start = (note.start_tick as i64 + delta_ticks).max(0) as u32;
                let length = note.end_tick - note.start_tick;
                // 「允许新重叠音符」关闭：副本与操作前已有音符重叠 → 跳过该副本。
                // 检查在批量插入前进行，批次内部互不影响（含各副本的原音符）。
                if !allow_overlap
                    && batch_ops::has_overlapping_note(
                        model,
                        note.track,
                        new_key,
                        new_start,
                        new_start + length,
                    )
                {
                    return;
                }
                // id 延后到插入前统一发号（流式阶段借用 &model，无法 alloc）。
                new_by_key
                    .entry(new_key)
                    .or_default()
                    .push(yinhe_types::Note {
                        id: 0,
                        start_tick: new_start,
                        end_tick: new_start + length,
                        velocity: note.velocity,
                        track: note.track,
                    });
            });

            // 全部被跳过：不动选区、不产生 undo。
            if new_by_key.is_empty() {
                return None;
            }

            let model = Arc::make_mut(&mut self.data.model);
            let mut after: Vec<(yinhe_types::Note, u8)> = Vec::new();
            for (key, notes) in new_by_key.iter_mut() {
                for n in notes.iter_mut() {
                    n.id = model.alloc_note_id();
                    after.push((*n, *key));
                }
            }
            batch_ops::insert_batch(model, new_by_key);

            // 选区跟随副本（新 id 精确跟随，落点处的其他音符不纳入）。
            if auto_span {
                self.edit.offset_sel_ticks(delta_ticks);
            } else {
                self.edit.selected.offset(delta_ticks, delta_keys);
            }
            let mut copy_ids = yinhe_types::NoteBitsetBuilder::new();
            for (n, _) in &after {
                copy_ids.insert(n.id);
            }
            self.edit.selected.set_members_bitset(copy_ids.build());
            after
        };
        self.data.rebuild_model_dirty();
        // 复制会改变音符数据但 per-key revision 的变化不足以让 GPU 层缓存
        // 失效（gpu_upload 先查 data.revision），必须同步 bump 文档 revision。
        self.data.bump_revision();
        Some(UndoAction::Notes(NoteDelta {
            before: vec![],
            after,
        }))
    }
}
