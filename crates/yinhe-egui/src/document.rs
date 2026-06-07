use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_types::TRACK_PALETTE;

use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;

/// Per-document state: holds one MIDI file and all UI/GPU state for it.
pub(crate) struct Document {
    pub midi: Arc<yinhe_midi::MidiFile>,
    pub file_name: String,
    pub selected: HashSet<(u16, u32)>,
    pub track_visible: Vec<bool>,
    pub track_selected: Option<u16>,
    pub view: yinhe_pianoroll::PianoRollView,
    pub arr_view: yinhe_arrangement::ArrangementView,
    pub cursor_tick: Option<f64>,
    pub quantize: QuantizePreset,
    pub playback: PlaybackState,
    /// Cached track metadata (computed once at load time).
    pub track_info_cache: Vec<yinhe_midi::TrackInfo>,
    /// Cached first ProgramChange per channel (computed once at load time).
    pub pc_map_cache: HashMap<u8, u8>,
    /// Cached track colors (computed once at load time, avoids per-frame allocation).
    pub track_colors_cache: Vec<[f32; 3]>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            midi: Arc::new(yinhe_midi::MidiFile::default()),
            file_name: String::new(),
            selected: HashSet::new(),
            track_visible: Vec::new(),
            track_selected: None,
            view: yinhe_pianoroll::PianoRollView::default(),
            arr_view: yinhe_arrangement::ArrangementView::default(),
            cursor_tick: None,
            quantize: QuantizePreset::default(),
            playback: PlaybackState::default(),
            track_info_cache: Vec::new(),
            pc_map_cache: HashMap::new(),
            track_colors_cache: Vec::new(),
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
    pub fn from_midi(path: &str, midi: yinhe_midi::MidiFile) -> Self {
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
                view: yinhe_pianoroll::PianoRollView::default(),
                arr_view: yinhe_arrangement::ArrangementView::default(),
                cursor_tick: None,
                quantize: QuantizePreset::default(),
                playback: PlaybackState::default(),
                track_info_cache,
                pc_map_cache,
                track_colors_cache,
            }
        })
    }
}
