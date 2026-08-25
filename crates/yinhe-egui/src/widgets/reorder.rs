//! 列表行拖拽排序的通用状态与算法（音色库列表 / 音轨面板共用）。

use eframe::egui;

/// 拖拽排序状态：被拖行索引（升序，保持相对顺序）+ 插入位置。
/// 插入位置按"删除被拖行后的剩余行"计数（0..=剩余行数）。
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct DragReorder {
    pub indices: Vec<usize>,
    pub insert_idx: usize,
}

impl DragReorder {
    /// 指针越过剩余行中线则后移一位（跳过被拖行）。
    pub fn update_insert_idx(&mut self, pointer_y: f32, item_rects: &[egui::Rect]) {
        let mut insert = 0usize;
        for (i, rect) in item_rects.iter().enumerate() {
            if self.indices.contains(&i) {
                continue;
            }
            if pointer_y < rect.center().y {
                break;
            }
            insert += 1;
        }
        self.insert_idx = insert;
    }

    /// 指针越过剩余列中线则后移一位（跳过被拖列，标题栏横向用）。
    pub fn update_insert_idx_horizontal(&mut self, pointer_x: f32, item_rects: &[egui::Rect]) {
        let mut insert = 0usize;
        for (i, rect) in item_rects.iter().enumerate() {
            if self.indices.contains(&i) {
                continue;
            }
            if pointer_x < rect.center().x {
                break;
            }
            insert += 1;
        }
        self.insert_idx = insert;
    }

    /// 插入线 y 坐标：删除被拖行后，第 `insert_idx` 个剩余行的顶部；
    /// 插到末尾时为最后一个可见行的底部。
    pub fn insert_line_y(&self, item_rects: &[egui::Rect]) -> Option<f32> {
        let mut visible = 0usize;
        for (i, rect) in item_rects.iter().enumerate() {
            if self.indices.contains(&i) {
                continue;
            }
            if visible == self.insert_idx {
                return Some(rect.top());
            }
            visible += 1;
        }
        item_rects.last().map(|r| r.bottom())
    }

    /// 插入线 x 坐标（标题栏横向）：删除被拖列后，第 `insert_idx` 个剩余列的左缘；
    /// 插到末尾时为最后一个标签的右缘。
    pub fn insert_line_x(&self, item_rects: &[egui::Rect]) -> Option<f32> {
        let mut visible = 0usize;
        for (i, rect) in item_rects.iter().enumerate() {
            if self.indices.contains(&i) {
                continue;
            }
            if visible == self.insert_idx {
                return Some(rect.left());
            }
            visible += 1;
        }
        item_rects.last().map(|r| r.right() + 1.0)
    }
}

/// 计算拖拽排序后的最终顺序（原始索引序列）：被拖行整体移动到
/// "删除它们后的列表中的 `insert_at` 位置"，其余行保持原顺序。
pub(crate) fn plan_order(len: usize, indices: &[usize], insert_at: usize) -> Vec<usize> {
    let insert_at = insert_at.min(len.saturating_sub(indices.len()));
    let remaining: Vec<usize> = (0..len).filter(|i| !indices.contains(i)).collect();
    let mut order = Vec::with_capacity(len);
    order.extend_from_slice(&remaining[..insert_at]);
    order.extend_from_slice(indices);
    order.extend_from_slice(&remaining[insert_at..]);
    order
}

/// 把排序计划转成逐个 `move_track(from, to)` 调用（每次作用于当前列表，
/// 逐步逼近目标顺序）。返回序列按目标位置从前到后执行即得最终顺序。
pub(crate) fn plan_moves(len: usize, indices: &[usize], insert_at: usize) -> Vec<(usize, usize)> {
    let order = plan_order(len, indices, insert_at);
    let mut cur: Vec<usize> = (0..len).collect();
    let mut moves = Vec::new();
    for pos in 0..len {
        if cur[pos] == order[pos] {
            continue;
        }
        let j = cur
            .iter()
            .position(|&x| x == order[pos])
            .expect("order 是 cur 的排列");
        moves.push((j, pos));
        let item = cur.remove(j);
        cur.insert(pos, item);
    }
    moves
}

/// 对 Vec 直接应用排序（音色库列表用；音轨面板走 [`plan_moves`] + move_track）。
pub(crate) fn apply_reorder<T: Clone>(items: &mut Vec<T>, indices: &[usize], insert_at: usize) {
    let order = plan_order(items.len(), indices, insert_at);
    *items = order.iter().map(|&i| items[i].clone()).collect();
}

/// 对 Vec 直接应用排序（不要求 Clone，用于 Document / MixerRack 等）。
pub(crate) fn apply_reorder_noclone<T>(items: &mut Vec<T>, indices: &[usize], insert_at: usize) {
    let order = plan_order(items.len(), indices, insert_at);
    let mut old: Vec<Option<T>> = std::mem::take(items).into_iter().map(Some).collect();
    let mut new = Vec::with_capacity(order.len());
    for &idx in &order {
        new.push(old[idx].take().expect("order 是排列"));
    }
    *items = new;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// plan_order 的最终顺序与 apply_reorder 逐项应用 plan_moves 后一致。
    fn assert_moves_agree(len: usize, indices: &[usize], insert_at: usize) {
        let order = plan_order(len, indices, insert_at);
        let moves = plan_moves(len, indices, insert_at);
        let mut cur: Vec<usize> = (0..len).collect();
        for (from, to) in moves {
            let item = cur.remove(from);
            cur.insert(to, item);
        }
        assert_eq!(cur, order, "indices={indices:?} insert_at={insert_at}");
    }

    #[test]
    fn single_row_to_middle() {
        assert_moves_agree(5, &[0], 2);
        let order = plan_order(5, &[0], 2);
        assert_eq!(order, vec![1, 2, 0, 3, 4]);
    }

    #[test]
    fn single_row_to_front() {
        assert_moves_agree(5, &[3], 0);
        let order = plan_order(5, &[3], 0);
        assert_eq!(order, vec![3, 0, 1, 2, 4]);
    }

    #[test]
    fn multiple_rows_to_end() {
        assert_moves_agree(6, &[1, 3], 4);
        let order = plan_order(6, &[1, 3], 4);
        assert_eq!(order, vec![0, 2, 4, 5, 1, 3]);
    }

    #[test]
    fn multiple_rows_to_front() {
        assert_moves_agree(6, &[2, 4], 0);
        let order = plan_order(6, &[2, 4], 0);
        assert_eq!(order, vec![2, 4, 0, 1, 3, 5]);
    }

    #[test]
    fn insert_at_past_end_clamps() {
        assert_moves_agree(4, &[1], 99);
        let order = plan_order(4, &[1], 99);
        assert_eq!(order, vec![0, 2, 3, 1]);
    }

    #[test]
    fn non_contiguous_selection_keeps_relative_order() {
        assert_moves_agree(8, &[0, 2, 5], 3);
    }

    #[test]
    fn no_moves_when_already_in_place() {
        assert!(plan_moves(4, &[2], 2).is_empty());
        assert!(plan_moves(4, &[1, 2], 1).is_empty());
    }

    #[test]
    fn dragging_whole_list_is_noop() {
        // 全选拖拽：删除后列表为空，insert_at 只能是 0，顺序不变
        assert_moves_agree(4, &[0, 1, 2, 3], 0);
    }
}
