//! 选中高亮的「排除表」：成员态下矩形内非成员音符的 GPU 侧修正。
//!
//! GPU 的选中判定 = `矩形命中 && !排除表命中 && 属性筛选通过`。
//! 属性筛选由 shader 用 `SelectionFilter` 字段判定；排除表解决成员态
//! （Alt 复制跟随、重叠框选物化等）下"矩形内的路人音符被误染"。
//! 矩形态下矩形即精确，排除表为空。

use yinhe_types::NoteSource;

use crate::selection::Selection;

/// 排除表键打包：`lo = key | track<<8`，`hi = start_tick`。
#[inline]
pub fn exclude_key(track: u16, key: u8, start_tick: u32) -> (u32, u32) {
    (key as u32 | ((track as u32) << 8), start_tick)
}

/// 排除表哈希（CPU/GPU 必须一致；u32 乘法 wrapping）。
#[inline]
pub fn exclude_hash(lo: u32, hi: u32) -> u32 {
    lo.wrapping_mul(2_654_435_761) ^ hi.wrapping_mul(2_246_822_519)
}

/// 构建选中高亮的排除表：矩形空间命中但**不属于**选中成员集合的音符。
///
/// 矩形态（无显式成员）返回空表。返回打包键见 [`exclude_key`]。
pub fn build_selection_exclude_set(
    source: &dyn NoteSource,
    selection: &Selection,
) -> Vec<(u32, u32)> {
    if !selection.has_explicit_members() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (k, ranges) in selection.merged_tick_ranges_by_key().iter().enumerate() {
        if ranges.is_empty() {
            continue;
        }
        let key = k as u8;
        for &(lo, hi) in ranges {
            for n in source.key_notes_in_range(key, lo, hi) {
                // `key_notes_in_range` 左边界按 max_note_len 保守外扩，需精确过滤。
                if n.start_tick < lo || n.start_tick >= hi {
                    continue;
                }
                // 空间命中（矩形）但不在成员位图 → 需从高亮中排除。
                if selection.contains(n.track, n.start_tick, key)
                    && !selection.members_contains(n.id)
                {
                    out.push(exclude_key(n.track, key, n.start_tick));
                }
            }
        }
    }
    out
}

/// 把排除集合构建为 GPU 开放寻址表（`array<vec2<u32>>`，空槽 = `u32::MAX`）。
///
/// 容量 = 表项数 ×2 向上取 2 的幂（负载 ≤ 0.5）；返回 `(表, 掩码)`，
/// 空集合返回 `(空表, 0)`（shader 侧掩码 0 = 禁用查表）。
pub fn build_exclude_table_gpu(keys: &[(u32, u32)]) -> (Vec<[u32; 2]>, u32) {
    if keys.is_empty() {
        return (Vec::new(), 0);
    }
    let cap = (keys.len() * 2).next_power_of_two().max(8);
    let mask = (cap - 1) as u32;
    let mut table = vec![[u32::MAX, u32::MAX]; cap];
    for &(lo, hi) in keys {
        let mut slot = (exclude_hash(lo, hi) & mask) as usize;
        while table[slot][0] != u32::MAX {
            slot = (slot + 1) & mask as usize;
        }
        table[slot] = [lo, hi];
    }
    (table, mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::events::NoteEvent;
    use crate::model::{TrackData, YinModel};

    /// 3 个音符：k60 两条（tick 0 / 480）、k64 一条（tick 0）。
    fn model_with_notes() -> YinModel {
        let per_track = vec![vec![
            NoteEvent {
                id: 1,
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
            },
            NoteEvent {
                id: 2,
                start_tick: 480,
                end_tick: 960,
                key: 60,
                velocity: 80,
            },
            NoteEvent {
                id: 3,
                start_tick: 0,
                end_tick: 240,
                key: 64,
                velocity: 100,
            },
        ]];
        let mut m = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        m.load_track_notes(per_track);
        m.rebuild();
        m
    }

    /// 排除表：矩形态为空；成员态只含「空间命中但非成员」的音符。
    #[test]
    fn exclude_set_members_only() {
        let m = model_with_notes();
        let mut sel = Selection::default();
        sel.add_rect(0, u32::MAX, 0, yinhe_types::MAX_KEY);
        assert!(
            build_selection_exclude_set(&m, &sel).is_empty(),
            "矩形态矩形即精确 → 空表"
        );

        // 成员态：只选 id=2 → 其余两条（id1 k60、id3 k64）进排除表。
        sel.set_members([2]);
        let set = build_selection_exclude_set(&m, &sel);
        assert_eq!(set.len(), 2);
        assert!(set.contains(&exclude_key(0, 60, 0)), "id1 应排除");
        assert!(set.contains(&exclude_key(0, 64, 0)), "id3 应排除");
        assert!(
            !set.contains(&exclude_key(0, 60, 480)),
            "成员 id2 不进排除表"
        );

        sel.set_members([1, 2, 3]);
        assert!(
            build_selection_exclude_set(&m, &sel).is_empty(),
            "全成员 → 空表"
        );
    }

    /// GPU 表：开放寻址插入后能按同一哈希函数找回全部键、不误报。
    #[test]
    fn exclude_table_gpu_lookup_roundtrip() {
        let keys = vec![
            exclude_key(0, 60, 0),
            exclude_key(1, 62, 480),
            exclude_key(0, 64, 960),
        ];
        let (table, mask) = build_exclude_table_gpu(&keys);
        assert!(mask > 0 && (mask as usize + 1).is_power_of_two());
        for &(lo, hi) in &keys {
            let mut slot = (exclude_hash(lo, hi) & mask) as usize;
            let mut found = false;
            for _ in 0..=mask {
                let e = table[slot];
                if e[0] == u32::MAX {
                    break;
                }
                if e == [lo, hi] {
                    found = true;
                    break;
                }
                slot = (slot + 1) & mask as usize;
            }
            assert!(found, "键 ({lo},{hi}) 应可找回");
        }
        // 不存在的键不得误报。
        let (lo, hi) = exclude_key(9, 99, 12345);
        let mut slot = (exclude_hash(lo, hi) & mask) as usize;
        let mut found = false;
        for _ in 0..=mask {
            let e = table[slot];
            if e[0] == u32::MAX {
                break;
            }
            if e == [lo, hi] {
                found = true;
                break;
            }
            slot = (slot + 1) & mask as usize;
        }
        assert!(!found);
        assert_eq!(build_exclude_table_gpu(&[]).1, 0, "空集合掩码为 0");
    }
}
