use std::sync::Arc;
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
use yinhe_memtrace::{AllocTag, Snapshot};

// ── Panic-safe take guard ──
/// Restores a taken value back into its slot on drop, preventing data loss
/// if a panic occurs between `std::mem::take` and the manual put-back.
struct ReplaceGuard<'a, T> {
    slot: &'a mut T,
    value: Option<T>,
}

impl<'a, T> ReplaceGuard<'a, T> {
    fn new(slot: &'a mut T) -> Self
    where
        T: Default,
    {
        let value = std::mem::take(slot);
        ReplaceGuard { slot, value: Some(value) }
    }

    fn as_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("ReplaceGuard already consumed")
    }
}

impl<'a, T> Drop for ReplaceGuard<'a, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            *self.slot = value;
        }
    }
}

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

    // ── Cursor-follow mode (shared across arrangement & piano roll) ──
    follow_mode: crate::view_interaction::FollowMode,

    // ── Audio engine ──
    audio: Option<yinhe_audio::CpalAudioHandle>,
    audio_active_doc: Option<usize>,

    // ── Settings ──
    audio_settings: crate::settings::AudioSettings,

    // ── System resource monitoring ──
    sysinfo: System,
    self_pid: Option<Pid>,
    last_sys_refresh: Instant,
    cpu_usage: f32,
    mem_mb: f64,

    // ── Memory breakdown popup state ──
    show_mem_breakdown: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ── Load MiSans font ──
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
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

            // Initialize Material Icons font with adjusted metrics
            let mut font_insert = egui_material_icons::font_insert();
            // Default y_offset_factor=0.05 shifts glyphs down, causing them to
            // appear off-center toward bottom-right. Set to 0 for proper centering.
            font_insert.data.tweak.y_offset_factor = 0.0;
            cc.egui_ctx.add_font(font_insert);
        });

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
                let midi = {
                    let mut m = yinhe_midi::MidiFile::default();
                    m.track_ports = vec![0];
                    m.track_names = vec!["Track 1".to_string()];
                    m
                };
                let track_info_cache = midi.track_info();
                vec![Document {
                    midi: Arc::new(midi),
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
            follow_mode: crate::view_interaction::FollowMode::Page,

            audio: None,
            audio_active_doc: None,

            audio_settings: crate::settings::AudioSettings::load(),

            last_sys_refresh: Instant::now(),
            cpu_usage: 0.0,
            mem_mb: 0.0,

            show_mem_breakdown: false,
        }
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
        if let Some(idx) = self.active_doc
            && idx >= self.documents.len() {
                self.active_doc = if self.documents.is_empty() {
                    None
                } else {
                    Some(self.documents.len() - 1)
                };
            }

        // ── Keyboard shortcuts ──
        let mut toggle_play = false;
        let mut pause_return = false;
        let mut stop_play = false;
        let is_playing_any = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
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
                self.audio = None;
                self.audio_active_doc = None;
            }
            MidiLoadResult::NotReady => {}
        }

        // ── Ensure audio engine is loaded for the active document ──
        if let Some(idx) = self.active_doc {
            let needs_rebuild = self.audio_active_doc != Some(idx) || self.audio.is_none();

            if needs_rebuild {
                // Drop old audio (stops cpal stream, frees engine)
                self.audio = None;

                let doc = &self.documents[idx];
                let sr = self.audio_settings.sample_rate;
                let (num_ch, active_mask) = yinhe_audio::channels_for_midi(&doc.midi);

                match yinhe_audio::spawn_cpal_audio(sr, num_ch, active_mask) {
                    Ok(audio) => {
                        // Load MIDI
                        audio.handle.send(yinhe_audio::AudioCommand::LoadMidi {
                            midi: Arc::clone(&doc.midi),
                        });
                        // Load SoundFont
                        let sf_path = if !self.audio_settings.default_sf2_path.is_empty() {
                            self.audio_settings.default_sf2_path.clone()
                        } else {
                            let default_sf2 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                                .join("../assets/GeneralUser GS v1.472.sf2");
                            default_sf2.to_string_lossy().to_string()
                        };
                        let sf = std::path::Path::new(&sf_path);
                        if sf.exists() {
                            let num_ports = (num_ch / 16) as u8;
                            for port in 0..num_ports {
                                audio.handle.send(yinhe_audio::AudioCommand::LoadSoundFont {
                                    port,
                                    paths: vec![sf_path.clone()],
                                });
                            }
                        }
                        self.audio = Some(audio);
                        self.audio_active_doc = Some(idx);
                    }
                    Err(e) => {
                        eprintln!("Failed to create audio: {}", e);
                    }
                }
            }
        }

        // ── Handle playback actions ──
        if let (Some(idx), Some(audio)) = (self.active_doc, &self.audio) {
            let doc = &mut self.documents[idx];
            let handle = &audio.handle;

            if toggle_play {
                if handle.is_playing() {
                    handle.send(yinhe_audio::AudioCommand::Pause);
                    let sample = handle.sample_position();
                    let time = sample as f64 / audio.sample_rate as f64;
                    doc.cursor_tick = Some(doc.midi.tick_at_time(time));
                    doc.playback.stop();
                } else {
                    let tick = doc.cursor_tick.unwrap_or(0.0);
                    let cursor_sample = (doc.midi.tick_to_seconds(tick as u32) * audio.sample_rate as f64) as u64;
                    let engine_sample = handle.sample_position();
                    // If cursor is at the engine's position, just resume (no seek)
                    if cursor_sample.abs_diff(engine_sample) < (audio.sample_rate as u64 / 10) {
                        handle.send(yinhe_audio::AudioCommand::Resume);
                    } else {
                        handle.send(yinhe_audio::AudioCommand::Play { from_sample: cursor_sample });
                    }
                    doc.playback.toggle_play(tick, &doc.midi);
                }
            }
            if pause_return {
                handle.send(yinhe_audio::AudioCommand::Pause);
                let sample = handle.sample_position();
                let time = sample as f64 / audio.sample_rate as f64;
                doc.cursor_tick = Some(doc.midi.tick_at_time(time));
                doc.playback.stop();
            }
            if stop_play {
                handle.send(yinhe_audio::AudioCommand::Stop);
                doc.cursor_tick = Some(0.0);
                doc.playback.stop();
            }

            // Sync cursor from audio position during playback
            if handle.is_playing() {
                let sample = handle.sample_position();
                let time = sample as f64 / audio.sample_rate as f64;
                let tick = doc.midi.tick_at_time(time);
                let end_tick = doc.midi.tick_length as f64;
                if tick >= end_tick {
                    handle.send(yinhe_audio::AudioCommand::Stop);
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
            &mut self.show_mem_breakdown,
        );

        if let (Some(idx), Some(new_preset)) = (self.active_doc, pending_quantize)
            && let Some(doc) = self.documents.get_mut(idx) {
                doc.quantize = new_preset;
            }

        // ── Memory breakdown popup ──
        if self.show_mem_breakdown {
            let snapshot = Snapshot::capture();
            egui::Window::new("内存占用详情")
                .id(egui::Id::new("memory_breakdown_window"))
                .default_size([360.0, 260.0])
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(format!(
                        "系统统计总内存: {:.1} MB",
                        self.mem_mb
                    ));
                    ui.label(format!(
                        "分配器追踪内存: {:.1} MB",
                        snapshot.total_mb()
                    ));
                    ui.label(format!(
                        "wgpu 显式 GPU 资源: {:.1} MB",
                        snapshot.gpu_mb()
                    ));

                    #[cfg(target_os = "macos")]
                    {
                        let metal_size = self
                            .render_ctx
                            .metal_allocated_size()
                            .unwrap_or(0)
                            .saturating_add(
                                self.arr_render_ctx
                                    .metal_allocated_size()
                                    .unwrap_or(0),
                            );
                        ui.label(format!(
                            "Metal 驱动真实显存: {:.1} MB",
                            metal_size as f64 / 1_048_576.0
                        ));
                    }

                    ui.separator();

                    ui.heading("按子系统分类");
                    egui::Grid::new("mem_breakdown_grid")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            for tag in AllocTag::ALL {
                                if tag == AllocTag::Unknown && snapshot.get(tag) <= 0 {
                                    continue;
                                }
                                ui.label(tag.name());
                                ui.label(format!("{:.1} MB", snapshot.mb(tag)));
                                ui.end_row();
                            }
                        });

                    ui.separator();
                    ui.small(
                        "注：GPU 资源计数反映应用显式创建的 wgpu Texture/Buffer 大小；\
                         驱动层额外开销（swapchain、depth、pipeline cache 等）\
                         不纳入此项统计。",
                    );

                    if ui.button("关闭").clicked() {
                        self.show_mem_breakdown = false;
                    }
                });
        }

        // ── Handle file menu actions ──
        if let Some(action) = pending_file_action {
            match action {
                transport_bar::FileAction::NewProject => {
                    let midi = {
                        let mut m = yinhe_midi::MidiFile::default();
                        m.track_ports = vec![0];
                        m.track_names = vec!["Track 1".to_string()];
                        m
                    };
                    let track_info_cache = midi.track_info();
                    let doc = Document {
                        midi: Arc::new(midi),
                        file_name: "Untitled".into(),
                        track_info_cache,
                        track_visible: vec![true],
                        ..Default::default()
                    };
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
            self.audio = None;
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
            let is_playing = self.audio.as_ref().map(|a| a.handle.is_playing()).unwrap_or(false);
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
                    crate::theme::SPLIT_GAP
                } else {
                    0.0
                };

            // ── Arrangement view (transport track panel + arrangement GPU) ──
            if self.show_transport {
                let mut guard = ReplaceGuard::new(&mut self.documents[idx]);
                arrange::show(
                    ui,
                    guard.as_mut(),
                    remaining,
                    arr_h,
                    &mut self.transport_panel_width,
                    &mut self.arr_renderer,
                    &mut self.arr_render_ctx,
                    &mut self.last_cursor_tick,
                    is_playing,
                    &mut follow_mode,
                );
                // guard drops here → document restored even on panic
            }

            // ── Pianoroll area ──
            if self.show_pianoroll {
                let mut guard = ReplaceGuard::new(&mut self.documents[idx]);

                // Horizontal splitter (between arrangement and pianoroll)
                // Interact rect inset 0.5px at top so it never shares a
                // boundary with the arrangement scrollbar above.
                if self.show_transport {
                    let h_split_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h),
                        egui::pos2(remaining.max.x, remaining.min.y + arr_h + crate::theme::SPLIT_GAP),
                    );
                    let h_int_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h + 0.5),
                        egui::pos2(remaining.max.x, remaining.min.y + arr_h + crate::theme::SPLIT_GAP),
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
                let doc = guard.as_mut();
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    Some(&*doc.midi as &dyn yinhe_pianoroll::NoteSource);
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
                // guard drops here → document restored even on panic
            }

            self.follow_mode = follow_mode;
        }

        // ── Request repaint during playback ──
        let is_audio_playing = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
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
