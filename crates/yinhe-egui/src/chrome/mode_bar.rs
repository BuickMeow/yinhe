use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::right_panel::RightTab;

#[derive(PartialEq, Debug)]
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
    /// `show_pianoroll_in_arrange` 是用户偏好：Arrange/Mix 模式下是否同时显示 PR。
    #[inline]
    pub fn show_pianoroll(&self, show_pianoroll_in_arrange: bool) -> bool {
        match self {
            ViewMode::Arrange => show_pianoroll_in_arrange,
            // MIX：底部三选一面板之一（通道 AR 在上方）。
            ViewMode::Mix => show_pianoroll_in_arrange,
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

/// 设置钢琴卷帘可见性；展开时与设备链/混音台互斥（同时只展开一个）。
pub(crate) fn set_pianoroll_visible(
    show_pianoroll_in_arrange: &mut bool,
    show_bottom_dock: &mut bool,
    show_mixer_panel: &mut bool,
    visible: bool,
) {
    *show_pianoroll_in_arrange = visible;
    if visible {
        *show_bottom_dock = false;
        *show_mixer_panel = false;
    }
}

/// 设置底部设备栏可见性；展开时与钢琴卷帘/混音台互斥（同时只展开一个）。
pub(crate) fn set_dock_visible(
    show_pianoroll_in_arrange: &mut bool,
    show_bottom_dock: &mut bool,
    show_mixer_panel: &mut bool,
    visible: bool,
) {
    *show_bottom_dock = visible;
    if visible {
        *show_pianoroll_in_arrange = false;
        *show_mixer_panel = false;
    }
}

/// 设置混音台面板可见性（仅 MIX）；展开时与钢琴卷帘/设备链互斥。
pub(crate) fn set_mixer_visible(
    show_pianoroll_in_arrange: &mut bool,
    show_bottom_dock: &mut bool,
    show_mixer_panel: &mut bool,
    visible: bool,
) {
    *show_mixer_panel = visible;
    if visible {
        *show_pianoroll_in_arrange = false;
        *show_bottom_dock = false;
    }
}

/// 模式按钮（ARRANGE/MIX/EDIT）：返回 `Response`（hover 提示与浮动气泡共用）。
fn mode_button(
    ui: &mut egui::Ui,
    label: &str,
    is_selected: bool,
    on_click: impl FnOnce(),
) -> egui::Response {
    let resp = crate::widgets::hover::hover_button(
        ui,
        label,
        egui::FontId::proportional(crate::theme::MODE_LABEL_FONT),
        crate::theme::mode_bar_text(),
        is_selected,
    );
    if resp.clicked() {
        on_click();
    }
    resp
}

fn right_icon_button(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    is_active: bool,
    on_click: impl FnOnce(),
) -> bool {
    let resp = crate::widgets::hover::hover_button(
        ui,
        icon.codepoint,
        egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
        crate::theme::mode_bar_text(),
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
                .color(crate::theme::mode_bar_text()),
        )
        .selectable(false),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .size(value_sz)
                .color(crate::theme::accent_active()),
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
                .color(crate::theme::mode_bar_text()),
        )
        .selectable(false),
    );
    let resp = ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .size(value_sz)
                .color(crate::theme::accent_active()),
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
    show_bottom_dock: &mut bool,
    show_mixer_panel: &mut bool,
    float_pianoroll: &mut bool,
    float_bottom_dock: &mut bool,
    float_mixer: &mut bool,
    float_arrange: &mut bool,
    right_tab: &mut Option<RightTab>,
    cpu_usage: f32,
    mem_mb: f64,
    fps: f32,
    // 当前 PR 音符 LOD 块宽（tick）；None = 原始音符层。
    lod_block: Option<u32>,
    show_mem_breakdown: &mut bool,
    notifications: &mut crate::widgets::toast::Notifications,
) {
    // 面板按钮本帧信息（浮动气泡锚点/触发用；Panel 闭包内收集，闭包外消费）。
    let mut panel_buttons: Vec<PanelButtonInfo> = Vec::new();
    egui::Panel::bottom("bottom_bar")
        .frame(egui::Frame {
            inner_margin: egui::Margin::symmetric(8, 0),
            fill: crate::theme::app_bg(),
            ..Default::default()
        })
        .show(ui, |ui| {
            // 固定为与 title bar 相同的高度（TITLE_BAR_H），内容垂直居中。
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), crate::theme::TITLE_BAR_H),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add_space(2.0);

                    // 本帧左侧控件 hover 提示（右侧图标提示下帧显示，见 icon_hint_id）
                    let mut hovered_hint: Option<String> = None;

                    let arrange_resp =
                        mode_button(ui, "ARRANGE", *view_mode == ViewMode::Arrange, || {
                            *view_mode = ViewMode::Arrange;
                        });
                    if arrange_resp.hovered() {
                        hovered_hint = Some(t!("hint.mode_arrange").to_string());
                    }
                    panel_buttons.push(PanelButtonInfo {
                        target: PanelKind::Arrange,
                        rect: arrange_resp.rect,
                        hovered: arrange_resp.hovered(),
                        secondary_clicked: arrange_resp.secondary_clicked(),
                    });

                    ui.add_space(2.0);

                    let mix_resp = mode_button(ui, "MIX", *view_mode == ViewMode::Mix, || {
                        *view_mode = ViewMode::Mix;
                    });
                    if mix_resp.hovered() {
                        hovered_hint = Some(t!("hint.mode_mix").to_string());
                    }
                    // MIX 的气泡浮动「混音台」（通道走带主视图暂不支持浮动）。
                    panel_buttons.push(PanelButtonInfo {
                        target: PanelKind::Mixer,
                        rect: mix_resp.rect,
                        hovered: mix_resp.hovered(),
                        secondary_clicked: mix_resp.secondary_clicked(),
                    });

                    ui.add_space(2.0);

                    let edit_resp = mode_button(ui, "EDIT", *view_mode == ViewMode::Edit, || {
                        *view_mode = ViewMode::Edit;
                    });
                    if edit_resp.hovered() {
                        hovered_hint = Some(t!("hint.mode_edit").to_string());
                    }
                    // EDIT 的气泡浮动「钢琴卷帘」（与钢琴图标同一个窗口）。
                    panel_buttons.push(PanelButtonInfo {
                        target: PanelKind::Pianoroll,
                        rect: edit_resp.rect,
                        hovered: edit_resp.hovered(),
                        secondary_clicked: edit_resp.secondary_clicked(),
                    });

                    // ── Piano roll toggle（ARRANGE / MIX 底部面板之一）──
                    if matches!(*view_mode, ViewMode::Arrange | ViewMode::Mix) {
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);

                        let piano_resp = crate::widgets::hover::hover_button(
                            ui,
                            ICON_PIANO.codepoint,
                            egui::FontId::new(crate::theme::ICON_FONT, ICON_PIANO.font_family()),
                            crate::theme::mode_bar_text(),
                            *show_pianoroll_in_arrange || *float_pianoroll,
                        );
                        if piano_resp.clicked() {
                            if *float_pianoroll {
                                // 浮动窗口打开时：点击 = 关闭窗口（停靠形态保持不变）。
                                *float_pianoroll = false;
                                crate::chrome::dialog::close_viewport(
                                    ui.ctx(),
                                    crate::app::floating::pianoroll_id(),
                                );
                            } else {
                                set_pianoroll_visible(
                                    show_pianoroll_in_arrange,
                                    show_bottom_dock,
                                    show_mixer_panel,
                                    !*show_pianoroll_in_arrange,
                                );
                            }
                        }
                        if piano_resp.hovered() {
                            hovered_hint = Some(t!("hint.pr_toggle").to_string());
                        }
                        panel_buttons.push(PanelButtonInfo {
                            target: PanelKind::Pianoroll,
                            rect: piano_resp.rect,
                            hovered: piano_resp.hovered(),
                            secondary_clicked: piano_resp.secondary_clicked(),
                        });
                    } else {
                        // EDIT 无钢琴栏：dock 前保留一次分隔（与模式按钮分隔）。
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);
                    }

                    // ── Bottom dock toggle（三视图通用设备栏；紧贴钢琴按钮，无分隔线）──
                    {
                        let dock_icon = egui_material_icons::icons::ICON_DOCK_TO_RIGHT;
                        let dock_resp = crate::widgets::hover::hover_button(
                            ui,
                            dock_icon.codepoint,
                            egui::FontId::new(crate::theme::ICON_FONT, dock_icon.font_family()),
                            crate::theme::mode_bar_text(),
                            *show_bottom_dock || *float_bottom_dock,
                        );
                        if dock_resp.clicked() {
                            if *float_bottom_dock {
                                // 浮动窗口打开时：点击 = 关闭窗口（停靠形态保持不变）。
                                *float_bottom_dock = false;
                                crate::chrome::dialog::close_viewport(
                                    ui.ctx(),
                                    crate::app::floating::dock_id(),
                                );
                            } else {
                                set_dock_visible(
                                    show_pianoroll_in_arrange,
                                    show_bottom_dock,
                                    show_mixer_panel,
                                    !*show_bottom_dock,
                                );
                            }
                        }
                        if dock_resp.hovered() {
                            hovered_hint = Some(t!("hint.dock_toggle").to_string());
                        }
                        panel_buttons.push(PanelButtonInfo {
                            target: PanelKind::Dock,
                            rect: dock_resp.rect,
                            hovered: dock_resp.hovered(),
                            secondary_clicked: dock_resp.secondary_clicked(),
                        });
                    }

                    // ── Mixer toggle（仅 MIX：底部三选一之混音台）──
                    if *view_mode == ViewMode::Mix {
                        let mixer_icon = egui_material_icons::icons::ICON_TUNE;
                        // 图标旋转：顺时针 270°（= 逆时针 90°），朝向与实际混音台一致
                        let mixer_resp = crate::widgets::hover::hover_button_rotated(
                            ui,
                            mixer_icon.codepoint,
                            egui::FontId::new(crate::theme::ICON_FONT, mixer_icon.font_family()),
                            crate::theme::mode_bar_text(),
                            *show_mixer_panel || *float_mixer,
                            -std::f32::consts::FRAC_PI_2,
                        );
                        if mixer_resp.clicked() {
                            if *float_mixer {
                                // 浮动窗口打开时：点击 = 关闭窗口（停靠形态保持不变）。
                                *float_mixer = false;
                                crate::chrome::dialog::close_viewport(
                                    ui.ctx(),
                                    crate::app::floating::mixer_id(),
                                );
                            } else {
                                set_mixer_visible(
                                    show_pianoroll_in_arrange,
                                    show_bottom_dock,
                                    show_mixer_panel,
                                    !*show_mixer_panel,
                                );
                            }
                        }
                        if mixer_resp.hovered() {
                            hovered_hint = Some(t!("hint.mixer_toggle").to_string());
                        }
                        panel_buttons.push(PanelButtonInfo {
                            target: PanelKind::Mixer,
                            rect: mixer_resp.rect,
                            hovered: mixer_resp.hovered(),
                            secondary_clicked: mixer_resp.secondary_clicked(),
                        });
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
                            //  1. Notifications (bell)
                            //  2. ICON_INFO

                            // 通知铃铛：最右侧第一个，不与右侧栏联动；总开关关闭时隐藏
                            if notifications.is_enabled() {
                                let has_unread = notifications.has_unread();
                                let icon = if has_unread {
                                    ICON_NOTIFICATIONS_UNREAD
                                } else {
                                    ICON_NOTIFICATIONS
                                };
                                let was_open = notifications.center_open;
                                if right_icon_button(ui, icon, was_open, || {
                                    notifications.center_open = !was_open;
                                    if notifications.center_open {
                                        notifications.mark_all_read();
                                    }
                                }) {
                                    icon_hint = Some(t!("hint.notifications").to_string());
                                }
                                ui.add_space(4.0);
                            }

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
                                let lod_text = match lod_block {
                                    Some(block) => format!("{block}"),
                                    None => "0".to_string(),
                                };
                                metric_clickable(ui, "LOD", &lod_text, || {});
                                ui.add_space(12.0);
                                metric(ui, "FPS", &format!("{:.1}", fps));
                            });

                            icon_hint
                        })
                        .inner;

                    // ── 讲解/状态文字：模式栏控件 > 视图提示；模式栏空白处清空 ──
                    // 指针在弹窗（菜单/下拉等）上时，模式栏控件提示已失效，改用弹窗
                    // 自身写入的讲解（`hint::current`，由弹窗内控件 `hint::hover` 写入）。
                    let over_popup = crate::view_interaction::pointer_over_popup(ui.ctx());
                    let over_bar = ui.input(|i| {
                        i.pointer
                            .hover_pos()
                            .is_some_and(|p| ui.max_rect().contains(p))
                    });
                    let display_text = if over_popup {
                        crate::widgets::hint::current(ui.ctx())
                    } else if icon_hint.is_some() {
                        icon_hint
                    } else if hovered_hint.is_some() {
                        hovered_hint
                    } else if over_bar {
                        Some(t!("hint.panel.mode_bar").to_string())
                    } else {
                        crate::widgets::hint::current(ui.ctx())
                    };
                    if let Some(text) = display_text {
                        ui.painter().text(
                            egui::pos2(hint_x, bar_center_y),
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::proportional(crate::theme::MODE_LABEL_FONT),
                            crate::theme::mode_bar_text(),
                        );
                    }
                },
            );
        });

    // ── 面板按钮浮动气泡（悬停延迟显示 / 右键固定 / 点击图标浮动或收回）──
    float_bubble(
        ui.ctx(),
        &panel_buttons,
        view_mode,
        show_pianoroll_in_arrange,
        float_pianoroll,
        show_bottom_dock,
        float_bottom_dock,
        show_mixer_panel,
        float_mixer,
        float_arrange,
    );
}

