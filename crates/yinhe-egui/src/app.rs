use std::sync::mpsc;

pub(crate) mod actions;
pub(crate) mod audio;
pub(crate) mod audio_state;
pub(crate) mod automation_actions;
pub(crate) mod dialog_dispatch;
pub(crate) mod export_state;
pub(crate) mod layout;
pub(crate) mod main_loop;
pub(crate) mod poll;
pub(crate) mod rescale_state;

use crate::chrome::mode_bar::ViewMode;
use crate::dialogs::system_monitor::SystemMonitor;
use crate::file_loader::FileLoader;
use crate::render_context::RenderContext;
use yinhe_editor_core::document::Document;
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
    pub(crate) last_cull_revision_only: u64, // last revision (for incremental detection)
    pub(crate) last_hidden_hash: u64,   // last hidden_hash (for incremental detection)
    pub(crate) last_tv_hash: u64,       // last track_visible hash (track_mask 变化检测)
    /// Track 显隐后台重建状态机（见 gpu_upload::CullRebuild）。
    pub(crate) cull_rebuild: Option<crate::piano_view::gpu_upload::CullRebuild>,

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
    /// 保存进度（阶段 + 阶段内 0.0~1.0），由保存线程经 channel 推送。
    pub(crate) save_progress: Option<(yinhe_yin::YinProgressStage, f32)>,
    pub(crate) save_progress_rx: Option<mpsc::Receiver<yinhe_yin::YinProgress>>,

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
    pub(crate) automation_drag_ghost: Option<(u32, f32)>,

    // ── Tool palette ──
    pub(crate) active_tool: crate::widgets::tools_panel::Tool,
    pub(crate) show_pianoroll_in_arrange: bool,

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

    // ── PR/AR/AM 三视图选框互斥 ──
    // 记录上一帧各视图的选框数量与共享选区状态，用于检测"新增选框/选区被清空"，
    // 从而在创建新选框时清除其他视图的选框（见 layout.rs enforce_sel_rect_exclusivity）。
    // 选框本身都存于 doc.edit（sel_rect / arr_sel_rect / anchor_sel_rects）。
    pub(crate) prev_arr_count: usize,
    pub(crate) prev_pr_count: usize,
    pub(crate) prev_am_count: usize,
    pub(crate) prev_selected_nonempty: bool,

    // ── Document switch tracking ──
    pub(crate) prev_active_doc: Option<usize>,

    // ── Cursor-follow mode (shared across arrangement & piano roll) ──
    pub(crate) follow_mode: crate::view_interaction::FollowMode,

    // ── 状态栏讲解行（teaching bar）──
    /// mode_bar 左下角的讲解文字：各视图/控件每帧写入，mode_bar 展示。
    /// 鼠标悬停在钢琴卷帘/走带/自动化面板上时显示位置信息，
    /// 悬停在控件上时显示该控件的用途与快捷键。
    pub(crate) status_hint: Option<String>,

    // ── Audio engine ──
    pub(crate) audio_state: audio_state::AudioState,

    // ── Settings ──
    pub(crate) audio_settings: crate::audio_settings::AudioSettings,
    /// Tracks the last applied MIDI encoding to detect changes.
    pub(crate) last_midi_encoding: yinhe_mid2::MidiImportEncoding,
    /// Tracks the last applied automation density to detect changes.
    pub(crate) last_automation_density: u32,

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

    // ── Async PPQ rescale ──
    pub(crate) rescale: rescale_state::RescaleState,

    // ── macOS platform integrations ──
    pub(crate) menu_bar: crate::platform::MenuBar,
    /// Tracks the last `is_dirty` state to avoid redundant `setDocumentEdited` calls.
    pub(crate) last_dirty_state: bool,
    /// macOS: 正在进行的 Ctrl+左键（已被改写为右键）拖拽。
    /// 拖拽途中松开 Ctrl 也能正确结束右键拖拽（release 继续改写为 Secondary）。
    #[cfg(target_os = "macos")]
    pub(crate) ctrl_click_active: bool,

    // ── Clipboard (selection-rect based, not note data) ──
    pub(crate) clipboard: yinhe_core::Selection,
    /// Length of `doc.history.past` at the time of the last cut.
    /// Used by paste to locate the correct undo entry (undo bridge).
    pub(crate) cut_past_len: Option<usize>,

    /// 自动化锚点剪贴板（与音符剪贴板独立）。
    /// 存储复制的锚点事件 + 源 target，粘贴时只应用到 target 匹配的面板。
    pub(crate) automation_clipboard: crate::app::automation_actions::AutomationClipboard,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ── Load fonts: Pretendard-SemiBold primary, MiSans fallback ──
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "Pretendard".to_owned(),
                egui::FontData::from_static(include_bytes!(
                    "../../../assets/Pretendard-Medium.otf"
                ))
                .into(),
            );
            fonts.font_data.insert(
                "MiSans".to_owned(),
                egui::FontData::from_static(include_bytes!("../../../assets/MiSans-Medium.otf"))
                    .into(),
            );
            let props = fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            props.insert(0, "Pretendard".to_owned());
            props.insert(1, "MiSans".to_owned());
            let mono = fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default();
            mono.insert(0, "Pretendard".to_owned());
            mono.insert(1, "MiSans".to_owned());
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
        rust_i18n::set_locale(&audio_settings.locale);
        let last_automation_density = audio_settings.automation_event_density;

        let mut app = Self {
            render_ctx,
            pianoroll: yinhe_wgpu::InstanceRenderer::new(device.clone(), queue.clone(), format),
            render_thread: None,
            pianoroll_view: PianoRollView::default(),
            last_cull_revision: 0,
            last_cull_revision_only: 0,
            last_hidden_hash: 0,
            last_tv_hash: 0,
            cull_rebuild: None,

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
            save_progress: None,
            save_progress_rx: None,
            pending_unsaved: None,
            should_exit: false,
            export: export_state::ExportState::new(),
            rescale: rescale_state::RescaleState::new(),

            view_mode: ViewMode::Arrange,
            show_pianoroll_in_arrange: false,
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
            prev_arr_count: 0,
            prev_pr_count: 0,
            prev_am_count: 0,
            prev_selected_nonempty: false,

            follow_mode: crate::view_interaction::FollowMode::Page,
            status_hint: None,

            audio_state: audio_state::AudioState::new(),

            audio_settings,
            last_midi_encoding: yinhe_mid2::MidiImportEncoding::Utf8,
            last_automation_density,

            sys_monitor: SystemMonitor::new(),
            fps: 0.0,

            show_mem_breakdown: false,

            event_browser_state: crate::right_panel::event_browser::EventBrowserState::default(),

            menu_bar: crate::platform::MenuBar::new(),
            last_dirty_state: false,
            #[cfg(target_os = "macos")]
            ctrl_click_active: false,

            clipboard: yinhe_core::Selection::default(),
            cut_past_len: None,
            automation_clipboard: crate::app::automation_actions::AutomationClipboard::default(),
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

        app
    }

    // ── macOS: reserve_render_targets_for_window_anim has been removed ──

    pub(crate) fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        let was_active = self.active_doc == Some(index);
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

        // 关闭的是活跃工程时，全局 GPU cull buffer 还残留旧工程音符数据，
        // 必须清空 + 重置 cull 跟踪键，否则下一个活跃工程首帧会走增量路径
        // 跳过 upload，渲染出旧工程音符（多 tab 切换的同根问题在 main_loop
        // 的 document-switch 检测里统一处理）。
        if was_active {
            self.invalidate_cull_state();
        }
    }

    /// 清空 pianoroll / arr_renderer 的 GPU cull buffer 并重置 cull 跟踪键，
    /// 让下一次渲染走 full-upload 路径。在文档切换 / 关闭 / 替换时调用，
    /// 防止前一个文档的音符数据残留到下一个文档的首帧。
    pub(crate) fn invalidate_cull_state(&mut self) {
        self.pianoroll.clear_cull();
        self.arr_renderer.clear_cull();
        // 丢弃进行中的后台重建（旧文档数据不得上传到新文档；
        // 后台线程 send 失败自动退出）。
        self.cull_rebuild = None;
        self.last_cull_revision = 0;
        self.last_cull_revision_only = 0;
        self.last_hidden_hash = 0;
        self.last_tv_hash = 0;
    }

    /// 判断打开 MIDI/.yin 时是否应替换当前标签页而非另开一个。
    ///
    /// 仅当当前是首次启动的 Untitled（`documents.len() == 1`、活跃在 idx 0、
    /// `file_path.is_none()` 且未修改）时返回 true。用户手动 NewProject 后
    /// `documents.len() > 1`，或已编辑/已保存的 Untitled 都不替换。
    fn should_replace_initial_untitled(&self) -> bool {
        self.active_doc == Some(0)
            && self.documents.len() == 1
            && self.documents[0].file_path.is_none()
            && !self.documents[0].is_dirty()
    }

    /// 新建空白工程（Document::empty）并切换为活跃标签页。
    /// `execute_file_action::NewProject` 与 `execute_pending_file_action::NewProject`
    /// 共用此实现，避免两处重复 push + setup 代码。
    fn new_project(&mut self) {
        self.documents.push(Document::empty());
        self.active_doc = Some(self.documents.len() - 1);
        self.teardown_audio();
    }
}
