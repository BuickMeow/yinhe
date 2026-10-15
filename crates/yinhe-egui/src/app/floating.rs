//! 浮动面板窗口：钢琴卷帘 / 设备链 / 混音台泊出为独立 OS 窗口。
//!
//! 与对话框同一套 viewport 基建（`chrome::dialog`）：macOS 无边框 + 自绘标题栏，
//! 其余平台无系统装饰 + 自绘关闭按钮。
//!
//! 互斥规则（状态字段见 `LayoutSettings::float_*`）：
//! - `float_*`（浮动形态）与 `show_*`（停靠形态）互斥：泊出时 `show_*` 置 false，
//!   因此同一面板任意时刻只渲染一份（钢琴卷帘的 GPU 视图只允许单实例）。
//! - EDIT 模式主窗口恒为全屏 PR：进入时自动收起浮动 PR。
//! - 浮出的面板不参与底部三选一互斥（不占主窗口空间）。

use eframe::egui;
use rust_i18n::t;

use crate::app::App;

/// 浮动钢琴卷帘窗口 id。
pub(crate) fn pianoroll_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("float_pianoroll")
}

/// 浮动设备链窗口 id。
pub(crate) fn dock_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("float_bottom_dock")
}

/// 浮动混音台窗口 id。
pub(crate) fn mixer_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("float_mixer")
}

/// 持久化矩形 `[x, y, w, h]` → `egui::Rect`。
pub(in crate::app) fn arr_to_rect(a: [f32; 4]) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(a[0], a[1]), egui::vec2(a[2], a[3]))
}

/// `egui::Rect` → 持久化矩形 `[x, y, w, h]`。
pub(in crate::app) fn rect_to_arr(r: egui::Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.width(), r.height()]
}

impl App {
    /// 每帧绘制浮动面板窗口（`show_main_content` 之后调用）。
    pub(in crate::app) fn show_floating_panels(&mut self, ctx: &egui::Context) {
        self.show_float_pianoroll(ctx);
        self.show_float_dock(ctx);
        self.show_float_mixer(ctx);
    }

    /// 浮动钢琴卷帘窗口。
    fn show_float_pianoroll(&mut self, ctx: &egui::Context) {
        if !self.float_pianoroll {
            return;
        }
        // EDIT 模式主窗口恒为全屏 PR（GPU 视图单实例）→ 浮动 PR 自动收起。
        if self.view_mode == crate::chrome::mode_bar::ViewMode::Edit {
            self.float_pianoroll = false;
            crate::chrome::dialog::close_viewport(ctx, pianoroll_id());
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            self.float_pianoroll = false;
            crate::chrome::dialog::close_viewport(ctx, pianoroll_id());
            return;
        };
        let id = pianoroll_id();
        let title = t!("float.pianoroll").to_string();
        let builder = float_window_builder(
            ctx,
            &title,
            self.float_pianoroll_rect,
            egui::vec2(960.0, 520.0),
        );

        let is_playing = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        let sel_hint = Self::compute_sel_hint(&self.workspace.documents[idx]);

        let mut close = false;
        let mut cur_rect: Option<egui::Rect> = None;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            crate::theme::apply_to_ctx(ui.ctx());
            cur_rect = ui.input(|i| i.viewport().inner_rect);
            if ui.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(crate::theme::app_bg()))
                .show(ui, |ui| {
                    let mut title_close = false;
                    crate::chrome::dialog::title_bar(ui, &title, &mut title_close, true);
                    if title_close {
                        close = true;
                    }
                    if close {
                        return;
                    }
                    // 内容区：标题栏之下整块给 PR（左侧工具条 + 卷帘本体），
                    // 与主窗口停靠形态共用同一渲染路径（合成布局）。
                    let rect = ui.available_rect_before_wrap();
                    let layout = crate::app::layout::LayoutInfo {
                        remaining: rect,
                        arr_h: 0.0,
                        bottom_y: rect.min.y,
                        right_panel_rect: egui::Rect::NOTHING,
                    };
                    let mut follow_mode = self.follow_mode;
                    self.show_pianoroll_split(
                        ui,
                        &layout,
                        idx,
                        is_playing,
                        &mut follow_mode,
                        sel_hint.as_ref(),
                    );
                    self.follow_mode = follow_mode;
                });
            if close {
                self.float_pianoroll = false;
                crate::chrome::dialog::close_viewport(ui.ctx(), id);
            }
        });
        sync_window_rect(
            ctx,
            id,
            cur_rect,
            &mut self.float_pianoroll_rect,
            &mut self.layout_needs_save,
        );
    }

    /// 浮动设备链窗口。
    fn show_float_dock(&mut self, ctx: &egui::Context) {
        if !self.float_bottom_dock {
            return;
        }
        let Some(idx) = self.workspace.active_doc else {
            self.float_bottom_dock = false;
            crate::chrome::dialog::close_viewport(ctx, dock_id());
            return;
        };
        let id = dock_id();
        let title = t!("float.dock").to_string();
        let builder = float_window_builder(
            ctx,
            &title,
            self.float_bottom_dock_rect,
            egui::vec2(1100.0, 320.0),
        );

        let mut close = false;
        let mut cur_rect: Option<egui::Rect> = None;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            crate::theme::apply_to_ctx(ui.ctx());
            cur_rect = ui.input(|i| i.viewport().inner_rect);
            if ui.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(crate::theme::app_bg()))
                .show(ui, |ui| {
                    let mut title_close = false;
                    crate::chrome::dialog::title_bar(ui, &title, &mut title_close, true);
                    if title_close {
                        close = true;
                    }
                    if close {
                        return;
                    }
                    let rect = ui.available_rect_before_wrap();
                    crate::chrome::dock_bar::show_floating(self, idx, ui, rect);
                });
            if close {
                self.float_bottom_dock = false;
                crate::chrome::dialog::close_viewport(ui.ctx(), id);
            }
        });
        sync_window_rect(
            ctx,
            id,
            cur_rect,
            &mut self.float_bottom_dock_rect,
            &mut self.layout_needs_save,
        );
    }

    /// 浮动混音台窗口。
    fn show_float_mixer(&mut self, ctx: &egui::Context) {
        if !self.float_mixer {
            return;
        }
        if self.workspace.active_doc.is_none() {
            self.float_mixer = false;
            crate::chrome::dialog::close_viewport(ctx, mixer_id());
            return;
        }
        let id = mixer_id();
        let title = t!("float.mixer").to_string();
        let builder = float_window_builder(
            ctx,
            &title,
            self.float_mixer_rect,
            egui::vec2(1100.0, 520.0),
        );

        let mut close = false;
        let mut cur_rect: Option<egui::Rect> = None;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            crate::theme::apply_to_ctx(ui.ctx());
            cur_rect = ui.input(|i| i.viewport().inner_rect);
            if ui.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(crate::theme::app_bg()))
                .show(ui, |ui| {
                    let mut title_close = false;
                    crate::chrome::dialog::title_bar(ui, &title, &mut title_close, true);
                    if title_close {
                        close = true;
                    }
                    if close {
                        return;
                    }
                    let rect = ui.available_rect_before_wrap();
                    crate::mix::show(self, ui, rect);
                });
            if close {
                self.float_mixer = false;
                crate::chrome::dialog::close_viewport(ui.ctx(), id);
            }
        });
        sync_window_rect(
            ctx,
            id,
            cur_rect,
            &mut self.float_mixer_rect,
            &mut self.layout_needs_save,
        );
    }
}

