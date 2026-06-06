use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui;

use crate::document::Document;
use crate::file_loader::{FileLoader, MidiLoadResult};
use crate::title_bar;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::arrange;
use crate::mode_bar::{self, ViewMode};
use crate::piano_view;
use crate::render_context::RenderContext;
use crate::transport_bar;

pub struct App {
    // ── Pianoroll (shared GPU resources) ──
    render_ctx: RenderContext,
    pianoroll: yinhe_pianoroll::PianorollRenderer,

    // ── Arrangement (shared GPU resources) ──
    arr_render_ctx: RenderContext,
    arr_renderer: yinhe_arrangement::PianorollRenderer,
    arr_split: f32,

    // ── Multi-document state ──
    documents: Vec<Document>,
    active_doc: Option<usize>,

    // ── Shared state ──
    transport_panel_width: f32,
    file_loader: FileLoader,

    // ── View mode ──
    view_mode: ViewMode,
    show_pianoroll_in_arrange: bool,

    // ── Visibility toggles (derived from view_mode) ──
    show_transport: bool,
    show_pianoroll: bool,

    // ── Manual click tracking for title bar tabs ──
    title_bar_press_pos: Option<egui::Pos2>,

    // ── Cursor tick tracking for cross-view sync ──
    last_cursor_tick: Option<f64>,
    piano_last_cursor_tick: Option<f64>,

    // ── Playback start position (pause returns here) ──
    play_start_tick: f64,

    // ── Cursor-follow mode (shared across arrangement & piano roll) ──
    follow_mode: crate::view_interaction::FollowMode,

    // ── Audio engine ──
    audio_engine: Option<Arc<Mutex<yinhe_audio::AudioEngine>>>,
    audio_sink: Option<yinhe_audio::sink::cpal::CpalSink>,
    audio_active_doc: Option<usize>,

    // ── Settings ──
    audio_settings: crate::settings::AudioSettings,

