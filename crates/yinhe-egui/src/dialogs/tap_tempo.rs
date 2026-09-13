//! 敲击测速（Tap Tempo）对话框（标准 viewport 形式）。
//!
//! 播放菜单「敲击测速」触发：独立 viewport 内点击敲击区或按空格键敲击，
//! 根据敲击间隔的平均值估算 BPM。仅用于测量显示，暂不写入工程。
//! 起步阶段没找稳节奏的敲击会随敲击次数逐步被忽略（见 [`ignored_prefix_len`]）。
//!
//! 空格键冲突说明：主窗口的空格是「播放/暂停」，本对话框是独立 OS 窗口，
//! 键盘事件只在聚焦窗口内分发，因此弹窗内空格不会触发主窗口播放。

use eframe::egui;
use rust_i18n::t;

/// 距上次敲击超过当前拍长的该倍数（漏敲超过一拍）时，视为重新开始一轮测量。
const TAP_RESET_BEATS: f64 = 2.0;

/// 从第几次敲击开始忽略最前面的敲击。
const IGNORE_START_TAP: usize = 9;
/// 之后每多敲该次数，就多忽略最前面的 1 次。
const IGNORE_STEP_TAPS: usize = 2;
/// 最多忽略最前面的敲击次数。
const MAX_IGNORED_TAPS: usize = 16;

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

/// 当前应忽略的最前面敲击次数：
/// 从第 [`IGNORE_START_TAP`] 次敲击起，每多敲 [`IGNORE_STEP_TAPS`] 次多忽略 1 次
/// （第 9 次忽略第 1 次，第 11 次忽略前 2 次，第 13 次忽略前 3 次…），
/// 最多忽略前 [`MAX_IGNORED_TAPS`] 次。
fn ignored_prefix_len(tap_count: usize) -> usize {
    if tap_count < IGNORE_START_TAP {
        return 0;
    }
    ((tap_count - IGNORE_START_TAP) / IGNORE_STEP_TAPS + 1).min(MAX_IGNORED_TAPS)
}

/// 忽略起步阶段后所有敲击的平均间隔（拍长，秒）；不足 2 次或跨度非正时返回 `None`。
fn average_interval(taps: &[f64]) -> Option<f64> {
    let used = taps.get(ignored_prefix_len(taps.len())..)?;
    if used.len() < 2 {
        return None;
    }
    let span = used[used.len() - 1] - used[0];
    if span <= 0.0 {
        return None;
    }
    Some(span / (used.len() - 1) as f64)
}

/// 由敲击时间戳估算 BPM（忽略起步阶段后的平均间隔的倒数）。
fn bpm_from_taps(taps: &[f64]) -> Option<f32> {
    Some((60.0 / average_interval(taps)?) as f32)
}

/// 记录一次敲击：距上次敲击超过当前拍长（忽略起步阶段后的平均间隔）
/// 的 [`TAP_RESET_BEATS`] 倍（漏敲超过一拍）时清空重来。
///
/// 不足 2 次敲击时还没有拍长可参考，直接追加，等待第二次敲击建立节奏。
fn register_tap(taps: &mut Vec<f64>, now: f64) {
    if let (Some(last), Some(beat)) = (taps.last().copied(), average_interval(taps))
        && now - last > beat * TAP_RESET_BEATS
    {
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
                            let btn_zone_h = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                            crate::chrome::dialog::content_with_bottom_buttons(
                                ui,
                                btn_zone_h,
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
                                    use crate::chrome::dialog_buttons::{
                                        DialogButton, dialog_button_row,
                                    };
                                    ui.add_space(8.0);
                                    let reset_label = t!("dialog.tap_tempo.reset");
                                    if dialog_button_row(
                                        ui,
                                        &[DialogButton::secondary(reset_label.as_ref())],
                                    )
                                    .is_some()
                                    {
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
    fn tap_keeps_tapping_within_two_beats() {
        // 拍长 0.5s → 阈值 1.0s：0.9s 以内继续累计
        let mut taps = vec![0.0, 0.5];
        register_tap(&mut taps, 0.9);
        assert_eq!(taps.len(), 3);
        // 恰好 1.0s（漏敲一拍）不重置
        let mut taps = vec![0.0, 0.5];
        register_tap(&mut taps, 1.5);
        assert_eq!(taps.len(), 3);
    }

    #[test]
    fn tap_resets_when_missing_more_than_one_beat() {
        // 拍长 0.5s → 阈值 1.0s：1.1s 超过两拍，重新开始
        let mut taps = vec![0.0, 0.5];
        register_tap(&mut taps, 1.6);
        assert_eq!(taps, vec![1.6]);
        // 按当前平均拍长判定：3 次 1.0s 间隔（拍长 1.0s → 阈值 2.0s）
        let mut taps = vec![0.0, 1.0, 2.0];
        register_tap(&mut taps, 4.1);
        assert_eq!(taps, vec![4.1]);
    }

    #[test]
    fn first_interval_has_no_reference_beat() {
        // 不足 2 次敲击时没有拍长可参考，直接追加
        let mut taps = vec![5.0];
        register_tap(&mut taps, 105.0);
        assert_eq!(taps, vec![5.0, 105.0]);
    }

    #[test]
    fn ignored_prefix_ramps_every_two_taps() {
        assert_eq!(ignored_prefix_len(0), 0);
        assert_eq!(ignored_prefix_len(8), 0);
        // 第 9 次忽略第 1 次；第 10 次仍是 1
        assert_eq!(ignored_prefix_len(9), 1);
        assert_eq!(ignored_prefix_len(10), 1);
        // 第 11 次忽略前 2 次；第 13 次忽略前 3 次
        assert_eq!(ignored_prefix_len(11), 2);
        assert_eq!(ignored_prefix_len(12), 2);
        assert_eq!(ignored_prefix_len(13), 3);
        // 第 39 次达到上限 16，之后不再增加
        assert_eq!(ignored_prefix_len(39), 16);
        assert_eq!(ignored_prefix_len(100), 16);
    }

    #[test]
    fn bpm_ignores_unstable_start_progressively() {
        // 起步没找稳：0.0 → 1.0 是 1 拍误差，之后稳定 0.5s
        let taps = [
            0.0, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5,
        ];
        // 第 9 次敲击（忽略第 1 次）：1.0 起都是 0.5s 间隔 → 120 BPM
        let bpm = bpm_from_taps(&taps[..9]).unwrap();
        assert!((bpm - 120.0).abs() < 1e-3);
        // 第 13 次敲击（忽略前 3 次）：2.0 起都是 0.5s 间隔 → 120 BPM
        let bpm = bpm_from_taps(&taps).unwrap();
        assert!((bpm - 120.0).abs() < 1e-3);
    }
}
