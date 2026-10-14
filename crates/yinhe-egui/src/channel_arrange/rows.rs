//! 通道 AR 行布局。
//!
//! 行序：`Conductor`（若存在）→ 各 MIDI 通道 → 各音频通道。
//! 每条通道一行合并行；展开后其下方平铺该通道的
//! 「轨道音符行 + 每条轨道的自动化子行」。
//!
//! 与轨道 AR（`ArRowLayout`）的关键差异：**行高不统一**——展开的通道合并行
//! 变窄为一小条（总览），其余行均为全行高。因此行号 ↔ y 换算走每行
//! y 顶部的前缀和 + 二分，而不是均匀乘法。

use std::collections::HashMap;
use std::ops::Range;

use super::groups::{CaChannel, CaGroups};

/// 通道 AR 的一行。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CaRow {
    /// Conductor（Tempo 直显，不展开）。
    Conductor,
    /// 通道合并行（折叠时全高；展开时变窄作总览）。
    Channel(CaChannel),
    /// 轨道音符行。
    Track(u16),
    /// 轨道自动化子行（sub = automation_lanes 下标）。
    Automation { track: u16, sub: usize },
}

impl CaRow {
    /// 所属轨道（通道合并行 / Conductor 返回 None）。
    pub fn track(self) -> Option<u16> {
        match self {
            CaRow::Track(t) | CaRow::Automation { track: t, .. } => Some(t),
            CaRow::Channel(_) | CaRow::Conductor => None,
        }
    }
}

/// 通道 AR 行布局（每帧重建；所有行等高 = lane_height）。
#[derive(Clone, Debug, Default)]
pub(crate) struct CaRowLayout {
    rows: Vec<CaRow>,
    heights: Vec<f32>,
    /// 每行顶部 y（未减 scroll_y），升序前缀和。
    y_tops: Vec<f32>,
    total_height: f32,
}

impl CaRowLayout {
    /// 构建行布局。
    ///
    /// - `conductor`：是否存在 Conductor 行（有 Conductor 轨时为 true）。
    /// - `expanded`：通道展开判定。
    /// - `lane_counts`：轨道 → 自动化 lane 数。
    /// - `lane_height`：全行高（px）。
    pub fn build(
        conductor: bool,
        groups: &CaGroups,
        expanded: impl Fn(CaChannel) -> bool,
        lane_counts: impl Fn(u16) -> usize,
        lane_height: f32,
    ) -> Self {
        let full = lane_height.max(1.0);

        let mut rows = Vec::new();
        let mut heights = Vec::new();
        let mut push = |row: CaRow, h: f32| {
            rows.push(row);
            heights.push(h);
        };
        if conductor {
            push(CaRow::Conductor, full);
        }
        for group in groups.iter_all() {
            let exp = expanded(group.channel);
            push(CaRow::Channel(group.channel), full);
            if exp {
                // 两段式：先全部音符轨道行，再全部自动化行
                // （同通道的轨道集中在上、自动化集中在下，便于对比与框选）。
                for &t in &group.tracks {
                    push(CaRow::Track(t), full);
                }
                for &t in &group.tracks {
                    for sub in 0..lane_counts(t) {
                        push(CaRow::Automation { track: t, sub }, full);
                    }
                }
            }
        }

        let mut y_tops = Vec::with_capacity(rows.len());
        let mut y = 0.0f32;
        for &h in &heights {
            y_tops.push(y);
            y += h;
        }
        Self {
            rows,
            heights,
            y_tops,
            total_height: y,
        }
    }

