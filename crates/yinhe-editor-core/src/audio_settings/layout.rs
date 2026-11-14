use serde::{Deserialize, Serialize};

use crate::right_panel_layout::RightPanelLayout;

/// 单个底部面板的「浮动窗口」状态（跨会话持久化）。
///
/// 与 `show_*` 停靠开关互斥：`floating = true` 时面板以独立 OS 窗口显示，
/// 停靠形态关闭；`floating = false` 时由 `show_*` 决定停靠/隐藏。
/// 同一面板任意时刻只渲染一份（钢琴卷帘的 GPU 视图只允许单实例）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FloatPanelState {
    /// 是否以独立窗口形态显示。
    pub floating: bool,
    /// 窗口内区矩形 `[x, y, w, h]`（逻辑像素）；`None` = 使用默认尺寸。
    pub rect: Option<[f32; 4]>,
}

/// 主输出表模块种类。顺序/宽度由用户拖拽调整并持久化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterModuleKind {
    Loudness,
    Oscilloscope,
    Spectrum,
    Spectrogram,
    Stereometer,
    Vu,
    Waveform,
}

impl MeterModuleKind {
    pub const ALL: [Self; 7] = [
        Self::Loudness,
        Self::Oscilloscope,
        Self::Spectrum,
        Self::Spectrogram,
        Self::Stereometer,
        Self::Vu,
        Self::Waveform,
    ];
}

/// 一个主输出表模块槽位：种类 + 相对宽度权重。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeterModuleSlot {
    pub kind: MeterModuleKind,
    /// 相对宽度权重（布局时归一化；不是像素）。
    pub weight: f32,
}

/// 默认主输出表模块：常用的四个；频谱图/波形/VU 从「+」添加。
pub fn default_meter_modules() -> Vec<MeterModuleSlot> {
    vec![
        MeterModuleSlot {
            kind: MeterModuleKind::Loudness,
            weight: 1.15,
        },
        MeterModuleSlot {
            kind: MeterModuleKind::Stereometer,
            weight: 1.0,
        },
        MeterModuleSlot {
            kind: MeterModuleKind::Spectrum,
            weight: 1.6,
        },
        MeterModuleSlot {
            kind: MeterModuleKind::Oscilloscope,
            weight: 1.25,
        },
    ]
}

/// 用户可拖拽调整的布局状态（跨会话持久化）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutSettings {
    pub right_panel_width: f32,
    pub arr_split: f32,
    pub transport_panel_width: f32,
    pub show_pianoroll_in_arrange: bool,
    /// 三视图通用底部设备栏是否展开。
    pub show_bottom_dock: bool,
    /// 底部设备栏高度（px）。
    pub bottom_dock_height: f32,
    /// MIX 视图底部是否显示混音台面板（钢琴卷帘/设备链/混音台三选一之一）。
    pub show_mixer_panel: bool,
    /// MIX 视图底部混音台面板高度（px）。仅在 `mix_height_auto == false`
    /// （用户拖拽过分割条）时使用。
    pub mix_height: f32,
    /// 混音台高度是否随总线数量自动扩展。用户拖拽分割条后转为手动，
    /// 双击分割条恢复自动。
    #[serde(default = "default_mix_height_auto")]
    pub mix_height_auto: bool,
    /// 钢琴卷帘浮动窗口（泊出为独立窗口）。
    pub float_pianoroll: FloatPanelState,
    /// 底部设备栏浮动窗口。
    pub float_bottom_dock: FloatPanelState,
    /// 混音台浮动窗口。
    pub float_mixer: FloatPanelState,
    /// 工程走带（ARRANGE 主视图）浮动窗口。
    pub float_arrange: FloatPanelState,
    /// 三视图通用主输出表是否展开。
    pub show_meter_panel: bool,
    /// 主输出表高度（px）。
    pub meter_height: f32,
    /// 主输出表浮动窗口。
    pub float_meter: FloatPanelState,
    /// 主输出表模块顺序与宽度。
    pub meter_modules: Vec<MeterModuleSlot>,
    /// 右栏多栏选项卡布局。
    pub right_panel_layout: RightPanelLayout,
    /// PR 视图方向是否为纵向瀑布流（跨会话持久化，打开新文档时沿用）。
    pub pr_vertical: bool,
}

/// `mix_height_auto` 的 serde 默认值：旧配置文件没有该字段时默认开启自动高度。
fn default_mix_height_auto() -> bool {
    true
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            right_panel_width: 320.0,
            arr_split: 0.3,
            transport_panel_width: 200.0,
            show_pianoroll_in_arrange: false,
            show_bottom_dock: false,
            bottom_dock_height: 200.0,
            show_mixer_panel: true,
            mix_height: 200.0,
            mix_height_auto: true,
            float_pianoroll: FloatPanelState::default(),
            float_bottom_dock: FloatPanelState::default(),
            float_mixer: FloatPanelState::default(),
            float_arrange: FloatPanelState::default(),
            show_meter_panel: false,
            meter_height: 220.0,
            float_meter: FloatPanelState::default(),
            meter_modules: default_meter_modules(),
            right_panel_layout: RightPanelLayout::default(),
            pr_vertical: false,
        }
    }
}

#[cfg(test)]
mod meter_layout_tests {
    use super::*;

    #[test]
    fn old_config_gets_default_meter_modules() {
        let v: LayoutSettings = serde_json::from_str("{}").expect("空配置应可读");
        assert_eq!(v.meter_modules, default_meter_modules());
        assert!(!v.show_meter_panel);
    }

    #[test]
    fn explicitly_empty_meter_layout_is_preserved() {
        let v: LayoutSettings =
            serde_json::from_str(r#"{"meter_modules":[]}"#).expect("空布局应可读");
        assert!(v.meter_modules.is_empty());
    }

    #[test]
    fn meter_modules_roundtrip_in_order() {
        let v = LayoutSettings {
            meter_modules: vec![
                MeterModuleSlot {
                    kind: MeterModuleKind::Spectrum,
                    weight: 2.0,
                },
                MeterModuleSlot {
                    kind: MeterModuleKind::Vu,
                    weight: 0.5,
                },
            ],
            ..Default::default()
        };
        let text = serde_json::to_string(&v).expect("序列化");
        let back: LayoutSettings = serde_json::from_str(&text).expect("反序列化");
        assert_eq!(back.meter_modules, v.meter_modules);
    }
}
