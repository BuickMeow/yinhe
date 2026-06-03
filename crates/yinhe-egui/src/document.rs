use std::collections::HashSet;

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
    pub arr_view: yinhe_pianoroll::ArrangementView,
    pub arr_instances: Vec<yinhe_pianoroll::NoteInstance>,
    pub cursor_tick: Option<f64>,
    pub playback: PlaybackState,
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
        Document {
            midi,
            file_name,
            track_visible: vec![true; num_tracks],
            track_selected: None,
            selected: HashSet::new(),
            view: yinhe_pianoroll::PianoRollView::default(),
            arr_view: yinhe_pianoroll::ArrangementView::default(),
            arr_instances: Vec::new(),
            cursor_tick: None,
            playback: PlaybackState::default(),
        }
    }
}
