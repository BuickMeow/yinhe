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

/// 音符剪贴板的数据来源。
#[derive(Clone)]
pub enum NotesClipboardData {
    /// 同实例复制：模型结构共享快照 + 选区，O(1) 且延迟查询。
    Snapshot {
        snapshot: Arc<YinModel>,
        selection: Selection,
    },
    /// 从系统剪贴板文件加载的外来数据（已物化）。
    Materialized(Vec<(Note, u8)>),
}

/// 音符剪贴板。
#[derive(Clone)]
pub struct NotesClipboard {
    pub data: NotesClipboardData,
}

impl NotesClipboard {
    /// 同实例复制：O(1) 的结构共享快照。
    pub fn from_snapshot(snapshot: Arc<YinModel>, selection: Selection) -> Self {
        Self {
            data: NotesClipboardData::Snapshot {
                snapshot,
                selection,
            },
        }
    }

    /// 跨实例加载：已物化的音符列表。
    pub fn from_materialized(notes: Vec<(Note, u8)>) -> Self {
        Self {
            data: NotesClipboardData::Materialized(notes),
        }
    }

    /// 收集选中音符（快照从模型查询，物化数据直接克隆）。
    pub fn collect(&self) -> Vec<(Note, u8)> {
        match &self.data {
            NotesClipboardData::Snapshot {
                snapshot,
                selection,
            } => crate::batch_ops::collect_selected(snapshot, selection),
            NotesClipboardData::Materialized(notes) => notes.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.data {
            NotesClipboardData::Snapshot { selection, .. } => selection.is_empty(),
            NotesClipboardData::Materialized(notes) => notes.is_empty(),
        }
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
