use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::file_loader::FileLoader;
use crate::view_interaction::{FollowMode, FollowModeExt};
use crate::widgets::tools_panel::{ALL_TOOLS, Tool};
use yinhe_editor_core::document::Document;
use yinhe_editor_core::shortcuts;
use yinhe_types::time_format;

/// Actions triggered from the file menu dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    NewProject,
    Open,
    Save,
    SaveAs,
    CloseDocument,
    ExportAudio,
    ExportMidi,
    Settings,
    Exit,
}

impl FileAction {
    /// 全部文件动作。**顺序即 `AudioSettings::pinned_file_actions` 数组索引**。
    pub const ALL: [FileAction; 9] = [
        FileAction::NewProject,
        FileAction::Open,
        FileAction::Save,
        FileAction::SaveAs,
        FileAction::CloseDocument,
        FileAction::ExportAudio,
        FileAction::ExportMidi,
        FileAction::Settings,
        FileAction::Exit,
    ];

    pub const fn pinned_index(self) -> usize {
        match self {
            FileAction::NewProject => 0,
            FileAction::Open => 1,
            FileAction::Save => 2,
            FileAction::SaveAs => 3,
            FileAction::CloseDocument => 4,
            FileAction::ExportAudio => 5,
            FileAction::ExportMidi => 6,
            FileAction::Settings => 7,
            FileAction::Exit => 8,
        }
    }

    /// 快捷键表（`Keybindings`）中的动作 id。
    pub const fn action_id(self) -> &'static str {
        match self {
            FileAction::NewProject => shortcuts::ACTION_NEW_PROJECT,
            FileAction::Open => shortcuts::ACTION_OPEN,
            FileAction::Save => shortcuts::ACTION_SAVE,
            FileAction::SaveAs => shortcuts::ACTION_SAVE_AS,
            FileAction::CloseDocument => shortcuts::ACTION_CLOSE_DOCUMENT,
            FileAction::ExportAudio => shortcuts::ACTION_EXPORT_AUDIO,
            FileAction::ExportMidi => shortcuts::ACTION_EXPORT_MIDI,
            FileAction::Settings => shortcuts::ACTION_SETTINGS,
            FileAction::Exit => shortcuts::ACTION_EXIT,
        }
    }

    pub const fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            FileAction::NewProject => ICON_NOTE_ADD,
            FileAction::Open => ICON_FOLDER_OPEN,
            FileAction::Save => ICON_SAVE,
            FileAction::SaveAs => ICON_SAVE_ALT,
            FileAction::CloseDocument => ICON_CLOSE,
            FileAction::ExportAudio => ICON_AUDIO_FILE,
            FileAction::ExportMidi => ICON_MUSIC_NOTE,
            FileAction::Settings => ICON_SETTINGS,
            FileAction::Exit => ICON_EXIT_TO_APP,
        }
    }

    /// 动作名的 i18n key（由 `crate::shortcuts::action_label_key` 统一维护）。
    pub fn label_key(self) -> &'static str {
        crate::shortcuts::action_label_key(self.action_id())
    }

    /// 该动作是否可用（菜单中置灰）。
    fn is_enabled(self, has_active: bool, loading: bool) -> bool {
        match self {
            FileAction::NewProject | FileAction::Open => !loading,
            FileAction::Save
            | FileAction::SaveAs
            | FileAction::CloseDocument
            | FileAction::ExportAudio
            | FileAction::ExportMidi => has_active,
            FileAction::Settings | FileAction::Exit => true,
        }
    }
}

/// Aggregated input for the transport bar — replaces 12 positional parameters.
pub struct TransportContext<'a> {
    pub file_loader: &'a mut FileLoader,
    pub doc: Option<&'a Document>,
    pub follow_mode: &'a mut FollowMode,
    pub active_tool: &'a mut Tool,
    /// 状态栏讲解行：控件 hover 时写入提示，空白处清空；鼠标不在传输栏时不动。
    pub status_hint: &'a mut Option<String>,
    /// 应用设置（快捷键表 + 图钉状态，图钉变化时在此 save）。
    pub settings: &'a mut AudioSettings,
}

/// Output from the transport bar — replaces `&mut bool` out-parameters.
pub struct TransportResponse {
    pub toggle_play: bool,
    pub pause_return: bool,
    pub stop_play: bool,
    pub pending_file_action: Option<FileAction>,
}

