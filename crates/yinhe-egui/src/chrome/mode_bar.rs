use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::right_panel::RightTab;

#[derive(PartialEq)]
pub enum ViewMode {
    Arrange,
    Mix,
    Edit,
}

impl ViewMode {
    /// 是否显示 arrange（transport）区域。
    #[inline]
    pub fn show_transport(&self) -> bool {
        matches!(self, ViewMode::Arrange)
    }

    /// 是否显示 piano roll 区域。
    /// `show_pianoroll_in_arrange` 是用户偏好：Arrange 模式下是否同时显示 PR。
    #[inline]
    pub fn show_pianoroll(&self, show_pianoroll_in_arrange: bool) -> bool {
        match self {
            ViewMode::Arrange => show_pianoroll_in_arrange,
            ViewMode::Mix => false,
            ViewMode::Edit => true,
        }
    }
}

/// 平台主修饰键：macOS 用 ⌘，其他平台用 Ctrl+。
pub(crate) fn mod_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    }
}

/// 返回 true 表示当前鼠标悬停在该按钮上（供状态栏讲解行使用）。
fn mode_button(ui: &mut egui::Ui, label: &str, is_selected: bool, on_click: impl FnOnce()) -> bool {
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(crate::theme::MODE_LABEL_FONT)
                .color(if is_selected {
                    crate::theme::ACCENT_ACTIVE
                } else {
                    crate::theme::MODE_BAR_TEXT
                }),
        )
        .sense(egui::Sense::click())
        .selectable(false),
    );
    crate::widgets::hover::hover_highlight(
        ui,
        &resp,
        label,
        egui::FontId::proportional(crate::theme::MODE_LABEL_FONT),
        is_selected,
    );
    if resp.clicked() {
        on_click();
    }
    resp.hovered()
}

fn right_icon_button(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    is_active: bool,
    on_click: impl FnOnce(),
) -> bool {
    let color = if is_active {
        crate::theme::ACCENT_ACTIVE
    } else {
        crate::theme::MODE_BAR_TEXT
    };
    let resp = ui.add(
        egui::Label::new(icon.rich_text().size(14.0).color(color))
            .sense(egui::Sense::click())
            .selectable(false),
    );
    crate::widgets::hover::hover_highlight(
        ui,
        &resp,
        icon.codepoint,
        egui::FontId::new(14.0, icon.font_family()),
        is_active,
    );
    if resp.clicked() {
        on_click();
    }
    resp.hovered()
}

/// A compact "LABEL value" readout. Both label and value at `MODE_LABEL_FONT`.
fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    metric_with_value_sz(ui, label, value, crate::theme::MODE_LABEL_FONT);
}

fn metric_with_value_sz(ui: &mut egui::Ui, label: &str, value: &str, value_sz: f32) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(crate::theme::MODE_LABEL_FONT)
                .color(crate::theme::MODE_BAR_TEXT),
        )
        .selectable(false),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .size(value_sz)
                .color(crate::theme::ACCENT_ACTIVE),
        )
        .selectable(false),
    );
}

/// Like [`metric`], but the value is clickable (e.g. to open a detail popup).
/// Returns true when hovered (for the status-line hint).
fn metric_clickable(ui: &mut egui::Ui, label: &str, value: &str, on_click: impl FnOnce()) -> bool {
    metric_clickable_with_value_sz(ui, label, value, crate::theme::MODE_LABEL_FONT, on_click)
}

fn metric_clickable_with_value_sz(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    value_sz: f32,
    on_click: impl FnOnce(),
) -> bool {
    ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(crate::theme::MODE_LABEL_FONT)
                .color(crate::theme::MODE_BAR_TEXT),
        )
        .selectable(false),
    );
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .size(value_sz)
                .color(crate::theme::ACCENT_ACTIVE),
        )
        .sense(egui::Sense::click())
        .selectable(false),
    );
    if resp.clicked() {
        on_click();
    }
    resp.hovered()
}

