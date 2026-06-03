use std::collections::{HashMap, HashSet};

use yinhe_types::TRACK_PALETTE;

use crate::playback::PlaybackState;

/// Per-document state: holds one MIDI file and all UI/GPU state for it.
pub(crate) struct Document {
    pub midi: yinhe_midi::MidiFile,
    pub file_name: String,
    pub selected: HashSet<(u16, u32)>,
    pub track_visible: Vec<bool>,
    pub track_selected: Option<u16>,
    pub view: yinhe_pianoroll::PianoRollView,
    pub arr_view: yinhe_arrangement::ArrangementView,
    pub arr_instances: Vec<yinhe_arrangement::NoteInstance>,
    pub cursor_tick: Option<f64>,
    pub playback: PlaybackState,
    /// Cached track metadata (computed once at load time).
    pub track_info_cache: Vec<yinhe_midi::TrackInfo>,
    /// Cached first ProgramChange per channel (computed once at load time).
    pub pc_map_cache: HashMap<u8, u8>,
}

impl Document {
    pub fn track_colors(&self) -> Vec<[f32; 3]> {
        let n = self.track_visible.len();
        (0..n)
            .map(|i| TRACK_PALETTE[i % TRACK_PALETTE.len()])
            .collect()
    }

    pub fn track_names(&self) -> Vec<String> {
        self.midi.track_names.clone()
    }

    /// Create a new Document from a loaded MIDI file.
    pub fn from_midi(path: &str, midi: yinhe_midi::MidiFile) -> Self {
        let num_tracks = midi.track_ports.len();
        let file_name = std::path::Path::new(path)
            .file_stem()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let track_info_cache = midi.track_info();
        let mut pc_map_cache = HashMap::new();
        for ev in &midi.control_events {
            if let yinhe_midi::MidiControlEvent::ProgramChange { channel, program, .. } = ev {
                pc_map_cache.entry(*channel).or_insert(*program);
            }
        }

        Document {
            midi,
            file_name,
            track_visible: vec![true; num_tracks],
            track_selected: None,
            selected: HashSet::new(),
            view: yinhe_pianoroll::PianoRollView::default(),
            arr_view: yinhe_arrangement::ArrangementView::default(),
            arr_instances: Vec::new(),
            cursor_tick: None,
            playback: PlaybackState::default(),
            track_info_cache,
            pc_map_cache,
        }
    }
}
