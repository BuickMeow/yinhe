//! AR（编排视图）行布局：音轨可展开自动化 lane 后，每轨占据
//! 1（未展开）或 1 + lane_count（展开）个等高行。
//!
//! 所有行等高（= 音轨行高 lane_height），因此 y ↔ 行号的换算保持均匀模型：
//! y = row * lane_height - scroll_y；只有“行号 → (音轨, 子 lane)”的映射
//! 是不均匀的，由本结构的前缀和提供 O(log n) 查询。

/// 一行的命中结果：所属音轨 + 是否为自动化子行（子 lane 索引）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArRow {
    /// 音轨主行（音符行）。
    Track(usize),
    /// 音轨的自动化子行，usize 是该轨 automation_lanes 的下标。
    Automation(usize, usize),
}

/// 每帧由“各轨展开的 lane 数”构建的行布局。
#[derive(Clone, Debug, Default)]
pub struct ArRowLayout {
    /// 每轨第一行的行号（前缀和），len = num_tracks。
    row_starts: Vec<u32>,
    /// 每轨的行数（1 + 展开的 lane 数），len = num_tracks。
    row_counts: Vec<u32>,
    /// 总行数。
    total_rows: u32,
}

impl ArRowLayout {
    /// 从每轨展开的 lane 数构建（未展开的轨传 0）。
    pub fn new(expanded_lane_counts: impl IntoIterator<Item = u32>) -> Self {
        let mut row_starts = Vec::new();
        let mut row_counts = Vec::new();
        let mut acc = 0u32;
        for lanes in expanded_lane_counts {
            row_starts.push(acc);
            let rows = 1 + lanes;
            row_counts.push(rows);
            acc += rows;
        }
        Self {
            row_starts,
            row_counts,
            total_rows: acc,
        }
    }

    /// 总行数（所有轨的主行 + 展开的自动化子行）。
    #[inline]
    pub fn total_rows(&self) -> usize {
        self.total_rows as usize
    }

    /// 音轨主行的行号。
    #[inline]
    pub fn track_row(&self, track: usize) -> usize {
        self.row_starts.get(track).copied().unwrap_or(0) as usize
    }

    /// 音轨占据的行数（1 + 展开的 lane 数）。
    #[inline]
    pub fn track_rows(&self, track: usize) -> usize {
        self.row_counts.get(track).copied().unwrap_or(1) as usize
    }

    /// 音轨的自动化子行 sub 的行号。
    #[inline]
    pub fn lane_row(&self, track: usize, sub: usize) -> usize {
        self.track_row(track) + 1 + sub
    }

    /// 行号 → 命中结果。越界行号返回 None。
    pub fn row_hit(&self, row: usize) -> Option<ArRow> {
        if row >= self.total_rows as usize {
            return None;
        }
        // 最后一个 row_start <= row 的轨。
        let track = self.row_starts.partition_point(|&s| s as usize <= row) - 1;
        let sub = row - self.row_starts[track] as usize;
        if sub == 0 {
            Some(ArRow::Track(track))
        } else {
            Some(ArRow::Automation(track, sub - 1))
        }
    }

    /// 音乐坐标 y（未减 scroll_y）→ 命中行。
    #[inline]
    pub fn hit_at_music_y(&self, y: f32, lane_height: f32) -> Option<ArRow> {
        if y < 0.0 || lane_height <= 0.0 {
            return None;
        }
        self.row_hit((y / lane_height).floor() as usize)
    }

    /// 音轨主行的音乐坐标 y（未减 scroll_y）。
    #[inline]
    pub fn track_y(&self, track: usize, lane_height: f32) -> f32 {
        self.track_row(track) as f32 * lane_height
    }

    /// 音轨的音乐坐标总高（主行 + 展开的子行）。
    #[inline]
    pub fn track_height(&self, track: usize, lane_height: f32) -> f32 {
        self.track_rows(track) as f32 * lane_height
    }

    /// 全部音轨主行的音乐坐标 y（供 GPU shader 的 per-track 偏移表）。
    pub fn track_offsets(&self, lane_height: f32) -> Vec<f32> {
        self.row_starts
            .iter()
            .map(|&s| s as f32 * lane_height)
            .collect()
    }

    /// 内容总高（音乐坐标）。
    #[inline]
    pub fn total_height(&self, lane_height: f32) -> f32 {
        self.total_rows as f32 * lane_height
    }

