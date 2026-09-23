use crate::TimelineViewBase;

/// AR 垂直缩放（行高）的最小档：16px（与改造前的最小值一致）。
pub const LANE_HEIGHT_MIN: f32 = 16.0;
/// AR 垂直缩放每档步进：8px（等差数列 16, 24, …, 120）。
pub const LANE_HEIGHT_STEP: f32 = 8.0;
/// AR 垂直缩放的最大档：120px（落在等差序列上）。
pub const LANE_HEIGHT_MAX: f32 = 120.0;
/// 跳一档所需的累积缩放倍率：单帧 `factor` 先累乘到此值（或倒数）才跳一档。
/// 1.1 ≈ 原来一次滚轮/一次可见缩放的量，避免触控板微小输入连跳多档。
pub const LANE_ZOOM_STEP_RATIO: f32 = 1.1;

/// 把任意行高吸附到最近的离散档位（16 + k·8，clamp 到 [16, 120]）。
pub fn snap_lane_height(h: f32) -> f32 {
    let k = ((h - LANE_HEIGHT_MIN) / LANE_HEIGHT_STEP).round();
    (LANE_HEIGHT_MIN + k * LANE_HEIGHT_STEP).clamp(LANE_HEIGHT_MIN, LANE_HEIGHT_MAX)
}

/// Arrangement view state: manages coordinate transforms between
/// tick/track-space and screen pixel space.
#[derive(Clone, Debug)]
pub struct ArrangementView {
    /// Shared horizontal timeline state.
    ///
    /// Lane height (AR vertical scale) is `base.track_panel_row_height`,
    /// the single source of truth shared with the track panel.
    pub base: TimelineViewBase,
    /// 离散行高缩放的输入累积器：乘积达到 `LANE_ZOOM_STEP_RATIO`（或其倒数）
    /// 才跳一档；1.0 = 无累积。见 `zoom_lane_height`。
    lane_zoom_accum: f32,
}

impl Default for ArrangementView {
    fn default() -> Self {
        Self {
            base: TimelineViewBase {
                pixels_per_tick: 0.08,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 0.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            lane_zoom_accum: 1.0,
        }
    }
}

impl ArrangementView {
    /// 用给定 base 构造（其余字段默认）。跨 crate 无法用结构体字面量
    /// 构造含私有字段的类型（如 `lane_zoom_accum`），移动端初始化用。
    pub fn with_base(base: TimelineViewBase) -> Self {
        Self {
            base,
            ..Default::default()
        }
    }

    /// Lane height in pixels (single source of truth shared with the track panel).
    #[inline]
    pub fn lane_height(&self) -> f32 {
        self.base.track_panel_row_height
    }
}

impl ArrangementView {
    /// Convert a MIDI tick to screen x coordinate.
    #[inline]
    pub fn tick_to_x(&self, tick: f64) -> f32 {
        self.base.tick_to_x(tick)
    }

    /// Convert screen x to MIDI tick.
    #[inline]
    pub fn x_to_tick(&self, x: f32) -> f64 {
        self.base.x_to_tick(x)
    }

    /// Get the screen y coordinate for a track lane.
    pub fn lane_y(&self, track_idx: usize) -> f32 {
        track_idx as f32 * self.lane_height() - self.base.scroll_y
    }

    /// The tick range visible on screen.
    #[inline]
    pub fn visible_tick_range(&self, width: f32) -> (f64, f64) {
        self.base.visible_tick_range(width)
    }

    /// The track range visible on screen.
    pub fn visible_track_range(&self, height: f32, num_tracks: usize) -> (usize, usize) {
        Self::visible_track_range_static(self.base.scroll_y, height, self.lane_height(), num_tracks)
    }

    /// Static version of `visible_track_range` — no view reference needed.
    pub fn visible_track_range_static(
        scroll_y: f32,
        height: f32,
        lane_height: f32,
        num_tracks: usize,
    ) -> (usize, usize) {
        let first = ((scroll_y / lane_height).floor() as usize).min(num_tracks.saturating_sub(1));
        let visible_count = (height / lane_height).ceil() as usize + 1;
        let last = (first + visible_count).min(num_tracks);
        (first, last)
    }

    /// Static version of `lane_y` — no view reference needed.
    pub fn lane_y_static(track_idx: usize, scroll_y: f32, lane_height: f32) -> f32 {
        track_idx as f32 * lane_height - scroll_y
    }

    /// Clamp scroll so the view doesn't go out of bounds.
    pub fn clamp_scroll(&mut self, width: f32, height: f32, total_ticks: f64, num_tracks: usize) {
        let old_x = self.base.scroll_x;
        let old_y = self.base.scroll_y;

        // Horizontal
        self.base.clamp_scroll_x(width, total_ticks);

        // Vertical
        let max_scroll_y = (num_tracks as f32 * self.lane_height() - height).max(0.0);
        self.base.scroll_y = self.base.scroll_y.clamp(0.0, max_scroll_y);

        if old_x != self.base.scroll_x || old_y != self.base.scroll_y {
            self.base.dirty = true;
        }
    }

    /// Zoom around a pointer position (horizontal).
    #[inline]
    pub fn zoom_around_x(&mut self, pointer_x: f32, zoom_factor: f32) {
        self.base.zoom_around_x(pointer_x, zoom_factor);
    }