pub fn show(ui: &mut egui::Ui, ctx: &mut TransportContext<'_>) -> TransportResponse {
    let has_active = ctx.doc.is_some();

    let mut toggle_play = false;
    let mut pause_return = false;
    let mut stop_play = false;
    let mut pending_file_action = None;

    egui::Panel::top("transport_bar")
        .frame(egui::Frame {
            fill: crate::theme::app_bg(),
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 0,
                bottom: 8,
            },
            stroke: egui::Stroke::NONE,
            ..Default::default()
        })
        .show(ui, |ui| {
            // Taller buttons for the transport bar
            ui.spacing_mut().interact_size.y = 32.0;

            let mut timecode_rect: Option<egui::Rect> = None;
            let mut button_right: Option<f32> = None;

            // 本帧控件 hover 提示（状态栏讲解行）
            let mut hovered_hint: Option<String> = None;

            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let btn_size = egui::vec2(
                    crate::theme::TRANSPORT_BTN_SIZE,
                    crate::theme::TRANSPORT_BTN_SIZE,
                );
                let btn_rounding = egui::CornerRadius::same(2);

                let file_btn = ui.add(
                    egui::Button::new(
                        ICON_DESCRIPTION
                            .rich_text()
                            .size(crate::theme::TRANSPORT_BTN_FONT)
                            .color(crate::theme::text_primary()),
                    )
                    .min_size(btn_size)
                    .corner_radius(btn_rounding),
                );
                if file_btn.hovered() {
                    let m = crate::chrome::mode_bar::mod_key();
                    hovered_hint = Some(format!("{} ({}N/{}O/{}S)", t!("hint.file_menu"), m, m, m));
                }
                show_file_menu(
                    &file_btn,
                    ctx.file_loader,
                    has_active,
                    ctx.settings,
                    &mut pending_file_action,
                );

                // ── 图钉固定的文件动作（顺序 = 菜单顺序）：作为独立按钮
                //    紧跟在文件按钮右侧，全部钉上时就是一整行图标 ──
                for (i, action) in FileAction::ALL.iter().enumerate() {
                    if !ctx.settings.pinned_file_actions[i] {
                        continue;
                    }
                    let enabled = action.is_enabled(has_active, ctx.file_loader.is_loading());
                    let icon = action.icon();
                    let pin_resp = ui.add_enabled(
                        enabled,
                        egui::Button::new(
                            icon.rich_text()
                                .size(crate::theme::TRANSPORT_BTN_FONT)
                                .color(if enabled {
                                    crate::theme::text_primary()
                                } else {
                                    crate::theme::text_disabled()
                                }),
                        )
                        .min_size(btn_size)
                        .corner_radius(btn_rounding),
                    );
                    if pin_resp.clicked() {
                        pending_file_action = Some(*action);
                    }
                    if pin_resp.hovered() {
                        hovered_hint = Some(t!(action.label_key()).to_string());
                    }
                }

                if has_active {
                    let is_playing = ctx
                        .doc
                        .map(|d| d.edit.playback.is_playing())
                        .unwrap_or(false);

                    let play_resp = ui.add(
                        egui::Button::new(
                            (if is_playing {
                                ICON_PAUSE
                            } else {
                                ICON_PLAY_ARROW
                            })
                            .rich_text()
                            .size(crate::theme::TRANSPORT_BTN_FONT)
                            .color(crate::theme::text_primary()),
                        )
                        .min_size(btn_size)
                        .corner_radius(btn_rounding),
                    );
                    if play_resp.clicked() {
                        if is_playing {
                            pause_return = true;
                        } else {
                            toggle_play = true;
                        }
                    }
                    if play_resp.hovered() {
                        hovered_hint = Some(t!("hint.play").to_string());
                    }

                    let stop_resp = ui.add(
                        egui::Button::new(
                            ICON_STOP
                                .rich_text()
                                .size(crate::theme::TRANSPORT_BTN_FONT)
                                .color(crate::theme::text_primary()),
                        )
                        .min_size(btn_size)
                        .corner_radius(btn_rounding),
                    );
                    if stop_resp.clicked() {
                        stop_play = true;
                    }
                    if stop_resp.hovered() {
                        hovered_hint = Some(t!("hint.stop").to_string());
                    }

                    // ── Follow-mode button (cycle: None → Page → Continuous) ──
                    let follow_resp = ui.add(
                        egui::Button::new(
                            ctx.follow_mode
                                .icon()
                                .rich_text()
                                .size(crate::theme::TRANSPORT_BTN_FONT)
                                .color(crate::theme::text_primary()),
                        )
                        .min_size(btn_size)
                        .corner_radius(btn_rounding),
                    );
                    if follow_resp.clicked() {
                        *ctx.follow_mode = ctx.follow_mode.next();
                    }
                    if follow_resp.hovered() {
                        hovered_hint = Some(t!("hint.follow").to_string());
                    }
                }

                if let Some(doc) = ctx.doc {
                    timecode_rect = Some(show_timecode_display(ui, doc));

                    // ── 工具按钮：黑色矩形右侧，水平排列 ──
                    // 无按钮外框（透明背景），单一绘制：hover 变色不叠层。
                    // 字号与 transport 其他按钮一致（TRANSPORT_BTN_FONT）。
                    ui.add_space(4.0);
                    for tool in ALL_TOOLS {
                        let is_active = *ctx.active_tool == tool;
                        let icon = tool.icon();
                        let resp = crate::widgets::hover::hover_button(
                            ui,
                            icon.codepoint,
                            egui::FontId::new(crate::theme::TRANSPORT_BTN_FONT, icon.font_family()),
                            crate::theme::text_label(),
                            is_active,
                        );
                        if resp.clicked() {
                            *ctx.active_tool = tool;
                        }
                        if resp.hovered() {
                            hovered_hint = Some(tool_hint(tool));
                        }
                        ui.add_space(2.0);
                    }
                    // 把左侧按钮 + 右侧工具按钮都纳入"按钮区"，
                    // 避免双击/拖拽误触发窗口最大化或拖动。
                    button_right = Some(ui.cursor().min.x);
                }
            });

            // ── 状态栏讲解行：控件 hover 提示写入，传输栏空白处清空 ──
            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
            if pointer_pos.is_some_and(|p| timecode_rect.is_some_and(|r| r.contains(p))) {
                hovered_hint = Some(t!("hint.timecode").to_string());
            }
            let bar_rect = ui.max_rect();
            if let Some(hint) = hovered_hint {
                *ctx.status_hint = Some(hint);
            } else if pointer_pos.is_some_and(|p| bar_rect.contains(p)) {
                *ctx.status_hint = None;
            }

            // ── Double-click transport bar blank area to toggle maximize/restore ──
            // Manual click-timestamp tracking avoids egui's button_double_clicked()
            // misfiring on the first click when the window regains focus.
            // Only triggers on the background gaps (between buttons and timecode,
            // and after timecode to the right edge), NOT on buttons or timecode.
            const DOUBLE_CLICK_MS: f64 = 400.0;
            let dbl_id = ui.id().with("transport_bar_dbl_click");
            if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            {
                let bar_rect = ui.max_rect();
                let in_bar = bar_rect.contains(pos);
                let in_timecode = timecode_rect
                    .map(|r: egui::Rect| r.contains(pos))
                    .unwrap_or(false);
                let in_buttons = button_right
                    .map(|r: f32| pos.x >= bar_rect.min.x && pos.x < r)
                    .unwrap_or(false);
                if in_bar && !in_timecode && !in_buttons {
                    let now = ui.input(|i| i.time);
                    let last_click: f64 = ui.data_mut(|d| d.get_persisted(dbl_id)).unwrap_or(0.0);
                    if now - last_click < DOUBLE_CLICK_MS / 1000.0 {
                        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        ui.data_mut(|d| d.insert_persisted(dbl_id, 0.0)); // reset
                    } else {
                        ui.data_mut(|d| d.insert_persisted(dbl_id, now));
                    }
                }
            }

            // ── Drag transport bar blank area to move the window ──
            // Uses manual pointer tracking (no ui.interact) to avoid consuming
            // button clicks. Only starts a drag if the press began in a blank area
            // (outside buttons and timecode display).
            let bar_rect = ui.max_rect();
            let drag_id = ui.id().with("tb_drag_started");
            let mut drag_started: bool = ui.data_mut(|d| d.get_temp(drag_id)).unwrap_or(false);

            if ui.input(|i| i.pointer.primary_down()) {
                if !drag_started && let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                    let in_bar = bar_rect.contains(pos);
                    let in_timecode = timecode_rect
                        .map(|r: egui::Rect| r.contains(pos))
                        .unwrap_or(false);
                    let in_buttons = button_right
                        .map(|r: f32| pos.x >= bar_rect.min.x && pos.x < r)
                        .unwrap_or(false);
                    if in_bar && !in_timecode && !in_buttons {
                        drag_started = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
            } else {
                drag_started = false;
            }

            ui.data_mut(|d| d.insert_temp(drag_id, drag_started));
        });

    TransportResponse {
        toggle_play,
        pause_return,
        stop_play,
        pending_file_action,
    }
}

