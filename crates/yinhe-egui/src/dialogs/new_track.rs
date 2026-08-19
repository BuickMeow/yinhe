//! 新建音轨对话框（标准 viewport 形式）。
//!
//! AR 走带面板「+」按钮触发：arrange.rs 点击后把 OPEN_REQUEST_ID 写进 ctx
//! memory，dialog_dispatch 每帧检测并打开本对话框。通道分配规则全部走
//! yinhe_editor_core::channel_alloc 的纯函数（这里只做 UI 与预览）；确认后由
//! dialog_dispatch 调 Document::add_tracks_batch 落地并 teardown 音频引擎。

use std::sync::Arc;

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::NewTrackSpec;
use yinhe_editor_core::channel_alloc;

/// 打开请求标志：arrange.rs「+」按钮写入，dialog_dispatch 读取后清除。
pub(crate) const OPEN_REQUEST_ID: &str = "new_track_dialog_open";

/// 一次最多创建的音轨数。
const MAX_COUNT: usize = 64;

/// 对话框内的音轨种类选择。音频轨为预留（禁用展示，不可选）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum KindChoice {
    Midi,
    Instrument,
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
    /// 手动起始乐器通道（UI 显示 1 起，这里存的也是显示值）。
    manual_instrument: usize,
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
            manual_instrument: 1,
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

/// 分配方案：将创建的 specs + 预览文本 + 错误提示（有值且 specs 为空 = 禁止确认）。
struct Plan {
    specs: Vec<NewTrackSpec>,
    preview: String,
    error: Option<String>,
}

/// MIDI 通道 badge 文本（如 A01、P16），与轨道面板 badge 同规则。
fn midi_badge(port: u8, channel: u8) -> String {
    format!("{}{:02}", (b'A' + port.min(15)) as char, channel + 1)
}

