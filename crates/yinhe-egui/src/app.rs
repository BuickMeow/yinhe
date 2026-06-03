use eframe::egui;

use crate::document::Document;
use crate::file_loader::{FileLoader, MidiLoadResult};
use crate::title_bar::{self, TitleBarAnim};
use crate::track_panel;

use crate::arrangement_view_ui;
use crate::piano_view;
use crate::render_context::RenderContext;

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
    track_panel_width: f32,
    transport_panel_width: f32,
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

    // ── Manual click tracking for title bar tabs ──
    title_bar_press_pos: Option<egui::Pos2>,

    // ── Cursor tick tracking for cross-view sync ──
    last_cursor_tick: Option<f64>,
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

            arr_render_ctx,
            arr_renderer: yinhe_arrangement::PianorollRenderer::new(device, queue, format),
            arr_split: 0.3,

            documents: Vec::new(),
            active_doc: None,

            track_panel_width: 200.0,
            transport_panel_width: 200.0,
            file_loader: FileLoader::new(),

            show_track_panel: true,
            was_track_panel_on: true,
            show_transport: true,
            show_pianoroll: true,

            restore_rect: None,
            anim: None,

            title_bar_press_pos: None,

            last_cursor_tick: None,
        }
    }

    fn active_doc(&self) -> Option<&Document> {
        self.active_doc.and_then(|idx| self.documents.get(idx))
    }

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
        // ── macOS: drive title-bar maximize/restore animation ──
        #[cfg(target_os = "macos")]
        title_bar::process_title_bar_anim(&mut self.anim, ui.ctx());

        // ── Force dark mode ──
        ui.ctx().set_visuals(egui::Visuals::dark());

        // ── Custom title bar ──
        let title_bar_action = title_bar::show(
            ui,
            &self.documents,
            &mut self.active_doc,
            &mut self.title_bar_press_pos,
            #[cfg(target_os = "macos")]
            &mut self.restore_rect,
            #[cfg(target_os = "macos")]
            &mut self.anim,
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
        let mut stop_play = false;
        ui.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                toggle_play = true;
            }
            if i.key_pressed(egui::Key::Escape) {
                stop_play = true;
            }
        });

        let has_active = self.active_doc.is_some();

        // ── Transport bar ──
        egui::Panel::top("transport_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let open_btn = ui.add_enabled(
                    !self.file_loader.is_loading(),
                    egui::Button::new("Open MIDI"),
                );
                if open_btn.clicked() {
                    self.file_loader.pick_midi_file();
                }

                if has_active {
                    ui.separator();
                    let is_playing = self
                        .active_doc()
                        .map(|d| d.playback.is_playing())
                        .unwrap_or(false);
                    let play_label = if is_playing { "Pause" } else { "Play" };
                    if ui.button(play_label).clicked() {
                        toggle_play = true;
                    }
                    if ui.button("Stop").clicked() {
                        stop_play = true;
                    }
                }

                ui.separator();

                if let Some(doc) = self.active_doc() {
                    ui.label(egui::RichText::new(&doc.file_name).strong());
                }

                if let Some(doc) = self.active_doc() {
                    ui.separator();
                    ui.label(format!("Notes: {}", doc.midi.note_count));
                    ui.separator();
                    ui.label(format!("Tracks: {}", doc.midi.track_ports.len()));
                    ui.separator();
                    ui.label(format!("TPB: {}", doc.midi.ticks_per_beat));
                }
            });
        });

        // ── Poll async MIDI loading ──
        match self.file_loader.poll_midi_loading() {
            MidiLoadResult::Loaded { path, midi } => {
                let doc = Document::from_midi(&path, midi);
                self.documents.push(doc);
                self.active_doc = Some(self.documents.len() - 1);
            }
            MidiLoadResult::NotReady => {}
        }

        // ── Handle playback actions ──
        if let Some(idx) = self.active_doc {
            let doc = &mut self.documents[idx];
            if toggle_play {
                let tick = doc.cursor_tick.unwrap_or(0.0);
                doc.playback.toggle_play(tick, &doc.midi);
            }
            if stop_play {
                doc.playback.stop();
                doc.cursor_tick = Some(0.0);
            }
            if let Some((tick, reached_end)) = doc.playback.current_tick(&doc.midi) {
                doc.cursor_tick = Some(tick);
                if reached_end {
                    doc.playback.stop();
                }
            }
        }

        // ── Bottom toggle bar ──
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
                        self.was_track_panel_on = self.show_track_panel;
                        self.show_track_panel = false;
                    } else {
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

        if let Some(idx) = self.active_doc {
            let total = remaining.size();
            let is_playing = self.documents[idx].playback.is_playing();

            let arr_h = if self.show_transport {
                if self.show_pianoroll {
                    (total.y * self.arr_split).max(60.0)
                } else {
                    total.y
                }
            } else {
                0.0
            };
            let bottom_y = remaining.min.y + arr_h
                + if self.show_transport && self.show_pianoroll { 4.0 } else { 0.0 };

            // ── Arrangement view (transport track panel + arrangement GPU) ──
            if self.show_transport {
                // mem::take to work around borrow checker: move doc out, operate, move back
                let mut doc = std::mem::take(&mut self.documents[idx]);

                // Cross-view cursor sync: if pianoroll updated cursor_tick, force arrangement rebuild
                if doc.cursor_tick != self.last_cursor_tick {
                    doc.arr_view.dirty = true;
                }
                self.last_cursor_tick = doc.cursor_tick;

                let arr_total_w = remaining.width();
                let tp_w = self.transport_panel_width
                    .clamp(60.0, (arr_total_w - 60.0).max(60.0));
                self.transport_panel_width = tp_w;

                let arr_rect = egui::Rect::from_min_max(
                    remaining.min,
                    egui::pos2(remaining.max.x, remaining.min.y + arr_h),
                );

                // Transport track panel (left)
                let tp_rect = egui::Rect::from_min_max(
                    arr_rect.min,
                    egui::pos2(arr_rect.min.x + tp_w, arr_rect.max.y),
                );
                let gpu_rect = egui::Rect::from_min_max(
                    egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.min.y),
                    arr_rect.max,
                );

                // Allocate space for transport track panel
                let _tp_child = ui.allocate_ui_with_layout(
                    egui::vec2(tp_w, arr_h),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.set_clip_rect(tp_rect);
                        ui.painter().rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);

                        // Sync track_panel_scroll_y with arrangement scroll_y
                        doc.arr_view.track_panel_scroll_y = doc.arr_view.scroll_y;

                        // Pinch-to-zoom on transport track panel
                        let zoom_delta = ui.input(|i| i.zoom_delta());
                        if (zoom_delta - 1.0).abs() > 0.001 {
                            if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                                if tp_rect.contains(hover) {
                                    let pointer_y = hover.y - tp_rect.min.y;
                                    let old = doc.arr_view.track_panel_row_height;
                                    doc.arr_view.track_panel_row_height =
                                        (doc.arr_view.track_panel_row_height * zoom_delta)
                                            .clamp(16.0, 120.0);
                                    doc.arr_view.lane_height = doc.arr_view.track_panel_row_height;
                                    let track_frac = (pointer_y + doc.arr_view.track_panel_scroll_y) / old;
                                    doc.arr_view.track_panel_scroll_y =
                                        (track_frac * doc.arr_view.track_panel_row_height - pointer_y)
                                            .max(0.0);
                                    doc.arr_view.dirty = true;
                                }
                            }
                        }

                        track_panel::show(
                            ui,
                            &doc.track_info_cache,
                            &mut doc.track_visible,
                            &mut doc.track_selected,
                            &doc.pc_map_cache,
                            &mut doc.arr_view.track_panel_row_height,
                            &mut doc.arr_view.track_panel_scroll_y,
                        );

                        // Write back scroll_y if changed by track panel interaction
                        doc.arr_view.scroll_y = doc.arr_view.track_panel_scroll_y;
                    },
                );

                // Vertical splitter between track panel and arrangement
                let v_handle = egui::Rect::from_min_max(
                    egui::pos2(arr_rect.min.x + tp_w, arr_rect.min.y),
                    egui::pos2(arr_rect.min.x + tp_w + 4.0, arr_rect.max.y),
                );
                let v_resp = ui.interact(v_handle, ui.next_auto_id(), egui::Sense::click_and_drag());
                let v_hovered = v_resp.hovered() || v_resp.dragged();
                ui.painter().rect_filled(
                    v_handle, 0.0,
                    if v_hovered { egui::Color32::from_gray(160) } else { egui::Color32::from_gray(80) },
                );
                if v_resp.dragged() {
                    self.transport_panel_width = (self.transport_panel_width + v_resp.drag_delta().x)
                        .clamp(60.0, arr_total_w - 60.0);
                }
                if v_hovered {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }

                // Horizontal splitter (allocate space)
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

                // Arrangement GPU view
                let arr_midi: Option<&dyn yinhe_arrangement::NoteSource> =
                    Some(&doc.midi as &dyn yinhe_arrangement::NoteSource);
                let track_colors = doc.track_colors();
                let track_names = doc.track_names();
                let gpu_size = gpu_rect.size();
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(gpu_rect), |ui| {
                    arrangement_view_ui::show(
                        ui,
                        gpu_size,
                        &mut self.arr_renderer,
                        &mut self.arr_render_ctx,
                        &mut doc.arr_view,
                        arr_midi,
                        &doc.track_visible,
                        &track_colors,
                        &mut doc.cursor_tick,
                        is_playing,
                        &track_names,
                        &mut doc.arr_instances,
                    );
                });

                // Put doc back
                self.documents[idx] = doc;
            }

            // ── Bottom area: track panel (left) + pianoroll (right) ──
            if self.show_track_panel && self.show_pianoroll {
                // mem::take to work around borrow checker
                let mut doc = std::mem::take(&mut self.documents[idx]);

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

                    // Pinch-to-zoom on pianoroll track panel
                    let zoom_delta = ui.input(|i| i.zoom_delta());
                    if (zoom_delta - 1.0).abs() > 0.001 {
                        if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                            if track_rect.contains(hover) {
                                let pointer_y = hover.y - track_rect.min.y;
                                let old = doc.view.track_panel_row_height;
                                doc.view.track_panel_row_height =
                                    (doc.view.track_panel_row_height * zoom_delta)
                                        .clamp(16.0, 120.0);
                                let track_frac = (pointer_y + doc.view.track_panel_scroll_y) / old;
                                doc.view.track_panel_scroll_y =
                                    (track_frac * doc.view.track_panel_row_height - pointer_y)
                                        .max(0.0);
                            }
                        }
                    }

                    track_panel::show(
                        ui,
                        &doc.track_info_cache,
                        &mut doc.track_visible,
                        &mut doc.track_selected,
                        &doc.pc_map_cache,
                        &mut doc.view.track_panel_row_height,
                        &mut doc.view.track_panel_scroll_y,
                    );
                });

                // Vertical handle
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

                // Pianoroll
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    Some(&doc.midi as &dyn yinhe_pianoroll::NoteSource);
                let piano_rect = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x + sidebar_w + 4.0, bottom_y),
                    remaining.max,
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                    piano_view::show(
                        ui, ui.available_size(),
                        &mut self.pianoroll, &mut self.render_ctx, &mut doc.view,
                        midi_source, &doc.selected, &doc.track_visible,
                        &mut doc.cursor_tick, is_playing,
                    );
                });

                // Put doc back
                self.documents[idx] = doc;
            } else if self.show_track_panel {
                let mut doc = std::mem::take(&mut self.documents[idx]);
                let track_rect = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x, bottom_y),
                    remaining.max,
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(track_rect), |ui| {
                    ui.set_clip_rect(ui.max_rect());
                    ui.painter().rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
                    track_panel::show(
                        ui,
                        &doc.track_info_cache,
                        &mut doc.track_visible,
                        &mut doc.track_selected,
                        &doc.pc_map_cache,
                        &mut doc.view.track_panel_row_height,
                        &mut doc.view.track_panel_scroll_y,
                    );
                });
                self.documents[idx] = doc;
            } else if self.show_pianoroll {
                let doc = &mut self.documents[idx];
                let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                    Some(&doc.midi as &dyn yinhe_pianoroll::NoteSource);
                let piano_rect = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x, bottom_y),
                    remaining.max,
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(piano_rect), |ui| {
                    piano_view::show(
                        ui, ui.available_size(),
                        &mut self.pianoroll, &mut self.render_ctx, &mut doc.view,
                        midi_source, &doc.selected, &doc.track_visible,
                        &mut doc.cursor_tick, is_playing,
                    );
                });
            }
        }

        // ── Request repaint during playback ──
        if self.active_doc()
            .map(|d| d.playback.is_playing())
            .unwrap_or(false)
        {
            ui.ctx().request_repaint();
        }

        // ── Loading overlay ──
        self.file_loader.show_midi_loading_overlay(ui);
        if self.file_loader.is_loading() {
            ui.ctx().request_repaint();
        }
    }
}
