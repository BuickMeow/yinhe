use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::audio_settings::AudioSettings;
use crate::file_loader::FileLoader;
use crate::view_interaction::FollowMode;
use crate::widgets::tools_panel::{ALL_TOOLS, Tool};
use yinhe_editor_core::document::Document;
use yinhe_editor_core::shortcuts;
use yinhe_types::time_format;

/// Actions triggered from the edit menu dropdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Duplicate,
    Delete,
    TransposeUp,
    TransposeDown,
}

impl EditAction {
    /// 全部编辑动作。**顺序即 `AudioSettings::pinned_edit_actions` 数组索引**。
    pub const ALL: [EditAction; 10] = [
        EditAction::Undo,
        EditAction::Redo,
        EditAction::Cut,
        EditAction::Copy,
        EditAction::Paste,
        EditAction::SelectAll,
        EditAction::Duplicate,
        EditAction::Delete,
        EditAction::TransposeUp,
        EditAction::TransposeDown,
    ];

    pub const fn pinned_index(self) -> usize {
        match self {
            EditAction::Undo => 0,
            EditAction::Redo => 1,
            EditAction::Cut => 2,
            EditAction::Copy => 3,
            EditAction::Paste => 4,
            EditAction::SelectAll => 5,
            EditAction::Duplicate => 6,
            EditAction::Delete => 7,
            EditAction::TransposeUp => 8,
            EditAction::TransposeDown => 9,
        }
    }

    /// 快捷键表（`Keybindings`）中的动作 id。
    pub const fn action_id(self) -> &'static str {
        match self {
            EditAction::Undo => shortcuts::ACTION_UNDO,
            EditAction::Redo => shortcuts::ACTION_REDO,
            EditAction::Cut => shortcuts::ACTION_CUT,
            EditAction::Copy => shortcuts::ACTION_COPY,
            EditAction::Paste => shortcuts::ACTION_PASTE,
            EditAction::SelectAll => shortcuts::ACTION_SELECT_ALL,
            EditAction::Duplicate => shortcuts::ACTION_DUPLICATE,
            EditAction::Delete => shortcuts::ACTION_DELETE,
            EditAction::TransposeUp => shortcuts::ACTION_TRANSPOSE_UP,
            EditAction::TransposeDown => shortcuts::ACTION_TRANSPOSE_DOWN,
        }
    }

    pub const fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            EditAction::Undo => ICON_UNDO,
            EditAction::Redo => ICON_REDO,
            EditAction::Cut => ICON_CONTENT_CUT,
            EditAction::Copy => ICON_CONTENT_COPY,
            EditAction::Paste => ICON_CONTENT_PASTE,
            EditAction::SelectAll => ICON_SELECT_ALL,
            EditAction::Duplicate => ICON_COPY_ALL,
            EditAction::Delete => ICON_DELETE,
            EditAction::TransposeUp => ICON_ARROW_UPWARD,
            EditAction::TransposeDown => ICON_ARROW_DOWNWARD,
        }
    }

    /// 动作名的 i18n key（由 `crate::shortcuts::action_label_key` 统一维护）。
    pub fn label_key(self) -> &'static str {
        crate::shortcuts::action_label_key(self.action_id())
    }

    /// 该动作是否可用（菜单中置灰；无活动文档时编辑无意义）。
    fn is_enabled(self, has_active: bool) -> bool {
        has_active
    }
}

/// 菜单 popup 行的统一接口：文件/编辑/播放动作共用同一套渲染逻辑
/// （图标 + 名称 + 快捷键 + 可选图钉 + 可选选中态），保证三个 popup 行为一致。
pub trait PopupRow: Copy {
    fn pinned_index(self) -> usize;
    fn action_id(self) -> &'static str;
    fn icon(self) -> egui_material_icons::MaterialIcon;
    fn label_key(self) -> &'static str;
    fn is_enabled(self, has_active: bool, loading: bool) -> bool;

    /// 该行是否渲染右侧图钉按钮（默认 true；播放菜单等无图钉动作返回 false）。
    fn has_pin(self) -> bool {
        true
    }

    /// 该行是否处于选中态（单选菜单的当前项，如播放跟随档）。
    fn is_selected(self) -> bool {
        false
    }
}

impl PopupRow for FileAction {
    fn pinned_index(self) -> usize {
        self.pinned_index()
    }
    fn action_id(self) -> &'static str {
        self.action_id()
    }
    fn icon(self) -> egui_material_icons::MaterialIcon {
        self.icon()
    }
    fn label_key(self) -> &'static str {
        self.label_key()
    }
    fn is_enabled(self, has_active: bool, loading: bool) -> bool {
        self.is_enabled(has_active, loading)
    }
}

