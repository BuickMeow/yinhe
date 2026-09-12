use std::collections::HashMap;

/// 完全在带外才跳过。传“显示位置”（y 动画后的值）而非目标位置：目标先出带时
/// 动画可能还没滑完，按目标裁会提前消失。顶部 `y >= max_y`，底部 `y + h <= BOTTOM_PAD`。
/// 零面积（恰好贴住带边）即跳过，无贡献所以无闪烁；有 1px 重叠即画。
pub(super) fn is_fully_outside(y: f32, card_h: f32, bottom_pad: f32, max_y: f32) -> bool {
    y >= max_y || y + card_h <= bottom_pad
}

/// 堆叠 y 累加：按 ids 顺序（调用方先排好，如最新在底则传 rev 后），逐项取实测高度，
/// 缺失 id 用 fallback。返回与 ids 等长对齐的 y。
pub(super) fn stack_ys(
    heights: &HashMap<u64, f32>,
    ids: &[u64],
    bottom_pad: f32,
    gap: f32,
    fallback: f32,
) -> Vec<f32> {
    let mut cum: f32 = 0.0;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(bottom_pad + cum);
        cum += heights.get(id).copied().unwrap_or(fallback) + gap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_ys_equal_heights() {
        let heights: HashMap<u64, f32> = [(1, 80.0), (2, 80.0)].into_iter().collect();
        assert_eq!(
            stack_ys(&heights, &[1, 2], 48.0, 8.0, 110.0),
            vec![48.0, 136.0]
        );
    }

    #[test]
    fn stack_ys_mixed_heights_with_fallback() {
        let heights: HashMap<u64, f32> = [(1, 60.0), (3, 90.0)].into_iter().collect();
        // id=2 缺失用 fallback=110：48, 48+60+8=116, 116+110+8=234
        assert_eq!(
            stack_ys(&heights, &[1, 2, 3], 48.0, 8.0, 110.0),
            vec![48.0, 116.0, 234.0]
        );
    }

    #[test]
    fn stack_ys_empty() {
        let heights: HashMap<u64, f32> = HashMap::new();
        assert!(stack_ys(&heights, &[], 48.0, 8.0, 110.0).is_empty());
    }

    #[test]
    fn fully_outside_boundary() {
        let bottom = 48.0;
        let max_y = 500.0;
        // 底部：零面积贴边（y+h == BOTTOM_PAD）在外，1px 重叠即画
        assert!(is_fully_outside(-52.0, 100.0, bottom, max_y));
        assert!(!is_fully_outside(-51.0, 100.0, bottom, max_y));
        // 底边 flush 在内（卡坐底边上）可见
        assert!(!is_fully_outside(bottom, 100.0, bottom, max_y));
        // 顶部：零面积贴边（y == max_y）在外，1px 重叠即画
        assert!(is_fully_outside(max_y, 100.0, bottom, max_y));
        assert!(!is_fully_outside(max_y - 1.0, 100.0, bottom, max_y));
        // 顶边 flush 在内可见，中间正常可见
        assert!(!is_fully_outside(max_y - 100.0, 100.0, bottom, max_y));
        assert!(!is_fully_outside(100.0, 100.0, bottom, max_y));
    }
}
