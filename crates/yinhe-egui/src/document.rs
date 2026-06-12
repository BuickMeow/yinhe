use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yinhe_types::TRACK_PALETTE;

use crate::playback::PlaybackState;
use crate::quantize::QuantizePreset;
use crate::right_panel::config::ProjectSfConfig;
use crate::widgets::track_panel::ChannelGroup;

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
    /// In-memory project archive (source of truth for save).
    /// Set when loading a .mid (converted) or a .yin (loaded directly).
    pub archive: Option<yinhe_project::ProjectArchive>,
    pub file_name: String,
    pub file_path: Option<String>,
    pub selected: HashSet<(u16, u32, u8)>,
    pub track_visible: Vec<bool>,
    pub track_selected: Option<u16>,
    pub cursor_tick: Option<f64>,
    pub quantize: QuantizePreset,
    pub playback: PlaybackState,
    /// Authoritative, editable track names. Mirrored into `track_info_cache[i].name`.
    pub track_names: Vec<String>,
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
    /// Song-specific soundfont config.
    pub project_sf: ProjectSfConfig,
    /// Currently selected port in the soundbank panel (persists across frames).
    pub soundbank_selected_port: u8,
    /// Per-track mute/solo overrides.
    pub track_overrides: Vec<TrackOverride>,
    /// Editable project metadata (synced with project.json on save/load).
    pub project_name: String,
    pub project_artist: String,
    pub project_description: String,
    /// Editable PPQ (ticks per beat). Saved to project.json; takes effect on next load.
    pub project_ppq: u32,
    /// Channel groups (port+channel) for track panel folder display.
    pub channel_groups: Vec<ChannelGroup>,
    /// Whether the conductor track is expanded.
    pub conductor_expanded: bool,
    /// BPM automation lane (built from tempo_segments).
    pub tempo_lane: Option<yinhe_types::AutomationLane>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            midi: Arc::new(yinhe_midi::MidiFile::default()),
            file_name: String::new(),
            file_path: None,
            archive: None,
            selected: HashSet::new(),
            track_visible: Vec::new(),
            track_selected: None,
            cursor_tick: Some(0.0),
            quantize: QuantizePreset::default(),
            playback: PlaybackState::default(),
            track_names: Vec::new(),
            track_info_cache: Vec::new(),
            pc_map_cache: HashMap::new(),
            track_colors_cache: Vec::new(),
            controller_panels: vec![yinhe_automation::AutomationPanelView::default()],
            show_controller_panels: true,
            project_sf: ProjectSfConfig::default(),
            soundbank_selected_port: 0,
            track_overrides: vec![TrackOverride::default()],
            project_name: String::new(),
            project_artist: String::new(),
            project_description: String::new(),
            project_ppq: 480,
            channel_groups: Vec::new(),
            conductor_expanded: true,
            tempo_lane: None,
        }
    }
}

impl Document {
    /// Create a new empty "Untitled" document with a default single-track MIDI.
    pub fn empty() -> Self {
        let mut m = yinhe_midi::MidiFile::default();
        m.track_ports = vec![0];
        m.track_names = vec!["Track 1".to_string()];
        let track_names = m.track_names.clone();
        let track_info_cache = m.track_info();
        let channel_groups =
            crate::widgets::track_panel::build_channel_groups(&track_info_cache, &[]);
        Document {
            midi: Arc::new(m),
            file_name: "Untitled".into(),
            file_path: None,
            archive: None,
            track_visible: vec![true],
            track_names,
            track_info_cache,
            channel_groups,
            conductor_expanded: true,
            tempo_lane: None,
            ..Default::default()
        }
    }

    pub fn track_colors(&self) -> &[[f32; 3]] {
        &self.track_colors_cache
    }

    /// Create a new Document from a loaded MIDI file.
    ///
    /// Immediately converts to a ProjectArchive and derives the MidiFile from it,
    /// so the document behaves identically to one loaded from a .yin file.
    ///
    /// `quantize` is inherited from the current document so the user's
    /// quantization setting is preserved across MIDI loads.
    pub fn from_midi(path: &str, midi: yinhe_midi::MidiFile, quantize: QuantizePreset) -> Self {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            // Convert to archive (the canonical storage format)
            let archive = crate::project_io::midi_to_archive(&midi);
            // Derive MidiFile from the archive (same path as .yin loading)
            let midi = crate::project_io::archive_to_midi(&archive);
            let num_tracks = midi.track_ports.len();
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let track_names: Vec<String> = (0..num_tracks)
                .map(|i| {
                    midi.track_names
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("Track {}", i + 1))
                })
                .collect();
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

