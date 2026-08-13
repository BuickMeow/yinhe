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
    /// 诊断：visible count 打印计数。
    diag_frame: u32,
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
            diag_frame: 0,
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

    /// 主 UI 入口：背景 + 键盘列 + 网格 + GPU 音符层 + 触摸交互。
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(22));

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
        if !self.view_initialized {
            self.view.base.scroll_y = (60.0 * self.view.key_height - rect.height() / 2.0).max(0.0);
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
            log::info!(
                "pr_view: draw 帧 notes_uploaded={} cull_ready={} ppu={} scroll=({:.0},{:.0})",
                self.notes_uploaded,
                renderer.cull_is_ready(),
                job.uniforms.pixels_per_tick,
                job.uniforms.scroll_x,
                job.uniforms.scroll_y,
            );
            self.diag_frame += 1;
            if self.diag_frame.is_multiple_of(30) {
                log::info!("pr_view: cull visible = {}", renderer.cull_visible_count());
            }
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
            self.diag_frame += 1;
            if self.diag_frame.is_multiple_of(60) {
                log::info!(
                    "pr_view: 诊断 cull_visible={} ",
                    renderer.cull_visible_count()
                );
                // 读回 key 60 的 draw_args 前 2 个 chunk（5 字段/chunk）+
                // visible_indices 前 8 个槽，确认 GPU 端数据真实存在。
                let args = renderer.cull_draw_args_diag(60, 2);
                let vis = renderer.cull_visible_indices_diag(60, 8);
                log::info!("pr_view: key60 args={args:?} visible_indices={vis:?}");
                // 三对照实验：定位「cull 数据全对但渲染 0 像素」的断点（indirect /
                // 顶点 storage / 光栅化三环节逐个排除）。
                let exp = renderer.diag_draw_experiments(
                    self.wgpu_state.target_format,
                    self.width,
                    self.height,
                );
                log::info!("pr_view: draw实验 {exp}");
                self.diag_tex_readback();
            }
        }

        let kb_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.min.x + self.keyboard_w, rect.max.y),
        );
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + self.keyboard_w, rect.min.y),
            rect.max,
        );
        self.draw_keyboard(ui, kb_rect);
        self.draw_grid(ui, content_rect);
        // 离屏纹理覆盖整个 rect（含键盘列，与桌面端一致）：shader 的音符
        // 像素坐标从 keyboard_width 起步，NDC 用总宽做分母，纹理必须同宽，
        // 否则音符整体右移 keyboard_width 像素且右端溢出被裁（安卓看不到
        // 音符的根因）。键盘列是纹理的透明区，由 egui 键盘层透出。
        painter.image(
            self.texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        let notes: u64 = model.track_note_count.iter().sum();
        let status = format!(
            "{} | {notes} 音符 | {:.3} px/tick | 双指缩放/拖动",
            self.status, self.view.base.pixels_per_tick,
        );
        painter.text(
            egui::pos2(rect.min.x + 8.0, rect.min.y + 8.0),
            egui::Align2::LEFT_TOP,
            status,
            egui::FontId::proportional(13.0),
            egui::Color32::from_gray(200),
        );
    }

    /// 触摸手势：双指捏合 → 水平缩放（围绕屏幕中心）；双指/单指 → 滚动。
    fn handle_touch(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let (zoom, pan, touches, pointer_delta, pointer_down) = ui.ctx().input(|i| {
            let mt = i.multi_touch();
            (
                mt.map(|m| m.zoom_delta),
                mt.map(|m| m.translation_delta),
                mt.map(|m| m.num_touches).unwrap_or(0),
                i.pointer.delta(),
                i.pointer.primary_down(),
            )
        });
        let base = &mut self.view.base;
        if let Some(zoom) = zoom
            && (zoom - 1.0).abs() > 0.001
        {
            // scroll_x 是内容区（不含键盘列）的滚动偏移，缩放中心取内容区中点。
            let content_w = rect.width() - self.keyboard_w;
            let center_tick = (base.scroll_x + content_w / 2.0) / base.pixels_per_tick.max(1e-6);
            base.pixels_per_tick = (base.pixels_per_tick * zoom).clamp(0.0005, 8.0);
            base.scroll_x = center_tick * base.pixels_per_tick - content_w / 2.0;
            base.dirty = true;
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
    fn draw_keyboard(&self, ui: &egui::Ui, rect: egui::Rect) {
        let kh = self.view.key_height;
        let scroll_y = self.view.base.scroll_y;
        let key_lo = (scroll_y / kh).floor() as i32;
        let key_hi = ((scroll_y + rect.height()) / kh).ceil() as i32;
        let painter = ui.painter();
        let is_black = |k: i32| matches!(k % 12, 1 | 3 | 6 | 8 | 10);
        for key in key_lo.clamp(0, 127)..=key_hi.clamp(0, 127) {
            let y = rect.min.y + key as f32 * kh - scroll_y;
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

    /// 诊断：读回离屏纹理并统计非零像素，确认 GPU 是否真的画了音符。
    fn diag_tex_readback(&mut self) {
        let device = &self.wgpu_state.device;
        let bpp = 4;
        // wgpu 要求 bytes_per_row 是 COPY_BYTES_PER_ROW_ALIGNMENT(256) 的倍数，
        // 否则 submit 会被拒绝、读回全 0（上次就是这个原因导致诊断误判）。
        let bytes_per_row = (self.width * bpp).div_ceil(256) * 256;
        let size = bytes_per_row as u64 * self.height as u64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pr_diag_readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.wgpu_state.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if rx.recv().is_err() {
            log::error!("pr_view: readback map 失败");
            return;
        }
        let data = match buffer.slice(..).get_mapped_range() {
            Ok(d) => d,
            Err(e) => {
                log::error!("pr_view: readback 映射失败: {e}");
                return;
            }
        };
        let mut nonzero_alpha = 0u64;
        let mut nonzero_rgb = 0u64;
        let mut non_red = 0u64;
        let mut sample_first: Option<[u8; 4]> = None;
        for chunk in data.chunks_exact(4) {
            let px = [chunk[0], chunk[1], chunk[2], chunk[3]];
            if px[3] > 0 {
                nonzero_alpha += 1;
                if sample_first.is_none() {
                    sample_first = Some(px);
                }
            }
            if px[0] > 0 || px[1] > 0 || px[2] > 0 {
                nonzero_rgb += 1;
            }
            // 当前 clear 是纯红 [255,0,0,255]：统计非红像素判断是否画了东西。
            if px[0] != 255 || px[1] != 0 || px[2] != 0 {
                non_red += 1;
            }
        }
        drop(data);
        buffer.unmap();
        log::info!(
            "pr_view: 纹理读回 {}x{} 非零alpha={} 非零RGB={} 非红像素={} 首个={:?}",
            self.width,
            self.height,
            nonzero_alpha,
            nonzero_rgb,
            non_red,
            sample_first
        );
    }

    /// 网格线：每 1920 tick（4/4 小节 @480ppq）一条竖线，过密时跳过。
    fn draw_grid(&self, ui: &egui::Ui, rect: egui::Rect) {
        let ppu = self.view.base.pixels_per_tick;
        let scroll_x = self.view.base.scroll_x;
        const STEP_TICKS: i64 = 1920;
        let step_px = STEP_TICKS as f32 * ppu;
        if step_px < 6.0 {
            return;
        }
        let start_tick = ((scroll_x / ppu) as i64 / STEP_TICKS) * STEP_TICKS;
        let painter = ui.painter();
        let mut x = rect.min.x + (start_tick as f32 * ppu - scroll_x);
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