impl PopupRow for EditAction {
    fn pinned_index(self) -> usize {
        self.pinned_index()
    }
    fn action_id(self) -> &'static str {
        self.action_id()
    }
    fn icon(self) -> egui_material_icons::MaterialIcon {
        self.icon()
    }
    fn label_key(self) -> &'static str {
        self.label_key()
    }
    fn is_enabled(self, has_active: bool, _loading: bool) -> bool {
        self.is_enabled(has_active)
    }
}

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
    pub pending_edit_action: Option<EditAction>,
}

pub fn show(ui: &mut egui::Ui, ctx: &mut TransportContext<'_>) -> TransportResponse {
    let has_active = ctx.doc.is_some();

    let mut play_actions = PlayActions::default();
    let mut pending_file_action = None;
    let mut pending_edit_action = None;

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
                pinned_action_buttons(
                    ui,
                    &FileAction::ALL,
                    &mut ctx.settings.pinned_file_actions,
                    has_active,
                    ctx.file_loader.is_loading(),
                    &mut hovered_hint,
                    &mut pending_file_action,
                );

                // ── 编辑按钮 + 编辑菜单 popup（与文件按钮同款）──
                let edit_btn = ui.add(
                    egui::Button::new(
                        ICON_EDIT
                            .rich_text()
                            .size(crate::theme::TRANSPORT_BTN_FONT)
                            .color(crate::theme::text_primary()),
                    )
                    .min_size(btn_size)
                    .corner_radius(btn_rounding),
                );
                if edit_btn.hovered() {
                    hovered_hint = Some(t!("hint.edit_menu").to_string());
                }
                show_edit_menu(
                    &edit_btn,
                    has_active,
                    ctx.settings,
                    &mut pending_edit_action,
                );

                // ── 图钉固定的编辑动作 ──
                pinned_action_buttons(
                    ui,
                    &EditAction::ALL,
                    &mut ctx.settings.pinned_edit_actions,
                    has_active,
                    false,
                    &mut hovered_hint,
                    &mut pending_edit_action,
                );

                // ── 播放菜单按钮（播放/停止/跟随并入 popup）──
                let is_playing = ctx
                    .doc
                    .map(|d| d.edit.playback.is_playing())
                    .unwrap_or(false);
                let play_menu_btn = ui.add(
                    egui::Button::new(
                        ICON_PLAY_CIRCLE
                            .rich_text()
                            .size(crate::theme::TRANSPORT_BTN_FONT)
                            .color(crate::theme::text_primary()),
                    )
                    .min_size(btn_size)
                    .corner_radius(btn_rounding),
                );
                if play_menu_btn.hovered() {
                    hovered_hint = Some(t!("hint.play_menu").to_string());
                }
                show_play_menu(
                    &play_menu_btn,
                    has_active,
                    is_playing,
                    ctx.follow_mode,
                    ctx.settings,
                    &mut play_actions,
                );

                // ── 图钉固定的"播放/暂停"：作为独立按钮紧跟在播放菜单按钮右侧 ──
                if ctx.settings.pinned_play_pause {
                    let play_resp = ui.add_enabled(
                        has_active,
                        egui::Button::new(
                            (if is_playing {
                                ICON_PAUSE
                            } else {
                                ICON_PLAY_ARROW
                            })
                            .rich_text()
                            .size(crate::theme::TRANSPORT_BTN_FONT)
                            .color(if has_active {
                                crate::theme::text_primary()
                            } else {
                                crate::theme::text_disabled()
                            }),
                        )
                        .min_size(btn_size)
                        .corner_radius(btn_rounding),
                    );
                    if play_resp.clicked() {
                        if is_playing {
                            play_actions.pause_return = true;
                        } else {
                            play_actions.toggle_play = true;
                        }
                    }
                    if play_resp.hovered() {
                        hovered_hint = Some(t!("hint.play").to_string());
                    }
                }

                // ── 图钉固定的"停止"：紧跟在播放/暂停图钉按钮右侧 ──
                if ctx.settings.pinned_stop {
                    let stop_resp = ui.add_enabled(
                        has_active,
                        egui::Button::new(
                            ICON_STOP
                                .rich_text()
                                .size(crate::theme::TRANSPORT_BTN_FONT)
                                .color(if has_active {
                                    crate::theme::text_primary()
                                } else {
                                    crate::theme::text_disabled()
                                }),
                        )
                        .min_size(btn_size)
                        .corner_radius(btn_rounding),
                    );
                    if stop_resp.clicked() {
                        play_actions.stop_play = true;
                    }
                    if stop_resp.hovered() {
                        hovered_hint = Some(t!("hint.stop").to_string());
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
            // 空白区判定用 egui 的 hit test 而非按钮区坐标范围：本帧点击若被
            // 任何 widget 消费（interaction_snapshot().clicked 非空），就不是
            // 空白区。transport bar 上可能存在透明/隐藏的按钮（图钉、hover
            // 图标等），坐标范围算不出它们的位置，但点击它们时 clicked 一定
            // 非空，双击它们不会触发最大化。
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
                // 本帧点击未被任何 widget 消费 = 真空白区（隐藏按钮也会被排除）
                let clicked_blank = ui.ctx().interaction_snapshot(|w| w.clicked.is_none());
                if in_bar && !in_timecode && clicked_blank {
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
            // 空白区判定用 egui 的 hit test：press 帧指针下没有任何可交互
            // widget（interaction_snapshot().hovered 为空）才算空白，隐藏按钮
            // 也是 widget，press 它们时 hovered 非空，不会被误判为空白区。
            // StartDrag 只在指针移过点击距离阈值后才发送：按下就 StartDrag
            // 会在 macOS 启动系统级窗口拖拽并吞掉 release 事件，click（进而
            // 双击最大化）无法产生。
            let bar_rect = ui.max_rect();
            let drag_id = ui.id().with("tb_drag_started");
            let mut drag_started: bool = ui.data_mut(|d| d.get_temp(drag_id)).unwrap_or(false);

            if ui.input(|i| i.pointer.primary_down()) {
                if !drag_started && let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                    let in_bar = bar_rect.contains(pos);
                    let in_timecode = timecode_rect
                        .map(|r: egui::Rect| r.contains(pos))
                        .unwrap_or(false);
                    // press 帧指针下无任何可交互 widget = 空白区
                    let pressed_blank = ui.ctx().interaction_snapshot(|w| w.hovered.is_empty());
                    if in_bar && !in_timecode && pressed_blank {
                        // 位移超过点击阈值（egui 判定 click/drag 的分界线）才启动窗口拖动
                        let moved_past_click_dist = ui.input(|i| {
                            i.pointer.hover_pos().is_some_and(|p| {
                                p.distance(pos) >= egui::InputOptions::default().max_click_dist
                            })
                        });
                        if moved_past_click_dist {
                            drag_started = true;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                    }
                }
            } else {
                drag_started = false;
            }

            ui.data_mut(|d| d.insert_temp(drag_id, drag_started));
        });

    TransportResponse {
        toggle_play: play_actions.toggle_play,
        pause_return: play_actions.pause_return,
        stop_play: play_actions.stop_play,
        pending_file_action,
        pending_edit_action,
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

/// 文件菜单 popup 分组（顺序即菜单展示顺序；macOS 原生文件菜单共用，缺"设置/退出"组）。
pub const FILE_GROUPS: [&[FileAction]; 4] = [
    &[FileAction::NewProject, FileAction::Open],
    &[
        FileAction::Save,
        FileAction::SaveAs,
        FileAction::CloseDocument,
    ],
    &[FileAction::ExportAudio, FileAction::ExportMidi],
    &[FileAction::Settings, FileAction::Exit],
];

/// 编辑菜单 popup 分组（macOS 原生编辑菜单共用）。
pub const EDIT_GROUPS: [&[EditAction]; 4] = [
    &[EditAction::Undo, EditAction::Redo],
    &[EditAction::Cut, EditAction::Copy, EditAction::Paste],
    &[
        EditAction::SelectAll,
        EditAction::Duplicate,
        EditAction::Delete,
    ],
    &[EditAction::TransposeUp, EditAction::TransposeDown],
];

/// popup 菜单行的统一渲染（图标 + 文本 + 可选右侧快捷键），
/// 支持选中高亮（单选模式，如播放菜单的跟随档）与可选图钉按钮。
/// 文件/编辑/播放菜单共用，保证三个 popup 视觉与交互一致。
/// 返回 (主按钮响应, 图钉响应)。
struct PopupRowSpec<'a> {
    icon: egui_material_icons::MaterialIcon,
    label: &'a str,
    shortcut: Option<&'a str>,
    enabled: bool,
    /// 选中高亮（单选模式当前项）。
    selected: bool,
    /// Some(当前是否钉住) 渲染图钉按钮；None 不渲染（行宽占满）。
    pin: Option<bool>,
}

fn popup_menu_row(
    ui: &mut egui::Ui,
    spec: PopupRowSpec<'_>,
) -> (egui::Response, Option<egui::Response>) {
    // 每行绝对定位（ui.put）固定尺寸：主按钮 + 可选右侧图钉，
    // 行宽恰好等于菜单内容宽，不参与 popup 宽度反馈；
    // 无快捷键的项用空 shortcut_text 保持左对齐（grow 占中间）。
    const PIN_W: f32 = 26.0;
    const MAIN_PIN_GAP: f32 = 2.0;
    // 行高取紧凑值：transport bar 的 interact_size（32）是为大按钮设计的，
    // 下拉菜单行用 24 更紧凑，避免行内上下留白造成"行间距过大"的观感。
    let row_h = ui.spacing().interact_size.y.min(24.0);
    let row_w = ui.available_width();
    let (row_rect, _) = ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::hover());

    let has_pin = spec.pin.is_some();
    let main_w = if has_pin {
        row_w - PIN_W - MAIN_PIN_GAP
    } else {
        row_w
    };
    let main_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(main_w, row_h));
    let icon_color = if spec.enabled {
        crate::theme::text_bright()
    } else {
        crate::theme::text_disabled()
    };
    let main_btn = egui::Button::selectable(
        spec.selected,
        crate::widgets::icon_text::icon_text(
            spec.icon,
            spec.label,
            crate::theme::FILE_MENU_FONT,
            icon_color,
        ),
    )
    // 去掉边框：egui 按钮 inactive 无边框、hover 时 1px 边框从无到有，
    // 视觉上像文字位移；stroke NONE 后 hover 只剩背景色变化
    .stroke(egui::Stroke::NONE)
    .wrap_mode(egui::TextWrapMode::Truncate)
    .shortcut_text(spec.shortcut.unwrap_or(""));
    // 直接 put（不用 add_enabled_ui 包裹）：scope 嵌套 put 会把已含
    // item_spacing 的 cursor 起点并入 min_rect，导致每行多推进一次 spacing。
    // disabled 状态由调用方过滤点击 + 灰色文本表达。
    let main_resp = ui.put(main_rect, main_btn);

    let mut pin_resp = None;
    if let Some(is_pinned) = spec.pin {
        let pin_rect = egui::Rect::from_min_size(
            egui::pos2(row_rect.max.x - PIN_W, row_rect.min.y),
            egui::vec2(PIN_W, row_h),
        );
        let pin_color = if is_pinned {
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
        let resp = ui.put(pin_rect, pin_btn);
        // 无边框按钮 hover 时补背景反馈，提示可点击
        if resp.hovered() {
            ui.painter().rect_filled(
                pin_rect,
                4.0,
                crate::theme::hover_color(crate::theme::app_bg()),
            );
        }
        pin_resp = Some(resp);
    }
    (main_resp, pin_resp)
}

/// 动作菜单 popup 通用容器：与量化弹框同款
/// （Popup::from_toggle_button_response + CloseOnClickOutside），
/// 固定宽度（快捷键 + 图钉需要稳定的行宽）；每项右侧显示快捷键与图钉按钮。
/// 文件/编辑 popup 共用，保证行为一致。
/// 返回 true 表示图钉状态发生变化（调用方需 save）。
fn show_action_menu<T: PopupRow>(
    button: &egui::Response,
    groups: &[&[T]],
    has_active: bool,
    loading: bool,
    keybindings: &yinhe_editor_core::shortcuts::Keybindings,
    pinned: Option<&mut [bool]>,
    pending_action: &mut Option<T>,
) -> bool {
    let mut pinned_changed = false;
    let mut pin_toggled: Option<usize> = None;
    egui::Popup::from_toggle_button_response(button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(crate::theme::FILE_MENU_WIDTH)
        .show(|ui| {
            // 锁死内容宽度（min == max）：popup 实际宽度由内容决定，
            // 亚像素抖动会让 get_best_align 在候选位置间翻转、整体文字微跳；
            // 宽度恒定后 Area 尺寸与对齐计算全部稳定。
            ui.set_min_width(crate::theme::FILE_MENU_WIDTH);
            ui.set_max_width(crate::theme::FILE_MENU_WIDTH);
            for (gi, group) in groups.iter().enumerate() {
                if gi > 0 {
                    ui.separator();
                }
                for &action in *group {
                    let enabled = action.is_enabled(has_active, loading);
                    // 无图钉的动作不触碰 pinned 数组（播放菜单传 None）
                    let is_pinned = pinned.as_ref().is_some_and(|p| p[action.pinned_index()]);
                    let shortcut = keybindings
                        .get(action.action_id())
                        .first()
                        .map(crate::shortcuts::display_combo);
                    let (main_resp, pin_resp) = popup_menu_row(
                        ui,
                        PopupRowSpec {
                            icon: action.icon(),
                            label: &t!(action.label_key()),
                            shortcut: shortcut.as_deref(),
                            enabled,
                            selected: action.is_selected(),
                            pin: if action.has_pin() {
                                Some(is_pinned)
                            } else {
                                None
                            },
                        },
                    );

                    if enabled && main_resp.clicked() {
                        *pending_action = Some(action);
                        ui.close();
                    }
                    if pin_resp.is_some_and(|r| r.clicked()) {
                        // 图钉只切换固定状态，不关闭菜单；切换到闭包外统一执行
                        pin_toggled = Some(action.pinned_index());
                    }
                }
            }
        });
    if let Some(idx) = pin_toggled
        && let Some(p) = pinned
    {
        p[idx] = !p[idx];
        pinned_changed = true;
    }
    pinned_changed
}

/// 文件按钮 popup（文件动作分组 + 图钉）。
fn show_file_menu(
    button: &egui::Response,
    file_loader: &FileLoader,
    has_active: bool,
    settings: &mut AudioSettings,
    pending_action: &mut Option<FileAction>,
) {
    // 字段级拆分借用：keybindings 只读 + pinned 可变 + 图钉变化后 save
    let keybindings = &settings.keybindings;
    let pinned = &mut settings.pinned_file_actions;
    if show_action_menu(
        button,
        &FILE_GROUPS,
        has_active,
        file_loader.is_loading(),
        keybindings,
        Some(pinned),
        pending_action,
    ) {
        settings.save();
    }
}

/// 编辑按钮 popup（编辑动作分组 + 图钉）。
fn show_edit_menu(
    button: &egui::Response,
    has_active: bool,
    settings: &mut AudioSettings,
    pending_action: &mut Option<EditAction>,
) {
    let keybindings = &settings.keybindings;
    let pinned = &mut settings.pinned_edit_actions;
    if show_action_menu(
        button,
        &EDIT_GROUPS,
        has_active,
        false,
        keybindings,
        Some(pinned),
        pending_action,
    ) {
        settings.save();
    }
}

/// 播放菜单触发的播放动作标志（合并参数，与 KeyboardActions 同风格）。
#[derive(Default)]
struct PlayActions {
    toggle_play: bool,
    pause_return: bool,
    stop_play: bool,
}

/// 播放菜单动作（含跟随档位）。
/// 走与文件/编辑相同的 PopupRow 模板：无图钉（has_pin=false），
/// 跟随档携带选中态（is_selected），播放/暂停携带播放状态（动态图标）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayMenuAction {
    PlayPause { playing: bool },
    Stop,
    Follow(FollowMode, bool),
}

impl PopupRow for PlayMenuAction {
    fn pinned_index(self) -> usize {
        match self {
            PlayMenuAction::PlayPause { .. } => 0,
            PlayMenuAction::Stop => 1,
            // 跟随档无图钉，索引仅占位
            PlayMenuAction::Follow(..) => 0,
        }
    }

    fn has_pin(self) -> bool {
        // 播放/暂停与停止提供图钉；跟随档无图钉
        matches!(
            self,
            PlayMenuAction::PlayPause { .. } | PlayMenuAction::Stop
        )
    }

    fn action_id(self) -> &'static str {
        match self {
            PlayMenuAction::PlayPause { .. } => shortcuts::ACTION_TOGGLE_PLAY,
            PlayMenuAction::Stop => shortcuts::ACTION_STOP,
            // 跟随档没有快捷键
            PlayMenuAction::Follow(..) => "",
        }
    }

    fn icon(self) -> egui_material_icons::MaterialIcon {
        use crate::view_interaction::FollowModeExt;
        match self {
            PlayMenuAction::PlayPause { playing } => {
                if playing {
                    ICON_PAUSE
                } else {
                    ICON_PLAY_ARROW
                }
            }
            PlayMenuAction::Stop => ICON_STOP,
            PlayMenuAction::Follow(mode, _) => mode.icon(),
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            PlayMenuAction::PlayPause { .. } => "shortcuts.play_toggle",
            PlayMenuAction::Stop => "shortcuts.stop",
            PlayMenuAction::Follow(mode, _) => match mode {
                FollowMode::None => "follow.none",
                FollowMode::Centered => "follow.centered",
                FollowMode::Page => "follow.page",
                FollowMode::Continuous => "follow.continuous",
            },
        }
    }

    fn is_enabled(self, has_active: bool, _loading: bool) -> bool {
        has_active
    }

    fn is_selected(self) -> bool {
        match self {
            PlayMenuAction::Follow(_, selected) => selected,
            _ => false,
        }
    }
}

