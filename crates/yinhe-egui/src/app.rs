use eframe::egui;
use std::collections::HashSet;

use crate::arrangement_view_ui;
use crate::piano_view;
use crate::playback::PlaybackState;
use crate::render_context::RenderContext;

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

pub struct App {
    // ── Pianoroll ──
    render_ctx: RenderContext,
    pianoroll: yinhe_pianoroll::PianorollRenderer,
    view: yinhe_pianoroll::PianoRollView,

    // ── Arrangement ──
    arr_render_ctx: RenderContext,
    arr_renderer: yinhe_pianoroll::PianorollRenderer,
    arr_view: yinhe_pianoroll::ArrangementView,
    arr_split: f32, // fraction of central area for arrangement (0.0-1.0)
    arr_instances: Vec<yinhe_pianoroll::NoteInstance>, // reusable scratch buffer

    // ── Shared state ──
    midi: Option<yinhe_midi::MidiFile>,
    selected: HashSet<(u16, u32)>,
    file_name: Option<String>,
    cursor_tick: Option<f64>,
    track_visible: Vec<bool>,
    track_selected: Option<u16>,
    track_panel_width: f32,
    playback: PlaybackState,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let default_w = 1920u32;
        let default_h = 1080u32;

        let render_ctx = RenderContext::new(cc, default_w, default_h);
        let arr_render_ctx = RenderContext::new(cc, default_w, default_h / 3);

        let device = render_ctx.device().clone();
        let queue = render_ctx.queue().clone();
        let format = render_ctx.target_format();

        Self {
            render_ctx,
            pianoroll: yinhe_pianoroll::PianorollRenderer::new(device.clone(), queue.clone(), format),
            view: yinhe_pianoroll::PianoRollView::default(),

            arr_render_ctx,
            arr_renderer: yinhe_pianoroll::PianorollRenderer::new(device, queue, format),
            arr_view: yinhe_pianoroll::ArrangementView::default(),
            arr_split: 0.3,
            arr_instances: Vec::new(),

            midi: None,
            selected: HashSet::new(),
            file_name: None,
            cursor_tick: None,
            track_visible: Vec::new(),
            track_selected: None,
            track_panel_width: 200.0,
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
                        self.arr_view = yinhe_pianoroll::ArrangementView::default();
                        self.arr_instances.clear();
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

    /// Build track colors from palette + track count.
    fn track_colors(&self) -> Vec<[f32; 3]> {
        let n = self.track_visible.len();
        (0..n)
            .map(|i| TRACK_PALETTE[i % TRACK_PALETTE.len()])
            .collect()
    }

    /// Collect track names for the arrangement view.
    fn track_names(&self) -> Vec<String> {
        self.midi
            .as_ref()
            .map(|m| m.track_names.clone())
            .unwrap_or_default()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ── Force dark mode ──
        ui.ctx().set_visuals(egui::Visuals::dark());

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

            if let Some((tick, reached_end)) = self.playback.current_tick(midi) {
                self.cursor_tick = Some(tick);
                if reached_end {
                    self.playback.stop();
                }
            }
        }

        // ── Main area: manual sidebar + central ──
        if self.midi.is_some() {
            let remaining = ui.available_rect_before_wrap();
            let sidebar_w = self.track_panel_width
                .min(remaining.width() - 150.0)
                .max(60.0);
            self.track_panel_width = sidebar_w;

            // ── Sidebar ──
            let sidebar_rect = egui::Rect::from_min_max(
                remaining.min,
                egui::pos2(remaining.min.x + sidebar_w, remaining.max.y),
            );
            ui.allocate_ui_at_rect(sidebar_rect, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Tracks");
                    ui.separator();
                    if let Some(ref midi) = self.midi {
                        let info = midi.track_info();
                        let avail_w = ui.available_width();
                        let name_max_w = (avail_w - 160.0).max(40.0);

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
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(12.0, 12.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(rect, 2.0, color32);

                                    let mut vis =
                                        self.track_visible.get(idx).copied().unwrap_or(true);
                                    if ui.checkbox(&mut vis, "").changed() {
                                        if idx < self.track_visible.len() {
                                            self.track_visible[idx] = vis;
                                        }
                                    }

                                    let name = egui::RichText::new(&ti.name).strong();
                                    let label = ui.add_sized(
                                        [name_max_w, ui.spacing().interact_size.y],
                                        egui::Label::new(name),
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

            // ── Vertical resize handle ──
            let handle_rect = egui::Rect::from_min_max(
                egui::pos2(remaining.min.x + sidebar_w, remaining.min.y),
                egui::pos2(remaining.min.x + sidebar_w + 4.0, remaining.max.y),
            );
            let handle_resp = ui.interact(handle_rect, ui.next_auto_id(), egui::Sense::click_and_drag());
            let hovered = handle_resp.hovered() || handle_resp.dragged();
            ui.painter().rect_filled(
                handle_rect,
                0.0,
                if hovered {
                    egui::Color32::from_gray(160)
                } else {
                    egui::Color32::from_gray(80)
                },
            );
            if handle_resp.dragged() {
                self.track_panel_width += handle_resp.drag_delta().x;
            }
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }

            // ── Central area ──
            let central_rect = egui::Rect::from_min_max(
                egui::pos2(remaining.min.x + sidebar_w + 4.0, remaining.min.y),
                remaining.max,
            );
            ui.allocate_ui_at_rect(central_rect, |ui| {
                let total = ui.available_size();
                let is_playing = self.playback.is_playing();
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);
                let track_colors = self.track_colors();
                let track_names = self.track_names();

                // Split: arrangement on top, pianoroll on bottom
                let arr_h = (total.y * self.arr_split).max(60.0);
                let piano_h = (total.y - arr_h - 4.0).max(60.0);

                // Arrangement view
                arrangement_view_ui::show(
                    ui,
                    egui::vec2(total.x, arr_h),
                    &mut self.arr_renderer,
                    &mut self.arr_render_ctx,
                    &mut self.arr_view,
                    midi_source,
                    &self.track_visible,
                    &track_colors,
                    &mut self.cursor_tick,
                    is_playing,
                    &track_names,
                    &mut self.arr_instances,
                );

                // Draggable horizontal split handle
                ui.horizontal(|ui| {
                    let (_, resp) = ui.allocate_exact_size(
                        egui::vec2(total.x, 4.0),
                        egui::Sense::click_and_drag(),
                    );
                    ui.painter().rect_filled(
                        resp.rect,
                        0.0,
                        if resp.hovered() || resp.dragged() {
                            egui::Color32::from_gray(100)
                        } else {
                            egui::Color32::from_gray(60)
                        },
                    );
                    if resp.dragged() {
                        let delta = resp.drag_delta().y;
                        self.arr_split =
                            ((arr_h + delta) / total.y).clamp(0.1, 0.7);
                    }
                });

                // Pianoroll view
                piano_view::show(
                    ui,
                    egui::vec2(total.x, piano_h),
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
        } else {
            // No MIDI loaded — show pianoroll only
            let total = ui.available_size();
            let is_playing = self.playback.is_playing();
            let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);
            piano_view::show(
                ui,
                total,
                &mut self.pianoroll,
                &mut self.render_ctx,
                &mut self.view,
                midi_source,
                &self.selected,
                &self.track_visible,
                &mut self.cursor_tick,
                is_playing,
            );
        }

        // Request repaint during playback
        if self.playback.is_playing() {
            ui.ctx().request_repaint();
        }
    }
}
