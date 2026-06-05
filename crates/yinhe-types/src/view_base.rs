/// Shared horizontal timeline state used by both PianoRollView and ArrangementView.
///
/// Encapsulates the fields and methods that are identical between the two views:
/// horizontal zoom, horizontal scroll, left-panel width, and coordinate transforms.
#[derive(Clone, Debug)]
pub struct TimelineViewBase {
    /// Pixels per MIDI tick (horizontal zoom level).
    pub pixels_per_tick: f32,
    /// Horizontal scroll offset in pixels.
    pub scroll_x: f32,
    /// Vertical scroll offset in pixels.
    pub scroll_y: f32,
    /// Width of the left-side fixed panel (keyboard for pianoroll, labels for arrangement).
    pub left_panel_width: f32,
    /// Whether view state has changed since last render.
    pub dirty: bool,
    /// Row height for the track panel (synced with lane_height in arrangement).
    pub track_panel_row_height: f32,
    /// Vertical scroll offset for the track panel.
    pub track_panel_scroll_y: f32,
}

impl TimelineViewBase {
    /// Convert a MIDI tick to screen x coordinate.
    #[inline]
    pub fn tick_to_x(&self, tick: f64) -> f32 {
        self.left_panel_width + (tick as f32 * self.pixels_per_tick) - self.scroll_x
    }

    /// Convert screen x to MIDI tick.
    #[inline]
    pub fn x_to_tick(&self, x: f32) -> f64 {
        ((x - self.left_panel_width + self.scroll_x) / self.pixels_per_tick) as f64
    }

    /// The tick range visible on screen.
    pub fn visible_tick_range(&self, width: f32) -> (f64, f64) {
        let start = self.x_to_tick(self.left_panel_width).max(0.0);
        let end = self.x_to_tick(width);
        (start, end)
    }

    /// Zoom around a pointer position (horizontal).
    pub fn zoom_around_x(&mut self, pointer_x: f32, zoom_factor: f32) {
        let old = self.pixels_per_tick;
        self.pixels_per_tick = (self.pixels_per_tick * zoom_factor).clamp(0.001, 10.0);

        // Keep the tick under the pointer stationary
        let tick = (pointer_x - self.left_panel_width + self.scroll_x) / old;
        self.scroll_x = tick * self.pixels_per_tick - (pointer_x - self.left_panel_width);
        self.dirty = true;
    }

    /// Clamp horizontal scroll so the view doesn't go out of bounds.
    pub fn clamp_scroll_x(&mut self, width: f32, total_ticks: f64) {
        let max_scroll_x =
            (total_ticks as f32 * self.pixels_per_tick - (width - self.left_panel_width)).max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, max_scroll_x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_base() -> TimelineViewBase {
        TimelineViewBase {
            pixels_per_tick: 0.1,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 100.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
        }
    }

    #[test]
    fn test_tick_to_x_origin() {
        let b = make_base();
        // tick=0 at left_panel_width
        assert!((b.tick_to_x(0.0) - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_tick_to_x_with_scroll() {
        let mut b = make_base();
        b.scroll_x = 50.0;
        // tick=0 → 100 + 0 - 50 = 50
        assert!((b.tick_to_x(0.0) - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_x_to_tick_roundtrip() {
        let b = make_base();
        let tick = 480.0f64;
        let x = b.tick_to_x(tick);
        let back = b.x_to_tick(x);
        assert!((back - tick).abs() < 0.1);
    }

    #[test]
    fn test_visible_tick_range() {
        let b = make_base();
        let (start, end) = b.visible_tick_range(1100.0);
        // start = x_to_tick(100) = 0, end = x_to_tick(1100) = 10000
        assert!((start - 0.0).abs() < 1.0);
        assert!((end - 10000.0).abs() < 1.0);
    }

    #[test]
    fn test_zoom_around_x_preserves_tick() {
        let mut b = make_base();
        let pointer_x = 300.0;
        let tick_before = b.x_to_tick(pointer_x);
        b.zoom_around_x(pointer_x, 2.0);
        let tick_after = b.x_to_tick(pointer_x);
        assert!((tick_before - tick_after).abs() < 1.0);
    }

    #[test]
    fn test_clamp_scroll_x() {
        let mut b = make_base();
        b.scroll_x = 99999.0;
        b.clamp_scroll_x(1000.0, 10000.0);
        // max = 10000*0.1 - (1000-100) = 1000 - 900 = 100
        assert!(b.scroll_x <= 100.0 + 0.01);
    }
}
