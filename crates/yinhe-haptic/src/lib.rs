/// Haptic feedback engine for trackpad boundary feedback.
///
/// On macOS this uses the private `MultitouchSupport.framework` `MTActuator`
/// API to drive the trackpad Taptic Engine directly.  On other platforms this
/// is a no-op stub.
///
/// The engine tracks per-edge flags internally so that haptic is only fired
/// when the user *enters* a boundary, not while staying in one.  Flags are
/// automatically cleared when the user scrolls away from the boundary.

#[cfg(target_os = "macos")]
mod macos;

use std::cell::Cell;

/// Configuration and platform state for haptic feedback.
pub struct HapticEngine {
    enabled: bool,
    intensity: f32, // 0.0..=1.0

    // Per-edge "already at boundary" flags — prevent repeated buzzing.
    at_top: Cell<bool>,
    at_bottom: Cell<bool>,
    at_left: Cell<bool>,
    at_right: Cell<bool>,

    #[cfg(target_os = "macos")]
    performer: macos::HapticPerformer,
}

impl HapticEngine {
    /// Create a new haptic engine.
    ///
    /// On macOS this lazily initializes the `NSHapticFeedbackPerformer`.
    /// On other platforms this is a no-op.
    ///
    /// **Must be called from the main thread** on macOS.
    pub fn new() -> Self {
        Self {
            enabled: true,
            intensity: 0.5,
            at_top: Cell::new(false),
            at_bottom: Cell::new(false),
            at_left: Cell::new(false),
            at_right: Cell::new(false),
            #[cfg(target_os = "macos")]
            performer: macos::HapticPerformer::new(),
        }
    }

    /// Notify the engine of the current scroll-boundary state.
    ///
    /// The haptic motor fires **only when a new edge is entered** (transition
    /// from `false` → `true`).  If the user stays on a boundary the flag is
    /// already `true` and nothing happens.  When the user scrolls away the
    /// flag is automatically cleared for the next entry.
    ///
    /// Call this every frame after clamping scroll, passing the result of
    /// your edge-detection check.
    pub fn notify_boundary(&self, at_top: bool, at_bottom: bool, at_left: bool, at_right: bool) {
        if !self.enabled {
            return;
        }

        let fire = (!self.at_top.get() && at_top)
            || (!self.at_bottom.get() && at_bottom)
            || (!self.at_left.get() && at_left)
            || (!self.at_right.get() && at_right);

        self.at_top.set(at_top);
        self.at_bottom.set(at_bottom);
        self.at_left.set(at_left);
        self.at_right.set(at_right);

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
    ///
    /// On macOS this is mapped to the `NSHapticFeedbackPattern` — currently
    /// we always use `Alignment` which has a fixed strength, but the
    /// intensity value is stored for future use (e.g. calling the pattern
    /// multiple times in quick succession for stronger feedback).
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
