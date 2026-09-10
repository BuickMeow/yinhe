use std::collections::HashMap;

use eframe::egui;

pub(super) fn scroll_max(total: f32, visible: f32) -> f32 {
    (total - visible).max(0.0)
}

pub(super) fn follow_scroll(prev_scroll: f32, prev_max: f32, new_max: f32) -> f32 {
    if prev_scroll >= prev_max - 40.0 {
        new_max
    } else {
        prev_scroll.clamp(0.0, new_max)
    }
}

/// 列表可见高度：视口高 − 底部留白 − 顶部 24 留白，窗口过矮时保底 120。
pub(super) fn visible_h(viewport_h: f32, bottom_pad: f32) -> f32 {
    (viewport_h - bottom_pad - 24.0).max(120.0)
}

/// 可见带：x 沿用进入时的 clip，y 裁到 `[viewport.max.y - max_y, viewport.max.y - bottom_pad]`。
/// `max_y` = 底留白 + 可见高，与滚动上限同源（`BOTTOM_PAD + visible_h(..)`）。
pub(super) fn notif_band(
    clip: egui::Rect,
    viewport: egui::Rect,
    bottom_pad: f32,
    max_y: f32,
) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(clip.min.x, viewport.max.y - max_y),
        egui::pos2(clip.max.x, viewport.max.y - bottom_pad),
    )
}

/// 完全在带外才跳过。传“显示位置”（y 动画后的值）而非目标位置：目标先出带时
/// 动画可能还没滑完，按目标裁会提前消失。顶部 `y >= max_y`，底部 `y + h <= BOTTOM_PAD`。
/// 零面积（恰好贴住带边）即跳过，无贡献所以无闪烁；有 1px 重叠即画，由外层 band 裁剪滑出。
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

    /// 回归：可见带顶部与滚动上限同源——滚到最顶时最旧卡顶边恰在带顶（视口顶 + 24），
    /// 不再被多切 48px（曾把 BOTTOM_PAD 重复计入 max_y 导致顶部截断）。
    #[test]
    fn band_top_matches_scroll_limit() {
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1400.0, 900.0));
        let bottom = 48.0;
        let max_y = bottom + visible_h(viewport.height(), bottom);
        let band = notif_band(viewport, viewport, bottom, max_y);
        // 带顶 = 视口顶 + 24；带底 = 视口底 − 底留白
        assert!((band.min.y - 24.0).abs() < 1e-4, "band top {}", band.min.y);
        assert!((band.max.y - (viewport.max.y - bottom)).abs() < 1e-4);
        // 滚到最顶：最旧卡顶边 offset = bottom + total、scroll = total − visible
        // → anchor = max_y − h，仍在带内不被裁
        let h = 110.0;
        assert!(!is_fully_outside(max_y - h, h, bottom, max_y));
    }

    #[test]
    fn notif_band_geometry() {
        let clip = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1400.0, 900.0));
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1400.0, 900.0));
        let band = notif_band(clip, viewport, 48.0, 500.0);
        // y 裁到 [900-500, 900-48]，x 沿用进入 clip
        assert!((band.min.y - 400.0).abs() < 1e-4);
        assert!((band.max.y - 852.0).abs() < 1e-4);
        assert!((band.min.x - 0.0).abs() < 1e-4);
        assert!((band.max.x - 1400.0).abs() < 1e-4);
    }

    #[test]
    fn scroll_max_clamps_to_zero() {
        assert!((scroll_max(100.0, 200.0) - 0.0).abs() < 1e-6);
        assert!((scroll_max(200.0, 200.0) - 0.0).abs() < 1e-6);
        assert!((scroll_max(500.0, 200.0) - 300.0).abs() < 1e-6);
    }

    #[test]
    fn scroll_follow_sticks_near_bottom() {
        // 在底部附近（40 以内）内容变高 → 吸到新 max
        assert!((follow_scroll(460.0, 500.0, 600.0) - 600.0).abs() < 1e-6);
        assert!((follow_scroll(500.0, 500.0, 600.0) - 600.0).abs() < 1e-6);
        // 离底部远 → 保持旧值
        assert!((follow_scroll(100.0, 500.0, 600.0) - 100.0).abs() < 1e-6);
        // 内容变矮导致旧值越界 → clamp 到新 max
        assert!((follow_scroll(500.0, 500.0, 200.0) - 200.0).abs() < 1e-6);
    }
}
