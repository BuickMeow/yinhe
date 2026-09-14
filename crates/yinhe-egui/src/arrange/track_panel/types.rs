/// Actions requested by the track panel that need Document access.
#[derive(Clone, Debug)]
pub(crate) enum TrackAction {
    /// Add a new track after the given index (or at end if None)
    AddTrack { after_idx: Option<usize> },
    /// Remove the track at the given index
    RemoveTrack { idx: usize },
    /// Move a track up (swap with previous)
    MoveUp { idx: usize },
    /// Move a track down (swap with next)
    MoveDown { idx: usize },
    /// 拖拽排序：把 `indices`（升序，保持相对顺序）整体移动到
    /// 删除它们后的列表中的 `insert_at` 位置。
    MoveTracks {
        indices: Vec<usize>,
        insert_at: usize,
    },
    /// 「添加自动化」：打开设备参数选择窗口（不改动模型）。
    OpenAutomationPicker { idx: usize },
    /// AM 子行右键「删除自动化」：删除 idx 轨的第 lane_idx 条 lane。
    DeleteAutomation { idx: usize, lane_idx: usize },
    /// 右键「音轨属性」：选中该轨并请求打开属性浮窗（不改动模型）。
    ShowProperties { idx: usize },
}
