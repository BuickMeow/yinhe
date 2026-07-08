use crate::TimeSigEvent;

/// Hash time signature events using position-sensitive folding.
/// Order matters: reordering events changes the hash.
pub fn hash_time_sigs(events: &[TimeSigEvent]) -> u64 {
    let mut h: u64 = 0;
    for ev in events {
        h = h.wrapping_mul(31).wrapping_add(ev.tick as u64);
        h = h.wrapping_mul(31).wrapping_add(ev.numerator as u64);
        h = h.wrapping_mul(31).wrapping_add(ev.denominator as u64);
    }
    h
}

/// Compute `(scroll_x_pos, scroll_frac)` from raw scroll_x and scroll_mode.
///
/// - `scroll_mode == 0`: raw scroll, returns `(scroll_x, 0.0)`
/// - `scroll_mode != 0`: integer-aligned scroll, returns
///   `(scroll_x.floor(), scroll_x - scroll_x.floor())`
pub fn compute_scroll_frac(scroll_x: f32, scroll_mode: u32) -> (f32, f32) {
    match scroll_mode {
        0 => (scroll_x, 0.0),
        _ => {
            let f = scroll_x.floor();
            (f, scroll_x - f)
        }
    }
}

/// Hash a sequence of f64 values using position-sensitive folding.
/// Order matters: `hash_f64s(&[1.0, 2.0]) != hash_f64s(&[2.0, 1.0])`.
pub fn hash_f64s(values: &[f64]) -> u64 {
    let mut h: u64 = 0;
    for &v in values {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v.to_bits() as u64);
    }
    h
}

/// Hash a sequence of f32 values using position-sensitive folding.
pub fn hash_f32s(values: &[f32]) -> u64 {
    let mut h: u64 = 0;
    for &v in values {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v.to_bits() as u64);
    }
    h
}

/// Hash a sequence of bool values using position-sensitive folding.
pub fn hash_bools(values: &[bool]) -> u64 {
    let mut h: u64 = 0;
    for &v in values {
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
    }
    h
}
