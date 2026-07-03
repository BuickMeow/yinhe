use yinhe_types::TimelineViewBase;

/// Arrangement view state: manages coordinate transforms between
/// tick/track-space and screen pixel space.
#[derive(Clone, Debug)]
pub struct ArrangementView {
    /// Shared horizontal timeline state.
    pub base: TimelineViewBase,
    /// Height of each track lane in pixels.
    pub lane_height: f32,
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
            },
            lane_height: 40.0,
        }
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
        track_idx as f32 * self.lane_height - self.base.scroll_y
    }

    /// The tick range visible on screen.
    #[inline]
    pub fn visible_tick_range(&self, width: f32) -> (f64, f64) {
        self.base.visible_tick_range(width)
    }

    /// The track range visible on screen.
    pub fn visible_track_range(&self, height: f32, num_tracks: usize) -> (usize, usize) {
        Self::visible_track_range_static(self.base.scroll_y, height, self.lane_height, num_tracks)
    }

    /// Static version of `visible_track_range` — no view reference needed.
    pub fn visible_track_range_static(
        scroll_y: f32,
        height: f32,
        lane_height: f32,
        num_tracks: usize,
    ) -> (usize, usize) {
        let first = ((scroll_y / lane_height).floor() as usize)
            .min(num_tracks.saturating_sub(1));
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
        let max_scroll_y = (num_tracks as f32 * self.lane_height - height).max(0.0);
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
    pub fn zoom_lane_height(&mut self, pointer_y: f32, factor: f32) {
        let old = self.lane_height;
        self.lane_height = (self.lane_height * factor).clamp(16.0, 120.0);
        self.base.track_panel_row_height = self.lane_height;

        let track_frac = (pointer_y + self.base.scroll_y) / old;
        self.base.scroll_y = track_frac * self.lane_height - pointer_y;
        self.base.scroll_y = self.base.scroll_y.max(0.0);
        self.base.dirty = true;
    }

    /// Hash of all fields that affect GPU rendering output.
    /// Used as cache key for GPU layers.
    pub fn render_hash(&self) -> u64 {
        yinhe_wgpu::hash_f32s(&[
            self.base.pixels_per_tick,
            self.base.scroll_x,
            self.base.scroll_y,
            self.base.left_panel_width,
            self.lane_height,
        ])
    }
}
