use yinhe_types::AutomationTarget;
use yinhe_types::automation::{
    BuiltinParamInfo, CHANNEL_DSP_PARAMS, MidiBinding, ParamDevice, XSYNTH_PARAMS,
};

use crate::theme;

/// 已知自动化目标（PR 自动化面板下拉 + AR「添加自动化」窗口共用）。
///
/// 统一参数模型：CC 绑定的内置参数映射回低层 `CC{controller}` lane（回放时
/// 广播给订阅的效果器 / 走合成器常规路径）；PB/RPN 仍是设备参数（`Param`）。
/// `channel == None`（无具体设备通道）时只返回 Tempo。
pub fn automation_targets(channel: Option<u8>) -> Vec<AutomationTarget> {
    let mut out = vec![AutomationTarget::Tempo];
    let Some(channel) = channel else {
        return out;
    };
    out.extend(XSYNTH_PARAMS.iter().map(|p| builtin_target(p, channel)));
    out.extend(
        CHANNEL_DSP_PARAMS
            .iter()
            .map(|p| builtin_target(p, channel)),
    );
    out
}

/// 内置参数 → 自动化目标：CC 绑定 = 低层 `CC{controller}` lane（通道 DSP 参数
/// 全部如此，回放广播给效果器模块）；PB/RPN 保留设备参数（`ChannelInstrument`）。
/// XSYNTH_PARAMS 与 CHANNEL_DSP_PARAMS 共用（dock 旋钮生成 XSynth 目标同样复用）。
pub(crate) fn builtin_target(p: &BuiltinParamInfo, channel: u8) -> AutomationTarget {
    match p.midi {
        MidiBinding::Cc(cc) => AutomationTarget::CC { controller: cc },
        _ => AutomationTarget::Param {
            device: ParamDevice::ChannelInstrument { channel },
            id: p.id,
            name: String::new(),
        },
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

    fn find(table: &[BuiltinParamInfo], id: u32) -> &BuiltinParamInfo {
        table.iter().find(|p| p.id == id).expect("参数表应含该 id")
    }

    /// 内置参数映射：CC 绑定 → 低层 CC lane；PB/RPN → 设备参数。
    #[test]
    fn builtin_target_splits_cc_and_device_params() {
        let inst = |id| param(ParamDevice::ChannelInstrument { channel: 5 }, id);

        assert_eq!(
            builtin_target(find(XSYNTH_PARAMS, xsynth_param::SUSTAIN), 5),
            AutomationTarget::CC { controller: 64 }
        );
        assert_eq!(
            builtin_target(find(XSYNTH_PARAMS, xsynth_param::PITCH_BEND), 5),
            inst(xsynth_param::PITCH_BEND)
        );
        // 通道 DSP 参数（Volume=CC7）全部是 CC 绑定，映射为低层 CC lane。
        assert_eq!(
            builtin_target(find(CHANNEL_DSP_PARAMS, channel_dsp_param::VOLUME), 5),
            AutomationTarget::CC { controller: 7 }
        );
    }

    /// 设备参数列表按通道生成：Tempo + CC 绑定参数（低层 lane）+ PB/RPN 设备参数。
    #[test]
    fn automation_targets_lists_builtin_params() {
        let targets = automation_targets(Some(5));
        assert_eq!(targets[0], AutomationTarget::Tempo);
        let cc_bound = XSYNTH_PARAMS
            .iter()
            .chain(CHANNEL_DSP_PARAMS)
            .filter(|p| matches!(p.midi, MidiBinding::Cc(_)))
            .count();
        let device_params = XSYNTH_PARAMS
            .iter()
            .filter(|p| !matches!(p.midi, MidiBinding::Cc(_)))
            .count();
        assert_eq!(targets.len(), 1 + cc_bound + device_params);

        // XSynth（64/72/73）与通道 DSP（7/11/10/74/71）都列低层 CC。
        for cc in [64u8, 72, 73, 7, 11, 10, 74, 71] {
            assert!(
                targets.contains(&AutomationTarget::CC { controller: cc }),
                "CC{cc} 应列出"
            );
        }
        // PB/RPN 仍是设备参数。
        assert!(targets.contains(&param(
            ParamDevice::ChannelInstrument { channel: 5 },
            xsynth_param::PITCH_BEND
        )));
        assert!(targets.contains(&param(
            ParamDevice::ChannelInstrument { channel: 5 },
            xsynth_param::PB_SENSITIVITY
        )));
        assert!(
            !targets.contains(&param(
                ParamDevice::ChannelInstrument { channel: 5 },
                xsynth_param::SUSTAIN
            )),
            "CC 绑定参数不再生成设备参数条目"
        );
        assert_eq!(automation_targets(None), vec![AutomationTarget::Tempo]);
    }
}
