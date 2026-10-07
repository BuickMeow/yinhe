//! 右栏多栏选项卡布局模型（可拖拽停靠）。
//!
//! 右栏由若干**栏（[`PanelColumn`]）**垂直堆叠，栏之间用可拖拽分割线分隔，
//! 最少 1 栏、最多不限。每栏是一个**选项卡组**：多个 [`PanelKind`] 横向排布，
//! 一次显示其中选中的那个。
//!
//! 用户拖动选项卡可实现：并入其它栏、在栏之间分裂出新栏、上下重排。
//! 该结构跨会话持久化（见 `LayoutSettings`）。

use serde::{Deserialize, Serialize};

/// 右栏可停靠的面板类型（选项卡）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    /// 音轨属性。
    Track,
    /// 选框批处理（力度/键位/变速/翻转等）。
    Batch,
    /// 图层（每轨选中/可见/锁定）。
    Layers,
    /// 树图（工程总览）。
    ProjectTree,
    /// 事件浏览器表格。
    EventBrowser,
    /// 历史记录。
    History,
    /// 属性概要。
    Summary,
    /// 选框属性（选框位置/数量/跨度等只读信息）。
    Selection,
}

impl PanelKind {
    /// 全部面板类型（用于旧布局迁移补齐）。
    pub const ALL: [PanelKind; 8] = [
        PanelKind::Track,
        PanelKind::Batch,
        PanelKind::ProjectTree,
        PanelKind::EventBrowser,
        PanelKind::Layers,
        PanelKind::History,
        PanelKind::Summary,
        PanelKind::Selection,
    ];

    /// 选项卡标题 i18n key。
    pub fn label_key(self) -> &'static str {
        match self {
            PanelKind::Track => "panel.section.track",
            PanelKind::Batch => "panel.tab.batch",
            PanelKind::Layers => "panel.section.layers",
            PanelKind::ProjectTree => "panel.section.tree",
            PanelKind::EventBrowser => "panel.section.events",
            PanelKind::History => "panel.tab.history",
            PanelKind::Summary => "panel.tab.summary",
            PanelKind::Selection => "panel.tab.selection",
        }
    }
}

/// 一个栏内的选项卡组。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelColumn {
    /// 栏内选项卡（横向排列）。
    pub tabs: Vec<PanelKind>,
    /// 当前选中显示的选项卡下标。
    pub active: usize,
    /// 相对高度权重（同一右栏内按权重分配高度）。
    pub height_weight: f32,
}

impl PanelColumn {
    /// 新建只含一个选项卡的栏。
    pub fn single(kind: PanelKind) -> Self {
        Self {
            tabs: vec![kind],
            active: 0,
            height_weight: 1.0,
        }
    }

    /// 当前选中的选项卡（越界回落首个）。
    pub fn active_kind(&self) -> Option<PanelKind> {
        self.tabs
            .get(self.active)
            .copied()
            .or_else(|| self.tabs.first().copied())
    }
}

/// 右栏多栏布局。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RightPanelLayout {
    pub columns: Vec<PanelColumn>,
}

impl Default for RightPanelLayout {
    fn default() -> Self {
        // 默认：上[音轨,批处理] / 中[树图,事件,图层] / 下[历史记录,属性概要,选框属性]。
        Self {
            columns: vec![
                PanelColumn {
                    tabs: vec![PanelKind::Track, PanelKind::Batch],
                    active: 0,
                    height_weight: 1.0,
                },
                PanelColumn {
                    tabs: vec![
                        PanelKind::ProjectTree,
                        PanelKind::EventBrowser,
                        PanelKind::Layers,
                    ],
                    active: 0,
                    height_weight: 1.0,
                },
                PanelColumn {
                    tabs: vec![PanelKind::History, PanelKind::Summary, PanelKind::Selection],
                    active: 0,
                    height_weight: 1.0,
                },
            ],
        }
    }
}

impl RightPanelLayout {
    /// 是否任意栏包含该选项卡。
    pub fn contains(&self, kind: PanelKind) -> bool {
        self.columns.iter().any(|c| c.tabs.contains(&kind))
    }

