//! PR 钢琴卷帘视图：GPU cull 渲染（复用 yinhe-wgpu 管道）+ 触摸交互。
//!
//! 渲染管线：音符数据在模型加载后后台构建、一次性上传 GPU（compute cull），
//! 每帧只更新 uniforms（滚动/缩放）→ GPU cull → draw 到离屏纹理 → egui 显示。
//! 背景/网格线/键盘列由 egui 绘制（与桌面端一致的分工）。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use eframe::egui;
use yinhe_core::{Selection, YinModel};
use yinhe_theme::base::BaseColors;
use yinhe_theme::egui_colors::{Theme, derive_theme, mix};
use yinhe_types::{
    PianoRollView, TimelineViewBase, build_time_sig_segments, compute_measure_divisor,
    measure_ticks,
};
use yinhe_wgpu::{InstanceRenderer, NoteInstance, build_all_notes, build_render_job};

/// 后台构建的音符实例数据（加载后一次性构建 + 上传）。
struct NoteBuildResult {
    notes: Vec<NoteInstance>,
    offsets: [u32; 129],
}

/// 顶部小节标尺高度（px）：内容区整体下移该距离给标尺让位。
/// 手机屏幕紧张，比桌面端（~28px）更矮。
const RULER_H: f32 = 16.0;

/// PR 视图：视口状态 + GPU 渲染 + 触摸交互。
pub struct PrView {
    wgpu_state: Arc<eframe::egui_wgpu::RenderState>,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    texture_id: egui::TextureId,
    renderer: Option<InstanceRenderer>,
    /// uniforms/视口逻辑尺寸（逻辑像素，egui 坐标单位）。
    width: u32,
    height: u32,
    /// 离屏纹理物理尺寸（逻辑尺寸 × pixels_per_point）。纹理按物理像素
    /// 创建，否则高分屏上音符发虚。
    tex_w: u32,
    tex_h: u32,
    /// 当前像素密度（每帧从 egui 读取）。
    ppp: f32,
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
    /// 主题色（与桌面端同源：BaseColors 7 色派生）。
    theme: Theme,
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
            tex_w: 1,
            tex_h: 1,
            ppp: 1.0,
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
            theme: derive_theme(BaseColors::DARK),
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
    /// safe 为安全区 insets（逻辑点）：[left, top, right, bottom]。
    pub fn ui(&mut self, ui: &mut egui::Ui, safe: [f32; 4]) {
        let full = ui.available_rect_before_wrap();
        let painter = ui.painter();
        // 背景铺满整个视口（延伸到挖孔/刘海后面，视觉融合）；
        // 内容区（纹理/键盘/标尺）整体避开安全区，挖孔区域只显示背景色。
        painter.rect_filled(full, 0.0, self.theme.app_bg);
        let rect = egui::Rect::from_min_max(
            full.min + egui::vec2(safe[0], safe[1]),
            full.max - egui::vec2(safe[2], safe[3]),
        );
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }

        // 顶部小节标尺带：高度充足时内容区整体下移 24px 让位，否则不画标尺。
        let ruler_h = if rect.height() > 200.0 { RULER_H } else { 0.0 };

