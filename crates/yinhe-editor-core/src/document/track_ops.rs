//! Track structure operations: add, remove, move.

use std::sync::Arc;

use crate::history::UndoAction;

use super::Document;

/// 按 `remap`（old→new 的排列）就地重排一个「按音轨索引」的数组。
/// 长度不匹配时不动（结构变化过程中可能短暂不等长）。
fn remap_indexed<T>(v: &mut Vec<T>, remap: &[u16]) {
    if v.len() != remap.len() {
        return;
    }
    let mut old: Vec<Option<T>> = std::mem::take(v).into_iter().map(Some).collect();
    let mut slots: Vec<Option<T>> = (0..old.len()).map(|_| None).collect();
    for (i, &new_idx) in remap.iter().enumerate() {
        if let Some(slot) = slots.get_mut(new_idx as usize) {
            *slot = old[i].take();
        }
    }
    *v = slots.into_iter().flatten().collect();
}

/// 新建音轨的规格（新建音轨对话框 → Document::add_tracks_batch）。
#[derive(Clone, Copy, Debug)]
pub struct NewTrackSpec {
    /// 音轨种类（MIDI / 音频）。
    pub kind: yinhe_core::TrackKind,
    /// MIDI port（0 起，UI 显示 A..P）。仅 MIDI 轨有意义。
    pub port: u8,
    /// MIDI channel（0 起，UI 显示 1..16）。仅 MIDI 轨有意义。
    pub channel: u8,
    /// 音频通道（0 起，UI 显示 1 起）。仅音频轨有值；
    /// 多条音频轨同号 = 共享同一条混音 strip/insert 链。
    pub audio_channel: Option<u16>,
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

/// 找出 `0..target_len` 中没被 `remap` 指向的目标槽位（撤销删除轨时恢复的
/// 那条轨）。没有缺失返回 `None`。
pub(crate) fn missing_target_index(remap: &[u16], target_len: usize) -> Option<usize> {
    let mut seen = vec![false; target_len];
    for &t in remap {
        if let Some(slot) = seen.get_mut(t as usize) {
            *slot = true;
        }
    }
    seen.iter().position(|&v| !v)
}

impl Document {
    /// 轨道结构变化（插入/删除/移动）后，把按轨道索引的编辑态从旧索引
    /// 重映射到新索引；`insert_idx` 非空时在新位置插入一条默认槽位。
    ///
    /// 选区一律清空：`Selection` 的矩形是 `track_lo..=track_hi` 连续区间，
    /// 结构变化后无法可靠拆分/重映射，保留旧索引会指向错误轨道。
    /// `track_selected` 由调用方按操作结果重新设置。
    pub(crate) fn remap_edit_for_track_change(&mut self, remap: &[u16], insert_idx: Option<usize>) {
        // 结构变化刚发生时，按轨数组可能还没补齐到旧轨数（新建空文档等）；
        // 先按旧长度补默认值，remap_indexed 才能一一对应。
        let old_len = remap.len();
        while self.edit.track_visible.len() < old_len {
            self.edit.track_visible.push(true);
        }
        while self.edit.track_pianoroll_visible.len() < old_len {
            self.edit.track_pianoroll_visible.push(true);
        }
        while self.edit.track_locked.len() < old_len {
            self.edit.track_locked.push(false);
        }
        while self.edit.track_overrides.len() < old_len {
            self.edit.track_overrides.push(Default::default());
        }
        while self.edit.layer_solo_prev.len() < old_len {
            self.edit.layer_solo_prev.push(false);
        }

        remap_indexed(&mut self.edit.track_visible, remap);
        remap_indexed(&mut self.edit.track_pianoroll_visible, remap);
        remap_indexed(&mut self.edit.track_locked, remap);
        remap_indexed(&mut self.edit.track_overrides, remap);
        remap_indexed(&mut self.edit.layer_solo_prev, remap);

        if let Some(i) = insert_idx {
            let i = i.min(self.edit.track_visible.len());
            self.edit.track_visible.insert(i, true);
            self.edit.track_pianoroll_visible.insert(i, true);
            self.edit.track_locked.insert(i, false);
            self.edit.track_overrides.insert(i, Default::default());
            self.edit.layer_solo_prev.insert(i, false);
        }

        if let Some(t) = self.edit.editing_track {
            self.edit.editing_track = remap.get(t as usize).copied().filter(|&v| v != u16::MAX);
        }
        // AR 自动化展开状态按音轨索引保存：插入/删除文件夹后必须同步重映射，
        // 否则 lane 会整体挂到相邻轨，表现为自动化行错位一整条音轨。
        if self.edit.arr_am_expanded.len() < old_len {
            self.edit.arr_am_expanded.resize(old_len, false);
        }
        if self.edit.arr_am_expanded.len() == old_len {
            let old_expanded = std::mem::take(&mut self.edit.arr_am_expanded);
            let new_len = remap
                .iter()
                .copied()
                .filter(|&i| i != u16::MAX)
                .max()
                .map(|m| m as usize + 1)
                .unwrap_or(old_len);
            let mut expanded = vec![false; new_len];
            for (old, &new) in remap.iter().enumerate() {
                if new != u16::MAX
                    && let Some(slot) = expanded.get_mut(new as usize)
                {
                    *slot = old_expanded.get(old).copied().unwrap_or(false);
                }
            }
            self.edit.arr_am_expanded = expanded;
        }
        self.edit
            .remap_am_track_keys(|t| remap.get(t as usize).copied().filter(|&v| v != u16::MAX));
        self.edit.layer_solo = self
            .edit
            .layer_solo
            .iter()
            .filter_map(|&t| remap.get(t as usize).copied().filter(|&v| v != u16::MAX))
            .collect();

        self.edit.selected.clear_rects();
        self.edit.sel_rect.clear();
        self.edit.arr_sel_rect.clear();
        self.edit.chan_ar_sel.clear();
        self.edit.selected_audio_clips.clear();
    }

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

