pub mod channels_panel;
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
    Project,
    Channels,
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
/// needs to be reloaded (soundfont config changed).
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
) -> (bool, Option<event_browser::JumpRequest>) {
    let tab = *right_tab;
    if tab.is_none() {
        return (false, None);
    }

    let theme = crate::theme::RIGHT_PANEL_MIN_WIDTH;
    let total_avail = ui.available_rect_before_wrap().width();
    let max_w = (total_avail - 60.0).max(theme + 4.0);
    let clamp_w = (*right_panel_width + 4.0).clamp(theme + 4.0, max_w);
    *right_panel_width = (clamp_w - 4.0).max(theme);

    // ── Split handle (4px at the left edge) ──
    let handle_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y),
        egui::pos2(rect.min.x + 4.0, rect.max.y),
    );
    let resp = crate::widgets::split_handle::vertical(ui, "__right_split__", handle_rect);
    if resp.dragged() {
        // Handle is at the left edge of a right-aligned panel.
        // Dragging right → panel narrows (width decreases).
        *right_panel_width = (*right_panel_width - resp.drag_delta().x).clamp(theme, max_w - 4.0);
    }

    // ── Panel content area (8px left/right padding, after the handle) ──
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 4.0 + 8.0, rect.min.y),
        egui::pos2(rect.max.x - 8.0, rect.max.y),
    );

    let mut changed = false;
    let mut jump_request: Option<event_browser::JumpRequest> = None;

    ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
        ui.set_clip_rect(content_rect);

        // Background
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, crate::theme::APP_BG);

        // ── Content ──
        if let Some(tab) = tab {
            match tab {
                RightTab::Info => {
                    changed |= info_panel::show(ui, doc, audio, info_content, automation_drag_ghost);
                }
                RightTab::SoundFont => {
                    changed |= soundfont::show(ui, audio_settings, doc);
                }
                RightTab::Project => {
                    project_info::show(ui, doc);
                }
                RightTab::Channels => {
                    channels_panel::show(ui, doc, audio_settings);
                }
                RightTab::EventBrowser => {
                    jump_request = event_browser::show(ui, doc, event_browser_state);
                }
            }
        }
    });

    (changed, jump_request)
}
