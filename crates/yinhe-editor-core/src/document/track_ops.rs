//! Track structure operations: add, remove, move.

use std::sync::Arc;

use crate::history::UndoAction;

use super::Document;

/// 新建音轨的规格（新建音轨对话框 → Document::add_tracks_batch）。
#[derive(Clone, Copy, Debug)]
pub struct NewTrackSpec {
    /// 音轨种类（MIDI / 乐器；音频轨为预留，暂不能创建）。
    pub kind: yinhe_core::TrackKind,
    /// MIDI port（0 起，UI 显示 A..P）。仅 MIDI 轨有意义。
    pub port: u8,
    /// MIDI channel（0 起，UI 显示 1..16）。仅 MIDI 轨有意义。
    pub channel: u8,
    /// 乐器通道（0 起，UI 显示 1 起）。仅乐器轨有值；
    /// 多条乐器轨同号 = 共享同一个 CLAP 插件实例。
    pub instrument_channel: Option<u16>,
}

/// 现有轨道中自动命名 "Track N" 的最大 N（Conductor/导入的真实轨名如
/// "Piano" 不参与）。新音轨编号 = 最大 N + 1 递增，不随插入位置变化：
/// 已有 16 条音轨时在 Track 2 下方插入，新音轨应为 Track 17 而非 Track 3。
/// 不能按 tracks.len() 取名——删除中间音轨后数量会撞上仍存在的编号
/// （删掉 Track 3 后剩 16 轨，但 Track 16 还在）。
/// 不按通道命名：通道经常被改，轨道号相对固定。
/// MIDI 轨与乐器轨共用同一编号序列，避免重名（编号只是名字，与通道无关）。
fn max_track_number(model: &yinhe_core::YinModel) -> u32 {
    model
        .tracks
        .iter()
        .filter_map(|t| {
            t.name
                .strip_prefix("Track ")
                .and_then(|s| s.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
}

impl Document {
    /// Insert a new MIDI track after `after_idx`. Returns UndoAction.
    /// The new track gets port 0, channel = first unused channel on port 0.
    pub fn add_track(&mut self, after_idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        let num_tracks = model.tracks.len();
        if after_idx >= num_tracks {
            return None;
        }
        // Don't allow adding after conductor if it would insert before conductor
        let insert_idx = after_idx + 1;

        // Find a free channel on port 0
        let used_channels: std::collections::HashSet<u8> = model
            .tracks
            .iter()
            .filter(|t| t.port == 0)
            .map(|t| t.channel)
            .collect();
        let channel = (0..16u8).find(|c| !used_channels.contains(c)).unwrap_or(0);

        let mut new_track = yinhe_core::TrackData::new(0, channel);
        // 命名规则说明见 max_track_number。
        new_track.name = format!("Track {}", max_track_number(model) + 1);

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.insert(insert_idx, Arc::new(new_track));

        // Remap notes: track >= insert_idx gets +1
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| {
                if i >= insert_idx {
                    (i + 1) as u16
                } else {
                    i as u16
                }
            })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| {
                if i == insert_idx {
                    u16::MAX
                } else if i < insert_idx {
                    i as u16
                } else {
                    (i - 1) as u16
                }
            })
            .collect();

        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap to notes
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            for note in bucket.iter_mut() {
                // 越界音符按删除处理（unwrap_or(u16::MAX)），避免 panic（规则 17）。
                note.track = note_remap
                    .get(note.track as usize)
                    .copied()
                    .unwrap_or(u16::MAX);
            }
        }

        model.rebuild();
        self.data.bump_revision();

        // Update edit state
        self.sync_track_caches();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(insert_idx as u16);
        // 新增 track 后，editing_track 后面的索引要 +1
        if let Some(t) = self.edit.editing_track
            && (t as usize) >= insert_idx
        {
            self.edit.editing_track = Some(t + 1);
        }

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
            deleted_notes: Vec::new(),
        })
    }

    /// 批量新建音轨：按 `specs` 追加到最后一条轨之后，整个批次只产生
    /// 一个 UndoAction（撤销一次全撤）。命名沿用「Track N = 现有最大编号 + 1」
    /// 递增（见 max_track_number）。通道分配规则（自动/手动顺延）由调用方用
    /// channel_alloc 的纯函数算好后写进 specs，这里只负责落地。
    pub fn add_tracks_batch(&mut self, specs: &[NewTrackSpec]) -> Option<UndoAction> {
        if specs.is_empty() {
            return None;
        }
        let insert_idx = self.data.model.tracks.len();
        let next_num = max_track_number(&self.data.model) + 1;

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = self.data.model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        for (num, spec) in (next_num..).zip(specs.iter()) {
            let mut new_track = yinhe_core::TrackData::new(spec.port, spec.channel);
            new_track.kind = spec.kind;
            new_track.instrument_channel = spec.instrument_channel;
            new_track.name = format!("Track {}", num);
            model.tracks.push(Arc::new(new_track));
        }

        // 末尾追加：既有音符的轨号不变（恒等 remap），新轨还没有音符。
        let note_remap: Vec<u16> = (0..tracks_before.len()).map(|i| i as u16).collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| if i < insert_idx { i as u16 } else { u16::MAX })
            .collect();
        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        let total_tracks = model.tracks.len();

        model.rebuild();
        self.data.bump_revision();

        // Update edit state：选中本批新建的全部音轨。
        // 追加在末尾，editing_track 无需调整。
        self.sync_track_caches();
        self.edit.track_selected.clear();
        for i in insert_idx..total_tracks {
            self.edit.track_selected.insert(i as u16);
        }

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
            deleted_notes: Vec::new(),
        })
    }

    /// Remove the track at `idx`. Notes belonging to it are deleted.
    pub fn remove_track(&mut self, idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        if idx >= model.tracks.len() {
            return None;
        }
        // Don't remove conductor track
        if self.edit.conductor_track_idx == Some(idx as u16) {
            return None;
        }
        // Don't remove if only 2 tracks (conductor + 1)
        if model.tracks.len() <= 2 {
            return None;
        }

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // 先捕获被删轨道上的音符（含 key），供 undo 恢复。
        // 必须在 make_mut + retain 之前从只读 model 读取，否则音符已被删。
        let deleted_notes: Vec<(yinhe_types::Note, u8)> = model
            .notes
            .iter()
            .enumerate()
            .flat_map(|(key, bucket)| {
                bucket
                    .iter()
                    .filter(|n| n.track as usize == idx)
                    .map(move |n| (*n, key as u8))
            })
            .collect();

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.remove(idx);

        // Remap: track < idx stays, track == idx is deleted (u16::MAX), track > idx gets -1
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| {
                if i == idx {
                    u16::MAX
                } else if i > idx {
                    (i - 1) as u16
                } else {
                    i as u16
                }
            })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| if i < idx { i as u16 } else { (i + 1) as u16 })
            .collect();

        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap: delete notes on removed track, shift others
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            // 越界音符按删除处理（unwrap_or(u16::MAX)），避免 panic（规则 17）。
            bucket.retain(|n| {
                note_remap
                    .get(n.track as usize)
                    .copied()
                    .unwrap_or(u16::MAX)
                    != u16::MAX
            });
            for note in bucket.iter_mut() {
                note.track = note_remap
                    .get(note.track as usize)
                    .copied()
                    .unwrap_or(u16::MAX);
            }
        }
        // Mark all buckets dirty since we may have removed notes from any
        for k in 0..128 {
            model.mark_dirty(k as u8);
        }
        model.rebuild();
        let num_tracks = model.tracks.len();
        self.data.bump_revision();

        // Update edit state
        self.sync_track_caches();
        self.edit.track_selected.clear();
        // Select the track that took its place (or last track)
        let new_sel = idx.min(num_tracks - 1) as u16;
        self.edit.track_selected.insert(new_sel);
        // 删除 track 后，editing_track 同步调整：
        //  - 等于被删 track：清空
        //  - 大于被删 track：-1
        match self.edit.editing_track {
            Some(t) if t as usize == idx => self.edit.editing_track = None,
            Some(t) if (t as usize) > idx => self.edit.editing_track = Some(t - 1),
            _ => {}
        }

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
            deleted_notes,
        })
    }

    /// Move track at `from_idx` to `to_idx`. Other tracks shift to fill the gap.
    pub fn move_track(&mut self, from_idx: usize, to_idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        let num_tracks = model.tracks.len();
        if from_idx >= num_tracks || to_idx >= num_tracks || from_idx == to_idx {
            return None;
        }
        // Don't move conductor track
        if self.edit.conductor_track_idx == Some(from_idx as u16)
            || self.edit.conductor_track_idx == Some(to_idx as u16)
        {
            return None;
        }

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.remove(from_idx);
        model.tracks.insert(to_idx, track);

        // Build remap table
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| {
                if i == from_idx {
                    to_idx as u16
                } else if from_idx < to_idx && i > from_idx && i <= to_idx {
                    (i - 1) as u16
                } else if from_idx > to_idx && i >= to_idx && i < from_idx {
                    (i + 1) as u16
                } else {
                    i as u16
                }
            })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| {
                if i == to_idx {
                    from_idx as u16
                } else if from_idx < to_idx && i >= from_idx && i < to_idx {
                    (i + 1) as u16
                } else if from_idx > to_idx && i > to_idx && i <= from_idx {
                    (i - 1) as u16
                } else {
                    i as u16
                }
            })
            .collect();

        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap to notes
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            for note in bucket.iter_mut() {
                // 越界音符按删除处理（unwrap_or(u16::MAX)），避免 panic（规则 17）。
                note.track = note_remap
                    .get(note.track as usize)
                    .copied()
                    .unwrap_or(u16::MAX);
            }
        }

        model.rebuild();
        self.data.bump_revision();

        // Update edit state
        self.edit.track_info_cache = self.data.track_info();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(to_idx as u16);
        // 移动 track 后，editing_track 用同样的 remap 规则更新
        if let Some(t) = self.edit.editing_track {
            let t_usize = t as usize;
            let new_t = if t_usize == from_idx {
                to_idx
            } else if from_idx < to_idx && t_usize > from_idx && t_usize <= to_idx {
                t_usize - 1
            } else if from_idx > to_idx && t_usize >= to_idx && t_usize < from_idx {
                t_usize + 1
            } else {
                t_usize
            };
            self.edit.editing_track = Some(new_t as u16);
        }

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
            deleted_notes: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Document, NewTrackSpec};

    /// 新音轨编号 = 已有音轨数 + 1，而不是插入位置：
    /// 在 16 条音轨的工程里于 Track 2 下方插入，新音轨应为 Track 17（而非 Track 3），
    /// 且全工程音轨名不重复（Track 3 已存在）。
    #[test]
    fn add_track_names_by_total_count_not_insert_position() {
        let mut doc = Document::empty(); // Conductor + Track 1..16
        doc.add_track(2); // 在 Track 2 下方插入

        let model = doc.model();
        assert_eq!(model.tracks.len(), 18);
        assert_eq!(model.tracks[3].name, "Track 17");

        let mut names: Vec<&str> = model.tracks.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), model.tracks.len(), "新音轨名与既有音轨重名");
    }

    /// 删除中间音轨后数量会撞上仍存在的编号（删掉 Track 3 后剩 16 轨，
    /// 但 Track 16 还在），新音轨必须取最大已用编号 + 1（Track 17）。
    #[test]
    fn add_track_after_removing_middle_track_avoids_collision() {
        let mut doc = Document::empty(); // Conductor + Track 1..16
        assert!(doc.remove_track(3).is_some()); // 删除 Track 3
        assert_eq!(doc.model().tracks.len(), 16);

        doc.add_track(2); // 在 Track 2 下方插入

        let model = doc.model();
        assert_eq!(model.tracks.len(), 17);
        assert_eq!(model.tracks[3].name, "Track 17");

        let mut names: Vec<&str> = model.tracks.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), model.tracks.len(), "新音轨名与既有音轨重名");
    }

    /// 批量创建：追加到末尾、按 spec 设置 kind/通道、命名从最大编号 + 1 递增
    /// （MIDI 轨与乐器轨统一编号），新建的整批被选中。
    #[test]
    fn add_tracks_batch_appends_with_specs() {
        let mut doc = Document::empty(); // Conductor + Track 1..16（17 条）
        let specs = vec![
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Midi,
                port: 1,
                channel: 0,
                instrument_channel: None,
            },
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Midi,
                port: 1,
                channel: 1,
                instrument_channel: None,
            },
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Instrument,
                port: 0,
                channel: 0,
                instrument_channel: Some(0),
            },
        ];
        assert!(doc.add_tracks_batch(&specs).is_some());

        let model = doc.model();
        assert_eq!(model.tracks.len(), 20);
        assert_eq!(model.tracks[17].name, "Track 17");
        assert_eq!(model.tracks[18].name, "Track 18");
        assert_eq!(model.tracks[19].name, "Track 19");
        assert_eq!((model.tracks[17].port, model.tracks[17].channel), (1, 0));
        assert_eq!(model.tracks[19].kind, yinhe_core::TrackKind::Instrument);
        assert_eq!(model.tracks[19].instrument_channel, Some(0));
        // 新建的 3 条全部被选中
        assert_eq!(doc.edit.track_selected.len(), 3);
        assert!(doc.edit.track_selected.contains(&19));
    }

    /// 批量创建只产生一个 UndoAction：undo 一次（reversed + redo，即
    /// UndoStack 内部的 undo 路径）整批全撤。
    #[test]
    fn add_tracks_batch_undoes_all_at_once() {
        let mut doc = Document::empty(); // 17 条
        let specs = vec![
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Midi,
                port: 0,
                channel: 15,
                instrument_channel: None,
            },
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Midi,
                port: 1,
                channel: 0,
                instrument_channel: None,
            },
        ];
        let action = doc.add_tracks_batch(&specs).expect("批量创建应成功");
        assert_eq!(doc.model().tracks.len(), 19);

        action.reversed().redo(&mut doc);
        assert_eq!(doc.model().tracks.len(), 17);
        assert_eq!(doc.model().tracks[16].name, "Track 16");
    }

    /// 空批次是 no-op，不产生 undo。
    #[test]
    fn add_tracks_batch_empty_is_noop() {
        let mut doc = Document::empty();
        assert!(doc.add_tracks_batch(&[]).is_none());
        assert_eq!(doc.model().tracks.len(), 17);
    }
}
