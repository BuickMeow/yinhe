use yinhe_types::AutomationTarget;
use yinhe_types::automation::{
    CHANNEL_DSP_PARAMS, MidiBinding, ParamDevice, XSYNTH_PARAMS, builtin_param,
    channel_dsp_param_id_for_midi, xsynth_param_id_for_midi,
};

use crate::theme;

/// 已知自动化目标（PR 自动化面板下拉 + AR「添加自动化」窗口共用）。
///
/// 统一参数模型：设备参数从内置参数表按设备通道生成（XSynth 音源参数 +
/// 通道 DSP 参数）；低层 MIDI CC 由 UI 的自定义 CC 输入框单独提供。
/// `channel == None`（无具体设备通道）时只返回 Tempo。
pub fn automation_targets(channel: Option<u8>) -> Vec<AutomationTarget> {
    let mut out = vec![AutomationTarget::Tempo];
    let Some(channel) = channel else {
        return out;
    };
    out.extend(
        XSYNTH_PARAMS
            .iter()
            .map(|p| param(ParamDevice::ChannelInstrument { channel }, p.id)),
    );
    out.extend(
        CHANNEL_DSP_PARAMS
            .iter()
            .map(|p| param(ParamDevice::ChannelDsp { channel }, p.id)),
    );
    out
}

/// 构造内置设备参数 target（内置参数的显示名以参数表为准，name 留空）。
fn param(device: ParamDevice, id: u32) -> AutomationTarget {
    AutomationTarget::Param {
        device,
        id,
        name: String::new(),
    }
}

/// lane target 的设备参数 ↔ 低层 CC 双向转换（仅限有 MIDI CC 绑定的参数）。
///
/// 返回 `None` = 该 target 不支持转换（PB/RPN 绑定、第三方插件参数、非 CC 目标）。
/// 值域两边都是归一化 0..1，转换不改变事件值。
pub fn convert_lane_target(target: &AutomationTarget, channel: u8) -> Option<AutomationTarget> {
    match target {
        AutomationTarget::CC { controller } => {
            let midi = MidiBinding::Cc(*controller);
            if let Some(id) = channel_dsp_param_id_for_midi(midi) {
                return Some(param(ParamDevice::ChannelDsp { channel }, id));
            }
            xsynth_param_id_for_midi(midi)
                .map(|id| param(ParamDevice::ChannelInstrument { channel }, id))
        }
        AutomationTarget::Param { device, id, .. } => {
            // 仅内置设备参数有 MIDI 绑定；第三方插件参数不参与转换。
            let cc = match builtin_param(device, *id)?.midi {
                MidiBinding::Cc(cc) => cc,
                _ => return None,
            };
            Some(AutomationTarget::CC { controller: cc })
        }
        _ => None,
    }
}

/// 锚点命中半径（像素）。鼠标在此半径内点击视为选中该锚点。
pub const ANCHOR_HIT_PX: f32 = 10.0;

/// Height of the split/handle between automation panels.
pub const SPLIT_H: f32 = theme::AUTO_PANEL_SPLIT_H;

/// 悬停在锚点上多久后显示 tooltip（秒）。
pub const HOVER_DELAY: f64 = 0.6;

/// 选框拖拽触发阈值（像素）。小于此距离视为点击，不触发选区清空。
pub const MARQUEE_THRESHOLD: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::automation::{channel_dsp_param, xsynth_param};

    fn param(device: ParamDevice, id: u32) -> AutomationTarget {
        AutomationTarget::Param {
            device,
            id,
            name: String::new(),
        }
    }

    /// CC ↔ 设备参数双向映射（DSP 优先；PB/RPN 绑定与未知 CC 不转换）。
    #[test]
    fn convert_lane_target_maps_cc_both_ways() {
        let cc7 = AutomationTarget::CC { controller: 7 };
        let dsp = ParamDevice::ChannelDsp { channel: 3 };
        let inst = ParamDevice::ChannelInstrument { channel: 3 };

        assert_eq!(
            convert_lane_target(&cc7, 3),
            Some(param(dsp.clone(), channel_dsp_param::VOLUME))
        );
        assert_eq!(
            convert_lane_target(&cc7, 3).and_then(|t| convert_lane_target(&t, 3)),
            Some(cc7.clone()),
            "双向转换应互逆"
        );
        assert_eq!(
            convert_lane_target(&AutomationTarget::CC { controller: 64 }, 3),
            Some(param(inst.clone(), xsynth_param::SUSTAIN))
        );
        assert_eq!(
            convert_lane_target(&param(inst, xsynth_param::PITCH_BEND), 3),
            None,
            "PB 绑定不参与转换"
        );
        assert_eq!(
            convert_lane_target(&AutomationTarget::CC { controller: 1 }, 3),
            None,
            "无绑定的 CC 不参与转换"
        );
        assert_eq!(convert_lane_target(&AutomationTarget::Tempo, 3), None);
    }

    /// 设备参数列表按通道生成，含 Tempo 与两张内置参数表。
    #[test]
    fn automation_targets_lists_builtin_params() {
        let targets = automation_targets(Some(5));
        assert_eq!(targets[0], AutomationTarget::Tempo);
        let expected = XSYNTH_PARAMS.len() + CHANNEL_DSP_PARAMS.len();
        assert_eq!(targets.len(), expected + 1);
        assert!(targets.contains(&AutomationTarget::Param {
            device: ParamDevice::ChannelInstrument { channel: 5 },
            id: xsynth_param::SUSTAIN,
            name: String::new(),
        }));
        assert!(targets.contains(&AutomationTarget::Param {
            device: ParamDevice::ChannelDsp { channel: 5 },
            id: channel_dsp_param::PAN,
            name: String::new(),
        }));
        assert_eq!(automation_targets(None), vec![AutomationTarget::Tempo]);
    }
}
