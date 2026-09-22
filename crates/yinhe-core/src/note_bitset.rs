//! 按音符 id 索引的分页位图（`Selection` 的显式成员集）。
//!
//! 页号 = `id >> 16`，每页 65536 bit = 8KB，与 `NoteBucket` 的
//! `BUCKET_CHUNK_CAP` 同粒度。稀疏分配：只有出现成员 id 的页才存在，
//! 因此 1000 万成员的位图约 1.25MB（对比 `HashSet<u32>` 的数百 MB）。
//!
//! 页用 `Arc` 共享：`Selection` 的 undo 快照 / 剪贴板 clone 只复制
//! BTreeMap 结构（O(页数) 指针），写入时按页 CoW（8KB）。
//!
//! id 0 是发号器哨兵（未分配），本结构不使用该位，调用方应跳过 id 0。

use std::collections::BTreeMap;
use std::sync::Arc;

/// 每页覆盖 2^16 = 65536 个连续 id（8KB）。
const PAGE_SHIFT: u32 = 16;
const PAGE_BITS: u32 = 1 << PAGE_SHIFT;
const PAGE_WORDS: usize = (PAGE_BITS / 64) as usize;

type Page = [u64; PAGE_WORDS];

#[derive(Clone, Default)]
pub struct NoteBitset {
    pages: BTreeMap<u32, Arc<Page>>,
    count: u64,
}

impl NoteBitset {
    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn contains(&self, id: u32) -> bool {
        let Some(page) = self.pages.get(&(id >> PAGE_SHIFT)) else {
            return false;
        };
        let word = ((id & (PAGE_BITS - 1)) >> 6) as usize;
        page[word] & (1u64 << (id & 63)) != 0
    }

    /// 置位。返回是否新增（0→1），重复置位返回 false 且不改变 count。
    pub fn insert(&mut self, id: u32) -> bool {
        let word = ((id & (PAGE_BITS - 1)) >> 6) as usize;
        let mask = 1u64 << (id & 63);
        let page = self
            .pages
            .entry(id >> PAGE_SHIFT)
            .or_insert_with(|| Arc::new([0u64; PAGE_WORDS]));
        let page = Arc::make_mut(page);
        if page[word] & mask != 0 {
            return false;
        }
        page[word] |= mask;
        self.count += 1;
        true
    }

    /// 清位。返回是否清除（1→0），未置位返回 false。
    pub fn remove(&mut self, id: u32) -> bool {
        let word = ((id & (PAGE_BITS - 1)) >> 6) as usize;
        let mask = 1u64 << (id & 63);
        let Some(page) = self.pages.get_mut(&(id >> PAGE_SHIFT)) else {
            return false;
        };
        let page = Arc::make_mut(page);
        if page[word] & mask == 0 {
            return false;
        }
        page[word] &= !mask;
        self.count -= 1;
        true
    }

    pub fn clear(&mut self) {
        self.pages.clear();
        self.count = 0;
    }
}

impl std::fmt::Debug for NoteBitset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NoteBitset({} ids, {} pages)",
            self.count,
            self.pages.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_contains_count() {
        let mut bits = NoteBitset::default();
        assert!(bits.is_empty());
        assert!(bits.insert(1));
        assert!(!bits.insert(1), "重复置位不算新增");
        assert!(bits.insert(2));
        assert_eq!(bits.count(), 2);
        assert!(bits.contains(1));
        assert!(bits.contains(2));
        assert!(!bits.contains(3));
    }

    #[test]
    fn cross_page_boundary() {
        let mut bits = NoteBitset::default();
        // 页边界：65535 属于第 0 页，65536 属于第 1 页
        for id in [65535, 65536, 131071, 131072] {
            assert!(bits.insert(id));
        }
        assert_eq!(bits.count(), 4);
        for id in [65535, 65536, 131071, 131072] {
            assert!(bits.contains(id));
        }
        assert!(!bits.contains(65534));
        assert!(!bits.contains(131070));
    }

    #[test]
    fn remove_clears_bit_and_count() {
        let mut bits = NoteBitset::default();
        bits.insert(7);
        bits.insert(8);
        assert!(bits.remove(7));
        assert!(!bits.remove(7), "重复清除返回 false");
        assert_eq!(bits.count(), 1);
        assert!(!bits.contains(7));
        assert!(bits.contains(8));
        assert!(!bits.remove(u32::MAX), "不存在的 id 不改变 count");
        assert_eq!(bits.count(), 1);
    }

    #[test]
    fn clear_resets_everything() {
        let mut bits = NoteBitset::default();
        bits.insert(1);
        bits.insert(100_000);
        bits.clear();
        assert!(bits.is_empty());
        assert_eq!(bits.count(), 0);
        assert!(!bits.contains(1));
    }

    #[test]
    fn clone_isolated_pages_share_until_write() {
        let mut a = NoteBitset::default();
        a.insert(1);
        a.insert(70_000); // 两个页
        let mut b = a.clone();

        // CoW 前页是共享的
        let pa = a.pages.get(&0).unwrap();
        let pb = b.pages.get(&0).unwrap();
        assert!(Arc::ptr_eq(pa, pb), "clone 后同一页应共享 Arc");

        b.insert(2);
        assert!(b.contains(2));
        assert!(!a.contains(2), "写入不得影响源位图");
        assert!(a.contains(1));
        assert!(b.contains(1));
        assert!(!Arc::ptr_eq(
            a.pages.get(&0).unwrap(),
            b.pages.get(&0).unwrap()
        ));
        assert_eq!(a.count(), 2);
        assert_eq!(b.count(), 3);
    }
}
