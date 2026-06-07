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
    pub fn sync_from_pianoroll(&mut self, scroll_x: f32, pixels_per_tick: f32, left_panel_width: f32) {
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
}
