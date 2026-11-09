//! 音高轴布局：键数（88/128/256）、起始键与调式折叠。
//!
//! 折叠开启时，调内音每键占一行，调外音压成 `THIN_RATIO` 行；
//! 调外音仍保留（渲染成贴着边界细条），不丢音符。
//!
//! 这里只做纯几何（单位 = 调内一行的高度），像素换算由调用方乘
//! `key_height`。GPU 侧 `shader.wgsl` / `cull.wgsl` 的 `pr_units_before`
//! 与 `units_before` 必须保持同一公式。

/// 88 键钢琴的最低键：A0 = 21。
pub const PIANO88_START: u16 = 21;
/// 调外音行高（相对调内一行）。
pub const THIN_RATIO: f32 = 0.18;

/// 音高轴布局。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyLayout {
    /// 起始键（88 键 = 21，其余 = 0）。
    pub start: u16,
    /// 键数（88 / 128 / 256）。
    pub count: u16,
    /// 12 位音级掩码（bit i = pc i 在调内）。0 = 关闭折叠。
    pub fold_mask: u16,
}

impl Default for KeyLayout {
    fn default() -> Self {
        Self {
            start: 0,
            count: 128,
            fold_mask: 0,
        }
    }
}

impl KeyLayout {
    /// 按键数构造（88 / 128 / 256），关闭折叠。
    pub fn for_key_count(count: u16) -> Self {
        Self {
            start: if count == 88 { PIANO88_START } else { 0 },
            count,
            fold_mask: 0,
        }
    }

    #[inline]
    pub fn end(&self) -> u16 {
        self.start.saturating_add(self.count).saturating_sub(1)
    }

    #[inline]
    pub fn fold_on(&self) -> bool {
        self.fold_mask != 0
    }

    /// 折叠后 `key` 之前累计的行单位高度（绝对键号，含起始偏移）。
    pub fn units_before(&self, key: u16) -> f32 {
        if !self.fold_on() {
            return key as f32;
        }
        let k = key as u32;
        let pc = (k % 12) as u16;
        let oct = (k / 12) as f32;
        let n_in = self.fold_mask.count_ones() as f32;
        let per_oct = n_in + (12.0 - n_in) * THIN_RATIO;
        let before = if pc == 0 {
            0
        } else {
            (self.fold_mask & ((1u16 << pc) - 1)).count_ones()
        };
        oct * per_oct + before as f32 + (pc as f32 - before as f32) * THIN_RATIO
    }

    /// `key` 这一行的高度（单位）。
    pub fn row_units(&self, key: u16) -> f32 {
        if !self.fold_on() {
            return 1.0;
        }
        let pc = key % 12;
        if self.fold_mask & (1u16 << pc) != 0 {
            1.0
        } else {
            THIN_RATIO
        }
    }

    /// 整个音高范围的总高度（单位）。
    pub fn total_units(&self) -> f32 {
        if !self.fold_on() {
            return self.count as f32;
        }
        self.units_before(self.end().saturating_add(1)) - self.units_before(self.start)
    }

    /// `key` 行顶部的像素 y（相对内容区顶部；高音在上）。
    pub fn key_to_y(&self, key: u16, key_height: f32, scroll_y: f32) -> f32 {
        let base = self.units_before(self.start);
        let rel = self.units_before(key) - base;
        let h = self.row_units(key);
        self.total_units() * key_height - scroll_y - (rel + h) * key_height
    }

    /// 像素 y → key（落在哪一行）。`y` 相对内容区顶部。
    pub fn y_to_key(&self, y: f32, key_height: f32, scroll_y: f32) -> u16 {
        if key_height <= 0.0 {
            return self.start;
        }
        let bottom = self.total_units() * key_height - scroll_y;
        let target = (bottom - y) / key_height + self.units_before(self.start);
        // 行数最多 256，线性扫描足够快且不会因细条行做浮点误差较大的反解。
        let mut lo = self.start;
        for key in self.start..=self.end() {
            // `>=`：恰好落在行边界时归给下方那一行（与旧 `ceil-1` 语义一致）。
            if self.units_before(key.saturating_add(1)) >= target {
                return key;
            }
            lo = key;
        }
        lo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c_major() -> KeyLayout {
        KeyLayout {
            start: 0,
            count: 128,
            fold_mask: 0b1010_1011_0101, // C D E F G A B
        }
    }

    #[test]
    fn unfolded_matches_chromatic_rows() {
        let l = KeyLayout::default();
        assert_eq!(l.total_units(), 128.0);
        assert_eq!(l.key_to_y(60, 12.0, 0.0), 128.0 * 12.0 - 61.0 * 12.0);
        assert_eq!(l.y_to_key(l.key_to_y(60, 12.0, 0.0) + 6.0, 12.0, 0.0), 60);
    }

    #[test]
    fn c_major_per_octave_is_7_plus_5_thin() {
        let l = c_major();
        assert!(l.fold_on());
        assert_eq!(l.fold_mask, 0xAB5, "C 大调掩码");
        let per_oct = 7.0 + 5.0 * THIN_RATIO;
        assert!((l.units_before(12) - per_oct).abs() < 1e-4);
        assert!((l.units_before(24) - 2.0 * per_oct).abs() < 1e-4);
    }

    #[test]
    fn folded_roundtrip_every_key() {
        let l = c_major();
        for key in 0..=127u16 {
            let y = l.key_to_y(key, 12.0, 0.0) + l.row_units(key) * 12.0 * 0.5;
            assert_eq!(l.y_to_key(y, 12.0, 0.0), key, "key {key} roundtrip");
        }
    }

    #[test]
    fn piano88_starts_at_a0_and_ends_at_c8() {
        let l = KeyLayout::for_key_count(88);
        assert_eq!(l.start, PIANO88_START);
        assert_eq!(l.end(), 108);
        assert_eq!(l.total_units(), 88.0);
    }

    #[test]
    fn folded_total_is_smaller_and_out_of_scale_rows_are_thin() {
        let l = c_major();
        // 128 键 7/12 调内：约 128 * (7 + 5*0.18)/12 ≈ 84.3
        assert!(l.total_units() < 90.0);
        assert_eq!(l.row_units(1), THIN_RATIO); // C#
        assert_eq!(l.row_units(0), 1.0); // C
    }
}
