//! 选框右侧控制点的纯几何：桌面（egui）与 Flutter 共用。
//!
//! 三/两个方块的中心与命中判定必须两边一致，否则同一选框在桌面和移动端
//! 会出现“看得见但点不中”或“点中角落却触发变速”的偏差。

/// gate 方块内缩后距离选框左缘的最小余量（像素），避免方块压住左缘。
pub const GATE_MIN_MARGIN: f32 = 4.0;

/// 选框控制点种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelHandleKind {
    /// 右下角：按选区宽度重复（Riff）。
    Riff,
    /// 右缘中点：整体变速。
    Scale,
    /// 右缘内缩一个量化格：批量改 gate（仅 PR）。
    Gate,
}

impl SelHandleKind {
    /// 兼容 Flutter 现有 `edge` 编码：Riff = 4、Scale = 3、Gate = 5。
    pub const fn code(self) -> u8 {
        match self {
            Self::Riff => 4,
            Self::Scale => 3,
            Self::Gate => 5,
        }
    }
}

/// 控制点中心坐标（屏幕/内容坐标）。
#[derive(Clone, Copy, Debug)]
pub struct SelHandleCenters {
    pub corner: (f32, f32),
    pub scale: (f32, f32),
    pub gate: Option<(f32, f32)>,
}

#[inline]
fn normalize(bounds: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (x0, y0, x1, y1) = bounds;
    (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))
}

/// 计算三个控制点中心。
///
/// `vertical` 为 true 时主轴是纵向（时间轴在 Y），变速/ gate 沿纵向排布。
/// `show_gate` 为 false 时（AR）不生成 gate 控制点。
pub fn centers(
    bounds: (f32, f32, f32, f32),
    interval_px: f32,
    vertical: bool,
    show_gate: bool,
) -> SelHandleCenters {
    let (x0, y0, x1, y1) = normalize(bounds);
    let corner = (x1, y1);
    if vertical {
        let cx = (x0 + x1) * 0.5;
        let gate_y = y1 - interval_px;
        SelHandleCenters {
            corner,
            scale: (cx, y1),
            gate: (show_gate && gate_y > y0 + GATE_MIN_MARGIN).then_some((cx, gate_y)),
        }
    } else {
        let cy = (y0 + y1) * 0.5;
        let gate_x = x1 - interval_px;
        SelHandleCenters {
            corner,
            scale: (x1, cy),
            gate: (show_gate && gate_x > x0 + GATE_MIN_MARGIN).then_some((gate_x, cy)),
        }
    }
}

/// 命中选框控制点：取半径内最近者；距离相同时按 Riff → Scale → Gate。
pub fn hit_test(
    bounds: (f32, f32, f32, f32),
    interval_px: f32,
    vertical: bool,
    show_gate: bool,
    x: f32,
    y: f32,
    radius: f32,
) -> Option<SelHandleKind> {
    let c = centers(bounds, interval_px, vertical, show_gate);
    let r2 = radius * radius;
    let mut best: Option<(SelHandleKind, f32)> = None;
    let candidates = [
        (SelHandleKind::Riff, Some(c.corner)),
        (SelHandleKind::Scale, Some(c.scale)),
        (SelHandleKind::Gate, c.gate),
    ];
    for (kind, p) in candidates {
        let Some((hx, hy)) = p else { continue };
        let d = (x - hx) * (x - hx) + (y - hy) * (y - hy);
        if d <= r2 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((kind, d));
        }
    }
    best.map(|(kind, _)| kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_corner_scale_and_gate() {
        let c = centers((0.0, 0.0, 100.0, 100.0), 20.0, false, true);
        assert_eq!(c.corner, (100.0, 100.0));
        assert_eq!(c.scale, (100.0, 50.0));
        assert_eq!(c.gate, Some((80.0, 50.0)));

        let hit = |x, y| hit_test((0.0, 0.0, 100.0, 100.0), 20.0, false, true, x, y, 11.0);
        assert_eq!(hit(100.0, 100.0), Some(SelHandleKind::Riff));
        assert_eq!(hit(100.0, 50.0), Some(SelHandleKind::Scale));
        assert_eq!(hit(80.0, 50.0), Some(SelHandleKind::Gate));
    }

    #[test]
    fn vertical_uses_bottom_edge() {
        let c = centers((0.0, 0.0, 100.0, 200.0), 40.0, true, true);
        assert_eq!(c.corner, (100.0, 200.0));
        assert_eq!(c.scale, (50.0, 200.0));
        assert_eq!(c.gate, Some((50.0, 160.0)));
    }

    #[test]
    fn gate_disabled_for_ar() {
        let c = centers((0.0, 0.0, 100.0, 100.0), 20.0, false, false);
        assert_eq!(c.gate, None);
        assert_eq!(
            hit_test(
                (0.0, 0.0, 100.0, 100.0),
                20.0,
                false,
                false,
                80.0,
                50.0,
                11.0
            ),
            None
        );
    }

    #[test]
    fn nearest_wins_when_handles_overlap() {
        // 选框高度很小：corner 与 scale 距离接近，取更近者。
        let c = centers((0.0, 0.0, 100.0, 100.0), 20.0, false, true);
        assert_eq!(c.scale.1, 50.0);
        assert_eq!(
            hit_test(
                (0.0, 0.0, 100.0, 100.0),
                20.0,
                false,
                true,
                100.0,
                54.0,
                11.0
            ),
            Some(SelHandleKind::Scale)
        );
    }
}