            let ticks_per_beat = midi.ticks_per_beat;
            let midi = Arc::new(midi);

            let track_colors_cache = (0..num_tracks)
                .map(|i| TRACK_PALETTE[i % TRACK_PALETTE.len()])
                .collect();

            let channel_groups = crate::widgets::track_panel::build_channel_groups(
                &track_info_cache,
                &midi.automation_lanes,
            );
            let tempo_lane = Some(yinhe_midi::build_tempo_automation_lane(
                &midi.tempo_segments,
            ));

            Document {
                midi,
                file_name,
                file_path: None,
                archive: Some(archive),
                track_visible: vec![true; num_tracks],
                track_selected: None,
                selected: HashSet::new(),
                cursor_tick: Some(0.0),
                quantize,                           // inherit from current document
                playback: PlaybackState::default(), // reset
                track_names,
                track_info_cache,
                pc_map_cache,
                track_colors_cache,
                controller_panels: vec![yinhe_automation::AutomationPanelView::default()],
                show_controller_panels: true,
                project_sf: ProjectSfConfig::default(),
                track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
                project_ppq: ticks_per_beat,
                channel_groups,
                conductor_expanded: true,
                tempo_lane,
                ..Default::default()
            }
        })
    }

    /// Create a new Document from a .yin project file.
    ///
    /// Returns `(document, soundfont_project_mode)` — the caller should
    /// set `audio_settings.global_sf_config.global_enabled = !soundfont_project_mode`.
    pub fn from_yin(
        path: &str,
        quantize: QuantizePreset,
    ) -> std::io::Result<(Self, bool)> {
        let (midi, file_name, archive) = crate::project_io::load_project_full(path)?;
        let num_tracks = midi.track_ports.len();
        let track_names: Vec<String> = (0..num_tracks)
            .map(|i| {
                midi.track_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Track {}", i + 1))
            })
            .collect();
        let track_info_cache = midi.track_info();
        let mut pc_map_cache = std::collections::HashMap::new();
        for ev in &midi.control_events {
            if let yinhe_midi::MidiControlEvent::ProgramChange {
                channel, program, ..
            } = ev
            {
                pc_map_cache.entry(*channel).or_insert(*program);
            }
        }

        // Read project metadata + soundfont config from archive
        let (
            project_name,
            project_artist,
            project_description,
            project_ppq,
            project_sf,
            soundfont_project_mode,
        ) = archive
            .get_events::<yinhe_project::ProjectJson>("project.json")
            .and_then(|v| v.into_iter().next())
            .map(|p| {
                let sf = crate::right_panel::config::ProjectSfConfig {
                    overrides: p
                        .soundfont_overrides
                        .into_iter()
                        .map(|ov| {
                            (
                                ov.port,
                                ov.entries
                                    .into_iter()
                                    .map(|e| crate::right_panel::config::SfEntry {
                                        path: e.path,
                                        name: e.name,
                                        enabled: e.enabled,
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                };
                (p.name, p.artist, p.description, p.ppq, sf, p.soundfont_project_mode)
            })
            .unwrap_or((
                String::new(),
                String::new(),
                String::new(),
                480,
                crate::right_panel::config::ProjectSfConfig::default(),
                false,
            ));

        let track_colors_cache = (0..num_tracks)
            .map(|i| yinhe_types::TRACK_PALETTE[i % yinhe_types::TRACK_PALETTE.len()])
            .collect();

        let channel_groups = crate::widgets::track_panel::build_channel_groups(
            &track_info_cache,
            &midi.automation_lanes,
        );
        let tempo_lane = Some(yinhe_midi::build_tempo_automation_lane(
            &midi.tempo_segments,
        ));

        Ok((Document {
            midi: Arc::new(midi),
            file_name,
            file_path: Some(path.to_string()),
            archive: Some(archive),
            track_visible: vec![true; num_tracks],
            track_selected: None,
            selected: HashSet::new(),
            cursor_tick: Some(0.0),
            quantize,
            playback: PlaybackState::default(),
            track_names,
            track_info_cache,
            pc_map_cache,
            track_colors_cache,
            controller_panels: vec![yinhe_automation::AutomationPanelView::default()],
            show_controller_panels: true,
            project_sf,
            soundbank_selected_port: 0,
            track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
            project_name,
            project_artist,
            project_description,
            project_ppq,
            channel_groups,
            conductor_expanded: true,
            tempo_lane,
        }, soundfont_project_mode))
    }
}
