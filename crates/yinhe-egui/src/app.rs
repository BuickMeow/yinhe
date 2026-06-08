use crate::dialogs::file_loader::FileLoader;
use crate::dialogs::system_monitor::SystemMonitor;
use crate::document::Document;
use crate::render_context::RenderContext;
use crate::widgets::mode_bar::ViewMode;
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

    // ── View mode ──
    pub(crate) view_mode: ViewMode,

    // ── Right panel ──
    pub(crate) right_panel_width: f32,
    pub(crate) right_tab: Option<crate::right_panel::RightTab>,
    pub(crate) show_pianoroll_in_arrange: bool,

    // ── Visibility toggles (derived from view_mode) ──
    pub(crate) show_transport: bool,
    pub(crate) show_pianoroll: bool,

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

    // ── Settings ──
    pub(crate) audio_settings: crate::dialogs::settings::AudioSettings,

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
            arr_split: crate::widgets::theme::DEFAULT_ARR_SPLIT,

            controller_renderers: Vec::new(),

            documents: vec![Document::empty()],
            active_doc: Some(0),
            prev_active_doc: Some(0),

            transport_panel_width: 200.0,
            file_loader: FileLoader::new(),

            view_mode: ViewMode::Arrange,
            show_pianoroll_in_arrange: true,
            show_transport: true,
            show_pianoroll: true,

            right_panel_width: crate::widgets::theme::RIGHT_PANEL_DEFAULT_WIDTH,
            right_tab: None,

            title_bar_press_pos: None,

            last_cursor_tick: None,
            piano_last_cursor_tick: None,

            follow_mode: crate::view_interaction::FollowMode::Page,

            audio: None,
            audio_active_doc: None,

            audio_settings: crate::dialogs::settings::AudioSettings::load(),

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
