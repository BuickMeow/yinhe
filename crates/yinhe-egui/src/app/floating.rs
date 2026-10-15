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

/// 浮动工程走带（ARRANGE 主视图）窗口 id。
pub(crate) fn arrange_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("float_arrange")
}

/// 持久化矩形 `[x, y, w, h]` → `egui::Rect`。
pub(in crate::app) fn arr_to_rect(a: [f32; 4]) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(a[0], a[1]), egui::vec2(a[2], a[3]))
}

/// `egui::Rect` → 持久化矩形 `[x, y, w, h]`。
pub(in crate::app) fn rect_to_arr(r: egui::Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.width(), r.height()]
}

/// 浮动窗口内容区：标题栏之下整块。
///
/// 不用 `ui.available_rect_before_wrap()`：`title_bar` 结尾的 `allocate_space`
/// 会额外推进 `item_spacing`（默认 3px），内容与标题栏之间多出一条缝，
/// 看起来像标题栏偏高（PR 窗口尤其明显）。
fn float_content_rect(ui: &egui::Ui) -> egui::Rect {
    let max = ui.max_rect();
    egui::Rect::from_min_max(
        egui::pos2(max.min.x, max.min.y + crate::theme::TITLE_BAR_H),
        max.max,
    )
}

impl App {
    /// 浮动开关的借用集合（模式栏气泡 / 视图菜单共用切换逻辑）。
    pub(in crate::app) fn float_switches(&mut self) -> crate::chrome::mode_bar::FloatSwitches<'_> {
        crate::chrome::mode_bar::FloatSwitches {
            show_pianoroll: &mut self.show_pianoroll_in_arrange,
            float_pianoroll: &mut self.float_pianoroll,
            show_dock: &mut self.show_bottom_dock,
            float_dock: &mut self.float_bottom_dock,
            show_mixer: &mut self.show_mixer_panel,
            float_mixer: &mut self.float_mixer,
            float_arrange: &mut self.float_arrange,
            view_mode: &mut self.view_mode,
        }
    }

    /// 每帧绘制浮动面板窗口（`show_main_content` 之后调用）。
    pub(in crate::app) fn show_floating_panels(&mut self, ctx: &egui::Context) {
        self.show_float_arrange(ctx);
        self.show_float_pianoroll(ctx);
        self.show_float_dock(ctx);
        self.show_float_mixer(ctx);
    }

    /// 浮动工程走带窗口。
    fn show_float_arrange(&mut self, ctx: &egui::Context) {
        if !self.float_arrange {
            return;
        }
        if self.workspace.active_doc.is_none() {
            self.float_arrange = false;
            crate::chrome::dialog::close_viewport(ctx, arrange_id());
            return;
        }
        let id = arrange_id();
        let title = t!("float.arrange").to_string();
        let builder = float_window_builder(
            ctx,
            &title,
            self.float_arrange_rect,
            egui::vec2(1280.0, 720.0),
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
                    let rect = float_content_rect(ui);
                    let mut follow_mode = self.follow_mode;
                    self.show_arrange_view(ui, rect, rect.height(), &mut follow_mode, true);
                    self.follow_mode = follow_mode;
                });
            if close {
                self.float_arrange = false;
                crate::chrome::dialog::close_viewport(ui.ctx(), id);
            }
        });
        sync_window_rect(
            ctx,
            id,
            cur_rect,
            &mut self.float_arrange_rect,
            &mut self.layout_needs_save,
        );
    }

    /// 浮动钢琴卷帘窗口。
    fn show_float_pianoroll(&mut self, ctx: &egui::Context) {
        if !self.float_pianoroll {
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
                    let rect = float_content_rect(ui);
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
                    let rect = float_content_rect(ui);
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
                    let rect = float_content_rect(ui);
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
/// 恢复上次位置（窗口坐标即显示器坐标，原样恢复；拔屏后由系统收回主显示器），
/// 可缩放、可最大化。
fn float_window_builder(
    _ctx: &egui::Context,
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
        vb = vb.with_position(pos);
    }
    vb
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

    /// 回归：浮动窗口内容区紧贴标题栏下沿（无 `item_spacing` 缝）。
    #[test]
    fn float_content_starts_at_title_bar_bottom() {
        let ctx = egui::Context::default();
        let mut content_top = 0.0;
        let mut old_top = 0.0;
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                let mut close = false;
                crate::chrome::dialog::title_bar(ui, "T", &mut close, true);
                // 旧写法：多出 item_spacing 缝（这也是"标题栏看起来偏高"的来源）。
                old_top = ui.available_rect_before_wrap().min.y;
                content_top = float_content_rect(ui).min.y;
            },
        )
        .drop_without_applying_deltas();
        assert_eq!(
            content_top,
            crate::theme::TITLE_BAR_H,
            "内容区应紧贴标题栏下沿"
        );
        assert!(
            old_top > crate::theme::TITLE_BAR_H,
            "对照：available_rect_before_wrap 会多出缝（{old_top}）"
        );
    }
}
