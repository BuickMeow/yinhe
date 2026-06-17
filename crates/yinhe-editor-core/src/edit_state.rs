use std::collections::{HashMap, HashSet};

use crate::config::ProjectSfConfig;
use crate::document::TrackOverride;
use crate::history::PendingEdits;
use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;

/// Transient editing state. Not persisted to disk, not included in undo snapshots.
/// Preserved across document switches (zoom/scroll live in App, not here).
pub struct EditState {
    pub selected: HashSet<(u16, u32, u8)>,
    pub track_selected: HashSet<u16>,
    pub cursor_tick: Option<f64>,
    pub quantize: QuantizePreset,
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
}

impl Default for EditState {
    fn default() -> Self {
        Self {
            selected: HashSet::new(),
            track_selected: HashSet::new(),
            cursor_tick: Some(0.0),
            quantize: QuantizePreset::default(),
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
        }
    }
}
