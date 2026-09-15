//! 内置效果器注册表。
//!
//! UI 通过 [`BuiltinEffectKind::ALL`] 列出内置效果器；工程持久化只存
//! [`BuiltinEffectKind::id`]（`InsertRef.plugin_id`），加载时 `from_id` 找回。

use yinhe_mixer::InsertProcessor;

use crate::cc::gain::ChannelGain;

/// 内置效果器种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinEffectKind {
    /// 通道音量/表情（CC7/11）。
    ChannelGain,
}

impl BuiltinEffectKind {
    /// 全部内置效果器（UI 选择器展示顺序）。
    pub const ALL: &'static [BuiltinEffectKind] = &[BuiltinEffectKind::ChannelGain];

    /// 持久化标识（`InsertRef.plugin_id`）。
    pub const fn id(self) -> &'static str {
        match self {
            BuiltinEffectKind::ChannelGain => "channel_gain",
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
        }
    }

    /// 功能说明（UI 选择器副标题）。
    pub const fn description(self) -> &'static str {
        match self {
            BuiltinEffectKind::ChannelGain => "CC7/CC11 channel volume & expression",
        }
    }

    /// 接管的 CC 列表（UI 提示用）。
    pub const fn handled_ccs(self) -> &'static [u8] {
        match self {
            BuiltinEffectKind::ChannelGain => &[7, 11],
        }
    }

    /// 构造处理器（激活时调用，传入引擎采样率）。
    pub fn build(self, sample_rate: u32) -> Box<dyn InsertProcessor> {
        match self {
            BuiltinEffectKind::ChannelGain => Box::new(ChannelGain::new(sample_rate)),
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
