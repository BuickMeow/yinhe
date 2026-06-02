use eframe::egui;
use std::collections::HashSet;

use crate::piano_view;
use crate::render_context::RenderContext;

pub struct App {
    render_ctx: RenderContext,
    pianoroll: yinhe_pianoroll::PianorollRenderer,
    midi: Option<yinhe_midi::MidiFile>,
    view: yinhe_pianoroll::PianoRollView,
    selected: HashSet<(u16, u32)>,
    file_name: Option<String>,
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
                        self.file_name = path
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .map(|s| s.to_string());
                        self.midi = Some(midi);
                        self.selected.clear();
                        self.view = yinhe_pianoroll::PianoRollView::default();
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Top panel: Open button + file info
        egui::Panel::top("top_panel").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open MIDI").clicked() {
                    self.open_midi_file();
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

        // Central panel: piano roll canvas
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let available = ui.available_size();
            let midi_source: Option<&dyn yinhe_pianoroll::NoteSource> =
                self.midi.as_ref().map(|m| m as &dyn yinhe_pianoroll::NoteSource);

            piano_view::show(
                ui,
                available,
                &mut self.pianoroll,
                &mut self.render_ctx,
                &mut self.view,
                midi_source,
                &self.selected,
            );
        });

        ui.ctx().request_repaint();
    }
}