    /// 可视音轨范围 [first, last)：任一可见行所属的音轨（含部分可见）。
    pub fn visible_track_range(
        &self,
        scroll_y: f32,
        height: f32,
        lane_height: f32,
    ) -> (usize, usize) {
        let num_tracks = self.row_starts.len();
        if num_tracks == 0 || lane_height <= 0.0 {
            return (0, 0);
        }
        let first_row = (scroll_y / lane_height).floor().max(0.0) as usize;
        let last_row = ((scroll_y + height) / lane_height).ceil().max(0.0) as usize;
        let first = self.row_hit(first_row).map(|h| h.track()).unwrap_or(0);
        let last = self
            .row_hit(last_row.min(self.total_rows as usize).saturating_sub(1))
            .map(|h| h.track() + 1)
            .unwrap_or(num_tracks)
            .min(num_tracks);
        (first.min(last), last)
    }
}

impl ArRow {
    /// 所属音轨索引。
    #[inline]
    pub fn track(self) -> usize {
        match self {
            ArRow::Track(t) | ArRow::Automation(t, _) => t,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3 轨：轨 0 未展开，轨 1 展开 2 条 lane，轨 2 展开 1 条 lane。
    /// 行分布：row0=轨0主行, row1=轨1主行, row2=轨1.lane0, row3=轨1.lane1,
    ///         row4=轨2主行, row5=轨2.lane0
    fn layout() -> ArRowLayout {
        ArRowLayout::new([0, 2, 1])
    }

    #[test]
    fn totals_and_track_rows() {
        let l = layout();
        assert_eq!(l.total_rows(), 6);
        assert_eq!(l.track_row(0), 0);
        assert_eq!(l.track_row(1), 1);
        assert_eq!(l.track_row(2), 4);
        assert_eq!(l.track_rows(0), 1);
        assert_eq!(l.track_rows(1), 3);
        assert_eq!(l.track_rows(2), 2);
        assert_eq!(l.lane_row(1, 0), 2);
        assert_eq!(l.lane_row(1, 1), 3);
        assert_eq!(l.lane_row(2, 0), 5);
    }

    #[test]
    fn row_hit_roundtrip() {
        let l = layout();
        assert_eq!(l.row_hit(0), Some(ArRow::Track(0)));
        assert_eq!(l.row_hit(1), Some(ArRow::Track(1)));
        assert_eq!(l.row_hit(2), Some(ArRow::Automation(1, 0)));
        assert_eq!(l.row_hit(3), Some(ArRow::Automation(1, 1)));
        assert_eq!(l.row_hit(4), Some(ArRow::Track(2)));
        assert_eq!(l.row_hit(5), Some(ArRow::Automation(2, 0)));
        assert_eq!(l.row_hit(6), None);
    }

    #[test]
    fn hit_at_music_y_uses_lane_height() {
        let l = layout();
        let lh = 40.0;
        assert_eq!(l.hit_at_music_y(0.0, lh), Some(ArRow::Track(0)));
        assert_eq!(l.hit_at_music_y(79.9, lh), Some(ArRow::Track(1)));
        assert_eq!(l.hit_at_music_y(80.0, lh), Some(ArRow::Automation(1, 0)));
        assert_eq!(l.hit_at_music_y(-1.0, lh), None);
        assert_eq!(l.hit_at_music_y(240.0, lh), None);
    }

    #[test]
    fn offsets_and_heights() {
        let l = layout();
        assert_eq!(l.track_offsets(40.0), vec![0.0, 40.0, 160.0]);
        assert_eq!(l.track_height(1, 40.0), 120.0);
        assert_eq!(l.total_height(40.0), 240.0);
    }

    #[test]
    fn visible_track_range_covers_partial_rows() {
        let l = layout();
        let lh = 40.0;
        // 视口 y ∈ [40, 120)：覆盖 row1..row3 → 轨 1（含其 lane）。
        assert_eq!(l.visible_track_range(40.0, 80.0, lh), (1, 2));
        // 视口 y ∈ [0, 240]：全部。
        assert_eq!(l.visible_track_range(0.0, 240.0, lh), (0, 3));
        // 视口只盖住 row2（轨 1 的 lane0）。
        assert_eq!(l.visible_track_range(81.0, 30.0, lh), (1, 2));
        // 空布局。
        let empty = ArRowLayout::new([]);
        assert_eq!(empty.visible_track_range(0.0, 100.0, lh), (0, 0));
    }
}
