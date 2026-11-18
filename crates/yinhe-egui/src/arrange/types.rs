use super::*;

/// Arrange 拖拽偏移量：(tick delta, track 行 delta, alt 复制标志)。
pub(crate) type ArrDragDelta = (i64, i32, bool);

/// Arrange 选框/橡皮擦矩形：(t_start, t_end, track_lo, track_hi)。
pub(crate) type ArrSelRect = (f64, f64, usize, usize);

/// Arrange 视图的只读模型数据（同一帧内不变）。
pub(crate) struct ArrangeData<'a> {
    pub midi: Option<&'a dyn yinhe_types::NoteSource>,
    pub track_visible: &'a [bool],
    pub track_colors: &'a [[f32; 4]],
    pub track_info: &'a [yinhe_core::TrackInfo],
    pub quantize: QuantizePreset,
    pub ppq: u32,
    pub bar_line_data: Option<(u32, u8, u8, &'a [yinhe_types::TimeSigEvent])>,
    pub total_ticks: f64,
    pub num_tracks: usize,
    /// 音轨数据（AM lane 渲染/交互读取 automation_lanes）。
    pub tracks: &'a [std::sync::Arc<yinhe_core::TrackData>],
    /// Conductor 的 Tempo lane（Conductor 主行直显/直编）。
    pub tempo_lane: &'a yinhe_types::AutomationLane,
    /// 每轨 LC / 音频 LC 节点的 automation（与 Track 索引对齐）。
    pub track_mixer_lanes: &'a [Vec<(yinhe_types::ChannelId, Vec<yinhe_types::AutomationLane>)>],
    pub conductor_track_idx: Option<u16>,
    /// Tempo 映射（音频片段秒 ↔ tick 换算）。
    pub tempo_map: &'a yinhe_core::TempoMap,
    /// 音频素材库（波形峰值读取；未解码完成的片段显示占位）。
    pub audio_library: &'a crate::app::audio_library::AudioLibrary,
    /// 工程内嵌音频素材表（片段显示名查询）。
    pub audio_sources: &'a [std::sync::Arc<yinhe_core::AudioSource>],
    /// Clip 功能总开关（设置里可全局关闭）。
    pub clip_enabled: bool,
}

/// Arrange 交互产生的可变编辑状态（out-params 聚合）。
pub(crate) struct ArrangeEdit<'a> {
    pub selected: &'a mut yinhe_core::Selection,
    pub cursor_tick: &'a mut Option<f64>,
    pub arr_sel_rect: &'a mut Vec<ArrSelRect>,
    pub arr_drag_delta: &'a mut Option<ArrDragDelta>,
    pub arr_eraser_rect: &'a mut Option<ArrSelRect>,
    pub arr_scale_span: &'a mut Option<u64>,
    pub arr_riff_steps: &'a mut Option<(i64, i64)>,
    pub track_selected: &'a mut std::collections::HashSet<u16>,
    pub selection_anchor: &'a mut Option<u16>,
    pub info_content: &'a mut Option<crate::right_panel::InfoContent>,
    /// 每条 AM lane 的持久视图状态（锚点选框等），key = (音轨, target)。
    pub arr_am_views: &'a mut std::collections::HashMap<
        (u16, yinhe_types::AutomationTarget),
        yinhe_types::AutomationPanelView,
    >,
    /// AR 自动化 lane 交互产生的编辑（arrange.rs 在 GPU scope 后统一应用）。
    pub am_edits: &'a mut Vec<yinhe_types::AutomationEdit>,
    /// AM 锚点右键打开信息面板用。
    pub right_tab: &'a mut Option<crate::right_panel::RightTab>,
    /// 已选中的音频片段 (轨道索引, 片段 id)。AR 音频交互读写。
    pub selected_audio_clips: &'a mut std::collections::HashSet<(u16, u32)>,
    /// 音频片段编辑命令（view_ui 产生，本模块统一应用 + undo）。
    pub audio_commands: &'a mut Vec<view_ui::AudioEditCmd>,
    /// MIDI Clip 编辑命令（view_ui 产生，本模块统一应用 + undo）。
    pub clip_commands: &'a mut Vec<view_ui::ClipEditCmd>,
    /// 时间线右键菜单动作（由调用方 `App` 落地）。
    pub menu_actions: &'a mut Vec<context_menu::TimelineMenuAction>,
}

/// Arrange 视图/播放/渲染配置（layout.rs 每帧构造）。
pub(crate) struct ArrangeViewCfg<'a> {
    pub is_playing: bool,
    pub follow_mode: &'a mut crate::view_interaction::FollowMode,
    pub active_tool: &'a Tool,
    pub min_border_width: f32,
    pub revision: u64,
    pub clip_enabled: bool,
}

/// Arrange 布局几何。
pub(crate) struct ArrangeLayout<'a> {
    pub remaining: egui::Rect,
    pub arr_h: f32,
    pub transport_panel_width: &'a mut f32,
    /// 分割线拖拽刚结束（本帧释放），调用方据此持久化布局设置。
    pub drag_ended: &'a mut bool,
}