/// 状态栏讲解行：工具的短说明（与 tool.label() 的悬停 tooltip 互补）。
fn tool_hint(tool: Tool) -> String {
    match tool {
        Tool::Select => t!("hint.tool.select").to_string(),
        Tool::SelectVertical => t!("hint.tool.select_vertical").to_string(),
        Tool::Pan => t!("hint.tool.pan").to_string(),
        Tool::Pencil => t!("hint.tool.pencil").to_string(),
        Tool::Curve => t!("hint.tool.curve").to_string(),
        Tool::Scissors => t!("hint.tool.scissors").to_string(),
        Tool::Eraser => t!("hint.tool.eraser").to_string(),
    }
}

/// Show the file menu popup. Extracted from `show` for readability.
/// 容器与量化弹框同款：Popup::from_toggle_button_response + CloseOnClickOutside。
/// 固定宽度（快捷键 + 图钉需要稳定的行宽）；每项右侧显示快捷键与图钉按钮。
fn show_file_menu(
    button: &egui::Response,
    file_loader: &FileLoader,
    has_active: bool,
    settings: &mut AudioSettings,
    pending_action: &mut Option<FileAction>,
) {
    egui::Popup::from_toggle_button_response(button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(crate::theme::FILE_MENU_WIDTH)
        .show(|ui| {
            fn menu_items(
                ui: &mut egui::Ui,
                actions: &[FileAction],
                has_active: bool,
                loading: bool,
                settings: &mut AudioSettings,
                pending_action: &mut Option<FileAction>,
            ) {
                // 每行绝对定位（ui.put）固定尺寸：主按钮 + 右侧图钉，
                // 行宽恰好等于菜单内容宽，不参与 popup 宽度反馈；
                // 无快捷键的项用空 shortcut_text 保持左对齐（grow 占中间）。
                const PIN_W: f32 = 26.0;
                const MAIN_PIN_GAP: f32 = 2.0;
                let row_h = ui.spacing().interact_size.y;
                for &action in actions {
                    let enabled = action.is_enabled(has_active, loading);
                    let pinned = settings.pinned_file_actions[action.pinned_index()];
                    let icon_color = if enabled {
                        crate::theme::text_bright()
                    } else {
                        crate::theme::text_disabled()
                    };
                    let shortcut = settings
                        .keybindings
                        .get(action.action_id())
                        .map(|c| crate::shortcuts::display_combo(&c));

                    let row_w = ui.available_width();
                    let (row_rect, _) =
                        ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::hover());
                    let main_rect = egui::Rect::from_min_size(
                        row_rect.min,
                        egui::vec2(row_w - PIN_W - MAIN_PIN_GAP, row_h),
                    );
                    let pin_rect = egui::Rect::from_min_size(
                        egui::pos2(row_rect.max.x - PIN_W, row_rect.min.y),
                        egui::vec2(PIN_W, row_h),
                    );

                    let main_btn = egui::Button::selectable(
                        false,
                        crate::widgets::icon_text::icon_text(
                            action.icon(),
                            &t!(action.label_key()),
                            crate::theme::FILE_MENU_FONT,
                            icon_color,
                        ),
                    )
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .shortcut_text(shortcut.as_deref().unwrap_or(""));
                    let main_resp = ui
                        .add_enabled_ui(enabled, |ui| ui.put(main_rect, main_btn))
                        .inner;

                    let pin_color = if pinned {
                        crate::theme::accent_active()
                    } else {
                        crate::theme::text_disabled()
                    };
                    let pin_btn = egui::Button::new(
                        ICON_KEEP
                            .rich_text()
                            .size(crate::theme::FILE_MENU_FONT)
                            .color(pin_color),
                    )
                    .frame(false);
                    let pin_resp = ui.put(pin_rect, pin_btn);
                    // 无边框按钮 hover 时补背景反馈，提示可点击
                    if pin_resp.hovered() {
                        ui.painter().rect_filled(
                            pin_rect,
                            4.0,
                            crate::theme::hover_color(crate::theme::app_bg()),
                        );
                    }

                    if main_resp.clicked() {
                        *pending_action = Some(action);
                        ui.close();
                    }
                    if pin_resp.clicked() {
                        // 图钉只切换固定状态，不关闭菜单
                        let idx = action.pinned_index();
                        settings.pinned_file_actions[idx] = !settings.pinned_file_actions[idx];
                        settings.save();
                    }
                }
            }

            // 分组：新建/打开 | 保存/另存/关闭 | 导出音频/导出MIDI | 设置/退出
            let groups: [&[FileAction]; 4] = [
                &[FileAction::NewProject, FileAction::Open],
                &[
                    FileAction::Save,
                    FileAction::SaveAs,
                    FileAction::CloseDocument,
                ],
                &[FileAction::ExportAudio, FileAction::ExportMidi],
                &[FileAction::Settings, FileAction::Exit],
            ];
            let loading = file_loader.is_loading();
            for (i, group) in groups.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                menu_items(ui, group, has_active, loading, settings, pending_action);
            }
        });
}

