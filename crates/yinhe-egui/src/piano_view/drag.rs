//! Selection-tool drag logic: move + edge-resize.
//!
//! 选框工具的 press → drag → release 状态机。marquee 框选逻辑在 `marquee.rs`.

#![allow(unused_imports)]

mod frame;
mod group_move;
mod group_resize;
mod hit;
mod interact;
mod press;
mod single_move;
mod single_resize;
mod state;
mod types;

pub(crate) use frame::sel_drag_frame;
pub(crate) use group_move::note_drag_frame;
pub(crate) use group_resize::sel_resize_frame;
pub(crate) use hit::{double_click_note, hit_test_note, rect_has_notes};
pub(crate) use interact::{
    clamped_local, cursor_tick_from_click, drag_scroll_and_clamp, on_action_bar,
};
pub(crate) use press::sel_press;
pub(crate) use single_move::single_note_move_frame;
pub(crate) use single_resize::single_note_resize_frame;
pub(crate) use state::{SelDragFrameState, sel_drag_in_progress};
pub(crate) use types::*;

// 通用逻辑已抽取到 crate::selection::drag：
pub(crate) use crate::selection::drag::{
    collect_selected_notes, compute_resize_dt, hit_test_sel_edge, main_cross_x_y,
    main_px_to_tick_dir, orient_rect, tick_to_main_px_dir,
};

#[cfg(test)]
#[path = "drag_tests.rs"]
mod tests;
