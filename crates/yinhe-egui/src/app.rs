use eframe::egui;
use std::collections::HashSet;
use std::sync::mpsc;

use crate::loading::{MidiLoadEvent, MidiLoader};

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
    file_loader: FileLoader,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ── Load MiSans font ──
        {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "MiSans".to_owned(),
                egui::FontData::from_static(include_bytes!(
                    "../../../assets/MiSans-Regular.otf"
                ))
                .into(),
            );
            // Set MiSans as the default proportional font
            let props = fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            props.insert(0, "MiSans".to_owned());
            // Also set for monospace
            let mono = fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default();
            mono.insert(0, "MiSans".to_owned());
            cc.egui_ctx.set_fonts(fonts);
        }

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
            file_loader: FileLoader::new(),
        }
    }

    /// Called when async loading completes.
    fn on_midi_loaded(&mut self, path: String, midi: yinhe_midi::MidiFile) {
        tracing::info!(
            "Loaded MIDI: {} notes, {} tracks, tpb={}",
            midi.note_count,
            midi.track_ports.len(),
            midi.ticks_per_beat,
        );
        let num_tracks = midi.track_ports.len();
        self.file_name = std::path::Path::new(&path)
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

// ── Async MIDI file loading ──

pub(crate) enum MidiLoadResult {
    Loaded {
        path: String,
        midi: yinhe_midi::MidiFile,
    },
    NotReady,
}

pub(crate) struct FileLoader {
    midi_loader: Option<MidiLoader>,
}

impl FileLoader {
    pub fn new() -> Self {
        Self { midi_loader: None }
    }

    pub fn is_loading(&self) -> bool {
        self.midi_loader.is_some()
    }

    /// Show file dialog and start loading MIDI in a background thread.
    pub fn pick_midi_file(&mut self) {
        if self.is_loading() {
            return;
        }

        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .pick_file()
        {
            let (tx, rx) = mpsc::channel();
            let path_str = path.to_string_lossy().to_string();
            let path_for_thread = path_str.clone();

            std::thread::spawn(move || {
                let data = match std::fs::read(&path_for_thread) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx.send(MidiLoadEvent::Complete(Box::new(Err(
                            yinhe_midi::MidiError::Io(e),
                        ))));
                        return;
                    }
                };
                let result = yinhe_midi::MidiFile::load_from_bytes_with_progress(
                    &data,
                    |progress| {
                        let _ = tx.send(MidiLoadEvent::Progress(progress));
                    },
                );
                let _ = tx.send(MidiLoadEvent::Complete(Box::new(result)));
            });

            self.midi_loader = Some(MidiLoader {
                path: path_str,
                rx,
                current_progress: None,
            });
        }
    }

    /// Poll the background thread for loading progress/completion.
    pub fn poll_midi_loading(&mut self) -> MidiLoadResult {
        if let Some(mut loader) = self.midi_loader.take() {
            while let Ok(event) = loader.rx.try_recv() {
                match event {
                    MidiLoadEvent::Progress(progress) => {
                        loader.current_progress = Some(progress);
                    }
                    MidiLoadEvent::Complete(result) => {
                        match *result {
                            Ok(midi) => {
                                let path = loader.path.clone();
                                return MidiLoadResult::Loaded { path, midi };
                            }
                            Err(e) => {
                                tracing::error!("Failed to load MIDI: {}", e);
                            }
                        }
                        return MidiLoadResult::NotReady;
                    }
                }
            }
            self.midi_loader = Some(loader);
        }
        MidiLoadResult::NotReady
    }

    /// Draw a dark overlay + centered window with loading progress.
    pub fn show_midi_loading_overlay(&self, ui: &mut egui::Ui) {
        if let Some(loader) = &self.midi_loader {
            let screen_rect = ui.ctx().content_rect();
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    "midi_loading_overlay".into(),
                ))
                .rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_rgba_premultiplied(0, 0, 0, 160),
                );

            egui::Window::new("Loading MIDI")
                .order(egui::Order::Tooltip)
                .collapsible(false)
                .resizable(false)
                .movable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    if let Some(progress) = &loader.current_progress {
                        ui.label(format!(
                            "Parsing track {} / {}...",
                            progress.current_track, progress.total_tracks
                        ));
                        let ratio =
                            progress.current_track as f32 / progress.total_tracks.max(1) as f32;
                        ui.add(egui::ProgressBar::new(ratio).show_percentage());
                    } else {
                        ui.label("Reading MIDI file...");
                        ui.add(egui::Spinner::new());
                    }
                });
        }
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
                let open_btn = ui.add_enabled(
                    !self.file_loader.is_loading(),
                    egui::Button::new("Open MIDI"),
                );
                if open_btn.clicked() {
                    self.file_loader.pick_midi_file();
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

        // ── Poll async MIDI loading ──
        match self.file_loader.poll_midi_loading() {
            MidiLoadResult::Loaded { path, midi } => {
                self.on_midi_loaded(path, midi);
            }
            MidiLoadResult::NotReady => {}
        }

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

        // ── Main area: sidebar (left) + central (right), manual positions ──
        if self.midi.is_some() {
            let remaining = ui.available_rect_before_wrap();
            let sidebar_w = self.track_panel_width
                .clamp(60.0, (remaining.width() - 60.0).max(60.0));
            self.track_panel_width = sidebar_w;

            // ── Sidebar (allocated at fixed rect, not consuming layout space) ──
            let sidebar_rect = egui::Rect::from_min_max(
                remaining.min,
                egui::pos2(remaining.min.x + sidebar_w, remaining.max.y),
            );
            ui.allocate_ui_at_rect(sidebar_rect, |ui| {
                // Clamp rendering to sidebar bounds only
                ui.set_clip_rect(ui.max_rect());
                ui.painter().rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(ref midi) = self.midi {
                        let info = midi.track_info();

                        // Build PC map: first ProgramChange event per channel
                        let mut pc_map: [Option<u8>; 16] = [None; 16];
                        for ev in &midi.control_events {
                            if let yinhe_midi::MidiControlEvent::ProgramChange {
                                channel,
                                program,
                                ..
                            } = ev
                            {
                            if *channel < 16 && pc_map[*channel as usize].is_none() {
                                pc_map[*channel as usize] = Some(*program);
                            }
                            }
                        }

                        for ti in &info {
                            let idx = ti.index as usize;
                            let color = TRACK_PALETTE[idx % TRACK_PALETTE.len()];
                            let color32 = egui::Color32::from_rgb(
                                (color[0] * 255.0) as u8,
                                (color[1] * 255.0) as u8,
                                (color[2] * 255.0) as u8,
                            );

                            let selected = self.track_selected == Some(ti.index);
                            let bg = if selected {
                                ui.visuals().selection.bg_fill
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            let frame = egui::Frame::default()
                                .fill(bg)
                                .inner_margin(egui::Margin::symmetric(6, 3));
                            frame.show(ui, |ui| {
                                // ── Line 1: channel badge + track name ──
                                ui.horizontal(|ui| {
                                    // Visibility checkbox
                                    let mut vis =
                                        self.track_visible.get(idx).copied().unwrap_or(true);
                                    if ui.checkbox(&mut vis, "").changed() {
                                        if idx < self.track_visible.len() {
                                            self.track_visible[idx] = vis;
                                        }
                                    }

                                    // Channel badge: small rounded rect
                                    let channel = (ti.port & 0x0F) + 1;
                                    let port_letter = match ti.port >> 4 {
                                        0 => 'A', 1 => 'B', 2 => 'C', 3 => 'D',
                                        4 => 'E', 5 => 'F', 6 => 'G', 7 => 'H',
                                        _ => '?',
                                    };
                                    let badge_text = format!("{}{:02}", port_letter, channel);
                                    let (_badge, _) = ui.allocate_exact_size(
                                        egui::vec2(28.0, 16.0),
                                        egui::Sense::hover(),
                                    );
                                    let badge_rect = ui.min_rect();
                                    let badge_rect = egui::Rect::from_min_size(
                                        egui::pos2(badge_rect.min.x, badge_rect.min.y + 2.0),
                                        egui::vec2(28.0, 14.0),
                                    );
                                    ui.painter().rect_filled(badge_rect, 3.0, color32);
                                    ui.painter().text(
                                        badge_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        badge_text,
                                        egui::FontId::monospace(10.0),
                                        egui::Color32::WHITE,
                                    );

                                    // Track name with ellipsis truncation
                                    let name_w = ui.available_width().max(10.0);
                                    let name = egui::RichText::new(&ti.name).size(13.0);
                                    let label = ui.add_sized(
                                        [name_w, 16.0],
                                        egui::Label::new(name).truncate(),
                                    );
                                    if label.clicked() {
                                        self.track_selected = Some(ti.index);
                                    }
                                });

                                // ── Line 2: note count + optional PC ──
                                {
                                    let channel_idx = (ti.port & 0x0F) as usize;
                                    let mut line2 = format!("{} notes", ti.note_count);
                                    if let Some(pc) = pc_map[channel_idx] {
                                        line2.push_str(&format!(" | PC:{}", pc));
                                    }
                                    let w2 = ui.available_width().max(10.0);
                                    ui.add_sized(
                                        [w2, 14.0],
                                        egui::Label::new(
                                            egui::RichText::new(line2)
                                                .size(11.0)
                                                .color(egui::Color32::GRAY),
                                        )
                                        .truncate(),
                                    );
                                }
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
                let new_w = self.track_panel_width + handle_resp.drag_delta().x;
                self.track_panel_width = new_w.clamp(60.0, remaining.width() - 60.0);
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

        // ── Loading overlay (drawn last, on top) ──
        self.file_loader.show_midi_loading_overlay(ui);
        if self.file_loader.is_loading() {
            ui.ctx().request_repaint();
        }
    }
}
