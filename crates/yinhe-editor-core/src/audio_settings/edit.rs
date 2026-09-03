use serde::{Deserialize, Serialize};

/// 快速删除音符的方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum QuickDeleteMode {
    #[default]
    Off,
    DoubleClick,
    RightClick,
    Both,
}

impl QuickDeleteMode {
    pub fn allows_double_click(self) -> bool {
        matches!(self, Self::DoubleClick | Self::Both)
    }
    pub fn allows_right_click(self) -> bool {
        matches!(self, Self::RightClick | Self::Both)
    }
}

/// 重叠关闭时的处理策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OverlapBlockedBehavior {
    #[default]
    ReplaceTarget,
    DeleteOriginal,
    KeepOriginal,
}

impl OverlapBlockedBehavior {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReplaceTarget => "替换目标",
            Self::DeleteOriginal => "仅删除原",
            Self::KeepOriginal => "退回原位",
        }
    }
}