/// 可浮动视图/面板种类（模式栏气泡 / 视图菜单 / macOS 原生菜单共用）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PanelKind {
    /// 工程走带主视图（ARRANGE）。
    Arrange,
    Pianoroll,
    Dock,
    Mixer,
}

impl PanelKind {
    /// 全部可浮动视图（视图菜单/原生菜单遍历用）。
    pub const ALL: [PanelKind; 4] = [Self::Arrange, Self::Pianoroll, Self::Dock, Self::Mixer];
}

/// 浮动开关的借用集合（`App` 字段或模式栏参数）。
/// 集中「泊出 / 收回停靠」逻辑，避免气泡与菜单各自重复 match 分支。
pub(crate) struct FloatSwitches<'a> {
    pub show_pianoroll: &'a mut bool,
    pub float_pianoroll: &'a mut bool,
    pub show_dock: &'a mut bool,
    pub float_dock: &'a mut bool,
    pub show_mixer: &'a mut bool,
    pub float_mixer: &'a mut bool,
    /// 工程走带视图浮动（视图由模式决定，没有独立停靠开关）。
    pub float_arrange: &'a mut bool,
    /// 当前视图模式：收回停靠时切到该视图所在模式；
    /// EDIT 下 PR 即主视图，收回不启用底部面板。
    pub view_mode: &'a mut ViewMode,
}

