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

/// Height of the custom title bar.
const TITLE_BAR_HEIGHT: f32 = 32.0;

// ── macOS custom title bar animation ──

/// State for a smooth maximize/restore animation.
/// Each frame we send slightly different OuterPosition + InnerSize
/// to drive a smooth transition.
#[cfg(target_os = "macos")]
struct TitleBarAnim {
    start: std::time::Instant,
    duration: std::time::Duration,
    from_pos: egui::Pos2,
    from_size: egui::Vec2,
    to_pos: egui::Pos2,
    to_size: egui::Vec2,
}

#[cfg(target_os = "macos")]
fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - f64::powi(-2.0 * t + 2.0, 3) / 2.0
    }
}

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

    // ── Visibility toggles ──
    show_track_panel: bool,
    was_track_panel_on: bool,
    show_transport: bool,
    show_pianoroll: bool,

    // ── Window state (for manual maximize/restore on macOS) ──
    #[cfg(target_os = "macos")]
    restore_rect: Option<egui::Rect>,

    #[cfg(target_os = "macos")]
    anim: Option<TitleBarAnim>,
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

            show_track_panel: true,
            was_track_panel_on: true,
            show_transport: true,
            show_pianoroll: true,

            restore_rect: None,
            anim: None,
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

    /// Render the track list inside a Ui that is already clipped and positioned.
    fn show_track_list(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(ref midi) = self.midi {
                let info = midi.track_info();

                // Build PC map: first ProgramChange event per (port,channel)
                let mut pc_map: std::collections::HashMap<u8, u8> = std::collections::HashMap::new();
                for ev in &midi.control_events {
                    if let yinhe_midi::MidiControlEvent::ProgramChange {
                        channel,
                        program,
                        ..
                    } = ev
                    {
                        pc_map.entry(*channel).or_insert(*program);
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
                            let channel = ti.channel;
                            let port_letter = match ti.port {
                                0 => 'A',
                                1 => 'B',
                                2 => 'C',
                                3 => 'D',
                                4 => 'E',
                                5 => 'F',
                                6 => 'G',
                                7 => 'H',
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
                            let global_ch = ti.port * 16 + (ti.channel - 1);
                            let mut line2 = format!("{} notes", ti.note_count);
                            if let Some(pc) = pc_map.get(&global_ch) {
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
    }

    /// Draw the custom title bar at the top of the window.
    fn show_title_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("title_bar")
            .frame(egui::Frame {
                fill: egui::Color32::from_rgb(25, 25, 28),
                inner_margin: egui::Margin::ZERO,
                outer_margin: egui::Margin::ZERO,
                ..Default::default()
            })
            .show_inside(ui, |ui| {
                let bar_rect = ui.max_rect();

                // Bottom 1px separator line
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(bar_rect.min.x, bar_rect.max.y - 1.0),
                        bar_rect.max,
                    ),
                    0.0,
                    egui::Color32::from_gray(50),
                );

                // macOS: leave ~70px on the left for traffic lights
                let left_padding = if cfg!(target_os = "macos") {
                    70.0
                } else {
                    10.0
                };

                // App name
                ui.painter().text(
                    egui::pos2(bar_rect.min.x + left_padding, bar_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Yinhe MIDI Editor",
                    egui::FontId::proportional(13.0),
                    egui::Color32::from_gray(180),
                );

                // Non-macOS: draw -口x buttons
                #[cfg(not(target_os = "macos"))]
                self.draw_window_buttons(ui, bar_rect);

                // Window drag region
                // macOS: exclude traffic light zone on the left
                // non-macOS: exclude window buttons on the right
                let drag_rect = if cfg!(target_os = "macos") {
                    egui::Rect::from_min_max(
                        egui::pos2(bar_rect.min.x + 70.0, bar_rect.min.y),
                        bar_rect.max,
                    )
                } else {
                    egui::Rect::from_min_max(
                        egui::pos2(bar_rect.min.x + 10.0, bar_rect.min.y),
                        egui::pos2(bar_rect.max.x - 138.0, bar_rect.max.y),
                    )
                };

                let drag_resp = ui.interact(
                    drag_rect,
                    ui.next_auto_id(),
                    egui::Sense::click_and_drag(),
                );
                if drag_resp.dragged_by(egui::PointerButton::Primary) {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                // Double-click title bar to toggle maximize/restore
                if drag_resp.double_clicked_by(egui::PointerButton::Primary) {
                    #[cfg(not(target_os = "macos"))]
                    {
                        // Non-macOS: Maximized has smooth animation naturally
                        let maximized = ui
                            .input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }

                    #[cfg(target_os = "macos")]
                    {
                        // macOS: start a smooth 200ms animation
                        // that interpolates OuterPosition + InnerSize per frame.
                        if self.anim.is_some() {
                            // Already animating; ignore extra double-click.
                        } else if let Some(restore) = self.restore_rect.take() {
                            // Start restore animation
                            let current = ui.input(|i| i.viewport().outer_rect);
                            if let Some(cur) = current {
                                self.anim = Some(TitleBarAnim {
                                    start: std::time::Instant::now(),
                                    duration: std::time::Duration::from_millis(350),
                                    from_pos: cur.min,
                                    from_size: cur.size(),
                                    to_pos: restore.min,
                                    to_size: restore.size(),
                                });
                                ui.ctx().request_repaint();
                            } else {
                                self.restore_rect = Some(restore);
                            }
                        } else {
                            // Start maximize animation
                            let outer = ui.input(|i| i.viewport().outer_rect);
                            let mon = ui.input(|i| i.viewport().monitor_size);
                            if let (Some(cur), Some(mon_size)) = (outer, mon) {
                                self.restore_rect = Some(cur);
                                self.anim = Some(TitleBarAnim {
                                    start: std::time::Instant::now(),
                                    duration: std::time::Duration::from_millis(350),
                                    from_pos: cur.min,
                                    from_size: cur.size(),
                                    to_pos: egui::Pos2::new(0.0, 0.0),
                                    to_size: mon_size,
                                });
                                ui.ctx().request_repaint();
                            }
                        }
                    }
                }

                // Reserve space for title bar height
                ui.allocate_space(egui::vec2(0.0, TITLE_BAR_HEIGHT));
            });
    }

    #[cfg(not(target_os = "macos"))]
    fn draw_window_buttons(
        &mut self,
        ui: &mut egui::Ui,
        bar_rect: egui::Rect,
    ) {
        let btn_w = 46.0;
        let btn_h = TITLE_BAR_HEIGHT;
        let btn_y = bar_rect.min.y;

        let close_rect = egui::Rect::from_min_size(
            egui::pos2(bar_rect.max.x - btn_w, btn_y),
            egui::vec2(btn_w, btn_h),
        );
        let max_rect = egui::Rect::from_min_size(
            egui::pos2(close_rect.min.x - btn_w, btn_y),
            egui::vec2(btn_w, btn_h),
        );
        let min_rect = egui::Rect::from_min_size(
            egui::pos2(max_rect.min.x - btn_w, btn_y),
            egui::vec2(btn_w, btn_h),
        );

        // Close button (✕)
        let close_hover = close_rect
            .contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()));
        if close_hover {
            ui.painter()
                .rect_filled(close_rect, 0.0, egui::Color32::from_rgb(200, 50, 50));
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "✕",
            egui::FontId::proportional(14.0),
            if close_hover {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_gray(180)
            },
        );

        // Maximize (□)
        ui.painter().text(
            max_rect.center(),
            egui::Align2::CENTER_CENTER,
            "□",
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(180),
        );

        // Minimize (─)
        ui.painter().text(
            min_rect.center(),
            egui::Align2::CENTER_CENTER,
            "─",
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(180),
        );

        // Interaction
        let close_resp = ui.interact(close_rect, ui.next_auto_id(), egui::Sense::click());
        let _max_resp = ui.interact(max_rect, ui.next_auto_id(), egui::Sense::click());
        let _min_resp = ui.interact(min_rect, ui.next_auto_id(), egui::Sense::click());

        if close_resp.clicked() {
            frame.close();
        }
    }

    // ── macOS: per-frame title bar animation driver ──

    /// Called at the start of each `ui()` frame. If a title-bar animation
    /// is in progress, interpolates the window position/size and sends
    /// `OuterPosition` + `InnerSize` commands for this frame.
    #[cfg(target_os = "macos")]
    fn process_title_bar_anim(&mut self, ctx: &egui::Context) {
        let Some(anim) = &self.anim else {
            return;
        };

        let elapsed = anim.start.elapsed().as_secs_f64();
        let duration = anim.duration.as_secs_f64();
        let raw_t = (elapsed / duration).min(1.0);
        let eased = ease_in_out_cubic(raw_t) as f32;

        let x = anim.from_pos.x + (anim.to_pos.x - anim.from_pos.x) * eased;
        let y = anim.from_pos.y + (anim.to_pos.y - anim.from_pos.y) * eased;
        let w = anim.from_size.x + (anim.to_size.x - anim.from_size.x) * eased;
        let h = anim.from_size.y + (anim.to_size.y - anim.from_size.y) * eased;

        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y)));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));

        if raw_t >= 1.0 {
            // Snap to final position (ensures exact pixel alignment)
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(anim.to_pos));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(anim.to_size));
            self.anim = None;
        } else {
            ctx.request_repaint();
        }
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
        // ── macOS: drive title-bar maximize/restore animation (per-frame) ──
        #[cfg(target_os = "macos")]
        self.process_title_bar_anim(ui.ctx());

        // ── Force dark mode ──
        ui.ctx().set_visuals(egui::Visuals::dark());

        // ── Custom title bar ──
        self.show_title_bar(ui);

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

        // ── Transport bar (top, always visible) ──
        egui::Panel::top("transport_bar").show_inside(ui, |ui| {
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

        // ── Bottom toggle bar (always visible) ──
        egui::Panel::bottom("bottom_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_track_panel, "音轨面板");
                ui.separator();
                let was_transport = self.show_transport;
                if ui.checkbox(&mut self.show_transport, "走带").changed()
                    && !self.show_transport && !self.show_pianoroll
                {
                    self.show_transport = was_transport;
                }
                ui.separator();
                let was_pianoroll = self.show_pianoroll;
                if ui.checkbox(&mut self.show_pianoroll, "卷帘").changed() {
                    if !self.show_pianoroll {
                        // Turning off pianoroll: auto-close track panel
                        self.was_track_panel_on = self.show_track_panel;
                        self.show_track_panel = false;
                    } else {
                        // Turning on pianoroll: restore track panel
                        self.show_track_panel = self.was_track_panel_on;
                    }
                    if !self.show_transport && !self.show_pianoroll {
                        self.show_pianoroll = was_pianoroll;
                    }
                }
            });
        });

        // ── Main area: arrangement (上) + 品字形 bottom (左下音轨 + 右下卷帘) ──
        let remaining = ui.available_rect_before_wrap();

        if self.midi.is_some() {
            let total = remaining.size();
            let is_playing = self.playback.is_playing();

            // ── Vertical split: arrangement on top, bottom area below (only when pianoroll visible) ──
            let arr_h = if self.show_transport {
                if self.show_pianoroll {
                    (total.y * self.arr_split).max(60.0)
                } else {
                    total.y // fill entire remaining when pianoroll hidden
                }
            } else {
                0.0
            };
            let bottom_y = remaining.min.y + arr_h
                + if self.show_transport && self.show_pianoroll { 4.0 } else { 0.0 };

            // ── Arrangement view (走带) ──
            if self.show_transport {
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);
                let track_colors = self.track_colors();
                let track_names = self.track_names();
                let arr_rect = egui::Rect::from_min_max(
                    remaining.min,
                    egui::pos2(remaining.max.x, remaining.min.y + arr_h),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(arr_rect), |ui| {
                    arrangement_view_ui::show(
                        ui,
                        ui.available_size(),
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
                });

                // Horizontal splitter (only when bottom area exists)
                if self.show_pianoroll {
                    let h_split_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h),
                        egui::pos2(remaining.max.x, remaining.min.y + arr_h + 4.0),
                    );
                    let h_split_resp = ui.interact(h_split_rect, ui.next_auto_id(), egui::Sense::click_and_drag());
                    ui.painter().rect_filled(
                        h_split_rect,
                        0.0,
                        if h_split_resp.hovered() || h_split_resp.dragged() {
                            egui::Color32::from_gray(100)
                        } else {
                            egui::Color32::from_gray(60)
                        },
                    );
                    if h_split_resp.dragged() {
                        let delta = h_split_resp.drag_delta().y;
                        self.arr_split = ((arr_h + delta) / total.y).clamp(0.1, 0.7);
                    }
                }
            }

            // ── Bottom area: track panel (left) + pianoroll (right) ──
            if self.show_track_panel && self.show_pianoroll {
                // Full 品字形 bottom
                let sidebar_w = self.track_panel_width
                    .clamp(60.0, (remaining.width() - 60.0).max(60.0));
                self.track_panel_width = sidebar_w;

                let track_rect = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x, bottom_y),
                    egui::pos2(remaining.min.x + sidebar_w, remaining.max.y),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(track_rect), |ui| {
                    ui.set_clip_rect(ui.max_rect());
                    ui.painter().rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
                    self.show_track_list(ui);
                });

                // Vertical handle between track panel and pianoroll
                let v_handle = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x + sidebar_w, bottom_y),
                    egui::pos2(remaining.min.x + sidebar_w + 4.0, remaining.max.y),
                );
                let v_resp = ui.interact(v_handle, ui.next_auto_id(), egui::Sense::click_and_drag());
                let v_hovered = v_resp.hovered() || v_resp.dragged();
                ui.painter().rect_filled(
                    v_handle, 0.0,
                    if v_hovered { egui::Color32::from_gray(160) } else { egui::Color32::from_gray(80) },
                );
                if v_resp.dragged() {
                    self.track_panel_width = (self.track_panel_width + v_resp.drag_delta().x)
                        .clamp(60.0, remaining.width() - 60.0);
                }
                if v_hovered {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }

                // Pianoroll (right)
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);
                let piano_rect = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x + sidebar_w + 4.0, bottom_y),
                    remaining.max,
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                    piano_view::show(
                        ui, ui.available_size(),
                        &mut self.pianoroll, &mut self.render_ctx, &mut self.view,
                        midi_source, &self.selected, &self.track_visible,
                        &mut self.cursor_tick, is_playing,
                    );
                });
            } else if self.show_track_panel {
                // Only track panel
                let track_rect = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x, bottom_y),
                    remaining.max,
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(track_rect), |ui| {
                    ui.set_clip_rect(ui.max_rect());
                    ui.painter().rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
                    self.show_track_list(ui);
                });
            } else if self.show_pianoroll {
                // Only pianoroll
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);
                let piano_rect = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x, bottom_y),
                    remaining.max,
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                    piano_view::show(
                        ui, ui.available_size(),
                        &mut self.pianoroll, &mut self.render_ctx, &mut self.view,
                        midi_source, &self.selected, &self.track_visible,
                        &mut self.cursor_tick, is_playing,
                    );
                });
            }
        } else if self.show_pianoroll {
            // ── No MIDI loaded — show pianoroll only ──
            let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);
            let is_playing = self.playback.is_playing();
            piano_view::show(
                ui, remaining.size(),
                &mut self.pianoroll, &mut self.render_ctx, &mut self.view,
                midi_source, &self.selected, &self.track_visible,
                &mut self.cursor_tick, is_playing,
            );
        }

        // ── Request repaint during playback ──
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