        // Find a free channel on port 0（Master/Conductor 轨不占 MIDI 通道）
        let used_channels: std::collections::HashSet<u8> = model
            .tracks
            .iter()
            .filter(|t| t.port == 0 && t.kind == yinhe_core::TrackKind::Midi)
            .map(|t| t.channel)
            .collect();
        let channel = (0..16u8).find(|c| !used_channels.contains(c)).unwrap_or(0);

        let mut new_track = yinhe_core::TrackData::new(0, channel);
        // 新轨跟随插入位置的层级：插在子轨后仍是子轨，插在文件夹后是同级。
        new_track.folder_depth = model
            .tracks
            .get(after_idx)
            .map(|t| t.folder_depth)
            .unwrap_or(0);
        // 命名规则说明见 max_track_number。
        new_track.name = format!("Track {}", max_track_number(model) + 1);

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.insert(insert_idx, Arc::new(new_track));
        model.sync_automation_lane_tracks();

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

        if insert_idx == tracks_before.len() {
            // 末尾追加：remap 恒等（apply_track_remap 自动跳过），只补统计槽位。
            model.push_empty_track_stats(1);
        } else {
            // 中间插入：音符轨号平移（remap 保序），统计按 remap 搬移。
            model.apply_track_remap(&note_remap);
            model.remap_track_stats(&note_remap);
        }
        self.data.bump_revision();