impl FloatSwitches<'_> {
    /// 该视图当前是否处于浮动形态。
    pub(crate) fn is_floating(&self, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Arrange => *self.float_arrange,
            PanelKind::Pianoroll => *self.float_pianoroll,
            PanelKind::Dock => *self.float_dock,
            PanelKind::Mixer => *self.float_mixer,
        }
    }

    /// 泊出：从主窗口移走并拉前台（浮动不占主窗口空间，不触发三选一互斥）。
    pub(crate) fn float(&mut self, ctx: &egui::Context, kind: PanelKind) {
        match kind {
            PanelKind::Arrange => {
                *self.float_arrange = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::app::floating::arrange_id());
            }
            PanelKind::Pianoroll => {
                *self.show_pianoroll = false;
                *self.float_pianoroll = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::app::floating::pianoroll_id());
            }
            PanelKind::Dock => {
                *self.show_dock = false;
                *self.float_dock = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::app::floating::dock_id());
            }
            PanelKind::Mixer => {
                *self.show_mixer = false;
                *self.float_mixer = true;
                crate::chrome::dialog::raise_viewport(ctx, crate::app::floating::mixer_id());
            }
        }
    }

    /// 收回停靠：关窗口 + 视图回主窗口（底部面板与其它停靠面板三选一互斥）。
    pub(crate) fn dock_back(&mut self, ctx: &egui::Context, kind: PanelKind) {
        match kind {
            PanelKind::Arrange => {
                *self.float_arrange = false;
                crate::chrome::dialog::close_viewport(ctx, crate::app::floating::arrange_id());
                // 工程走带只在 ARRANGE 模式的主窗口显示：收回即切回该模式。
                *self.view_mode = ViewMode::Arrange;
            }
            PanelKind::Pianoroll => {
                *self.float_pianoroll = false;
                // EDIT 模式 PR 即主视图：收回直接回主窗口，不启用底部面板
                // （避免误关停靠的设备栏/混音台）。
                if *self.view_mode != ViewMode::Edit {
                    set_pianoroll_visible(
                        self.show_pianoroll,
                        self.show_dock,
                        self.show_mixer,
                        true,
                    );
                }
                crate::chrome::dialog::close_viewport(ctx, crate::app::floating::pianoroll_id());
            }
            PanelKind::Dock => {
                *self.float_dock = false;
                set_dock_visible(self.show_pianoroll, self.show_dock, self.show_mixer, true);
                crate::chrome::dialog::close_viewport(ctx, crate::app::floating::dock_id());
            }
            PanelKind::Mixer => {
                *self.float_mixer = false;
                set_mixer_visible(self.show_pianoroll, self.show_dock, self.show_mixer, true);
                crate::chrome::dialog::close_viewport(ctx, crate::app::floating::mixer_id());
                // 混音台面板只在 MIX 模式停靠显示：收回即切到 MIX。
                *self.view_mode = ViewMode::Mix;
            }
        }
    }

    /// 切换：未浮动 → 泊出；已浮动 → 收回停靠。
    pub(crate) fn toggle(&mut self, ctx: &egui::Context, kind: PanelKind) {
        if self.is_floating(kind) {
            self.dock_back(ctx, kind);
        } else {
            self.float(ctx, kind);
        }
    }
}

