//! 应用级剪贴板。
//!
//! 音符剪贴板只存「选区矩形 + 复制那一刻的模型快照」。快照是
//! `Arc<YinModel>`：clone 是 O(1)，未编辑的音符桶与源文档结构共享，
//! 不额外占内存；源文档之后的编辑/删除/关闭都不改变粘贴内容。
//! 这正是「选框省内存」与「快照语义」的兼顾：复制 O(1)，
//! 实际数据拷贝推迟到源桶第一次被编辑时的 copy-on-write。

use std::sync::Arc;

use yinhe_core::{Selection, YinModel};
use yinhe_types::{AutomationTarget, Note, SegmentShape};

/// 音符剪贴板：复制时刻的模型快照 + 选区。
#[derive(Clone)]
pub struct NotesClipboard {
    /// 复制那一刻的模型（结构共享，O(1) clone）。
    pub snapshot: Arc<YinModel>,
    /// 复制时的选区（决定从快照里取哪些音符）。
    pub selection: Selection,
}

impl NotesClipboard {
    /// 从快照收集选中音符（不查询当前文档）。
    pub fn collect(&self) -> Vec<(Note, u8)> {
        crate::batch_ops::collect_selected(&self.snapshot, &self.selection)
    }
}

/// 单个自动化面板复制的内容。
#[derive(Clone, Debug)]
pub struct AutomationClip {
    /// 锚点所属 target。
    pub target: AutomationTarget,
    /// `(tick, value, shape)`，按 tick 升序。
    pub events: Vec<(u32, f32, SegmentShape)>,
}

/// 自动化剪贴板：可同时包含多个面板的锚点。
#[derive(Clone, Debug, Default)]
pub struct AutomationClipboard {
    pub clips: Vec<AutomationClip>,
}

/// 粘贴放置方式（音符与自动化共用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PasteMode {
    /// 对齐光标（默认）。
    #[default]
    AtCursor,
    /// 原位粘贴：保持源 tick / 轨道坐标，忽略光标与目标轨道。
    AtOriginal,
    /// 时间镜像粘贴：以光标为基准水平翻转内容（tick 轴镜像，value/key 不变）。
    Flipped,
}

/// 应用级剪贴板内容。音符与自动化互斥：最后一次复制决定内容类型，
/// 粘贴按内容类型分派（不再依赖「当前有无锚点选中」猜测）。
#[derive(Clone, Default)]
pub enum ClipboardContent {
    #[default]
    Empty,
    Notes(NotesClipboard),
    Automation(AutomationClipboard),
}

impl ClipboardContent {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
}