/// 播放按钮 popup：播放/暂停、停止 + 播放跟随四档单选。
/// 与文件/编辑共用 show_action_menu 模板（无图钉，跟随档选中高亮）。
fn show_play_menu(
    button: &egui::Response,
    has_active: bool,
    is_playing: bool,
    follow_mode: &mut FollowMode,
    settings: &mut AudioSettings,
    actions: &mut PlayActions,
) {
    let groups: [&[PlayMenuAction]; 2] = [
        &[
            PlayMenuAction::PlayPause {
                playing: is_playing,
            },
            PlayMenuAction::Stop,
        ],
        &[
            PlayMenuAction::Follow(FollowMode::None, *follow_mode == FollowMode::None),
            PlayMenuAction::Follow(FollowMode::Centered, *follow_mode == FollowMode::Centered),
            PlayMenuAction::Follow(FollowMode::Page, *follow_mode == FollowMode::Page),
            PlayMenuAction::Follow(
                FollowMode::Continuous,
                *follow_mode == FollowMode::Continuous,
            ),
        ],
    ];
    let mut pending = None;
    // 播放/暂停与停止可钉（局部数组，popup 内修改，结束后写回保存）
    let mut pinned = [settings.pinned_play_pause, settings.pinned_stop];
    if show_action_menu(
        button,
        &groups,
        has_active,
        false,
        &settings.keybindings,
        Some(&mut pinned),
        &mut pending,
    ) {
        settings.pinned_play_pause = pinned[0];
        settings.pinned_stop = pinned[1];
        settings.save();
    }
    if let Some(action) = pending {
        match action {
            PlayMenuAction::PlayPause { playing } => {
                if playing {
                    actions.pause_return = true;
                } else {
                    actions.toggle_play = true;
                }
            }
            PlayMenuAction::Stop => actions.stop_play = true,
            PlayMenuAction::Follow(mode, _) => *follow_mode = mode,
        }
    }
}

