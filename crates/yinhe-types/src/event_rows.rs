//! 全局事件行（Marker / Tempo / Meter / Key / Chord / Lyrics）的视图状态。
//!
//! 行本身不是音轨行：不进 `ArRowLayout`，不参与轨道行号计算，固定显示在
//! 时间标尺与音轨/钢琴卷帘之间。详见 `docs/spec-global-event-rows.md`。

/// 全局事件行的种类。顺序即默认显示顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventRowKind {
    Marker,
    Tempo,
    Meter,
    Key,
    Chord,
    Lyrics,
}

impl EventRowKind {
    /// 默认顺序：Marker → Tempo → Meter → Key → Chord → Lyrics。
    pub const ALL: [Self; 6] = [
        Self::Marker,
        Self::Tempo,
        Self::Meter,
        Self::Key,
        Self::Chord,
        Self::Lyrics,
    ];

    /// 行头短标签（UI 使用英文小标签，避免为每个行新增 15 语言 i18n key）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Marker => "MARKER",
            Self::Tempo => "TEMPO",
            Self::Meter => "METER",
            Self::Key => "KEY",
            Self::Chord => "CHORD",
            Self::Lyrics => "LYRIC",
        }
    }
}

/// 单行状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventRowState {
    pub kind: EventRowKind,
    pub visible: bool,
}

/// 全局事件行的会话态。
///
/// 不进入 undo、不写入事件数据；只控制固定行区的显示高度。
#[derive(Clone, Debug, PartialEq)]
pub struct EventRowsState {
    pub rows: Vec<EventRowState>,
    /// 每行高度（px）。v1 固定，不逐行配置。
    pub row_height: f32,
}

impl Default for EventRowsState {
    fn default() -> Self {
        Self {
            rows: EventRowKind::ALL
                .iter()
                .copied()
                .map(|kind| EventRowState {
                    kind,
                    visible: true,
                })
                .collect(),
            row_height: 18.0,
        }
    }
}

impl EventRowsState {
    /// 可见行总高度（px）。
    pub fn visible_height(&self) -> f32 {
        self.visible_count() as f32 * self.row_height
    }

    pub fn visible_count(&self) -> usize {
        self.rows.iter().filter(|r| r.visible).count()
    }

    pub fn visible_rows(&self) -> impl Iterator<Item = EventRowKind> + '_ {
        self.rows.iter().filter(|r| r.visible).map(|r| r.kind)
    }

    pub fn is_visible(&self, kind: EventRowKind) -> bool {
        self.rows
            .iter()
            .find(|r| r.kind == kind)
            .is_some_and(|r| r.visible)
    }

    /// 切换某行显示。用于后续 View 菜单，不影响事件数据。
    pub fn toggle(&mut self, kind: EventRowKind) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.kind == kind) {
            row.visible = !row.visible;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shows_all_rows_in_order() {
        let state = EventRowsState::default();
        assert_eq!(state.visible_count(), EventRowKind::ALL.len());
        assert_eq!(
            state.visible_rows().collect::<Vec<_>>(),
            EventRowKind::ALL.to_vec()
        );
        assert_eq!(state.visible_height(), 6.0 * state.row_height);
    }

    #[test]
    fn toggle_hides_and_shows() {
        let mut state = EventRowsState::default();
        state.toggle(EventRowKind::Lyrics);
        assert!(!state.is_visible(EventRowKind::Lyrics));
        assert_eq!(state.visible_count(), 5);
        state.toggle(EventRowKind::Lyrics);
        assert!(state.is_visible(EventRowKind::Lyrics));
    }

    #[test]
    fn zero_height_when_all_hidden() {
        let mut state = EventRowsState::default();
        for kind in EventRowKind::ALL {
            state.toggle(kind);
        }
        assert_eq!(state.visible_height(), 0.0);
    }
}
