/// Piano roll view state: manages coordinate transforms between
/// tick/key space and screen pixel space.
#[derive(Clone, Debug)]
pub struct PianoRollView {
    /// Pixels per MIDI tick (horizontal zoom).
    pub pixels_per_tick: f32,
    /// Pixels per MIDI key (vertical zoom).
    pub key_height: f32,
    /// Horizontal scroll offset in pixels.
    pub scroll_x: f32,
    /// Vertical scroll offset in pixels.
    pub scroll_y: f32,
    /// Width of the piano keyboard on the left side (pixels).
    pub keyboard_width: f32,
    /// Whether view state has changed since last render — set to false after prepare.
    pub dirty: bool,
}

impl Default for PianoRollView {
    fn default() -> Self {
        Self {
            pixels_per_tick: 0.15,
            key_height: 12.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            keyboard_width: 80.0,
            dirty: true,
        }
    }
}

impl PianoRollView {
    /// Total height of all 128 keys in pixels.
    pub fn total_key_height(&self) -> f32 {
        128.0 * self.key_height
    }

    /// Convert a MIDI tick to screen x coordinate.
    pub fn tick_to_x(&self, tick: f64) -> f32 {
        self.keyboard_width + (tick as f32 * self.pixels_per_tick) - self.scroll_x
    }

    /// Convert a MIDI key (0-127) to screen y coordinate.
    /// Key 127 (G9) is at the top, key 0 (C-1) is at the bottom.
    pub fn key_to_y(&self, key: u8) -> f32 {
        let bottom = self.total_key_height() - self.scroll_y;
        bottom - (key as f32 + 1.0) * self.key_height
    }

    /// Convert screen x to MIDI tick.
    pub fn x_to_tick(&self, x: f32) -> f64 {
        ((x - self.keyboard_width + self.scroll_x) / self.pixels_per_tick) as f64
    }

    /// Convert screen y to MIDI key.
    ///
    /// Key k's row occupies y ∈ [key_to_y(k), key_to_y(k) + key_height).
    /// The correct key for a given y is ceil((bottom - y) / key_height) - 1.
    pub fn y_to_key(&self, y: f32) -> u8 {
        let bottom = self.total_key_height() - self.scroll_y;
        let key_f = ((bottom - y) / self.key_height).clamp(0.0, 128.0);
        (key_f.ceil() as u8).saturating_sub(1)
    }

    /// The tick range visible on screen.
    pub fn visible_tick_range(&self, width: f32) -> (f64, f64) {
        let start = self.x_to_tick(self.keyboard_width).max(0.0);
        let end = self.x_to_tick(width);
        (start, end)
    }

    /// The key range visible on screen.
    pub fn visible_key_range(&self, height: f32) -> (u8, u8) {
        let top_key = self.y_to_key(0.0);
        let bottom_key = self.y_to_key(height);
        (bottom_key.min(top_key), top_key.max(bottom_key))
    }

    /// Clamp scroll so the view doesn't go out of bounds.
    pub fn clamp_scroll(&mut self, width: f32, height: f32, total_ticks: f64) {
        let old_x = self.scroll_x;
        let old_y = self.scroll_y;
        let old_kh = self.key_height;

        // Horizontal: don't scroll before tick 0
        let min_scroll_x = 0.0;
        let max_scroll_x = (total_ticks as f32 * self.pixels_per_tick - (width - self.keyboard_width)).max(0.0);
        self.scroll_x = self.scroll_x.clamp(min_scroll_x, max_scroll_x);

        // When total height exceeds viewport by only a few pixels, it means the
        // viewport shrunk after a minimum-zoom was set.  Re-snap key_height so
        // that all 128 keys exactly fill the viewport, eliminating the tiny
        // scroll margin.
        let total = self.total_key_height();
        if total > height && total - height < 5.0 {
            self.key_height = height / 128.0;
        }

        // Vertical: don't scroll beyond key range
        let max_scroll_y = (self.total_key_height() - height).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll_y);

        if old_x != self.scroll_x || old_y != self.scroll_y || old_kh != self.key_height {
            self.dirty = true;
        }
    }

    /// Zoom around a pointer position (horizontal).
    pub fn zoom_around_x(&mut self, pointer_x: f32, zoom_factor: f32) {
        let old = self.pixels_per_tick;
        self.pixels_per_tick = (self.pixels_per_tick * zoom_factor).clamp(0.001, 10.0);

        // Keep the tick under the pointer stationary
        let tick = (pointer_x - self.keyboard_width + self.scroll_x) / old;
        self.scroll_x = tick * self.pixels_per_tick - (pointer_x - self.keyboard_width);
        self.dirty = true;
    }

    /// Zoom around a pointer position (vertical).
    /// viewport_height is used to compute the minimum key_height so all 128 keys fill the viewport.
    pub fn zoom_around_y(&mut self, pointer_y: f32, zoom_factor: f32, viewport_height: f32) {
        let min_kh = viewport_height / 128.0;
        let old = self.key_height;
        self.key_height = (self.key_height * zoom_factor).clamp(min_kh, 60.0);

        // Keep the key row under the pointer stationary
        self.scroll_y = (self.scroll_y + pointer_y) / old * self.key_height - pointer_y;
        self.dirty = true;
    }
}
