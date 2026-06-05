use yinhe_types::TimelineViewBase;

/// Piano roll view state: manages coordinate transforms between
/// tick/key space and screen pixel space.
#[derive(Clone, Debug)]
pub struct PianoRollView {
    /// Shared horizontal timeline state.
    pub base: TimelineViewBase,
    /// Pixels per MIDI key (vertical zoom).
    pub key_height: f32,
}

impl Default for PianoRollView {
    fn default() -> Self {
        Self {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            key_height: 12.0,
        }
    }
}

impl PianoRollView {
    /// Convenience alias for the keyboard width.
    #[inline]
    pub fn keyboard_width(&self) -> f32 {
        self.base.left_panel_width
    }

    /// Total height of all 128 keys in pixels.
    pub fn total_key_height(&self) -> f32 {
        128.0 * self.key_height
    }

    /// Convert a MIDI tick to screen x coordinate.
    #[inline]
    pub fn tick_to_x(&self, tick: f64) -> f32 {
        self.base.tick_to_x(tick)
    }

    /// Convert a MIDI key (0-127) to screen y coordinate.
    /// Key 127 (G9) is at the top, key 0 (C-1) is at the bottom.
    pub fn key_to_y(&self, key: u8) -> f32 {
        let bottom = self.total_key_height() - self.base.scroll_y;
        bottom - (key as f32 + 1.0) * self.key_height
    }

    /// Convert screen x to MIDI tick.
    #[inline]
    pub fn x_to_tick(&self, x: f32) -> f64 {
        self.base.x_to_tick(x)
    }

    /// Convert screen y to MIDI key.
    ///
    /// Key k's row occupies y in [key_to_y(k), key_to_y(k) + key_height).
    pub fn y_to_key(&self, y: f32) -> u8 {
        let bottom = self.total_key_height() - self.base.scroll_y;
        let key_f = ((bottom - y) / self.key_height).clamp(0.0, 128.0);
        (key_f.ceil() as u8).saturating_sub(1)
    }

    /// The tick range visible on screen.
    #[inline]
    pub fn visible_tick_range(&self, width: f32) -> (f64, f64) {
        self.base.visible_tick_range(width)
    }

    /// The key range visible on screen.
    pub fn visible_key_range(&self, height: f32) -> (u8, u8) {
        let top_key = self.y_to_key(0.0);
        let bottom_key = self.y_to_key(height);
        (bottom_key.min(top_key), top_key.max(bottom_key))
    }

    /// Clamp scroll so the view doesn't go out of bounds.
    pub fn clamp_scroll(&mut self, width: f32, height: f32, total_ticks: f64) {
        let old_x = self.base.scroll_x;
        let old_y = self.base.scroll_y;
        let old_kh = self.key_height;

        // Horizontal
        self.base.clamp_scroll_x(width, total_ticks);

        // When total height exceeds viewport by only a few pixels, re-snap key_height
        let total = self.total_key_height();
        if total > height && total - height < 5.0 {
            self.key_height = height / 128.0;
        }

        // Vertical: don't scroll beyond key range
        let max_scroll_y = (self.total_key_height() - height).max(0.0);
        self.base.scroll_y = self.base.scroll_y.clamp(0.0, max_scroll_y);

        if old_x != self.base.scroll_x || old_y != self.base.scroll_y || old_kh != self.key_height {
            self.base.dirty = true;
        }
    }

    /// Zoom around a pointer position (horizontal).
    #[inline]
    pub fn zoom_around_x(&mut self, pointer_x: f32, zoom_factor: f32) {
        self.base.zoom_around_x(pointer_x, zoom_factor);
    }

    /// Zoom around a pointer position (vertical).
    pub fn zoom_around_y(&mut self, pointer_y: f32, zoom_factor: f32, viewport_height: f32) {
        let min_kh = viewport_height / 128.0;
        let old = self.key_height;
        self.key_height = (self.key_height * zoom_factor).clamp(min_kh, 60.0);

        self.base.scroll_y = (self.base.scroll_y + pointer_y) / old * self.key_height - pointer_y;
        self.base.dirty = true;
    }
}
