use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_types::TRACK_PALETTE;

use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;
use crate::right_panel::config::ProjectSfConfig;

/// Per-track mutable overrides (mute, solo, future name/port/channel edits).
#[derive(Clone)]
pub(crate) struct TrackOverride {
    pub muted: bool,
    pub soloed: bool,
}

impl Default for TrackOverride {
    fn default() -> Self {
        Self {
            muted: false,
            soloed: false,
        }
    }
}

/// Per-document state: holds one MIDI file and editing state for it.
///
/// Layout/zoom state (PianoRollView, ArrangementView) lives in `App`,
/// not here, so loading a new MIDI file preserves the user's zoom/scroll.
pub(crate) struct Document {
    pub midi: Arc<yinhe_midi::MidiFile>,
    pub file_name: String,
    pub selected: HashSet<(u16, u32)>,
    pub track_visible: Vec<bool>,
    pub track_selected: Option<u16>,
    pub cursor_tick: Option<f64>,
    pub quantize: QuantizePreset,
    pub playback: PlaybackState,
    /// Cached track metadata (computed once at load time).
    pub track_info_cache: Vec<yinhe_midi::TrackInfo>,
    /// Cached first ProgramChange per channel (computed once at load time).
    pub pc_map_cache: HashMap<u8, u8>,
    /// Cached track colors (computed once at load time, avoids per-frame allocation).
    pub track_colors_cache: Vec<[f32; 3]>,
    /// Automation panel view states.
    pub controller_panels: Vec<yinhe_automation::AutomationPanelView>,
    /// Whether any automation panels are visible.
    pub show_controller_panels: bool,
    /// Song-specific soundfont overrides (not yet persisted).
    pub project_sf: ProjectSfConfig,
    /// Per-track mute/solo overrides.
    pub track_overrides: Vec<TrackOverride>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            midi: Arc::new(yinhe_midi::MidiFile::default()),
            file_name: String::new(),
            selected: HashSet::new(),
            track_visible: Vec::new(),
            track_selected: None,
            cursor_tick: Some(0.0),
            quantize: QuantizePreset::default(),
            playback: PlaybackState::default(),
            track_info_cache: Vec::new(),
            pc_map_cache: HashMap::new(),
            track_colors_cache: Vec::new(),
            controller_panels: vec![yinhe_automation::AutomationPanelView::default()],
            show_controller_panels: true,
            project_sf: ProjectSfConfig::default(),
            track_overrides: vec![TrackOverride::default()],
        }
    }
}

impl Document {
    /// Create a new empty "Untitled" document with a default single-track MIDI.
    pub fn empty() -> Self {
        let mut m = yinhe_midi::MidiFile::default();
        m.track_ports = vec![0];
        m.track_names = vec!["Track 1".to_string()];
        let track_info_cache = m.track_info();
        Document {
            midi: Arc::new(m),
            file_name: "Untitled".into(),
            track_visible: vec![true],
            track_info_cache,
            ..Default::default()
        }
    }

    pub fn track_colors(&self) -> &[[f32; 3]] {
        &self.track_colors_cache
    }

    pub fn track_names(&self) -> &[String] {
        &self.midi.track_names
    }

    /// Create a new Document from a loaded MIDI file.
    ///
    /// `quantize` is inherited from the current document so the user's
    /// quantization setting is preserved across MIDI loads.
    pub fn from_midi(path: &str, midi: yinhe_midi::MidiFile, quantize: QuantizePreset) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let num_tracks = midi.track_ports.len();
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let track_info_cache = midi.track_info();
            let mut pc_map_cache = HashMap::new();
            for ev in &midi.control_events {
                if let yinhe_midi::MidiControlEvent::ProgramChange {
                    channel, program, ..
                } = ev
                {
                    pc_map_cache.entry(*channel).or_insert(*program);
                }
            }

            let midi = Arc::new(midi);

            let track_colors_cache = (0..num_tracks)
                .map(|i| TRACK_PALETTE[i % TRACK_PALETTE.len()])
                .collect();

            Document {
                midi,
                file_name,
                track_visible: vec![true; num_tracks],
                track_selected: None,
                selected: HashSet::new(),
                cursor_tick: Some(0.0),
                quantize,                           // inherit from current document
                playback: PlaybackState::default(), // reset
                track_info_cache,
                pc_map_cache,
                track_colors_cache,
                controller_panels: vec![yinhe_automation::AutomationPanelView::default()],
                show_controller_panels: true,
                project_sf: ProjectSfConfig::default(),
                track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
            }
        })
    }
}
