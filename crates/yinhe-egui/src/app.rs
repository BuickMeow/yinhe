use eframe::egui;
use std::collections::HashSet;

use crate::piano_view;
use crate::playback::PlaybackState;
use crate::render_context::RenderContext;

pub struct App {
    render_ctx: RenderContext,
    pianoroll: yinhe_pianoroll::PianorollRenderer,
    midi: Option<yinhe_midi::MidiFile>,
    view: yinhe_pianoroll::PianoRollView,
    selected: HashSet<(u16, u32)>,
    file_name: Option<String>,
    cursor_tick: Option<f64>,
    /// Per-track visibility (index = track number).
    track_visible: Vec<bool>,
    /// Currently selected track (for future editing).
    track_selected: Option<u16>,
    /// Playback state.
    playback: PlaybackState,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let default_w = 1920u32;
        let default_h = 1080u32;

        let render_ctx = RenderContext::new(cc, default_w, default_h);
        let device = render_ctx.device().clone();
        let queue = render_ctx.queue().clone();
        let format = render_ctx.target_format();

        Self {
            render_ctx,
            pianoroll: yinhe_pianoroll::PianorollRenderer::new(device, queue, format),
            midi: None,
            view: yinhe_pianoroll::PianoRollView::default(),
            selected: HashSet::new(),
            file_name: None,
            cursor_tick: None,
            track_visible: Vec::new(),
            track_selected: None,
            playback: PlaybackState::default(),
        }
    }

    fn open_midi_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .pick_file()
        {
            match std::fs::read(&path) {
                Ok(data) => match yinhe_midi::MidiFile::load_from_bytes(&data) {
                    Ok(midi) => {
                        tracing::info!(
                            "Loaded MIDI: {} notes, {} tracks, tpb={}",
                            midi.note_count,
                            midi.track_ports.len(),
                            midi.ticks_per_beat,
                        );
                        let num_tracks = midi.track_ports.len();
                        self.file_name = path
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .map(|s| s.to_string());
                        self.track_visible = vec![true; num_tracks];
                        self.track_selected = None;
                        self.midi = Some(midi);
                        self.selected.clear();
                        self.view = yinhe_pianoroll::PianoRollView::default();
                        self.cursor_tick = None;
                        self.playback = PlaybackState::default();
                    }
                    Err(e) => {
                        tracing::error!("Failed to parse MIDI: {}", e);
                    }
                },
                Err(e) => {
                    tracing::error!("Failed to read file: {}", e);
                }
            }
        }
    }
}

const TRACK_PALETTE: [[f32; 3]; 16] = [
    [0.29, 0.56, 0.89],
    [0.89, 0.35, 0.35],
    [0.30, 0.78, 0.30],
    [0.95, 0.65, 0.20],
    [0.65, 0.40, 0.85],
    [0.20, 0.80, 0.80],
    [0.95, 0.75, 0.20],
    [0.90, 0.45, 0.70],
    [0.40, 0.65, 0.35],
    [0.70, 0.50, 0.30],
    [0.35, 0.55, 0.75],
    [0.85, 0.55, 0.35],
    [0.45, 0.80, 0.55],
    [0.75, 0.35, 0.55],
    [0.55, 0.55, 0.80],
    [0.60, 0.75, 0.30],
];

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ── Keyboard shortcuts ──
        let mut toggle_play = false;
        let mut stop_play = false;
        ui.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                toggle_play = true;
            }
            if i.key_pressed(egui::Key::Escape) {
                stop_play = true;
            }
        });

        // ── Top panel ──
        egui::Panel::top("top_panel").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open MIDI").clicked() {
                    self.open_midi_file();
                }

                // Playback controls
                if self.midi.is_some() {
                    ui.separator();
                    let play_label = if self.playback.is_playing() {
                        "Pause"
                    } else {
                        "Play"
                    };
                    if ui.button(play_label).clicked() {
                        toggle_play = true;
                    }
                    if ui.button("Stop").clicked() {
                        stop_play = true;
                    }
                }

                ui.separator();

                if let Some(ref name) = self.file_name {
                    ui.label(egui::RichText::new(name).strong());
                }

                if let Some(ref midi) = self.midi {
                    ui.separator();
                    ui.label(format!("Notes: {}", midi.note_count));
                    ui.separator();
                    ui.label(format!("Tracks: {}", midi.track_ports.len()));
                    ui.separator();
                    ui.label(format!("TPB: {}", midi.ticks_per_beat));
                }
            });
        });

        // ── Handle playback actions ──
        if let Some(ref midi) = self.midi {
            if toggle_play {
                let tick = self.cursor_tick.unwrap_or(0.0);
                self.playback.toggle_play(tick, midi);
            }
            if stop_play {
                self.playback.stop();
                self.cursor_tick = Some(0.0);
            }

            // Advance cursor during playback
            if let Some((tick, reached_end)) = self.playback.current_tick(midi) {
                self.cursor_tick = Some(tick);
                if reached_end {
                    self.playback.stop();
                }
            }
        }

        // ── Left panel: track list ──
        if self.midi.is_some() {
            egui::Panel::left("track_panel")
                .resizable(true)
                .default_size(200.0)
                .show_inside(ui, |ui| {
                    ui.heading("Tracks");
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(ref midi) = self.midi {
                            let info = midi.track_info();
                            for ti in &info {
                                let idx = ti.index as usize;
                                let color = TRACK_PALETTE[idx % TRACK_PALETTE.len()];
                                let color32 = egui::Color32::from_rgb(
                                    (color[0] * 255.0) as u8,
                                    (color[1] * 255.0) as u8,
                                    (color[2] * 255.0) as u8,
                                );

                                let selected = self.track_selected == Some(ti.index);
                                let frame = egui::Frame::default()
                                    .fill(if selected {
                                        ui.visuals().selection.bg_fill
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    })
                                    .inner_margin(4.0);
                                frame.show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        // Color swatch
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(12.0, 12.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().rect_filled(rect, 2.0, color32);

                                        // Visibility checkbox
                                        let mut vis = self.track_visible.get(idx).copied().unwrap_or(true);
                                        if ui.checkbox(&mut vis, "").changed() {
                                            if idx < self.track_visible.len() {
                                                self.track_visible[idx] = vis;
                                            }
                                        }

                                        // Track name + info
                                        let label = ui.label(
                                            egui::RichText::new(&ti.name).strong(),
                                        );
                                        if label.clicked() {
                                            self.track_selected = Some(ti.index);
                                        }
                                        ui.label(format!("{} notes", ti.note_count));
                                        if ti.port > 0 {
                                            ui.label(format!("P{}", ti.port));
                                        }
                                    });
                                });
                            }
                        }
                    });
                });
        }

        // ── Central panel: piano roll canvas ──
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let available = ui.available_size();
            let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);

            let is_playing = self.playback.is_playing();

            piano_view::show(
                ui,
                available,
                &mut self.pianoroll,
                &mut self.render_ctx,
                &mut self.view,
                midi_source,
                &self.selected,
                &self.track_visible,
                &mut self.cursor_tick,
                is_playing,
            );
        });

        // Request repaint during playback for smooth animation
        if self.playback.is_playing() {
            ui.ctx().request_repaint();
        }
    }
}
