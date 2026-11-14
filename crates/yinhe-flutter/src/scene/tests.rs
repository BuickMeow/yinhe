use yinhe_core::TrackData;
use yinhe_types::automation::{AutomationLane, AutomationTarget};

use super::automation::visible_lane;

fn lane(target: AutomationTarget) -> AutomationLane {
    AutomationLane {
        target,
        track: 0,
        events: Vec::new(),
    }
}

/// 拖动选框控制点期间不能丢掉 sel_target，否则 overlay 会退回旧矩形来回抖。
#[test]
fn sel_target_survives_while_drag_active() {
    let mut s = super::RenderScene::default();
    s.set_sel_target([0.0, 0.0, 100.0, 40.0]);
    s.sel_drag_active = true;
    s.sel_anim = [0.0; 4];
    s.tick_preview_anim();
    assert!(s.sel_target.is_some(), "拖动期间 sel_target 必须保留");
    s.sel_drag_active = false;
    s.tick_preview_anim();
    assert!(s.sel_target.is_none(), "松手后预览应回到已提交矩形");
}

/// `panel.lane` 与渲染层一致使用 0 基下标：0 = 第一条可绘制自动化 lane。
#[test]
fn visible_lane_is_zero_based() {
    let mut track = TrackData::new(0, 0);
    track.automation_lanes = vec![
        lane(AutomationTarget::CC { controller: 7 }),
        lane(AutomationTarget::CC { controller: 10 }),
    ];
    assert_eq!(visible_lane(&track, 0).map(|v| v.0), Some(0));
    assert_eq!(visible_lane(&track, 1).map(|v| v.0), Some(1));
    assert!(visible_lane(&track, 2).is_none());
}
