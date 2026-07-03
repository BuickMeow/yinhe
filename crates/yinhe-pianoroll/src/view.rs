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

    /// Hash of all fields that affect GPU rendering output.
    /// Used as cache key for GPU layers.  Includes only fields that
    /// change the visual output — adding a new field here is mandatory
    /// when it affects rendering.
    pub fn render_hash(&self) -> u64 {
        yinhe_wgpu::hash_f32s(&[
            self.base.pixels_per_tick,
            self.base.scroll_x,
            self.base.scroll_y,
            self.base.left_panel_width,
            self.key_height,
        ])
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_view() -> PianoRollView {
        PianoRollView {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: false,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            key_height: 12.0,
        }
    }

    #[test]
    fn test_default_values() {
        let v = PianoRollView::default();
        assert_eq!(v.key_height, 12.0);
        assert_eq!(v.base.pixels_per_tick, 0.15);
        assert_eq!(v.base.left_panel_width, 60.0);
        assert!(v.base.dirty);
    }

    #[test]
    fn test_keyboard_width() {
        let v = make_view();
        assert_eq!(v.keyboard_width(), 60.0);
    }

    #[test]
    fn test_total_key_height() {
        let v = make_view();
        assert_eq!(v.total_key_height(), 128.0 * 12.0);
    }

    #[test]
    fn test_tick_to_x_origin() {
        let v = make_view();
        let x = v.tick_to_x(0.0);
        assert!((x - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_tick_to_x_with_value() {
        let v = make_view();
        let x = v.tick_to_x(480.0);
        assert!((x - (60.0 + 480.0 * 0.15)).abs() < 0.01);
    }

    #[test]
    fn test_key_to_y_key_60() {
        let v = make_view();
        // Key 60 (middle C): bottom = 128*12 - 0 = 1536
        // y = 1536 - (60+1)*12 = 1536 - 732 = 804
        let y = v.key_to_y(60);
        assert!((y - 804.0).abs() < 0.01);
    }

    #[test]
    fn test_key_to_y_top_key() {
        let v = make_view();
        let y = v.key_to_y(127);
        assert!((y - (1536.0 - 128.0 * 12.0)).abs() < 0.01);
    }

    #[test]
    fn test_key_to_y_bottom_key() {
        let v = make_view();
        let y = v.key_to_y(0);
        assert!((y - (1536.0 - 12.0)).abs() < 0.01);
    }

    #[test]
    fn test_x_to_tick_roundtrip() {
        let v = make_view();
        let tick = 480.0;
        let x = v.tick_to_x(tick);
        let back = v.x_to_tick(x);
        assert!((back - tick).abs() < 0.1);
    }

    #[test]
    fn test_y_to_key_roundtrip() {
        let v = make_view();
        for key in [0, 12, 36, 60, 72, 127] {
            let y = v.key_to_y(key);
            let back = v.y_to_key(y + 6.0); // middle of the key
            assert_eq!(back, key, "key {} roundtrip failed", key);
        }
    }

    #[test]
    fn test_y_to_key_clamps() {
        let v = make_view();
        assert_eq!(v.y_to_key(-100.0), 127);
        assert_eq!(v.y_to_key(99999.0), 0);
    }

    #[test]
    fn test_visible_tick_range() {
        let v = make_view();
        let (start, end) = v.visible_tick_range(1100.0);
        assert!((start - 0.0).abs() < 1.0);
        assert!((end - 6933.33).abs() < 1.0);
    }

    #[test]
    fn test_visible_key_range() {
        let v = make_view();
        let (lo, hi) = v.visible_key_range(500.0);
        assert!(lo < hi);
        assert!(hi <= 127);
    }

    #[test]
    fn test_clamp_scroll_horizontal() {
        let mut v = make_view();
        v.base.scroll_x = 99999.0;
        v.clamp_scroll(1000.0, 500.0, 10000.0);
        assert!(v.base.scroll_x < 99999.0);
    }

    #[test]
    fn test_clamp_scroll_vertical() {
        let mut v = make_view();
        v.base.scroll_y = 99999.0;
        v.clamp_scroll(1000.0, 500.0, 10000.0);
        let max_scroll = (128.0f32 * 12.0 - 500.0).max(0.0);
        assert!(v.base.scroll_y <= max_scroll);
    }

    #[test]
    fn test_clamp_scroll_sets_dirty_on_change() {
        let mut v = make_view();
        v.base.dirty = false;
        v.base.scroll_x = 100.0;
        v.clamp_scroll(1000.0, 500.0, 100.0);
        assert!(v.base.dirty);
    }

    #[test]
    fn test_clamp_scroll_snaps_key_height_when_close() {
        let mut v = make_view();
        v.key_height = 12.0;
        // total = 1536, height = 1532, diff = 4 < 5
        v.clamp_scroll(1000.0, 1532.0, 10000.0);
        assert!((v.key_height - 1532.0 / 128.0).abs() < 0.01);
    }

    #[test]
    fn test_zoom_around_x_preserves_tick() {
        let mut v = make_view();
        let px = 300.0;
        let tick_before = v.x_to_tick(px);
        v.zoom_around_x(px, 2.0);
        let tick_after = v.x_to_tick(px);
        assert!((tick_before - tick_after).abs() < 1.0);
    }

    #[test]
    fn test_zoom_around_x_clamps() {
        let mut v = make_view();
        v.zoom_around_x(300.0, 0.0001);
        assert!(v.base.pixels_per_tick >= 0.001);
        v.zoom_around_x(300.0, 100.0);
        assert!(v.base.pixels_per_tick <= 10.0);
    }

    #[test]
    fn test_zoom_around_y_preserves_pointer() {
        let mut v = make_view();
        let py = 200.0;
        let key_before = v.y_to_key(py);
        v.zoom_around_y(py, 2.0, 500.0);
        let key_after = v.y_to_key(py);
        assert_eq!(key_before, key_after);
    }

    #[test]
    fn test_zoom_around_y_clamps_min() {
        let mut v = make_view();
        v.zoom_around_y(200.0, 0.01, 500.0);
        let min_kh = 500.0 / 128.0;
        assert!(v.key_height >= min_kh - 0.001);
    }

    #[test]
    fn test_zoom_around_y_clamps_max() {
        let mut v = make_view();
        v.zoom_around_y(200.0, 100.0, 500.0);
        assert!(v.key_height <= 60.0);
    }

    #[test]
    fn test_zoom_around_y_sets_dirty() {
        let mut v = make_view();
        v.base.dirty = false;
        v.zoom_around_y(200.0, 1.0, 500.0);
        assert!(v.base.dirty);
    }
}
