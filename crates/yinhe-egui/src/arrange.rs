mod actions;
pub(crate) mod am_lanes;
mod edit_cmds;
mod layout;
mod types;
mod view;
pub(crate) use am_lanes::lane_label;
pub(crate) use edit_cmds::{apply_audio_edit_commands, apply_clip_edit_commands};
pub(crate) use types::{
    ArrDragDelta, ArrSelRect, ArrangeData, ArrangeEdit, ArrangeLayout, ArrangeViewCfg,
};
pub use view::show;
pub(crate) mod context_menu;
pub(crate) mod track_panel;
pub(crate) mod view_ui;

use eframe::egui;
use rust_i18n::t;

use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;

use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;
use yinhe_editor_core::audio_settings::LayoutSettings;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

/// Height of the time ruler band at the top of the arrangement view.
use crate::theme;
const RULER_H: f32 = theme::AR_RULER_H;
