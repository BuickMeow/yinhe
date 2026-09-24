//! 应用级剪贴板。
//!
//! 音符与自动化剪贴板都只存「选择范围 + 复制那一刻的结构共享快照」。
//! 音符快照是 `Arc<YinModel>`，自动化快照是 `Vec<Arc<TrackData>>` +
//! `Arc<ConductorData>`：clone 都是 O(1)，未编辑的桶/轨道与源文档
//! 结构共享，不额外占内存；源文档之后的编辑/删除/关闭都不改变粘贴
//! 内容。这正是「选框省内存」与「快照语义」的兼顾：复制 O(1)，
//! 实际数据拷贝推迟到源第一次被编辑时的 copy-on-write。

use std::sync::Arc;

use yinhe_core::{ConductorData, Selection, TrackData, YinModel};
use yinhe_types::{AnchorSelRect, AutomationTarget, Note, SegmentShape};

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

    /// 流式遍历选中音符（不物化全量）：大选区粘贴时先扫一遍范围、
    /// 再扫一遍构建，避免 `collect()` 的 3GB 级中间副本。
    pub fn for_each_note(&self, mut f: impl FnMut(Note, u8)) {
        match &self.data {
            NotesClipboardData::Snapshot {
                snapshot,
                selection,
            } => crate::batch_ops::for_each_selected(snapshot, selection, |n, k| f(*n, k)),
            NotesClipboardData::Materialized(notes) => {
                for (n, k) in notes {
                    f(*n, *k);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.data {
            NotesClipboardData::Snapshot { selection, .. } => selection.is_empty(),
            NotesClipboardData::Materialized(notes) => notes.is_empty(),
        }
    }
}

/// 单个自动化面板复制的内容（物化形态）。
#[derive(Clone, Debug)]
pub struct AutomationClip {
    /// 锚点所属 target。
    pub target: AutomationTarget,
    /// `(tick, value, shape)`，按 tick 升序。
    pub events: Vec<(u32, f32, SegmentShape)>,
}

/// 一个面板的自动化选择范围（复制时的选框 + 可选成员身份）。
#[derive(Clone, Debug)]
pub struct AutomationSelection {
    pub target: AutomationTarget,
    pub sel_rects: Vec<AnchorSelRect>,
    /// 锚点成员位图（按 `AutomationEvent.id`）。`Some` = 成员态（AM 框选
    /// 物化后）：复制按 id 精确筛选，不吸收落点处的其他锚点。
    pub members: Option<yinhe_types::NoteBitset>,
}

/// 自动化剪贴板的数据来源。
#[derive(Clone)]
pub enum AutomationClipboardData {
    /// 同实例复制：轨道 / Conductor 结构共享快照 + 各面板选择范围，O(1)。
    Snapshot {
        tracks: Vec<Arc<TrackData>>,
        conductor: Arc<ConductorData>,
        selections: Vec<AutomationSelection>,
    },
    /// 跨实例加载：已物化的事件。
    Materialized(Vec<AutomationClip>),
}

/// 自动化剪贴板：可同时包含多个面板的选择范围。
#[derive(Clone)]
pub struct AutomationClipboard {
    pub data: AutomationClipboardData,
}

impl AutomationClipboard {
    /// 同实例复制：O(1) 的轨道 / Conductor 结构共享快照。
    pub fn from_snapshot(
        tracks: Vec<Arc<TrackData>>,
        conductor: Arc<ConductorData>,
        selections: Vec<AutomationSelection>,
    ) -> Self {
        Self {
            data: AutomationClipboardData::Snapshot {
                tracks,
                conductor,
                selections,
            },
        }
    }

    /// 跨实例加载：已物化的事件列表。
    pub fn from_materialized(clips: Vec<AutomationClip>) -> Self {
        Self {
            data: AutomationClipboardData::Materialized(clips),
        }
    }

    /// 物化复制内容：从快照按选择范围筛出各 target 的锚点事件。
    ///
    /// 返回的 clips 按 target 分组、事件按 tick 升序；没有命中锚点的
    /// 选择范围被跳过。
    pub fn collect(&self) -> Vec<AutomationClip> {
        match &self.data {
            AutomationClipboardData::Snapshot {
                tracks,
                conductor,
                selections,
            } => selections
                .iter()
                .filter_map(|sel| {
                    let events: Vec<(u32, u32, f32, SegmentShape)> =
                        if matches!(sel.target, AutomationTarget::Tempo) {
                            conductor
                                .tempo
                                .events
                                .iter()
                                .map(|e| (e.id, e.tick, e.value, e.shape))
                                .collect()
                        } else {
                            tracks
                                .iter()
                                .flat_map(|t| t.automation_lanes.iter())
                                .find(|l| l.target == sel.target)?
                                .events
                                .iter()
                                .map(|e| (e.id, e.tick, e.value, e.shape))
                                .collect()
                        };
                    let mut hit: Vec<(u32, f32, SegmentShape)> = events
                        .into_iter()
                        .filter(|(id, tick, value, _)| match &sel.members {
                            // 成员态按 id 判定；矩形态回退选框几何。
                            Some(bits) => bits.contains(*id),
                            None => sel.sel_rects.iter().any(|r| r.contains(*tick, *value)),
                        })
                        .map(|(_, tick, value, shape)| (tick, value, shape))
                        .collect();
                    if hit.is_empty() {
                        return None;
                    }
                    hit.sort_by_key(|(t, _, _)| *t);
                    Some(AutomationClip {
                        target: sel.target.clone(),
                        events: hit,
                    })
                })
                .collect(),
            AutomationClipboardData::Materialized(clips) => clips.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.data {
            AutomationClipboardData::Snapshot { selections, .. } => selections.is_empty(),
            AutomationClipboardData::Materialized(clips) => clips.is_empty(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{AutomationEvent, AutomationLane};

    fn lane(target: AutomationTarget, events: Vec<(u32, f32)>) -> AutomationLane {
        AutomationLane {
            target,
            track: 0,
            events: events
                .into_iter()
                .map(|(tick, value)| AutomationEvent {
                    id: 0,
                    tick,
                    value,
                    shape: SegmentShape::Step,
                })
                .collect(),
        }
    }

    fn rect(tick_start: f64, tick_end: f64) -> AnchorSelRect {
        AnchorSelRect {
            tick_start,
            tick_end,
            value_range: None,
        }
    }

    /// 快照剪贴板按各面板选择范围筛选锚点（Tempo 与 CC 各自独立）。
    #[test]
    fn automation_snapshot_collect_filters_by_rects() {
        let cc = AutomationTarget::CC { controller: 74 };
        let mut track = TrackData::new(0, 0);
        track
            .automation_lanes
            .push(lane(cc.clone(), vec![(0, 10.0), (100, 20.0), (200, 30.0)]));
        let tracks = vec![Arc::new(track)];
        let conductor = Arc::new(ConductorData {
            tempo: lane(AutomationTarget::Tempo, vec![(0, 120.0), (500, 140.0)]),
            ..Default::default()
        });
        let selections = vec![
            AutomationSelection {
                members: None,
                target: cc.clone(),
                sel_rects: vec![rect(50.0, 250.0)],
            },
            AutomationSelection {
                members: None,
                target: AutomationTarget::Tempo,
                sel_rects: vec![rect(400.0, 600.0)],
            },
        ];
        let cb = AutomationClipboard::from_snapshot(tracks, conductor, selections);

        let clips = cb.collect();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].events.len(), 2, "tick 100/200 命中 CC 选框");
        assert_eq!(clips[0].events[0].0, 100);
        assert_eq!(clips[1].events.len(), 1, "tick 500 命中 Tempo 选框");
        assert_eq!(clips[1].events[0].1, 140.0);
    }

    /// 快照语义：复制后源轨道被 COW 编辑，剪贴板仍是复制时的值。
    #[test]
    fn automation_snapshot_keeps_source_before_edit() {
        let cc = AutomationTarget::CC { controller: 74 };
        let mut track = TrackData::new(0, 0);
        track
            .automation_lanes
            .push(lane(cc.clone(), vec![(100, 20.0)]));
        let tracks = vec![Arc::new(track)];
        let conductor = Arc::new(ConductorData::default());
        let selections = vec![AutomationSelection {
            members: None,
            target: cc,
            sel_rects: vec![rect(0.0, 1000.0)],
        }];
        let cb = AutomationClipboard::from_snapshot(tracks.clone(), conductor, selections);

        // 模拟文档编辑：Arc::make_mut 触发 COW
        let mut edited = tracks[0].clone();
        Arc::make_mut(&mut edited).automation_lanes[0].events[0].value = 999.0;

        let clips = cb.collect();
        assert_eq!(clips[0].events[0].1, 20.0, "剪贴板应保留复制时的值");
        assert_eq!(edited.automation_lanes[0].events[0].value, 999.0);
    }

    /// 没有锚点命中时该选择范围被跳过。
    #[test]
    fn automation_snapshot_skips_empty_selection() {
        let cc = AutomationTarget::CC { controller: 1 };
        let mut track = TrackData::new(0, 0);
        track
            .automation_lanes
            .push(lane(cc.clone(), vec![(100, 20.0)]));
        let cb = AutomationClipboard::from_snapshot(
            vec![Arc::new(track)],
            Arc::new(ConductorData::default()),
            vec![AutomationSelection {
                members: None,
                target: cc,
                sel_rects: vec![rect(500.0, 600.0)],
            }],
        );
        assert!(cb.collect().is_empty());
    }
}
