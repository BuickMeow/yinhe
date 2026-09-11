//! 敲击测速（Tap Tempo）对话框（标准 viewport 形式）。
//!
//! 播放菜单「敲击测速」触发：独立 viewport 内点击敲击区或按空格键敲击，
//! 根据敲击间隔平均值估算 BPM。仅用于测量显示，暂不写入工程。
//!
//! 空格键冲突说明：主窗口的空格是「播放/暂停」，本对话框是独立 OS 窗口，
//! 键盘事件只在聚焦窗口内分发，因此弹窗内空格不会触发主窗口播放。

use eframe::egui;
use rust_i18n::t;

/// 停顿超过该秒数后再敲击视为重新开始一轮测量。
const TAP_RESET_SECS: f64 = 2.0;

/// 对话框状态（挂在 App 上，跨帧保留；打开时重置）。
#[derive(Default)]
pub(crate) struct TapTempoDialogState {
    pub open: bool,
    /// 本轮敲击的时间戳（egui 时间，秒）。
    taps: Vec<f64>,
}

impl TapTempoDialogState {
    /// 打开对话框并开始新一轮测量。
    pub(crate) fn open(&mut self) {
        self.open = true;
        self.taps.clear();
    }
}

/// 由敲击时间戳估算 BPM：所有间隔的平均值。
///
/// 少于 2 次敲击或时间跨度非正（同一帧重复敲击）时返回 `None`。
fn bpm_from_taps(taps: &[f64]) -> Option<f32> {
    if taps.len() < 2 {
        return None;
    }
    let span = taps[taps.len() - 1] - taps[0];
    if span <= 0.0 {
        return None;
    }
    Some((60.0 * (taps.len() - 1) as f64 / span) as f32)
}

/// 记录一次敲击：距上次敲击超过 [`TAP_RESET_SECS`] 时清空重来。
fn register_tap(taps: &mut Vec<f64>, now: f64) {
    if taps.last().is_some_and(|last| now - last > TAP_RESET_SECS) {
        taps.clear();
    }
    taps.push(now);
}

/// 显示敲击测速对话框。返回 `true` 表示用户已关闭窗口。
pub(crate) fn show_viewport(ctx: &egui::Context, state: &mut TapTempoDialogState) -> bool {
    let viewport_id = egui::ViewportId::from_hash_of("tap_tempo_dialog");
    let title = t!("dialog.tap_tempo.title");
    let mut closed = false;

    ctx.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(title.as_ref(), [320.0, 286.0], false),
        |vctx, _class| {
            if vctx.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
            let space_pressed = vctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Space,
                            pressed: true,
                            repeat: false,
                            ..
                        }
                    )
                })
            });
            let now = vctx.input(|i| i.time);
            let mut close = closed;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::app_bg(),
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut close);
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            let bpm = bpm_from_taps(&state.taps);

                            let mut tapped = space_pressed;
                            let mut reset = false;
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                36.0,
                                |ui| {
                                    ui.add_space(6.0);
                                    let bpm_text = bpm
                                        .map(yinhe_types::time_format::format_bpm)
                                        .unwrap_or_else(|| "--".to_string());
                                    ui.label(
                                        egui::RichText::new(bpm_text)
                                            .size(42.0)
                                            .color(crate::theme::text_bright()),
                                    );
                                    ui.add_space(2.0);
                                    ui.label(
                                        egui::RichText::new("BPM")
                                            .size(crate::theme::SMALL_FONT)
                                            .color(crate::theme::text_label()),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new(t!(
                                            "dialog.tap_tempo.taps",
                                            n = state.taps.len()
                                        ))
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_muted()),
                                    );
                                    ui.add_space(10.0);
                                    let size = egui::vec2(ui.available_width().min(256.0), 64.0);
                                    let (rect, resp) =
                                        ui.allocate_exact_size(size, egui::Sense::CLICK);
                                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                                    let bg = if resp.is_pointer_button_down_on() {
                                        crate::theme::control_selected_bg()
                                    } else if resp.hovered() {
                                        crate::theme::hover_color(crate::theme::control_bg())
                                    } else {
                                        crate::theme::control_bg()
                                    };
                                    let painter = ui.painter();
                                    painter.rect_filled(rect, 8.0, bg);
                                    painter.rect_stroke(
                                        rect,
                                        8.0,
                                        egui::Stroke::new(1.0, crate::theme::line_fg()),
                                        egui::StrokeKind::Inside,
                                    );
                                    painter.text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        t!("dialog.tap_tempo.tap"),
                                        egui::FontId::proportional(16.0),
                                        crate::theme::text_bright(),
                                    );
                                    if resp.clicked() {
                                        tapped = true;
                                    }
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new(t!("dialog.tap_tempo.hint"))
                                            .size(crate::theme::SMALL_FONT)
                                            .color(crate::theme::text_muted()),
                                    );
                                },
                                |ui| {
                                    ui.add_space(4.0);
                                    let reset_btn =
                                        egui::Button::new(t!("dialog.tap_tempo.reset").as_ref())
                                            .sense(egui::Sense::CLICK);
                                    if ui.add(reset_btn).clicked() {
                                        reset = true;
                                    }
                                },
                            );

                            if reset {
                                state.taps.clear();
                            } else if tapped {
                                register_tap(&mut state.taps, now);
                            }
                        });
                });
            if close {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                closed = true;
            }
        },
    );

    closed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_none_with_fewer_than_two_taps() {
        assert_eq!(bpm_from_taps(&[]), None);
        assert_eq!(bpm_from_taps(&[1.0]), None);
    }

    #[test]
    fn bpm_none_when_span_not_positive() {
        assert_eq!(bpm_from_taps(&[2.0, 2.0]), None);
    }

    #[test]
    fn bpm_averages_intervals() {
        // 0.5s 间隔 → 120 BPM
        let bpm = bpm_from_taps(&[0.0, 0.5, 1.0]).unwrap();
        assert!((bpm - 120.0).abs() < 1e-3);
        // 0.6s 间隔 → 100 BPM
        let bpm = bpm_from_taps(&[1.0, 1.6, 2.2, 2.8]).unwrap();
        assert!((bpm - 100.0).abs() < 1e-3);
    }

    #[test]
    fn tap_resets_after_long_pause() {
        let mut taps = vec![0.0, 0.5];
        register_tap(&mut taps, 0.7);
        assert_eq!(taps.len(), 3);
        register_tap(&mut taps, 10.0);
        assert_eq!(taps, vec![10.0]);
    }
}