/// 浮动面板窗口 builder：复用对话框窗口样式（macOS 无边框/全尺寸内容），
/// 恢复上次位置（越界时夹回主显示器可见范围），可缩放、可最大化。
fn float_window_builder(
    ctx: &egui::Context,
    title: &str,
    stored: Option<egui::Rect>,
    default_size: egui::Vec2,
) -> egui::ViewportBuilder {
    let (size, pos) = match stored {
        Some(r) => (r.size(), Some(r.min)),
        None => (default_size, None),
    };
    let mut vb = crate::chrome::dialog::viewport_builder(title, [size.x, size.y], true)
        .with_min_inner_size([360.0, 200.0]);
    if let Some(pos) = pos {
        let monitor = ctx.input(|i| i.viewport().monitor_size);
        vb = vb.with_position(clamp_window_pos(pos, size, monitor));
    }
    vb
}

/// 夹回主显示器：保证标题栏区域可见（换显示器/拔掉副屏后窗口不丢）。
fn clamp_window_pos(pos: egui::Pos2, size: egui::Vec2, monitor: Option<egui::Vec2>) -> egui::Pos2 {
    let Some(m) = monitor else {
        return pos;
    };
    egui::pos2(
        pos.x.clamp(120.0 - size.x, (m.x - 120.0).max(0.0)),
        pos.y.clamp(0.0, (m.y - 48.0).max(0.0)),
    )
}

/// 同步浮动窗口矩形：窗口移动/缩放中只更新「上帧快照」，
/// 停稳一帧后才写入持久化字段并置写盘标志（拖窗口时不逐帧刷盘）。
fn sync_window_rect(
    ctx: &egui::Context,
    id: egui::ViewportId,
    cur: Option<egui::Rect>,
    stored: &mut Option<egui::Rect>,
    needs_save: &mut bool,
) {
    let key = egui::Id::new(("float_window_rect_last", id));
    let last: Option<egui::Rect> = ctx
        .data_mut(|d| d.get_temp::<Option<egui::Rect>>(key))
        .flatten();
    if cur.is_some() && cur == last {
        if *stored != cur {
            *stored = cur;
            *needs_save = true;
        }
    } else {
        ctx.data_mut(|d| d.insert_temp(key, cur));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 矩形持久化往返：`[x, y, w, h]` 与 `egui::Rect` 互转无损。
    #[test]
    fn rect_roundtrip() {
        let r = egui::Rect::from_min_size(egui::pos2(120.0, 80.0), egui::vec2(960.0, 520.0));
        assert_eq!(arr_to_rect(rect_to_arr(r)), r);
    }

    /// 越界位置夹回主显示器：标题栏区域保持可见。
    #[test]
    fn clamp_window_pos_keeps_title_visible() {
        let size = egui::vec2(900.0, 500.0);
        let monitor = Some(egui::vec2(1920.0, 1080.0));
        // 正常位置不动
        let p = egui::pos2(100.0, 60.0);
        assert_eq!(clamp_window_pos(p, size, monitor), p);
        // 右下方越界 → 夹回
        let p = clamp_window_pos(egui::pos2(5000.0, 3000.0), size, monitor);
        assert!(p.x <= 1920.0 - 120.0 && p.y <= 1080.0 - 48.0);
        // 左上方越界 → 夹回
        let p = clamp_window_pos(egui::pos2(-2000.0, -300.0), size, monitor);
        assert!(p.x >= 120.0 - size.x && p.y >= 0.0);
        // 无显示器信息 → 原样
        let p = egui::pos2(-500.0, -100.0);
        assert_eq!(clamp_window_pos(p, size, None), p);
    }
}
