//! PR 钢琴卷帘视图：GPU cull 渲染（复用 yinhe-wgpu 管道）+ 触摸交互。
//!
//! 渲染管线：音符数据在模型加载后后台构建、一次性上传 GPU（compute cull），
//! 每帧只更新 uniforms（滚动/缩放）→ GPU cull → draw 到离屏纹理 → egui 显示。
//! 背景/网格线/键盘列由 egui 绘制（与桌面端一致的分工）。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use eframe::egui;
use yinhe_core::{Selection, YinModel};
use yinhe_types::{PianoRollView, TimelineViewBase};
use yinhe_wgpu::{InstanceRenderer, NoteInstance, build_all_notes, build_render_job};

/// 后台构建的音符实例数据（加载后一次性构建 + 上传）。
struct NoteBuildResult {
    notes: Vec<NoteInstance>,
    offsets: [u32; 129],
}

/// 轨道颜色调色板（12 色循环；桌面端来自主题，安卓先用固定调色板）。
const TRACK_PALETTE: [[f32; 4]; 12] = [
    [0.98, 0.55, 0.35, 1.0],
    [0.35, 0.75, 0.98, 1.0],
    [0.55, 0.90, 0.45, 1.0],
    [0.95, 0.85, 0.30, 1.0],
    [0.80, 0.45, 0.95, 1.0],
    [0.40, 0.90, 0.90, 1.0],
    [0.98, 0.40, 0.55, 1.0],
    [0.70, 0.70, 0.75, 1.0],
    [0.90, 0.70, 0.40, 1.0],
    [0.45, 0.55, 0.98, 1.0],
    [0.85, 0.65, 0.60, 1.0],
    [0.60, 0.80, 0.55, 1.0],
];

/// 顶部小节标尺高度（px）：内容区整体下移该距离给标尺让位。
const RULER_H: f32 = 24.0;

/// 一小节 tick 数（4/4 拍 @ 480ppq）。
const BAR_TICKS: i64 = 1920;

/// PR 视图：视口状态 + GPU 渲染 + 触摸交互。
pub struct PrView {
    wgpu_state: Arc<eframe::egui_wgpu::RenderState>,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    texture_id: egui::TextureId,
    renderer: Option<InstanceRenderer>,
    width: u32,
    height: u32,
    view: PianoRollView,
    model: Option<Arc<YinModel>>,
    notes_build: Option<Arc<Mutex<Option<NoteBuildResult>>>>,
    notes_uploaded: bool,
    status: String,
    keyboard_w: f32,
    /// 首次渲染时是否已完成初始视口定位（key 60 居中）。
    view_initialized: bool,
    /// 播放光标 tick（None = 不显示）。由 lib.rs 每帧从音频位置换算后设置。
    cursor_tick: Option<f64>,
}

