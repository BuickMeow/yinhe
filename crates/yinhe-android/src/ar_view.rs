//! AR 工程走带视图：wgpu 音符渲染（mode 2，轨道 lane）+ 音轨面板 + 触摸交互。
//!
//! 布局：左侧音轨面板列（egui 绘制），右侧 AR 音符视图（GPU 渲染到离屏
//! 纹理后贴图）。与 PR 相同的"纹理全宽、面板列透明"模式：音符 x 坐标从
//! 面板列宽起步（shader keyboard_width），面板列由 egui 层绘制。

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use yinhe_core::YinModel;
use yinhe_theme::base::BaseColors;
use yinhe_theme::egui_colors::{Theme, derive_theme};
use yinhe_types::ArrangementView;
use yinhe_wgpu::{InstanceRenderer, Uniforms, build_arr_notes, layer_cache_key};

/// 音轨面板列宽（逻辑像素）。音符 lane 区域从该宽度起步。
const PANEL_W: f32 = 168.0;

/// AR 视图事件：本帧产生的 UI 操作，由 lib.rs 消费。
#[derive(Debug, PartialEq)]
pub enum ArEvent {
    /// 点击轨道行 → 进入 PR 编辑该轨。
    EnterPr(u16),
    /// 静音/独奏状态变化后的音频 skip mask。
    SkipTracks(Vec<bool>),
}

/// AR 视图：视口状态 + GPU 渲染 + 音轨面板 + 触摸交互。
pub struct ArView {
    wgpu_state: Arc<eframe::egui_wgpu::RenderState>,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    texture_id: egui::TextureId,
    renderer: Option<InstanceRenderer>,
    /// uniforms/视口逻辑尺寸（逻辑像素，egui 坐标单位）。
    width: u32,
    height: u32,
    /// 离屏纹理物理尺寸（逻辑尺寸 × pixels_per_point）。
    tex_w: u32,
    tex_h: u32,
    /// 当前像素密度（每帧从 egui 读取）。
    ppp: f32,
    view: ArrangementView,
    model: Option<Arc<YinModel>>,
    status: String,
    /// 独奏轨索引（None = 无独奏）。与 muted 共同决定音频 skip mask。
    solo: Option<u16>,
    /// 每轨静音状态。
    muted: Vec<bool>,
    /// 播放光标 tick（None = 隐藏），lib.rs 每帧从音频位置换算后设置。
    cursor_tick: Option<f64>,
    /// 上一次内容矩形（诊断日志用：变化时打印坐标）。
    last_rect: egui::Rect,
    /// 主题色（与桌面端同源：BaseColors 7 色派生）。
    theme: Theme,
}

/// 音轨面板行内的 M/S 小按钮。
#[derive(Clone, Copy, PartialEq)]
enum MsButton {
    Mute,
    Solo,
}