/// 面板按钮本帧信息（气泡触发与锚点定位用）。
struct PanelButtonInfo {
    target: PanelKind,
    rect: egui::Rect,
    hovered: bool,
    secondary_clicked: bool,
}

/// 悬停气泡跨帧状态（存 egui temp data）。
#[derive(Clone, Copy, Default)]
struct FloatBubbleState {
    /// 当前气泡指向的面板（None = 未显示）。
    target: Option<PanelKind>,
    /// 右键固定：不随指针移出关闭（点外部 / Esc / 再次右键关闭）。
    pinned: bool,
    /// 悬停候选（指针停在按钮上、等待延迟计时）。
    candidate: Option<PanelKind>,
    candidate_since: f64,
    /// 指针离开按钮与气泡后的隐藏计时起点（None = 不应隐藏）。
    hide_since: Option<f64>,
}

/// 悬停延迟：指针停留多久才弹出气泡（避免扫过模式栏时一路弹）。
const BUBBLE_SHOW_DELAY: f64 = 0.25;
/// 隐藏宽限：指针离开按钮与气泡后延迟多久关闭（允许鼠标从按钮移进气泡点击）。
const BUBBLE_HIDE_GRACE: f64 = 0.2;
/// 气泡尺寸（单图标；正方形，尾巴不计入）。
const BUBBLE_SIZE: egui::Vec2 = egui::vec2(32.0, 32.0);
/// 尾巴高度 = 气泡底边到按钮顶边的距离。
const BUBBLE_TAIL_H: f32 = 7.0;
/// 尾巴底宽。
const BUBBLE_TAIL_W: f32 = 10.0;

