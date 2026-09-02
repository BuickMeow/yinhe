use eframe::egui;
use rust_i18n::t;

use crate::app::{App, PendingFileAction};
use crate::chrome::title_bar;

use crate::chrome::mode_bar;
use crate::chrome::transport_bar;

// ── Panic-safe take guard ──
/// Restores a taken value back into its slot on drop, preventing data loss
/// if a panic occurs between `std::mem::take` and the manual put-back.
pub(super) struct ReplaceGuard<'a, T> {
    slot: &'a mut T,
    value: Option<T>,
}

impl<'a, T> ReplaceGuard<'a, T> {
    pub(super) fn new(slot: &'a mut T) -> Self
    where
        T: Default,
    {
        let value = std::mem::take(slot);
        ReplaceGuard {
            slot,
            value: Some(value),
        }
    }

    pub(super) fn as_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("ReplaceGuard already consumed")
    }

    pub(super) fn as_ref(&self) -> &T {
        self.value.as_ref().expect("ReplaceGuard already consumed")
    }
}

impl<'a, T> Drop for ReplaceGuard<'a, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            *self.slot = value;
        }
    }
}

impl eframe::App for App {
    /// macOS: 把 Ctrl+左键改写为右键（系统惯例，Finder/多数原生应用如此）。
    /// 改写发生在 egui 处理输入之前，因此 `secondary_clicked()` 等会正确触发；
    /// 同时清除 ctrl 修饰符，避免 PR 视图把它误判为"加选/快捷键"。
    #[cfg(target_os = "macos")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        use egui::{Event, PointerButton};
        for event in &mut raw_input.events {
            match event {
                Event::PointerButton {
                    button,
                    pressed: true,
                    modifiers,
                    ..
                } if *button == PointerButton::Primary && modifiers.ctrl => {
                    *button = PointerButton::Secondary;
                    modifiers.ctrl = false;
                    self.ctrl_click_active = true;
                }
                Event::PointerButton {
                    button,
                    pressed: false,
                    modifiers,
                    ..
                } if *button == PointerButton::Primary && self.ctrl_click_active => {
                    // 拖拽途中松开 Ctrl 也能正确结束右键拖拽
                    *button = PointerButton::Secondary;
                    modifiers.ctrl = false;
                    self.ctrl_click_active = false;
                }
                Event::PointerGone => {
                    self.ctrl_click_active = false;
                }
                _ => {}
            }
        }
    }

    /// 背景清除色。eframe 默认清成半透明深灰 rgba(12,12,12,180)，会盖住透明窗口；
    /// 这里统一清成全透明：全视口背景由各控件自行绘制，控件覆盖不到的地方
    /// （正常模式下 app_bg 填充已覆盖，透明模式下露出桌面）都是清除色。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _ui_total_start = if yinhe_memtrace::perf_probe::enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // ── MIDI 输入（直通试听/录音），每帧消费 ──
        self.poll_midi_input();

        // ── Close interception ──
        let close_requested = ui.ctx().input(|i| i.viewport().close_requested());
        if close_requested && !self.should_exit {
            let any_dirty = self.workspace.documents.iter().any(|d| d.is_dirty());
            if any_dirty {
                // Cancel the close and show the unsaved dialog instead
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.pending_unsaved = Some(PendingFileAction::Exit);
                // 让 Dock 栏图标跳动，提示用户注意
                crate::platform::request_user_attention();
                // 把 unsaved 弹窗拉到主窗口前台（取消/切换其他工程后再次触发时
                // egui 不会自动 raise 已存在的 viewport，必须显式 raise 一次）
                crate::chrome::dialog::raise_viewport(
                    ui.ctx(),
                    egui::ViewportId::from_hash_of("unsaved_dialog"),
                );
            }
            // If no dirty documents, let the close proceed normally
        }

        // ── macOS: update document-edited dot in traffic light ──
        let any_dirty = self.workspace.documents.iter().any(|d| d.is_dirty());
        if any_dirty != self.last_dirty_state {
            self.last_dirty_state = any_dirty;
            crate::platform::set_document_edited(frame, any_dirty);
        }
        // macOS：禁用系统标题栏区域的背景拖动（否则拖文档标签会变成系统级拖窗口）
        crate::platform::disable_background_window_drag(frame);

        // ── macOS: poll native menu bar actions ──
        // 设置窗口打开、快捷键录制或输入框聚焦期间暂停原生菜单加速键：
        // - 设置/录制：macOS 的菜单加速键由 AppKit 在系统层面拦截（不经过 egui），
        //   不暂停会导致设置页内按 Cmd+S 等组合直接触发菜单动作、录制器收不到按键；
        // - 输入框聚焦：同样会被 AppKit 拦截（TextEdit 收不到 Cmd+A/C/V 等），
        //   必须清空加速键让按键直达 egui，由输入框自身消费（全选/复制/粘贴文本）。
        let wants_keyboard_input = ui.ctx().egui_wants_keyboard_input();
        let suspend_menu_accels = self.audio_settings.show_settings
            || self.audio_settings.shortcut_recording
            || wants_keyboard_input;
        // 播放菜单触发的播放/暂停/停止（与键盘、transport bar 合并处理）
        let mut menu_toggle_play = false;
        let mut menu_pause_return = false;
        let mut menu_stop = false;
        let mut menu_record = false;
        let mut menu_step = false;
        for action in self.menu_bar.poll(
            &self.audio_settings.keybindings,
            suspend_menu_accels,
            &self.audio_settings.recent_files,
            self.follow_mode,
        ) {
            use crate::platform::MenuAction;
            // 输入框聚焦时编辑类菜单动作让位给文本编辑（与 handle_keyboard_shortcuts
            // 的 egui_wants_keyboard_input 保护一致）。双保险：加速键暂停是上一帧
            // 的焦点状态，菜单动作仍可能在本帧到达。
            if wants_keyboard_input
                && matches!(
                    action,
                    MenuAction::Undo
                        | MenuAction::Redo
                        | MenuAction::Cut
                        | MenuAction::Copy
                        | MenuAction::Paste
                        | MenuAction::SelectAll
                        | MenuAction::Duplicate
                        | MenuAction::Delete
                        | MenuAction::TransposeUp
                        | MenuAction::TransposeDown
                        | MenuAction::DedupWithinTrack
                        | MenuAction::DedupAcrossTracks
                )
            {
                continue;
            }
            let file_action = match action {
                MenuAction::NewProject => transport_bar::FileAction::NewProject,
                MenuAction::Open => transport_bar::FileAction::Open,
                MenuAction::Save => transport_bar::FileAction::Save,
                MenuAction::SaveAs => transport_bar::FileAction::SaveAs,
                MenuAction::CloseDocument => transport_bar::FileAction::CloseDocument,
                MenuAction::ExportAudio => transport_bar::FileAction::ExportAudio,
                MenuAction::ExportMidi => transport_bar::FileAction::ExportMidi,
                MenuAction::ProjectSettings => transport_bar::FileAction::ProjectSettings,
                MenuAction::Undo => {
                    self.handle_edit_action(transport_bar::EditAction::Undo);
                    continue;
                }
                MenuAction::Redo => {
                    self.handle_edit_action(transport_bar::EditAction::Redo);
                    continue;
                }
                MenuAction::Cut => {
                    self.handle_edit_action(transport_bar::EditAction::Cut);
                    continue;
                }
                MenuAction::Copy => {
                    self.handle_edit_action(transport_bar::EditAction::Copy);
                    continue;
                }
                MenuAction::Paste => {
                    self.handle_edit_action(transport_bar::EditAction::Paste);
                    continue;
                }
                MenuAction::SelectAll => {
                    self.handle_edit_action(transport_bar::EditAction::SelectAll);
                    continue;
                }
                MenuAction::Duplicate => {
                    self.handle_edit_action(transport_bar::EditAction::Duplicate);
                    continue;
                }
                MenuAction::Delete => {
                    self.handle_edit_action(transport_bar::EditAction::Delete);
                    continue;
                }
                MenuAction::TransposeUp => {
                    self.handle_edit_action(transport_bar::EditAction::TransposeUp);
                    continue;
                }
                MenuAction::TransposeDown => {
                    self.handle_edit_action(transport_bar::EditAction::TransposeDown);
                    continue;
                }
                MenuAction::DedupWithinTrack => {
                    self.handle_edit_action(transport_bar::EditAction::DedupWithinTrack);
                    continue;
                }
                MenuAction::DedupAcrossTracks => {
                    self.handle_edit_action(transport_bar::EditAction::DedupAcrossTracks);
                    continue;
                }
                MenuAction::TogglePlay => {
                    let is_playing = self
                        .audio_state
                        .handle
                        .as_ref()
                        .map(|a| a.handle.is_playing())
                        .unwrap_or(false);
                    if is_playing {
                        menu_pause_return = true;
                    } else {
                        menu_toggle_play = true;
                    }
                    continue;
                }
                MenuAction::Stop => {
                    menu_stop = true;
                    continue;
                }
                MenuAction::ToggleRecord => {
                    menu_record = true;
                    continue;
                }
                MenuAction::ToggleStepInput => {
                    menu_step = true;
                    continue;
                }
                MenuAction::SetFollowMode(mode) => {
                    self.follow_mode = mode;
                    continue;
                }
                MenuAction::OpenRecent(path) => {
                    self.open_recent_file(&path, ui.ctx());
                    continue;
                }
                MenuAction::Settings => {
                    self.audio_settings.show_settings = true;
                    crate::chrome::dialog::raise_viewport(
                        ui.ctx(),
                        egui::ViewportId::from_hash_of("settings_dialog"),
                    );
                    continue;
                }
                MenuAction::Exit => transport_bar::FileAction::Exit,
                // 系统级动作（About/Hide 等）已在平台层就地处理，不会到达这里
                MenuAction::About
                | MenuAction::Hide
                | MenuAction::HideOthers
                | MenuAction::ShowAll => continue,
            };
            self.handle_file_action(file_action, ui.ctx());
        }

        // ── macOS: Finder/桌面双击或"打开方式"传入的文件（Apple Events）──
        for path in self.menu_bar.poll_open_files() {
            tracing::info!("Opening file from Finder: {}", path);
            self.file_loader
                .load_path(path, self.audio_settings.midi_import_encoding);
        }

        // ── Detect document switch → invalidate GPU caches ──
        if self.workspace.active_doc != self.workspace.prev_active_doc {
            self.arrange_view.base.dirty = true;
            self.pianoroll_view.base.dirty = true;
            // 全局 GPU cull buffer 是跨文档共享的，切换后必须清空 + 重置跟踪键，
            // 否则下一个文档首帧可能因 revision/track_visible 巧合相等而跳过
            // upload，渲染出上一个文档的音符（见 close_document 同根修复）。
            self.invalidate_cull_state();
            self.workspace.prev_active_doc = self.workspace.active_doc;
            // 三视图选框互斥：切换文档后把 prev 计数对齐到新文档当前状态，
            // 避免把"已有选框"误判为"新创建的选框"而清除其他视图。
            // 选框本身都存于 doc.edit，切文档后状态自然随文档走。
            match self.workspace.active_doc {
                Some(i) => {
                    let edit = &self.workspace.documents[i].edit;
                    self.prev_arr_count = edit.arr_sel_rect.len();
                    self.prev_pr_count = edit.sel_rect.rects.len();
                    self.prev_am_count = edit
                        .controller_panels
                        .iter()
                        .map(|p| p.anchor_sel_rects.len())
                        .sum();
                    self.prev_selected_nonempty = !edit.selected.is_empty();
                }
                None => {
                    self.prev_arr_count = 0;
                    self.prev_pr_count = 0;
                    self.prev_am_count = 0;
                    self.prev_selected_nonempty = false;
                }
            }
        }

        // ── Conductor 颜色随主文字切换 + 刷新 AM 曲线 GPU（烘焙色需重建）──
        let cur_conductor = crate::theme::conductor_color_f32();
        if cur_conductor != self.last_conductor_color {
            self.last_conductor_color = cur_conductor;
            for doc in &mut self.workspace.documents {
                doc.sync_track_caches_with_conductor_color(cur_conductor);
                for p in &mut doc.edit.controller_panels {
                    p.dirty = true;
                }
                for v in doc.edit.arr_am_views.values_mut() {
                    v.dirty = true;
                }
            }
        }

        // ── 统一主题色：Visuals 基底跟随主题明/暗（egui 原生控件配色） ──
        ui.ctx().set_visuals({
            let mut visuals = if crate::theme::dark_mode() {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            // 弹窗/面板背景色统一为程序背景色（egui 默认 gray(27) 与主题不符）
            visuals.window_fill = crate::theme::app_bg();
            visuals.panel_fill = crate::theme::app_bg();
            // 选中高亮色统一为 ROW_SELECTED_BG；选中描边与输入光标改用强调色
            visuals.selection.bg_fill = crate::theme::selected_bg();
            visuals.selection.stroke = egui::Stroke::new(1.5, crate::theme::accent_active());
            // 闪烁竖线（光标）与 IME 下划线改用强调色
            let accent = crate::theme::accent_active();
            visuals.text_cursor.stroke = egui::Stroke::new(2.0, accent);
            visuals.text_cursor.preview = false;
            // 输入框/TextEdit 背景呼应主题（egui 默认灰色与主题不搭）
            visuals.extreme_bg_color = crate::theme::control_bg();
            // egui 原生控件（Button/ComboBox/Slider/Checkbox 等）三态统一：
            // inactive = btn_bg，hover/active 用统一增益；描边统一 line_fg
            let btn = crate::theme::btn_bg();
            let line = crate::theme::line_fg();
            visuals.widgets.inactive.bg_fill = btn;
            visuals.widgets.inactive.weak_bg_fill = crate::theme::app_bg();
            visuals.widgets.hovered.bg_fill = crate::theme::hover_color(btn);
            visuals.widgets.hovered.weak_bg_fill =
                crate::theme::hover_color(crate::theme::app_bg());
            visuals.widgets.active.bg_fill = crate::theme::pressed_color(btn);
            visuals.widgets.active.weak_bg_fill =
                crate::theme::pressed_color(crate::theme::app_bg());
            // 原生描边/滑轨线（Slider rail、ComboBox 边框等）统一为 line_fg
            // 输入框聚焦/悬停描边改用强调色，下划线亦随之
            // 对勾（fg_stroke）用主文字色：与 btn_bg 同系的 line_fg 会导致对勾几乎不可见（见 widgets::checkbox）
            visuals.widgets.inactive.fg_stroke =
                egui::Stroke::new(1.5, crate::theme::text_primary());
            visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, line);
            visuals.widgets.hovered.fg_stroke =
                egui::Stroke::new(1.5, crate::theme::text_primary());
            visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent.gamma_multiply(0.85));
            visuals.widgets.active.fg_stroke = egui::Stroke::new(1.5, crate::theme::text_primary());
            visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent);
            visuals.widgets.noninteractive.fg_stroke =
                egui::Stroke::new(1.5, crate::theme::text_disabled());
            // 原生控件文字统一用主题主文字色（egui 默认灰与主题不协调）
            visuals.override_text_color = Some(crate::theme::text_primary());
            // Noninteractive 态（disabled 按钮等）背景也统一为 app_bg：
            // 缺省会回落到 egui 默认灰，与主题背景不一致（亮色主题下差异明显）
            visuals.widgets.noninteractive.weak_bg_fill = crate::theme::app_bg();
            visuals
        });

        // ── Custom title bar ──
        let title_bar_action = title_bar::show(
            ui,
            &self.workspace.documents,
            &mut self.workspace.active_doc,
            &mut self.tab_scroll_offset,
            &mut self.status_hint,
        );
        match title_bar_action {
            Some(title_bar::TitleBarAction::CloseDocument(idx)) => {
                if self
                    .workspace
                    .documents
                    .get(idx)
                    .is_some_and(|d| d.is_dirty())
                {
                    self.pending_unsaved = Some(PendingFileAction::CloseDocument(idx));
                    // 把 unsaved 弹窗拉到主窗口前台（用户点击 tab 关闭按钮是主动
                    // 操作，应该立刻看到弹窗）
                    crate::chrome::dialog::raise_viewport(
                        ui.ctx(),
                        egui::ViewportId::from_hash_of("unsaved_dialog"),
                    );
                } else {
                    self.close_document(idx);
                }
            }
            Some(title_bar::TitleBarAction::ReorderTab { from, insert_at }) => {
                self.reorder_tab(from, insert_at);
            }
            Some(title_bar::TitleBarAction::DetachTab(idx)) => {
                // 脏文档也直接 detach：临时保存包含未保存内容，不丢数据，不弹确认
                self.detach_tab_to_new_window(idx);
            }
            None => {}
        }

        // ── Defensive: ensure active_doc is always in bounds ──
        if let Some(idx) = self.workspace.active_doc
            && idx >= self.workspace.documents.len()
        {
            self.workspace.active_doc = if self.workspace.documents.is_empty() {
                None
            } else {
                Some(self.workspace.documents.len() - 1)
            };
        }

        // ── Keyboard shortcuts ──
        let kb = self.handle_keyboard_shortcuts(ui);
        // 路由：Select/SelectVertical 工具且有锚点选中时，copy/paste/duplicate/delete 作用于自动化锚点
        let route_to_automation = self.has_selected_automation_anchors();
        if kb.delete_selected {
            if route_to_automation {
                self.delete_automation_anchors();
            } else {
                self.delete_selected_notes();
            }
        }
        if kb.duplicate_selected {
            if route_to_automation {
                self.duplicate_automation_anchors();
            } else {
                self.duplicate_selected_notes();
            }
        }
        if kb.transpose_up {
            self.transpose_selected_notes(12);
        }
        if kb.transpose_down {
            self.transpose_selected_notes(-12);
        }
        if kb.undo {
            self.undo();
        }
        if kb.redo {
            self.redo();
        }
        if kb.copy {
            if route_to_automation {
                self.copy_automation_anchors();
            } else {
                self.copy_selection();
            }
        }
        if kb.cut {
            self.cut_selection();
        }
        if kb.paste {
            if route_to_automation {
                self.paste_automation_anchors();
            } else {
                self.paste_clipboard();
            }
        }
        if kb.select_all {
            self.select_all();
        }
        if let Some(tool) = kb.tool_to_activate {
            self.active_tool = tool;
        }

        // ── Live FPS (real, EMA-smoothed from egui frame delta) ──
        let dt = ui.input(|i| i.stable_dt);
        let inst_fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        // Reset EMA when frames were paused (e.g. UI was idle), otherwise a
        // stale high value (like 140 fps from quick mouse movement) would
        // take tens of frames to decay back to the real rate (~60 fps).
        let stale = dt > 0.1 || self.fps <= 0.0;
        self.fps = if stale {
            inst_fps
        } else {
            self.fps * 0.9 + inst_fps * 0.1
        };

        // ── System resource monitoring ──
        self.refresh_system_stats();

        // ── Poll async operations ──
        // 先检测是否有 rescale 请求待启动（由 project_info 弹框确认后写入），
        // 再 poll 现有异步操作（包括 rescale 完成检测）。
        self.start_rescale_if_requested(ui.ctx());
        self.poll_async_operations();

        // ── Handle deferred exit ──
        // 不重置 should_exit：保持 true 直到窗口真正关闭，
        // 这样下一帧 close_requested 为 true 时，
        // 守卫 !self.should_exit 为 false，不会重新拦截并弹窗。
        if self.should_exit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // ── Ensure audio engine is loaded for the active document ──
        self.rebuild_audio_if_needed();

        // ── 后台 spawn 结果收取（引擎初始化 stage 完成）──
        self.poll_audio_spawn();

        // ── 音色库异步加载进度（完成计数驱动 stage 2）──
        self.poll_audio_progress();

        // ── 混音台 insert：回收退回处理器 + 插件反向请求（restart 等）──
        self.poll_mixer_plugins();

        // ── Transport bar ──
        let active_doc = self
            .workspace
            .active_doc
            .and_then(|idx| self.workspace.documents.get(idx));
        let transport_response = transport_bar::show(
            ui,
            &mut transport_bar::TransportContext {
                file_loader: &mut self.file_loader,
                doc: active_doc,
                follow_mode: &mut self.follow_mode,
                active_tool: &mut self.active_tool,
                status_hint: &mut self.status_hint,
                settings: &mut self.audio_settings,
                is_recording: self.recording.is_some(),
                step_input: self.step_input,
                orientation: &mut self.pianoroll_view.orientation,
            },
        );

        // mode_bar 的 MEM 数字：memtrace 开启时用分类追踪的堆内存，
        // 关闭时用系统 RSS（sys_monitor）。
        let mem_mb = if yinhe_memtrace::enabled() {
            yinhe_memtrace::Snapshot::capture().total_mb()
        } else {
            self.sys_monitor.mem_mb
        };

        // ── Handle playback actions ──
        self.handle_playback(
            kb.toggle_play || transport_response.toggle_play || menu_toggle_play,
            kb.pause_return || transport_response.pause_return || menu_pause_return,
            kb.stop_play || transport_response.stop_play || menu_stop,
        );

        // ── 步进输入模式切换 ──
        if transport_response.step_toggle || menu_step {
            self.step_input = !self.step_input;
        }

        // ── 钢琴卷帘方向切换（横向 / 纵向瀑布流二选一）──
        if transport_response.toggle_orientation {
            self.pianoroll_view
                .set_orientation(self.pianoroll_view.orientation().toggled());
        }

        // ── MIDI 录音切换（REC 按钮 / macOS 播放菜单）──
        if transport_response.record_toggle || menu_record {
            if self.recording.is_some() {
                self.stop_recording();
            } else {
                self.start_recording();
            }
        }

        // ── Smooth cursor interpolation between audio callback updates ──
        self.interpolate_playback_cursor();

        // ── Handle edit menu actions（传输栏编辑 popup / 图钉）──
        if let Some(action) = transport_response.pending_edit_action {
            self.handle_edit_action(action);
        }

        // ── Handle file menu actions ──
        // 键盘触发的文件动作（非 macOS）与传输栏菜单触发的合并处理
        if let Some(action) = kb.file_action {
            self.handle_file_action(action, ui.ctx());
        }
        if let Some(action) = transport_response.pending_file_action {
            self.handle_file_action(action, ui.ctx());
        }
        // 文件菜单「最近修改的文件」子菜单点击
        if let Some(path) = transport_response.pending_open_path {
            self.open_recent_file(&path, ui.ctx());
        }

        // ── MIDI encoding change ──
        let new_enc = self.audio_settings.midi_import_encoding;
        if new_enc != self.last_midi_encoding {
            self.last_midi_encoding = new_enc;
            if self.workspace.active_doc.is_some() {
                self.with_undo(t!("undo.recode_track_names").as_ref(), |doc| {
                    doc.recode_track_names(new_enc);
                    None::<yinhe_editor_core::history::UndoAction>
                });
            }
        }

        // ── Bottom mode bar ──
        mode_bar::show(
            ui,
            &mut self.view_mode,
            &mut self.show_pianoroll_in_arrange,
            &mut self.right_tab,
            self.sys_monitor.cpu_usage,
            mem_mb,
            self.fps,
            &mut self.show_mem_breakdown,
            &self.status_hint,
        );
        // PR 显示切换 → 布局设置写盘（帧末 sync_layout_settings 统一落盘）
        if self.show_pianoroll_in_arrange != self.audio_settings.layout.show_pianoroll_in_arrange {
            self.layout_needs_save = true;
        }

        // ── Main content area ──
        let layout = self.compute_layout(ui);
        self.show_main_content(ui, &layout);
        self.show_panels_and_overlays(ui, &layout);
        self.show_dialogs(ui);
        self.show_load_error_modal(ui);

        // ── 布局设置持久化（拖拽结束帧才写盘）──
        self.sync_layout_settings();
        self.sync_automation_density();

        if let Some(t0) = _ui_total_start {
            yinhe_memtrace::perf_probe::record_ui_total(t0.elapsed());
        }
    }
}
