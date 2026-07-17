use std::sync::{Arc, Mutex, mpsc};

pub(crate) mod actions;
pub(crate) mod audio;
pub(crate) mod audio_state;
pub(crate) mod dialog_dispatch;
pub(crate) mod export_state;
pub(crate) mod layout;
pub(crate) mod main_loop;
pub(crate) mod poll;

use crate::file_loader::FileLoader;
use crate::dialogs::system_monitor::SystemMonitor;
use yinhe_editor_core::document::Document;
use crate::render_context::RenderContext;
use crate::chrome::mode_bar::ViewMode;
use yinhe_types::{ArrangementView, PianoRollView};

/// A file action that was deferred because the current document has unsaved changes.
#[derive(Clone, Debug)]
pub(crate) enum PendingFileAction {
    NewProject,
    Open,
    CloseDocument(usize),
    Exit,
}

pub struct App {
    // ── Pianoroll (shared GPU resources + global view state) ──
    pub(crate) render_ctx: RenderContext,
    pub(crate) pianoroll: yinhe_wgpu::InstanceRenderer,
    pub(crate) render_thread: Option<yinhe_wgpu::RenderThreadHandle>,
    pub(crate) pianoroll_view: PianoRollView,
    pub(crate) last_cull_revision: u64, // revision ^ hidden_hash

    // ── Arrangement (shared GPU resources + global view state) ──
    pub(crate) arr_render_ctx: RenderContext,
    pub(crate) arr_renderer: yinhe_wgpu::InstanceRenderer,
    pub(crate) arrange_view: ArrangementView,
    pub(crate) arr_split: f32,

    // ── Automation panel GPU resources (per-document, per-panel) ──
    pub(crate) controller_renderers: Vec<Vec<(yinhe_wgpu::InstanceRenderer, RenderContext)>>,

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

    // ── Unsaved changes confirmation ──
    /// A file action deferred until the user chooses save/discard/cancel.
    pub(crate) pending_unsaved: Option<PendingFileAction>,
    /// Set to true when the user chose to exit without saving.
    pub(crate) should_exit: bool,

    // ── View mode ──
    pub(crate) view_mode: ViewMode,

    // ── Right panel ──
    pub(crate) right_panel_width: f32,
    pub(crate) right_tab: Option<crate::right_panel::RightTab>,
    pub(crate) info_content: Option<crate::right_panel::InfoContent>,
    /// 拖拽锚点时的 ghost 值（tick, value），供信息面板实时显示
    pub(crate) automation_drag_ghost: Option<(u32, u16)>,

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
    /// Horizontal scroll offset for title bar tabs (pixels).
    pub(crate) tab_scroll_offset: f32,

    // ── Cursor tick tracking for cross-view sync ──
    pub(crate) last_cursor_tick: Option<f64>,
    pub(crate) piano_last_cursor_tick: Option<f64>,

    // ── Arrangement selection rect persistence ──
    pub(crate) arr_sel_rect: Option<(f64, f64, usize, usize)>,

    // ── Document switch tracking ──
    pub(crate) prev_active_doc: Option<usize>,

    // ── Cursor-follow mode (shared across arrangement & piano roll) ──
    pub(crate) follow_mode: crate::view_interaction::FollowMode,

    // ── Audio engine ──
    pub(crate) audio_state: audio_state::AudioState,

    // ── Settings ──
    pub(crate) audio_settings: crate::audio_settings::AudioSettings,
    /// Tracks the last applied MIDI encoding to detect changes.
    pub(crate) last_midi_encoding: yinhe_mid2::MidiImportEncoding,
    /// Tracks the last applied automation density to detect changes.
    pub(crate) last_automation_density: u32,

    // ── Haptic feedback ──
    pub(crate) haptic_engine: yinhe_haptic::HapticEngine,

    // ── System resource monitoring ──
    pub(crate) sys_monitor: SystemMonitor,
    /// Live FPS (real, EMA-smoothed from egui frame delta).
    pub(crate) fps: f32,

