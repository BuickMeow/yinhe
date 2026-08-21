use yinhe_types::{AutomationLane, AutomationPanelView, AutomationTarget};

/// 计算 target 的值上限。达到此上限时不可再缩小 value_zoom.
/// - Tempo: `AutomationTarget::Tempo.max_value()`（BPM 理论上限 60_000_000）
/// - CC/PB/RPN/NRPN: max_value()
pub(crate) fn value_upper_bound(panel: &AutomationPanelView) -> f32 {
    if panel.show_velocity {
        127.0
    } else if panel.selected_target == AutomationTarget::Tempo {
        AutomationTarget::Tempo.max_value()
    } else {
        panel.selected_target.max_value()
    }
}

/// 面板当前 target 的值上限（velocity=127；Tempo 由实际事件动态计算；其他 max_value()）。
/// show_panels（zoom/scroll/标签）与 interaction（y↔value 换算）共用。
pub(crate) fn panel_max_val(panel: &AutomationPanelView, tempo_lane: &AutomationLane) -> f32 {
    if panel.show_velocity {
        127.0
    } else if panel.selected_target == AutomationTarget::Tempo {
        tempo_lane
            .events
            .iter()
            .map(|e| e.value)
            .fold(0.0_f32, f32::max)
            .max(1.0)
    } else {
        panel.selected_target.max_value()
    }
}

/// 计算 value_zoom 的下限，使得 visible_range 不超过 upper_bound.
pub(crate) fn min_value_zoom(max_val: f32, upper_bound: f32) -> f32 {
    if upper_bound <= 0.0 {
        return 1.0;
    }
    (max_val / upper_bound).max(0.01)
}
