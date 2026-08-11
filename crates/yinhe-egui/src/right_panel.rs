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
/// Called inside an `egui::Panel::right` (see `App::show_right_panel`), which
/// owns the resize handle and width state. Returns `true` if the audio engine
/// needs to be reloaded (soundfont config changed).
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn show(
    ui: &mut egui::Ui,
    right_tab: &mut Option<RightTab>,
    audio_settings: &mut AudioSettings,
    doc: Option<&mut Document>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    event_browser_state: &mut event_browser::EventBrowserState,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
    status_hint: &mut Option<String>,
) -> (bool, Option<event_browser::JumpRequest>) {
    let tab = *right_tab;

    // 状态栏讲解行：鼠标在右面板上时清空（右面板不属于可讲解区域）
    if ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| ui.max_rect().contains(p))
    }) {
        *status_hint = None;
    }

    let mut changed = false;
    let mut jump_request: Option<event_browser::JumpRequest> = None;

    // ── Content ──
    if let Some(tab) = tab {
        match tab {
            RightTab::Info => {
                changed |= info_panel::show(ui, doc, audio, info_content, automation_drag_ghost);
            }
            RightTab::SoundFont => {
                changed |= soundfont::show(ui, audio_settings, doc);
            }
            RightTab::EventBrowser => {
                jump_request = event_browser::show(ui, doc, event_browser_state);
            }
        }
    }

    (changed, jump_request)
}
