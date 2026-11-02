//! 拖动预览时“原位置隐藏”的音符集合。
//!
//! 移动端用按 note id 索引的分页位图（[`HiddenNotes`]）：大选区（百万级）
//! 只占几十 KB，且 hash/count 是 O(1) 维护，不再在 UI 线程遍历 HashSet。
//!
//! 桌面 egui 仍以 `(track, start_tick, key)` 位置集合收集隐藏音符；两者通过
//! [`HiddenFilter`] 统一给 wgpu 的实例构建使用，避免渲染层出现两套路径。

use std::collections::HashSet;

use crate::note::Note;
use crate::note_bitset::{NoteBitset, NoteBitsetBuilder};

/// 音高数量（hidden 的 key 掩码按它取位）。
const KEY_COUNT: usize = 128;
const KEY_WORDS: usize = KEY_COUNT.div_ceil(64);

/// 渲染层判断“这个音符当前是否被拖动 ghost 替代”。
pub trait HiddenFilter: Send + Sync {
    /// `key` 是音符所在音高桶（位置集合需要它定位；id 位图忽略）。
    fn is_hidden(&self, note: &Note, key: u8) -> bool;
    /// 隐藏音符数量（缓存失效判定用）。
    fn hidden_count(&self) -> u64;
    /// 内容指纹：内容相同必须得到相同值，空集合必须为 0。
    fn hidden_hash(&self) -> u64;
    fn is_empty_hidden(&self) -> bool {
        self.hidden_count() == 0
    }
}

/// 位置集合（桌面 egui 路径）：按 `(track, start_tick, key)` 判定。
impl HiddenFilter for HashSet<(u16, u32, u8)> {
    fn is_hidden(&self, note: &Note, key: u8) -> bool {
        self.contains(&(note.track, note.start_tick, key))
    }

    fn hidden_count(&self) -> u64 {
        self.len() as u64
    }

    fn hidden_hash(&self) -> u64 {
        // 顺序无关的内容 hash（与旧 `yinhe_wgpu::hash_hidden` 相同算法）。
        self.iter().fold(0u64, |acc, &(trk, tick, key)| {
            acc.wrapping_add(u64::from(trk))
                .wrapping_mul(6364136223846793005)
                .wrapping_add(u64::from(tick))
                .wrapping_mul(6364136223846793005)
                .wrapping_add(u64::from(key))
        })
    }
}

fn mix_id(id: u32) -> u64 {
    u64::from(id).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// 按 note id 位图表示的隐藏集合（移动端）。
#[derive(Clone, Default)]
pub struct HiddenNotes {
    bits: NoteBitset,
    keys: [u64; KEY_WORDS],
    hash: u64,
    count: u64,
}

impl HiddenNotes {
    /// 只隐藏一个音符（单音符拖动）。
    pub fn single(id: u32, key: u8) -> Self {
        let mut out = Self::default();
        out.insert(id, key);
        out
    }

    /// 加入一个隐藏音符；重复 id 不会重复计数（id 0 忽略）。
    pub fn insert(&mut self, id: u32, key: u8) {
        if id == 0 {
            return;
        }
        if self.bits.insert(id) {
            self.count += 1;
            self.hash = self.hash.wrapping_add(mix_id(id));
        }
        self.keys[key as usize / 64] |= 1u64 << (key % 64);
    }

    pub fn contains_id(&self, id: u32) -> bool {
        self.bits.contains(id)
    }

    /// 含隐藏音符的音高桶掩码（GPU 增量上传判定用）。
    pub fn key_mask(&self) -> [u64; KEY_WORDS] {
        self.keys
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn len(&self) -> u64 {
        self.count
    }
}

impl HiddenFilter for HiddenNotes {
    fn is_hidden(&self, note: &Note, _key: u8) -> bool {
        self.bits.contains(note.id)
    }

    fn hidden_count(&self) -> u64 {
        self.count
    }

    fn hidden_hash(&self) -> u64 {
        self.hash
    }
}

/// 流式构建位图（百万级插入不需要逐次 BTreeMap 查找）。
#[derive(Default)]
pub struct HiddenNotesBuilder {
    bits: NoteBitsetBuilder,
    keys: [u64; KEY_WORDS],
    hash: u64,
    count: u64,
}

impl HiddenNotesBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: u32, key: u8) {
        if id == 0 {
            return;
        }
        if self.bits.insert(id) {
            self.count += 1;
            self.hash = self.hash.wrapping_add(mix_id(id));
        }
        self.keys[key as usize / 64] |= 1u64 << (key % 64);
    }

    pub fn build(self) -> HiddenNotes {
        HiddenNotes {
            bits: self.bits.build(),
            keys: self.keys,
            hash: self.hash,
            count: self.count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32, track: u16, start: u32) -> Note {
        Note {
            id,
            track,
            start_tick: start,
            end_tick: start + 10,
            velocity: 100,
        }
    }

    #[test]
    fn empty_hash_is_zero() {
        assert_eq!(HiddenNotes::default().hidden_hash(), 0);
        assert!(HiddenNotes::default().is_empty());
    }

    #[test]
    fn hash_is_content_stable_and_order_independent() {
        let mut a = HiddenNotesBuilder::new();
        a.insert(10, 60);
        a.insert(11, 61);
        let a = a.build();
        let mut b = HiddenNotesBuilder::new();
        b.insert(11, 61);
        b.insert(10, 60);
        let b = b.build();
        assert_eq!(a.hidden_hash(), b.hidden_hash());
        assert_eq!(a.hidden_count(), 2);
        // 重复插入不改变计数/hash。
        let mut c = HiddenNotes::default();
        c.insert(10, 60);
        c.insert(10, 60);
        assert_eq!(c.hidden_count(), 1);
        assert_eq!(c.hidden_hash(), HiddenNotes::single(10, 60).hidden_hash());
    }

    #[test]
    fn id_bitset_hides_only_matching_note() {
        let mut hidden = HiddenNotes::default();
        hidden.insert(42, 60);
        let hit = note(42, 1, 100);
        let other = note(43, 1, 100);
        assert!(hidden.is_hidden(&hit, 60));
        assert!(!hidden.is_hidden(&other, 60));
        assert_eq!(hidden.key_mask(), {
            let mut mask = [0u64; KEY_WORDS];
            mask[60 / 64] |= 1 << 60;
            mask
        });
    }
}
