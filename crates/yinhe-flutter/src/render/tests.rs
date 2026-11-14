use super::upload::z_order_track_set;
use super::*;

#[test]
fn z_order_includes_active_track() {
    let selected: HashSet<u16> = [2, 5].into_iter().collect();

    let out = z_order_track_set(&selected, Some(3));

    assert!(out.contains(&2) && out.contains(&5) && out.contains(&3));

    // 不修改传入集合。

    assert_eq!(selected.len(), 2);
}

#[test]
fn z_order_without_active_keeps_selected() {
    let selected: HashSet<u16> = [7].into_iter().collect();

    assert_eq!(z_order_track_set(&selected, None), selected);
}
