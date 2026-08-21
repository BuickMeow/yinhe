use yinhe_editor_core::ResizeSide;
use yinhe_types::PencilNoteDrag;

use crate::piano_view::PreviewReq; // super::PreviewReq
use crate::selection::drag::CollectedNote;

/// Pre-computed info for each selected note during a selection drag.
/// Built once at drag start, reused every frame — eliminates O(N×M) midi lookups.
pub(crate) type SelDragNoteInfo = CollectedNote;

/// 拖拽预览的幽灵音符：(start_tick, end_tick, key, track)。
pub(crate) type GhostNote = (u32, u32, u8, u16);
/// 拖拽时隐藏的原音符：(track, start_tick, key)。
pub(crate) type HiddenNote = (u16, u32, u8);

/// 双击写音符的提交：(note, track)。
pub(crate) type SelNoteEvent = Option<(yinhe_core::NoteEvent, u16)>;

/// 选择工具单音符边缘伸缩：(side, track, start_tick, end_tick, key)。
/// 与选框整体伸缩（sel_resize_state）互斥，音符边缘优先。
pub(crate) type SelNoteResize = (ResizeSide, u16, u32, u32, u8);

/// 快速删除的提交：(track, start_tick, key)。
pub(crate) type QuickDeleteEvent = Option<(u16, u32, u8)>;

/// sel_drag_frame 的帧输出。
pub(crate) type SelFrameOut = (
    Vec<GhostNote>,
    Vec<HiddenNote>,
    Vec<PreviewReq>,
    SelNoteEvent,
    Option<PencilNoteDrag>,
    QuickDeleteEvent,
);
