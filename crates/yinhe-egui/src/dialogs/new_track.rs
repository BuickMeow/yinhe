//! 新建音轨对话框（标准 viewport 形式）。
//!
//! AR 走带面板「+」按钮触发：arrange.rs 点击后把 OPEN_REQUEST_ID 写进 ctx
//! memory，dialog_dispatch 每帧检测并打开本对话框。通道分配规则全部走
//! yinhe_editor_core::channel_alloc 的纯函数（这里只做 UI 与预览）；确认后由
//! dialog_dispatch 调 Document::add_tracks_batch 落地并 teardown 音频引擎。

use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::{ICON_GRAPHIC_EQ, ICON_PIANO};
use rust_i18n::t;

use yinhe_editor_core::NewTrackSpec;
use yinhe_editor_core::channel_alloc;

/// 打开请求标志：arrange.rs「+」按钮写入，dialog_dispatch 读取后清除。
pub(crate) const OPEN_REQUEST_ID: &str = "new_track_dialog_open";

/// 一次最多创建的音轨数。
const MAX_COUNT: usize = 64;

/// 对话框内的音轨种类选择。
#[derive(Clone, Copy, PartialEq, Eq)]
enum KindChoice {
    Midi,
    Audio,
}

/// 通道分配方式：自动（顺延既有最大通道）或指定起点向后顺延。
#[derive(Clone, Copy, PartialEq, Eq)]
enum AssignMode {
    Auto,
    Manual,
}

/// 对话框持久状态（挂在 App 上，跨帧保留；每次打开时重置为默认）。
pub(crate) struct NewTrackDialogState {
    pub open: bool,
    kind: KindChoice,
    /// 一次创建的条数（1..=MAX_COUNT）。
    count: usize,
    mode: AssignMode,
    /// 手动起点 port（0 起，UI 显示 A..P）。
    manual_port: u8,
    /// 手动起点 channel（0 起，UI 显示 1..16）。
    manual_channel: u8,
    /// 手动起始音频通道（UI 显示 1 起，这里存的也是显示值）。
    manual_audio: usize,
}

impl Default for NewTrackDialogState {
    fn default() -> Self {
        Self {
            open: false,
            kind: KindChoice::Midi,
            count: 1,
            mode: AssignMode::Auto,
            manual_port: 0,
            manual_channel: 0,
            manual_audio: 1,
        }
    }
}

impl NewTrackDialogState {
    /// 打开对话框并重置为默认值（数量 1、自动分配），
    /// 避免残留上次的大数量误建。
    pub(crate) fn open(&mut self) {
        *self = Self {
            open: true,
            ..Self::default()
        };
    }
}

/// 用户操作结果。
pub(crate) enum NewTrackAction {
    /// 用户还没做出选择（窗口仍打开）。
    None,
    /// 确认：按 specs 批量创建。
    Confirm(Vec<NewTrackSpec>),
    /// 取消（含点窗口关闭按钮）。
    Cancel,
}

/// 轨道种类卡片：图标 + 名称；选中用强调色描边，hover 只变背景
///（项目规范：按钮不带边框，避免悬停时边框出现导致内容视觉位移）。
fn kind_card(
    ui: &mut egui::Ui,
    icon: &str,
    family: egui::FontFamily,
    label: &str,
    selected: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(116.0, 64.0), egui::Sense::click());
    let bg = if selected {
        crate::theme::selected_bg()
    } else if resp.hovered() {
        crate::theme::hover_color(crate::theme::btn_bg())
    } else {
        crate::theme::btn_bg()
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, bg);
    if selected {
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.2, crate::theme::accent_active()),
            egui::StrokeKind::Inside,
        );
    }
    let icon_color = if selected {
        crate::theme::accent_active()
    } else {
        crate::theme::text_primary()
    };
    painter.text(
        egui::pos2(rect.center().x, rect.min.y + 22.0),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::new(22.0, family),
        icon_color,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.min.y + 47.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
        crate::theme::text_primary(),
    );
    resp.clicked()
}

/// 分配方案：将创建的 specs + 错误提示（有值且 specs 为空 = 禁止确认）。
struct Plan {
    specs: Vec<NewTrackSpec>,
    error: Option<String>,
}