        // Update edit state
        self.remap_edit_for_track_change(&note_remap, Some(insert_idx));
        self.sync_track_caches();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(insert_idx as u16);
        self.default_own_lc_for_new_midi_tracks(insert_idx);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
            deleted_notes: Vec::new(),
        })
    }

    /// 把一段连续选中轨归入新建文件夹。选中区间必须完整覆盖其中的文件夹子树。
    pub fn group_tracks(&mut self, indices: &[usize]) -> Option<UndoAction> {
        if indices.is_empty() {
            return None;
        }
        let model = &self.data.model;
        let num_tracks = model.tracks.len();
        if indices.iter().any(|&i| i >= num_tracks) {
            return None;
        }
        if indices
            .iter()
            .any(|&i| model.tracks[i].kind == yinhe_core::TrackKind::Master)
        {
            return None;
        }
        // 选中的 Folder 自动带上整个子树。
        let mut selected = vec![false; num_tracks];
        for &i in indices {
            let end = model.subtree_end(i);
            for flag in &mut selected[i..end] {
                *flag = true;
            }
        }
        let first = selected.iter().position(|&b| b)?;
        let last = selected.iter().rposition(|&b| b)?;
        if !selected[first..=last].iter().all(|&b| b) {
            return None;
        }
        let base_depth = model.tracks[first].folder_depth;
        let folder_color = model.tracks[first].color;

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        let model = Arc::make_mut(&mut self.data.model);
        let mut folder = yinhe_core::TrackData::new(0, 0);
        folder.name = "Folder".to_string();
        folder.kind = yinhe_core::TrackKind::Folder;
        folder.folder_depth = base_depth;
        folder.color = folder_color;
        model.tracks.insert(first, Arc::new(folder));

        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| if i < first { i as u16 } else { (i + 1) as u16 })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| {
                if i == first {
                    u16::MAX
                } else if i < first {
                    i as u16
                } else {
                    (i - 1) as u16
                }
            })
            .collect();

        if first == tracks_before.len() {
            model.push_empty_track_stats(1);
        } else {
            model.apply_track_remap(&note_remap);
            model.remap_track_stats(&note_remap);
        }
        // 原选中区间整体成为新文件夹的子轨，深度 +1。
        for track in &mut model.tracks[first + 1..=last + 1] {
            let track = Arc::make_mut(track);
            track.folder_depth = track.folder_depth.saturating_add(1);
        }
        model.normalize_folder_structure();
        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        self.data.bump_revision();
        self.remap_edit_for_track_change(&note_remap, Some(first));
        self.sync_track_caches();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(first as u16);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
            deleted_notes: Vec::new(),
        })
    }

    /// 解散文件夹：删除 Folder 行，子轨整体上移一层。
    pub fn ungroup_track(&mut self, idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        if model.tracks.get(idx)?.kind != yinhe_core::TrackKind::Folder {
            return None;
        }
        let end = model.subtree_end(idx);
        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.remove(idx);
        for track in &mut model.tracks[idx..end.saturating_sub(1)] {
            let track = Arc::make_mut(track);
            track.folder_depth = track.folder_depth.saturating_sub(1);
        }
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| {
                if i < idx {
                    i as u16
                } else if i == idx {
                    u16::MAX
                } else {
                    (i - 1) as u16
                }
            })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| if i < idx { i as u16 } else { (i + 1) as u16 })
            .collect();
        let deleted_notes = model.apply_track_remap_collect(&note_remap);
        model.normalize_folder_structure();
        model.rebuild_stats_only();
        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        self.data.bump_revision();
        self.remap_edit_for_track_change(&note_remap, None);
        self.sync_track_caches();

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
            deleted_notes,
        })
    }

    /// 按新的轨顺序重排（order[new] = old）；只用于归组前的块移动。
    fn reorder_tracks_to(&mut self, order: &[usize]) -> Option<UndoAction> {
        let model = &self.data.model;
        let n = model.tracks.len();
        if order.len() != n {
            return None;
        }
        let mut seen = vec![false; n];
        for &old in order {
            if old >= n || seen[old] {
                return None;
            }
            seen[old] = true;
        }
        if order.iter().copied().eq(0..n) {
            return None;
        }
        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        let mut old_to_new = vec![0u16; n];
        for (new, &old) in order.iter().enumerate() {
            old_to_new[old] = new as u16;
        }
        let new_to_old: Vec<u16> = order.iter().map(|&old| old as u16).collect();
        let model = Arc::make_mut(&mut self.data.model);
        model.tracks = order
            .iter()
            .map(|&old| Arc::clone(&tracks_before[old]))
            .collect();
        model.apply_track_remap(&old_to_new);
        model.remap_track_stats(&old_to_new);
        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        self.data.bump_revision();
        self.remap_edit_for_track_change(&old_to_new, None);
        self.sync_track_caches();
        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap: old_to_new,
            note_remap_inverse: new_to_old,
            deleted_notes: Vec::new(),
        })
    }

    /// Ctrl/Cmd+G：把选中轨归组。允许选中轨之间有未选中轨：先把选中块
    /// 按原相对顺序移到最前选中位置，再连续归组。仅支持根层块，避免跨父级
    /// 移动破坏层级。
    pub fn group_tracks_reordered(&mut self, indices: &[usize]) -> Option<UndoAction> {
        if indices.is_empty() {
            return None;
        }
        let model = &self.data.model;
        let n = model.tracks.len();
        if indices.iter().any(|&i| i >= n) {
            return None;
        }
        if indices
            .iter()
            .any(|&i| model.tracks[i].kind == yinhe_core::TrackKind::Master)
        {
            return None;
        }
        let mut selected = vec![false; n];
        for &i in indices {
            let end = model.subtree_end(i);
            for flag in &mut selected[i..end] {
                *flag = true;
            }
        }
        // 根层块才做重排；含子轨的选区交给连续归组，失败则拒绝。
        if (0..n).any(|i| selected[i] && model.tracks[i].folder_depth > 0) {
            return self.group_tracks(indices);
        }
        let first = selected.iter().position(|&b| b)?;
        let last = selected.iter().rposition(|&b| b)?;
        if selected[first..=last].iter().all(|&b| b) {
            return self.group_tracks(indices);
        }

        let selected_indices: Vec<usize> = (0..n).filter(|&i| selected[i]).collect();
        let mut order: Vec<usize> = Vec::with_capacity(n);
        order.extend(0..first);
        order.extend(selected_indices.iter().copied());
        order.extend((first..=last).filter(|&i| !selected[i]));
        order.extend(last + 1..n);

        let reorder = self.reorder_tracks_to(&order);
        let group = self.group_tracks(&(first..first + selected_indices.len()).collect::<Vec<_>>());
        match (reorder, group) {
            (Some(a), Some(b)) => Some(UndoAction::Composite(vec![a, b])),
            (None, Some(b)) => Some(b),
            (Some(a), None) => {
                // 归组失败：回滚重排，避免只移动不归组。
                a.reversed().redo(self);
                None
            }
            (None, None) => None,
        }
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
        let append_depth = self
            .data
            .model
            .tracks
            .last()
            .map(|t| t.folder_depth)
            .unwrap_or(0);

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = self.data.model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        for (num, spec) in (next_num..).zip(specs.iter()) {
            let mut new_track = yinhe_core::TrackData::new(spec.port, spec.channel);
            new_track.kind = spec.kind;
            new_track.folder_depth = append_depth;
            new_track.audio_channel = spec.audio_channel;
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

        // 末尾追加：既有音符轨号不变（恒等 remap），新轨无音符 → 只补统计槽位。
        model.push_empty_track_stats(specs.len());
        self.data.bump_revision();

        // Update edit state：选中本批新建的全部音轨。
        // 追加在末尾，editing_track 无需调整；旧选区仍指向有效轨，但为了与
        // 新增轨的选择保持一致，一并清空。
        self.sync_track_caches();
        self.edit.selected.clear_rects();
        self.edit.sel_rect.clear();
        self.edit.arr_sel_rect.clear();
        self.edit.chan_ar_sel.clear();
        self.edit.selected_audio_clips.clear();
        self.edit.track_selected.clear();
        for i in insert_idx..total_tracks {
            self.edit.track_selected.insert(i as u16);
        }
        self.default_own_lc_for_new_midi_tracks(insert_idx);

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
        if self.edit.track_cache.conductor_idx == Some(idx as u16) {
            return None;
        }
        // 文件夹请走 ungroup_track，直接删除会让子轨层级断裂。
        if model.tracks[idx].kind == yinhe_core::TrackKind::Folder {
            return None;
        }
        // Don't remove if only 2 tracks (conductor + 1)
        if model.tracks.len() <= 2 {
            return None;
        }

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();
        let removed_lc = model.tracks[idx].lc;

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.remove(idx);
        model.sync_automation_lane_tracks();

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

        // 一趟完成删除 + 重编号 + 收集被删音符（供 undo）；remap 保序，
        // 统计重建无需全量 sort。
        let deleted_notes: Vec<(yinhe_types::Note, u8)> =
            model.apply_track_remap_collect(&note_remap);
        model.rebuild_stats_only();
        let num_tracks = model.tracks.len();
        self.data.bump_revision();

        // Update edit state
        if let Some(lc) = removed_lc {
            self.mixer.remove_track_lc(lc);
        }
        self.remap_edit_for_track_change(&note_remap, None);
        self.sync_track_caches();
        self.edit.track_selected.clear();
        // Select the track that took its place (or last track)
        let new_sel = idx.min(num_tracks - 1) as u16;
        self.edit.track_selected.insert(new_sel);
        // 删除 track 后，editing_track 已由 remap_edit_for_track_change 处理。
        // AM 试听键已按 remap 重映射：被删轨丢弃，其后轨索引 -1。

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
        // v1 文件夹：子树拖拽尚未实现，存在 Folder 时禁止普通换位，避免层级断裂。
        if model
            .tracks
            .iter()
            .any(|t| t.kind == yinhe_core::TrackKind::Folder)
        {
            return None;
        }
        // Don't move conductor track
        if self.edit.track_cache.conductor_idx == Some(from_idx as u16)
            || self.edit.track_cache.conductor_idx == Some(to_idx as u16)
        {
            return None;
        }

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.remove(from_idx);
        model.tracks.insert(to_idx, track);
        model.sync_automation_lane_tracks();

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

        model.apply_track_remap(&note_remap);
        // 纯重排：音符 tick 分布不变，统计按 remap 搬移（零音符遍历）。
        model.remap_track_stats(&note_remap);
        self.data.bump_revision();

        // Update edit state
        self.remap_edit_for_track_change(&note_remap, None);
        self.sync_track_caches();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(to_idx as u16);

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
#[allow(clippy::items_after_test_module)] // 历史文件：测试模块后还有 impl 块
mod tests {
    use super::{Document, NewTrackSpec};

    /// 删除音轨后，自动化 lane 的 track 字段必须跟着新下标走，
    /// 否则 AR 自动化曲线会继续用旧下标取色（删除 001 后 002 应继承 001 的颜色）。
    #[test]
    fn remove_track_remaps_automation_lane_track() {
        let mut doc = Document::empty();
        let target = yinhe_types::automation::AutomationTarget::CC { controller: 7 };
        assert!(doc.add_automation_lane(2, target).is_some());
        assert!(doc.remove_track(1).is_some());
        assert_eq!(doc.data.model.tracks[1].automation_lanes[0].track, 1);
    }

    #[test]
    fn group_and_ungroup_folder_restores_track_layout() {
        let mut doc = Document::empty(); // Master + Track 1..16
        let before = doc.data.model.tracks.len();
        let action = doc.group_tracks(&[1, 2]).expect("group succeeds");
        assert!(matches!(
            action,
            crate::history::UndoAction::TrackStructure { .. }
        ));
        assert_eq!(doc.data.model.tracks.len(), before + 1);
        assert_eq!(doc.data.model.tracks[1].kind, yinhe_core::TrackKind::Folder);
        assert_eq!(doc.data.model.tracks[2].folder_depth, 1);
        assert_eq!(doc.data.model.tracks[3].folder_depth, 1);

        assert!(doc.ungroup_track(1).is_some());
        assert_eq!(doc.data.model.tracks.len(), before);
        assert_eq!(doc.data.model.tracks[1].folder_depth, 0);
        assert_eq!(doc.data.model.tracks[2].folder_depth, 0);
    }

    #[test]
    fn group_folder_undo_restores_layout() {
        let mut doc = Document::empty();
        let before = doc.data.model.tracks.len();
        let action = doc.group_tracks(&[1, 2]).expect("group succeeds");
        assert_eq!(doc.data.model.tracks.len(), before + 1);
        action.reversed().redo(&mut doc);
        assert_eq!(doc.data.model.tracks.len(), before);
        assert_eq!(doc.data.model.tracks[1].kind, yinhe_core::TrackKind::Midi);
        assert_eq!(doc.data.model.tracks[1].folder_depth, 0);
    }

    #[test]
    fn collapsed_folder_hides_children_only() {
        let mut doc = Document::empty();
        doc.group_tracks(&[1, 2]).expect("group succeeds");
        assert_eq!(doc.data.model.tracks[1].kind, yinhe_core::TrackKind::Folder);
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        std::sync::Arc::make_mut(&mut model.tracks[1]).collapsed = true;
        let visible = doc.data.model.folder_visible_mask();
        assert!(visible[0], "Master 始终可见");
        assert!(visible[1], "文件夹行本身可见");
        assert!(!visible[2] && !visible[3], "子轨隐藏");
        assert!(visible[4], "文件夹之后的根轨可见");
    }

    #[test]
    fn group_with_gap_moves_selected_blocks_together() {
        let mut doc = Document::empty();
        let before_names: Vec<String> = doc
            .data
            .model
            .tracks
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let action = doc
            .group_tracks_reordered(&[1, 3])
            .expect("non-contiguous group succeeds");
        assert_eq!(doc.data.model.tracks[1].kind, yinhe_core::TrackKind::Folder);
        assert_eq!(doc.data.model.tracks[2].name, before_names[1]);
        assert_eq!(doc.data.model.tracks[3].name, before_names[3]);
        assert_eq!(
            doc.data.model.tracks[4].name, before_names[2],
            "夹在中间的轨移到组后"
        );
        action.reversed().redo(&mut doc);
        let after_names: Vec<String> = doc
            .data
            .model
            .tracks
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(after_names, before_names, "undo 恢复原顺序");
    }

    #[test]
    fn grouping_remaps_automation_expansion_state() {
        let mut doc = Document::empty();
        let n = doc.data.model.tracks.len();
        doc.edit.arr_am_expanded = vec![false; n];
        doc.edit.arr_am_expanded[2] = true;
        let action = doc.group_tracks(&[1, 2]).expect("group succeeds");
        assert_eq!(doc.data.model.tracks[1].kind, yinhe_core::TrackKind::Folder);
        assert_eq!(
            doc.edit.arr_am_expanded.len(),
            n + 1,
            "展开状态数组随插入延长"
        );
        assert!(doc.edit.arr_am_expanded[3], "原轨 2 的展开状态跟到新索引 3");
        assert!(!doc.edit.arr_am_expanded[1], "文件夹默认收起");
        action.reversed().redo(&mut doc);
        assert_eq!(doc.edit.arr_am_expanded.len(), n);
        assert!(doc.edit.arr_am_expanded[2], "undo 后恢复原索引");
    }

    #[test]
    fn non_contiguous_group_is_rejected() {
        let mut doc = Document::empty();
        assert!(doc.group_tracks(&[1, 3]).is_none());
    }

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

    /// 批量创建：追加到末尾、按 spec 设置 kind/通道、命名从最大编号 + 1 递增，
    /// 新建的整批被选中。
    #[test]
    fn add_tracks_batch_appends_with_specs() {
        let mut doc = Document::empty(); // Conductor + Track 1..16（17 条）
        let specs = vec![
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Midi,
                port: 1,
                channel: 0,
                audio_channel: None,
            },
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Midi,
                port: 1,
                channel: 1,
                audio_channel: None,
            },
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Audio,
                port: 0,
                channel: 0,
                audio_channel: Some(0),
            },
        ];
        assert!(doc.add_tracks_batch(&specs).is_some());

        let model = doc.model();
        assert_eq!(model.tracks.len(), 20);
        assert_eq!(model.tracks[17].name, "Track 17");
        assert_eq!(model.tracks[18].name, "Track 18");
        assert_eq!(model.tracks[19].name, "Track 19");
        assert_eq!((model.tracks[17].port, model.tracks[17].channel), (1, 0));
        assert_eq!(model.tracks[19].kind, yinhe_core::TrackKind::Audio);
        assert_eq!(model.tracks[19].audio_channel, Some(0));
        // 新建的 3 条全部被选中
        assert_eq!(doc.edit.track_selected.len(), 3);
        assert!(doc.edit.track_selected.contains(&19));
    }

    /// C 默认：新建 MIDI 轨落在空 MIDI 通道时，自动把该通道切到 PerTrack，
    /// 并给新轨分配自己的 LC。
    #[test]
    fn new_midi_track_on_empty_channel_gets_own_lc() {
        let mut doc = Document::empty();
        let specs = vec![NewTrackSpec {
            kind: yinhe_core::TrackKind::Midi,
            port: 1,
            channel: 0,
            audio_channel: None,
        }];
        assert!(doc.add_tracks_batch(&specs).is_some());
        let track = doc.model().tracks.last().expect("new track");
        assert_eq!(track.global_channel(), 16);
        assert!(track.lc.is_some(), "新通道的 MIDI 轨应默认 OwnLc");
        assert_eq!(
            doc.rack_output_mode(16),
            yinhe_types::RackOutputMode::PerTrack
        );
        assert!(doc.mixer.ap_for_midi(16).is_none());
    }

    /// 已有 AP 的通道不因新轨自动拆分：新轨加入 AP，保持共享关系。
    #[test]
    fn new_midi_track_on_ap_channel_joins_ap() {
        let mut doc = Document::empty();
        assert!(doc.set_rack_output_mode(0, yinhe_types::RackOutputMode::Shared));
        let specs = vec![NewTrackSpec {
            kind: yinhe_core::TrackKind::Midi,
            port: 0,
            channel: 0,
            audio_channel: None,
        }];
        assert!(doc.add_tracks_batch(&specs).is_some());
        let track = doc.model().tracks.last().expect("new track");
        assert_eq!(track.global_channel(), 0);
        assert!(track.lc.is_none(), "已有 AP 的通道应保持共享");
        assert!(doc.mixer.ap_for_midi(0).is_some());
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
                audio_channel: None,
            },
            NewTrackSpec {
                kind: yinhe_core::TrackKind::Midi,
                port: 1,
                channel: 0,
                audio_channel: None,
            },
        ];
        let action = doc.add_tracks_batch(&specs).expect("批量创建应成功");
        assert_eq!(doc.model().tracks.len(), 19);

        action.reversed().redo(&mut doc);
        assert_eq!(doc.model().tracks.len(), 17);
        assert_eq!(doc.model().tracks[16].name, "Track 16");
    }

    /// 轨道结构变更（追加/删除）后统计保持一致，undo 回放后同样正确。
    #[test]
    fn track_structure_ops_keep_stats_consistent() {
        let mut doc = Document::empty();
        doc.add_note(
            1,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 100,
                end_tick: 200,
                key: 60,
                velocity: 100,
            },
        );
        doc.add_note(
            2,
            yinhe_core::NoteEvent {
                id: 0,
                start_tick: 300,
                end_tick: 400,
                key: 61,
                velocity: 100,
            },
        );
        {
            let m = doc.model();
            assert_eq!(m.track_note_count[1], 1);
            assert_eq!(m.track_note_count[2], 1);
            assert_eq!(m.note_count, 2);
            assert_eq!(m.tick_length, 400);
        }

        // 末尾追加：统计只补 0 槽位（新轨无音符）。
        doc.add_tracks_batch(&[NewTrackSpec {
            kind: yinhe_core::TrackKind::Midi,
            port: 0,
            channel: 0,
            audio_channel: None,
        }])
        .expect("批量创建应成功");
        {
            let m = doc.model();
            assert_eq!(m.track_note_count.len(), m.tracks.len());
            assert_eq!(m.track_note_count[17], 0, "新轨统计为 0");
            assert_eq!(m.note_count, 2);
            assert_eq!(m.tick_length, 400);
        }

        // 删除 idx 1（含音符的轨）：note_count 减少、统计长度 -1。
        let action = doc.remove_track(1).expect("删除应产生 undo");
        {
            let m = doc.model();
            assert_eq!(m.track_note_count.len(), m.tracks.len());
            assert_eq!(m.note_count, 1, "被删轨音符应从 note_count 移除");
            assert_eq!(m.tick_length, 400, "原 Track 2 的音符仍是最大 end");
            assert_eq!(m.track_note_count[1], 1, "原 Track 2 现在 idx 1");
        }

        // undo 回放（TrackStructure 路径）后统计恢复。
        action.reversed().redo(&mut doc);
        {
            let m = doc.model();
            assert_eq!(m.note_count, 2, "undo 后音符与统计应恢复");
            assert_eq!(m.track_note_count[1], 1);
            assert_eq!(m.track_note_count[2], 1);
            assert_eq!(m.tick_length, 400);
        }
    }

    /// 空批次是 no-op，不产生 undo。
    #[test]
    fn add_tracks_batch_empty_is_noop() {
        let mut doc = Document::empty();
        assert!(doc.add_tracks_batch(&[]).is_none());
        assert_eq!(doc.model().tracks.len(), 17);
    }

    /// 重排音轨时，按 track 索引的编辑态数组（可见/锁定等）必须跟着音轨走。
    #[test]
    fn move_track_remaps_per_track_edit_state() {
        let mut doc = Document::empty();
        let n = doc.data.model.tracks.len();
        doc.edit.track_pianoroll_visible = vec![true; n];
        doc.edit.track_locked = vec![false; n];
        doc.edit.track_visible = vec![true; n];
        doc.edit.track_pianoroll_visible[2] = false;
        doc.edit.track_locked[3] = true;
        let name_at_2 = doc.data.model.tracks[2].name.clone();
        let name_at_3 = doc.data.model.tracks[3].name.clone();

        // 把 index 2 移到 index 5。
        assert!(doc.move_track(2, 5).is_some());

        // 原 index 2 的音轨现在在 5：可见性跟随。
        assert_eq!(doc.data.model.tracks[5].name, name_at_2);
        assert!(!doc.edit.track_pianoroll_visible[5], "可见性未跟随音轨");
        // 原 index 3 的音轨左移到 2：锁定跟随。
        assert_eq!(doc.data.model.tracks[2].name, name_at_3);
        assert!(doc.edit.track_locked[2], "锁定未跟随音轨");
    }
}

