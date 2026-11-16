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
    /// 音轨面板内联重命名提交。
    RenameTrack { idx: usize, name: String },
    /// AR 轨道头 M/S 手势结束：before/after 已应用，只需补一条 undo。
    CommitMuteSolo {
        before: Vec<yinhe_editor_core::document::TrackOverride>,
        after: Vec<yinhe_editor_core::document::TrackOverride>,
    },
    /// 展开/折叠文件夹。
    ToggleFolder { idx: usize },
    /// 把选中轨归入新文件夹。
    GroupTracks { indices: Vec<usize> },
    /// 解散文件夹。
    UngroupTrack { idx: usize },
    /// 固定总线行 M/S：写入总线 strip 参数。
    SetBusStrip {
        bus: yinhe_types::BusId,
        params: yinhe_mixer::StripParams,
    },
    /// 双击固定总线行：打开该总线的设备链。
    OpenBusDock { bus: yinhe_types::BusId },
    /// 右键固定总线行：删除该总线。
    RemoveBus { bus: yinhe_types::BusId },
    /// 右键 Master 轨：打开 Master 设备链。
    OpenMasterDock,
    /// 右键固定 Bus 行：添加总线增益自动化。
    AddBusAutomation { bus: yinhe_types::BusId },
}
