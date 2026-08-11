pub mod automation_undo;
pub mod event_browser;
pub mod info_panel;
pub mod project_info;
pub mod sf_list;
pub mod soundfont;

use eframe::egui;

use crate::audio_settings::AudioSettings;
use yinhe_editor_core::document::Document;
use yinhe_types::AutomationTarget;

#[derive(PartialEq, Clone, Copy)]
pub enum RightTab {
    Info,
    SoundFont,
    EventBrowser,
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
    audio_settings: &mut AudioSettings,
    doc: Option<&mut Document>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    event_browser_state: &mut event_browser::EventBrowserState,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
    status_hint: &mut Option<String>,
) -> (bool, Option<event_browser::JumpRequest>, bool) {
    let tab = *right_tab;
    if tab.is_none() {
        return (false, None, false);
    }

    // 状态栏讲解行：鼠标在右面板上时清空（右面板不属于可讲解区域）
    if ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p))) {
        *status_hint = None;
    }

    let theme = crate::theme::RIGHT_PANEL_MIN_WIDTH;
    let total_avail = ui.available_rect_before_wrap().width();
    let max_w = (total_avail - 60.0).max(theme + 4.0);
    let clamp_w = (*right_panel_width + 4.0).clamp(theme + 4.0, max_w);
    *right_panel_width = (clamp_w - 4.0).max(theme);

    // ── Split handle (SPLIT_HANDLE_W at the left edge) ──
    let handle_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y),
        egui::pos2(rect.min.x + crate::theme::SPLIT_HANDLE_W, rect.max.y),
    );
    let resp = crate::widgets::split_handle::vertical(ui, "__right_split__", handle_rect);
    let width_drag_ended = resp.drag_stopped();
    if resp.dragged() {
        // Handle is at the left edge of a right-aligned panel.
        // Dragging right → panel narrows (width decreases).
        *right_panel_width = (*right_panel_width - resp.drag_delta().x)
            .clamp(theme, max_w - crate::theme::SPLIT_HANDLE_W);
    }

    // ── Panel content area: full width after the split handle ──
    // 背景铺满整个面板（不再往内收缩，避免两侧 0 层缝隙）；
    // 文字等内容由下方统一收缩 8px，各 tab 内部可再调整。
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + crate::theme::SPLIT_HANDLE_W, rect.min.y),
        egui::pos2(rect.max.x, rect.max.y),
    );

    let mut changed = false;
    let mut jump_request: Option<event_browser::JumpRequest> = None;

    ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
        ui.set_clip_rect(content_rect);

        // Background
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, crate::theme::app_bg());

        // 内容区收缩 8px（左右），避免文字贴边
        let inner = egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x + 8.0, content_rect.min.y),
            egui::pos2(content_rect.max.x - 8.0, content_rect.max.y),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);

            // ── Content ──
            if let Some(tab) = tab {
                match tab {
                    RightTab::Info => {
                        changed |=
                            info_panel::show(ui, doc, audio, info_content, automation_drag_ghost);
                    }
                    RightTab::SoundFont => {
                        changed |= soundfont::show(ui, audio_settings, doc);
                    }
                    RightTab::EventBrowser => {
                        jump_request = event_browser::show(ui, doc, event_browser_state);
                    }
                }
            }
        });
    });

    (changed, jump_request, width_drag_ended)
}
