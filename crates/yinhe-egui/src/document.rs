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
    /// In-memory project archive (source of truth for save).
    /// Set when loading a .mid (converted) or a .yin (loaded directly).
    pub archive: Option<yinhe_project::ProjectArchive>,
    pub file_name: String,
    pub file_path: Option<String>,
    pub selected: HashSet<(u16, u32, u8)>,
    pub track_visible: Vec<bool>,
    pub track_selected: HashSet<u16>,
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
    /// Currently selected port in the soundfont panel (persists across frames).
    pub soundfont_selected_port: u8,
    /// Per-track mute/solo overrides.
    pub track_overrides: Vec<TrackOverride>,
    /// Per-track pianoroll-only visibility (V button). Independent of `track_visible`.
    /// `track_visible[i] && track_pianoroll_visible[i]` is the effective pianoroll mask.
    /// Memory-only (not persisted).
    pub track_pianoroll_visible: Vec<bool>,
    /// Snapshot of `track_pianoroll_visible` taken when the user double-clicks a
    /// track row to "solo" it. Cleared (and snapshot restored) when the user
    /// single-clicks the already-selected row's badge/name area.
    /// Memory-only (not persisted).
    pub track_pianoroll_visible_snapshot: Option<Vec<bool>>,
    /// Index of the conductor track, if one was detected on load.
    /// Heuristic: track 0 with zero notes and zero control events.
    pub conductor_track_idx: Option<u16>,
    /// Editable project metadata (synced with project.json on save/load).
    pub project_name: String,
    pub project_artist: String,
    pub project_description: String,
    /// Editable PPQ (ticks per beat). Saved to project.json; takes effect on next load.
    pub project_ppq: u32,
}

/// Detect the conductor track using a SMF format-1 heuristic.
///
/// A track is treated as the conductor if it is index 0, has zero notes, and
/// has no control events targeting it. This is true for virtually all
/// well-formed format-1 files (where track 0 holds tempo / time-sig / key-sig
/// meta and the song title via TrackName).
pub(crate) fn detect_conductor(
    track_info: &[yinhe_midi::TrackInfo],
    control_events: &[yinhe_midi::MidiControlEvent],
) -> Option<u16> {
    if track_info.is_empty() {
        return None;
    }
    if track_info[0].note_count != 0 {
        return None;
    }
    let has_ctrl_on_zero = control_events.iter().any(|e| match e {
        yinhe_midi::MidiControlEvent::ControlChange { track, .. }
        | yinhe_midi::MidiControlEvent::ProgramChange { track, .. }
        | yinhe_midi::MidiControlEvent::PitchBend { track, .. } => *track == 0,
    });
    if has_ctrl_on_zero {
        return None;
    }
    Some(0)
}

