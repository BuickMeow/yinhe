/// Haptic feedback engine for trackpad boundary feedback.
///
/// On macOS this uses the private `MultitouchSupport.framework` `MTActuator`
/// API to drive the trackpad Taptic Engine directly.  On other platforms this
/// is a no-op stub.
///
/// The engine tracks per-slot, per-edge flags internally so that haptic is
/// only fired when the user *enters* a boundary, not while staying in one.
/// Each visual view (piano roll, arrangement, etc.) gets its own slot so
/// they don't interfere with each other.

#[cfg(target_os = "macos")]
mod macos;

use std::cell::Cell;

/// Identifies which view is requesting haptic feedback.
/// Each slot maintains independent boundary flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HapticSlot {
    PianoRoll,
    Arrangement,
}

const NUM_SLOTS: usize = 2;

/// Per-slot boundary flags.
struct SlotState {
    // Scroll boundary flags
    at_top: Cell<bool>,
    at_bottom: Cell<bool>,
    at_left: Cell<bool>,
    at_right: Cell<bool>,
    // Zoom boundary flags
    at_zoom_max_x: Cell<bool>,
    at_zoom_min_x: Cell<bool>,
    at_zoom_max_y: Cell<bool>,
    at_zoom_min_y: Cell<bool>,
}

impl SlotState {
    const fn new() -> Self {
        Self {
            at_top: Cell::new(false),
            at_bottom: Cell::new(false),
            at_left: Cell::new(false),
            at_right: Cell::new(false),
            at_zoom_max_x: Cell::new(false),
            at_zoom_min_x: Cell::new(false),
            at_zoom_max_y: Cell::new(false),
            at_zoom_min_y: Cell::new(false),
        }
    }
}

/// Configuration and platform state for haptic feedback.
pub struct HapticEngine {
    enabled: bool,
    intensity: f32, // 0.0..=1.0

    /// Per-slot boundary flags.
    slots: [SlotState; NUM_SLOTS],

    #[cfg(target_os = "macos")]
    performer: macos::HapticPerformer,
}

fn slot_index(slot: HapticSlot) -> usize {
    match slot {
        HapticSlot::PianoRoll => 0,
        HapticSlot::Arrangement => 1,
    }
}

impl HapticEngine {
    /// Create a new haptic engine.
    ///
    /// **Must be called from the main thread** on macOS.
    pub fn new() -> Self {
        Self {
            enabled: true,
            intensity: 0.5,
            slots: [SlotState::new(), SlotState::new()],
            #[cfg(target_os = "macos")]
            performer: macos::HapticPerformer::new(),
        }
    }

    /// Notify the engine of the current scroll state.
    ///
    /// `max_scroll_x` / `max_scroll_y` are the maximum scroll values for the
    /// current view.  If either is `<= 0.0` the corresponding axis is
    /// considered to have no scrollable range and will not trigger haptics.
    ///
    /// The haptic motor fires **only when a new edge is entered** (transition
    /// from `false` → `true`).  If the user stays on a boundary the flag is
    /// already `true` and nothing happens.  When the user scrolls away the
    /// flag is automatically cleared for the next entry.
    pub fn notify_boundary(
        &self,
        slot: HapticSlot,
        old_scroll_x: f32,
        old_scroll_y: f32,
        new_scroll_x: f32,
        new_scroll_y: f32,
        max_scroll_x: f32,
        max_scroll_y: f32,
        raw_scroll_delta: (f32, f32),
    ) {
        if !self.enabled {
            return;
        }
        let (dx, dy) = raw_scroll_delta;
        if dx == 0.0 && dy == 0.0 {
            return;
        }

        let idx = slot_index(slot);
        let s = &self.slots[idx];

        // Determine which edges were hit.
        // Skip axes that have no scrollable range.
        let at_left = max_scroll_x > 0.0 && dx < 0.0 && old_scroll_x == new_scroll_x;
        let at_right = max_scroll_x > 0.0 && dx > 0.0 && old_scroll_x == new_scroll_x;
        let at_top = max_scroll_y > 0.0 && dy < 0.0 && old_scroll_y == new_scroll_y;
        let at_bottom = max_scroll_y > 0.0 && dy > 0.0 && old_scroll_y == new_scroll_y;

        let fire = (!s.at_top.get() && at_top)
            || (!s.at_bottom.get() && at_bottom)
            || (!s.at_left.get() && at_left)
            || (!s.at_right.get() && at_right);

        s.at_top.set(at_top);
        s.at_bottom.set(at_bottom);
        s.at_left.set(at_left);
        s.at_right.set(at_right);

        if fire {
            #[cfg(target_os = "macos")]
            {
                self.performer.perform(self.intensity);
            }
            #[cfg(not(target_os = "macos"))]
            {}
        }
    }

    /// Notify the engine of the current zoom state.
    ///
    /// `zoom_x` / `zoom_y` are the current zoom values; `min_x` / `max_x` /
    /// `min_y` / `max_y` are the allowed range.  Haptic fires on entry into
    /// any zoom limit.
    pub fn notify_zoom_boundary(
        &self,
        slot: HapticSlot,
        zoom_x: f32,
        zoom_y: f32,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) {
        if !self.enabled {
            return;
        }
        let idx = slot_index(slot);
        let s = &self.slots[idx];

        let at_min_x = zoom_x <= min_x;
        let at_max_x = zoom_x >= max_x;
        let at_min_y = zoom_y <= min_y;
        let at_max_y = zoom_y >= max_y;

        let fire = (!s.at_zoom_min_x.get() && at_min_x)
            || (!s.at_zoom_max_x.get() && at_max_x)
            || (!s.at_zoom_min_y.get() && at_min_y)
            || (!s.at_zoom_max_y.get() && at_max_y);

        s.at_zoom_min_x.set(at_min_x);
        s.at_zoom_max_x.set(at_max_x);
        s.at_zoom_min_y.set(at_min_y);
        s.at_zoom_max_y.set(at_max_y);

        if fire {
            #[cfg(target_os = "macos")]
            {
                self.performer.perform(self.intensity);
            }
            #[cfg(not(target_os = "macos"))]
            {}
        }
    }

    // ── Settings ──

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Set intensity (0.0 = lightest, 1.0 = strongest).
    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.clamp(0.0, 1.0);
    }

    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// Apply settings from the persistent config.
    pub fn apply_settings(&mut self, enabled: bool, intensity: f32) {
        self.enabled = enabled;
        self.intensity = intensity.clamp(0.0, 1.0);
    }
}

impl Default for HapticEngine {
    fn default() -> Self {
        Self::new()
    }
}
