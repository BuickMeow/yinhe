//! macOS implementation that drives the trackpad Taptic Engine directly via
//! the private `MultitouchSupport.framework` `MTActuator*` APIs.
//!
//! The public `NSHapticFeedbackPerformer` API is intentionally NOT used here:
//! Apple documents that it silently suppresses feedback unless the user is
//! actively touching the trackpad and the action is a recognized "alignment"
//! gesture.  Inertial scroll-to-boundary is not such a gesture, so it never
//! fires.  `MTActuator` bypasses that policy and commands the motor directly.
//!
//! This relies on a private framework — it will NOT pass App Store review,
//! but works fine for self-built / open-source distribution.

use std::ffi::c_void;
use std::os::raw::c_int;

/// Opaque handle returned by `MTActuatorCreateFromDeviceID`.
type MTActuatorRef = *mut c_void;
/// Opaque multitouch device handle.
type MTDeviceRef = *mut c_void;
type CFArrayRef = *const c_void;

#[allow(non_snake_case)]
unsafe extern "C" {
    fn MTActuatorCreateFromDeviceID(device_id: u64) -> MTActuatorRef;
    fn MTActuatorOpen(actuator: MTActuatorRef) -> c_int;
    fn MTActuatorActuate(
        actuator: MTActuatorRef,
        actuation_id: c_int,
        unknown1: u32,
        unknown2: f32,
        unknown3: f32,
    ) -> c_int;
    fn MTActuatorClose(actuator: MTActuatorRef) -> c_int;

    // Device enumeration (fallback to discover the trackpad's ID).
    fn MTDeviceCreateList() -> CFArrayRef;
    fn MTDeviceGetDeviceID(device: MTDeviceRef, device_id: *mut u64) -> c_int;
}

#[allow(non_snake_case)]
unsafe extern "C" {
    fn CFArrayGetCount(array: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: isize) -> *const c_void;
}

/// Candidate device IDs to try, in order.
const CANDIDATE_IDS: &[u64] = &[
    0x7000000000000e6, // Apple Silicon MacBook Air/Pro trackpad (verified)
    0x200000001,       // Older Intel MacBook trackpad (from RE tools)
    0,                 // Default actuator
];

/// Wraps an opened `MTActuator` handle.
pub(crate) struct HapticPerformer {
    actuator: MTActuatorRef,
}

unsafe impl Send for HapticPerformer {}
unsafe impl Sync for HapticPerformer {}

impl HapticPerformer {
    pub(crate) fn new() -> Self {
        let mut actuator = std::ptr::null_mut();

        // Try known device IDs first.
        for &id in CANDIDATE_IDS {
            actuator = unsafe { try_open(id) };
            if !actuator.is_null() {
                break;
            }
        }

        // Fallback: enumerate all multitouch devices.
        if actuator.is_null() {
            actuator = Self::enumerate_devices();
        }

        Self { actuator }
    }

    /// Trigger haptic feedback by commanding the Taptic Engine directly.
    ///
    /// `intensity` selects between predefined waveform IDs:
    ///   < 0.34 → light (id 3), < 0.67 → medium (id 4), else firm (id 6).
    pub(crate) fn perform(&self, intensity: f32) {
        if self.actuator.is_null() {
            return;
        }
        let actuation_id = if intensity < 0.34 {
            3
        } else if intensity < 0.67 {
            4
        } else {
            6
        };
        unsafe {
            MTActuatorActuate(self.actuator, actuation_id, 0, 0.0, 0.0);
        }
    }

    /// Enumerate all multitouch devices and try to open an actuator for each.
    fn enumerate_devices() -> MTActuatorRef {
        unsafe {
            let list = MTDeviceCreateList();
            if list.is_null() {
                return std::ptr::null_mut();
            }
            let count = CFArrayGetCount(list);
            for i in 0..count {
                let dev = CFArrayGetValueAtIndex(list, i) as MTDeviceRef;
                if dev.is_null() {
                    continue;
                }
                let mut id: u64 = 0;
                let rc = MTDeviceGetDeviceID(dev, &mut id);
                if rc == 0 && id != 0 {
                    let a = try_open(id);
                    if !a.is_null() {
                        return a;
                    }
                }
            }
        }
        std::ptr::null_mut()
    }
}

impl Drop for HapticPerformer {
    fn drop(&mut self) {
        if !self.actuator.is_null() {
            unsafe {
                MTActuatorClose(self.actuator);
            }
        }
    }
}

/// Try to create and open an actuator for the given device ID.
/// Returns a non-null handle on success.
unsafe fn try_open(device_id: u64) -> MTActuatorRef {
    let a = unsafe { MTActuatorCreateFromDeviceID(device_id) };
    if a.is_null() {
        return std::ptr::null_mut();
    }
    let rc = unsafe { MTActuatorOpen(a) };
    if rc != 0 {
        unsafe { MTActuatorClose(a) };
        return std::ptr::null_mut();
    }
    a
}
