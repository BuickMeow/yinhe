//! Compile-time-enforced note buffer cache key.
//!
//! ALL inputs that affect the GPU note buffer must be passed to
//! `NoteBufferKey::new()`. If you add a field, the compiler tells you
//! every construction site that needs updating — no more "forgot to
//! include X in the cache key" bugs.

use std::collections::HashSet;

use crate::hash_bools;

/// Cache key for the GPU all-notes buffer.
///
/// Every field here is an input that affects which notes end up in the
/// buffer.  Constructing this struct is the ONLY way to obtain a valid
/// cache key — you cannot accidentally omit an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteBufferKey(u64);

impl NoteBufferKey {
    /// Compute the cache key from all relevant inputs.
    pub fn new(
        revision: u64,
        track_visible: &[bool],
        hidden_notes: &HashSet<(u16, u32, u8)>,
    ) -> Self {
        let tv_hash = hash_bools(track_visible);
        let hidden_hash = hidden_notes.iter().fold(0u64, |acc, &(trk, tick, key)| {
            acc.wrapping_add(trk as u64)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(tick as u64)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(key as u64)
        });
        Self(revision ^ tv_hash ^ hidden_hash)
    }

    /// The raw cache key value for comparison / storage.
    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Compute a hash of the hidden_notes set for cache invalidation.
///
/// Exposed separately so callers can check whether hidden_notes changed
/// independently of revision/track_visible, enabling the upload decision
/// logic to choose between full vs. incremental upload.
pub fn hash_hidden(hidden_notes: &HashSet<(u16, u32, u8)>) -> u64 {
    hidden_notes.iter().fold(0u64, |acc, &(trk, tick, key)| {
        acc.wrapping_add(trk as u64)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(tick as u64)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(key as u64)
    })
}