/// Show the timecode display panel. Returns the allocated rect.
fn show_timecode_display(ui: &mut egui::Ui, doc: &Document) -> egui::Rect {
    let tick = doc.edit.cursor_tick.unwrap_or(0.0);
    let model = &doc.data.model;
    let seconds = model.tempo_map.tick_to_seconds(tick as u64);
    let bpm = model.tempo_map.bpm_at_time(seconds);
    let (num, _denom_power) = model.tempo_map.time_sig_at_tick(tick as u32);
    let ppq = model.meta.ppq;

    let bpm_str = time_format::format_bpm(bpm);
    let ts_str = format!(
        "{}  {}",
        time_format::format_time_sig(num, _denom_power),
        ppq
    );
    let time_str = time_format::format_time(seconds);
    let pos_str = time_format::format_tick_bar_beat_with_time_sig(
        tick,
        ppq,
        &model.tempo_map.time_sig_events,
        model.tempo_map.time_sig_default.0,
        model.tempo_map.time_sig_default.1,
    );

    let col_widths = [76.0, 90.0];
    let rect_h = 36.0;
    let rect_w = col_widths.iter().sum::<f32>();
    let bar_cx = ui.max_rect().center().x;
    let cursor_x = ui.cursor().min.x;
    let rect_l = bar_cx - rect_w * 0.5;
    let pad = (rect_l - cursor_x).max(0.0);
    ui.add_space(pad);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(rect_w, rect_h), egui::Sense::hover());

    let c = crate::theme::accent_active();
    let font = egui::FontId::proportional(crate::theme::TIMECODE_FONT);
    let grid = egui::Stroke::new(1.0, crate::theme::line_fg());

    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), crate::theme::track_bg());

    let texts_top = [bpm_str, pos_str];
    let texts_bot = [ts_str, time_str];

    let mut col_x = rect.min.x;
    for i in 0..2 {
        let cx = col_x + col_widths[i] * 0.5;
        if i > 0 {
            ui.painter().line_segment(
                [egui::pos2(col_x, rect.min.y), egui::pos2(col_x, rect.max.y)],
                grid,
            );
        }
        let top_pos = egui::pos2(cx, rect.min.y + rect_h * 0.25);
        let bot_pos = egui::pos2(cx, rect.min.y + rect_h * 0.75);
        ui.painter().text(
            top_pos,
            egui::Align2::CENTER_CENTER,
            &texts_top[i],
            font.clone(),
            c,
        );
        ui.painter().text(
            bot_pos,
            egui::Align2::CENTER_CENTER,
            &texts_bot[i],
            font.clone(),
            c,
        );
        col_x += col_widths[i];
    }

    rect
}