impl PrView {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let wgpu_state: Arc<eframe::egui_wgpu::RenderState> = cc
            .wgpu_render_state
            .clone()
            .expect("wgpu backend required")
            .into();
        let (texture, texture_view, texture_id) = create_target(
            &wgpu_state.device,
            &mut wgpu_state.renderer.write(),
            wgpu_state.target_format,
            1,
            1,
        );
        Self {
            wgpu_state,
            texture,
            texture_view,
            texture_id,
            renderer: None,
            width: 1,
            height: 1,
            view: PianoRollView {
                base: TimelineViewBase {
                    pixels_per_tick: 0.05,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    left_panel_width: 0.0,
                    dirty: true,
                    track_panel_row_height: 0.0,
                    track_panel_scroll_y: 0.0,
                },
                key_height: 12.0,
                viewport_h: 0.0,
            },
            model: None,
            notes_build: None,
            notes_uploaded: false,
            status: String::new(),
            keyboard_w: 0.0,
            view_initialized: false,
            cursor_tick: None,
        }
    }

    /// 设置模型并初始化视口（全曲可见）。
    pub fn set_model(&mut self, model: Arc<YinModel>) {
        // 初始缩放：全曲宽约两个屏幕，用户再捏合放大。
        let tick_length = model.tempo_map.tick_length.max(1) as f32;
        let ppu = (1280.0 * 2.0 / tick_length).clamp(0.0005, 8.0);
        self.view.base.pixels_per_tick = ppu;
        self.view.base.scroll_x = 0.0;
        self.view.base.scroll_y = 0.0;
        self.view.base.dirty = true;
        self.model = Some(model);
        self.notes_build = None;
        self.notes_uploaded = false;
        self.status = "正在构建音符数据...".to_string();
    }

    /// 设置播放光标位置（tick），None 隐藏。由 lib.rs 每帧从音频位置换算后调用。
    pub fn set_cursor(&mut self, tick: Option<f64>) {
        self.cursor_tick = tick;
    }

    /// 播放跟随：水平滚动让光标位于内容区中央（lib.rs 在跟随模式开启时每帧调用）。
    pub fn follow_cursor(&mut self) {
        let Some(tick) = self.cursor_tick else {
            return;
        };
        let content_w = (self.width as f32 - self.keyboard_w).max(1.0);
        let target = tick as f32 * self.view.base.pixels_per_tick - content_w / 2.0;
        self.view.base.scroll_x = target.max(0.0);
        self.view.base.dirty = true;
    }

    /// 主 UI 入口：背景 + 键盘列 + 网格 + GPU 音符层 + 触摸交互。
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(22));

        // 顶部小节标尺带：高度充足时内容区整体下移 24px 让位，否则不画标尺。
        let ruler_h = if rect.height() > 200.0 { RULER_H } else { 0.0 };

        let Some(model) = self.model.clone() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "未加载 MIDI",
                egui::FontId::proportional(18.0),
                egui::Color32::GRAY,
            );
            return;
        };

        self.keyboard_w = 64.0;
        self.view.base.left_panel_width = self.keyboard_w;
        self.view.viewport_h = rect.height();
        // 首次渲染：纵向定位到中央音区（key 60），否则 scroll_y=0 显示最高音区。
        // 与 GPU 音符层同一参考系：key 127 在顶，key 60 行中心 = 128kh - 60.5kh。
        if !self.view_initialized {
            let kh = self.view.key_height;
            self.view.base.scroll_y = ((128.0 - 60.5) * kh - rect.height() / 2.0).max(0.0);
            self.view_initialized = true;
        }
        self.ensure_texture_size(rect);
        self.handle_touch(ui, rect);
        self.ensure_renderer();
        self.ensure_notes(&model);

        if let Some(renderer) = &mut self.renderer {
            let track_colors = track_colors_for(&model);
            let selected = Selection::default();
            let job = build_render_job(
                self.width,
                self.height,
                &self.view,
                &selected,
                &track_colors,
                0,
                0.0,
                true,
            );
            renderer.upload_uniforms(job.uniforms);
            renderer.upload_track_colors(&job.track_colors);
            renderer.upload_selection(&job.selection);
            let mut encoder = self
                .wgpu_state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            // draw 的 pass 自带透明 clear，音符绘制在这里完成。
            renderer.draw(&mut encoder, &self.texture_view, self.width, self.height);
            self.wgpu_state.queue.submit([encoder.finish()]);
        }

        // egui 层：键盘列与内容区都从标尺带下方开始，与音符层（纹理 1:1
        // 贴图、顶部让位）保持对齐。
        let kb_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.min.y + ruler_h),
            egui::pos2(rect.min.x + self.keyboard_w, rect.max.y),
        );
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + self.keyboard_w, rect.min.y + ruler_h),
            rect.max,
        );
        self.draw_scale_bands(ui, content_rect);
        self.draw_keyboard(ui, kb_rect);
        self.draw_grid(ui, content_rect);
        // 离屏纹理覆盖整个 rect（含键盘列，与桌面端一致）：shader 的音符
        // 像素坐标从 keyboard_width 起步，NDC 用总宽做分母，纹理必须同宽，
        // 否则音符整体右移 keyboard_width 像素且右端溢出被裁（安卓看不到
        // 音符的根因）。键盘列是纹理的透明区，由 egui 键盘层透出。
        // 顶部 ruler_h 像素让给标尺：贴图目标与 UV 同步裁掉该段，保持 1:1。
        painter.image(
            self.texture_id,
            egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + ruler_h), rect.max),
            egui::Rect::from_min_max(
                egui::pos2(0.0, ruler_h / self.height.max(1) as f32),
                egui::pos2(1.0, 1.0),
            ),
            egui::Color32::WHITE,
        );
        self.draw_ruler(ui, rect, content_rect, ruler_h);
        self.draw_cursor(ui, rect, ruler_h);

        let notes: u64 = model.track_note_count.iter().sum();
        let status = format!(
            "{} | {notes} 音符 | {:.3} px/tick | 双指缩放/拖动",
            self.status, self.view.base.pixels_per_tick,
        );
        painter.text(
            egui::pos2(rect.min.x + 8.0, rect.min.y + ruler_h + 8.0),
            egui::Align2::LEFT_TOP,
            status,
            egui::FontId::proportional(13.0),
            egui::Color32::from_gray(200),
        );
    }

    /// 触摸手势：双指捏合 → 键盘列上垂直缩放 key_height / 内容区水平缩放；
    /// 双指/单指 → 滚动。
    fn handle_touch(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let (zoom, pan, touches, pointer_delta, pointer_down, touch_center) = ui.ctx().input(|i| {
            let mt = i.multi_touch();
            (
                mt.map(|m| m.zoom_delta),
                mt.map(|m| m.translation_delta),
                mt.map(|m| m.num_touches).unwrap_or(0),
                i.pointer.delta(),
                i.pointer.primary_down(),
                mt.map(|m| m.center_pos),
            )
        });
        let base = &mut self.view.base;
        if let Some(zoom) = zoom
            && (zoom - 1.0).abs() > 0.001
        {
            if touch_center.is_some_and(|c| c.x < rect.min.x + self.keyboard_w) {
                // 键盘列上捏合 → 垂直缩放 key_height。锚点 = 双指中心对应的
                // key 浮点位置，缩放后该 key 仍停留在手指处（同 shader 参考系）。
                let y0 = touch_center.expect("checked above").y;
                let kh = self.view.key_height;
                let bottom = 128.0 * kh - base.scroll_y;
                let anchor_key = (bottom - (y0 - rect.min.y)) / kh;
                let new_kh = (kh * zoom).clamp(4.0, 48.0);
                let new_bottom = (y0 - rect.min.y) + anchor_key * new_kh;
                base.scroll_y = (128.0 * new_kh - new_bottom).max(0.0);
                self.view.key_height = new_kh;
                base.dirty = true;
            } else {
                // 内容区捏合 → 水平缩放。scroll_x 是内容区（不含键盘列）的
                // 滚动偏移，缩放中心取内容区中点。
                let content_w = rect.width() - self.keyboard_w;
                let center_tick =
                    (base.scroll_x + content_w / 2.0) / base.pixels_per_tick.max(1e-6);
                base.pixels_per_tick = (base.pixels_per_tick * zoom).clamp(0.0005, 8.0);
                base.scroll_x = center_tick * base.pixels_per_tick - content_w / 2.0;
                base.dirty = true;
            }
        }
        if touches >= 2 {
            if let Some(pan) = pan {
                base.scroll_x = (base.scroll_x - pan.x).max(0.0);
                base.scroll_y = (base.scroll_y - pan.y).max(0.0);
                base.dirty = true;
            }
        } else if pointer_down {
            base.scroll_x = (base.scroll_x - pointer_delta.x).max(0.0);
            base.scroll_y = (base.scroll_y - pointer_delta.y).max(0.0);
            base.dirty = true;
        }
    }

    /// 惰性初始化 InstanceRenderer（GPU 管道：compute cull + draw）。
    fn ensure_renderer(&mut self) {
        if self.renderer.is_none() {
            // 注册 wgpu 错误回调：pipeline/bind group 创建失败在 wgpu 30 是静默的
            //（错误进 uncaptured error），安卓 stderr 不可见，必须显式打到 logcat。
            let device = self.wgpu_state.device.clone();
            device.on_uncaptured_error(Arc::new(|err| {
                log::error!("wgpu uncaptured error: {err}");
            }));
            let queue = self.wgpu_state.queue.clone();
            let format = self.wgpu_state.target_format;
            log::info!("pr_view: InstanceRenderer 初始化，target_format={format:?}");
            log::info!(
                "pr_view: 设备 features INDIRECT_FIRST_INSTANCE={}",
                device
                    .features()
                    .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            );
            self.renderer = Some(InstanceRenderer::new(device, queue, format));
            log::info!("pr_view: InstanceRenderer 初始化完成");
        }
    }

    /// 音符数据：后台线程构建（build_all_notes 是 CPU 密集），完成后上传 GPU。
    fn ensure_notes(&mut self, model: &Arc<YinModel>) {
        if self.notes_uploaded {
            return;
        }
        if self.notes_build.is_none() {
            let model = model.clone();
            let state: Arc<Mutex<Option<NoteBuildResult>>> = Arc::new(Mutex::new(None));
            let state2 = state.clone();
            log::info!("pr_view: 启动音符构建线程");
            std::thread::Builder::new()
                .name("yinhe-note-build".into())
                .spawn(move || {
                    // catch_unwind：安卓上 Rust panic 的 stderr 不可见（不进 logcat），
                    // 不捕获的话 UI 永远停在"正在构建"且无任何线索。
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let track_visible = vec![true; model.tracks.len()];
                        let (notes, offsets) =
                            build_all_notes(model.as_ref(), &HashSet::new(), &track_visible);
                        NoteBuildResult { notes, offsets }
                    }));
                    match result {
                        Ok(r) => {
                            log::info!("pr_view: 音符构建完成 {} 实例", r.notes.len());
                            *state2.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                        }
                        Err(e) => {
                            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                                (*s).to_string()
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic".to_string()
                            };
                            log::error!("pr_view: build_all_notes panic: {msg}");
                            // 用空结果标记失败（None 表示仍在构建）
                            *state2.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(NoteBuildResult {
                                    notes: Vec::new(),
                                    offsets: [0; 129],
                                });
                        }
                    }
                })
                .expect("failed to spawn note build thread");
            self.notes_build = Some(state);
        }
        if let Some(state) = &self.notes_build {
            let result = state.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(result) = result
                && let Some(renderer) = &mut self.renderer
            {
                if result.notes.is_empty() {
                    self.status = "音符构建失败（见 logcat）".to_string();
                    self.notes_uploaded = true; // 不再重试
                    return;
                }
                renderer.upload_all_notes_for_cull(&result.notes, &result.offsets, &[0u64; 128]);
                self.notes_uploaded = true;
                let notes: u64 = model.track_note_count.iter().sum();
                self.status = format!("{notes} 音符已上传 GPU");
            }
        }
    }

    /// 离屏纹理尺寸跟随可用区域（双向重建）。
    /// 纹理宽 = 整个 rect（含键盘列）：shader 的音符像素坐标以
    /// keyboard_width 为原点（x_offset = keyboard_width - scroll_x），
    /// NDC 用总宽做分母，与桌面端一致（桌面端纹理就是 content_rect 总宽）。
    fn ensure_texture_size(&mut self, rect: egui::Rect) {
        let w = rect.width().max(1.0) as u32;
        let h = rect.height().max(1.0) as u32;
        if w == self.width && h == self.height {
            return;
        }
        let (texture, texture_view, texture_id) = create_target(
            &self.wgpu_state.device,
            &mut self.wgpu_state.renderer.write(),
            self.wgpu_state.target_format,
            w,
            h,
        );
        self.texture = texture;
        self.texture_view = texture_view;
        self.texture_id = texture_id;
        self.width = w;
        self.height = h;
    }

    /// 键盘列：可见键范围内的黑白键。
    /// 纵向参考系与 GPU 音符层一致（key 127 在顶，piano 惯例高音在上）：
    /// 之前用 key 0 在顶的公式导致键盘列上下颠倒且与音符层错位。
    fn draw_keyboard(&self, ui: &egui::Ui, rect: egui::Rect) {
        let kh = self.view.key_height;
        let scroll_y = self.view.base.scroll_y;
        let bottom = 128.0 * kh - scroll_y;
        let top_key = ((bottom / kh).ceil() as i32 - 1).clamp(0, 127);
        let bottom_key = (((bottom - rect.height()) / kh).ceil() as i32 - 1).clamp(0, 127);
        let (key_lo, key_hi) = (bottom_key.min(top_key), bottom_key.max(top_key));
        let painter = ui.painter();
        let is_black = |k: i32| matches!(k % 12, 1 | 3 | 6 | 8 | 10);
        for key in key_lo..=key_hi {
            let y = rect.min.y + bottom - (key as f32 + 1.0) * kh;
            let color = if is_black(key) {
                egui::Color32::from_gray(36)
            } else {
                egui::Color32::from_gray(72)
            };
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y + kh)),
                0.0,
                color,
            );
        }
        painter.line_segment(
            [
                egui::pos2(rect.max.x, rect.min.y),
                egui::pos2(rect.max.x, rect.max.y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
        );
    }

    /// 横向背景色带：有调号时按当前生效调式着色（调内音浅色、根音更亮），
    /// 无调号时退化为白键行浅色——与桌面端 bg::paint 同一策略，辅助定位
    /// 音高与调性。y 与 GPU 音符层同参考系（key 越大越靠上）。
    fn draw_scale_bands(&self, ui: &egui::Ui, rect: egui::Rect) {
        let kh = self.view.key_height;
        let scroll_y = self.view.base.scroll_y;
        let bottom = 128.0 * kh - scroll_y;
        let top_key = ((bottom / kh).ceil() as i32 - 1).clamp(0, 127);
        let bottom_key = (((bottom - rect.height()) / kh).ceil() as i32 - 1).clamp(0, 127);
        let (key_lo, key_hi) = (bottom_key.min(top_key), bottom_key.max(top_key));
        let painter = ui.painter();
        let band = |painter: &egui::Painter, key: i32, color: egui::Color32| {
            let y = rect.min.y + bottom - (key as f32 + 1.0) * kh;
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y + kh)),
                0.0,
                color,
            );
        };

        // 当前生效调号：视口起点之前最后一个调号事件（无则取第一个）。
        let scroll_tick = self.view.base.scroll_x / self.view.base.pixels_per_tick.max(1e-6);
        let scroll_tick = scroll_tick as f64;
        let key_sig = self
            .model
            .as_ref()
            .map(|m| m.conductor.key_sig.as_slice())
            .unwrap_or(&[]);
        let eff = key_sig
            .iter()
            .rev()
            .find(|e| e.tick as f64 <= scroll_tick)
            .or_else(|| key_sig.first());

        let Some(ev) = eff else {
            // 无调号：白键行浅色，与键盘列黑白键呼应。
            for key in key_lo..=key_hi {
                if !matches!(key % 12, 1 | 3 | 6 | 8 | 10) {
                    band(painter, key, egui::Color32::from_gray(26));
                }
            }
            return;
        };
        let mask = ev.scale.pitch_classes(ev.root);
        for key in key_lo..=key_hi {
            let pc = (key as u8) % 12;
            let color = if pc == ev.root {
                egui::Color32::from_gray(34)
            } else if mask & (1u16 << pc) != 0 {
                egui::Color32::from_gray(26)
            } else {
                continue;
            };
            band(painter, key, color);
        }
    }

    /// 顶部小节标尺：深色底 + 每小节（1920 tick）刻度线 + 小节号。
    /// 背景覆盖整个 rect（含键盘列上方，避免空缝），刻度只画在内容区。
    /// 刻度 x 与 draw_grid 同公式（tick→像素）；步长过密时只画刻度不画
    /// 数字，更密时整条隐藏（阈值与 draw_grid 一致）。
    fn draw_ruler(&self, ui: &egui::Ui, rect: egui::Rect, content_rect: egui::Rect, ruler_h: f32) {
        if ruler_h <= 0.0 {
            return;
        }
        let ppu = self.view.base.pixels_per_tick;
        let scroll_x = self.view.base.scroll_x;
        let step_px = BAR_TICKS as f32 * ppu;
        if step_px < 6.0 {
            return;
        }
        // 背景铺满整个 rect 宽度（含键盘列上方），刻度从内容区开始。
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.min.y),
            egui::pos2(rect.max.x, rect.min.y + ruler_h),
        );
        let painter = ui.painter();
        painter.rect_filled(ruler_rect, 0.0, egui::Color32::from_gray(30));
        // 与内容区的分界线。
        painter.line_segment(
            [
                egui::pos2(ruler_rect.min.x, ruler_rect.max.y),
                egui::pos2(ruler_rect.max.x, ruler_rect.max.y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
        );
        let show_number = step_px >= 30.0;
        let start_tick = ((scroll_x / ppu) as i64 / BAR_TICKS) * BAR_TICKS;
        let mut bar = start_tick / BAR_TICKS;
        let mut x = content_rect.min.x + (start_tick as f32 * ppu - scroll_x);
        // 跳过仍位于键盘列上方的刻度。
        while x < content_rect.min.x {
            x += step_px;
            bar += 1;
        }
        while x < content_rect.max.x {
            painter.line_segment(
                [
                    egui::pos2(x, ruler_rect.min.y),
                    egui::pos2(x, ruler_rect.max.y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
            );
            if show_number {
                painter.text(
                    egui::pos2(x + 3.0, ruler_rect.min.y + 1.0),
                    egui::Align2::LEFT_TOP,
                    // 小节号从 1 开始（tick 0 = 第 1 小节）。
                    (bar + 1).to_string(),
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_gray(170),
                );
            }
            x += step_px;
            bar += 1;
        }
    }

    /// 播放光标：竖线随 scroll_x 移动，y 从标尺底部到内容区底。
    /// x 与网格同公式（tick→像素），并 clamp 在内容区内。
    fn draw_cursor(&self, ui: &egui::Ui, rect: egui::Rect, ruler_h: f32) {
        let Some(tick) = self.cursor_tick else {
            return;
        };
        let ppu = self.view.base.pixels_per_tick;
        let scroll_x = self.view.base.scroll_x;
        let x = rect.min.x + self.keyboard_w + (tick as f32 * ppu - scroll_x);
        let x = x.clamp(rect.min.x + self.keyboard_w, rect.max.x);
        let painter = ui.painter();
        painter.line_segment(
            [
                egui::pos2(x, rect.min.y + ruler_h),
                egui::pos2(x, rect.max.y),
            ],
            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 80)),
        );
    }

    /// 网格线：每 1920 tick（4/4 小节 @480ppq）一条竖线，过密时跳过。
    /// 首条 start_tick 是 <= 视口起点的最大整小节，x 可能在键盘列上方，
    /// 需推进到内容区内再画（否则多画一条盖在钢琴列上）。
    fn draw_grid(&self, ui: &egui::Ui, rect: egui::Rect) {
        let ppu = self.view.base.pixels_per_tick;
        let scroll_x = self.view.base.scroll_x;
        let step_px = BAR_TICKS as f32 * ppu;
        if step_px < 6.0 {
            return;
        }
        let start_tick = ((scroll_x / ppu) as i64 / BAR_TICKS) * BAR_TICKS;
        let painter = ui.painter();
        let mut x = rect.min.x + (start_tick as f32 * ppu - scroll_x);
        while x < rect.min.x {
            x += step_px;
        }
        while x < rect.max.x {
            painter.line_segment(
                [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                egui::Stroke::new(1.0, egui::Color32::from_gray(45)),
            );
            x += step_px;
        }
    }
}

/// 按轨道索引生成颜色（12 色调色板循环）。
fn track_colors_for(model: &YinModel) -> Vec<[f32; 4]> {
    model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, _)| TRACK_PALETTE[i % TRACK_PALETTE.len()])
        .collect()
}

/// 创建离屏渲染纹理并注册为 egui 纹理（与桌面 RenderContext 同款）。
fn create_target(
    device: &wgpu::Device,
    egui_renderer: &mut eframe::egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, egui::TextureId) {
    let max_dim = device.limits().max_texture_dimension_2d;
    let width = width.min(max_dim).max(1);
    let height = height.min(max_dim).max(1);
    let linear_format = match format {
        wgpu::TextureFormat::Bgra8UnormSrgb => Some(wgpu::TextureFormat::Bgra8Unorm),
        wgpu::TextureFormat::Rgba8UnormSrgb => Some(wgpu::TextureFormat::Rgba8Unorm),
        _ => None,
    };
    let view_formats: &[wgpu::TextureFormat] = if let Some(lf) = &linear_format {
        std::slice::from_ref(lf)
    } else {
        &[]
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pianoroll_preview"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let texture_id =
        egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Nearest);
    (texture, view, texture_id)
}