impl Document {
    /// 编辑音轨信息：名称 / 端口 / 通道 + 通道级默认乐器（None = 清除）。
    ///
    /// - Master（Conductor）轨只允许改名（同步到歌曲标题）；端口/通道/乐器忽略。
    /// - 乐器是通道级设置：同一通道的所有音轨共享，MIDI 内 PC 优先。
    /// - 无实际变化时返回 None（不产生 undo）。
    pub fn set_track_info(
        &mut self,
        track_idx: usize,
        name: String,
        port: u8,
        channel: u8,
        instrument: Option<yinhe_core::ChannelInstrument>,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get(track_idx)?;
        let is_master = track.kind == yinhe_core::TrackKind::Master || track_idx == 0;
        let old_name = track.name.clone();
        let old_port = track.port;
        let old_channel = track.channel;
        let (new_name, new_port, new_channel) = if is_master {
            (name, old_port, old_channel)
        } else {
            (name, port.min(15), channel.min(15))
        };
        let instrument_channel = (!is_master).then_some(new_channel);
        let instrument_before = instrument_channel.and_then(|ch| {
            model
                .channel_instruments
                .get(ch as usize)
                .copied()
                .flatten()
        });
        let instrument_after = if is_master {
            instrument_before
        } else {
            instrument
        };
        if old_name == new_name
            && old_port == new_port
            && old_channel == new_channel
            && instrument_before == instrument_after
        {
            return None;
        }
        {
            let track = Arc::make_mut(model.tracks.get_mut(track_idx)?);
            track.name = new_name.clone();
            if !is_master {
                track.port = new_port;
                track.channel = new_channel;
            }
        }
        if let Some(ch) = instrument_channel {
            model.channel_instruments[ch as usize] = instrument_after;
        }
        if is_master {
            model.meta.name = new_name.clone();
        }
        if let Some(info) = self.edit.track_cache.info.get_mut(track_idx) {
            info.name = new_name.clone();
            info.port = new_port;
            info.channel = new_channel;
        }
        self.data.bump_revision();
        Some(UndoAction::TrackInfo {
            track_idx,
            old_name,
            new_name,
            old_port,
            new_port,
            old_channel,
            new_channel,
            instrument_channel,
            instrument_before,
            instrument_after,
        })
    }