        let Some(model) = self.model.clone() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "未加载 MIDI",
                egui::FontId::proportional(18.0),
                self.theme.text_muted,
            );
            return;
        };

        self.keyboard_w = 64.0;
        self.view.base.left_panel_width = self.keyboard_w;
        self.view.viewport_h = rect.height();
        // 像素密度：纹理按物理像素创建（高分屏不发虚）。
        self.ppp = ui.ctx().pixels_per_point().max(0.25);
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
            let track_colors = crate::track_colors_for(&model);
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
            // viewport 必须用纹理物理尺寸。
            renderer.draw(&mut encoder, &self.texture_view, self.tex_w, self.tex_h);
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
        self.draw_octave_lines(ui, content_rect);
        self.draw_keyboard(ui, kb_rect);
        self.draw_grid(ui, content_rect);
        // 离屏纹理覆盖整个 rect（含键盘列，与桌面端一致）：shader 的音符
        // 像素坐标从 keyboard_width 起步，NDC 用总宽做分母，纹理必须同宽，
        // 否则音符整体右移 keyboard_width 像素且右端溢出被裁（安卓看不到
        // 音符的根因）。键盘列是纹理的透明区，由 egui 键盘层透出。
        // 顶部 ruler_h 像素让给标尺：贴图目标与 UV 同步裁掉该段，保持 1:1。
        // UV 用物理像素（纹理是物理尺寸，ruler_h 是逻辑像素）。
        painter.image(
            self.texture_id,
            egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.min.y + ruler_h), rect.max),
            egui::Rect::from_min_max(
                egui::pos2(0.0, (ruler_h * self.ppp) / self.tex_h.max(1) as f32),
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
            self.theme.text_label,
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
                // 下限 = 128 键恰好铺满视口（不允许比视口更小）。
                let y0 = touch_center.expect("checked above").y;
                let kh = self.view.key_height;
                let bottom = 128.0 * kh - base.scroll_y;
                let anchor_key = (bottom - (y0 - rect.min.y)) / kh;
                let new_kh = (kh * zoom).clamp(rect.height() / 128.0, 48.0);
                let new_bottom = (y0 - rect.min.y) + anchor_key * new_kh;
                let max_scroll = (128.0 * new_kh - rect.height()).max(0.0);
                base.scroll_y = (128.0 * new_kh - new_bottom).clamp(0.0, max_scroll);
                self.view.key_height = new_kh;
                base.dirty = true;
            } else {
                // 内容区捏合 → 水平缩放。锚点 = 手指（双指中心）在内容区内
                // 的位置，缩放后该 tick 停在手指处；无手指信息时取内容区中点。
                let content_w = rect.width() - self.keyboard_w;
                let cx = touch_center
                    .map(|c| (c.x - rect.min.x - self.keyboard_w).clamp(0.0, content_w))
                    .unwrap_or(content_w / 2.0);
                let center_tick = (base.scroll_x + cx) / base.pixels_per_tick.max(1e-6);
                base.pixels_per_tick = (base.pixels_per_tick * zoom).clamp(0.0005, 8.0);
                base.scroll_x = (center_tick * base.pixels_per_tick - cx).max(0.0);
                base.dirty = true;
            }
        }
        // 垂直滚动上限：128 键总高（128*kh）不能小于视口高，滚到底即全部可见。
        let max_scroll = (128.0 * self.view.key_height - rect.height()).max(0.0);
        if touches >= 2 {
            if let Some(pan) = pan {
                base.scroll_x = (base.scroll_x - pan.x).max(0.0);
                base.scroll_y = (base.scroll_y - pan.y).clamp(0.0, max_scroll);
                base.dirty = true;
            }
        } else if pointer_down {
            base.scroll_x = (base.scroll_x - pointer_delta.x).max(0.0);
            base.scroll_y = (base.scroll_y - pointer_delta.y).clamp(0.0, max_scroll);
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
    /// 纹理 = 逻辑尺寸 × pixels_per_point（物理像素，高分屏清晰）；
    /// uniforms 与滚动仍是逻辑坐标（shader 里 NDC 用 u.width 做分母，
    /// 只要分子分母同单位比例就正确，纹理物理尺寸只决定采样分辨率）。
    fn ensure_texture_size(&mut self, rect: egui::Rect) {
        let w = rect.width().max(1.0) as u32;
        let h = rect.height().max(1.0) as u32;
        let tex_w = (rect.width() * self.ppp).round().max(1.0) as u32;
        let tex_h = (rect.height() * self.ppp).round().max(1.0) as u32;
        if w == self.width && h == self.height && tex_w == self.tex_w && tex_h == self.tex_h {
            return;
        }
        let (texture, texture_view, texture_id) = create_target(
            &self.wgpu_state.device,
            &mut self.wgpu_state.renderer.write(),
            self.wgpu_state.target_format,
            tex_w,
            tex_h,
        );
        self.texture = texture;
        self.texture_view = texture_view;
        self.texture_id = texture_id;
        self.width = w;
        self.height = h;
        self.tex_w = tex_w;
        self.tex_h = tex_h;
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
                self.theme.track_bg
            } else {
                self.theme.control_bg
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
            egui::Stroke::new(1.0, self.theme.line_fg),
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
            let white = mix(self.theme.app_bg, self.theme.text_primary, 0.06);
            for key in key_lo..=key_hi {
                if !matches!(key % 12, 1 | 3 | 6 | 8 | 10) {
                    band(painter, key, white);
                }
            }
            return;
        };
        let mask = ev.scale.pitch_classes(ev.root);
        let in_scale = mix(self.theme.app_bg, self.theme.text_primary, 0.06);
        let root = mix(self.theme.app_bg, self.theme.text_primary, 0.12);
        for key in key_lo..=key_hi {
            let pc = (key as u8) % 12;
            let color = if pc == ev.root {
                root
            } else if mask & (1u16 << pc) != 0 {
                in_scale
            } else {
                continue;
            };
            band(painter, key, color);
        }
    }

    /// 八度分隔线：每个 C（key % 12 == 0）的顶部一条横线，与桌面端
    /// bg::paint_octave_lines 一致，纵向定位八度。
    fn draw_octave_lines(&self, ui: &egui::Ui, rect: egui::Rect) {
        let kh = self.view.key_height;
        let bottom = 128.0 * kh - self.view.base.scroll_y;
        let painter = ui.painter();
        for key in (0u8..128).step_by(12) {
            let y = rect.min.y + bottom - key as f32 * kh;
            if y < rect.min.y || y > rect.max.y {
                continue;
            }
            painter.line_segment(
                [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                egui::Stroke::new(1.0, self.theme.line_fg),
            );
        }
    }

    /// 顶部小节标尺：深色底（铺满全宽，含键盘列上方）+ 每小节刻度线 + 小节号。
    /// 与网格线同级别：刻度 = 每小节（变拍子感知），数字只在合并边界显示
    /// （缩放小时每 2/4/8… 小节一个数字），过密时整条隐藏。
    fn draw_ruler(&self, ui: &egui::Ui, rect: egui::Rect, content_rect: egui::Rect, ruler_h: f32) {
        if ruler_h <= 0.0 {
            return;
        }
        let Some(model) = &self.model else {
            return;
        };
        let ppu = self.view.base.pixels_per_tick;
        if ppu <= 0.001 {
            return;
        }
        let scroll_x = self.view.base.scroll_x;
        let tpb = model.meta.ppq.max(1);
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let def_den = def_den.trailing_zeros() as u8;
        let segments = build_time_sig_segments(&model.conductor.time_sig, def_num, def_den);
        let tick_start = scroll_x as f64 / ppu as f64;
        let tick_end = (scroll_x + content_rect.width()) as f64 / ppu as f64;
        const MIN_SPACING: f32 = 38.0;

        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.min.y),
            egui::pos2(rect.max.x, rect.min.y + ruler_h),
        );
        let painter = ui.painter();
        painter.rect_filled(ruler_rect, 0.0, self.theme.track_bg);
        painter.line_segment(
            [
                egui::pos2(ruler_rect.min.x, ruler_rect.max.y),
                egui::pos2(ruler_rect.max.x, ruler_rect.max.y),
            ],
            egui::Stroke::new(1.0, self.theme.line_fg),
        );

        let mut bar_offset = 0u32;
        for (i, &(seg_start, num, den)) in segments.iter().enumerate() {
            let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
            if seg_start as f64 > tick_end {
                break;
            }
            let ticks_per_measure = measure_ticks(tpb, num, den).max(1);
            let measure_px = ticks_per_measure as f32 * ppu;
            if measure_px < 6.0 {
                // 太密整条隐藏（与桌面阈值一致）；段内小节数仍累计。
                let seg_ticks = seg_end.saturating_sub(seg_start) as f32;
                bar_offset += (seg_ticks / ticks_per_measure as f32).ceil() as u32;
                continue;
            }
            let divisor = compute_measure_divisor(measure_px, MIN_SPACING);
            let show_number = measure_px >= 30.0;
            let first_tick = seg_start.max(tick_start as u32);
            let mut tick = seg_start.saturating_add(
                first_tick.saturating_sub(seg_start) / ticks_per_measure * ticks_per_measure,
            );
            while (tick as f64) <= tick_end && tick < seg_end {
                let local = tick - seg_start;
                let x = content_rect.min.x + tick as f32 * ppu - scroll_x;
                if x >= content_rect.min.x && x <= content_rect.max.x {
                    painter.line_segment(
                        [
                            egui::pos2(x, ruler_rect.min.y),
                            egui::pos2(x, ruler_rect.max.y),
                        ],
                        egui::Stroke::new(1.0, self.theme.tick_label),
                    );
                    if show_number && local % (ticks_per_measure * divisor) == 0 {
                        painter.text(
                            egui::pos2(x + 3.0, ruler_rect.min.y + 1.0),
                            egui::Align2::LEFT_TOP,
                            // 小节号从 1 开始（tick 0 = 第 1 小节），跨段累计。
                            (bar_offset + local / ticks_per_measure + 1).to_string(),
                            egui::FontId::proportional(10.0),
                            self.theme.measure_label,
                        );
                    }
                }
                tick += ticks_per_measure;
            }
            let seg_ticks = seg_end.saturating_sub(seg_start) as f32;
            bar_offset += (seg_ticks / ticks_per_measure as f32).ceil() as u32;
        }
    }

    /// 播放光标：竖线随 scroll_x 移动，y 从标尺底部到内容区底。
    /// 光标不在视口内时不画——贴边显示会误导"线就在那里"。
    fn draw_cursor(&self, ui: &egui::Ui, rect: egui::Rect, ruler_h: f32) {
        let Some(tick) = self.cursor_tick else {
            return;
        };
        let ppu = self.view.base.pixels_per_tick;
        let scroll_x = self.view.base.scroll_x;
        let left = rect.min.x + self.keyboard_w;
        let x = left + (tick as f32 * ppu - scroll_x);
        if x < left || x > rect.max.x {
            return;
        }
        let painter = ui.painter();
        painter.line_segment(
            [
                egui::pos2(x, rect.min.y + ruler_h),
                egui::pos2(x, rect.max.y),
            ],
            egui::Stroke::new(2.0, self.theme.accent_active),
        );
    }

    /// 网格线分层（与桌面端 grid_lines 一致，支持变拍子）：
    /// 小节线 2px、四分音符线 1px、十六分线 1px、tick 线（最大缩放）。
    /// 每拍号段从段起点 local 对齐，变拍子段不会错位。
    fn draw_grid(&self, ui: &egui::Ui, rect: egui::Rect) {
        let Some(model) = &self.model else {
            return;
        };
        let ppu = self.view.base.pixels_per_tick;
        if ppu <= 0.001 {
            return;
        }
        let scroll_x = self.view.base.scroll_x;
        let left = rect.min.x;
        let right = rect.max.x;
        let tick_start = scroll_x as f64 / ppu as f64;
        let tick_end = (scroll_x + rect.width()) as f64 / ppu as f64;
        let tpb = model.meta.ppq.max(1);
        let (def_num, def_den) = model.tempo_map.time_sig_default;
        let def_den = def_den.trailing_zeros() as u8;
        let segments = build_time_sig_segments(&model.conductor.time_sig, def_num, def_den);
        let painter = ui.painter();
        // 网格线 = 标尺标签的下一级（与桌面 grid_lines 同规则）：
        // 合并小节标签 → 合并/2 小节线；每小节标签 → 四分音符线；
        // beat 标签 → 十六分线；sub 标签 → tick 线（仅最大缩放）。
        const MIN_SPACING: f32 = 38.0;
        const SUB_BEAT_DIV: u32 = 4;
        // tick 线只在最大缩放显示：安卓 ppu 上限 8.0（与 handle_touch clamp 一致），
        // 桌面端 10.0 在这里永远不可达，tick 线会永不显示。
        const MAX_PPU: f32 = 8.0;
        let ticks_per_sub = (tpb / SUB_BEAT_DIV).max(1);

        for (i, &(seg_start, num, den)) in segments.iter().enumerate() {
            let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
            let seg_start_f = seg_start as f64;
            if seg_start_f > tick_end {
                break;
            }

            let ticks_per_measure = measure_ticks(tpb, num, den).max(1);
            let ticks_per_beat = (ticks_per_measure / num.max(1) as u32).max(1);
            let measure_divisor =
                compute_measure_divisor(ticks_per_measure as f32 * ppu, MIN_SPACING);
            let merged_measure_ticks = ticks_per_measure.saturating_mul(measure_divisor);
            let merged = measure_divisor > 1;
            let show_beat = !merged;
            let show_sub = show_beat && (ticks_per_beat as f32 * ppu) >= MIN_SPACING;
            let show_tick = show_sub && ppu >= MAX_PPU;
            let grid_measure_ticks = if merged {
                (merged_measure_ticks / 2).max(1)
            } else {
                ticks_per_measure
            };
            let step = if show_tick {
                1u32
            } else if show_sub {
                ticks_per_sub
            } else if show_beat {
                ticks_per_beat
            } else {
                grid_measure_ticks.max(1)
            };

            // 段内 local 对齐：变拍子段的 seg_start 通常不在全局 step 网格上。
            let first_tick = seg_start_f.max(tick_start);
            let step_f = step as f64;
            let first = seg_start.saturating_add(
                (((first_tick - seg_start_f) / step_f).floor() as u32).saturating_mul(step),
            );

            let mut tick = first;
            while (tick as f64) <= tick_end && tick < seg_end {
                let local = tick - seg_start;
                let x = rect.min.x + tick as f32 * ppu - scroll_x;
                if x >= left && x <= right {
                    let is_measure = local % ticks_per_measure == 0;
                    let beat_local = local % ticks_per_measure;
                    let is_beat_pos = beat_local.is_multiple_of(ticks_per_beat) && beat_local > 0;
                    let is_sub_pos = local % ticks_per_sub == 0;
                    let (w, color) = if is_measure {
                        (2.0, self.theme.line_fg)
                    } else if show_beat && is_beat_pos {
                        (1.0, self.theme.text_label)
                    } else if show_sub && is_sub_pos {
                        (1.0, self.theme.grid_sub_beat)
                    } else if show_tick {
                        (1.0, self.theme.grid_tick)
                    } else {
                        tick += step;
                        continue;
                    };
                    painter.line_segment(
                        [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                        egui::Stroke::new(w, color),
                    );
                }
                tick += step;
            }
        }
    }
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
