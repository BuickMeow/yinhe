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
}

impl PanelKind {
    /// 选项卡标题 i18n key。
    pub fn label_key(self) -> &'static str {
        match self {
            PanelKind::Track => "panel.section.track",
            PanelKind::Layers => "panel.section.layers",
            PanelKind::ProjectTree => "panel.section.tree",
            PanelKind::EventBrowser => "panel.section.events",
            PanelKind::History => "panel.tab.history",
            PanelKind::Summary => "panel.tab.summary",
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
        // 默认：上[音轨] / 中[树图,事件,图层] / 下[历史记录,属性概要]。
        Self {
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
                PanelColumn {
                    tabs: vec![PanelKind::History, PanelKind::Summary],
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

    /// 在 `column_idx` 栏内追加选项卡并选中（若已存在于别处先移除）。
    pub fn insert_tab(&mut self, kind: PanelKind, column_idx: usize) {
        self.remove_tab(kind);
        let idx = column_idx.min(self.columns.len().saturating_sub(1));
        if let Some(col) = self.columns.get_mut(idx) {
            col.tabs.push(kind);
            col.active = col.tabs.len() - 1;
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
        assert_eq!(l.columns[0].tabs, vec![PanelKind::Track]);
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
            vec![PanelKind::History, PanelKind::Summary]
        );
    }

    #[test]
    fn insert_tab_moves_between_columns() {
        let mut l = RightPanelLayout::default();
        // 把 History 从第 3 栏移入第 1 栏。
        l.insert_tab(PanelKind::History, 0);
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
    fn split_new_column_inserts_at_position() {
        let mut l = RightPanelLayout::default();
        l.split_new_column(PanelKind::EventBrowser, 1);
        assert_eq!(l.columns.len(), 4);
        assert_eq!(l.columns[1].tabs, vec![PanelKind::EventBrowser]);
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
