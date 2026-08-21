use yinhe_types::AutomationTarget;

use crate::theme;

/// Curated list of known automation targets shown in the dropdown.
/// AR 音轨面板「创建自动化」右键菜单也复用此列表（跳过 Tempo）。
pub const AUTOMATION_TARGETS: &[AutomationTarget] = &[
    AutomationTarget::Tempo,
    AutomationTarget::PitchBend,
    AutomationTarget::CC { controller: 7 },  // Volume
    AutomationTarget::CC { controller: 10 }, // Pan
    AutomationTarget::CC { controller: 11 }, // Expression
    AutomationTarget::CC { controller: 64 }, // Sustain
    AutomationTarget::CC { controller: 71 }, // Resonance
    AutomationTarget::CC { controller: 72 }, // Release
    AutomationTarget::CC { controller: 73 }, // Attack
    AutomationTarget::CC { controller: 74 }, // Cutoff
    AutomationTarget::Rpn { parameter: 0 },  // PB Sensitivity
    AutomationTarget::Rpn { parameter: 1 },  // Fine Tune
    AutomationTarget::Rpn { parameter: 2 },  // Coarse Tune
];

/// 锚点命中半径（像素）。鼠标在此半径内点击视为选中该锚点。
pub const ANCHOR_HIT_PX: f32 = 10.0;

/// Height of the split/handle between automation panels.
pub const SPLIT_H: f32 = theme::AUTO_PANEL_SPLIT_H;

/// 悬停在锚点上多久后显示 tooltip（秒）。
pub const HOVER_DELAY: f64 = 0.6;

/// 选框拖拽触发阈值（像素）。小于此距离视为点击，不触发选区清空。
pub const MARQUEE_THRESHOLD: f32 = 3.0;