/// 根据当前状态计算分配方案（预览/提示/确认共用同一规则）。
fn plan(state: &NewTrackDialogState, tracks: &[Arc<yinhe_core::TrackData>]) -> Plan {
    match state.kind {
        KindChoice::Midi => {
            let start = match state.mode {
                AssignMode::Auto => channel_alloc::auto_midi_channel_start(tracks),
                AssignMode::Manual => {
                    Some(u16::from(state.manual_port) * 16 + u16::from(state.manual_channel))
                }
            };
            let Some(start) = start else {
                // 256 通道（A1..P16）全满：禁止确认。
                return Plan {
                    specs: Vec::new(),
                    error: Some(t!("dialog.new_track.full").to_string()),
                };
            };
            let alloc = channel_alloc::alloc_channels_from(start, state.count);
            let specs = alloc
                .iter()
                .map(|&(port, channel)| NewTrackSpec {
                    kind: yinhe_core::TrackKind::Midi,
                    port,
                    channel,
                    audio_channel: None,
                })
                .collect::<Vec<_>>();
            // 超出 P16 截断：提示实际创建数量（截断后仍可确认）。
            let error = if alloc.len() < state.count {
                Some(t!("dialog.new_track.truncated", n = alloc.len()).to_string())
            } else {
                None
            };
            Plan { specs, error }
        }
        KindChoice::Audio => {
            let start = match state.mode {
                AssignMode::Auto => channel_alloc::auto_audio_channel_start(tracks),
                AssignMode::Manual => state.manual_audio.saturating_sub(1) as u16,
            };
            let specs = (0..state.count)
                .map(|i| NewTrackSpec {
                    kind: yinhe_core::TrackKind::Audio,
                    port: 0,
                    channel: 0,
                    audio_channel: Some(start.saturating_add(i as u16)),
                })
                .collect::<Vec<_>>();
            Plan { specs, error: None }
        }
    }
}