    /// 补齐缺失的选项卡（旧持久化布局迁移用）。返回是否有改动。
    ///
    /// 新增选项卡后，老配置里不会自动出现；补齐保证所有 [`PanelKind`] 都可访问。
    pub fn ensure_all(&mut self) -> bool {
        let mut changed = false;
        for kind in PanelKind::ALL {
            if self.contains(kind) {
                continue;
            }
            match self.sibling_column(kind) {
                Some(i) => self.move_tab(kind, i, usize::MAX),
                None => {
                    let at = match kind {
                        PanelKind::Track | PanelKind::Batch => 0,
                        PanelKind::History | PanelKind::Summary | PanelKind::Selection => {
                            self.columns.len()
                        }
                        _ => self.columns.len().min(1),
                    };
                    self.split_new_column(kind, at);
                }
            }
            changed = true;
        }
        changed
    }

    /// 与 `kind` 同属一组、且已存在的栏下标（用于把缺失选项卡补到同类栏）。
    fn sibling_column(&self, kind: PanelKind) -> Option<usize> {
        let siblings: &[PanelKind] = match kind {
            PanelKind::Layers => &[PanelKind::ProjectTree, PanelKind::EventBrowser],
            PanelKind::ProjectTree | PanelKind::EventBrowser => &[PanelKind::Layers],
            PanelKind::History => &[PanelKind::Summary, PanelKind::Selection],
            PanelKind::Summary => &[PanelKind::History, PanelKind::Selection],
            PanelKind::Selection => &[PanelKind::Summary, PanelKind::History],
            PanelKind::Batch => &[PanelKind::Track],
            PanelKind::Track => &[PanelKind::Batch],
        };
        self.columns
            .iter()
            .position(|c| siblings.iter().any(|s| c.tabs.contains(s)))
    }

    /// 把某选项卡从所有栏移除；返回其原所属栏下标。
    /// 移除后空栏被删除；保证至少剩 1 栏。
    pub fn remove_tab(&mut self, kind: PanelKind) -> Option<usize> {
        let mut removed_from = None;
        for (i, col) in self.columns.iter_mut().enumerate() {
            if let Some(pos) = col.tabs.iter().position(|&k| k == kind) {
                col.tabs.remove(pos);
                if col.active >= col.tabs.len() {
                    col.active = col.tabs.len().saturating_sub(1);
                }
                removed_from = Some(i);
                break;
            }
        }
        self.columns.retain(|c| !c.tabs.is_empty());
        if self.columns.is_empty() {
            self.columns.push(PanelColumn::single(PanelKind::Track));
        }
        removed_from
    }

    /// 把 `kind` 移到 `column_idx` 栏的 `insert_idx` 位置（同栏内即重排）。
    ///
    /// `insert_idx` 按**移动前**该栏的选项卡顺序计算（0..=len）；移除源选项卡后
    /// 会自动修正插入点与目标栏下标（源栏可能因搬空而被删除）。
    pub fn move_tab(&mut self, kind: PanelKind, mut column_idx: usize, mut insert_idx: usize) {
        let src = self.columns.iter().position(|c| c.tabs.contains(&kind));
        let src_pos = src.map(|ci| {
            self.columns[ci]
                .tabs
                .iter()
                .position(|&k| k == kind)
                .unwrap_or(0)
        });
        if let (Some(si), Some(sp)) = (src, src_pos) {
            let src_single = self.columns[si].tabs.len() == 1;
            // 同栏：移除后插入点前移。
            if si == column_idx && sp < insert_idx {
                insert_idx = insert_idx.saturating_sub(1);
            }
            // 源栏被搬空删除后，目标栏下标前移。
            if src_single && si < column_idx {
                column_idx = column_idx.saturating_sub(1);
            }
        }
        self.remove_tab(kind);
        let idx = column_idx.min(self.columns.len().saturating_sub(1));
        if let Some(col) = self.columns.get_mut(idx) {
            let pos = insert_idx.min(col.tabs.len());
            col.tabs.insert(pos, kind);
            col.active = pos;
        } else {
            self.columns.push(PanelColumn::single(kind));
        }
    }

    /// 在 `at` 处分裂出新栏，仅含 `kind`（若已存在于别处先移除）。
    pub fn split_new_column(&mut self, kind: PanelKind, at: usize) {
        self.remove_tab(kind);
        let at = at.min(self.columns.len());
        self.columns.insert(at, PanelColumn::single(kind));
    }

    /// 重新选中的选项卡（供 Ctrl/菜单快速定位）。
    pub fn focus_tab(&mut self, kind: PanelKind) {
        for _ in 0..self.columns.len() {
            if self.set_active(kind) {
                return;
            }
        }
    }

