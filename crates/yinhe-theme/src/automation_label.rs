//! 自动化目标的显示文案（下拉 / AR / 事件浏览器共用）。
//!
//! 属于展示层：`yinhe-types::AutomationTarget` 只描述语义，文案在这里，
//! 便于 i18n 与跨客户端（egui / flutter）共用。

use yinhe_types::automation::{
    AutomationTarget, MidiBinding, ParamDevice, builtin_param, cc_name, rpn_name,
};

/// 给 `AutomationTarget` 提供 `display_name()`。
pub trait AutomationTargetLabel {
    fn display_name(&self) -> String;
}

impl AutomationTargetLabel for AutomationTarget {
    fn display_name(&self) -> String {
        match self {
            AutomationTarget::Param { device, id, name } => match builtin_param(device, *id) {
                // 内置参数按 MIDI 绑定补前缀：RPN 参数显示为 "RPN nnn 名称"
                //（如 "RPN 000 PB Sensitivity"），CC/PB 直接用参数名。
                Some(p) => match p.midi {
                    MidiBinding::Rpn(n) => format!("RPN {:03} {}", n, p.name),
                    _ => p.name.to_string(),
                },
                None if !name.is_empty() => name.clone(),
                None => match device {
                    ParamDevice::PluginInstrument { channel, .. } => {
                        format!("Plugin Param {id} (ch {channel})")
                    }
                    _ => format!("Param {id}"),
                },
            },
            AutomationTarget::CC { controller } => {
                let name = cc_name(*controller);
                if name.is_empty() {
                    format!("CC {:03}", controller)
                } else {
                    format!("CC {:03} {}", controller, name)
                }
            }
            AutomationTarget::PitchBend => "Pitch Bend".into(),
            AutomationTarget::Rpn { parameter } => {
                let name = rpn_name(*parameter);
                if name.is_empty() {
                    format!("RPN {:03}", parameter)
                } else {
                    format!("RPN {:03} {}", parameter, name)
                }
            }
            AutomationTarget::Nrpn { parameter } => format!("NRPN {:03}", parameter),
            AutomationTarget::Tempo => "Tempo".into(),
            AutomationTarget::MixerGain { channel } => format!("Mixer Gain {:?}", channel),
            AutomationTarget::MixerPan { channel } => format!("Mixer Pan {:?}", channel),
            AutomationTarget::MixerSend { source, target } => {
                format!("Mixer Send {:?} -> {:?}", source, target)
            }
            AutomationTarget::InsertParam {
                target,
                slot,
                id,
                name,
            } => {
                if name.is_empty() {
                    format!("Insert {:?}#{} Param {}", target, slot, id)
                } else {
                    format!("Insert {:?}#{} {}", target, slot, name)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::automation::{SegmentShape, builtin_synth_param};

    fn param(device: &ParamDevice, id: u32) -> AutomationTarget {
        AutomationTarget::Param {
            device: device.clone(),
            id,
            name: String::new(),
        }
    }

    #[test]
    fn builtin_and_plugin_param_labels() {
        let xs = ParamDevice::ChannelInstrument { channel: 0 };
        assert_eq!(
            param(&xs, builtin_synth_param::PITCH_BEND).display_name(),
            "Pitch Bend"
        );
        assert_eq!(
            param(&xs, builtin_synth_param::PB_SENSITIVITY).display_name(),
            "RPN 000 PB Sensitivity"
        );
        let plug = AutomationTarget::Param {
            device: ParamDevice::PluginInstrument { channel: 2, uid: 1 },
            id: 42,
            name: "Cutoff".into(),
        };
        assert_eq!(plug.display_name(), "Cutoff");
        let unnamed = AutomationTarget::Param {
            device: ParamDevice::PluginInstrument { channel: 2, uid: 1 },
            id: 42,
            name: String::new(),
        };
        assert_eq!(unnamed.display_name(), "Plugin Param 42 (ch 2)");
    }

    #[test]
    fn low_level_target_labels() {
        assert_eq!(
            AutomationTarget::CC { controller: 7 }.display_name(),
            "CC 007 Volume"
        );
        assert_eq!(
            AutomationTarget::CC { controller: 10 }.display_name(),
            "CC 010 Pan"
        );
        assert_eq!(
            AutomationTarget::CC { controller: 3 }.display_name(),
            "CC 003"
        );
        assert_eq!(AutomationTarget::PitchBend.display_name(), "Pitch Bend");
        assert_eq!(
            AutomationTarget::Rpn { parameter: 5 }.display_name(),
            "RPN 005"
        );
        assert_eq!(
            AutomationTarget::Rpn { parameter: 0 }.display_name(),
            "RPN 000 PB Sensitivity"
        );
        assert_eq!(
            AutomationTarget::Nrpn { parameter: 1 }.display_name(),
            "NRPN 001"
        );
        assert_eq!(AutomationTarget::Tempo.display_name(), "Tempo");
        // 保持旧行为：混音节点用 Debug 形式。
        assert_eq!(
            AutomationTarget::MixerGain {
                channel: yinhe_types::MixerChannelId::Midi(0)
            }
            .display_name(),
            "Mixer Gain Midi(0)"
        );
        assert_eq!(
            AutomationTarget::CC { controller: 7 }.default_shape(),
            SegmentShape::linear_curve()
        );
    }
}
