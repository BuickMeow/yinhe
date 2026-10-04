pub mod automation_undo;
pub mod dock;
pub mod event_browser;
pub mod info_panel;
pub mod project_info;
pub mod sf_list;

use eframe::egui;

use yinhe_editor_core::audio_settings::LayoutSettings;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::right_panel_layout::{PanelKind, RightPanelLayout};
use yinhe_types::AutomationTarget;

/// 兼容旧接口：哪一类内容需要展示（用于右键/快捷键定位到对应选项卡）。
/// 新的多栏布局里，它映射到 `PanelKind` 并 focus 对应标签。
#[derive(PartialEq, Clone, Copy)]
pub enum RightTab {
    Info,
    EventBrowser,
}

/// 工程设置独立窗口的打开状态（浮动 viewport，保留为独立窗口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatPanel {
    /// 工程设置浮窗（唯一入口）。
    ProjectSettings,
}

/// 信息面板中展示的内容类型（多合一设计）。
#[derive(Clone, Debug)]
pub enum InfoContent {
    /// 选中的自动化锚点，通过 event_idx 在 lane.events 中的索引定位。
    /// value/tick/shape 从模型实时读取，锚点移动/undo 后索引仍能跟踪。
    Anchor {
        track_idx: u16,
        lane_idx: usize,
        event_idx: usize,
        target: AutomationTarget,
    },
    /// 选中的音轨（由 doc.edit.track_selected 决定哪些音轨）
    Track,
}

/// Render the right panel (if a tab is active).
///
/// `rect` is the full area reserved for the right panel, including a 4px
/// split-handle strip at its left edge.  Returns `true` if the audio engine
/// needs to be reloaded (soundfont config changed), plus whether the width
/// drag just ended this frame (layout settings persist trigger).
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn show(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    right_panel_width: &mut f32,
    right_tab: &mut Option<RightTab>,
    layout: &mut RightPanelLayout,
    mut doc: Option<&mut Document>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    event_browser_state: &mut event_browser::EventBrowserState,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
    status_hint: &mut Option<String>,
) -> (bool, Option<event_browser::JumpRequest>, bool) {
    let Some(tab) = *right_tab else {
        return (false, None, false);
    };
    // 旧入口（右键/快捷键 focus Info/EventBrowser）→ 仅在切换的那一帧定位选项卡，
    // 避免每帧重置用户手动选择的选项卡。
    let focus_id = ui.id().with("rpanel_focus_hint");
    let last: Option<RightTab> = ui.data_mut(|d| d.get_temp(focus_id));
    if last != Some(tab) {
        match tab {
            RightTab::Info => layout.focus_tab(PanelKind::Track),
            RightTab::EventBrowser => layout.focus_tab(PanelKind::EventBrowser),
        }
        ui.data_mut(|d| d.insert_temp(focus_id, tab));
    }

    // 状态栏讲解行：鼠标在右面板上时清空（右面板不属于可讲解区域）
    if ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p))) {
        *status_hint = None;
    }

    let theme = crate::theme::RIGHT_PANEL_MIN_WIDTH;
    // 宽度 clamp 由 App::right_panel_total_width 在布局时统一处理；这里只算拖拽上限。
    // 基准取窗口宽（不能取 ui.available：右栏占位 Panel 已扣掉右栏宽度）。
    let max_w = (ui.ctx().viewport_rect().width() - 60.0).max(theme + 4.0);

    // ── Split handle (SPLIT_HANDLE_W at the left edge) ──
    let handle_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y),
        egui::pos2(rect.min.x + crate::theme::SPLIT_HANDLE_W, rect.max.y),
    );
    let resp = crate::widgets::split_handle::vertical(ui, "__right_split__", handle_rect);
    let width_drag_ended = resp.drag_stopped() || resp.double_clicked();
    if resp.dragged() {
        // Handle is at the left edge of a right-aligned panel.
        // Dragging right → panel narrows (width decreases).
        *right_panel_width = (*right_panel_width - resp.drag_delta().x)
            .clamp(theme, max_w - crate::theme::SPLIT_HANDLE_W);
    }
    if resp.double_clicked() {
        // 双击分割线 → 还原右侧栏默认宽度
        *right_panel_width = LayoutSettings::default().right_panel_width;
    }

    // ── Panel content area: full width after the split handle ──
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + crate::theme::SPLIT_HANDLE_W, rect.min.y),
        egui::pos2(rect.max.x, rect.max.y),
    );

    let mut port_changed = false;
    let mut jump_request: Option<event_browser::JumpRequest> = None;

    // 内容区左右收缩 8px，避免文字贴边。
    let inner = egui::Rect::from_min_max(
        egui::pos2(content_rect.min.x + 8.0, content_rect.min.y),
        egui::pos2(content_rect.max.x - 8.0, content_rect.max.y),
    );

    let mut layout_changed = false;
    ui.painter()
        .rect_filled(content_rect, 0.0, crate::theme::app_bg());

    let doc_ref = &mut doc;
    layout_changed |= dock::show(ui, inner, layout, |ui, kind, _content| {
        match kind {
            PanelKind::Track => {
                port_changed |= info_panel::show_track(
                    ui,
                    doc_ref.as_deref_mut(),
                    info_content,
                    automation_drag_ghost,
                );
            }
            PanelKind::ProjectTree => {
                event_browser::show_tree(ui, doc_ref.as_deref_mut(), event_browser_state);
            }
            PanelKind::EventBrowser => {
                jump_request =
                    event_browser::show_events(ui, doc_ref.as_deref_mut(), event_browser_state);
            }
            PanelKind::History => {
                if let Some(doc) = doc_ref.as_deref_mut() {
                    info_panel::show_history(ui, doc);
                }
            }
            PanelKind::Summary => {
                if let Some(doc) = doc_ref.as_deref_mut() {
                    info_panel::show_summary(ui, doc);
                }
            }
        }
        let _ = audio;
    });

    (
        port_changed,
        jump_request,
        width_drag_ended || layout_changed,
    )
}
