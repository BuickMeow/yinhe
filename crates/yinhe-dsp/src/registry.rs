//! 内置效果器注册表。
//!
//! UI 通过 [`BuiltinEffectKind::ALL`] 列出内置效果器；工程持久化只存
//! [`BuiltinEffectKind::id`]（`InsertRef.plugin_id`），加载时 `from_id` 找回。

use yinhe_mixer::InsertProcessor;

use crate::cc::filter::ChannelFilter;
use crate::cc::gain::ChannelGain;
use crate::cc::pan::ChannelPan;

/// 内置效果器的一个参数（UI 显示 + 底层映射）。
///
/// 命名约定：这些是**效果器自己的参数**（Volume/Cutoff/…），
/// 只是在工程底层**伪装成 MIDI CC**（用 CC lane 存储、导出 MIDI 时按 CC 写出）。
/// UI 与文档中不得称其为"CC"。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectParamInfo {
    /// 参数显示名（效果器语义）。
    pub name: &'static str,
    /// 底层伪 CC 号（存储/导出映射，内部实现细节）。
    pub cc: u8,
    /// 值域上限（0..max）。
    pub max: f32,
    /// 默认值（无 lane 事件时采用；GM 标准默认）。
    pub default: f32,
}

/// 内置效果器种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinEffectKind {
    /// 通道音量/表情。
    ChannelGain,
    /// 通道声像。
    ChannelPan,
    /// 通道低通滤波。
    ChannelFilter,
}

impl BuiltinEffectKind {
    /// 全部内置效果器（UI 选择器展示顺序）。
    pub const ALL: &'static [BuiltinEffectKind] = &[
        BuiltinEffectKind::ChannelGain,
        BuiltinEffectKind::ChannelPan,
        BuiltinEffectKind::ChannelFilter,
    ];

    /// 持久化标识（`InsertRef.plugin_id`）。
    pub const fn id(self) -> &'static str {
        match self {
            BuiltinEffectKind::ChannelGain => "channel_gain",
            BuiltinEffectKind::ChannelPan => "channel_pan",
            BuiltinEffectKind::ChannelFilter => "channel_filter",
        }
    }

    /// 由持久化标识找回种类。
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.id() == id)
    }

    /// 显示名。
    pub const fn name(self) -> &'static str {
        match self {
            BuiltinEffectKind::ChannelGain => "Channel Gain",
            BuiltinEffectKind::ChannelPan => "Channel Pan",
            BuiltinEffectKind::ChannelFilter => "Channel Filter",
        }
    }

    /// 功能说明（UI 选择器副标题）。
    pub const fn description(self) -> &'static str {
        match self {
            BuiltinEffectKind::ChannelGain => "Channel volume & expression",
            BuiltinEffectKind::ChannelPan => "Equal-power pan",
            BuiltinEffectKind::ChannelFilter => "Channel low-pass filter",
        }
    }

    /// 参数表（UI 旋钮 + 底层伪 CC 映射）。
    pub const fn params(self) -> &'static [EffectParamInfo] {
        match self {
            BuiltinEffectKind::ChannelGain => &[
                EffectParamInfo {
                    name: "Volume",
                    cc: 7,
                    max: 127.0,
                    default: 127.0,
                },
                EffectParamInfo {
                    name: "Expression",
                    cc: 11,
                    max: 127.0,
                    default: 127.0,
                },
            ],
            BuiltinEffectKind::ChannelPan => &[EffectParamInfo {
                name: "Pan",
                cc: 10,
                max: 127.0,
                default: 64.0,
            }],
            BuiltinEffectKind::ChannelFilter => &[
                EffectParamInfo {
                    name: "Cutoff",
                    cc: 74,
                    max: 127.0,
                    default: 64.0,
                },
                EffectParamInfo {
                    name: "Resonance",
                    cc: 71,
                    max: 127.0,
                    default: 64.0,
                },
            ],
        }
    }

    /// 底层伪 CC 列表（dispatch 分发与广播过滤用的内部映射；
    /// 不得用于 UI 显示）。
    pub const fn handled_ccs(self) -> &'static [u8] {
        match self {
            BuiltinEffectKind::ChannelGain => &[7, 11],
            BuiltinEffectKind::ChannelPan => &[10],
            BuiltinEffectKind::ChannelFilter => &[71, 74],
        }
    }

    /// 构造处理器（激活时调用，传入引擎采样率）。
    pub fn build(self, sample_rate: u32) -> Box<dyn InsertProcessor> {
        match self {
            BuiltinEffectKind::ChannelGain => Box::new(ChannelGain::new(sample_rate)),
            BuiltinEffectKind::ChannelPan => Box::new(ChannelPan::new(sample_rate)),
            BuiltinEffectKind::ChannelFilter => Box::new(ChannelFilter::new(sample_rate)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrip() {
        for kind in BuiltinEffectKind::ALL {
            assert_eq!(BuiltinEffectKind::from_id(kind.id()), Some(*kind));
        }
        assert_eq!(BuiltinEffectKind::from_id("nope"), None);
    }

    #[test]
    fn dsp_channel_ccs_matches_module_union() {
        // 白名单（dispatch 直发）必须等于所有模块 handled_ccs 的并集，
        // 否则新增模块/改白名单时会漏发或错发。
        let mut union: Vec<u8> = Vec::new();
        for kind in BuiltinEffectKind::ALL {
            for &cc in kind.handled_ccs() {
                if !union.contains(&cc) {
                    union.push(cc);
                }
            }
        }
        union.sort_unstable();
        let mut expected = crate::cc::DSP_CHANNEL_CCS.to_vec();
        expected.sort_unstable();
        assert_eq!(
            union, expected,
            "DSP_CHANNEL_CCS 与模块 handled_ccs 并集不同步"
        );
    }

    #[test]
    fn params_map_to_handled_ccs() {
        // 参数表的底层伪 CC 必须与 handled_ccs 覆盖同一集合（顺序无关：
        // 参数表用于 UI 显示顺序，handled_ccs 用于分发过滤）。
        for kind in BuiltinEffectKind::ALL {
            let mut from_params: Vec<u8> = kind.params().iter().map(|p| p.cc).collect();
            let mut handled = kind.handled_ccs().to_vec();
            from_params.sort_unstable();
            handled.sort_unstable();
            assert_eq!(
                from_params,
                handled,
                "{} 参数映射与 handled_ccs 不一致",
                kind.name()
            );
        }
    }
}