#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn show(
    ui: &mut egui::Ui,
    view_mode: &mut ViewMode,
    show_pianoroll_in_arrange: &mut bool,
    right_tab: &mut Option<RightTab>,
    cpu_usage: f32,
    mem_mb: f64,
    fps: f32,
    show_mem_breakdown: &mut bool,
    status_hint: &Option<String>,
) {
    egui::Panel::bottom("bottom_bar")
        .frame(egui::Frame {
            inner_margin: egui::Margin::symmetric(8, 6),
            fill: crate::theme::APP_BG,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(2.0);

                // 本帧左侧控件 hover 提示（右侧图标提示下帧显示，见 icon_hint_id）
                let mut hovered_hint: Option<String> = None;

                if mode_button(ui, "ARRANGE", *view_mode == ViewMode::Arrange, || {
                    *view_mode = ViewMode::Arrange;
                }) {
                    hovered_hint = Some(t!("hint.mode_arrange").to_string());
                }

                ui.add_space(2.0);

                if mode_button(ui, "MIX", *view_mode == ViewMode::Mix, || {
                    *view_mode = ViewMode::Mix;
                }) {
                    hovered_hint = Some(t!("hint.mode_mix").to_string());
                }

                ui.add_space(2.0);

                if mode_button(ui, "EDIT", *view_mode == ViewMode::Edit, || {
                    *view_mode = ViewMode::Edit;
                }) {
                    hovered_hint = Some(t!("hint.mode_edit").to_string());
                }

                // ── Piano roll toggle ──
                if *view_mode == ViewMode::Arrange {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);

                    let piano_color = if *show_pianoroll_in_arrange {
                        crate::theme::ACCENT_ACTIVE
                    } else {
                        crate::theme::MODE_BAR_TEXT
                    };
                    let piano_resp = ui.add(
                        egui::Label::new(ICON_PIANO.rich_text().size(14.0).color(piano_color))
                            .sense(egui::Sense::click())
                            .selectable(false),
                    );
                    crate::widgets::hover::hover_highlight(
                        ui,
                        &piano_resp,
                        ICON_PIANO.codepoint,
                        egui::FontId::new(14.0, ICON_PIANO.font_family()),
                        *show_pianoroll_in_arrange,
                    );
                    if piano_resp.clicked() {
                        *show_pianoroll_in_arrange = !*show_pianoroll_in_arrange;
                    }
                    if piano_resp.hovered() {
                        hovered_hint = Some(t!("hint.pr_toggle").to_string());
                    }
                }

                // ── Spacer: push right icons to the right edge ──
                // 讲解行用 painter 绘制在 mode 按钮右侧（左对齐），绘制时机在
                // 本布局之后，右侧图标 hover 状态当帧有效（无跨帧闪烁），
                // 且不参与布局，不会像 right_to_left 内联那样被推到屏幕中部。
                let hint_x = ui.cursor().min.x + 12.0;
                let bar_center_y = ui.max_rect().center().y;

                let icon_hint: Option<String> = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut icon_hint: Option<String> = None;

                        // Right-most first (from right to left):
                        //  1. ICON_INFO
                        //  2. ICON_MUSIC_CAST
                        //  3. ICON_AUTO_STORIES (event browser)

                        if right_icon_button(
                            ui,
                            ICON_INFO,
                            *right_tab == Some(RightTab::Info),
                            || {
                                *right_tab = if *right_tab == Some(RightTab::Info) {
                                    None
                                } else {
                                    Some(RightTab::Info)
                                };
                            },
                        ) {
                            icon_hint = Some(t!("hint.right_info").to_string());
                        }

                        ui.add_space(4.0);

                        if right_icon_button(
                            ui,
                            ICON_MUSIC_CAST,
                            *right_tab == Some(RightTab::SoundFont),
                            || {
                                *right_tab = if *right_tab == Some(RightTab::SoundFont) {
                                    None
                                } else {
                                    Some(RightTab::SoundFont)
                                };
                            },
                        ) {
                            icon_hint = Some(t!("hint.right_soundfont").to_string());
                        }

                        ui.add_space(4.0);

                        if right_icon_button(
                            ui,
                            ICON_FOLDER_ZIP,
                            *right_tab == Some(RightTab::EventBrowser),
                            || {
                                *right_tab = if *right_tab == Some(RightTab::EventBrowser) {
                                    None
                                } else {
                                    Some(RightTab::EventBrowser)
                                };
                            },
                        ) {
                            icon_hint = Some(t!("hint.right_event_browser").to_string());
                        }

                        // ── Resource metrics (CPU / MEM / FPS) — left of the right icons ──
                        ui.separator();
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            metric(ui, "CPU", &format!("{:.1}%", cpu_usage));
                            ui.add_space(12.0);
                            let ctx_clone = ui.ctx().clone();
                            if metric_clickable(ui, "MEM", &format!("{:.1} MB", mem_mb), || {
                                *show_mem_breakdown = true;
                                crate::chrome::dialog::raise_viewport(
                                    &ctx_clone,
                                    egui::ViewportId::from_hash_of("memory_breakdown_dialog"),
                                );
                            }) {
                                icon_hint = Some(t!("hint.mem").to_string());
                            }
                            ui.add_space(12.0);
                            metric(ui, "FPS", &format!("{:.1}", fps));
                        });

                        icon_hint
                    })
                    .inner;

                // ── 讲解/状态文字：模式栏控件 > 视图提示；模式栏空白处清空 ──
                let over_popup = crate::view_interaction::pointer_over_popup(ui.ctx());
                let over_bar = ui.input(|i| {
                    i.pointer
                        .hover_pos()
                        .is_some_and(|p| ui.max_rect().contains(p))
                });
                let display_text = if over_popup {
                    None
                } else if icon_hint.is_some() {
                    icon_hint
                } else if hovered_hint.is_some() {
                    hovered_hint
                } else if over_bar {
                    None
                } else {
                    status_hint.clone()
                };
                if let Some(text) = display_text {
                    ui.painter().text(
                        egui::pos2(hint_x, bar_center_y),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::proportional(crate::theme::MODE_LABEL_FONT),
                        crate::theme::MODE_BAR_TEXT,
                    );
                }
            });
        });
}
