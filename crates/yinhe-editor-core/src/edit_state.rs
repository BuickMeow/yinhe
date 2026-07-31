use std::collections::{HashMap, HashSet};

use crate::config::ProjectSfConfig;
use crate::document::TrackOverride;
use crate::history::PendingEdits;
use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;

/// Selection rectangle state. Single source of truth for the visual selection boxes.
/// Replaces scattered egui persisted data (sel_rect_persist, sel_drag_origin, last_delta).
///
/// 支持多选框：shift+框选时不清空已有选框，而是 append。
/// 拖拽时所有选框一起偏移。
#[derive(Clone, Default)]
pub struct SelRectState {
    /// Committed selection rects: (t_start, t_end, key_lo, key_hi).
    /// 多选框时按 shift+框选顺序累加。
    pub rects: Vec<(f64, f64, u8, u8)>,
    /// Saved rects at drag start; never modified during drag.
    drag_origins: Vec<(f64, f64, u8, u8)>,
    /// Current drag delta in (tick, key) units.
    drag_delta: Option<(i64, i32)>,
    /// Pending delta from duplicate/transpose; applied once then cleared.
    pub pending_delta: Option<(i64, i32)>,
    /// Saved rects at resize start; never modified during resize.
    resize_origins: Vec<(f64, f64, u8, u8)>,
    /// Active resize side (Left/Right); None when not resizing.
    resize_side: Option<ResizeSide>,
    /// Current resize delta in tick units.
    resize_dt: Option<i64>,
}

/// Which edge of the selection rect is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeSide {
    Left,
    Right,
}

impl SelRectState {
    fn offset_rect(rect: (f64, f64, u8, u8), dt: i64, dk: i32) -> (f64, f64, u8, u8) {
        let (t0, t1, kl, kh) = rect;
        (
            t0 + dt as f64,
            t1 + dt as f64,
            (kl as i32 + dk).clamp(0, 127) as u8,
            (kh as i32 + dk).clamp(0, 127) as u8,
        )
    }

    /// Apply a resize delta to a single rect. Left side changes t_start,
    /// Right side changes t_end. Ensures t_end > t_start (min width 1 tick).
    fn resize_rect(rect: (f64, f64, u8, u8), side: ResizeSide, dt: i64) -> (f64, f64, u8, u8) {
        let (t0, t1, kl, kh) = rect;
        match side {
            ResizeSide::Left => {
                let new_t0 = (t0 + dt as f64).max(0.0).min(t1 - 1.0);
                (new_t0, t1, kl, kh)
            }
            ResizeSide::Right => {
                let new_t1 = (t1 + dt as f64).max(t0 + 1.0);
                (t0, new_t1, kl, kh)
            }
        }
    }

    /// Returns the effective selection rects:
    /// - During drag: drag_origins + drag_delta
    /// - During resize: resize_origins + resize_side + resize_dt
    /// - Otherwise: rects
    pub fn effective_rects(&self) -> Vec<(f64, f64, u8, u8)> {
        if let Some((dt, dk)) = self.drag_delta {
            self.drag_origins.iter().map(|&r| Self::offset_rect(r, dt, dk)).collect()
        } else if let (Some(side), Some(dt)) = (self.resize_side, self.resize_dt) {
            self.resize_origins.iter().map(|&r| Self::resize_rect(r, side, dt)).collect()
        } else {
            self.rects.clone()
        }
    }

    /// 是否正在 resize。
    pub fn is_resizing(&self) -> bool {
        self.resize_side.is_some()
    }

    /// 是否没有任何选框。
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// 清空所有选框。
    pub fn clear(&mut self) {
        self.rects.clear();
    }

    /// Begin dragging: save current rects as origins, clear delta.
    pub fn start_drag(&mut self) {
        self.drag_origins = self.rects.clone();
        self.drag_delta = None;
    }

    /// Update drag delta.
    pub fn update_drag(&mut self, dt: i64, dk: i32) {
        self.drag_delta = Some((dt, dk));
    }

    /// End drag: commit origins + delta to rects, clear drag state.
    pub fn end_drag(&mut self) {
        if let Some((dt, dk)) = self.drag_delta {
            self.rects = self.drag_origins.iter().map(|&r| Self::offset_rect(r, dt, dk)).collect();
        }
        self.drag_origins.clear();
        self.drag_delta = None;
    }

    /// Cancel drag without committing.
    pub fn cancel_drag(&mut self) {
        self.drag_origins.clear();
        self.drag_delta = None;
    }

