//! Unified selection model for notes.
//!
//! Replaces the old `HashSet<(u16, u32, u8)>` with a compact representation:
//! a list of rectangular ranges in (tick, key, track) space.
//! A note is selected iff it falls within at least one rectangle.
//!
//! Memory: 1000 万音符的矩形选择 = 1 个 rect (~40 bytes) vs 800MB HashSet.

/// Unified selection model for notes.
#[derive(Clone, Default)]
pub struct Selection {
    /// Rectangular ranges: (tick_start, tick_end, key_lo, key_hi, track_lo, track_hi).
    /// tick_end is exclusive (half-open range). track_lo..=track_hi inclusive.
    pub rects: Vec<(u32, u32, u8, u8, u16, u16)>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    /// Add a rect with full (tick, key, track) range.
    /// Defaults track_lo=0, track_hi=65535 (match all tracks).
    pub fn add_rect(&mut self, tick_start: u32, tick_end: u32, key_lo: u8, key_hi: u8) {
        self.add_rect_track(tick_start, tick_end, key_lo, key_hi, 0, u16::MAX);
    }

    /// Add a rect with explicit track range.
    pub fn add_rect_track(
        &mut self,
        tick_start: u32,
        tick_end: u32,
        key_lo: u8,
        key_hi: u8,
        track_lo: u16,
        track_hi: u16,
    ) {
        if tick_end > tick_start {
            self.rects.push((tick_start, tick_end, key_lo, key_hi, track_lo, track_hi));
        }
    }

    /// Check if a specific note is selected.
    pub fn contains(&self, track: u16, start_tick: u32, key: u8) -> bool {
        self.rects.iter().any(|&(ts, te, kl, kh, tl, th)| {
            track >= tl && track <= th && key >= kl && key <= kh && start_tick >= ts && start_tick < te
        })
    }

    /// Number of rects (for undo snapshot size estimation).
    pub fn len(&self) -> usize {
        self.rects.len()
    }

    /// Offset all rects by (delta_ticks, delta_keys).
    /// Clamps key to [0, 127], tick to >= 0. Track range unchanged.
    pub fn offset(&mut self, delta_ticks: i64, delta_keys: i32) {
        for rect in &mut self.rects {
            let (ts, te, kl, kh, tl, th) = *rect;
            let new_ts = (ts as i64 + delta_ticks).max(0) as u32;
            let new_te = (te as i64 + delta_ticks).max(0) as u32;
            let new_kl = (kl as i32 + delta_keys).clamp(0, 127) as u8;
            let new_kh = (kh as i32 + delta_keys).clamp(0, 127) as u8;
            if new_te > new_ts {
                *rect = (new_ts, new_te, new_kl, new_kh, tl, th);
            }
        }
    }

    /// Offset only the tick range of all rects (used by AR arrange drag).
    pub fn offset_ticks(&mut self, delta_ticks: i64) {
        for rect in &mut self.rects {
            let (ts, te, kl, kh, tl, th) = *rect;
            let new_ts = (ts as i64 + delta_ticks).max(0) as u32;
            let new_te = (te as i64 + delta_ticks).max(0) as u32;
            if new_te > new_ts {
                *rect = (new_ts, new_te, kl, kh, tl, th);
            }
        }
    }

    /// Offset only the track range of all rects (used by AR arrange drag).
    pub fn offset_tracks(&mut self, delta_tracks: i32) {
        for rect in &mut self.rects {
            let (ts, te, kl, kh, tl, th) = *rect;
            let new_tl = (tl as i32 + delta_tracks).max(0) as u16;
            let new_th = (th as i32 + delta_tracks).max(0) as u16;
            *rect = (ts, te, kl, kh, new_tl, new_th);
        }
    }

    /// Compute an order-independent XOR hash of all rects (for GPU cache keys).
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0;
        for &(ts, te, kl, kh, tl, th) in &self.rects {
            h ^= (ts as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (te as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (kl as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (kh as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (tl as u64).wrapping_mul(0x9e3779b97f4a7c15);
            h ^= (th as u64).wrapping_mul(0x9e3779b97f4a7c15);
        }
        h
    }
}