/// 乐器通道 badge 文本（如 I01），与轨道面板 badge 同规则。
fn instrument_badge(channel0: u16) -> String {
    // saturating：u16 上限 65535 时 +1 不溢出（显示 65536 超出 u16 表达范围）。
    format!("I{:02}", u32::from(channel0) + 1)
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
                    preview: String::new(),
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
                    instrument_channel: None,
                })
                .collect::<Vec<_>>();
            let preview = alloc
                .iter()
                .map(|&(p, c)| midi_badge(p, c))
                .collect::<Vec<_>>()
                .join(", ");
            // 超出 P16 截断：提示实际创建数量（截断后仍可确认）。
            let error = if alloc.len() < state.count {
                Some(t!("dialog.new_track.truncated", n = alloc.len()).to_string())
            } else {
                None
            };
            Plan {
                specs,
                preview,
                error,
            }
        }
        KindChoice::Instrument => {
            let start = match state.mode {
                AssignMode::Auto => channel_alloc::auto_instrument_channel_start(tracks),
                AssignMode::Manual => state.manual_instrument.saturating_sub(1) as u16,
            };
            let specs = (0..state.count)
                .map(|i| NewTrackSpec {
                    kind: yinhe_core::TrackKind::Instrument,
                    port: 0,
                    channel: 0,
                    // 大起点 + 大数量时饱和，不回绕。
                    instrument_channel: Some(start.saturating_add(i as u16)),
                })
                .collect::<Vec<_>>();
            let preview = specs
                .iter()
                .filter_map(|s| s.instrument_channel)
                .map(instrument_badge)
                .collect::<Vec<_>>()
                .join(", ");
            Plan {
                specs,
                preview,
                error: None,
            }
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
            [400.0, 320.0],
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
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                36.0,
                                |ui| {
                                    ui.add_space(6.0);
                                    // 种类：MIDI 轨 / 乐器轨 / 音频轨（预留，禁用）
                                    ui.horizontal(|ui| {
                                        ui.label(t!("dialog.new_track.kind").as_ref());
                                        ui.selectable_value(
                                            &mut state.kind,
                                            KindChoice::Midi,
                                            t!("dialog.new_track.kind.midi").as_ref(),
                                        );
                                        ui.selectable_value(
                                            &mut state.kind,
                                            KindChoice::Instrument,
                                            t!("dialog.new_track.kind.instrument").as_ref(),
                                        );
                                        ui.add_enabled_ui(false, |ui| {
                                            let _ = ui.selectable_label(
                                                false,
                                                t!("dialog.new_track.kind.audio").as_ref(),
                                            );
                                        });
                                    });

                                    // 数量：1..=64
                                    ui.horizontal(|ui| {
                                        ui.label(t!("dialog.new_track.count").as_ref());
                                        ui.add(
                                            crate::widgets::numeric_input::decimal_drag_value(
                                                &mut state.count,
                                            )
                                            .range(1..=MAX_COUNT),
                                        );
                                    });

                                    // 通道分配：自动 / 指定起点
                                    ui.horizontal(|ui| {
                                        ui.label(t!("dialog.new_track.assign").as_ref());
                                        ui.selectable_value(
                                            &mut state.mode,
                                            AssignMode::Auto,
                                            t!("dialog.new_track.assign.auto").as_ref(),
                                        );
                                        ui.selectable_value(
                                            &mut state.mode,
                                            AssignMode::Manual,
                                            t!("dialog.new_track.assign.manual").as_ref(),
                                        );
                                    });

                                    // 手动起点输入
                                    if state.mode == AssignMode::Manual {
                                        match state.kind {
                                            KindChoice::Midi => {
                                                ui.horizontal(|ui| {
                                                    ui.label(t!("dialog.new_track.port").as_ref());
                                                    egui::ComboBox::from_id_salt("new_track_port")
                                                        .selected_text(
                                                            ((b'A' + state.manual_port.min(15))
                                                                as char)
                                                                .to_string(),
                                                        )
                                                        .show_ui(ui, |ui| {
                                                            for p in 0..16u8 {
                                                                ui.selectable_value(
                                                                    &mut state.manual_port,
                                                                    p,
                                                                    ((b'A' + p) as char)
                                                                        .to_string(),
                                                                );
                                                            }
                                                        });
                                                    ui.label(
                                                        t!("dialog.new_track.channel").as_ref(),
                                                    );
                                                    egui::ComboBox::from_id_salt(
                                                        "new_track_channel",
                                                    )
                                                    .selected_text(format!(
                                                        "{}",
                                                        state.manual_channel + 1
                                                    ))
                                                    .show_ui(ui, |ui| {
                                                        for c in 0..16u8 {
                                                            ui.selectable_value(
                                                                &mut state.manual_channel,
                                                                c,
                                                                format!("{}", c + 1),
                                                            );
                                                        }
                                                    });
                                                });
                                            }
                                            KindChoice::Instrument => {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        t!("dialog.new_track.instrument_start")
                                                            .as_ref(),
                                                    );
                                                    ui.add(
                                                        crate::widgets::numeric_input::decimal_drag_value(
                                                            &mut state.manual_instrument,
                                                        )
                                                        .range(1..=u16::MAX as usize + 1),
                                                    );
                                                });
                                            }
                                        }
                                    }

                                    // 预览将分配的通道序列（同帧反映上面的输入）
                                    let plan = plan(state, tracks);
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new(
                                            t!(
                                                "dialog.new_track.preview",
                                                channels = plan.preview.as_str()
                                            )
                                            .as_ref(),
                                        )
                                        .color(crate::theme::text_label())
                                        .size(crate::theme::SMALL_FONT),
                                    );
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
                                    // 内容区同帧已算好方案，直接取用（不碰 state）。
                                    let mut plan_cell = plan_rc.borrow_mut();
                                    let can_confirm = plan_cell
                                        .as_ref()
                                        .is_some_and(|p| !p.specs.is_empty());
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
                                        // 通道全满（specs 为空）时禁止确认。
                                        if ui
                                            .add_enabled(
                                                can_confirm,
                                                egui::Button::new(t!("common.confirm").as_ref()),
                                            )
                                            .clicked()
                                            && let Some(p) = plan_cell.take()
                                        {
                                            *action_cb.borrow_mut() =
                                                Some(NewTrackAction::Confirm(p.specs));
                                            close = true;
                                        }
                                        ui.add_space(4.0);
                                        if ui.button(t!("common.cancel").as_ref()).clicked() {
                                            *action_cb.borrow_mut() = Some(NewTrackAction::Cancel);
                                            close = true;
                                        }
                                    });
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
