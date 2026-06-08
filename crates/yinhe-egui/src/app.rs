use eframe::egui;

use crate::document::Document;
use crate::file_loader::{FileLoader, MidiLoadResult};
use crate::title_bar;

use crate::arrange;
use crate::mode_bar::{self, ViewMode};
use crate::piano_view;
use crate::render_context::RenderContext;
use crate::system_monitor::SystemMonitor;
use crate::transport_bar;
use yinhe_arrangement::ArrangementView;
use yinhe_pianoroll::PianoRollView;

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
        ReplaceGuard {
            slot,
            value: Some(value),
        }
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
    // ── Pianoroll (shared GPU resources + global view state) ──
    pub(crate) render_ctx: RenderContext,
    pianoroll: yinhe_pianoroll::PianorollRenderer,
    pub(crate) pianoroll_view: PianoRollView,

    // ── Arrangement (shared GPU resources + global view state) ──
    pub(crate) arr_render_ctx: RenderContext,
    arr_renderer: yinhe_arrangement::PianorollRenderer,
    pub(crate) arrange_view: ArrangementView,
    arr_split: f32,

    // ── Automation panel GPU resources (per-document, per-panel) ──
    controller_renderers: Vec<Vec<(yinhe_automation::PianorollRenderer, RenderContext)>>,

    // ── Multi-document state ──
    pub(crate) documents: Vec<Document>,
    pub(crate) active_doc: Option<usize>,

    // ── Shared state ──
    transport_panel_width: f32,
    pub(crate) file_loader: FileLoader,

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
    pub(crate) audio: Option<yinhe_audio::CpalAudioHandle>,
    pub(crate) audio_active_doc: Option<usize>,

    // ── Settings ──
    pub(crate) audio_settings: crate::settings::AudioSettings,

    // ── System resource monitoring ──
    pub(crate) sys_monitor: SystemMonitor,

    // ── Memory breakdown popup state ──
    pub(crate) show_mem_breakdown: bool,
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
            pianoroll_view: PianoRollView::default(),

            arr_render_ctx,
            arr_renderer: yinhe_arrangement::PianorollRenderer::new(device, queue, format),
            arrange_view: ArrangementView::default(),
            arr_split: crate::theme::DEFAULT_ARR_SPLIT,

            controller_renderers: Vec::new(),

            documents: vec![Document::empty()],
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

            follow_mode: crate::view_interaction::FollowMode::Page,

            audio: None,
            audio_active_doc: None,

            audio_settings: crate::settings::AudioSettings::load(),

            sys_monitor: SystemMonitor::new(),

            show_mem_breakdown: false,
        }
    }

    // ── macOS: reserve_render_targets_for_window_anim has been removed ──

    pub(crate) fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        self.documents.remove(index);
        if index < self.controller_renderers.len() {
            self.controller_renderers.remove(index);
        }
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
            && idx >= self.documents.len()
        {
            self.active_doc = if self.documents.is_empty() {
                None
            } else {
                Some(self.documents.len() - 1)
            };
        }

        // ── Keyboard shortcuts ──
        let (kb_toggle, kb_pause, kb_stop) = self.handle_keyboard_shortcuts(ui);

        // ── System resource monitoring ──
        self.refresh_system_stats();

        // ── Poll async MIDI loading ──
        match self.file_loader.poll_midi_loading() {
            MidiLoadResult::Loaded { path, midi } => {
                // Inherit quantize from the current active document.
                let quantize = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| doc.quantize)
                    .unwrap_or_default();

                let doc = Document::from_midi(&path, midi, quantize);
                let insert_idx = self.documents.len();
                self.documents.push(doc);
                self.active_doc = Some(insert_idx);
                self.teardown_audio();
            }
            MidiLoadResult::NotReady => {}
        }

        // ── Ensure audio engine is loaded for the active document ──
        self.rebuild_audio_if_needed();

        // ── Transport bar (renders before handle_playback so button clicks
        //    are processed in the same frame as keyboard shortcuts) ──
        let active_doc = self.active_doc.and_then(|idx| self.documents.get(idx));
        let transport_response = transport_bar::show(
            ui,
            &mut transport_bar::TransportContext {
                file_loader: &mut self.file_loader,
                doc: active_doc,
                cpu_usage: self.sys_monitor.cpu_usage,
                mem_mb: self.sys_monitor.mem_mb,
                follow_mode: &mut self.follow_mode,
                show_mem_breakdown: &mut self.show_mem_breakdown,
            },
        );

        // ── Handle playback actions (merge keyboard + transport bar inputs) ──
        self.handle_playback(
            kb_toggle || transport_response.toggle_play,
            kb_pause || transport_response.pause_return,
            kb_stop || transport_response.stop_play,
        );

        if let (Some(idx), Some(new_preset)) =
            (self.active_doc, transport_response.pending_quantize)
            && let Some(doc) = self.documents.get_mut(idx)
        {
            doc.quantize = new_preset;
        }

        // ── Memory breakdown popup ──
        self.show_memory_breakdown(ui);

        // ── Handle file menu actions ──
        if let Some(action) = transport_response.pending_file_action {
            self.handle_file_action(action, ui.ctx());
        }

        // ── Settings panel ──
        let settings_changed = crate::settings::show(ui, &mut self.audio_settings);
        if settings_changed {
            self.teardown_audio();
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
            let is_playing = self
                .audio
                .as_ref()
                .map(|a| a.handle.is_playing())
                .unwrap_or(false);
            let mut follow_mode = self.follow_mode;

            let arr_h = if self.show_transport {
                if self.show_pianoroll {
                    (total.y * self.arr_split).max(crate::theme::MIN_ARR_HEIGHT)
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
                    &mut self.arrange_view,
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
                        egui::pos2(
                            remaining.max.x,
                            remaining.min.y + arr_h + crate::theme::SPLIT_GAP,
                        ),
                    );
                    let h_int_rect = egui::Rect::from_min_max(
                        egui::pos2(remaining.min.x, remaining.min.y + arr_h + 0.5),
                        egui::pos2(
                            remaining.max.x,
                            remaining.min.y + arr_h + crate::theme::SPLIT_GAP,
                        ),
                    );
                    let h_split_resp =
                        crate::split_handle::horizontal(ui, "__h_split__", h_int_rect);
                    // Overdraw visual rect — interaction rect is inset 0.5px
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
                        self.arr_split = ((arr_h + delta) / total.y)
                            .clamp(crate::theme::SPLIT_CLAMP_MIN, crate::theme::SPLIT_CLAMP_MAX);
                    }
                }

                // Pianoroll GPU view (full width, no track panel)
                let doc = guard.as_mut();
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    Some(&*doc.midi as &dyn yinhe_pianoroll::NoteSource);
                let piano_rect =
                    egui::Rect::from_min_max(egui::pos2(remaining.min.x, bottom_y), remaining.max);

                // Clone wgpu_state for automation panels before closure borrows render_ctx
                let auto_wgpu_state = self.render_ctx.wgpu_state().clone();
                let auto_lanes = doc.midi.automation_lanes.clone();
                // Ensure controller_renderers has an entry for this document
                while self.controller_renderers.len() <= idx {
                    self.controller_renderers.push(Vec::new());
                }

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                    piano_view::show(
                        ui,
                        ui.available_size(),
                        &mut self.pianoroll,
                        &mut self.render_ctx,
                        &mut self.pianoroll_view,
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
                        // Automation panel data
                        Some(&mut doc.controller_panels),
                        Some(&mut self.controller_renderers[idx]),
                        Some(&auto_lanes),
                        Some(&mut doc.show_controller_panels),
                        Some(&auto_wgpu_state),
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