/// Compute the display color for a track, respecting an optional conductor offset.
///
/// Conductor track gets a fixed near-white color; subsequent tracks index into
/// the palette starting at 0 (so the first non-conductor track gets palette[0]).
/// Without a conductor, all tracks use `TRACK_PALETTE[idx % LEN]` (legacy).
pub(crate) fn track_color(idx: usize, conductor_idx: Option<u16>) -> [f32; 3] {
    if Some(idx as u16) == conductor_idx {
        return [0.94, 0.94, 0.94]; // near-white "Master" badge
    }
    let palette_idx = match conductor_idx {
        Some(c) if (idx as u16) > c => idx - 1,
        _ => idx,
    };
    TRACK_PALETTE[palette_idx % TRACK_PALETTE.len()]
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
            track_selected: HashSet::new(),
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
            soundfont_selected_port: 0,
            track_overrides: vec![TrackOverride::default()],
            track_pianoroll_visible: Vec::new(),
            track_pianoroll_visible_snapshot: None,
            conductor_track_idx: None,
            project_name: String::new(),
            project_artist: String::new(),
            project_description: String::new(),
            project_ppq: 480,
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
        Document {
            midi: Arc::new(m),
            file_name: "Untitled".into(),
            file_path: None,
            archive: None,
            track_visible: vec![true],
            track_pianoroll_visible: vec![true],
            conductor_track_idx: None,
            track_names,
            track_info_cache,
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
    ///
    /// Returns `Err` with a user-facing message if the MIDI lacks a recognizable
    /// conductor track (e.g. format-0 single-track files). In that case the
    /// caller should surface the error to the user instead of opening the doc.
    pub fn from_midi(
        path: &str,
        midi: yinhe_midi::MidiFile,
        quantize: QuantizePreset,
    ) -> Result<Self, String> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            // Convert to archive (the canonical storage format)
            let archive = crate::project_io::midi_to_archive(&midi);
            // Derive MidiFile from the archive (same path as .yin loading)
            let mut midi = crate::project_io::archive_to_midi(&archive);
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            // ── Conductor detection (heuristic: track 0, no notes, no ctrl events) ──
            let track_info_cache_initial = midi.track_info();
            let conductor_track_idx = detect_conductor(&track_info_cache_initial, &midi.control_events);
            if conductor_track_idx.is_none() {
                // Non-standard MIDI: track 0 has notes/CCs. Insert a new
                // conductor track at index 0 and shift all existing tracks
                // down by one.
                for notes in &mut midi.key_notes {
                    for note in notes.iter_mut() {
                        note.track += 1;
                    }
                }
                for ev in &mut midi.control_events {
                    match ev {
                        yinhe_midi::MidiControlEvent::ControlChange { track, .. }
                        | yinhe_midi::MidiControlEvent::ProgramChange { track, .. }
                        | yinhe_midi::MidiControlEvent::PitchBend { track, .. } => *track += 1,
                    }
                }
                midi.track_ports.insert(0, 0);
                midi.track_channels.insert(0, 0);
                midi.track_names.insert(0, "Conductor".to_string());
                midi.track_channel_prefixes.insert(0, None);
            }
            let track_info_cache_initial = midi.track_info();
            let conductor_track_idx = detect_conductor(&track_info_cache_initial, &midi.control_events);
            let num_tracks = midi.track_ports.len();

            let mut track_names: Vec<String> = (0..num_tracks)
                .map(|i| {
                    midi.track_names
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| format!("Track {}", i + 1))
                })
                .collect();

            // Promote track 0's TrackName (song title) to project_name; rename track 0 to "Conductor".
            let mut project_name_override: Option<String> = None;
            if let Some(c_idx) = conductor_track_idx {
                let c = c_idx as usize;
                if let Some(name) = track_names.get(c) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with("Track ") {
                        project_name_override = Some(trimmed.to_string());
                    }
                }
                if c < track_names.len() {
                    track_names[c] = "Conductor".to_string();
                }
                if c < midi.track_names.len() {
                    midi.track_names[c] = "Conductor".to_string();
                } else {
                    while midi.track_names.len() < c {
                        midi.track_names.push(String::new());
                    }
                    midi.track_names.push("Conductor".to_string());
                }
            }
            // Rebuild after the rename so track_info_cache reflects "Conductor".
            let track_info_cache = midi.track_info();

            let mut pc_map_cache = HashMap::new();
            for ev in &midi.control_events {
                if let yinhe_midi::MidiControlEvent::ProgramChange {
                    program, track, ..
                } = ev
                {
                    let ch = midi
                        .track_channels
                        .get(*track as usize)
                        .copied()
                        .unwrap_or(0);
                    pc_map_cache.entry(ch).or_insert(*program);
                }
            }

            let ticks_per_beat = midi.ticks_per_beat;
            let midi = Arc::new(midi);

            let track_colors_cache = (0..num_tracks)
                .map(|i| track_color(i, conductor_track_idx))
                .collect();

            Ok(Document {
                midi,
                file_name,
                file_path: None,
                archive: Some(archive),
                track_visible: vec![true; num_tracks],
                track_pianoroll_visible: vec![true; num_tracks],
                conductor_track_idx,
                track_selected: HashSet::new(),
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
                project_name: project_name_override.unwrap_or_default(),
                ..Default::default()
            })
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
        let mut midi = midi;
        let num_tracks = midi.track_ports.len();
        let mut track_names: Vec<String> = (0..num_tracks)
            .map(|i| {
                midi.track_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Track {}", i + 1))
            })
            .collect();
        let track_info_cache_initial = midi.track_info();
        let mut pc_map_cache = std::collections::HashMap::new();
        for ev in &midi.control_events {
            if let yinhe_midi::MidiControlEvent::ProgramChange {
                program, track, ..
            } = ev
            {
                let ch = midi
                    .track_channels
                    .get(*track as usize)
                    .copied()
                    .unwrap_or(0);
                pc_map_cache.entry(ch).or_insert(*program);
            }
        }

        // ── Conductor detection ──
        // .yin files are produced by yinhe and so should always have a
        // conductor track at index 0 (zero notes, zero ctrl events). We still
        // run the heuristic for robustness; if the file was hand-edited and
        // lacks one, we leave conductor_track_idx = None and let the UI fall
        // back to "no special row" (no rejection here, since the file already
        // round-tripped through midi_to_archive once).
        let conductor_track_idx = detect_conductor(&track_info_cache_initial, &midi.control_events);
        if let Some(c_idx) = conductor_track_idx {
            let c = c_idx as usize;
            if c < track_names.len() {
                track_names[c] = "Conductor".to_string();
            }
            if c < midi.track_names.len() {
                midi.track_names[c] = "Conductor".to_string();
            } else {
                while midi.track_names.len() < c {
                    midi.track_names.push(String::new());
                }
                midi.track_names.push("Conductor".to_string());
            }
        }
        let track_info_cache = midi.track_info();

        // Read project metadata + soundfont config from archive
        let (
            project_name,
            project_artist,
            project_description,
            project_ppq,
            project_sf,
            soundfont_project_mode,
        ) = archive
            .get_json::<yinhe_project::ProjectJson>("project.json")
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
            .map(|i| track_color(i, conductor_track_idx))
            .collect();

        Ok((Document {
            midi: Arc::new(midi),
            file_name,
            file_path: Some(path.to_string()),
            archive: Some(archive),
            track_visible: vec![true; num_tracks],
            track_pianoroll_visible: vec![true; num_tracks],
            track_pianoroll_visible_snapshot: None,
            conductor_track_idx,
            track_selected: HashSet::new(),
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
            soundfont_selected_port: 0,
            track_overrides: (0..num_tracks).map(|_| TrackOverride::default()).collect(),
            project_name,
            project_artist,
            project_description,
            project_ppq,
        }, soundfont_project_mode))
    }
}
