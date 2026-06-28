use std::sync::{Arc, Mutex, mpsc};

pub(crate) mod actions;
pub(crate) mod audio;
pub(crate) mod main_loop;
pub(crate) mod ui_helpers;

use crate::file_loader::FileLoader;
use crate::dialogs::system_monitor::SystemMonitor;
use yinhe_editor_core::document::Document;
use crate::render_context::RenderContext;
use crate::chrome::mode_bar::ViewMode;
use yinhe_arrangement::ArrangementView;
use yinhe_pianoroll::PianoRollView;

pub struct App {
    // ── Pianoroll (shared GPU resources + global view state) ──
    pub(crate) render_ctx: RenderContext,
    pub(crate) pianoroll: yinhe_pianoroll::PianorollRenderer,
    pub(crate) pianoroll_view: PianoRollView,

    // ── Arrangement (shared GPU resources + global view state) ──
    pub(crate) arr_render_ctx: RenderContext,
    pub(crate) arr_renderer: yinhe_arrangement::PianorollRenderer,
    pub(crate) arrange_view: ArrangementView,
    pub(crate) arr_split: f32,

    // ── Automation panel GPU resources (per-document, per-panel) ──
    pub(crate) controller_renderers: Vec<Vec<(yinhe_automation::PianorollRenderer, RenderContext)>>,

    // ── Multi-document state ──
    pub(crate) documents: Vec<Document>,
    pub(crate) active_doc: Option<usize>,

    // ── Shared state ──
    pub(crate) transport_panel_width: f32,
    pub(crate) file_loader: FileLoader,
    /// Last user-facing load error (e.g. unsupported MIDI). Cleared on dismiss.
    pub(crate) load_error: Option<String>,

    // ── Async save ──
    pub(crate) save_rx: Option<mpsc::Receiver<()>>,

    // ── View mode ──
    pub(crate) view_mode: ViewMode,

    // ── Right panel ──
    pub(crate) right_panel_width: f32,
    pub(crate) right_tab: Option<crate::right_panel::RightTab>,

    // ── Tool palette ──
    pub(crate) active_tool: crate::widgets::tools_panel::Tool,
    pub(crate) show_pianoroll_in_arrange: bool,

    // ── Visibility toggles (derived from view_mode) ──
    pub(crate) show_transport: bool,
    pub(crate) show_pianoroll: bool,
    /// Anchor for shift-click range selection in the track panel.
    /// Set on every non-shift click; consumed on shift-click.
    pub(crate) track_selection_anchor: Option<u16>,

    // ── Manual click tracking for title bar tabs ──
    pub(crate) title_bar_press_pos: Option<egui::Pos2>,

    // ── Cursor tick tracking for cross-view sync ──
    pub(crate) last_cursor_tick: Option<f64>,
    pub(crate) piano_last_cursor_tick: Option<f64>,

    // ── Document switch tracking ──
    pub(crate) prev_active_doc: Option<usize>,

    // ── Cursor-follow mode (shared across arrangement & piano roll) ──
    pub(crate) follow_mode: crate::view_interaction::FollowMode,

    // ── Audio engine ──
    pub(crate) audio: Option<yinhe_audio::CpalAudioHandle>,
    pub(crate) audio_active_doc: Option<usize>,

    // ── Playback cursor interpolation ──
    /// Last known sample position from the audio engine, and the instant we read it.
    /// Used to interpolate cursor position between callback updates.
    pub(crate) playback_anchor: Option<(u64, std::time::Instant)>,

    // ── Settings ──
    pub(crate) audio_settings: crate::audio_settings::AudioSettings,
    /// Tracks the last applied MIDI encoding to detect changes.
    pub(crate) last_midi_encoding: yinhe_mid2::MidiImportEncoding,

    // ── Haptic feedback ──
    pub(crate) haptic_engine: yinhe_haptic::HapticEngine,

    // ── System resource monitoring ──
    pub(crate) sys_monitor: SystemMonitor,

    // ── Memory breakdown popup state ──
    pub(crate) show_mem_breakdown: bool,

    // ── Event browser ──
    pub(crate) event_browser_state: crate::right_panel::event_browser::EventBrowserState,

    // ── Multi-stage loading progress ──
    pub(crate) load_progress: yinhe_editor_core::progress::SharedProgress,

    // ── Async audio export ──
    pub(crate) export_rx: Option<mpsc::Receiver<Result<(), String>>>,
    pub(crate) export_progress: Arc<Mutex<crate::dialogs::export::ExportProgress>>,
    pub(crate) show_export_bit_depth: bool,
    pub(crate) export_bit_depth: yinhe_audio::export::WavBitDepth,
    pub(crate) export_layer_count: u32,
    pub(crate) export_sample_rate: u32, // 0 = 跟随全局设置
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

        let load_progress = yinhe_editor_core::progress::new_shared();

        let mut app = Self {
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
            prev_active_doc: Some(0),

            transport_panel_width: 200.0,
            load_progress: load_progress.clone(),
            file_loader: FileLoader::new(load_progress.clone()),
            load_error: None,
            save_rx: None,
            export_rx: None,
            export_progress: crate::dialogs::export::ExportProgress::new(),
            show_export_bit_depth: false,
            export_bit_depth: yinhe_audio::export::WavBitDepth::Bit24,
            export_layer_count: 4,
            export_sample_rate: 0,

            view_mode: ViewMode::Arrange,
            show_pianoroll_in_arrange: false,
            show_transport: true,
            show_pianoroll: false,
            track_selection_anchor: None,

            right_panel_width: crate::theme::RIGHT_PANEL_DEFAULT_WIDTH,
            right_tab: None,

            active_tool: crate::widgets::tools_panel::Tool::Select,

            title_bar_press_pos: None,

            last_cursor_tick: None,
            piano_last_cursor_tick: None,

            follow_mode: crate::view_interaction::FollowMode::Page,

            audio: None,
            audio_active_doc: None,
            playback_anchor: None,

            audio_settings: crate::audio_settings::load_audio_settings(),
            last_midi_encoding: yinhe_mid2::MidiImportEncoding::Utf8,

            haptic_engine: yinhe_haptic::HapticEngine::new(),

            sys_monitor: SystemMonitor::new(),

            show_mem_breakdown: false,

            event_browser_state: crate::right_panel::event_browser::EventBrowserState::default(),
        };

        // Sync haptic settings from persisted config
        app.haptic_engine.apply_settings(
            app.audio_settings.haptic_enabled,
            app.audio_settings.haptic_intensity,
        );
        app
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
