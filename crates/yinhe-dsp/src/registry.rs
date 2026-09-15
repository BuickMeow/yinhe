//! 内置效果器注册表。
//!
//! UI 通过 [`BuiltinEffectKind::ALL`] 列出内置效果器；工程持久化只存
//! [`BuiltinEffectKind::id`]（`InsertRef.plugin_id`），加载时 `from_id` 找回。

use yinhe_mixer::InsertProcessor;

use crate::cc::filter::ChannelFilter;
use crate::cc::gain::ChannelGain;
use crate::cc::pan::ChannelPan;

/// 内置效果器种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinEffectKind {
    /// 通道音量/表情（CC7/11）。
    ChannelGain,
    /// 通道声像（CC10）。
    ChannelPan,
    /// 通道低通滤波（CC71/74）。
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
            BuiltinEffectKind::ChannelGain => "CC7/CC11 channel volume & expression",
            BuiltinEffectKind::ChannelPan => "CC10 equal-power pan",
            BuiltinEffectKind::ChannelFilter => "CC71/CC74 channel low-pass filter",
        }
    }

    /// 接管的 CC 列表（UI 提示用）。
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
}
