//! Unified selection model for notes.
//!
//! Replaces the old `HashSet<(u16, u32, u8)>` with a compact representation:
//! a list of rectangular ranges in (tick, key, track) space.
//! A note is selected iff it falls within at least one rectangle.
//!
//! Memory: 1000 万音符的矩形选择 = 1 个 rect (~40 bytes) vs 800MB HashSet.

use yinhe_types::MAX_KEY;

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
            self.rects
                .push((tick_start, tick_end, key_lo, key_hi, track_lo, track_hi));
        }
    }

    /// Check if a specific note is selected.
    pub fn contains(&self, track: u16, start_tick: u32, key: u8) -> bool {
        self.rects.iter().any(|&(ts, te, kl, kh, tl, th)| {
            track >= tl
                && track <= th
                && key >= kl
                && key <= kh
                && start_tick >= ts
                && start_tick < te
        })
    }

    /// Number of rects (for undo snapshot size estimation).
    pub fn len(&self) -> usize {
        self.rects.len()
    }

    /// Offset all rects by (delta_ticks, delta_keys).
    /// Clamps key to [0, MAX_KEY], tick to >= 0. Track range unchanged.
    pub fn offset(&mut self, delta_ticks: i64, delta_keys: i32) {
        for rect in &mut self.rects {
            let (ts, te, kl, kh, tl, th) = *rect;
            let new_ts = (ts as i64 + delta_ticks).max(0) as u32;
            let new_te = (te as i64 + delta_ticks).max(0) as u32;
            let new_kl = (kl as i32 + delta_keys).clamp(0, MAX_KEY as i32) as u8;
            let new_kh = (kh as i32 + delta_keys).clamp(0, MAX_KEY as i32) as u8;
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

    /// Remove rects matching the given PR selection-box rects
    /// `(tick_start, tick_end, key_lo, key_hi)`. Used by cross-view selection
    /// exclusivity (PR/AR/AM 三视图选框互斥).
    pub fn remove_rects(&mut self, rects: &[(u32, u32, u8, u8)]) {
        self.rects.retain(|r| {
            !rects
                .iter()
                .any(|q| q.0 == r.0 && q.1 == r.1 && q.2 == r.2 && q.3 == r.3)
        });
    }

    /// Remove rects matching the given AR selection-box rects
    /// `(tick_start, tick_end, track_lo, track_hi)`.
    ///
    /// AR 的 rect 在 Selection 中总是 key 全范围 (kl=0, kh=MAX_KEY)，据此匹配避免误伤 PR 的 rect。
    pub fn remove_rects_track(&mut self, rects: &[(u32, u32, u16, u16)]) {
        self.rects.retain(|r| {
            !(r.2 == 0
                && r.3 == MAX_KEY
                && rects
                    .iter()
                    .any(|q| q.0 == r.0 && q.1 == r.1 && q.2 == r.4 && q.3 == r.5))
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_rects_only_matches_exact_pr_rects() {
        let mut sel = Selection::default();
        // 两个 PR 矩形（key 局部范围）+ 一个 AR 矩形（key 全范围）
        sel.add_rect(0, 100, 60, 70); // PR
        sel.add_rect(200, 300, 40, 50); // PR
        sel.add_rect_track(0, 100, 0, MAX_KEY, 3, 5); // AR

        sel.remove_rects(&[(0, 100, 60, 70)]);

        // 只删掉第一个 PR 矩形，其余保留（用不重叠的 tick/key 验证）
        assert_eq!(sel.rects.len(), 2);
        assert!(!sel.contains(0, 50, 65));
        assert!(sel.contains(0, 250, 45));
        assert!(sel.contains(3, 50, 65));
    }

    #[test]
    fn remove_rects_track_removes_ar_rects_not_pr() {
        let mut sel = Selection::default();
        sel.add_rect(0, 100, 60, 70); // PR：key 局部范围，必须保留
        sel.add_rect_track(0, 100, 0, MAX_KEY, 3, 5); // AR：命中 track 3..=5
        sel.add_rect_track(0, 100, 0, MAX_KEY, 7, 9); // AR：不命中 track，保留

        sel.remove_rects_track(&[(0, 100, 3, 5)]);

        assert_eq!(sel.rects.len(), 2);
        assert!(sel.contains(0, 50, 65)); // PR 矩形不受影响
        assert!(!sel.contains(3, 50, 80)); // 命中的 AR 矩形被删除（key 80 避开 PR 矩形范围）
        assert!(sel.contains(7, 50, 80)); // 未命中的 AR 矩形保留
    }

    #[test]
    fn remove_rects_track_matches_full_tick_range_only() {
        let mut sel = Selection::default();
        sel.add_rect_track(0, 100, 0, MAX_KEY, 0, 0); // tick 范围相同
        sel.add_rect_track(50, 150, 0, MAX_KEY, 0, 0); // tick 不同，保留

        sel.remove_rects_track(&[(0, 100, 0, 0)]);

        assert_eq!(sel.rects.len(), 1);
        assert!(!sel.contains(0, 10, 64)); // tick 10 只属于被删的矩形
        assert!(sel.contains(0, 60, 64)); // tick 60 只属于保留的矩形
    }
}