    #[inline]
    pub fn total_rows(&self) -> usize {
        self.rows.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[inline]
    pub fn total_height(&self) -> f32 {
        self.total_height
    }

    #[inline]
    pub fn row(&self, i: usize) -> Option<CaRow> {
        self.rows.get(i).copied()
    }

    /// 行 i 的顶部 y（未减 scroll_y）。
    #[inline]
    pub fn y_top(&self, i: usize) -> f32 {
        self.y_tops.get(i).copied().unwrap_or(0.0)
    }

    /// 行 i 的高度。
    #[inline]
    pub fn height(&self, i: usize) -> f32 {
        self.heights.get(i).copied().unwrap_or(0.0)
    }

    /// 行命中：超出内容底部返回 None（与 ArRowLayout 的越界语义一致）。
    pub fn hit_row(&self, music_y: f32) -> Option<usize> {
        if self.rows.is_empty() || music_y < 0.0 || music_y >= self.total_height {
            return None;
        }
        let i = self.y_tops.partition_point(|&t| t <= music_y);
        if i == 0 { None } else { Some(i - 1) }
    }

    /// 可视行范围 [first, last)（含部分可见；视口为 [scroll_y, scroll_y+viewport_h)）。
    pub fn visible_rows(&self, scroll_y: f32, viewport_h: f32) -> Range<usize> {
        if self.rows.is_empty() {
            return 0..0;
        }
        let scroll_y = scroll_y.max(0.0);
        // 第一行：最后一个 y_top <= scroll_y 的行（其底部跨过视口上缘）。
        let first = self
            .y_tops
            .partition_point(|&t| t <= scroll_y)
            .saturating_sub(1);
        // 末行（开区间）：y_top < 视口底部的行数。
        let bottom = scroll_y + viewport_h.max(0.0);
        let last = self.y_tops.partition_point(|&t| t < bottom);
        first..last.max(first)
    }

    /// 每行顶部 y 表（GPU `track_offsets` 语义：instance.track = 行 id）。
    pub fn row_offsets(&self) -> &[f32] {
        &self.y_tops
    }

    /// 每行高度表（GPU `row_heights`）。
    pub fn row_heights(&self) -> &[f32] {
        &self.heights
    }

    /// 轨道音符行 → 行号映射（同一轨道至多一个音符行）。
    pub fn track_row_map(&self) -> HashMap<u16, usize> {
        let mut map = HashMap::new();
        for (i, row) in self.rows.iter().enumerate() {
            if let CaRow::Track(t) = row {
                map.entry(*t).or_insert(i);
            }
        }
        map
    }

    /// 单个轨道的音符行号（O(rows) 扫描；交互高频路径行数很小，够用）。
    pub fn track_row_of(&self, track: u16) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| matches!(r, CaRow::Track(t) if *t == track))
    }

    /// 轨道集合覆盖的行范围 (min_row, max_row+1)：用于选框绘制。
    ///
    /// 相关行 = 集合内轨道的「音符行」与「自动化行」；两段式布局下
    /// 自动化行集中在音符行之后，取最小/最大相关行即包围盒
    /// （框内可能包含未选中轨的行，属矩形选框的正常语义——选中内容
    /// 仍按轨道集合精确）。
    pub fn rows_covering_tracks(&self, tracks: &[u16]) -> Option<(usize, usize)> {
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        for (i, row) in self.rows.iter().enumerate() {
            let hit = match row {
                CaRow::Track(t) | CaRow::Automation { track: t, .. } => tracks.contains(t),
                _ => false,
            };
            if hit {
                lo = lo.min(i);
                hi = hi.max(i);
            }
        }
        (lo != usize::MAX).then_some((lo, hi + 1))
    }
}

/// 行 → 该行代表的轨道集合（用于框选/橡皮擦的选区归纳）。
///
/// - 通道合并行 → 该通道全部轨道；
/// - 轨道行 / 自动化行 → 该轨道；
/// - Conductor → 空（Tempo 行不产生音符选区）。
pub(crate) fn row_tracks(row: CaRow, groups: &CaGroups) -> Vec<u16> {
    match row {
        CaRow::Channel(ch) => groups
            .group(ch)
            .map(|g| g.tracks.clone())
            .unwrap_or_default(),
        CaRow::Track(t) => vec![t],
        CaRow::Automation { track, .. } => vec![track],
        CaRow::Conductor => Vec::new(),
    }
}

/// 行区间 → 去重升序的轨道集合（跳过 Conductor 行）。
pub(crate) fn rows_range_tracks(
    layout: &CaRowLayout,
    rows: Range<usize>,
    groups: &CaGroups,
) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for i in rows {
        for t in row_tracks_checked(layout, i, groups) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out.sort_unstable();
    out
}

fn row_tracks_checked(layout: &CaRowLayout, i: usize, groups: &CaGroups) -> Vec<u16> {
    layout
        .row(i)
        .map(|r| row_tracks(r, groups))
        .unwrap_or_default()
}

/// 轨道集合拆成连续段 `(lo, hi)`（供 `Selection::add_rect_track` 等连续范围 API 使用）。
pub(crate) fn track_runs(tracks: &[u16]) -> Vec<(u16, u16)> {
    let mut runs: Vec<(u16, u16)> = Vec::new();
    for &t in tracks {
        match runs.last_mut() {
            Some((_, hi)) if hi.checked_add(1) == Some(t) => *hi = t,
            Some((_, hi)) if *hi == t => {}
            _ => runs.push((t, t)),
        }
    }
    runs
}