/// 图钉固定的动作按钮行：作为独立按钮紧跟在菜单按钮右侧，
/// 全部钉上时就是一整行图标。文件/编辑共用。
fn pinned_action_buttons<T: PopupRow>(
    ui: &mut egui::Ui,
    actions: &[T],
    pinned: &mut [bool],
    has_active: bool,
    loading: bool,
    hovered_hint: &mut Option<String>,
    pending: &mut Option<T>,
) {
    // 按钮样式与 transport bar 其他按钮一致
    let btn_size = egui::vec2(
        crate::theme::TRANSPORT_BTN_SIZE,
        crate::theme::TRANSPORT_BTN_SIZE,
    );
    let btn_rounding = egui::CornerRadius::same(2);
    for (i, action) in actions.iter().enumerate() {
        if !pinned[i] {
            continue;
        }
        let enabled = action.is_enabled(has_active, loading);
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
            *pending = Some(*action);
        }
        if pin_resp.hovered() {
            *hovered_hint = Some(t!(action.label_key()).to_string());
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_interaction::FollowModeExt;

    /// 回归测试：播放菜单各行的垂直间距必须一致。
    /// 此前出现过"播放/暂停"与"停止"之间间距异常的问题。
    #[test]
    fn play_menu_rows_have_consistent_spacing() {
        let mut ys: Vec<f32> = Vec::new();
        let mut spacing_y = 0.0f32;
        let mut interact_y = 0.0f32;
        let ctx = egui::Context::default();
        // 注册 material icons 字体（popup_menu_row 用图标字体渲染）
        ctx.add_font(egui_material_icons::font_insert());
        let output = ctx.run_ui(Default::default(), |ui| {
            spacing_y = ui.spacing().item_spacing.y;
            interact_y = ui.spacing().interact_size.y;
            ui.set_min_width(200.0);
            ui.set_max_width(200.0);
            // 带 shortcut（与真实 popup 一致：播放/暂停 Space、停止 Esc）
            let rows: [(PlayMenuAction, Option<&str>); 4] = [
                (PlayMenuAction::PlayPause { playing: false }, Some("Space")),
                (PlayMenuAction::Stop, Some("Esc")),
                (PlayMenuAction::Follow(FollowMode::None, true), None),
                (PlayMenuAction::Follow(FollowMode::Page, false), None),
            ];
            for (r, shortcut) in rows {
                let (resp, _) = popup_menu_row(
                    ui,
                    PopupRowSpec {
                        icon: r.icon(),
                        label: &t!(r.label_key()),
                        shortcut,
                        enabled: true,
                        selected: r.is_selected(),
                        pin: None,
                    },
                );
                ys.push(resp.rect.min.y);
            }
        });
        output.drop_without_applying_deltas();
        let gaps: Vec<f32> = ys.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            gaps.iter().all(|g| (g - gaps[0]).abs() < 0.5),
            "播放菜单行间距不一致: {gaps:?}，ys={ys:?}"
        );
        // 行间距 = 行高 + item_spacing（±1.5 容忍按钮内容高度的取整误差），
        // 不应出现额外的大空隙（曾因 scope 嵌套 put 双推进 spacing 导致每行多 3px）
        let expected = interact_y.min(24.0) + spacing_y;
        assert!(
            gaps.iter().all(|g| (g - expected).abs() < 1.5),
            "行间距异常: {gaps:?}（期望约 {expected}）"
        );
    }

    // ─────────────────────────────────────────────────────────────
    // 双击空白区最大化 / 空白区拖拽窗口 的回归测试（egui_kittest 无头模拟）
    // 空白区判定基于 egui hit test（点击未被任何 widget 消费 / 指针下
    // 无任何可交互 widget），因此 transport bar 上透明隐藏按钮（图钉、
    // hover 图标等）也会被正确排除，双击它们不会误触发最大化。
    // ─────────────────────────────────────────────────────────────

    use egui_kittest::Harness;

    /// 测试状态：记录隐藏按钮（模拟 transport bar 上的透明图标）是否被点击。
    #[derive(Default)]
    struct TbTestState {
        hidden_clicked: bool,
    }

    fn make_transport_harness<'a>(doc: Option<&'a Document>) -> Harness<'a, ()> {
        let mut file_loader = FileLoader::new(yinhe_editor_core::progress::new_shared());
        let mut follow_mode = FollowMode::None;
        let mut active_tool = Tool::Select;
        let mut status_hint: Option<String> = None;
        let mut settings = AudioSettings::default();

        let mut first_frame = true;
        Harness::builder()
            .with_size(egui::vec2(1200.0, 60.0))
            .build_ui_state(
                move |ui, _| {
                    // 第一帧只注册 material-icons 字体（add_font 下一 pass 才生效，
                    // 若同帧渲染图标按钮会 panic），后续帧才渲染 transport bar。
                    if first_frame {
                        first_frame = false;
                        ui.ctx().add_font(egui_material_icons::font_insert());
                        return;
                    }
                    let mut ctx = TransportContext {
                        file_loader: &mut file_loader,
                        doc,
                        follow_mode: &mut follow_mode,
                        active_tool: &mut active_tool,
                        status_hint: &mut status_hint,
                        settings: &mut settings,
                    };
                    show(ui, &mut ctx);
                },
                (),
            )
    }

    /// 同 make_transport_harness，但在 transport bar 最右端空白区放一个
    /// 透明按钮（Button::new("").frame(false)）模拟"隐藏图标"——它渲染在
    /// transport bar 之后（hit test 顶层），是真实存在的交互 widget。
    fn make_harness_with_hidden_button<'a>(doc: Option<&'a Document>) -> Harness<'a, TbTestState> {
        let mut file_loader = FileLoader::new(yinhe_editor_core::progress::new_shared());
        let mut follow_mode = FollowMode::None;
        let mut active_tool = Tool::Select;
        let mut status_hint: Option<String> = None;
        let mut settings = AudioSettings::default();

        let mut first_frame = true;
        Harness::builder()
            .with_size(egui::vec2(1200.0, 60.0))
            .build_ui_state(
                move |ui, state| {
                    if first_frame {
                        first_frame = false;
                        ui.ctx().add_font(egui_material_icons::font_insert());
                        return;
                    }
                    let mut ctx = TransportContext {
                        file_loader: &mut file_loader,
                        doc,
                        follow_mode: &mut follow_mode,
                        active_tool: &mut active_tool,
                        status_hint: &mut status_hint,
                        settings: &mut settings,
                    };
                    show(ui, &mut ctx);
                    // 透明隐藏按钮：位于 x 1150..1174、y 8..32
                    let hidden_btn = ui.put(
                        egui::Rect::from_min_size(egui::pos2(1150.0, 8.0), egui::vec2(24.0, 24.0)),
                        egui::Button::new("").frame(false),
                    );
                    if hidden_btn.clicked() {
                        state.hidden_clicked = true;
                    }
                },
                TbTestState::default(),
            )
    }

    /// 在给定时间注入一个指针事件并渲染一帧。
    fn event_at(h: &mut Harness<'_, ()>, time: f64, event: egui::Event) {
        h.input_mut().time = Some(time);
        h.event(event);
        h.step();
    }

    fn event_at_state(h: &mut Harness<'_, TbTestState>, time: f64, event: egui::Event) {
        h.input_mut().time = Some(time);
        h.event(event);
        h.step();
    }

    fn press_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
        event_at(h, time, egui::Event::PointerMoved(pos));
        event_at(
            h,
            time + 0.001,
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        );
    }

    fn press_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
        event_at_state(h, time, egui::Event::PointerMoved(pos));
        event_at_state(
            h,
            time + 0.001,
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        );
    }

    fn release_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
        event_at(
            h,
            time,
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        );
    }

    fn release_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
        event_at_state(
            h,
            time,
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        );
    }

    fn click_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
        press_at(h, pos, time);
        release_at(h, pos, time + 0.05);
    }

    fn click_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
        press_at_state(h, pos, time);
        release_at_state(h, pos, time + 0.05);
    }

    /// 两次单击，间隔 0.15s（小于 400ms 双击窗口）。
    fn double_click_at(h: &mut Harness<'_, ()>, pos: egui::Pos2, time: f64) {
        click_at(h, pos, time);
        click_at(h, pos, time + 0.15);
    }

    fn double_click_at_state(h: &mut Harness<'_, TbTestState>, pos: egui::Pos2, time: f64) {
        click_at_state(h, pos, time);
        click_at_state(h, pos, time + 0.15);
    }

    fn has_command(h: &Harness<'_, ()>, cmd: &egui::ViewportCommand) -> bool {
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|o| o.commands.iter().any(|c| c == cmd))
    }

    fn has_command_state(h: &Harness<'_, TbTestState>, cmd: &egui::ViewportCommand) -> bool {
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|o| o.commands.iter().any(|c| c == cmd))
    }

    /// 回归测试：双击 transport bar 真空白区域应发送最大化命令。
    #[test]
    fn double_click_blank_area_toggles_maximize() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_transport_harness(Some(&doc));
        double_click_at(&mut h, egui::pos2(1100.0, 20.0), 1.0);
        assert!(
            has_command(&h, &egui::ViewportCommand::Maximized(true)),
            "双击空白区应发送 Maximized 命令，实际命令: {:?}",
            h.output()
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .map(|o| &o.commands)
        );
    }

    /// 回归测试：双击按钮区域不得触发最大化。
    #[test]
    fn double_click_on_button_does_not_maximize() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_transport_harness(Some(&doc));
        // 最左侧按钮（文件菜单按钮）中心
        double_click_at(&mut h, egui::pos2(24.0, 20.0), 1.0);
        assert!(
            !has_command(&h, &egui::ViewportCommand::Maximized(true)),
            "双击按钮不应触发最大化"
        );
    }

    /// 回归测试：双击 transport bar 上的透明隐藏按钮（图钉/hover 图标等）
    /// 不得触发最大化——空白区判定基于 hit test，隐藏按钮也是 widget。
    #[test]
    fn double_click_on_hidden_button_does_not_maximize() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_harness_with_hidden_button(Some(&doc));
        // 隐藏按钮中心（x 1150..1174，y 8..32）
        double_click_at_state(&mut h, egui::pos2(1162.0, 20.0), 1.0);
        assert!(
            !has_command_state(&h, &egui::ViewportCommand::Maximized(true)),
            "双击隐藏按钮不应触发最大化"
        );
    }

    /// 回归测试：隐藏按钮（透明图标）自身必须可点击——不被拖拽/双击逻辑吞掉。
    #[test]
    fn hidden_button_still_clickable() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_harness_with_hidden_button(Some(&doc));
        assert!(!h.state().hidden_clicked);
        click_at_state(&mut h, egui::pos2(1162.0, 20.0), 1.0);
        assert!(h.state().hidden_clicked, "隐藏按钮应响应单击");
    }

    /// 单击空白区不应触发最大化（防止单次点击误触发）。
    #[test]
    fn single_click_blank_area_does_not_maximize() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_transport_harness(Some(&doc));
        click_at(&mut h, egui::pos2(1100.0, 20.0), 1.0);
        assert!(!has_command(&h, &egui::ViewportCommand::Maximized(true)));
    }

    /// 空白区单击（无位移）不应启动窗口拖动——press 不立即 StartDrag，
    /// 这是 click（进而双击）得以产生的保证。
    #[test]
    fn click_blank_area_does_not_start_drag() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_transport_harness(Some(&doc));
        let pos = egui::pos2(1100.0, 20.0);
        press_at(&mut h, pos, 3.0);
        assert!(
            !has_command(&h, &egui::ViewportCommand::StartDrag),
            "按下未移动时不应 StartDrag"
        );
        release_at(&mut h, pos, 3.05);
        assert!(!has_command(&h, &egui::ViewportCommand::StartDrag));
    }

    /// 空白区按住并移动超过点击阈值应启动窗口拖动。
    #[test]
    fn drag_blank_area_starts_window_drag() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_transport_harness(Some(&doc));
        let start = egui::pos2(1100.0, 20.0);
        press_at(&mut h, start, 2.0);
        assert!(!has_command(&h, &egui::ViewportCommand::StartDrag));
        // 移动 10px（超过 max_click_dist 默认 6px）→ 启动窗口拖动
        event_at(
            &mut h,
            2.1,
            egui::Event::PointerMoved(start + egui::vec2(10.0, 0.0)),
        );
        assert!(
            has_command(&h, &egui::ViewportCommand::StartDrag),
            "空白区拖动应发送 StartDrag"
        );
        release_at(&mut h, start + egui::vec2(10.0, 0.0), 2.15);
    }

    /// 隐藏按钮上按下并移动不应启动窗口拖动（隐藏按钮不是空白区）。
    #[test]
    fn drag_on_hidden_button_does_not_start_drag() {
        let doc = yinhe_test_helpers::make_test_document();
        let mut h = make_harness_with_hidden_button(Some(&doc));
        let start = egui::pos2(1162.0, 20.0);
        press_at_state(&mut h, start, 2.0);
        event_at_state(
            &mut h,
            2.1,
            egui::Event::PointerMoved(start + egui::vec2(10.0, 0.0)),
        );
        assert!(
            !has_command_state(&h, &egui::ViewportCommand::StartDrag),
            "隐藏按钮上拖动不应启动窗口拖动"
        );
        release_at_state(&mut h, start + egui::vec2(10.0, 0.0), 2.15);
    }
}
