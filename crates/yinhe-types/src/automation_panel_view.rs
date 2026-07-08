use crate::{AutomationTarget, TimelineViewBase};

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
    /// When `show_velocity` is true, this field is ignored for data rendering.
    pub selected_target: AutomationTarget,
    /// When true, render velocity bars from note data instead of automation lanes.
    pub show_velocity: bool,
    /// When true, render tempo curve from conductor tempo events.
    pub show_tempo: bool,
    /// Cached index into `MidiFile.automation_lanes` for fast lookup.
    pub lane_index: usize,
    /// Whether the panel content needs to be rebuilt.
    pub dirty: bool,
    /// 垂直缩放系数。1.0 = 满量程（0~max_val）映射到面板高度。
    /// > 1.0 = 放大（只显示部分值范围），< 1.0 = 缩小（显示更宽范围）。
    pub value_zoom: f32,
    /// 垂直滚动偏移（值空间单位，如 CC 的 0~127）。
    /// 面板顶部对应的值 = `value_scroll`。
    pub value_scroll: f32,
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
            selected_target: AutomationTarget::CC { controller: 7 },
            show_velocity: true,
            show_tempo: false,
            lane_index: 0,
            dirty: true,
            value_zoom: 1.0,
            value_scroll: 0.0,
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
        let h = crate::hash::hash_f32s(&[
            self.base.pixels_per_tick,
            self.base.scroll_x,
            self.base.left_panel_width,
            self.panel_height,
            self.value_zoom,
            self.value_scroll,
        ]);
        h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(self.show_tempo as u64)
    }

    /// 将自动化值转换为面板局部 Y 坐标（像素，0=顶部）。
    /// `max_val` = 当前 target 的最大值（如 CC 的 127）。
    #[inline]
    pub fn value_to_y(&self, value: f32, max_val: f32) -> f32 {
        let visible_range = max_val / self.value_zoom;
        if visible_range <= 0.0 {
            return 0.0;
        }
        let h = self.panel_height;
        h - ((value - self.value_scroll) / visible_range) * h
    }

    /// 将面板局部 Y 坐标（像素，0=顶部）转换回自动化值。
    /// `max_val` = 当前 target 的最大值。
    #[inline]
    pub fn y_to_value(&self, y: f32, max_val: f32) -> f32 {
        let visible_range = max_val / self.value_zoom;
        if visible_range <= 0.0 {
            return 0.0;
        }
        let h = self.panel_height;
        self.value_scroll + (1.0 - y / h) * visible_range
    }

    /// 根据 max_val 限制 value_scroll 的范围，防止滚出有效区间。
    pub fn clamp_value_scroll(&mut self, max_val: f32) {
        let visible_range = max_val / self.value_zoom;
        let max_scroll = (max_val - visible_range).max(0.0);
        self.value_scroll = self.value_scroll.clamp(0.0, max_scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let view = AutomationPanelView::default();
        assert_eq!(view.panel_height, DEFAULT_PANEL_HEIGHT);
        assert!(view.show_velocity);
        assert!(!view.show_tempo);
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
