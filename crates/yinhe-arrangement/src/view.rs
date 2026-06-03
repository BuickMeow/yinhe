/// Arrangement view state: manages coordinate transforms between
/// tick/track-space and screen pixel space.
#[derive(Clone, Debug)]
pub struct ArrangementView {
    /// Pixels per MIDI tick (horizontal zoom).
    pub pixels_per_tick: f32,
    /// Height of each track lane in pixels.
    pub lane_height: f32,
    /// Width of the left-side label area in pixels.
    pub label_width: f32,
    /// Horizontal scroll offset in pixels.
    pub scroll_x: f32,
    /// Vertical scroll offset in pixels (for track lanes).
    pub scroll_y: f32,
    /// Whether view state has changed since last render — set to false after prepare.
    pub dirty: bool,
}

impl Default for ArrangementView {
    fn default() -> Self {
        Self {
            pixels_per_tick: 0.08,
            lane_height: 40.0,
            label_width: 120.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            dirty: true,
        }
    }
}

impl ArrangementView {
    /// Convert a MIDI tick to screen x coordinate.
    pub fn tick_to_x(&self, tick: f64) -> f32 {
        self.label_width + (tick as f32 * self.pixels_per_tick) - self.scroll_x
    }

    /// Convert screen x to MIDI tick.
    pub fn x_to_tick(&self, x: f32) -> f64 {
        ((x - self.label_width + self.scroll_x) / self.pixels_per_tick) as f64
    }

    /// Get the screen y coordinate for a track lane.
    pub fn lane_y(&self, track_idx: usize) -> f32 {
        track_idx as f32 * self.lane_height - self.scroll_y
    }

    /// The tick range visible on screen.
    pub fn visible_tick_range(&self, width: f32) -> (f64, f64) {
        let start = self.x_to_tick(self.label_width).max(0.0);
        let end = self.x_to_tick(width);
        (start, end)
    }

    /// The track range visible on screen.
    pub fn visible_track_range(&self, height: f32, num_tracks: usize) -> (usize, usize) {
        let first = ((self.scroll_y / self.lane_height).floor() as usize).min(num_tracks.saturating_sub(1));
        let visible_count = (height / self.lane_height).ceil() as usize + 1;
        let last = (first + visible_count).min(num_tracks);
        (first, last)
    }

    /// Clamp scroll so the view doesn't go out of bounds.
    pub fn clamp_scroll(&mut self, width: f32, height: f32, total_ticks: f64, num_tracks: usize) {
        let old_x = self.scroll_x;
        let old_y = self.scroll_y;

        let min_scroll_x = 0.0;
        let max_scroll_x =
            (total_ticks as f32 * self.pixels_per_tick - (width - self.label_width)).max(0.0);
        self.scroll_x = self.scroll_x.clamp(min_scroll_x, max_scroll_x);

        let max_scroll_y = (num_tracks as f32 * self.lane_height - height).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll_y);

        if old_x != self.scroll_x || old_y != self.scroll_y {
            self.dirty = true;
        }
    }

    /// Zoom around a pointer position (horizontal).
    pub fn zoom_around_x(&mut self, pointer_x: f32, zoom_factor: f32) {
        let old = self.pixels_per_tick;
        self.pixels_per_tick = (self.pixels_per_tick * zoom_factor).clamp(0.001, 10.0);

        let tick = (pointer_x - self.label_width + self.scroll_x) / old;
        self.scroll_x = tick * self.pixels_per_tick - (pointer_x - self.label_width);
        self.dirty = true;
    }
}