    /// Begin resizing: save current rects as origins, clear resize delta.
    pub fn start_resize(&mut self, side: ResizeSide) {
        self.resize_origins = self.rects.clone();
        self.resize_side = Some(side);
        self.resize_dt = None;
    }

    /// Update resize delta.
    pub fn update_resize(&mut self, dt: i64) {
        self.resize_dt = Some(dt);
    }

    /// End resize: commit origins + dt to rects, clear resize state.
    pub fn end_resize(&mut self) {
        if let (Some(side), Some(dt)) = (self.resize_side, self.resize_dt) {
            self.rects = self.resize_origins.iter().map(|&r| Self::resize_rect(r, side, dt)).collect();
        }
        self.resize_origins.clear();
        self.resize_side = None;
        self.resize_dt = None;
    }

    /// Cancel resize without committing.
    pub fn cancel_resize(&mut self) {
        self.resize_origins.clear();
        self.resize_side = None;
        self.resize_dt = None;
    }

    /// Apply pending delta from duplicate/transpose to all rects.
    pub fn apply_pending(&mut self) {
        if let Some((dt, dk)) = self.pending_delta {
            for r in &mut self.rects {
                *r = Self::offset_rect(*r, dt, dk);
            }
        }
        self.pending_delta = None;
    }
}

/// Transient editing state. Not persisted to disk.
/// Selection (`selected` and `sel_rect`) is captured in undo snapshots; most
/// other fields are not. Preserved across document switches (zoom/scroll live
/// in App, not here).
pub struct EditState {
    pub selected: yinhe_core::Selection,
    pub track_selected: HashSet<u16>,
    pub cursor_tick: Option<f64>,
    pub quantize_arrange: QuantizePreset,
    pub quantize_pianoroll: QuantizePreset,
    pub playback: PlaybackState,
    pub track_overrides: Vec<TrackOverride>,
    pub track_visible: Vec<bool>,
    pub track_pianoroll_visible: Vec<bool>,
    pub track_pianoroll_visible_snapshot: Option<Vec<bool>>,
    pub controller_panels: Vec<yinhe_types::AutomationPanelView>,
    pub show_controller_panels: bool,
    pub soundfont_selected_port: u8,
    pub project_sf: ProjectSfConfig,
    pub pending_edits: PendingEdits,
    /// Per-track display colors (computed once at load time).
    pub track_colors_cache: Vec<[f32; 3]>,
    /// Cached track metadata (recomputed from midi + track_names).
    pub track_info_cache: Vec<yinhe_core::TrackInfo>,
    /// Cached first ProgramChange per channel.
    pub pc_map_cache: HashMap<u8, u8>,
    /// Index of the conductor track, if detected.
    pub conductor_track_idx: Option<u16>,
    /// 当前被铅笔工具编辑的轨道（双击 track 设置）。
    /// 同时只能有一个；可见且被选择时才允许编辑。
    /// Conductor 也可被设为 editing_track（仅用于 Tempo automation）。
    pub editing_track: Option<u16>,
    /// Selection rectangle state.
    pub sel_rect: SelRectState,
    /// AR 选框（钢琴卷帘/自动化的选框在 `sel_rect`/`anchor_sel_rects`）。
    /// 与 sel_rect 一样参与 undo 快照，随文档切换。
    pub arr_sel_rect: Vec<(f64, f64, usize, usize)>,
}

impl Default for EditState {
    fn default() -> Self {
        Self {
            selected: yinhe_core::Selection::default(),
            track_selected: HashSet::new(),
            cursor_tick: Some(0.0),
            quantize_arrange: QuantizePreset::Fraction(1, 4),
            quantize_pianoroll: QuantizePreset::Fraction(1, 16),
            playback: PlaybackState::default(),
            track_overrides: vec![TrackOverride::default()],
            track_visible: Vec::new(),
            track_pianoroll_visible: Vec::new(),
            track_pianoroll_visible_snapshot: None,
            controller_panels: vec![yinhe_types::AutomationPanelView::default()],
            show_controller_panels: true,
            soundfont_selected_port: 0,
            project_sf: ProjectSfConfig::default(),
            pending_edits: PendingEdits::default(),
            track_colors_cache: Vec::new(),
            track_info_cache: Vec::new(),
            pc_map_cache: HashMap::new(),
            conductor_track_idx: None,
            editing_track: None,
            sel_rect: SelRectState::default(),
            arr_sel_rect: Vec::new(),
        }
    }
}
