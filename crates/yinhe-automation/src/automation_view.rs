use yinhe_types::{AutomationTarget, TimelineViewBase};

/// Default panel height in pixels.
pub const DEFAULT_PANEL_HEIGHT: f32 = 80.0;
/// Minimum panel height when dragging.
pub const MIN_PANEL_HEIGHT: f32 = 40.0;
/// Maximum panel height when dragging.
pub const MAX_PANEL_HEIGHT: f32 = 200.0;

/// View state for a single automation panel in the controller area below the pianoroll.
#[derive(Clone, Debug)]
pub struct AutomationPanelView {
    /// Shared horizontal timeline state (scroll_x, pixels_per_tick, left_panel_width).
    /// These fields are synced from the pianoroll view each frame.
    pub base: TimelineViewBase,
    /// Current panel height in pixels.
    pub panel_height: f32,
    /// The automation target currently displayed in this panel.
    pub selected_target: AutomationTarget,
    /// Cached index into `MidiFile.automation_lanes` for fast lookup.
    pub lane_index: usize,
    /// Whether the panel content needs to be rebuilt.
    pub dirty: bool,
}

impl Default for AutomationPanelView {
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
            panel_height: DEFAULT_PANEL_HEIGHT,
            selected_target: AutomationTarget::Velocity,
            lane_index: 0,
            dirty: true,
        }
    }
}

impl AutomationPanelView {
    /// Sync horizontal scroll state from the pianoroll view.
    pub fn sync_from_pianoroll(
        &mut self,
        scroll_x: f32,
        pixels_per_tick: f32,
        left_panel_width: f32,
    ) {
        if self.base.scroll_x != scroll_x
            || self.base.pixels_per_tick != pixels_per_tick
            || self.base.left_panel_width != left_panel_width
        {
            self.base.scroll_x = scroll_x;
            self.base.pixels_per_tick = pixels_per_tick;
            self.base.left_panel_width = left_panel_width;
            self.dirty = true;
        }
    }

    /// Convenience: keyboard / left-panel width.
    #[inline]
    pub fn left_panel_width(&self) -> f32 {
        self.base.left_panel_width
    }

    /// Hash of all fields that affect GPU rendering output.
    /// Used as cache key for GPU layers.
    pub fn render_hash(&self) -> u64 {
        let mut h: u64 = 0;
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(self.base.pixels_per_tick.to_bits() as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(self.base.scroll_x.to_bits() as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(self.base.left_panel_width.to_bits() as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(self.panel_height.to_bits() as u64);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let view = AutomationPanelView::default();
        assert_eq!(view.panel_height, DEFAULT_PANEL_HEIGHT);
        assert_eq!(view.selected_target, AutomationTarget::Velocity);
        assert_eq!(view.lane_index, 0);
        assert!(view.dirty);
        assert_eq!(view.base.pixels_per_tick, 0.15);
        assert_eq!(view.base.scroll_x, 0.0);
        assert_eq!(view.base.left_panel_width, 60.0);
    }

    #[test]
    fn test_sync_from_pianoroll_updates_values() {
        let mut view = AutomationPanelView::default();
        view.dirty = false;

        view.sync_from_pianoroll(100.0, 0.5, 80.0);

        assert_eq!(view.base.scroll_x, 100.0);
        assert_eq!(view.base.pixels_per_tick, 0.5);
        assert_eq!(view.base.left_panel_width, 80.0);
        assert!(view.dirty);
    }

    #[test]
    fn test_sync_from_pianoroll_no_change_skips_dirty() {
        let mut view = AutomationPanelView::default();
        view.dirty = false;

        view.sync_from_pianoroll(0.0, 0.15, 60.0);

        assert!(
            !view.dirty,
            "dirty should remain false when values unchanged"
        );
    }

    #[test]
    fn test_sync_from_pianoroll_partial_update_triggers_dirty() {
        let mut view = AutomationPanelView::default();
        view.dirty = false;

        // Only change scroll_x
        view.sync_from_pianoroll(50.0, 0.15, 60.0);

        assert!(view.dirty);
        assert_eq!(view.base.scroll_x, 50.0);
        assert_eq!(view.base.pixels_per_tick, 0.15);
        assert_eq!(view.base.left_panel_width, 60.0);
    }

    #[test]
    fn test_left_panel_width_returns_base_value() {
        let view = AutomationPanelView::default();
        assert_eq!(view.left_panel_width(), view.base.left_panel_width);
    }

    #[test]
    fn test_panel_height_constants() {
        assert_eq!(DEFAULT_PANEL_HEIGHT, 80.0);
        assert_eq!(MIN_PANEL_HEIGHT, 40.0);
        assert_eq!(MAX_PANEL_HEIGHT, 200.0);
    }
}