    /// Zoom lane height around a pointer y position (vertical).
    /// 行高离散档位：输入先累积，达到 `LANE_ZOOM_STEP_RATIO` 才放大/缩小一档
    /// （已在边界则不动）。单次滚轮（factor ≈ 1.1）恰好一档。
    pub fn zoom_lane_height(&mut self, pointer_y: f32, factor: f32) {
        if factor <= 0.0 || factor == 1.0 {
            return;
        }
        self.lane_zoom_accum *= factor;
        let dir = if self.lane_zoom_accum >= LANE_ZOOM_STEP_RATIO {
            1.0
        } else if self.lane_zoom_accum <= 1.0 / LANE_ZOOM_STEP_RATIO {
            -1.0
        } else {
            return;
        };
        self.lane_zoom_accum = 1.0;

        let old = self.lane_height();
        let k = ((old - LANE_HEIGHT_MIN) / LANE_HEIGHT_STEP).round();
        let new_h = (LANE_HEIGHT_MIN + (k + dir) * LANE_HEIGHT_STEP)
            .clamp(LANE_HEIGHT_MIN, LANE_HEIGHT_MAX);
        if new_h == old {
            return;
        }
        self.base.track_panel_row_height = new_h;

        let track_frac = (pointer_y + self.base.scroll_y) / old;
        self.base.scroll_y = track_frac * new_h - pointer_y;
        self.base.scroll_y = self.base.scroll_y.max(0.0);
        self.base.dirty = true;
    }

    /// Hash of all fields that affect GPU rendering output.
    /// Used as cache key for GPU layers.
    pub fn render_hash(&self) -> u64 {
        crate::hash::hash_f32s(&[
            self.base.pixels_per_tick,
            self.base.scroll_x,
            self.base.scroll_y,
            self.base.left_panel_width,
            self.lane_height(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_lane_height_quantizes_to_step() {
        assert_eq!(snap_lane_height(16.0), 16.0);
        assert_eq!(snap_lane_height(19.0), 16.0);
        assert_eq!(snap_lane_height(21.0), 24.0);
        assert_eq!(snap_lane_height(40.0), 40.0);
        assert_eq!(snap_lane_height(0.0), LANE_HEIGHT_MIN);
        assert_eq!(snap_lane_height(200.0), LANE_HEIGHT_MAX);
    }

    #[test]
    fn zoom_lane_height_steps_one_notch_and_stops_at_bounds() {
        let mut v = ArrangementView::default(); // 40px
        v.zoom_lane_height(100.0, 1.1);
        assert_eq!(v.lane_height(), 48.0);
        v.zoom_lane_height(100.0, 0.9);
        assert_eq!(v.lane_height(), 40.0);

        v.base.track_panel_row_height = LANE_HEIGHT_MIN;
        v.zoom_lane_height(0.0, 0.9);
        assert_eq!(v.lane_height(), LANE_HEIGHT_MIN);

        v.base.track_panel_row_height = LANE_HEIGHT_MAX;
        v.zoom_lane_height(0.0, 1.1);
        assert_eq!(v.lane_height(), LANE_HEIGHT_MAX);
    }

    /// 细微连续输入不跳档，累积到阈值才跳一档（触控板微操作回归）。
    #[test]
    fn zoom_lane_height_accumulates_small_inputs() {
        let mut v = ArrangementView::default(); // 40px
        // 5 次 1%：1.051 < 1.1，不跳档
        for _ in 0..5 {
            v.zoom_lane_height(0.0, 1.01);
        }
        assert_eq!(v.lane_height(), 40.0);
        // 再 6%：累积 1.114 ≥ 1.1 → 跳一档
        v.zoom_lane_height(0.0, 1.06);
        assert_eq!(v.lane_height(), 48.0);
        // 单次滚轮 1.1 恰好一档
        v.zoom_lane_height(0.0, 1.1);
        assert_eq!(v.lane_height(), 56.0);
        // 反向同理：累积到倒数阈值才缩一档
        v.zoom_lane_height(0.0, 0.95);
        v.zoom_lane_height(0.0, 0.95);
        assert_eq!(v.lane_height(), 48.0, "0.9025 ≤ 1/1.1 才缩档");
    }

    /// 到达边界时累积器被消费重置，不会卡住后续反向缩放。
    #[test]
    fn zoom_lane_height_boundary_consumes_accumulator() {
        let mut v = ArrangementView::default();
        v.base.track_panel_row_height = LANE_HEIGHT_MAX;
        v.zoom_lane_height(0.0, 2.0); // 到顶：边界不动，累积器应被消费
        assert_eq!(v.lane_height(), LANE_HEIGHT_MAX);
        // 若累积器残留（2.0 × 0.9 = 1.8）会被误判为放大而卡在顶部；正确应缩一档。
        v.zoom_lane_height(0.0, 0.9);
        assert_eq!(v.lane_height(), LANE_HEIGHT_MAX - LANE_HEIGHT_STEP);
    }

    /// 等差档位恰好覆盖到最大档：16 + k·8 = 120。
    #[test]
    fn lane_height_max_lies_on_step_grid() {
        let k = (LANE_HEIGHT_MAX - LANE_HEIGHT_MIN) / LANE_HEIGHT_STEP;
        assert_eq!(k, k.round());
    }
}
