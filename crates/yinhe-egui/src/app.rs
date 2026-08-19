use std::sync::mpsc;

pub(crate) mod actions;
pub(crate) mod audio;
pub(crate) mod audio_state;
pub(crate) mod automation_actions;
pub(crate) mod dialog_dispatch;
pub(crate) mod export_state;
pub(crate) mod layout;
pub(crate) mod main_loop;
pub(crate) mod midi_input;
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
    /// 打开「最近修改的文件」子菜单选中的路径。
    OpenRecent(String),
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
    /// 上次上传时 hidden_notes 的 key 位图（hidden 增量重建的受影响 key 判定）。
    pub(crate) last_hidden_keys: u128,
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

    // ── MIX 混音台 ──
    /// MIX 界面 UI 状态（扫描结果/选择器/电平衰减，全局一份）。
    pub(crate) mix: crate::mix::MixUiState,
    /// 每文档的 CLAP insert 插件机架，与 `documents` 平行（同索引同生命周期）。
    pub(crate) mixer_racks: Vec<crate::mix::MixerRack>,

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
    /// macOS：播放中是否已阻止 App Nap（状态变化时才调用平台 API）。
    pub(crate) app_nap_active: bool,

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

    // ── MIDI 输入 ──
    /// 当前打开的 MIDI 输入流（设置里选中设备且直通开启时存活）。
    pub(crate) midi_input: Option<yinhe_midi_io::MidiInputStream>,
    /// 当前连接的设备名（检测设备切换/重连）。
    pub(crate) midi_connected_device: Option<String>,
    /// 直通模式按住的键 → 力度（NoteOff 后重发仍按住的键用）。
    pub(crate) midi_thru_keys: std::collections::HashMap<u8, u8>,
    /// MIDI 录音状态（None = 未录音）。
    pub(crate) recording: Option<crate::app::midi_input::RecordingState>,
    /// 步进输入模式：每按一键在光标处写入一个默认长度音符并前进一步。
    pub(crate) step_input: bool,
    /// 布局拖拽结束帧置位，帧末统一写盘（拖拽中不写，避免每帧刷盘）。
    pub(crate) layout_needs_save: bool,
    /// Tracks the last applied MIDI encoding to detect changes.
    pub(crate) last_midi_encoding: yinhe_midi::MidiImportEncoding,
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

            // 其余语言字体（MiSans Global 系列，按回退优先级排在 Pretendard/MiSans 之后）：
            // 繁体中文、泰文、天城文（印地语）、高棉文、老挝文、缅文、藏文。
            const EXTRA_FONTS: [(&str, &[u8]); 7] = [
                ("MiSans-TC", include_bytes!("../../../assets/MiSans-TC.otf")),
                (
                    "MiSans-Thai",
                    include_bytes!("../../../assets/MiSans-Thai.otf"),
                ),
                (
                    "MiSans-Devanagari",
                    include_bytes!("../../../assets/MiSans-Devanagari.otf"),
                ),
                (
                    "MiSans-Khmer",
                    include_bytes!("../../../assets/MiSans-Khmer.otf"),
                ),
                (
                    "MiSans-Lao",
                    include_bytes!("../../../assets/MiSans-Lao.otf"),
                ),
                (
                    "MiSans-Myanmar",
                    include_bytes!("../../../assets/MiSans-Myanmar.ttf"),
                ),
                (
                    "MiSans-Tibetan",
                    include_bytes!("../../../assets/MiSans-Tibetan.ttf"),
                ),
            ];
            for (name, data) in EXTRA_FONTS {
                fonts
                    .font_data
                    .insert(name.to_owned(), egui::FontData::from_static(data).into());
            }
            // 插到 Pretendard、MiSans 之后，egui 默认字体之前。
            // 循环内重新取 entry，避免与上面的 props/mono 借用冲突。
            for (i, (name, _)) in EXTRA_FONTS.iter().enumerate() {
                let pos = 2 + i;
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(pos, name.to_string());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(pos, name.to_string());
            }
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
        // 主题初始化（读取设置的标准色）
        crate::theme::set_theme(audio_settings.theme_base);
        // UI 缩放（DPI 选项）
        cc.egui_ctx.set_zoom_factor(audio_settings.ui_scale);
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
            last_hidden_keys: 0,
            cull_rebuild: None,

            arr_render_ctx,
            arr_renderer: yinhe_wgpu::InstanceRenderer::new(device, queue, format),
            arrange_view: ArrangementView::default(),
            arr_split: audio_settings.layout.arr_split,

            controller_renderers: Vec::new(),

            documents: vec![Document::empty()],
            active_doc: Some(0),
            prev_active_doc: Some(0),
            mix: crate::mix::MixUiState::default(),
            mixer_racks: vec![crate::mix::MixerRack::default()],

            transport_panel_width: audio_settings.layout.transport_panel_width,
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
            show_pianoroll_in_arrange: audio_settings.layout.show_pianoroll_in_arrange,
            track_selection_anchor: None,

            right_panel_width: audio_settings.layout.right_panel_width,
            right_tab: None,
            info_content: None,
            automation_drag_ghost: None,

            active_tool: crate::widgets::tools_panel::Tool::Select,

            title_bar_press_pos: None,
            tab_scroll_offset: 0.0,
            app_nap_active: false,

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
            midi_input: None,
            midi_connected_device: None,
            midi_thru_keys: std::collections::HashMap::new(),
            recording: None,
            step_input: false,
            layout_needs_save: false,
            last_midi_encoding: yinhe_midi::MidiImportEncoding::Utf8,
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

        // 命令行传入的文件路径（Windows 文件关联双击、各平台终端启动）。
        // 取第一个真实存在的文件；macOS 由 Finder 启动时系统注入的 -psn_xxx
        // 等参数会被 is_file 自然过滤。file_loader 同时只支持一个待加载项，
        // 所以只取第一个。
        let cli_file = std::env::args_os()
            .skip(1)
            .find(|a| std::path::Path::new(a).is_file());
        if let Some(path) = cli_file
            && let Some(s) = path.to_str()
        {
            app.file_loader
                .load_path(s.to_string(), app.audio_settings.midi_import_encoding);
        }

        app
    }

    // ── macOS: reserve_render_targets_for_window_anim has been removed ──

    pub(crate) fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        let was_active = self.active_doc == Some(index);
        // 音频绑定本文档时先 teardown：渲染线程退回的 insert 处理器需要
        // 交还本文档机架 deactivate（teardown_audio 内部完成回收），
        // 之后才移除机架与文档。
        if self.audio_state.active_doc == Some(index) {
            self.teardown_audio();
        }
        if index < self.mixer_racks.len() {
            self.mixer_racks.remove(index);
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

        // 同步 audio_state：音频绑定的文档已在上方 teardown，这里只修正索引
        if let Some(audio_idx) = self.audio_state.active_doc
            && audio_idx > index
        {
            self.audio_state.active_doc = Some(audio_idx - 1);
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
        self.last_hidden_keys = 0;
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
        self.mixer_racks.push(crate::mix::MixerRack::default());
        self.active_doc = Some(self.documents.len() - 1);
        self.teardown_audio();
    }
}