/// 气泡图标点击后的动作。
#[derive(Clone, Copy)]
enum BubbleAction {
    /// 泊出为独立窗口。
    Float,
    /// 收回停靠。
    DockBack,
}

/// 面板按钮的浮动气泡：悬停延迟显示、右键固定、点击图标浮动/收回。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn float_bubble(
    ctx: &egui::Context,
    buttons: &[PanelButtonInfo],
    view_mode: &mut ViewMode,
    show_pianoroll_in_arrange: &mut bool,
    float_pianoroll: &mut bool,
    show_bottom_dock: &mut bool,
    float_bottom_dock: &mut bool,
    show_mixer_panel: &mut bool,
    float_mixer: &mut bool,
    float_arrange: &mut bool,
) {
    let state_id = egui::Id::new("mode_bar_float_bubble");
    let mut st: FloatBubbleState = ctx.data_mut(|d| d.get_temp(state_id)).unwrap_or_default();
    let now = ctx.input(|i| i.time);
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let any_click = ctx.input(|i| i.pointer.any_click());
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let hovered = buttons.iter().find(|b| b.hovered).map(|b| b.target);

    // ── 右键：固定 / 取消固定（同一按钮再次右键 = 关闭）──
    let mut just_pinned = false;
    for b in buttons {
        if !b.secondary_clicked {
            continue;
        }
        if st.pinned && st.target == Some(b.target) {
            st = FloatBubbleState::default();
        } else {
            st.target = Some(b.target);
            st.pinned = true;
            st.candidate = None;
            just_pinned = true;
        }
    }

    // ── 悬停：延迟显示（固定态不参与）──
    if !st.pinned {
        match hovered {
            Some(t) => {
                if st.candidate != Some(t) {
                    st.candidate = Some(t);
                    st.candidate_since = now;
                    if st.target != Some(t) {
                        st.target = None;
                    }
                }
                if st.target != Some(t) && now - st.candidate_since >= BUBBLE_SHOW_DELAY {
                    st.target = Some(t);
                }
            }
            None => {
                st.candidate = None;
            }
        }
    }

    if esc {
        st = FloatBubbleState::default();
    }

    // ── 绘制气泡（若显示中）──
    let mut over_bubble = false;
    if let Some(target) = st.target {
        let Some(btn) = buttons.iter().find(|b| b.target == target) else {
            // 按钮本帧不可见（如 EDIT 无钢琴按钮）→ 气泡随之关闭。
            ctx.data_mut(|d| d.insert_temp(state_id, FloatBubbleState::default()));
            return;
        };
        // 浮动开关借用集合（图标状态与点击动作共用同一来源）。
        let mut sw = FloatSwitches {
            show_pianoroll: &mut *show_pianoroll_in_arrange,
            float_pianoroll: &mut *float_pianoroll,
            show_dock: &mut *show_bottom_dock,
            float_dock: &mut *float_bottom_dock,
            show_mixer: &mut *show_mixer_panel,
            float_mixer: &mut *float_mixer,
            float_arrange: &mut *float_arrange,
            view_mode: &mut *view_mode,
        };
        let floating = sw.is_floating(target);
        // 气泡在按钮正上方居中；靠窗口边缘时水平收进。
        // 注意：`inner_rect` 是**显示器坐标**（窗口不在显示器原点时 min.x ≠ 0），
        // 而按钮 rect 是视口局部坐标 → 只能取宽度做 clamp，不能用 min/max.x。
        let win_w = ctx.input(|i| i.viewport().inner_rect).map(|r| r.width());
        let cx = match win_w {
            Some(w) if w > BUBBLE_SIZE.x + 8.0 => btn
                .rect
                .center()
                .x
                .clamp(BUBBLE_SIZE.x / 2.0 + 4.0, w - BUBBLE_SIZE.x / 2.0 - 4.0),
            _ => btn.rect.center().x,
        };
        let bubble_rect = egui::Rect::from_min_size(
            egui::pos2(
                cx - BUBBLE_SIZE.x / 2.0,
                btn.rect.min.y - BUBBLE_TAIL_H - BUBBLE_SIZE.y,
            ),
            BUBBLE_SIZE,
        );
        let area = egui::Area::new(state_id.with("area"))
            .order(egui::Order::Foreground)
            .fixed_pos(bubble_rect.min)
            .constrain(false)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(BUBBLE_SIZE, egui::Sense::hover());
                let painter = ui.painter();
                // 样式对齐弹窗（`Frame::menu`）：window_fill + window_stroke + menu 圆角。
                let visuals = &ui.style().visuals;
                let fill = visuals.window_fill;
                let stroke = visuals.window_stroke;
                let corner = visuals.menu_corner_radius;
                // 尾巴（先画；气泡主体盖住其顶边，接缝再用底色抹平）。
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(cx - BUBBLE_TAIL_W / 2.0, rect.max.y),
                        egui::pos2(cx + BUBBLE_TAIL_W / 2.0, rect.max.y),
                        egui::pos2(cx, rect.max.y + BUBBLE_TAIL_H),
                    ],
                    fill,
                    stroke,
                ));
                painter.rect(rect, corner, fill, stroke, egui::StrokeKind::Inside);
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(cx - BUBBLE_TAIL_W / 2.0, rect.max.y - 1.0),
                        egui::pos2(cx + BUBBLE_TAIL_W / 2.0, rect.max.y + 0.5),
                    ),
                    0.0,
                    fill,
                );
                // 图标按钮：未浮动 = 泊出；已浮动 = 收回停靠。
                let icon = if floating {
                    ICON_OPEN_IN_NEW_OFF
                } else {
                    ICON_OPEN_IN_NEW
                };
                let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0));
                let resp = ui.interact(icon_rect, ui.id().with("float_icon"), egui::Sense::click());
                let color = crate::widgets::hover::hover_button_color(
                    &resp,
                    crate::theme::text_primary(),
                    false,
                );
                ui.painter().text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon.codepoint,
                    egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
                    color,
                );
                crate::widgets::hint::hover(
                    ctx,
                    &resp,
                    if floating {
                        t!("hint.float_panel_dock")
                    } else {
                        t!("hint.float_panel")
                    },
                );
                if resp.clicked() {
                    Some(if floating {
                        BubbleAction::DockBack
                    } else {
                        BubbleAction::Float
                    })
                } else {
                    None
                }
            });
        over_bubble = area
            .response
            .rect
            .expand(4.0)
            .contains(pointer.unwrap_or(egui::Pos2::ZERO));

        // 点击图标：执行浮动/收回，并关闭气泡。
        if let Some(act) = area.inner {
            match act {
                BubbleAction::Float => sw.float(ctx, target),
                BubbleAction::DockBack => sw.dock_back(ctx, target),
            }
            st = FloatBubbleState::default();
        }

        // 固定态：点击气泡外部（且不是刚右键固定）→ 关闭。
        if st.pinned && !just_pinned && any_click && !over_bubble {
            st = FloatBubbleState::default();
        }
    }

    // ── 指针离开按钮与气泡：宽限后关闭（固定态除外）──
    if st.target.is_some() {
        let over_button = hovered == st.target;
        if st.pinned || over_button || over_bubble {
            st.hide_since = None;
        } else {
            let since = *st.hide_since.get_or_insert(now);
            if now - since >= BUBBLE_HIDE_GRACE {
                st = FloatBubbleState::default();
            }
        }
    }

    ctx.data_mut(|d| d.insert_temp(state_id, st));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：钢琴卷帘、底部设备栏、混音台三选一，展开一个自动收起另两个；
    /// 收起动作不产生互斥副作用。
    #[test]
    fn pianoroll_dock_mixer_are_mutually_exclusive() {
        let (mut piano, mut dock, mut mixer) = (false, true, false);
        set_pianoroll_visible(&mut piano, &mut dock, &mut mixer, true);
        assert!(piano && !dock && !mixer, "打开钢琴应收起 dock 与混音台");

        let (mut piano, mut dock, mut mixer) = (true, false, false);
        set_dock_visible(&mut piano, &mut dock, &mut mixer, true);
        assert!(!piano && dock && !mixer, "打开 dock 应收起钢琴与混音台");

        let (mut piano, mut dock, mut mixer) = (true, false, false);
        set_mixer_visible(&mut piano, &mut dock, &mut mixer, true);
        assert!(!piano && !dock && mixer, "打开混音台应收起钢琴与 dock");

        let (mut piano, mut dock, mut mixer) = (false, true, false);
        set_pianoroll_visible(&mut piano, &mut dock, &mut mixer, false);
        assert!(!piano && dock && !mixer, "收起钢琴不应动 dock");
    }

    /// MIX 的钢琴卷帘显示跟随用户偏好（底部三选一之一），EDIT 恒全屏。
    #[test]
    fn show_pianoroll_by_view_mode() {
        assert!(!ViewMode::Arrange.show_pianoroll(false));
        assert!(ViewMode::Arrange.show_pianoroll(true));
        assert!(!ViewMode::Mix.show_pianoroll(false));
        assert!(ViewMode::Mix.show_pianoroll(true));
        assert!(ViewMode::Edit.show_pianoroll(false));
    }

    /// 浮动气泡状态机：悬停延迟显示；右键固定；Esc 关闭。
    #[test]
    fn float_bubble_hover_delay_and_pin() {
        let ctx = egui::Context::default();
        // 首帧注册图标字体（气泡绘制 material-icons 字形）。
        ctx.run_ui(egui::RawInput::default(), |ui| {
            egui_extras::install_image_loaders(ui.ctx());
            crate::fonts::install(
                ui.ctx(),
                yinhe_fonts::WEIGHT_MEDIUM,
                rust_i18n::locale().as_ref(),
            );
        })
        .drop_without_applying_deltas();
        let (mut sp, mut fp, mut sd, mut fd, mut sm, mut fm) =
            (false, false, false, false, false, false);
        let mut fa = false;
        let mut vm = ViewMode::Arrange;
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 700.0), egui::vec2(20.0, 20.0));
        let mut buttons = vec![PanelButtonInfo {
            target: PanelKind::Pianoroll,
            rect,
            hovered: true,
            secondary_clicked: false,
        }];
        let mut run = |time: f64, events: Vec<egui::Event>, buttons: &[PanelButtonInfo]| {
            let mut input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            input.events = events;
            ctx.run_ui(input, |ui| {
                float_bubble(
                    ui.ctx(),
                    buttons,
                    &mut vm,
                    &mut sp,
                    &mut fp,
                    &mut sd,
                    &mut fd,
                    &mut sm,
                    &mut fm,
                    &mut fa,
                );
            })
            .drop_without_applying_deltas();
        };
        let state = |ctx: &egui::Context| -> FloatBubbleState {
            ctx.data_mut(|d| d.get_temp(egui::Id::new("mode_bar_float_bubble")))
                .unwrap_or_default()
        };

        // 悬停首帧：不立即弹出（延迟计时开始）。
        run(0.0, Vec::new(), &buttons);
        assert_eq!(state(&ctx).target, None, "悬停首帧不应立即弹出");
        // 停留 0.25s 后弹出。
        run(0.3, Vec::new(), &buttons);
        assert_eq!(
            state(&ctx).target,
            Some(PanelKind::Pianoroll),
            "停留 0.25s 后应弹出气泡"
        );
        // 右键固定。
        buttons[0].secondary_clicked = true;
        run(0.4, Vec::new(), &buttons);
        assert!(state(&ctx).pinned, "右键应固定气泡");
        // Esc 关闭。
        buttons[0].secondary_clicked = false;
        run(
            0.5,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
            &buttons,
        );
        assert_eq!(state(&ctx).target, None, "Esc 应关闭气泡");
    }

    /// 回归：气泡水平居中不受窗口在显示器上的位置影响。
    ///
    /// 曾经误用 `inner_rect.min.x`（**显示器坐标**）去 clamp 按钮 rect
    /// （**视口局部坐标**）→ 窗口不在显示器原点时气泡被向右推（位置歪）。
    /// 现在只取 `inner_rect.width()` 做边缘收进，两个坐标系不再混用。
    #[test]
    fn bubble_center_unaffected_by_window_origin() {
        let ctx = egui::Context::default();
        // 首帧注册图标字体（气泡为图标字形）。
        ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().add_font(egui_material_icons::font_insert());
        })
        .drop_without_applying_deltas();

        let (mut sp, mut fp, mut sd, mut fd, mut sm, mut fm) =
            (false, false, false, false, false, false);
        let mut fa = false;
        let mut vm = ViewMode::Arrange;
        // 按钮位于视口局部 x≈100；窗口在显示器 x=300 处（inner_rect 为显示器坐标）。
        let btn_rect = egui::Rect::from_min_size(egui::pos2(90.0, 700.0), egui::vec2(20.0, 20.0));
        let buttons = vec![PanelButtonInfo {
            target: PanelKind::Pianoroll,
            rect: btn_rect,
            hovered: true,
            secondary_clicked: false,
        }];
        let mut run = |time: f64| {
            let mut input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            input.viewports.insert(
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    inner_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(300.0, 50.0),
                        egui::vec2(960.0, 32.0),
                    )),
                    ..Default::default()
                },
            );
            ctx.run_ui(input, |ui| {
                float_bubble(
                    ui.ctx(),
                    &buttons,
                    &mut vm,
                    &mut sp,
                    &mut fp,
                    &mut sd,
                    &mut fd,
                    &mut sm,
                    &mut fm,
                    &mut fa,
                );
            })
            .drop_without_applying_deltas();
        };
        run(0.0);
        run(0.3); // 越过悬停延迟 → 气泡弹出
        let area = ctx
            .memory(|m| m.area_rect(egui::Id::new("mode_bar_float_bubble").with("area")))
            .expect("停留 0.3s 后气泡应已显示");
        assert!(
            (area.center().x - btn_rect.center().x).abs() <= 2.0,
            "气泡应居中于按钮上方: area={area:?} button={btn_rect:?}"
        );
    }

    /// 浮动开关：ARRANGE 泊出只置浮动标志；收回关闭窗口并切回 ARRANGE 模式
    /// （工程走带只在 ARRANGE 模式的主窗口显示）。
    #[test]
    fn float_switches_arrange_roundtrip() {
        let ctx = egui::Context::default();
        let (mut sp, mut fp, mut sd, mut fd, mut sm, mut fm) =
            (false, false, false, false, false, false);
        let mut fa = false;
        let mut vm = ViewMode::Mix;
        let mut sw = FloatSwitches {
            show_pianoroll: &mut sp,
            float_pianoroll: &mut fp,
            show_dock: &mut sd,
            float_dock: &mut fd,
            show_mixer: &mut sm,
            float_mixer: &mut fm,
            float_arrange: &mut fa,
            view_mode: &mut vm,
        };
        assert!(!sw.is_floating(PanelKind::Arrange));
        sw.float(&ctx, PanelKind::Arrange);
        assert!(sw.is_floating(PanelKind::Arrange), "泊出后应为浮动形态");
        assert_eq!(*sw.view_mode, ViewMode::Mix, "泊出不切换模式");
        sw.dock_back(&ctx, PanelKind::Arrange);
        assert!(!sw.is_floating(PanelKind::Arrange), "收回后应关闭浮动");
        assert_eq!(
            *sw.view_mode,
            ViewMode::Arrange,
            "收回工程走带应切回 ARRANGE 模式"
        );
    }

    /// 浮动开关：EDIT 模式下收回钢琴卷帘只关窗口，不动底部面板互斥。
    #[test]
    fn float_switches_pianoroll_dock_back_in_edit_keeps_dock() {
        let ctx = egui::Context::default();
        let (mut sp, mut fp) = (false, true);
        let (mut sd, mut fd) = (true, false);
        let (mut sm, mut fm) = (false, false);
        let mut fa = false;
        let mut vm = ViewMode::Edit;
        let mut sw = FloatSwitches {
            show_pianoroll: &mut sp,
            float_pianoroll: &mut fp,
            show_dock: &mut sd,
            float_dock: &mut fd,
            show_mixer: &mut sm,
            float_mixer: &mut fm,
            float_arrange: &mut fa,
            view_mode: &mut vm,
        };
        sw.dock_back(&ctx, PanelKind::Pianoroll);
        assert!(!fp, "收回后应关闭浮动");
        assert!(sd, "EDIT 下收回卷帘不应收起停靠的设备栏");
        assert!(!sp, "EDIT 下不应启用底部卷帘面板");
    }
}
