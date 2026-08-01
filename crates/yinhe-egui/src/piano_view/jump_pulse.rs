//! 事件浏览器跳转后的闪烁高亮（jump pulse）。
//!
//! 通过 egui ctx memory 读取 `JumpPulse`（App 在 main_loop 每帧写入）。
//! 动画结束后清除，避免无效重绘。

use eframe::egui;

use yinhe_types::PianoRollView;

use crate::right_panel::event_browser::PulseKind;
use crate::view_interaction::JumpPulse;

const PULSE_KEY: &str = "jump_pulse";

/// 绘制 jump pulse（若存在）。动画结束后自动清理。
pub fn paint(ui: &egui::Ui, view: &PianoRollView, music_rect: egui::Rect) {
    let pulse_key = egui::Id::new(PULSE_KEY);
    let pulse: Option<JumpPulse> = ui.ctx().memory(|m| m.data.get_temp(pulse_key));
    let Some(p) = pulse else { return };

    if p.finished() {
        ui.ctx()
            .memory_mut(|m| m.data.remove::<JumpPulse>(pulse_key));
        return;
    }

    let alpha = p.progress(); // 1 → 0
    let x = view.tick_to_x(p.tick as f64);
    let stroke = egui::Stroke::new(2.0, egui::Color32::WHITE.gamma_multiply(alpha));

    match p.kind {
        PulseKind::NoteRect => {
            if let Some(key) = p.key {
                // 在音符位置画白色描边矩形，不透明度随动画衰减
                let y_top = view.key_to_y(key);
                let h = view.key_height;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(music_rect.min.x + x, y_top),
                    egui::vec2(30.0, h),
                );
                ui.painter()
                    .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Middle);
            }
        }
        PulseKind::TimesigLine => {
            // 贯穿 music_rect 高度的白色竖线
            ui.painter().line_segment(
                [
                    egui::pos2(music_rect.min.x + x, music_rect.min.y),
                    egui::pos2(music_rect.min.x + x, music_rect.max.y),
                ],
                stroke,
            );
        }
    }
    ui.ctx().request_repaint();
}
