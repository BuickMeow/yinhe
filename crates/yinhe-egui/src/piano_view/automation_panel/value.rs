use yinhe_types::{AutomationLane, AutomationPanelView, AutomationTarget};

/// Tempo 的显示/编辑上限（BPM 理论上限）。
///
/// 统一参数模型里 Tempo 是唯一不归一化的量（`display_max() == None`），
/// 但它仍需要一个可编辑上界（旧 `max_value()` 的语义）。
pub(crate) const TEMPO_DISPLAY_MAX: f32 = 60_000_000.0;

/// 值的显示上限：有 MIDI 绑定换算的返回原始上限（CC 127 / PB 16383），
/// 无换算的按原值处理（Tempo 用理论上限，第三方插件参数用归一化 1.0）。
pub(crate) fn display_max_or_bound(target: &AutomationTarget) -> f32 {
    match target.display_max() {
        Some(max) => max,
        None if matches!(target, AutomationTarget::Tempo) => TEMPO_DISPLAY_MAX,
        None => 1.0,
    }
}

/// 归一化值 → 显示文本（唯一格式化入口）。
///
/// 有 MIDI 绑定换算的目标按原始整数显示（CC/PB/RPN），取整；
/// Tempo（BPM）与第三方插件参数按小数显示。
pub(crate) fn format_display_value(target: &AutomationTarget, value: f32) -> String {
    match target.display_max() {
        Some(_) => format!("{}", target.to_display_value(value).round()),
        None => format!("{value:.2}"),
    }
}

/// 计算 target 的值上限。达到此上限时不可再缩小 value_zoom。
///
/// 值空间：非 Tempo 目标统一为归一化 0..1；Tempo 为 BPM 原值（动态上限由
/// `panel_max_val` 按事件计算，这里只给理论边界）。
pub(crate) fn value_upper_bound(panel: &AutomationPanelView) -> f32 {
    if panel.show_velocity {
        127.0
    } else if panel.selected_target == AutomationTarget::Tempo {
        TEMPO_DISPLAY_MAX
    } else {
        1.0
    }
}

/// 面板当前 target 的值上限（velocity=127；Tempo 由实际事件动态计算；
/// 其他 target 归一化 1.0）。show_panels（zoom/scroll/标签）与 interaction
/// （y↔value 换算）共用。
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
        1.0
    }
}

/// 计算 value_zoom 的下限，使得 visible_range 不超过 upper_bound.
pub(crate) fn min_value_zoom(max_val: f32, upper_bound: f32) -> f32 {
    if upper_bound <= 0.0 {
        return 1.0;
    }
    (max_val / upper_bound).max(0.01)
}