/// 显示新建音轨对话框。返回 NewTrackAction::None 表示用户还没选择。
pub(crate) fn show_viewport(
    ctx: &egui::Context,
    state: &mut NewTrackDialogState,
    tracks: &[Arc<yinhe_core::TrackData>],
) -> NewTrackAction {
    let viewport_id = egui::ViewportId::from_hash_of("new_track_dialog");

    let action_rc: std::rc::Rc<std::cell::RefCell<Option<NewTrackAction>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let action_cb = action_rc.clone();
    let ctx_clone = ctx.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.new_track.title").as_ref(),
            [420.0, 470.0],
            false,
        ),
        move |vctx, _class| {
            let mut close = false;
            if vctx.input(|i| i.viewport().close_requested()) {
                *action_cb.borrow_mut() = Some(NewTrackAction::Cancel);
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.new_track.title").as_ref(),
                        &mut close,
                        false,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(376.0);
                            // 分配方案：内容区算好后经 Rc cell 传给底部按钮区
                            // （两个闭包不能同时借 state，按钮区只读方案不读 state）。
                            let plan_rc: std::rc::Rc<std::cell::RefCell<Option<Plan>>> =
                                std::rc::Rc::new(std::cell::RefCell::new(None));
                            let plan_cb = plan_rc.clone();
                            let btn_zone_h = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                btn_zone_h,
                                |ui| {
                                    // 轨道种类：三张卡片（图标 + 名称），点击选中。
                                    ui.add_space(2.0);
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        if kind_card(
                                            ui,
                                            ICON_PIANO.codepoint,
                                            ICON_PIANO.font_family(),
                                            t!("dialog.new_track.kind.midi").as_ref(),
                                            state.kind == KindChoice::Midi,
                                        ) {
                                            state.kind = KindChoice::Midi;
                                        }
                                        if kind_card(
                                            ui,
                                            ICON_GRAPHIC_EQ.codepoint,
                                            ICON_GRAPHIC_EQ.font_family(),
                                            t!("dialog.new_track.kind.audio").as_ref(),
                                            state.kind == KindChoice::Audio,
                                        ) {
                                            state.kind = KindChoice::Audio;
                                        }
                                    });
                                    ui.add_space(10.0);

                                    // 数量：1..=64
                                    crate::dialogs::settings::setting_row(
                                        ui,
                                        t!("dialog.new_track.count").as_ref(),
                                        "",
                                        |ui| {
                                            ui.add(
                                                crate::widgets::numeric_input::decimal_drag_value(
                                                    &mut state.count,
                                                )
                                                .range(1..=MAX_COUNT),
                                            );
                                        },
                                    );

                                    // 通道分配：自动 / 指定起点
                                    crate::dialogs::settings::setting_row(
                                        ui,
                                        t!("dialog.new_track.assign").as_ref(),
                                        "",
                                        |ui| {
                                            crate::widgets::flat::flat_selectable_value(
                                            ui,
                                                &mut state.mode,
                                                AssignMode::Auto,
                                                t!("dialog.new_track.assign.auto").as_ref(),
                                            );
                                            crate::widgets::flat::flat_selectable_value(
                                            ui,
                                                &mut state.mode,
                                                AssignMode::Manual,
                                                t!("dialog.new_track.assign.manual").as_ref(),
                                            );
                                        },
                                    );

                                    // 手动起点输入
                                    if state.mode == AssignMode::Manual {
                                        match state.kind {
                                            KindChoice::Midi => {
                                                crate::dialogs::settings::setting_row(
                                                    ui,
                                                    t!("dialog.new_track.port").as_ref(),
                                                    "",
                                                    |ui| {
                                                        crate::widgets::combo::combo_box(
                                                            ui,
                                                            "new_track_port",
                                                            ((b'A'
                                                                + state.manual_port.min(15))
                                                                as char)
                                                                .to_string(),
                                                            100.0,
                                                            |ui| {
                                                                for p in 0..16u8 {
                                                                    if crate::widgets::combo::combo_item(
                                                                        ui,
                                                                        state.manual_port == p,
                                                                        ((b'A' + p) as char)
                                                                            .to_string(),
                                                                    )
                                                                    .clicked()
                                                                    {
                                                                        state.manual_port = p;
                                                                    }
                                                                }
                                                            },
                                                        );
                                                    },
                                                );
                                                crate::dialogs::settings::setting_row(
                                                    ui,
                                                    t!("dialog.new_track.channel").as_ref(),
                                                    "",
                                                    |ui| {
                                                        crate::widgets::combo::combo_box(
                                                            ui,
                                                            "new_track_channel",
                                                            format!(
                                                                "{}",
                                                                state.manual_channel + 1
                                                            ),
                                                            100.0,
                                                            |ui| {
                                                                for c in 0..16u8 {
                                                                    if crate::widgets::combo::combo_item(
                                                                        ui,
                                                                        state.manual_channel == c,
                                                                        format!("{}", c + 1),
                                                                    )
                                                                    .clicked()
                                                                    {
                                                                        state.manual_channel = c;
                                                                    }
                                                                }
                                                            },
                                                        );
                                                    },
                                                );
                                            }
                                            KindChoice::Audio => {
                                                crate::dialogs::settings::setting_row(
                                                    ui,
                                                    t!("dialog.new_track.audio_start").as_ref(),
                                                    "",
                                                    |ui| {
                                                        ui.add(
                                                            crate::widgets::numeric_input::decimal_drag_value(
                                                                &mut state.manual_audio,
                                                            )
                                                            .range(1..=u16::MAX as usize + 1),
                                                        );
                                                    },
                                                );
                                            }
                                        }
                                    }

                                    let plan = plan(state, tracks);
                                    if let Some(err) = &plan.error {
                                        ui.label(
                                            egui::RichText::new(err)
                                                .color(crate::theme::danger())
                                                .size(crate::theme::SMALL_FONT),
                                        );
                                    }
                                    *plan_cb.borrow_mut() = Some(plan);
                                },
                                |ui| {
                                    use crate::chrome::dialog_buttons::{DialogButton, dialog_button_row};
                                    // 内容区同帧已算好方案，直接取用（不碰 state）。
                                    let mut plan_cell = plan_rc.borrow_mut();
                                    let can_confirm = plan_cell
                                        .as_ref()
                                        .is_some_and(|p| !p.specs.is_empty());
                                    ui.add_space(8.0);
                                    let cancel = t!("common.cancel");
                                    let confirm = t!("common.confirm");
                                    if let Some(idx) = dialog_button_row(
                                        ui,
                                        &[
                                            DialogButton::secondary(cancel.as_ref()),
                                            DialogButton::primary(confirm.as_ref())
                                                .enabled(can_confirm),
                                        ],
                                    ) {
                                        if idx == 0 {
                                            *action_cb.borrow_mut() =
                                                Some(NewTrackAction::Cancel);
                                            close = true;
                                        } else if let Some(p) = plan_cell.take() {
                                            *action_cb.borrow_mut() =
                                                Some(NewTrackAction::Confirm(p.specs));
                                            close = true;
                                        }
                                    }
                                },
                            );
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );

    action_rc
        .borrow_mut()
        .take()
        .unwrap_or(NewTrackAction::None)
}
