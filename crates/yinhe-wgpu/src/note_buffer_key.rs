//! Compile-time-enforced note buffer cache key.
//!
//! ALL inputs that affect the GPU note buffer must be passed to
//! `NoteBufferKey::new()`. If you add a field, the compiler tells you
//! every construction site that needs updating — no more "forgot to
//! include X in the cache key" bugs.

use yinhe_types::HiddenFilter;

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
    pub fn new(revision: u64, track_visible: &[bool], hidden_notes: &dyn HiddenFilter) -> Self {
        Self::from_hashes(
            revision,
            hash_bools(track_visible),
            hash_hidden(hidden_notes),
        )
    }

    /// Compute the cache key from precomputed hashes.
    ///
    /// Lets callers that already hashed `track_visible`/`hidden_notes`
    /// (e.g. for mask sync or another cache key) reuse the values instead
    /// of hashing the same inputs again.
    pub fn from_hashes(revision: u64, tv_hash: u64, hidden_hash: u64) -> Self {
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
pub fn hash_hidden(hidden_notes: &dyn HiddenFilter) -> u64 {
    hidden_notes.hidden_hash()
}
