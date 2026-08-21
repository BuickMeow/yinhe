use eframe::egui;

use yinhe_editor_core::ResizeSide;

use super::types::*;

/// sel_drag_frame 的帧内可变状态：5 个互斥拖拽状态机 + 帧输出。
///
/// 曾全部内联在 sel_drag_frame 一个 800+ 行的函数里；拆分后各状态机
/// 函数共享本结构，主函数只负责加载 / 分发 / 持久化。
pub(crate) struct SelDragFrameState {
    /// 选区整体移动：(origin_tick, origin_key, alt)。None = 未在移动。
    pub(crate) note_drag_origin: Option<(f64, f64, bool)>,
    /// 拖拽中预计算的选中音符（选区移动/选区缩放共用，press 时构建一次）。
    pub(crate) drag_notes: Option<Vec<SelDragNoteInfo>>,
    /// 拖拽中已触发预览的 key delta（每变化 1 key 触发一次整组预览）。
    pub(crate) preview_last_dk: i32,
    /// 选区移动是否曾产生过位移（用于 Alt 复制：点一下不复制，拖动回原位也算移动）。
    pub(crate) note_drag_had_moved: bool,
    /// 选区边缘缩放：(side, origin_boundary_tick, other_boundary_tick)。
    pub(crate) sel_resize_state: Option<(ResizeSide, f64, f64)>,
    /// 单音符边缘伸缩：(side, track, start_tick, end_tick, key)。
    pub(crate) sel_note_resize: Option<SelNoteResize>,
    /// 单音符移动：(track, orig_start, orig_key, orig_end, press_tick, last_dk, alt)。
    pub(crate) sel_note_move: Option<(u16, u32, u8, u32, f64, i32, bool)>,
    /// 单音符移动是否曾产生过位移（同上）。
    pub(crate) single_note_had_moved: bool,
    /// 帧输出：幽灵音符 / 隐藏音符 / 预览请求。
    pub(crate) ghost_notes: Vec<super::types::GhostNote>,
    pub(crate) hidden_notes: Vec<super::types::HiddenNote>,
    pub(crate) preview_reqs: Vec<crate::piano_view::PreviewReq>,
}

/// 是否有选框工具的拖拽状态机正在进行（跨帧持久化状态）。
///
/// 供 `effective_tool` 在拖拽期间锁定选择工具：否则 Alt 拖拽克隆时
/// 鼠标一旦移出音符原位，hover 命中失败就会被误判为"悬停空白"，
/// 临时切成铅笔工具、中断本次拖拽。
pub(crate) fn sel_drag_in_progress(ui: &egui::Ui) -> bool {
    let id = ui.id();
    ui.data_mut(|d| {
        d.get_persisted::<Option<(f64, f64, bool)>>(id.with("note_drag_origin"))
            .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<(ResizeSide, f64, f64)>>(id.with("sel_resize_state"))
                .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<SelNoteResize>>(id.with("sel_note_resize_state"))
                .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<(u16, u32, u8, u32, f64, i32, bool)>>(
                id.with("sel_note_move_state"),
            )
            .is_some_and(|v| v.is_some())
            || d.get_persisted::<Option<((f64, f32), egui::Pos2, egui::Pos2)>>(id.with("sel_drag"))
                .is_some_and(|v| v.is_some())
    })
}

impl SelDragFrameState {
    /// 从 egui 持久化加载拖拽状态（拖拽跨帧保持）。
    pub(crate) fn load(ui: &mut egui::Ui) -> Self {
        Self {
            note_drag_origin: ui
                .data_mut(|d| d.get_persisted(ui.id().with("note_drag_origin")))
                .unwrap_or(None),
            drag_notes: ui
                .data_mut(|d| d.get_persisted(ui.id().with("drag_notes")))
                .unwrap_or(None),
            preview_last_dk: ui
                .data_mut(|d| d.get_persisted(ui.id().with("note_drag_preview_dk")))
                .unwrap_or(0),
            note_drag_had_moved: ui
                .data_mut(|d| d.get_persisted(ui.id().with("note_drag_had_moved")))
                .unwrap_or(false),
            sel_resize_state: ui
                .data_mut(|d| d.get_persisted(ui.id().with("sel_resize_state")))
                .unwrap_or(None),
            sel_note_resize: ui
                .data_mut(|d| d.get_persisted(ui.id().with("sel_note_resize_state")))
                .unwrap_or(None),
            sel_note_move: ui
                .data_mut(|d| d.get_persisted(ui.id().with("sel_note_move_state")))
                .unwrap_or(None),
            single_note_had_moved: ui
                .data_mut(|d| d.get_persisted(ui.id().with("single_note_had_moved")))
                .unwrap_or(false),
            ghost_notes: Vec::new(),
            hidden_notes: Vec::new(),
            preview_reqs: Vec::new(),
        }
    }

    /// 持久化拖拽状态（拖拽跨帧保持）。
    pub(crate) fn save(&mut self, ui: &mut egui::Ui) {
        // 解构出各状态字段的可变引用，避免闭包整体 move `self`。
        let Self {
            note_drag_origin,
            drag_notes,
            note_drag_had_moved,
            sel_resize_state,
            sel_note_resize,
            sel_note_move,
            single_note_had_moved,
            ..
        } = self;
        ui.data_mut(|d| {
            d.insert_persisted(ui.id().with("note_drag_origin"), note_drag_origin.take())
        });
        ui.data_mut(|d| d.insert_persisted(ui.id().with("drag_notes"), drag_notes.take()));
        ui.data_mut(|d| {
            d.insert_persisted(ui.id().with("note_drag_had_moved"), *note_drag_had_moved)
        });
        ui.data_mut(|d| {
            d.insert_persisted(ui.id().with("sel_resize_state"), sel_resize_state.take())
        });
        ui.data_mut(|d| {
            d.insert_persisted(
                ui.id().with("sel_note_resize_state"),
                sel_note_resize.take(),
            )
        });
        ui.data_mut(|d| {
            d.insert_persisted(ui.id().with("sel_note_move_state"), sel_note_move.take())
        });
        ui.data_mut(|d| {
            d.insert_persisted(
                ui.id().with("single_note_had_moved"),
                *single_note_had_moved,
            )
        });
    }

    /// 清理已 stale 的拖拽状态（指针抬起但状态未释放的兜底）。
    pub(crate) fn clear_stale(
        &mut self,
        sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
        pointer: &egui::PointerState,
    ) {
        if self.note_drag_origin.is_some() && !pointer.primary_down() && !pointer.primary_released()
        {
            self.note_drag_origin = None;
            self.drag_notes = None;
            self.note_drag_had_moved = false;
            sel_rect.cancel_drag();
        }
        if self.sel_resize_state.is_some() && !pointer.primary_down() && !pointer.primary_released()
        {
            self.sel_resize_state = None;
            self.drag_notes = None;
            sel_rect.cancel_resize();
        }
        if self.sel_note_resize.is_some() && !pointer.primary_down() && !pointer.primary_released()
        {
            self.sel_note_resize = None;
        }
        if self.sel_note_move.is_some() && !pointer.primary_down() && !pointer.primary_released() {
            self.sel_note_move = None;
            self.single_note_had_moved = false;
        }
    }
}