    fn set_active(&mut self, kind: PanelKind) -> bool {
        for col in self.columns.iter_mut() {
            if let Some(pos) = col.tabs.iter().position(|&k| k == kind) {
                col.active = pos;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_three_columns_with_expected_tabs() {
        let l = RightPanelLayout::default();
        assert_eq!(l.columns.len(), 3);
        assert_eq!(l.columns[0].tabs, vec![PanelKind::Track, PanelKind::Batch]);
        assert_eq!(
            l.columns[1].tabs,
            vec![
                PanelKind::ProjectTree,
                PanelKind::EventBrowser,
                PanelKind::Layers
            ]
        );
        assert_eq!(
            l.columns[2].tabs,
            vec![PanelKind::History, PanelKind::Summary, PanelKind::Selection]
        );
    }

    #[test]
    fn move_tab_moves_between_columns() {
        let mut l = RightPanelLayout::default();
        // 把 History 从第 3 栏移到第 1 栏末尾。
        l.move_tab(PanelKind::History, 0, usize::MAX);
        assert!(l.columns[0].tabs.contains(&PanelKind::History));
        // History 应只剩一处。
        let count = l
            .columns
            .iter()
            .filter(|c| c.tabs.contains(&PanelKind::History))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn move_tab_reorders_within_column() {
        let mut l = RightPanelLayout::default();
        // 中间栏 [Tree, Events, Layers]：把 Layers 移到最前。
        l.move_tab(PanelKind::Layers, 1, 0);
        assert_eq!(
            l.columns[1].tabs,
            vec![
                PanelKind::Layers,
                PanelKind::ProjectTree,
                PanelKind::EventBrowser
            ]
        );
        assert_eq!(l.columns[1].active, 0);
    }

    #[test]
    fn move_tab_moves_single_tab_column_forward() {
        // 自造一个单选项卡栏，验证搬空后源栏消失、目标栏下标前移。
        let mut l = RightPanelLayout {
            columns: vec![
                PanelColumn::single(PanelKind::Track),
                PanelColumn {
                    tabs: vec![
                        PanelKind::ProjectTree,
                        PanelKind::EventBrowser,
                        PanelKind::Layers,
                    ],
                    active: 0,
                    height_weight: 1.0,
                },
            ],
        };
        l.move_tab(PanelKind::Track, 1, 3);
        assert_eq!(l.columns.len(), 1);
        assert_eq!(
            l.columns[0].tabs,
            vec![
                PanelKind::ProjectTree,
                PanelKind::EventBrowser,
                PanelKind::Layers,
                PanelKind::Track
            ]
        );
    }

    #[test]
    fn split_new_column_inserts_at_position() {
        let mut l = RightPanelLayout::default();
        l.split_new_column(PanelKind::EventBrowser, 1);
        assert_eq!(l.columns.len(), 4);
        assert_eq!(l.columns[1].tabs, vec![PanelKind::EventBrowser]);
    }

    #[test]
    fn ensure_all_adds_missing_layers() {
        let mut l = RightPanelLayout {
            columns: vec![
                PanelColumn::single(PanelKind::Track),
                PanelColumn {
                    tabs: vec![PanelKind::ProjectTree, PanelKind::EventBrowser],
                    active: 0,
                    height_weight: 1.0,
                },
                PanelColumn {
                    tabs: vec![PanelKind::History, PanelKind::Summary],
                    active: 0,
                    height_weight: 1.0,
                },
            ],
        };
        assert!(l.ensure_all());
        assert_eq!(
            l.columns[1].tabs,
            vec![
                PanelKind::ProjectTree,
                PanelKind::EventBrowser,
                PanelKind::Layers
            ]
        );
        // 已齐全时不再改动。
        assert!(!l.ensure_all());
    }

    #[test]
    fn remove_last_tab_keeps_one_column() {
        let mut l = RightPanelLayout {
            columns: vec![PanelColumn::single(PanelKind::Track)],
        };
        l.remove_tab(PanelKind::Track);
        assert_eq!(l.columns.len(), 1);
        assert_eq!(l.columns[0].tabs, vec![PanelKind::Track]);
    }

    #[test]
    fn focus_tab_selects_within_column() {
        let mut l = RightPanelLayout::default();
        l.focus_tab(PanelKind::EventBrowser);
        assert_eq!(l.columns[1].active, 1);
    }

    #[test]
    fn roundtrip_serde() {
        let l = RightPanelLayout::default();
        let json = serde_json::to_string(&l).unwrap();
        let back: RightPanelLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(l, back);
    }
}