    /// 只改通道级默认乐器（MIX 通道属性面板；None = 跟随 MIDI）。
    /// 无实际变化时返回 None。
    pub fn set_channel_instrument(
        &mut self,
        channel: u8,
        instrument: Option<yinhe_core::ChannelInstrument>,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let slot = model.channel_instruments.get_mut(channel as usize)?;
        let before = *slot;
        if before == instrument {
            return None;
        }
        *slot = instrument;
        self.data.bump_revision();
        Some(UndoAction::ChannelInstrument {
            channel,
            before,
            after: instrument,
        })
    }
}

#[cfg(test)]
mod edit_state_remap_tests {
    use super::*;

    #[test]
    fn insert_track_remaps_visibility_and_lock() {
        let mut doc = Document::empty();
        let n = doc.data.model.tracks.len();
        doc.edit.track_locked = (0..n).map(|i| i == 3).collect();
        doc.edit.track_visible = (0..n).map(|i| i != 4).collect();

        let insert_at = 3; // 插在锁定轨之前
        assert!(doc.add_track(2).is_some());

        assert!(
            doc.edit.track_locked[insert_at + 1],
            "原锁定轨后移一位仍锁定"
        );
        assert_eq!(
            doc.edit.track_locked.iter().filter(|&&b| b).count(),
            1,
            "新轨不应继承锁定"
        );
        assert!(
            !doc.edit.track_visible[insert_at + 2],
            "原不可见轨后移仍不可见"
        );
        assert!(doc.edit.track_visible[insert_at], "新轨默认可见");
    }

    #[test]
    fn remove_track_remaps_visibility_and_lock() {
        let mut doc = Document::empty();
        let n = doc.data.model.tracks.len();
        doc.edit.track_locked = (0..n).map(|i| i == 4).collect();

        assert!(doc.remove_track(3).is_some());

        assert!(doc.edit.track_locked[3], "原索引 4 的锁定轨前移一位");
        assert_eq!(doc.edit.track_locked.len(), n - 1);
    }
}
