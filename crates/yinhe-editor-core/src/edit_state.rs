use std::collections::{HashMap, HashSet};

use crate::config::ProjectSfConfig;
use crate::document::TrackOverride;
use crate::history::PendingEdits;
use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;

/// Selection rectangle state. Single source of truth for the visual selection box.
/// Replaces scattered egui persisted data (sel_rect_persist, sel_drag_origin, last_delta).
#[derive(Clone, Default)]
pub struct SelRectState {
    /// Current committed selection rect: (t_start, t_end, key_lo, key_hi).
    pub rect: Option<(f64, f64, u8, u8)>,
    /// Saved rect at drag start; never modified during drag.
    pub drag_origin: Option<(f64, f64, u8, u8)>,
    /// Current drag delta in (tick, key) units.
    pub drag_delta: Option<(i64, i32)>,
    /// Pending delta from duplicate/transpose; applied once then cleared.
    pub pending_delta: Option<(i64, i32)>,
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

    /// Returns the effective selection rect:
    /// - During drag: drag_origin + drag_delta
    /// - Otherwise: rect
    pub fn effective(&self) -> Option<(f64, f64, u8, u8)> {
        if let (Some(origin), Some((dt, dk))) = (self.drag_origin, self.drag_delta) {
            Some(Self::offset_rect(origin, dt, dk))
        } else {
            self.rect
        }
    }

    /// Begin dragging: save current rect as origin, clear delta.
    pub fn start_drag(&mut self) {
        self.drag_origin = self.rect;
        self.drag_delta = None;
    }

    /// Update drag delta.
    pub fn update_drag(&mut self, dt: i64, dk: i32) {
        self.drag_delta = Some((dt, dk));
    }

    /// End drag: commit origin + delta to rect, clear drag state.
    pub fn end_drag(&mut self) {
        if let (Some(origin), Some((dt, dk))) = (self.drag_origin, self.drag_delta) {
            self.rect = Some(Self::offset_rect(origin, dt, dk));
        }
        self.drag_origin = None;
        self.drag_delta = None;
    }

    /// Cancel drag without committing.
    pub fn cancel_drag(&mut self) {
        self.drag_origin = None;
        self.drag_delta = None;
    }

    /// Apply pending delta from duplicate/transpose to rect.
    pub fn apply_pending(&mut self) {
        if let (Some(rect), Some((dt, dk))) = (self.rect, self.pending_delta) {
            self.rect = Some(Self::offset_rect(rect, dt, dk));
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
    pub controller_panels: Vec<yinhe_automation::AutomationPanelView>,
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
    /// Selection rectangle state.
    pub sel_rect: SelRectState,
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
            controller_panels: vec![yinhe_automation::AutomationPanelView::default()],
            show_controller_panels: true,
            soundfont_selected_port: 0,
            project_sf: ProjectSfConfig::default(),
            pending_edits: PendingEdits::default(),
            track_colors_cache: Vec::new(),
            track_info_cache: Vec::new(),
            pc_map_cache: HashMap::new(),
            conductor_track_idx: None,
            sel_rect: SelRectState::default(),
        }
    }
}