    // ── System resource monitoring ──
    sysinfo: System,
    self_pid: Option<Pid>,
    last_sys_refresh: Instant,
    cpu_usage: f32,
    mem_mb: f64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ── Load MiSans font ──
        {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "MiSans".to_owned(),
                egui::FontData::from_static(include_bytes!("../../../assets/MiSans-Medium.otf"))
                    .into(),
            );
            let props = fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            props.insert(0, "MiSans".to_owned());
            let mono = fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default();
            mono.insert(0, "MiSans".to_owned());
            cc.egui_ctx.set_fonts(fonts);
        }

        // ── Initialize Material Icons font with adjusted metrics ──
        {
            let mut font_insert = egui_material_icons::font_insert();
            // Default y_offset_factor=0.05 shifts glyphs down, causing them to
            // appear off-center toward bottom-right. Set to 0 for proper centering.
            font_insert.data.tweak.y_offset_factor = 0.0;
            cc.egui_ctx.add_font(font_insert);
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
            pianoroll: yinhe_pianoroll::PianorollRenderer::new(
                device.clone(),
                queue.clone(),
                format,
            ),

            arr_render_ctx,
            arr_renderer: yinhe_arrangement::PianorollRenderer::new(device, queue, format),
            arr_split: 0.3,

            documents: {
                let mut midi = yinhe_midi::MidiFile::default();
                midi.track_ports = vec![0];
                midi.track_names = vec!["Track 1".to_string()];
                let track_info_cache = midi.track_info();
                vec![Document {
                    midi,
                    file_name: "Untitled".into(),
                    track_visible: vec![true],
                    track_info_cache,
                    ..Default::default()
                }]
            },
            active_doc: Some(0),

            transport_panel_width: 200.0,
            file_loader: FileLoader::new(),

            view_mode: ViewMode::Arrange,
            show_pianoroll_in_arrange: true,
            show_transport: true,
            show_pianoroll: true,

            title_bar_press_pos: None,

            last_cursor_tick: None,
            piano_last_cursor_tick: None,

            sysinfo: System::new(),
            self_pid: sysinfo::get_current_pid().ok(),
            play_start_tick: 0.0,
            follow_mode: crate::view_interaction::FollowMode::Page,

            audio_engine: None,
            audio_sink: None,
            audio_active_doc: None,

            audio_settings: crate::settings::AudioSettings::default(),

            last_sys_refresh: Instant::now(),
            cpu_usage: 0.0,
            mem_mb: 0.0,
        }
    }

    fn active_doc(&self) -> Option<&Document> {
        self.active_doc.and_then(|idx| self.documents.get(idx))
    }

    // ── macOS: reserve_render_targets_for_window_anim has been removed ──

    fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        self.documents.remove(index);
        if self.documents.is_empty() {
            self.active_doc = None;
        } else if let Some(active) = self.active_doc {
            if index < active {
                self.active_doc = Some(active - 1);
            } else if index == active {
                self.active_doc = Some(active.min(self.documents.len() - 1));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ── Full-viewport background (matching title bar / transport bar) ──
        let bg = crate::theme::APP_BG;
        ui.painter().rect_filled(ui.ctx().screen_rect(), 0.0, bg);

        // ── Force dark mode ──
        ui.ctx().set_visuals(egui::Visuals::dark());

        // ── Custom title bar ──
        let title_bar_action = title_bar::show(
            ui,
            &self.documents,
            &mut self.active_doc,
            &mut self.title_bar_press_pos,
        );
        // Handle deferred title bar actions (e.g. close a document)
        if let Some(title_bar::TitleBarAction::CloseDocument(idx)) = title_bar_action {
            self.close_document(idx);
        }

        // ── Defensive: ensure active_doc is always in bounds ──
        if let Some(idx) = self.active_doc {
            if idx >= self.documents.len() {
                self.active_doc = if self.documents.is_empty() {
                    None
                } else {
                    Some(self.documents.len() - 1)
                };
            }
        }

        // ── Keyboard shortcuts ──
        let mut toggle_play = false;
        let mut pause_return = false;
        let mut stop_play = false;
        let is_playing_any = self
            .audio_engine
            .as_ref()
            .and_then(|e| e.lock().ok())
            .map(|e| e.is_playing())
            .unwrap_or(false);
        ui.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                if is_playing_any {
                    pause_return = true;
                } else {
                    toggle_play = true;
                }
            }
            if i.key_pressed(egui::Key::Escape) {
                stop_play = true;
            }
        });

        // ── System resource monitoring ──
        if self.last_sys_refresh.elapsed().as_secs_f32() >= 0.5 {
            if let Some(pid) = self.self_pid {
                let _ = self
                    .sysinfo
                    .refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
                if let Some(p) = self.sysinfo.process(pid) {
                    self.cpu_usage = p.cpu_usage();
                    self.mem_mb = p.memory() as f64 / 1_048_576.0;
                }
            }
            self.last_sys_refresh = Instant::now();
        }

        // ── Poll async MIDI loading ──
        match self.file_loader.poll_midi_loading() {
            MidiLoadResult::Loaded { path, midi } => {
                let doc = Document::from_midi(&path, midi);
                self.documents.push(doc);
                self.active_doc = Some(self.documents.len() - 1);
                self.audio_engine = None;
                self.audio_sink = None;
                self.audio_active_doc = None;
            }
            MidiLoadResult::NotReady => {}
        }

        // ── Ensure audio engine is loaded for the active document ──
        if let Some(idx) = self.active_doc {
            if self.audio_active_doc != Some(idx) || self.audio_engine.is_none() {
                let doc = &self.documents[idx];
                let sr = self.audio_settings.sample_rate;
                let mut engine = yinhe_audio::AudioEngine::new(sr);
                engine.load_midi(&doc.midi);
                let sf_path = if !self.audio_settings.default_sf2_path.is_empty() {
                    self.audio_settings.default_sf2_path.clone()
                } else {
                    let default_sf2 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../assets/GeneralUser GS v1.472.sf2");
                    default_sf2.to_string_lossy().to_string()
                };
                let sf = std::path::Path::new(&sf_path);
                if sf.exists() {
                    for port in 0..16u8 {
                        let _ = engine.load_soundfont_for_port(port, &[sf_path.clone()]);
                    }
                }
                let _ = engine.load_soundfonts(&doc.midi);
                self.audio_engine = Some(Arc::new(Mutex::new(engine)));
                self.audio_active_doc = Some(idx);
            }
        }

        // ── Handle playback actions (before transport bar so cursor_tick
        //     is settled before any view reads it) ──
        if let (Some(idx), Some(engine_arc)) = (self.active_doc, &self.audio_engine) {
            let doc = &mut self.documents[idx];
            if toggle_play {
                let engine = engine_arc.lock().unwrap();
                if engine.is_playing() {
                    engine.pause();
                    doc.playback.stop();
                } else {
                    drop(engine);
                    let tick = doc.cursor_tick.unwrap_or(0.0);
                    {
                        let mut engine = engine_arc.lock().unwrap();
                        let sample = (doc.midi.tick_to_seconds(tick as u32) * engine.sample_rate() as f64) as u64;
                        engine.seek(sample);
                        engine.play();
                    }
                    doc.playback.toggle_play(tick, &doc.midi);

                    if self.audio_sink.is_none() {
                        let engine_for_cb = Arc::clone(engine_arc);
                        let render_cb: Arc<Mutex<dyn FnMut(&mut [f32]) + Send>> =
                            Arc::new(Mutex::new(move |buf: &mut [f32]| {
                                if let Ok(mut eng) = engine_for_cb.lock() {
                                    eng.read_samples(buf);
                                } else {
                                    buf.fill(0.0);
                                }
                            }));
                        let sr = engine_arc.lock().unwrap().sample_rate();
                        let sp = engine_arc.lock().unwrap().sample_position_arc();
                        let playing = engine_arc.lock().unwrap().playing_arc();
                        match yinhe_audio::sink::cpal::CpalSink::new(sr, sp, playing, render_cb) {
                            Ok(sink) => {
                                self.audio_sink = Some(sink);
                            }
                            Err(e) => {
                                eprintln!("Failed to create audio sink: {}", e);
                            }
                        }
                    }
                }
            }
            if pause_return {
                let engine = engine_arc.lock().unwrap();
                engine.pause();
                let sample = engine.sample_position();
                let sr = engine.sample_rate();
                drop(engine);
                let time = sample as f64 / sr as f64;
                let cursor_tick = doc.midi.tick_at_time(time);
                doc.cursor_tick = Some(cursor_tick);
                doc.playback.stop();
            }
            if stop_play {
                let mut engine = engine_arc.lock().unwrap();
                engine.stop();
                drop(engine);
                doc.cursor_tick = Some(0.0);
                doc.playback.stop();
            }

            let engine = engine_arc.lock().unwrap();
            if engine.is_playing() {
                let sample = engine.sample_position();
                let sr = engine.sample_rate();
                drop(engine);
                let time = sample as f64 / sr as f64;
                let tick = doc.midi.tick_at_time(time);
                let end_tick = doc.midi.tick_length as f64;
                if tick >= end_tick {
                    engine_arc.lock().unwrap().stop();
                    doc.cursor_tick = Some(0.0);
                    doc.playback.stop();
                } else {
                    doc.cursor_tick = Some(tick.max(0.0));
                }
            }
        }

        // ── Transport bar ──
        let mut pending_quantize = None;
        let mut pending_file_action = None;
        let active_doc = self.active_doc.and_then(|idx| self.documents.get(idx));
        transport_bar::show(
            ui,
            &mut self.file_loader,
            &mut toggle_play,
            &mut pause_return,
            &mut stop_play,
            active_doc,
            self.cpu_usage,
            self.mem_mb,
            &mut pending_quantize,
            &mut pending_file_action,
            &mut self.follow_mode,
        );

        if let (Some(idx), Some(new_preset)) = (self.active_doc, pending_quantize) {
            if let Some(doc) = self.documents.get_mut(idx) {
                doc.quantize = new_preset;
            }
        }

        // ── Handle file menu actions ──
        if let Some(action) = pending_file_action {
            match action {
                transport_bar::FileAction::NewProject => {
                    let mut doc = Document::default();
                    doc.file_name = "Untitled".into();
                    doc.midi.track_ports = vec![0];
                    doc.midi.track_names = vec!["Track 1".to_string()];
                    doc.track_info_cache = doc.midi.track_info();
                    doc.track_visible = vec![true];
                    self.documents.push(doc);
                    self.active_doc = Some(self.documents.len() - 1);
                }
                transport_bar::FileAction::Open => {
                    self.file_loader.pick_midi_file();
                }
                transport_bar::FileAction::CloseDocument => {
                    if let Some(idx) = self.active_doc {
                        self.close_document(idx);
                    }
                }
                transport_bar::FileAction::Exit => {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                transport_bar::FileAction::Settings => {
                    self.audio_settings.show_settings = true;
                }
                _ => {
                    // Save, SaveAs, ExportAudio, ExportMidi
                    // not yet implemented
                }
            }
        }

        // ── Settings panel ──
        let settings_changed = crate::settings::show(ui, &mut self.audio_settings);
        if settings_changed {
            self.audio_engine = None;
            self.audio_sink = None;
            self.audio_active_doc = None;
        }

        // ── Bottom mode bar ──
        mode_bar::show(
            ui,
            &mut self.view_mode,
            &mut self.show_pianoroll_in_arrange,
            &mut self.show_transport,
            &mut self.show_pianoroll,
        );

        // ── Main area: arrangement (top) + pianoroll (bottom) ──
        let remaining = ui.available_rect_before_wrap();

        if let Some(idx) = self.active_doc {
            let total = remaining.size();
            let is_playing = self.audio_engine.as_ref().and_then(|e| e.lock().ok()).map(|e| e.is_playing()).unwrap_or(false);
            let mut follow_mode = self.follow_mode;

            let arr_h = if self.show_transport {
                if self.show_pianoroll {
                    (total.y * self.arr_split).max(60.0)
                } else {
                    total.y
                }
            } else {
                0.0
            };
            let bottom_y = remaining.min.y
                + arr_h
                + if self.show_transport && self.show_pianoroll {
                    4.0
                } else {
                    0.0
                };

            // ── Arrangement view (transport track panel + arrangement GPU) ──
            if self.show_transport {
                let mut doc = std::mem::take(&mut self.documents[idx]);
                arrange::show(
                    ui,
                    &mut doc,
                    remaining,
                    arr_h,
                    &mut self.transport_panel_width,
                    &mut self.arr_renderer,
                    &mut self.arr_render_ctx,
                    &mut self.last_cursor_tick,
                    is_playing,
                    &mut follow_mode,
                );
                self.documents[idx] = doc;
            }

            // ── Pianoroll area ──
            if self.show_pianoroll {
                let mut doc = std::mem::take(&mut self.documents[idx]);

                // Horizontal splitter (between arrangement and pianoroll)
                // Interact rect inset 0.5px at top so it never shares a
                // boundary with the arrangement scrollbar above.
                if self.show_transport {
                    let h_split_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h),
                        egui::pos2(remaining.max.x, remaining.min.y + arr_h + 4.0),
                    );
                    let h_int_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h + 0.5),
                        egui::pos2(remaining.max.x, remaining.min.y + arr_h + 4.0),
                    );
                    let h_split_resp = ui.interact(
                        h_int_rect,
                        ui.id().with("__h_split__"),
                        egui::Sense::click_and_drag(),
                    );
                    ui.painter().rect_filled(
                        h_split_rect,
                        0.0,
                        if h_split_resp.hovered() || h_split_resp.dragged() {
                            crate::theme::SPLIT_HOVER
                        } else {
                            crate::theme::SPLIT_DEFAULT
                        },
                    );
                    if h_split_resp.dragged() {
                        let delta = h_split_resp.drag_delta().y;
                        self.arr_split = ((arr_h + delta) / total.y).clamp(0.1, 0.7);
                    }
                    if h_split_resp.hovered() || h_split_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                }

                // Pianoroll GPU view (full width, no track panel)
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    Some(&doc.midi as &dyn yinhe_pianoroll::NoteSource);
                let piano_rect =
                    egui::Rect::from_min_max(egui::pos2(remaining.min.x, bottom_y), remaining.max);
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                    piano_view::show(
                        ui,
                        ui.available_size(),
                        &mut self.pianoroll,
                        &mut self.render_ctx,
                        &mut doc.view,
                        midi_source,
                        &doc.selected,
                        &doc.track_visible,
                        &mut doc.cursor_tick,
                        is_playing,
                        doc.quantize,
                        doc.midi.ticks_per_beat,
                        Some((
                            doc.midi.ticks_per_beat,
                            doc.midi.time_sig_numerator,
                            doc.midi.time_sig_denominator,
                            doc.midi.time_sig_events.as_slice(),
                        )),
                        &mut self.piano_last_cursor_tick,
                        &mut follow_mode,
                    );
                });

                // Put doc back
                self.documents[idx] = doc;
            }

            self.follow_mode = follow_mode;
        }

        // ── Request repaint during playback ──
        let is_audio_playing = self
            .audio_engine
            .as_ref()
            .and_then(|e| e.lock().ok())
            .map(|e| e.is_playing())
            .unwrap_or(false);
        if is_audio_playing {
            ui.ctx().request_repaint();
        }

        // ── Loading overlay ──
        self.file_loader.show_midi_loading_overlay(ui);
        if self.file_loader.is_loading() {
            ui.ctx().request_repaint();
        }
    }
}