/// 每个轨道音符应绘制的行 id 列表（按行序升序）：
/// - 折叠通道：仅通道合并行；
/// - 展开通道：通道合并行（变窄总览）+ 该轨的轨道行。
///
/// 返回长度 = `max_track + 1`（越界轨道为空）。
pub(crate) fn note_rows_by_track(layout: &CaRowLayout, groups: &CaGroups) -> Vec<Vec<u16>> {
    let max_track = groups
        .iter_all()
        .flat_map(|g| g.tracks.iter().copied())
        .max()
        .map(|m| m as usize)
        .unwrap_or(0);
    let mut out: Vec<Vec<u16>> = vec![Vec::new(); max_track + 1];
    for (i, row) in layout.rows.iter().enumerate() {
        let Ok(i16v) = u16::try_from(i) else { continue };
        match row {
            CaRow::Channel(ch) => {
                if let Some(g) = groups.group(*ch) {
                    // 折叠：音符只画在合并行；展开：先合并行（总览），轨道行再各自追加。
                    for &t in &g.tracks {
                        if let Some(v) = out.get_mut(t as usize) {
                            v.push(i16v);
                        }
                    }
                }
            }
            CaRow::Track(t) => {
                if let Some(v) = out.get_mut(*t as usize) {
                    v.push(i16v);
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::{TrackData, TrackKind, YinModel};

    fn midi_track(port: u8, channel: u8) -> TrackData {
        TrackData::new(port, channel)
    }

    fn audio_track(ach: u16) -> TrackData {
        let mut t = TrackData::new(0, 0);
        t.kind = TrackKind::Audio;
        t.audio_channel = Some(ach);
        t
    }

    fn model_with(tracks: Vec<TrackData>) -> YinModel {
        let mut model = YinModel::new_empty_with_16_tracks();
        let mut all = vec![model.tracks[0].as_ref().clone()];
        all.extend(tracks);
        model.tracks = all.into_iter().map(std::sync::Arc::new).collect();
        model
    }

    /// 折叠布局：Conductor + 每通道一行，全行高，行高均匀。
    #[test]
    fn collapsed_layout_rows_and_heights() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 2), audio_track(0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let layout = CaRowLayout::build(true, &groups, |_| false, |_| 0, 40.0);
        assert_eq!(layout.total_rows(), 4);
        assert_eq!(layout.row(0), Some(CaRow::Conductor));
        assert_eq!(layout.row(1), Some(CaRow::Channel(CaChannel::Midi(0))));
        assert_eq!(layout.row(2), Some(CaRow::Channel(CaChannel::Midi(2))));
        assert_eq!(layout.row(3), Some(CaRow::Channel(CaChannel::Audio(0))));
        assert_eq!(layout.total_height(), 160.0);
        assert_eq!(layout.hit_row(0.0), Some(0));
        assert_eq!(layout.hit_row(39.9), Some(0));
        assert_eq!(layout.hit_row(40.0), Some(1));
        assert_eq!(layout.hit_row(160.0), None);
        assert_eq!(layout.visible_rows(40.0, 80.0), 1..3);
    }

    /// 展开布局：合并行变窄，轨道行 + 自动化行全高，y 前缀和正确。
    #[test]
    fn expanded_layout_rows_are_uniform_height() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 0), midi_track(0, 2)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let lane_counts = |t: u16| match t {
            1 => 2,
            _ => 0,
        };
        let layout = CaRowLayout::build(
            true,
            &groups,
            |ch| ch == CaChannel::Midi(0),
            lane_counts,
            40.0,
        );
        // 两段式：先全部音符轨道行，再全部自动化行（全部全高）。
        // 行：Conductor, A01, t1, t2, t1.lane0, t1.lane1, A03。
        assert_eq!(layout.total_rows(), 7);
        assert_eq!(layout.row(1), Some(CaRow::Channel(CaChannel::Midi(0))));
        assert_eq!(layout.height(1), 40.0, "展开的合并行保持全高");
        assert_eq!(layout.row(2), Some(CaRow::Track(1)));
        assert_eq!(layout.row(3), Some(CaRow::Track(2)));
        assert_eq!(layout.row(4), Some(CaRow::Automation { track: 1, sub: 0 }));
        assert_eq!(layout.row(5), Some(CaRow::Automation { track: 1, sub: 1 }));
        assert_eq!(layout.row(6), Some(CaRow::Channel(CaChannel::Midi(2))));
        assert_eq!(layout.y_top(0), 0.0);
        assert_eq!(layout.y_top(1), 40.0);
        assert_eq!(layout.y_top(2), 80.0);
        assert_eq!(layout.y_top(4), 160.0);
        assert_eq!(layout.total_height(), 280.0);
        assert_eq!(layout.hit_row(79.9), Some(1));
        assert_eq!(layout.hit_row(80.0), Some(2));
        assert_eq!(layout.hit_row(119.9), Some(2));
        assert_eq!(layout.hit_row(120.0), Some(3));
    }

    /// 轨道 → 行映射与覆盖行范围。
    #[test]
    fn track_row_map_and_covering_range() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 0), midi_track(0, 2)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let layout = CaRowLayout::build(
            true,
            &groups,
            |ch| ch == CaChannel::Midi(0),
            |t| usize::from(t == 1) * 2,
            40.0,
        );
        let map = layout.track_row_map();
        // 两段式：t1 音符行在 2、t2 音符行在 3、t1 自动化行在 4/5。
        assert_eq!(map[&1], 2);
        assert_eq!(map[&2], 3);
        // 覆盖范围取相关行（音符行 + 自动化行）的包围盒。
        assert_eq!(layout.rows_covering_tracks(&[1]), Some((2, 6)));
        assert_eq!(layout.rows_covering_tracks(&[1, 2]), Some((2, 6)));
        assert_eq!(layout.rows_covering_tracks(&[9]), None);
    }

    /// 行 → 轨道集合、轨道集合 → 连续段。
    #[test]
    fn row_tracks_and_track_runs() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 2), midi_track(0, 0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        // A01 = 轨道 1 与 3（不连续）。
        assert_eq!(
            row_tracks(CaRow::Channel(CaChannel::Midi(0)), &groups),
            vec![1, 3]
        );
        assert_eq!(row_tracks(CaRow::Track(2), &groups), vec![2]);
        assert_eq!(
            row_tracks(CaRow::Automation { track: 3, sub: 0 }, &groups),
            vec![3]
        );
        assert_eq!(row_tracks(CaRow::Conductor, &groups), Vec::<u16>::new());

        let layout = CaRowLayout::build(true, &groups, |_| false, |_| 0, 40.0);
        // 行 1 = A01 合并行（轨道 1、3），行 2 = A03 合并行（轨道 2）。
        assert_eq!(rows_range_tracks(&layout, 0..2, &groups), vec![1, 3]);
        assert_eq!(rows_range_tracks(&layout, 0..3, &groups), vec![1, 2, 3]);

        assert_eq!(track_runs(&[1, 3]), vec![(1, 1), (3, 3)]);
        assert_eq!(track_runs(&[1, 2, 3, 7]), vec![(1, 3), (7, 7)]);
        assert_eq!(track_runs(&[5, 5]), vec![(5, 5)]);
    }

    /// 每轨音符绘制行：折叠 = 仅合并行；展开 = 合并行 + 轨道行。
    #[test]
    fn note_rows_by_track_merges_and_expands() {
        let model = model_with(vec![midi_track(0, 0), midi_track(0, 0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let collapsed = CaRowLayout::build(true, &groups, |_| false, |_| 0, 40.0);
        let rows = note_rows_by_track(&collapsed, &groups);
        // 轨道 1、2 折叠时都只在合并行（行 1）。
        assert_eq!(rows[1], vec![1]);
        assert_eq!(rows[2], vec![1]);

        let expanded = CaRowLayout::build(true, &groups, |_| true, |_| 0, 40.0);
        let rows = note_rows_by_track(&expanded, &groups);
        // 展开：合并行（行 1，变窄总览）+ 各自轨道行（2、3）。
        assert_eq!(rows[1], vec![1, 2]);
        assert_eq!(rows[2], vec![1, 3]);
    }

    /// 行偏移/行高表长度与行数一致（GPU 上传不变量）。
    #[test]
    fn gpu_tables_match_row_count() {
        let model = model_with(vec![midi_track(0, 0), audio_track(0)]);
        let groups = CaGroups::from_model(&model, Some(0));
        let layout = CaRowLayout::build(true, &groups, |_| true, |_| 1, 40.0);
        assert_eq!(layout.row_offsets().len(), layout.total_rows());
        assert_eq!(layout.row_heights().len(), layout.total_rows());
        assert_eq!(layout.row_offsets()[0], 0.0);
    }
}
