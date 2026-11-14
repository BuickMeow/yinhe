use super::parse_velocity_stroke;

/// 扁平数组按 f64 元素数解释：`[tick, vel]` 两两成对，不能把点数当长度。
#[test]
fn parse_velocity_stroke_uses_flat_len() {
    // 两个点 = 4 个 f64；若误把 2（点数）当长度，只会得到 1 个点。
    let flat = [100.0, 64.0, 200.0, 100.0];
    let points = parse_velocity_stroke(&flat);
    assert_eq!(points.len(), 2);
    assert_eq!(points[0], (100.0, 64.0));
    assert_eq!(points[1], (200.0, 100.0));
}

#[test]
fn parse_velocity_stroke_drops_non_finite() {
    let flat = [10.0, f64::NAN, 20.0, 30.0, f64::INFINITY, 40.0];
    let points = parse_velocity_stroke(&flat);
    assert_eq!(points, vec![(20.0, 30.0)]);
}

/// 自动化移动按 3 个 f64 一组解析：old_tick / new_tick / value。
#[test]
fn parse_automation_moves_uses_triples() {
    let flat = [100.0, 148.0, 0.5, 200.0, 200.0, 2.0];
    let moves = super::parse_automation_moves(&flat);
    assert_eq!(moves, vec![(100, 148, 0.5), (200, 200, 1.0)]);
}