impl ArView {
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
            view: ArrangementView {
                base: yinhe_types::TimelineViewBase {
                    pixels_per_tick: 0.08,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    left_panel_width: PANEL_W,
                    dirty: true,
                    track_panel_row_height: 56.0,
                    track_panel_scroll_y: 0.0,
                },
            },
            model: None,
            status: String::new(),
            solo: None,
            muted: Vec::new(),
            cursor_tick: None,
            last_rect: egui::Rect::NOTHING,
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
        self.muted = model.tracks.iter().map(|t| t.muted).collect();
        self.solo = None;
        self.model = Some(model);
        self.status = "就绪".to_string();
    }

    /// 设置播放光标位置（tick），None 隐藏。lib.rs 每帧从音频位置换算后调用。
    pub fn set_cursor(&mut self, tick: Option<f64>) {
        self.cursor_tick = tick;
    }

    /// 播放跟随：水平滚动让光标位于内容区中央（跟随播放开启时每帧调用）。
    pub fn follow_cursor(&mut self) {
        let Some(tick) = self.cursor_tick else {
            return;
        };
        let content_w = (self.width as f32 - PANEL_W).max(1.0);
        let target = tick as f32 * self.view.base.pixels_per_tick - content_w / 2.0;
        self.view.base.scroll_x = target.max(0.0);
        self.view.base.dirty = true;
    }

    /// 主 UI 入口：背景 + 音轨面板 + GPU 音符层 + 触摸交互。
    /// `safe` 为安全区 insets（逻辑点）：[left, top, right, bottom]。
    pub fn ui(&mut self, ui: &mut egui::Ui, safe: [f32; 4]) -> Vec<ArEvent> {
        let mut events = Vec::new();
        let full = ui.available_rect_before_wrap();
        let painter = ui.painter();
        // 背景铺满整个视口（延伸到挖孔/刘海后面）；内容区避开安全区。
        painter.rect_filled(full, 0.0, self.theme.app_bg);
        let rect = egui::Rect::from_min_max(
            full.min + egui::vec2(safe[0], safe[1]),
            full.max - egui::vec2(safe[2], safe[3]),
        );
        // 诊断：坐标变化时打印一次，定位挖孔避让/错位问题。
        if rect != self.last_rect {
            self.last_rect = rect;
            log::debug!(
                "ar_view: full={full:?} safe={safe:?} rect={rect:?} ppp={}",
                self.ppp
            );
        }
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return events;
        }

        self.width = rect.width() as u32;
        self.height = rect.height() as u32;
        self.ppp = ui.ctx().pixels_per_point().max(0.25);
        self.ensure_texture_size(rect);

        // 注意：不能在这里用 allocate_painter(rect.size())——它从当前光标位置
        // 分配，会把挖孔避让后的 rect 覆盖回 (0, top)，导致整个内容左移一个
        // 挖孔距离。用 allocate_rect 在指定位置分配。
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let painter = ui.painter().clone();
        let rect = resp.rect;

        // ── 触摸：单指拖动滚动，双指捏合缩放（x 缩放 + lane 缩放）──
        let (zoom, pan, touches, touch_center) = ui.ctx().input(|i| {
            let mt = i.multi_touch();
            (
                mt.map(|m| m.zoom_delta),
                mt.map(|m| m.translation_delta),
                mt.map(|m| m.num_touches).unwrap_or(0),
                mt.map(|m| m.center_pos),
            )
        });
        if let Some(zoom) = zoom
            && (zoom - 1.0).abs() > 0.001
        {
            // 音轨面板上捏合 = 垂直缩放（lane 高度）；AR 视口捏合 = 水平缩放（同 PR）。
            let in_panel = touch_center
                .map(|c| c.x < rect.min.x + PANEL_W)
                .unwrap_or(false);
            if in_panel {
                let cy = touch_center
                    .map(|c| (c.y - rect.min.y).clamp(0.0, rect.height()))
                    .unwrap_or(rect.height() / 2.0);
                let old_h = self.view.lane_height();
                let new_h = (old_h * zoom).clamp(16.0, 120.0);
                let track_frac = (cy + self.view.base.scroll_y) / old_h.max(1.0);
                self.view.base.scroll_y = (track_frac * new_h - cy).max(0.0);
                self.view.base.track_panel_row_height = new_h;
                self.view.base.dirty = true;
            } else {
                let content_w = (rect.width() - PANEL_W).max(1.0);
                let cx = touch_center
                    .map(|c| (c.x - rect.min.x - PANEL_W).clamp(0.0, content_w))
                    .unwrap_or(content_w / 2.0);
                let center_tick =
                    (self.view.base.scroll_x + cx) / self.view.base.pixels_per_tick.max(1e-6);
                self.view.base.pixels_per_tick =
                    (self.view.base.pixels_per_tick * zoom).clamp(0.0005, 8.0);
                self.view.base.scroll_x =
                    (center_tick * self.view.base.pixels_per_tick - cx).max(0.0);
                self.view.base.dirty = true;
            }
        }
        let num_tracks = self.model.as_ref().map(|m| m.tracks.len()).unwrap_or(0);
        let max_scroll_y = (num_tracks as f32 * self.view.lane_height() - rect.height()).max(0.0);
        if touches >= 2 {
            if let Some(pan) = pan {
                self.view.base.scroll_x = (self.view.base.scroll_x - pan.x).max(0.0);
                self.view.base.scroll_y =
                    (self.view.base.scroll_y - pan.y).clamp(0.0, max_scroll_y);
                self.view.base.dirty = true;
            }
        } else if resp.dragged() {
            let d = resp.drag_delta();
            self.view.base.scroll_x = (self.view.base.scroll_x - d.x).max(0.0);
            self.view.base.scroll_y = (self.view.base.scroll_y - d.y).clamp(0.0, max_scroll_y);
            self.view.base.dirty = true;
        }

        // ── 点击命中：M/S 按钮 → 静音/独奏；轨道行 → 进入 PR ──
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            events.extend(self.handle_tap(pos, rect));
        }

        // ── 渲染 ──
        let Some(model) = self.model.clone() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "未加载工程（点右上角打开）",
                egui::FontId::proportional(18.0),
                self.theme.text_muted,
            );
            return events;
        };

        self.view.clamp_scroll(
            rect.width(),
            rect.height(),
            model.tempo_map.tick_length as f64,
            num_tracks,
        );
        self.ensure_renderer();
        let track_visible: Vec<bool> = vec![true; num_tracks];
        if let Some(renderer) = &mut self.renderer {
            let uniforms = Uniforms {
                width: rect.width(),
                height: rect.height(),
                scroll_x: self.view.base.scroll_x,
                scroll_y: self.view.base.scroll_y,
                pixels_per_tick: self.view.base.pixels_per_tick,
                key_height: 0.0, // AR 未使用（shader 用 lane_height）
                keyboard_width: PANEL_W,
                mode: 2, // AR notes：shader 用 lane_height + scroll_y 计算 y
                scroll_frac: 0.0,
                scroll_mode: 0,
                min_border_width: 0.0,
                track_count: num_tracks.min(yinhe_wgpu::MAX_TRACKS) as u32,
                sel_rect_count: 0,
                note_outline: 1,
                lane_height: self.view.lane_height(),
                value_zoom: 0.0,
                value_scroll: 0.0,
            };
            renderer.upload_uniforms(uniforms);
            let tc = crate::track_colors_for(&model);
            renderer.upload_track_colors(&tc);
            renderer.ensure_layers(2);
            let vh = self.view.render_hash();
            let notes_key = layer_cache_key(&[vh, rect.width() as u64, rect.height() as u64]);
            renderer.upload_note_layer(0, notes_key, |out| {
                build_arr_notes(
                    out,
                    rect.width(),
                    rect.height(),
                    model.as_ref(),
                    &self.view,
                    &track_visible,
                    &HashSet::new(),
                );
            });
            let mut encoder = self
                .wgpu_state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            renderer.draw(&mut encoder, &self.texture_view, self.tex_w, self.tex_h);
            self.wgpu_state.queue.submit([encoder.finish()]);
        }

        // 纹理贴图：全宽（含面板列，面板列透明），1:1 像素映射。
        painter.image(
            self.texture_id,
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        // ── 音轨面板（egui 层，画在纹理面板列透明区之上）──
        let tc = crate::track_colors_for(&model);
        self.draw_track_panel(&painter, rect, &model, &tc);

        // ── 播放线 ──
        if let Some(tick) = self.cursor_tick {
            let x = rect.min.x + PANEL_W + tick as f32 * self.view.base.pixels_per_tick
                - self.view.base.scroll_x;
            if x >= rect.min.x + PANEL_W && x <= rect.max.x {
                painter.line_segment(
                    [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                    egui::Stroke::new(2.0, self.theme.accent_active),
                );
            }
        }

        let notes: u64 = model.track_note_count.iter().sum();
        painter.text(
            egui::pos2(rect.min.x + PANEL_W + 8.0, rect.min.y + 6.0),
            egui::Align2::LEFT_TOP,
            format!("{} | {notes} 音符 | {num_tracks} 轨", self.status),
            egui::FontId::proportional(13.0),
            self.theme.text_label,
        );
        events
    }

    /// 点击命中：优先 M/S 按钮，其次轨道行（进入 PR）。
    fn handle_tap(&mut self, pos: egui::Pos2, rect: egui::Rect) -> Vec<ArEvent> {
        let mut events = Vec::new();
        let Some(model) = &self.model else {
            return events;
        };
        if pos.x < rect.min.x || pos.x > rect.min.x + PANEL_W || pos.y < rect.min.y {
            return events;
        }
        let lane_h = self.view.lane_height();
        let row_idx = ((pos.y - rect.min.y + self.view.base.scroll_y) / lane_h) as usize;
        if row_idx >= model.tracks.len() {
            return events;
        }
        let y = rect.min.y + row_idx as f32 * lane_h - self.view.base.scroll_y;
        let row = egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(PANEL_W, lane_h));
        // 触摸命中区放大 4px，防止误触相邻按钮。
        let mut hit_m = false;
        let mut hit_s = false;
        if let Some(btn) = ms_button_rect(row, MsButton::Mute)
            && btn.expand(4.0).contains(pos)
        {
            hit_m = true;
        }
        if let Some(btn) = ms_button_rect(row, MsButton::Solo)
            && btn.expand(4.0).contains(pos)
        {
            hit_s = true;
        }
        if hit_m {
            let idx = row_idx as u16;
            let new_state = !self.muted.get(idx as usize).copied().unwrap_or(false);
            if self.muted.len() > idx as usize {
                self.muted[idx as usize] = new_state;
            }
            events.push(ArEvent::SkipTracks(self.skip_mask()));
            self.status = format!(
                "轨道 {idx:03} {}",
                if new_state { "静音" } else { "取消静音" }
            );
        } else if hit_s {
            let idx = row_idx as u16;
            self.solo = if self.solo == Some(idx) {
                None
            } else {
                Some(idx)
            };
            events.push(ArEvent::SkipTracks(self.skip_mask()));
            self.status = match self.solo {
                Some(s) => format!("独奏轨道 {s:03}"),
                None => "取消独奏".to_string(),
            };
        } else {
            events.push(ArEvent::EnterPr(row_idx as u16));
        }
        events
    }

    /// 当前 skip mask：独奏时只留独奏轨，否则按 muted。
    fn skip_mask(&self) -> Vec<bool> {
        let n = self.muted.len();
        (0..n)
            .map(|i| match self.solo {
                Some(s) => i as u16 != s,
                None => self.muted[i],
            })
            .collect()
    }

    /// 音轨面板：可见范围内的轨道行（色条/编号/名称/M/S）。
    fn draw_track_panel(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        model: &Arc<YinModel>,
        track_colors: &[[f32; 4]],
    ) {
        let lane_h = self.view.lane_height();
        let num = model.tracks.len();
        let (first, last) = ArrangementView::visible_track_range_static(
            self.view.base.scroll_y,
            rect.height(),
            lane_h,
            num,
        );
        for idx in first..last {
            let y =
                rect.min.y + ArrangementView::lane_y_static(idx, self.view.base.scroll_y, lane_h);
            let row =
                egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(PANEL_W, lane_h));
            if row.max.y < rect.min.y || row.min.y > rect.max.y {
                continue;
            }
            if idx % 2 == 0 {
                painter.rect_filled(row, 0.0, self.theme.stripe_bg);
            }
            let track = &model.tracks[idx];
            let c = track_colors
                .get(idx)
                .copied()
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
            let color = egui::Color32::from_rgba_unmultiplied(
                (c[0] * 255.0) as u8,
                (c[1] * 255.0) as u8,
                (c[2] * 255.0) as u8,
                (c[3] * 255.0) as u8,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(row.min, egui::vec2(6.0, lane_h)),
                0.0,
                color,
            );

            let text_x = row.min.x + 10.0;
            let font = egui::FontId::proportional((lane_h * 0.24).clamp(9.0, 13.0));
            let small = (lane_h * 0.2).clamp(8.0, 11.0);
            let port = match track.port {
                0 => 'A',
                1 => 'B',
                2 => 'C',
                3 => 'D',
                _ => '?',
            };
            painter.text(
                egui::pos2(text_x, row.min.y + lane_h * 0.30),
                egui::Align2::LEFT_CENTER,
                format!("{:03}  {port}{:02}", idx, track.channel + 1),
                font.clone(),
                self.theme.text_primary,
            );
            painter.text(
                egui::pos2(text_x, row.min.y + lane_h * 0.68),
                egui::Align2::LEFT_CENTER,
                &track.name,
                egui::FontId::proportional(small),
                self.theme.text_label,
            );
            // M/S 按钮：行右侧竖排。
            if let Some(m_rect) = ms_button_rect(row, MsButton::Mute) {
                let muted = self.muted.get(idx).copied().unwrap_or(false);
                painter.rect_filled(
                    m_rect,
                    4.0,
                    if muted {
                        self.theme.mute_active
                    } else {
                        self.theme.btn_bg
                    },
                );
                painter.text(
                    m_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "M",
                    egui::FontId::proportional(small),
                    if muted {
                        self.theme.contrast_fg
                    } else {
                        self.theme.text_muted
                    },
                );
            }
            if let Some(s_rect) = ms_button_rect(row, MsButton::Solo) {
                let solo = self.solo == Some(idx as u16);
                painter.rect_filled(
                    s_rect,
                    4.0,
                    if solo {
                        self.theme.solo_active
                    } else {
                        self.theme.btn_bg
                    },
                );
                painter.text(
                    s_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "S",
                    egui::FontId::proportional(small),
                    if solo {
                        self.theme.contrast_fg
                    } else {
                        self.theme.text_muted
                    },
                );
            }
        }
    }

    /// 惰性初始化 InstanceRenderer（GPU 管道：compute cull + draw）。
    fn ensure_renderer(&mut self) {
        if self.renderer.is_none() {
            let device = self.wgpu_state.device.clone();
            device.on_uncaptured_error(Arc::new(|err| {
                log::error!("wgpu uncaptured error: {err}");
            }));
            let queue = self.wgpu_state.queue.clone();
            let format = self.wgpu_state.target_format;
            log::info!("ar_view: InstanceRenderer 初始化，target_format={format:?}");
            self.renderer = Some(InstanceRenderer::new(device, queue, format));
        }
    }

    /// 按视口尺寸重建离屏纹理（尺寸变化时）。
    fn ensure_texture_size(&mut self, rect: egui::Rect) {
        let w = (rect.width() * self.ppp).round().max(1.0) as u32;
        let h = (rect.height() * self.ppp).round().max(1.0) as u32;
        if w == self.tex_w && h == self.tex_h {
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
        self.tex_w = w;
        self.tex_h = h;
        log::info!("ar_view: 纹理重建 {w}x{h}");
    }
}

/// 行内 M/S 按钮矩形（行高过矮时不显示，返回 None）。
fn ms_button_rect(row: egui::Rect, which: MsButton) -> Option<egui::Rect> {
    if row.height() < 34.0 {
        return None;
    }
    let size = (row.height() * 0.44).clamp(18.0, 26.0);
    let gap = 4.0;
    let right = row.max.x - 6.0;
    let x = match which {
        MsButton::Mute => right - size,
        MsButton::Solo => right - size * 2.0 - gap,
    };
    Some(egui::Rect::from_min_size(
        egui::pos2(x, row.center().y - size / 2.0),
        egui::vec2(size, size),
    ))
}

/// 创建离屏渲染目标（纹理 + view + egui 注册）。
/// 与 pr_view::create_target 同款（含 max_dim 钳制与线性格式 view_formats）。
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
        label: Some("yinhe-ar-offscreen"),
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