    // ── Memory breakdown popup state ──
    pub(crate) show_mem_breakdown: bool,

    // ── Event browser ──
    pub(crate) event_browser_state: crate::right_panel::event_browser::EventBrowserState,

    // ── Multi-stage loading progress ──
    pub(crate) load_progress: yinhe_editor_core::progress::SharedProgress,

    // ── Async audio export ──
    pub(crate) export: export_state::ExportState,

    // ── macOS platform integrations ──
    pub(crate) menu_bar: crate::platform::MenuBar,
    /// Tracks the last `is_dirty` state to avoid redundant `setDocumentEdited` calls.
    pub(crate) last_dirty_state: bool,

    // ── Clipboard (selection-rect based, not note data) ──
    pub(crate) clipboard: yinhe_core::Selection,
    /// Length of `doc.history.past` at the time of the last cut.
    /// Used by paste to locate the correct undo entry (undo bridge).
    pub(crate) cut_past_len: Option<usize>,
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

        let audio_settings = crate::audio_settings::load_audio_settings();
        let last_automation_density = audio_settings.automation_event_density;

        let mut app = Self {
            render_ctx,
            pianoroll: yinhe_wgpu::InstanceRenderer::new(
                device.clone(),
                queue.clone(),
                format,
            ),
            render_thread: None,
            pianoroll_view: PianoRollView::default(),
            last_cull_revision: 0,

            arr_render_ctx,
            arr_renderer: yinhe_wgpu::InstanceRenderer::new(device, queue, format),
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
            pending_unsaved: None,
            should_exit: false,
            export: export_state::ExportState::new(),

            view_mode: ViewMode::Arrange,
            show_pianoroll_in_arrange: false,
            show_transport: true,
            show_pianoroll: false,
            track_selection_anchor: None,

            right_panel_width: crate::theme::RIGHT_PANEL_DEFAULT_WIDTH,
            right_tab: None,
            info_content: None,
            automation_drag_ghost: None,

            active_tool: crate::widgets::tools_panel::Tool::Select,

            title_bar_press_pos: None,
            tab_scroll_offset: 0.0,

            last_cursor_tick: None,
            piano_last_cursor_tick: None,
            arr_sel_rect: None,

            follow_mode: crate::view_interaction::FollowMode::Page,

            audio_state: audio_state::AudioState::new(),

            audio_settings,
            last_midi_encoding: yinhe_mid2::MidiImportEncoding::Utf8,
            last_automation_density,

            haptic_engine: yinhe_haptic::HapticEngine::new(),

            sys_monitor: SystemMonitor::new(),
            fps: 0.0,

            show_mem_breakdown: false,

            event_browser_state: crate::right_panel::event_browser::EventBrowserState::default(),

            menu_bar: crate::platform::MenuBar::new(),
            last_dirty_state: false,

            clipboard: yinhe_core::Selection::default(),
            cut_past_len: None,
        };

        // Spawn the independent render thread for pianoroll GPU rendering.
        {
            let device = app.render_ctx.device().clone();
            let queue = app.render_ctx.queue().clone();
            let format = app.render_ctx.target_format();
            let view = app.render_ctx.preview_view().clone();
            let handle = yinhe_wgpu::RenderThreadHandle::spawn(
                device, queue, format, view, default_w, default_h,
            );
            app.render_thread = Some(handle);
        }

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

        // 同步 audio_state：关闭音频绑定的文档时释放引擎，否则修正索引
        match self.audio_state.active_doc {
            Some(audio_idx) if audio_idx == index => self.teardown_audio(),
            Some(audio_idx) if audio_idx > index => {
                self.audio_state.active_doc = Some(audio_idx - 1);
            }
            _ => {}
        }

        // 归还 jemalloc arena 中已释放的内存给 OS，防止 RSS 不下降
        yinhe_memtrace::purge_free_pages();
    }
}
